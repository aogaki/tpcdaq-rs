//! スローコントロール(ガス系 Bronkhorst FLOW-BUS / 環境 Arduino、SPEC §1.1、旧 psu)。
//!
//! **Phase 1(このユニット、TODO/079 §2.5)の範囲 = ProPar ASCII コーデックと同期リンクのみ。**
//! シリアルポート配線・tokio・TOML 設定・component 本体・logbook 出力は Phase 2 以降。
//! 安全境界は SPEC §11 に従う: 監視・記録・アラームが本分で、**書けるのは HV / LV の ch ごとの
//! ON / OFF だけ**(v1.42 の唯一の例外、TODO/124 —— [`Driver::set_power`])。

/// ProPar ASCII コーデック(純関数、IO なし)。Bronkhorst 9.17.027 §3.3–§3.9。
pub mod propar;

/// 同期 ProPar リンク(`Read + Write` ジェネリック)。9.17.027 §3.2/§3.7。
pub mod link;

/// インメモリ ProPar 応答器(テスト専用、または `slowctl-fake` feature)。
#[cfg(any(test, feature = "slowctl-fake"))]
pub mod fake;

/// CAEN DT1415ET 読み出しコーデック(純関数、IO なし)。TODO/085。SET は ch の ON/OFF だけ(TODO/124)。
pub mod hivolta;

/// R&S/HAMEG HMP2020 読み出しコーデック(純関数、IO なし)。TODO/085。設定系は OUTP ON/OFF だけ(TODO/124)。
pub mod hmp;

/// インメモリ HiVolta 応答器(テスト専用、または `slowctl-fake` feature)。TODO/085。
#[cfg(any(test, feature = "slowctl-fake"))]
pub mod fake_hv;

/// インメモリ HMP2020 応答器(テスト専用、または `slowctl-fake` feature)。TODO/085。
#[cfg(any(test, feature = "slowctl-fake"))]
pub mod fake_hmp;

// =====================================================================
// Phase 2(TODO/079 §2.6)—— ドメイン型 + `Driver` 抽象
// =====================================================================
//
// ここに置くのは **IO にも tokio にも依存しない型だけ**(CLAUDE.md の Clean
// Architecture: ドメイン核は IO に依存しない)。シリアルポートを掴むのは
// [`bronkhorst`]、ZMQ とスレッドを掴むのは [`component`]。

/// Bronkhorst FLOW-BUS(ProPar)ドライバ。`Driver` 実装。
pub mod bronkhorst;

/// slowctl コンポーネント本体(デバイススレッド + status PUB + LogPost)。
pub mod component;

/// 全サンプルを InfluxDB v2 write API へ書く writer スレッド(TODO/122)。
pub mod influx;

/// テスト用の偽デバイス(= [`fake::FakeProPar`] を噛ませた [`bronkhorst`] ドライバ)。
#[cfg(any(test, feature = "slowctl-fake"))]
pub mod fake_device;

/// Arduino MKR ENV Shield ドライバ(`Driver` 実装)。TODO/079 §2.3 / §2.7。
pub mod arduino;

/// インメモリ Arduino ENV 応答器(テスト専用、または `slowctl-fake` feature)。TODO/079 §2.7。
#[cfg(any(test, feature = "slowctl-fake"))]
pub mod fake_arduino;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// SPEC §2.3 の slowctl → monitor PUB ペイロードの `kind`。
pub const SLOW_KIND: &str = "slow";

/// `node` が無いデバイス(Arduino 等)を表すキー要素(SPEC §9.2「node 無しは `-`」)。
pub const NODELESS: &str = "-";

/// 壁時計の unix ミリ秒。`Sample::ts_ms` / `SlowPayload::ts_ms` の唯一の出所。
pub fn now_ms() -> u64 {
    // 1970 より前は表現しない(u64 へ落とす)。負になるのは時計が壊れている場合だけで、
    // そのときは 0 として記録に残す(silent に別の値を捏造しない)。
    u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or(0)
}

/// 1 計測値(SPEC §2.3 `samples[]` の要素、SPEC §9.2 `slow.values` の材料)。
///
/// `unit` は**機器が返した単位文字列そのまま**(SPEC §9.2)。コードで単位を決め打ちしない。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    /// TOML の `[[slowctl.device]] name`。
    pub device: String,
    /// 機器内アドレス(FLOW-BUS ノード番号 / HV ch 番号)。無ければ `None`(SPEC §9.2 の null)。
    pub node: Option<u8>,
    pub param: String,
    pub value: f64,
    pub unit: String,
    pub ts_ms: u64,
    /// 機器側 Alarm info(ProPar 1/20)≠ 0。`[lo, hi]` 逸脱は [`AlarmRule`] が別に見る。
    pub alarm: bool,
}

impl Sample {
    /// 直近値マップのキー `"<device>/<node>/<param>"`(SPEC §9.2、node 無しは `"-"`)。
    pub fn key(&self) -> String {
        latest_key(&self.device, self.node, &self.param)
    }
}

/// [`Sample::key`] と同じ組み立て(controller 側が Sample を持たずにキーを作るため)。
pub fn latest_key(device: &str, node: Option<u8>, param: &str) -> String {
    match node {
        Some(node) => format!("{device}/{node}/{param}"),
        None => format!("{device}/{NODELESS}/{param}"),
    }
}

/// 起動時の識別群(logbook `slow` の `event: "IDENT"`、TODO/079 §2.2)。
///
/// `fields` は文字列マップ(機種・シリアル・ファーム・単位…)。機器ごとに項目が違うので
/// 型を切らずに名前で持つ —— logbook の `values` にそのまま入る形(SPEC §9.2)。
/// `BTreeMap` は JSON のキー順を決定的にするため(手組みオラクルが書ける)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ident {
    pub device: String,
    pub node: Option<u8>,
    pub fields: BTreeMap<String, String>,
}

/// ドライバの失敗要因(TODO/079 §2.6)。silent failure を作らないため、
/// コンポーネントはこの区別でカウンタを振り分ける(Timeout/Io = `poll_err`、
/// Protocol = `parse_err`、Identity = `identity_mismatch`)。
#[derive(Debug, thiserror::Error)]
pub enum DriverError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("device did not answer in time")]
    Timeout,

    #[error("protocol error: {0}")]
    Protocol(String),

    /// open 時の名乗り照合に失敗(TODO/079 §2.1 のポート取り違え対策)。
    #[error("device identity mismatch: expected {expected}, got {got}")]
    Identity { expected: String, got: String },

    /// この機器に power ON/OFF は無い([`Driver::set_power`] の既定、TODO/124)。
    #[error("this device has no power on/off")]
    Unsupported,
}

/// [`Driver::set_power`] の結果(TODO/124、SPEC §11 v1.42)。
///
/// `before` / `after` は slowctl の REP 応答にそのまま載る JSON
/// (HV = `{"status": 33, "flags": ["ON", "UNV"]}`、LV = `{"outp": {"1": 1, "2": 0}}`)。
/// `before` を読む前に失敗したときは `Err(DriverError)`(応答の `before` は null)。
#[derive(Debug, Clone, PartialEq)]
pub struct PowerOutcome {
    /// SET 前の状態。
    pub before: serde_json::Value,
    /// `Ok(after)` = 切り替わった。`Err` = 機器が拒否した / 切り替わらなかった /
    /// SET 後の読み戻しに失敗した。
    pub result: Result<serde_json::Value, PowerRefused>,
}

/// [`PowerOutcome::result`] の失敗側(TODO/124)。
#[derive(Debug, Clone, PartialEq)]
pub struct PowerRefused {
    /// そのまま応答の `error` になる理由。
    pub error: String,
    /// SET 後に状態を読めていれば、その状態(応答の `after` に載る —— 「切り替わらなかった」の
    /// 中身を隠さない、レビュー 2026-10-12)。SET を送っていない・読めなかったなら `None`。
    pub after: Option<serde_json::Value>,
}

impl PowerRefused {
    /// SET 後の状態が無い拒否(DIS・LOCAL・送信失敗など)。
    pub fn without_after(error: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            after: None,
        }
    }
}

/// スロー制御デバイス 1 台。**同期(ブロッキング)**——呼び出し側が専用スレッドを持つ。
///
/// **書き込みは [`Driver::set_power`](ch の ON / OFF)だけ**(SPEC §11 安全境界 v1.42 の
/// 唯一の例外、TODO/124)。電圧値・電流上限・ランプ・setpoint・バルブを書く経路は型として作らない。
pub trait Driver {
    /// 起動時の識別(node scan 含む)。以後 [`Driver::poll`] が返すノード集合を確定する。
    fn ident(&mut self) -> Result<Vec<Ident>, DriverError>;

    /// 1 周期分の計測値。
    fn poll(&mut self) -> Result<Vec<Sample>, DriverError>;

    /// ch(`node`)の ON / OFF(TODO/124)。実装するのは HV(DT1415ET)と LV(HMP2020)だけ。
    /// 既定は [`DriverError::Unsupported`]。
    fn set_power(&mut self, _node: u8, _on: bool) -> Result<PowerOutcome, DriverError> {
        Err(DriverError::Unsupported)
    }
}

/// [`AlarmRule::evaluate`] の遷移(遷移が無ければ `None`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlarmEvent {
    Alarm,
    Recover,
}

impl AlarmEvent {
    /// logbook `slow.event` の文字列(SPEC §9.2)。
    pub fn as_str(self) -> &'static str {
        match self {
            AlarmEvent::Alarm => "ALARM",
            AlarmEvent::Recover => "RECOVER",
        }
    }
}

/// `[lo, hi]` 逸脱の監視規則(TOML `alarms = [{ node, param, lo, hi }]`、SPEC §3.1)。
///
/// **DAQ は止めない**(TODO/079 §3-3 の既定)—— 記録とバッジだけ。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlarmRule {
    /// 省略 = どのノードにも当たる。
    #[serde(default)]
    pub node: Option<u8>,
    pub param: String,
    pub lo: f64,
    pub hi: f64,
}

impl AlarmRule {
    /// この規則が `sample` を見るか(node と param の一致)。
    pub fn matches(&self, sample: &Sample) -> bool {
        self.param == sample.param && (self.node.is_none() || self.node == sample.node)
    }

    /// `[lo, hi]` は両端を含む —— 外れていれば true。
    pub fn is_out_of_range(&self, value: f64) -> bool {
        value < self.lo || value > self.hi
    }

    /// 前回状態 + サンプル → 遷移(純関数)。`was_alarm` はこの規則の前回の判定。
    pub fn evaluate(&self, was_alarm: bool, sample: &Sample) -> Option<AlarmEvent> {
        if !self.matches(sample) {
            return None;
        }
        match (was_alarm, self.is_out_of_range(sample.value)) {
            (false, true) => Some(AlarmEvent::Alarm),
            (true, false) => Some(AlarmEvent::Recover),
            _ => None,
        }
    }
}

/// TOML の `nodes`(SPEC §3.1): `"scan"` か明示リスト。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum NodesSpec {
    /// `"scan"` 以外の文字列は [`NodesSpec::resolve`] が拒否する(silent に scan へ倒さない)。
    Keyword(String),
    List(Vec<u8>),
}

impl NodesSpec {
    /// FLOW-BUS の走査範囲(TODO/079 §1.2 / §2.2: RS232 直結の先頭機器 = 128、他は 3…120)。
    pub fn scan_nodes() -> Vec<u8> {
        std::iter::once(128u8).chain(3u8..=120u8).collect()
    }

    /// 実際に ping するノード列。未知のキーワードは `Err`(設定ミスを黙って握らない)。
    pub fn resolve(&self) -> Result<Vec<u8>, String> {
        match self {
            NodesSpec::Keyword(word) if word == "scan" => Ok(Self::scan_nodes()),
            NodesSpec::Keyword(word) => Err(format!(
                "nodes must be \"scan\" or a list of node numbers, got {word:?}"
            )),
            NodesSpec::List(nodes) => Ok(nodes.clone()),
        }
    }
}

impl Default for NodesSpec {
    fn default() -> Self {
        NodesSpec::Keyword("scan".to_string())
    }
}

/// VALUE を logbook に書くかどうかの規則(SPEC §9.2「1 Hz の全サンプルは書かない」)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogPolicy {
    /// 値が動かなくてもこの間隔で 1 本書く(秒)。
    pub log_every_s: u64,
}

/// 相対 2 %(SPEC §9.2)。
const LOG_RELATIVE_THRESHOLD: f64 = 0.02;
/// 0 近傍で相対判定が効かなくなるときの絶対下限(TODO/079 §2.6)。
const LOG_ABSOLUTE_FLOOR: f64 = 1e-9;

impl LogPolicy {
    /// `last` = 前回 **書いた** (値, ts_ms)。`None`(= まだ書いていない)なら必ず書く。
    pub fn should_log(&self, last: Option<(f64, u64)>, value: f64, ts_ms: u64) -> bool {
        let Some((last_value, last_ts)) = last else {
            return true;
        };
        let threshold = (LOG_RELATIVE_THRESHOLD * last_value.abs()).max(LOG_ABSOLUTE_FLOOR);
        if (value - last_value).abs() >= threshold {
            return true;
        }
        ts_ms.saturating_sub(last_ts) >= self.log_every_s.saturating_mul(1_000)
    }
}

/// SPEC §2.3 の `counters`(slowctl → monitor PUB)。
///
/// `influx_*` は TODO/122(`[slowctl.influx]` が無ければ 0 / null のまま)。**点 = `Sample`
/// 1 個**(line protocol の field 1 個。`alarm` field は数えない)。`#[serde(default)]` は
/// 122 より前の slowctl が出したペイロードも monitor が読めるようにするため。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlowCounters {
    pub poll_ok: u64,
    pub poll_err: u64,
    pub parse_err: u64,
    pub reconnects: u64,
    pub identity_mismatch: u64,
    /// InfluxDB に書けた点数。
    #[serde(default)]
    pub influx_points_written: u64,
    /// 落とした点数(チャネル満杯・再送バッファ溢れ・NaN/inf・停止時の書き残し)。
    #[serde(default)]
    pub influx_points_dropped: u64,
    /// 失敗した POST の回数(接続不可・タイムアウト・HTTP 4xx/5xx)。
    #[serde(default)]
    pub influx_write_errors: u64,
    /// 最後に失敗した POST の内容。一度も失敗していなければ null。
    #[serde(default)]
    pub influx_last_error: Option<String>,
}

/// slowctl → monitor PUB のペイロード(SPEC §2.3 v1.31、kind = [`SLOW_KIND`])。
///
/// **`Sample` をそのままワイヤに載せる**(SPEC の `samples[]` の形と同一)。
/// ワイヤは root-sink と同じ流儀の map 形式 msgpack(`to_vec_named`)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlowPayload {
    pub kind: String,
    pub ts_ms: u64,
    pub samples: Vec<Sample>,
    pub counters: SlowCounters,
}

// ---------------------------------------------------------------------
// テスト(仕様書 — 先に書く)
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod domain_tests {
    use super::*;

    fn sample(param: &str, value: f64, node: Option<u8>) -> Sample {
        Sample {
            device: "gas".to_string(),
            node,
            param: param.to_string(),
            value,
            unit: "mbar".to_string(),
            ts_ms: 1_000,
            alarm: false,
        }
    }

    /// TODO/079 §2.1: `[lo, hi]` は**両端を含む**(逸脱で ALARM、戻れば RECOVER)。
    #[test]
    fn an_alarm_rule_fires_outside_the_closed_range_and_recovers_inside() {
        let rule = AlarmRule {
            node: Some(4),
            param: "fmeasure".to_string(),
            lo: 90.0,
            hi: 110.0,
        };
        // 範囲内(両端含む)= 平常
        assert_eq!(
            rule.evaluate(false, &sample("fmeasure", 90.0, Some(4))),
            None
        );
        assert_eq!(
            rule.evaluate(false, &sample("fmeasure", 110.0, Some(4))),
            None
        );
        // 外れれば ALARM(下・上とも)
        assert_eq!(
            rule.evaluate(false, &sample("fmeasure", 89.9, Some(4))),
            Some(AlarmEvent::Alarm)
        );
        assert_eq!(
            rule.evaluate(false, &sample("fmeasure", 110.1, Some(4))),
            Some(AlarmEvent::Alarm)
        );
        // ALARM 中に外れたままなら再通知しない(連打しない)
        assert_eq!(
            rule.evaluate(true, &sample("fmeasure", 200.0, Some(4))),
            None
        );
        // ALARM 中に戻れば RECOVER
        assert_eq!(
            rule.evaluate(true, &sample("fmeasure", 100.0, Some(4))),
            Some(AlarmEvent::Recover)
        );
    }

    /// 規則は node と param の両方が当たったサンプルにだけ効く。
    #[test]
    fn an_alarm_rule_only_matches_its_own_node_and_param() {
        let rule = AlarmRule {
            node: Some(4),
            param: "fmeasure".to_string(),
            lo: 90.0,
            hi: 110.0,
        };
        assert!(rule.matches(&sample("fmeasure", 100.0, Some(4))));
        assert!(
            !rule.matches(&sample("fmeasure", 100.0, Some(5))),
            "別ノード"
        );
        assert!(
            !rule.matches(&sample("temperature", 100.0, Some(4))),
            "別 param"
        );
        // 当たらない規則は範囲外でも何も言わない
        assert_eq!(
            rule.evaluate(false, &sample("fmeasure", 0.0, Some(5))),
            None
        );

        // node 省略 = どのノードにも当たる
        let any_node = AlarmRule {
            node: None,
            param: "fmeasure".to_string(),
            lo: 90.0,
            hi: 110.0,
        };
        assert!(any_node.matches(&sample("fmeasure", 100.0, Some(7))));
        assert!(any_node.matches(&sample("fmeasure", 100.0, None)));
    }

    /// SPEC §9.2: VALUE は「前回書いた値から 2 % 以上動いた」か「`log_every_s` 経過」。
    #[test]
    fn the_log_policy_writes_on_two_percent_or_on_time() {
        let policy = LogPolicy { log_every_s: 60 };

        // 初回(前回が無い)は必ず書く。
        assert!(policy.should_log(None, 100.0, 0));

        // 100.0 → 101.9 は 1.9 %(手計算: 1.9 / 100 = 0.019 < 0.02)→ 書かない。
        assert!(!policy.should_log(Some((100.0, 0)), 101.9, 1_000));
        // 100.0 → 102.0 はちょうど 2 %(閾値は「以上」)→ 書く。
        assert!(policy.should_log(Some((100.0, 0)), 102.0, 1_000));
        // 下向きも同じ: 100.0 → 98.0 = -2 % → 書く。
        assert!(policy.should_log(Some((100.0, 0)), 98.0, 1_000));

        // 動かなくても 60 s 経てば書く(59.999 s では書かない)。
        assert!(!policy.should_log(Some((100.0, 0)), 100.0, 59_999));
        assert!(policy.should_log(Some((100.0, 0)), 100.0, 60_000));

        // 0 近傍は相対 2 % が効かないので絶対 1e-9 を下限にする。
        assert!(!policy.should_log(Some((0.0, 0)), 5e-10, 1_000));
        assert!(policy.should_log(Some((0.0, 0)), 1e-9, 1_000));
    }

    /// SPEC §2.3 の kind=`slow` ペイロードが map 形式 msgpack で往復すること。
    #[test]
    fn the_slow_payload_round_trips_through_named_msgpack() {
        let payload = SlowPayload {
            kind: SLOW_KIND.to_string(),
            ts_ms: 1_758_000_000_123,
            samples: vec![
                sample("fmeasure", 130.5, Some(4)),
                Sample {
                    device: "env".to_string(),
                    node: None,
                    param: "t_c".to_string(),
                    value: 21.75,
                    unit: "degC".to_string(),
                    ts_ms: 1_758_000_000_100,
                    alarm: true,
                },
            ],
            counters: SlowCounters {
                poll_ok: 41,
                poll_err: 2,
                parse_err: 1,
                reconnects: 3,
                identity_mismatch: 0,
                influx_points_written: 420,
                influx_points_dropped: 7,
                influx_write_errors: 1,
                influx_last_error: Some("HTTP 503: busy".to_string()),
            },
        };
        let bytes = rmp_serde::to_vec_named(&payload).unwrap();
        let back: SlowPayload = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(back, payload);
        assert_eq!(back.samples[1].node, None, "node は nullable(SPEC §9.2)");
    }

    /// TODO/122 より前の slowctl の `counters`(5 キー)も読める —— influx 系は 0 / None。
    /// monitor だけ先に入れ替わっても、status の `slow` が壊れない。
    #[test]
    fn counters_without_the_influx_keys_still_decode() {
        #[derive(Serialize)]
        struct OldCounters {
            poll_ok: u64,
            poll_err: u64,
            parse_err: u64,
            reconnects: u64,
            identity_mismatch: u64,
        }
        let old = OldCounters {
            poll_ok: 9,
            poll_err: 1,
            parse_err: 0,
            reconnects: 2,
            identity_mismatch: 0,
        };
        let bytes = rmp_serde::to_vec_named(&old).unwrap();
        let back: SlowCounters = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(
            back,
            SlowCounters {
                poll_ok: 9,
                poll_err: 1,
                reconnects: 2,
                ..SlowCounters::default()
            }
        );
    }

    /// TODO/124: power を持たない機器(ガス・環境)は `set_power` の既定 = Unsupported。
    #[test]
    fn a_driver_without_power_control_reports_unsupported() {
        struct ReadOnly;
        impl Driver for ReadOnly {
            fn ident(&mut self) -> Result<Vec<Ident>, DriverError> {
                Ok(Vec::new())
            }
            fn poll(&mut self) -> Result<Vec<Sample>, DriverError> {
                Ok(Vec::new())
            }
        }
        assert!(matches!(
            ReadOnly.set_power(3, true),
            Err(DriverError::Unsupported)
        ));
    }

    /// logbook / status のキーは `"<device>/<node>/<param>"`、node 無しは `"-"`(SPEC §9.2)。
    #[test]
    fn the_latest_value_key_uses_a_dash_for_a_nodeless_device() {
        assert_eq!(sample("fmeasure", 1.0, Some(4)).key(), "gas/4/fmeasure");
        assert_eq!(sample("fmeasure", 1.0, None).key(), "gas/-/fmeasure");
    }
}

// =====================================================================
// TODO/089 —— HiVolta / HMP2020 の `Driver` 実装(085 コーデックの配線)
// =====================================================================

/// CAEN DT1415ET(HiVolta)ドライバ(`Driver` 実装)。TODO/089。
pub mod hivolta_driver;

/// R&S/HAMEG HMP2020 ドライバ(`Driver` 実装)。TODO/089。
pub mod hmp_driver;
