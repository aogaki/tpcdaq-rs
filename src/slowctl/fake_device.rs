//! テスト用の偽デバイス(TODO/079 §2.6-3)。
//!
//! **別の `Driver` 実装は作らない** —— Phase 1 の [`FakeProPar`] を
//! インメモリの双方向パイプ([`SharedFake`])で挟み、**本物の
//! [`BronkhorstDriver`](super::bronkhorst::BronkhorstDriver) をそのまま回す**。
//! したがってコーデック・chaining・型タグ・カウンタまで実経路を通る。
//!
//! # 値の注入
//!
//! [`FakeProPar`] は「要求行 → 応答行」の完全一致表なので、値を動かすには**応答表を
//! 作り直す**。[`FakeBus::refresh`] が毎回 `FakeProPar` を丸ごと組み直す(古い silent 指定や
//! 応答が残らない = 状態の持ち越し事故が起きない)。
//!
//! 子プロセス(統合テスト)へは [`FakeDevice::from_script`] の**スクリプトファイル**で
//! 時系列を渡す(1 行 = 1 poll。数値 or `silent`、`#` 以降はコメント、尽きたら最後の行を保持)。

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use super::bronkhorst::{measure_request_for, BronkhorstDriver};
use super::fake::FakeProPar;
use super::propar::{capacity_request, ident_request, ParType, Request};
use super::{Driver, DriverError, Ident, NodesSpec, Sample};

// ---------------------------------------------------------------------
// 応答行の組み立て(Phase 1 の `parse_reply` が読める形)
// ---------------------------------------------------------------------

/// [`encode_reply`] に渡す値。要求の [`ParType`] と型が揃っている必要がある。
#[derive(Debug, Clone, PartialEq)]
pub enum ReplyValue {
    U8(u8),
    U32(u32),
    F32(f32),
    Str(String),
}

/// 9.17.027 §3.4.2 の 2 bit 型タグ(`propar` 側と同じ表 —— fake 専用の写し)。
fn type_bits(ty: ParType) -> u8 {
    match ty {
        ParType::U8 => 0,
        ParType::U16 => 1,
        ParType::U32 | ParType::F32 => 2,
        ParType::Str(_) => 3,
    }
}

/// `request` に対する読み出し応答行(`:LL NN 02 …\r\n`)を組み立てる。
///
/// chain ビットは立てない(項目ごとに process バイトを起こす単純形)。Phase 1 の
/// `decode_items` はどちらの形も読めるので、fake は読みやすいほうを出す。
pub fn encode_reply(node: u8, request: &Request, values: &[ReplyValue]) -> String {
    let mut body = vec![node, 0x02];
    for (item, value) in request.items.iter().zip(values) {
        body.push(item.process & 0x7F);
        body.push((type_bits(item.ty) << 5) | (item.param & 0x1F));
        match value {
            ReplyValue::U8(v) => body.push(*v),
            ReplyValue::U32(v) => body.extend_from_slice(&v.to_be_bytes()),
            ReplyValue::F32(v) => body.extend_from_slice(&v.to_bits().to_be_bytes()),
            ReplyValue::Str(s) => {
                body.push(s.len() as u8);
                body.extend_from_slice(s.as_bytes());
            }
        }
    }
    let mut line = String::with_capacity(3 + body.len() * 2 + 2);
    line.push(':');
    line.push_str(&format!("{:02X}", body.len()));
    for b in &body {
        line.push_str(&format!("{b:02X}"));
    }
    line.push_str("\r\n");
    line
}

// ---------------------------------------------------------------------
// 双方向パイプ(FakeProPar を共有する Read + Write ハンドル)
// ---------------------------------------------------------------------

/// [`FakeProPar`] を共有する `Read + Write`。ドライバはこれをシリアルポートだと思って使う。
#[derive(Clone)]
pub struct SharedFake(Arc<Mutex<FakeProPar>>);

fn poisoned() -> io::Error {
    io::Error::other("FakeProPar mutex poisoned")
}

impl Read for SharedFake {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.lock().map_err(|_| poisoned())?.read(buf)
    }
}

impl Write for SharedFake {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().map_err(|_| poisoned())?.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.lock().map_err(|_| poisoned())?.flush()
    }
}

// ---------------------------------------------------------------------
// FakeBus(ノード表 + 応答表の再生成)
// ---------------------------------------------------------------------

/// 1 ノード分の偽の中身。
#[derive(Debug, Clone)]
struct FakeNode {
    unit: String,
    model: String,
    serial: String,
    controller: bool,
    fmeasure: f32,
    fsetpoint: f32,
    temperature: f32,
    alarm_info: u8,
    valve_output: u32,
    silent: bool,
    corrupt: bool,
}

/// 偽 FLOW-BUS。`io()` が返すハンドルをドライバに渡し、値はここから動かす。
pub struct FakeBus {
    nodes: BTreeMap<u8, FakeNode>,
    shared: SharedFake,
}

impl Default for FakeBus {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeBus {
    pub fn new() -> Self {
        Self {
            nodes: BTreeMap::new(),
            shared: SharedFake(Arc::new(Mutex::new(FakeProPar::new()))),
        }
    }

    /// ドライバに渡すトランスポート(共有ハンドル)。
    pub fn io(&self) -> SharedFake {
        self.shared.clone()
    }

    /// 計測のみの機器(Sensor type bit7 = 1)。
    pub fn add_meter(
        &mut self,
        node: u8,
        unit: &str,
        model: &str,
        serial: &str,
        fmeasure: f32,
    ) -> &mut Self {
        self.add(node, unit, model, serial, fmeasure, false, 0);
        self
    }

    /// 制御器(Sensor type bit7 = 0 —— バルブ開度を持つ)。
    pub fn add_controller(
        &mut self,
        node: u8,
        unit: &str,
        model: &str,
        serial: &str,
        fmeasure: f32,
        valve_output: u32,
    ) -> &mut Self {
        self.add(node, unit, model, serial, fmeasure, true, valve_output);
        self
    }

    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        node: u8,
        unit: &str,
        model: &str,
        serial: &str,
        fmeasure: f32,
        controller: bool,
        valve_output: u32,
    ) {
        self.nodes.insert(
            node,
            FakeNode {
                unit: unit.to_string(),
                model: model.to_string(),
                serial: serial.to_string(),
                controller,
                fmeasure,
                fsetpoint: fmeasure,
                temperature: 21.5,
                alarm_info: 0,
                valve_output,
                silent: false,
                corrupt: false,
            },
        );
        self.refresh();
    }

    pub fn set_measure(&mut self, node: u8, value: f32) -> &mut Self {
        if let Some(n) = self.nodes.get_mut(&node) {
            n.fmeasure = value;
        }
        self.refresh();
        self
    }

    pub fn set_alarm_info(&mut self, node: u8, bits: u8) -> &mut Self {
        if let Some(n) = self.nodes.get_mut(&node) {
            n.alarm_info = bits;
        }
        self.refresh();
        self
    }

    /// 無応答(タイムアウト相当)。
    pub fn set_silent(&mut self, node: u8, silent: bool) -> &mut Self {
        if let Some(n) = self.nodes.get_mut(&node) {
            n.silent = silent;
        }
        self.refresh();
        self
    }

    /// 計測応答だけ壊す(length 不整合 → `ProParError::LengthMismatch`)。
    pub fn corrupt(&mut self, node: u8, corrupt: bool) -> &mut Self {
        if let Some(n) = self.nodes.get_mut(&node) {
            n.corrupt = corrupt;
        }
        self.refresh();
        self
    }

    /// 応答表を**丸ごと**組み直す(古い silent 指定・古い値が残らない)。
    fn refresh(&self) {
        let mut fake = FakeProPar::new();
        for (node, n) in &self.nodes {
            if n.silent {
                fake.set_silent(*node);
                continue;
            }
            let ident = ident_request(*node);
            fake.set_response(
                ident.to_ascii_line(),
                encode_reply(
                    *node,
                    &ident,
                    &[
                        // Identification number: 9 DEPC(制御器)/ 10 DEPM(計測器)
                        ReplyValue::U8(if n.controller { 9 } else { 10 }),
                        ReplyValue::Str(n.model.clone()),
                        ReplyValue::Str(n.serial.clone()),
                        ReplyValue::Str("V6.07".to_string()),
                        ReplyValue::Str("FAKE".to_string()),
                    ],
                ),
            );
            let capacity = capacity_request(*node);
            fake.set_response(
                capacity.to_ascii_line(),
                encode_reply(
                    *node,
                    &capacity,
                    &[
                        ReplyValue::F32(200.0),
                        ReplyValue::Str(n.unit.clone()),
                        ReplyValue::Str("CO2".to_string()),
                        // Sensor type(1/14): bit7 = センサ(計測のみ)
                        ReplyValue::U8(if n.controller { 0 } else { 128 }),
                    ],
                ),
            );
            let measure = measure_request_for(*node, n.controller);
            let reply = if n.corrupt {
                // length は 0xFF と名乗るのに中身は 2 バイト = LengthMismatch。
                ":FF0302\r\n".to_string()
            } else {
                let mut values = vec![
                    ReplyValue::F32(n.fmeasure),
                    ReplyValue::F32(n.fsetpoint),
                    ReplyValue::F32(n.temperature),
                    ReplyValue::U8(n.alarm_info),
                ];
                if n.controller {
                    values.push(ReplyValue::U32(n.valve_output));
                }
                encode_reply(*node, &measure, &values)
            };
            fake.set_response(measure.to_ascii_line(), reply);
        }
        // 応答表の差し替え。exchange の途中では呼ばれない(refresh は poll の外側)。
        if let Ok(mut guard) = self.shared.0.lock() {
            *guard = fake;
        }
    }
}

// ---------------------------------------------------------------------
// FakeDevice(component から `type = "fake"` で作られる Driver)
// ---------------------------------------------------------------------

/// スクリプト 1 行分。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    /// fMeasure にこの値を載せる。
    Value(f64),
    /// この周期は無応答(タイムアウト)。
    Silent,
}

/// スクリプトファイルを 1 行ずつ [`Step`] にする。`#` 以降と空行は無視。
pub fn parse_script(text: &str) -> Result<Vec<Step>, String> {
    let mut steps = Vec::new();
    for (line_no, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.eq_ignore_ascii_case("silent") {
            steps.push(Step::Silent);
            continue;
        }
        let value: f64 = line
            .parse()
            .map_err(|e| format!("line {}: {line:?} is not a number ({e})", line_no + 1))?;
        steps.push(Step::Value(value));
    }
    if steps.is_empty() {
        return Err("script has no steps".to_string());
    }
    Ok(steps)
}

/// テストから時系列を注入できる偽デバイス。中身は本物の Bronkhorst ドライバ。
pub struct FakeDevice {
    bus: FakeBus,
    driver: BronkhorstDriver<SharedFake>,
    script: Vec<Step>,
    cursor: usize,
    node: u8,
}

/// fake が名乗るノード番号(FLOW-BUS の既定アドレス)。
pub const FAKE_NODE: u8 = 3;
/// fake が返す単位文字列。
pub const FAKE_UNIT: &str = "mbar";

impl FakeDevice {
    /// スクリプトファイルから作る(`type = "fake"` の `port` がこのパス)。
    pub fn from_script(device: &str, path: &Path) -> Result<Self, DriverError> {
        let text = std::fs::read_to_string(path)?;
        let script = parse_script(&text).map_err(DriverError::Protocol)?;
        Ok(Self::from_steps(device, script))
    }

    /// スクリプトを直接渡す版(単体テスト用)。
    pub fn from_steps(device: &str, script: Vec<Step>) -> Self {
        let mut bus = FakeBus::new();
        bus.add_meter(FAKE_NODE, FAKE_UNIT, "FAKE-EL-PRESS", "FAKE-0001", 0.0);
        let driver = BronkhorstDriver::new(
            device,
            bus.io(),
            NodesSpec::List(vec![FAKE_NODE]),
            String::new(),
        );
        Self {
            bus,
            driver,
            script,
            cursor: 0,
            node: FAKE_NODE,
        }
    }

    /// 次の 1 行を適用する(尽きたら最後の行を保持し続ける)。
    fn advance(&mut self) {
        let index = self.cursor.min(self.script.len() - 1);
        self.cursor += 1;
        match self.script[index] {
            Step::Value(value) => {
                self.bus.set_silent(self.node, false);
                self.bus.set_measure(self.node, value as f32);
            }
            Step::Silent => {
                self.bus.set_silent(self.node, true);
            }
        }
    }
}

impl Driver for FakeDevice {
    fn ident(&mut self) -> Result<Vec<Ident>, DriverError> {
        self.driver.ident()
    }

    fn poll(&mut self) -> Result<Vec<Sample>, DriverError> {
        self.advance();
        self.driver.poll()
    }
}

// ---------------------------------------------------------------------
// テスト
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_script_parser_reads_values_comments_and_silence() {
        let steps = parse_script("100.0\n# comment\n\n silent \n98.5 # trailing\n").unwrap();
        assert_eq!(
            steps,
            vec![
                Step::Value(100.0),
                Step::Silent,
                Step::Value(98.5),
                // 空行・コメントは Step にならない
            ]
        );
        assert!(parse_script("").is_err(), "空のスクリプトは設定ミス");
        assert!(parse_script("oops").is_err());
    }

    /// スクリプトの時系列がそのまま fmeasure に出て、`silent` は Timeout になる。
    #[test]
    fn the_fake_device_replays_its_script_step_by_step() {
        let mut device = FakeDevice::from_steps(
            "gas",
            vec![Step::Value(100.0), Step::Silent, Step::Value(120.0)],
        );
        let idents = device.ident().unwrap();
        assert_eq!(idents.len(), 1);
        assert_eq!(idents[0].fields["capacity_unit"], FAKE_UNIT);

        let first = device.poll().unwrap();
        assert_eq!(first[0].param, "fmeasure");
        assert_eq!(first[0].value, 100.0);

        assert!(matches!(device.poll(), Err(DriverError::Timeout)));

        let third = device.poll().unwrap();
        assert_eq!(third[0].value, 120.0);

        // 尽きたら最後の行を保持(統合テストが「復帰後も読み続ける」のを見られる)。
        assert_eq!(device.poll().unwrap()[0].value, 120.0);
    }
}
