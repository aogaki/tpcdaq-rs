//! Bronkhorst FLOW-BUS(ProPar)ドライバ —— [`Driver`] 実装(TODO/079 §2.2 / §2.6)。
//!
//! Phase 1 の [`ProParLink`](super::link::ProParLink) にシリアルポートを差しただけのもの。
//! **書き込みは型として存在しない**(Phase 1 と同じ。TODO/079 §1.5 の安全境界)。
//!
//! # トランスポートがジェネリックな理由
//!
//! 実機は `ProParLink<Box<dyn serialport::SerialPort>>`(発注書 §2.6-2)だが、テストは
//! 同じドライバに [`FakeProPar`](super::fake::FakeProPar) を差して回す(§2.6-3「別の偽物を
//! 作らない」)。`T: Read + Write` はそのための 1 個のパラメータであって、抽象化の先回り
//! ではない。

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::time::Duration;

use tracing::{info, warn};

use super::link::{LinkError, ProParLink};
use super::propar::{capacity_request, ident_request, ParType, ReadItem, Request, Value};
use super::{now_ms, Driver, DriverError, Ident, NodesSpec, Sample};

/// ProPar の読み出しタイムアウト(ミリ秒)。9.17.027 の 38400 8N1 で 1 交換は数 ms なので
/// 桁で余裕を取る。無応答ノードの scan がここで刻まれる(120 ノード全走査 = 最悪 ~24 s)。
pub const READ_TIMEOUT_MS: u64 = 200;

/// ProPar ASCII の通信速度(9.17.027 §3.1: RS232 は 38400 8N1 固定)。
pub const BAUD_RATE: u32 = 38_400;

/// 1 ノードから読む計測パラメータの名前(logbook / status の `param`)。
pub const PARAM_FMEASURE: &str = "fmeasure";
pub const PARAM_FSETPOINT: &str = "fsetpoint";
pub const PARAM_TEMPERATURE: &str = "temperature";
pub const PARAM_ALARM_INFO: &str = "alarm_info";
pub const PARAM_VALVE_OUTPUT: &str = "valve_output";

/// 温度の単位(9.17.023 の 33/7 は °C 固定 —— 機器は単位文字列を返さない)。
pub const UNIT_DEGC: &str = "degC";
/// バルブ開度の単位。生の u32 をそのまま載せる(% への換算式は現地で確定 —— 発注書 §2.6-2)。
pub const UNIT_RAW: &str = "raw";
/// Alarm info(1/20)のビットマスクをそのまま載せるときの単位。
pub const UNIT_BITMASK: &str = "bitmask";

/// scan で見つかったノード 1 台の静的情報(`ident` で確定し `poll` が使う)。
#[derive(Debug, Clone, PartialEq, Eq)]
struct NodeInfo {
    node: u8,
    /// Capacity unit(1/31)の文字列。fmeasure / fsetpoint の単位はこれ(焼き込み禁止)。
    unit: String,
    /// バルブを持つ = 制御器。9.17.023 の Sensor type(1/14)は **bit7 = センサ(計測のみ)**
    /// (TODO/079 §2.2 の表「0 圧力(制御器)/ 128 圧力(センサ)」)。
    controller: bool,
}

/// Bronkhorst ProPar ドライバ。
pub struct BronkhorstDriver<T: Read + Write> {
    device: String,
    link: ProParLink<T>,
    nodes: NodesSpec,
    /// 空でなければ `ident` で 113/3(Serial number)と照合する(TODO/079 §2.1)。
    expect_serial: String,
    found: Vec<NodeInfo>,
}

impl<T: Read + Write> BronkhorstDriver<T> {
    pub fn new(device: impl Into<String>, io: T, nodes: NodesSpec, expect_serial: String) -> Self {
        Self {
            device: device.into(),
            link: ProParLink::new(io),
            nodes,
            expect_serial,
            found: Vec::new(),
        }
    }

    /// 直近のリンク統計(`ok / timeout / malformed / node_mismatch`)。
    pub fn stats(&self) -> super::link::LinkStats {
        self.link.stats
    }

    /// `ident` が確定したノード数(テストと起動ログ用)。
    pub fn node_count(&self) -> usize {
        self.found.len()
    }
}

/// 1 ノード分の計測要求(発注書 §2.6-2: 33/0・33/3・33/7・1/20、制御器なら 114/1)。
///
/// fake(テスト)側も**この関数が作った行に対して**応答を用意する。要求の組み立てを
/// 2 箇所に書くと fake が静かにズレるので、1 箇所に閉じる。
pub(crate) fn measure_request_for(node: u8, controller: bool) -> Request {
    let mut items = vec![
        ReadItem {
            process: 33,
            param: 0,
            ty: ParType::F32,
        }, // fMeasure
        ReadItem {
            process: 33,
            param: 3,
            ty: ParType::F32,
        }, // fSetpoint(読むだけ)
        ReadItem {
            process: 33,
            param: 7,
            ty: ParType::F32,
        }, // Temperature
        ReadItem {
            process: 1,
            param: 20,
            ty: ParType::U8,
        }, // Alarm info
    ];
    if controller {
        items.push(ReadItem {
            process: 114,
            param: 1,
            ty: ParType::U32,
        }); // Valve output
    }
    Request { node, items }
}

/// `LinkError` → `DriverError`。**区別を潰さない**(コンポーネントがカウンタを振り分ける)。
fn driver_error(error: LinkError) -> DriverError {
    match error {
        LinkError::Io(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock
                    | std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::UnexpectedEof
            ) =>
        {
            DriverError::Timeout
        }
        LinkError::Io(e) => DriverError::Io(e),
        LinkError::Protocol(e) => DriverError::Protocol(e.to_string()),
    }
}

/// 「要求した型と違う値が返った」= プロトコル違反(復号は型通りなので本来起きない。
/// silent に飛ばさず `Err` にする — CLAUDE.md)。
fn shape_error(node: u8, what: &str) -> DriverError {
    DriverError::Protocol(format!(
        "node {node}: {what} reply does not have the requested shape"
    ))
}

impl<T: Read + Write> Driver for BronkhorstDriver<T> {
    fn ident(&mut self) -> Result<Vec<Ident>, DriverError> {
        let nodes = self.nodes.resolve().map_err(DriverError::Protocol)?;
        let mut idents = Vec::new();
        let mut found = Vec::new();

        for node in nodes {
            // 無応答ノードは scan の正常な結果。link.stats.timeout に計上済み(silent 禁止)。
            let Ok(reply) = self.link.exchange(&ident_request(node)) else {
                continue;
            };
            let [Value::U8(identification), Value::Str(model), Value::Str(serial), Value::Str(firmware), Value::Str(user_tag)] =
                reply.values.as_slice()
            else {
                return Err(shape_error(node, "ident"));
            };

            if !self.expect_serial.is_empty() && self.expect_serial != *serial {
                return Err(DriverError::Identity {
                    expected: self.expect_serial.clone(),
                    got: serial.clone(),
                });
            }

            let capacity = self
                .link
                .exchange(&capacity_request(node))
                .map_err(driver_error)?;
            let [Value::F32(capacity_100), Value::Str(unit), Value::Str(fluid), Value::U8(sensor_type)] =
                capacity.values.as_slice()
            else {
                return Err(shape_error(node, "capacity"));
            };

            let controller = sensor_type & 0x80 == 0;
            let mut fields = BTreeMap::new();
            fields.insert(
                "identification_number".to_string(),
                identification.to_string(),
            );
            fields.insert("model".to_string(), model.clone());
            fields.insert("serial".to_string(), serial.clone());
            fields.insert("firmware".to_string(), firmware.clone());
            fields.insert("user_tag".to_string(), user_tag.clone());
            fields.insert("capacity_100".to_string(), capacity_100.to_string());
            fields.insert("capacity_unit".to_string(), unit.clone());
            fields.insert("fluid_name".to_string(), fluid.clone());
            fields.insert("sensor_type".to_string(), sensor_type.to_string());

            info!(
                device = self.device,
                node, model, serial, unit, controller, "slowctl: bronkhorst node identified"
            );
            idents.push(Ident {
                device: self.device.clone(),
                node: Some(node),
                fields,
            });
            found.push(NodeInfo {
                node,
                unit: unit.clone(),
                controller,
            });
        }

        if found.is_empty() {
            warn!(device = self.device, stats = %self.link.stats,
                  "slowctl: bronkhorst node scan found nothing");
        }
        self.found = found;
        Ok(idents)
    }

    fn poll(&mut self) -> Result<Vec<Sample>, DriverError> {
        // `found` を借りたまま `self.link` を可変で使えないので、要求だけ先に作る
        // (ノード数は高々 120 で、ホットパスでもないので素直にやる)。
        let requests: Vec<(NodeInfo, Request)> = self
            .found
            .iter()
            .map(|info| {
                (
                    info.clone(),
                    measure_request_for(info.node, info.controller),
                )
            })
            .collect();

        let ts_ms = now_ms();
        let mut samples = Vec::with_capacity(requests.len() * 5);
        for (info, request) in requests {
            let reply = self.link.exchange(&request).map_err(driver_error)?;
            let (
                [Value::F32(fmeasure), Value::F32(fsetpoint), Value::F32(temperature), Value::U8(alarm_info)],
                valve,
            ) = (
                &reply.values[..4.min(reply.values.len())],
                reply.values.get(4),
            )
            else {
                return Err(shape_error(info.node, "measure"));
            };
            let alarm = *alarm_info != 0;

            let mut push = |param: &str, value: f64, unit: &str| {
                samples.push(Sample {
                    device: self.device.clone(),
                    node: Some(info.node),
                    param: param.to_string(),
                    value,
                    unit: unit.to_string(),
                    ts_ms,
                    alarm,
                });
            };
            push(PARAM_FMEASURE, f64::from(*fmeasure), &info.unit);
            push(PARAM_FSETPOINT, f64::from(*fsetpoint), &info.unit);
            push(PARAM_TEMPERATURE, f64::from(*temperature), UNIT_DEGC);
            push(PARAM_ALARM_INFO, f64::from(*alarm_info), UNIT_BITMASK);
            if let Some(Value::U32(valve_output)) = valve {
                push(PARAM_VALVE_OUTPUT, f64::from(*valve_output), UNIT_RAW);
            } else if valve.is_some() {
                return Err(shape_error(info.node, "valve output"));
            }
        }
        Ok(samples)
    }
}

// ---------------------------------------------------------------------
// 実シリアルポート版の組み立て(唯一 IO に触る場所)
// ---------------------------------------------------------------------

/// `/dev/serial/by-id/…` を開いて ProPar ドライバを組む(TODO/079 §2.6-2)。
///
/// `serialport` は `default-features = false`(libudev 不要 —— by-id の解決は OS の
/// symlink がやる)。ProPar は 38400 8N1(9.17.027 §3.1)。
pub fn open_serial(
    device: &str,
    port: &str,
    nodes: NodesSpec,
    expect_serial: String,
) -> Result<BronkhorstDriver<Box<dyn serialport::SerialPort>>, DriverError> {
    let handle = serialport::new(port, BAUD_RATE)
        .data_bits(serialport::DataBits::Eight)
        .parity(serialport::Parity::None)
        .stop_bits(serialport::StopBits::One)
        .timeout(Duration::from_millis(READ_TIMEOUT_MS))
        .open()
        .map_err(|e| DriverError::Protocol(format!("cannot open serial port {port}: {e}")))?;
    info!(device, port, baud = BAUD_RATE, "slowctl: serial port open");
    Ok(BronkhorstDriver::new(device, handle, nodes, expect_serial))
}

// ---------------------------------------------------------------------
// テスト
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::slowctl::fake_device::{encode_reply, FakeBus, ReplyValue};
    use crate::slowctl::propar::ProParError;

    /// 発注書 §2.6-2: `nodes = "scan"` は 128 → 3..=120 の順。
    #[test]
    fn scan_starts_at_the_rs232_head_node_then_walks_the_flow_bus() {
        let nodes = NodesSpec::default().resolve().unwrap();
        assert_eq!(
            nodes[0], 128,
            "RS232 直結の先頭機器は node 128(9.17.027 §3.2)"
        );
        assert_eq!(nodes[1], 3, "FLOW-BUS のノードアドレスは 3 から");
        assert_eq!(nodes[nodes.len() - 1], 120);
        assert_eq!(nodes.len(), 1 + (120 - 3 + 1));

        let explicit = NodesSpec::List(vec![3, 4]).resolve().unwrap();
        assert_eq!(explicit, vec![3, 4]);
        assert!(NodesSpec::Keyword("all".to_string()).resolve().is_err());
    }

    /// ident は識別 5 項目 + capacity 4 項目を 1 ノードにまとめ、無応答ノードは飛ばす。
    #[test]
    fn ident_collects_both_groups_and_skips_silent_nodes() {
        let mut bus = FakeBus::new();
        bus.add_meter(3, "mbar", "F-201CV", "M11200001A", 130.0);
        // node 4 は登録しない = 無応答

        let mut driver =
            BronkhorstDriver::new("gas", bus.io(), NodesSpec::List(vec![3, 4]), String::new());
        let idents = driver.ident().unwrap();

        assert_eq!(idents.len(), 1, "応答したノードだけ");
        assert_eq!(idents[0].device, "gas");
        assert_eq!(idents[0].node, Some(3));
        assert_eq!(idents[0].fields["model"], "F-201CV");
        assert_eq!(idents[0].fields["serial"], "M11200001A");
        assert_eq!(idents[0].fields["capacity_unit"], "mbar");
        assert_eq!(
            idents[0].fields["sensor_type"], "128",
            "128 = センサ(計測のみ)"
        );
        // 無応答は数えて可視化(silent 禁止)。ident + capacity で ok=2、timeout=1。
        assert_eq!(driver.stats().ok, 2);
        assert_eq!(driver.stats().timeout, 1);
    }

    /// `expect_serial` 不一致は `DriverError::Identity`(TODO/079 §2.1 のポート取り違え対策)。
    #[test]
    fn a_wrong_serial_number_is_an_identity_error() {
        let mut bus = FakeBus::new();
        bus.add_meter(3, "mbar", "F-201CV", "M11200001A", 130.0);
        let mut driver = BronkhorstDriver::new(
            "gas",
            bus.io(),
            NodesSpec::List(vec![3]),
            "OTHER-SERIAL".to_string(),
        );
        assert!(matches!(
            driver.ident(),
            Err(DriverError::Identity { expected, got })
                if expected == "OTHER-SERIAL" && got == "M11200001A"
        ));
    }

    /// 計測のみの機器はバルブ開度を読まない(4 サンプル)。単位は 1/31 の文字列そのまま。
    #[test]
    fn poll_of_a_meter_yields_four_samples_with_the_device_unit() {
        let mut bus = FakeBus::new();
        bus.add_meter(3, "mbar", "F-201CV", "M11200001A", 130.5);
        let mut driver =
            BronkhorstDriver::new("gas", bus.io(), NodesSpec::List(vec![3]), String::new());
        driver.ident().unwrap();

        let samples = driver.poll().unwrap();
        let names: Vec<&str> = samples.iter().map(|s| s.param.as_str()).collect();
        assert_eq!(
            names,
            vec!["fmeasure", "fsetpoint", "temperature", "alarm_info"]
        );
        assert_eq!(samples[0].value, 130.5);
        assert_eq!(samples[0].unit, "mbar", "単位は機器から読んだ文字列");
        assert_eq!(samples[2].unit, "degC");
        assert_eq!(samples[3].unit, "bitmask");
        assert!(samples.iter().all(|s| !s.alarm), "Alarm info = 0");
        assert_eq!(samples[0].key(), "gas/3/fmeasure");
    }

    /// 制御器はバルブ開度も読む(5 サンプル、生値 + unit "raw")。
    #[test]
    fn poll_of_a_controller_also_reads_the_valve_output() {
        let mut bus = FakeBus::new();
        bus.add_controller(4, "mln/min", "F-201CV", "M11200002B", 12.5, 0x1234_5678);
        let mut driver =
            BronkhorstDriver::new("gas", bus.io(), NodesSpec::List(vec![4]), String::new());
        driver.ident().unwrap();

        let samples = driver.poll().unwrap();
        assert_eq!(samples.len(), 5);
        assert_eq!(samples[4].param, "valve_output");
        // 生 u32 をそのまま。0x12345678 = 305419896(手計算)。
        assert_eq!(samples[4].value, 305_419_896.0);
        assert_eq!(samples[4].unit, "raw");
    }

    /// 機器側 Alarm info(1/20)≠ 0 はそのノードの全サンプルに `alarm` を立てる。
    #[test]
    fn a_nonzero_alarm_info_marks_every_sample_of_that_node() {
        let mut bus = FakeBus::new();
        bus.add_meter(3, "mbar", "F-201CV", "S1", 130.0);
        bus.set_alarm_info(3, 0x04); // min alarm(9.17.023)
        let mut driver =
            BronkhorstDriver::new("gas", bus.io(), NodesSpec::List(vec![3]), String::new());
        driver.ident().unwrap();

        let samples = driver.poll().unwrap();
        assert!(samples.iter().all(|s| s.alarm));
        assert_eq!(samples[3].value, 4.0, "alarm_info の生ビットも残す");
    }

    /// 無応答は `Timeout`、壊れた応答は `Protocol` —— コンポーネントのカウンタ振り分けの根拠。
    #[test]
    fn a_silent_node_times_out_and_a_broken_reply_is_a_protocol_error() {
        let mut bus = FakeBus::new();
        bus.add_meter(3, "mbar", "F-201CV", "S1", 130.0);
        let mut driver =
            BronkhorstDriver::new("gas", bus.io(), NodesSpec::List(vec![3]), String::new());
        driver.ident().unwrap();

        bus.set_silent(3, true);
        assert!(matches!(driver.poll(), Err(DriverError::Timeout)));

        bus.set_silent(3, false);
        bus.corrupt(3, true);
        assert!(matches!(driver.poll(), Err(DriverError::Protocol(_))));

        bus.corrupt(3, false);
        assert_eq!(driver.poll().unwrap().len(), 4, "復帰したら読み出しも戻る");
    }

    /// `encode_reply` が Phase 1 の `parse_reply` と噛み合うこと(fake の自己検査)。
    #[test]
    fn the_fake_reply_encoder_round_trips_through_the_phase_1_decoder() {
        let request = Request {
            node: 3,
            items: vec![
                ReadItem {
                    process: 33,
                    param: 0,
                    ty: ParType::F32,
                },
                ReadItem {
                    process: 1,
                    param: 20,
                    ty: ParType::U8,
                },
            ],
        };
        let line = encode_reply(3, &request, &[ReplyValue::F32(130.5), ReplyValue::U8(4)]);
        let reply = crate::slowctl::propar::parse_reply(&line, &request).unwrap();
        assert_eq!(reply.values, vec![Value::F32(130.5), Value::U8(4)]);
    }

    /// `ProParError` は `Protocol` に、IO は `Io`/`Timeout` に落ちる(区別を潰さない)。
    #[test]
    fn link_errors_keep_their_kind_when_they_become_driver_errors() {
        assert!(matches!(
            driver_error(LinkError::Protocol(ProParError::Framing)),
            DriverError::Protocol(_)
        ));
        assert!(matches!(
            driver_error(LinkError::Io(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "x"
            ))),
            DriverError::Timeout
        ));
        assert!(matches!(
            driver_error(LinkError::Io(std::io::Error::other("x"))),
            DriverError::Io(_)
        ));
    }
}
