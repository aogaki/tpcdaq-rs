//! パラメータ JSON(`tpcdaq-params/1`)のスキーマと検証(078 §2)。
//!
//! モデルは「**一様値 + 例外リスト**」。UI はノブを 1 回だけ見せ、特定の
//! AsAd / AGET / ch だけ違う値は `exceptions` に入る。これは xcfg 自身の継承規則
//! (`id="*"` が既定、個別 id が上書き)を JSON に写したもの。
//!
//! # ノブ集合(7 個)
//!
//! `docs/PARAMS_KNOBS_ja.md` の照合結果に従い、**テンプレート xcfg と位置・型・範囲が
//! 一致したノブだけ**を実装している。078 §2 の表のうち `peakingTime_ns` / `readout` /
//! `zeroSuppression` / `pulser` は表と実物が食い違うため**未実装**(Fable の裁定待ち)。
//!
//! 型と範囲は xcfg の `range` 属性をそのまま写す。`range` が無いもの
//! (`Module.triggerDelay`)は型と非負だけを見る(GET 側 `CoBoNode::setTriggerDelay` も
//! 範囲チェックをしていない — GetBench/src/get/rc/CoBoNode.cpp:3209)。

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use super::paths::Scope;

/// このスキーマの識別子。将来 FW が multiplicity に対応したらここを上げる(078 §2.1)。
pub const SCHEMA_ID: &str = "tpcdaq-params/1";

/// ノブの値の型と許す範囲(xcfg の `range` 属性の写し)。
#[derive(Debug, Clone, Copy)]
pub enum KnobKind {
    /// 整数。`min`/`max` は xcfg の `range="[min, max]"`。
    Int { min: Option<i64>, max: Option<i64> },
    /// 整数の離散集合(xcfg の `range="{a, b, c}"`)。
    IntEnum(&'static [i64]),
    /// 実数。`min`/`max` は xcfg の `range="[min, max]"`。
    Real { min: Option<f64>, max: Option<f64> },
    /// 実数の離散集合。
    RealEnum(&'static [f64]),
    /// 文字列の離散集合。
    StrEnum(&'static [&'static str]),
    /// 真偽値(xcfg は `true`/`false`)。
    Bool,
    /// **合成ノブ**: JSON の 1 つの値(モード名)から xcfg のテキストを引く対応表。
    /// 同じ JSON キーを持つ [`KnobSpec`] が複数あり、各々が別の要素に書く。
    Derived(&'static [(&'static str, &'static str)]),
}

/// 1 ノブの定義(JSON のキー ↔ xcfg 上の位置)。
#[derive(Debug, Clone, Copy)]
pub struct KnobSpec {
    /// JSON のキー。入れ子は `.` 区切り(`"pulser.enabled"` = `{"pulser":{"enabled":…}}`)。
    /// **合成ノブ(`readout`)では同じ名前のエントリが複数ある**。
    pub name: &'static str,
    /// xcfg 上でその値が住む階層。
    pub scope: Scope,
    /// スコープノードの下の要素名の並び。
    pub suffix: &'static [&'static str],
    pub kind: KnobKind,
}

/// 管理ノブの表。**ここが唯一の定義**(レンダラも検証もこれを読む)。
///
/// 出典: `docs/PARAMS_KNOBS_ja.md` §3(2026-09-22 に reference/config の
/// configure-*.xcfg 5 本で全数照合)。
pub const KNOBS: &[KnobSpec] = &[
    KnobSpec {
        name: "triggerDelay",
        scope: Scope::Cobo,
        suffix: &["Module", "triggerDelay"],
        // xcfg に range 属性なし(unit="10ns")。負の遅延は無い。
        kind: KnobKind::Int {
            min: Some(0),
            max: None,
        },
    },
    KnobSpec {
        name: "writingClockFrequency_MHz",
        scope: Scope::Cobo,
        suffix: &["Module", "writingClockFrequency"],
        // xcfg の range は [1.0, 200.0] だが、078 §2 の表は enum。実ファイルの出現値は
        // 12.5 / 25.0 / 100.0。CKW は 100 MHz/2**N(R-CoBo)の制約があるので離散で扱う。
        kind: KnobKind::RealEnum(&[12.5, 25.0, 50.0, 100.0]),
    },
    KnobSpec {
        name: "triggerMode",
        scope: Scope::Cobo,
        suffix: &["Module", "triggerMode"],
        // zCoBo FW が扱える値だけ(078 §2.1 裁定 1。onMultiplicity* / *_2p は拒否)。
        kind: KnobKind::StrEnum(&[
            "external",
            "noTrigger",
            "onRequest",
            "onSlowControl",
            "periodically",
        ]),
    },
    KnobSpec {
        name: "globalThreshold",
        scope: Scope::Aget,
        suffix: &["Global", "Reg1", "GlobalThresholdValue"],
        kind: KnobKind::Int {
            min: Some(0),
            max: Some(7),
        },
    },
    KnobSpec {
        name: "lsbThreshold",
        scope: Scope::Channel,
        suffix: &["LSBThresholdValue"],
        kind: KnobKind::Int {
            min: Some(0),
            max: Some(15),
        },
    },
    KnobSpec {
        name: "gain",
        scope: Scope::Channel,
        suffix: &["Gain"],
        kind: KnobKind::StrEnum(&["10pC", "120fC", "1pC", "240fC"]),
    },
    KnobSpec {
        name: "triggerInhibition",
        scope: Scope::Channel,
        suffix: &["TriggerInhibition"],
        // 078 §2.1 裁定 2: ノイズ ch のマスクの正規の道具(FW 非依存)。
        kind: KnobKind::StrEnum(&["inhibit_channel", "inhibit_trigger", "none"]),
    },
    // --- 2026-09-22 のレビュー裁定で追加した 4 ノブ ---
    KnobSpec {
        name: "peakingTime_ns",
        // **AGET スコープ**(表は channel と書いていたが実物は chip 単位のレジスタ)。
        // 要素名の綴りは GET 側のまま(`peackingTime`)。
        scope: Scope::Aget,
        suffix: &["Global", "Reg1", "peackingTime"],
        kind: KnobKind::IntEnum(PEAKING_TIME_NS),
    },
    // `readout` = 合成ノブ。full / partial が**同じ JSON キー**から 2 要素に展開される。
    // 値は実テンプレート 2 本の差分から: full = configure-physics、partial = configure-pulser。
    KnobSpec {
        name: "readout",
        scope: Scope::Aget,
        suffix: &["Global", "Reg1", "isAllChannelRead"],
        kind: KnobKind::Derived(&[("full", "true"), ("partial", "false")]),
    },
    KnobSpec {
        name: "readout",
        scope: Scope::Aget,
        suffix: &["Global", "Reg1", "isFPNRead"],
        kind: KnobKind::Derived(&[("full", "true"), ("partial", "false")]),
    },
    // `zeroSuppression` は JSON では 1 つのオブジェクトだが、xcfg では**スコープが違う 2 要素**。
    // 例外は threshold にだけ意味がある(enabled は CoBo スコープなので `at` は必ず細かすぎ → 拒否)。
    KnobSpec {
        name: "zeroSuppression.enabled",
        scope: Scope::Cobo,
        suffix: &["Module", "enableZeroSuppression"],
        kind: KnobKind::Bool,
    },
    KnobSpec {
        name: "zeroSuppression.threshold",
        scope: Scope::Channel,
        suffix: &["zeroSuppressionThreshold"],
        kind: KnobKind::Int {
            min: Some(0),
            max: Some(4095),
        },
    },
    // `pulser` は **CoBo スコープ**の `Instance.Generator`(AsAd の Generator ではない)。
    KnobSpec {
        name: "pulser.enabled",
        scope: Scope::Cobo,
        suffix: &["Generator", "periodicModeEnabled"],
        kind: KnobKind::Bool,
    },
    KnobSpec {
        name: "pulser.voltage_mV",
        scope: Scope::Cobo,
        suffix: &["Generator", "amplitudeStart"],
        // GET 側 `CoBoConfigParser::asadPulserVoltage_mV` は amplitudeStart をそのまま
        // 「出力電圧と基準電圧(Vref=1102mV)の差 [mV]」として使う(int16 に切り捨て)。
        // → JSON の `voltage_mV` は換算なしの同値。
        kind: KnobKind::Real {
            min: Some(-1000.0),
            max: Some(1000.0),
        },
    },
    KnobSpec {
        name: "pulser.period_ms",
        scope: Scope::Cobo,
        suffix: &["Generator", "period_ms"],
        kind: KnobKind::Int {
            min: Some(0),
            max: None,
        },
    },
];

/// AGET の peaking time の許容値 [ns]。
///
/// **GET の AGET 仕様の 16 値**(実 xcfg の `range` 属性がそのまま列挙しているもの:
/// `<peackingTime unit="ns" range="{70, 117, 232, …, 1014}">`)。テンプレートから実行時に
/// 読まず、ここを唯一の定義とする(`tests/params_render.rs` がフィクスチャの range 属性と
/// 一致することを確かめる)。実テンプレートに現れる値は 232(physics / pedestals 計 4 本)と
/// 1014(configure-pulser)。
pub const PEAKING_TIME_NS: &[i64] = &[
    70, 117, 232, 280, 334, 383, 502, 541, 568, 632, 721, 760, 831, 870, 976, 1014,
];

/// 名前からノブ定義を引く(合成ノブでは最初のエントリ)。
pub fn knob_spec(name: &str) -> Option<&'static KnobSpec> {
    KNOBS.iter().find(|k| k.name == name)
}

/// その JSON キーを持つ全エントリ(合成ノブは複数)。
pub fn knob_specs(name: &str) -> impl Iterator<Item = &'static KnobSpec> + '_ {
    KNOBS.iter().filter(move |k| k.name == name)
}

/// 例外の粗さ判定に使うスコープの深さ。合成ノブでは**一番浅いエントリ**に合わせる
/// (深い方だけに例外を付けても、浅い方が一様値のままでは意味が壊れるため)。
pub fn knob_scope_depth(name: &str) -> Option<usize> {
    knob_specs(name).map(|spec| spec.scope.depth()).min()
}

/// JSON の `knobs` から `.` 区切りの名前で値を引く。
pub fn lookup_knob<'a>(knobs: &'a BTreeMap<String, Value>, name: &str) -> Option<&'a Value> {
    let mut parts = name.split('.');
    let first = parts.next()?;
    let mut value = knobs.get(first)?;
    for part in parts {
        value = value.get(part)?;
    }
    Some(value)
}

/// JSON の `knobs` に実際に入っている葉の名前を `.` 区切りで集める(未知キー検出用)。
fn leaf_names(value: &Value, prefix: &str, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let name = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                leaf_names(child, &name, out);
            }
        }
        _ => out.push(prefix.to_string()),
    }
}

/// スキーマ違反(400 で返す想定 — レーン B)。**黙って丸めない**(CLAUDE.md)。
#[derive(Debug, thiserror::Error)]
pub enum ParamsError {
    #[error("JSON として読めない: {0}")]
    Json(#[from] serde_json::Error),

    #[error("未知のスキーマ id {found}(期待: {SCHEMA_ID})")]
    UnknownSchema { found: String },

    #[error("template が空")]
    EmptyTemplate,

    #[error("ノブ {knob} が無い(全ノブ必須)")]
    MissingKnob { knob: String },

    #[error("未知のノブ {knob}")]
    UnknownKnob { knob: String },

    #[error("ノブ {knob} の値 {value} が型違い(期待: {expected})")]
    BadType {
        knob: String,
        value: String,
        expected: &'static str,
    },

    #[error("ノブ {knob} の値 {value} が範囲外({min:?}..={max:?})")]
    OutOfRange {
        knob: String,
        value: String,
        min: Option<i64>,
        max: Option<i64>,
    },

    #[error("ノブ {knob} の値 {value} は許された値ではない(許容: {allowed})")]
    NotInEnum {
        knob: String,
        value: String,
        allowed: String,
    },

    #[error("例外の at {at} は (asad, aget, ch) の接頭辞ではない(空も不可)")]
    ExceptionAtNotPrefix { at: String },

    #[error(
        "例外の at {at} はノブ {knob} のスコープ(深さ {scope_depth})より細かい(深さ {at_depth})"
    )]
    ExceptionTooFine {
        knob: String,
        at: String,
        at_depth: usize,
        scope_depth: usize,
    },

    #[error("同じノブ {knob} に同じ at {at} の例外が 2 つある")]
    DuplicateException { knob: String, at: String },

    #[error("例外の対象 {level}[{id}] がテンプレートに無い")]
    TargetNotInTemplate { level: String, id: String },

    #[error("ノブ {knob} の例外 {at} を {level} の段に展開できない(テンプレートに明示 id が無い)")]
    CannotExpandException {
        knob: String,
        at: String,
        level: String,
    },

    #[error("テンプレートの形が想定と違う: {what}")]
    TemplateShape { what: String },

    #[error("テンプレートを XML として読めない: {0}")]
    Xml(String),
}

/// 例外の宛先。`at` は (asad, aget, ch) の**接頭辞**でなければならない
/// (粗さ = 末尾の切り落とし。078 §2 の例 `{"asad":1}`)。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct At {
    pub asad: Option<u32>,
    pub aget: Option<u32>,
    pub ch: Option<u32>,
}

impl At {
    /// 指定された階層の深さ(0 = 何も指定されていない)。
    pub fn depth(&self) -> usize {
        match (self.asad, self.aget, self.ch) {
            (Some(_), Some(_), Some(_)) => 3,
            (Some(_), Some(_), None) => 2,
            (Some(_), None, None) => 1,
            _ => 0,
        }
    }

    /// 接頭辞になっているか(`{ch:3}` 単独や `{aget:1, ch:2}` は不可)。
    pub fn is_prefix(&self) -> bool {
        self.depth() > 0
    }

    /// 深さ分の id 文字列(`knob_path` の `ids` に渡す形)。
    pub fn ids(&self) -> Vec<String> {
        let mut ids = Vec::new();
        if let Some(asad) = self.asad {
            ids.push(asad.to_string());
        }
        if let Some(aget) = self.aget {
            ids.push(aget.to_string());
        }
        if let Some(ch) = self.ch {
            ids.push(ch.to_string());
        }
        ids
    }
}

impl std::fmt::Display for At {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{")?;
        let mut first = true;
        for (name, value) in [("asad", self.asad), ("aget", self.aget), ("ch", self.ch)] {
            if let Some(value) = value {
                if !first {
                    write!(f, ", ")?;
                }
                write!(f, "{name}: {value}")?;
                first = false;
            }
        }
        write!(f, "}}")
    }
}

/// 例外 1 件(「あるチップだけ別値」)。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exception {
    pub at: At,
    pub knob: String,
    pub value: Value,
    #[serde(default)]
    pub note: Option<String>,
}

/// 検証済みのパラメータ JSON。
#[derive(Debug, Clone)]
pub struct Params {
    pub schema: String,
    /// テンプレート xcfg のファイル名(内容はレンダラの引数 — 依存は内向き)。
    pub template: String,
    /// ノブ名 → 一様値(全ノブが必ず入っている)。
    pub knobs: BTreeMap<String, Value>,
    pub exceptions: Vec<Exception>,
    pub note: Option<String>,
    /// 生成物の `<id>` に使う sha256 の元(受け取った JSON テキストそのもの)。
    raw_json: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ParamsRaw {
    schema: String,
    template: String,
    knobs: BTreeMap<String, Value>,
    #[serde(default)]
    exceptions: Vec<Exception>,
    #[serde(default)]
    note: Option<String>,
}

impl Params {
    /// JSON テキストを読んで検証する。**受け取ったテキストをそのまま保持**して
    /// `<id>` の sha256 に使う(078 §3-3「同じ JSON → 同じファイル」)。
    pub fn from_json_str(json: &str) -> Result<Params, ParamsError> {
        let raw: ParamsRaw = serde_json::from_str(json)?;

        if raw.schema != SCHEMA_ID {
            return Err(ParamsError::UnknownSchema { found: raw.schema });
        }
        if raw.template.trim().is_empty() {
            return Err(ParamsError::EmptyTemplate);
        }

        // 一様値: 全ノブが揃っていること + 未知キーが無いこと。
        for spec in KNOBS {
            let value =
                lookup_knob(&raw.knobs, spec.name).ok_or_else(|| ParamsError::MissingKnob {
                    knob: spec.name.to_string(),
                })?;
            check_value(spec, value)?;
        }
        let mut names = Vec::new();
        for (key, value) in &raw.knobs {
            leaf_names(value, key, &mut names);
        }
        for name in &names {
            if knob_spec(name).is_none() {
                return Err(ParamsError::UnknownKnob { knob: name.clone() });
            }
        }

        // 例外: ノブが既知 / at が接頭辞 / スコープ以上に粗い / 値が範囲内 / 重複なし。
        let mut seen: Vec<(String, String)> = Vec::new();
        for exception in &raw.exceptions {
            let spec = knob_spec(&exception.knob).ok_or_else(|| ParamsError::UnknownKnob {
                knob: exception.knob.clone(),
            })?;
            if !exception.at.is_prefix() {
                return Err(ParamsError::ExceptionAtNotPrefix {
                    at: exception.at.to_string(),
                });
            }
            let at_depth = exception.at.depth();
            let scope_depth = knob_scope_depth(&exception.knob).unwrap_or(spec.scope.depth());
            if at_depth > scope_depth {
                return Err(ParamsError::ExceptionTooFine {
                    knob: exception.knob.clone(),
                    at: exception.at.to_string(),
                    at_depth,
                    scope_depth,
                });
            }
            check_value(spec, &exception.value)?;

            let key = (exception.knob.clone(), exception.at.to_string());
            if seen.contains(&key) {
                return Err(ParamsError::DuplicateException {
                    knob: key.0,
                    at: key.1,
                });
            }
            seen.push(key);
        }

        Ok(Params {
            schema: raw.schema,
            template: raw.template,
            knobs: raw.knobs,
            exceptions: raw.exceptions,
            note: raw.note,
            raw_json: json.to_string(),
        })
    }

    /// JSON テキストの sha256 の先頭 8 桁(078 §3-3)。
    pub fn json_hash8(&self) -> String {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(self.raw_json.as_bytes());
        let hex = format!("{digest:x}");
        hex[..8].to_string()
    }

    /// 有効値(表示用)。**JSON 内で完結**する most specific wins(078 §3-4)。
    /// CompoundConfig の継承解決を Rust に再実装しない — 管理ノブは JSON が正。
    pub fn effective_value(&self, knob: &str, asad: u32, aget: u32, ch: u32) -> Option<&Value> {
        let uniform = lookup_knob(&self.knobs, knob)?;
        let mut best: Option<(usize, &Value)> = None;
        for exception in &self.exceptions {
            if exception.knob != knob {
                continue;
            }
            let matches = exception.at.asad.map(|v| v == asad).unwrap_or(true)
                && exception.at.aget.map(|v| v == aget).unwrap_or(true)
                && exception.at.ch.map(|v| v == ch).unwrap_or(true);
            if !matches {
                continue;
            }
            let depth = exception.at.depth();
            if best.map(|(d, _)| depth > d).unwrap_or(true) {
                best = Some((depth, &exception.value));
            }
        }
        Some(best.map(|(_, v)| v).unwrap_or(uniform))
    }
}

/// 値を xcfg に書くテキストにする(型ごとの整形)。
pub fn value_to_xcfg_text(spec: &KnobSpec, value: &Value) -> Result<String, ParamsError> {
    match spec.kind {
        KnobKind::Int { .. } | KnobKind::IntEnum(_) => value
            .as_i64()
            .map(|v| v.to_string())
            .ok_or_else(|| bad_type(spec, value, "integer")),
        KnobKind::Real { .. } | KnobKind::RealEnum(_) => value
            .as_f64()
            .map(format_real)
            .ok_or_else(|| bad_type(spec, value, "real")),
        KnobKind::StrEnum(_) => value
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| bad_type(spec, value, "string")),
        KnobKind::Bool => value
            .as_bool()
            .map(|v| v.to_string())
            .ok_or_else(|| bad_type(spec, value, "bool")),
        KnobKind::Derived(table) => {
            let mode = value
                .as_str()
                .ok_or_else(|| bad_type(spec, value, "string"))?;
            table
                .iter()
                .find(|(json, _)| *json == mode)
                .map(|(_, text)| (*text).to_string())
                .ok_or_else(|| ParamsError::NotInEnum {
                    knob: spec.name.to_string(),
                    value: mode.to_string(),
                    allowed: table
                        .iter()
                        .map(|(json, _)| *json)
                        .collect::<Vec<_>>()
                        .join(", "),
                })
        }
    }
}

/// xcfg の実数表記に合わせる(`25` ではなく `25.0`)。
fn format_real(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.1}")
    } else {
        format!("{value}")
    }
}

fn bad_type(spec: &KnobSpec, value: &Value, expected: &'static str) -> ParamsError {
    ParamsError::BadType {
        knob: spec.name.to_string(),
        value: value.to_string(),
        expected,
    }
}

pub(super) fn check_value(spec: &KnobSpec, value: &Value) -> Result<(), ParamsError> {
    match spec.kind {
        KnobKind::Int { min, max } => {
            let v = value
                .as_i64()
                .ok_or_else(|| bad_type(spec, value, "integer"))?;
            if min.map(|m| v < m).unwrap_or(false) || max.map(|m| v > m).unwrap_or(false) {
                return Err(ParamsError::OutOfRange {
                    knob: spec.name.to_string(),
                    value: v.to_string(),
                    min,
                    max,
                });
            }
        }
        KnobKind::IntEnum(allowed) => {
            let v = value
                .as_i64()
                .ok_or_else(|| bad_type(spec, value, "integer"))?;
            if !allowed.contains(&v) {
                return Err(ParamsError::NotInEnum {
                    knob: spec.name.to_string(),
                    value: v.to_string(),
                    allowed: format!("{allowed:?}"),
                });
            }
        }
        KnobKind::Real { min, max } => {
            let v = value
                .as_f64()
                .ok_or_else(|| bad_type(spec, value, "real"))?;
            if min.map(|m| v < m).unwrap_or(false) || max.map(|m| v > m).unwrap_or(false) {
                return Err(ParamsError::OutOfRange {
                    knob: spec.name.to_string(),
                    value: format_real(v),
                    min: min.map(|m| m as i64),
                    max: max.map(|m| m as i64),
                });
            }
        }
        KnobKind::RealEnum(allowed) => {
            let v = value
                .as_f64()
                .ok_or_else(|| bad_type(spec, value, "real"))?;
            if !allowed.iter().any(|a| (a - v).abs() < f64::EPSILON) {
                return Err(ParamsError::NotInEnum {
                    knob: spec.name.to_string(),
                    value: format_real(v),
                    allowed: format!("{allowed:?}"),
                });
            }
        }
        KnobKind::Bool => {
            if !value.is_boolean() {
                return Err(bad_type(spec, value, "bool"));
            }
        }
        KnobKind::Derived(_) => {
            // 対応表に無いモード名はここで NotInEnum になる(変換と同じ検査)。
            value_to_xcfg_text(spec, value)?;
        }
        KnobKind::StrEnum(allowed) => {
            let v = value
                .as_str()
                .ok_or_else(|| bad_type(spec, value, "string"))?;
            if !allowed.contains(&v) {
                return Err(ParamsError::NotInEnum {
                    knob: spec.name.to_string(),
                    value: v.to_string(),
                    allowed: allowed.join(", "),
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn knob_names_repeat_only_for_composite_knobs_and_targets_are_unique() {
        for (i, spec) in KNOBS.iter().enumerate() {
            assert!(!spec.suffix.is_empty(), "{} has no suffix", spec.name);

            for other in KNOBS.iter().skip(i + 1) {
                if other.name == spec.name {
                    // 名前が重複してよいのは合成ノブ(`readout`)だけ。
                    assert!(
                        matches!(spec.kind, KnobKind::Derived(_))
                            && matches!(other.kind, KnobKind::Derived(_)),
                        "duplicate knob {} は合成ノブ(Derived)でなければならない",
                        spec.name
                    );
                }
                // 2 つのノブが同じ要素に書いてはいけない(後勝ちの silent な上書きになる)。
                assert!(
                    !(other.scope == spec.scope && other.suffix == spec.suffix),
                    "{} と {} が同じ要素に書こうとしている",
                    spec.name,
                    other.name
                );
            }
        }
    }

    /// 入れ子キー(`pulser.enabled` など)は `.` 区切りで引ける。
    #[test]
    fn nested_knob_names_are_looked_up_through_the_dotted_path() {
        let knobs: BTreeMap<String, Value> =
            serde_json::from_str(r#"{"pulser":{"enabled":true,"period_ms":200},"gain":"120fC"}"#)
                .expect("fixture json");

        assert_eq!(
            lookup_knob(&knobs, "pulser.enabled"),
            Some(&Value::Bool(true))
        );
        assert_eq!(lookup_knob(&knobs, "gain"), Some(&Value::from("120fC")));
        assert_eq!(lookup_knob(&knobs, "pulser.voltage_mV"), None);
    }

    /// 合成ノブのスコープ深さは**一番浅いエントリ**(例外の粗さ判定に使う)。
    #[test]
    fn a_composite_knob_uses_the_shallowest_scope_for_the_exception_check() {
        assert_eq!(knob_scope_depth("readout"), Some(Scope::Aget.depth()));
        assert_eq!(knob_scope_depth("zeroSuppression.enabled"), Some(0));
        assert_eq!(knob_scope_depth("zeroSuppression.threshold"), Some(3));
        assert_eq!(knob_scope_depth("nope"), None);
    }

    #[test]
    fn reals_are_printed_the_way_the_xcfg_writes_them() {
        // テンプレートの実表記: `<writingClockFrequency unit="MHz" ...>25.0</...>`
        assert_eq!(format_real(25.0), "25.0");
        assert_eq!(format_real(12.5), "12.5");
        assert_eq!(format_real(100.0), "100.0");
    }

    #[test]
    fn the_at_depth_is_the_length_of_the_prefix() {
        let at = At {
            asad: Some(1),
            aget: Some(2),
            ch: None,
        };
        assert_eq!(at.depth(), 2);
        assert!(at.is_prefix());
        assert_eq!(at.ids(), vec!["1".to_string(), "2".to_string()]);

        let broken = At {
            asad: None,
            aget: None,
            ch: Some(5),
        };
        assert_eq!(broken.depth(), 0);
        assert!(!broken.is_prefix());
    }
}
