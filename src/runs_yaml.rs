//! JSONL ログブック → minitpc-analysis `runs.yaml` 生成(TODO/082)。
//!
//! 解析側(別リポ `~/Workspace/Higs/minitpc-analysis`、読み取り専用)の `runs/runs.yaml` は
//! 「run 条件の唯一の情報源」だが、これまで手打ちだった。ここでは DAQ 自身が書く
//! `logbook.jsonl`(SPEC §9、[`crate::logbook`])を唯一の入力として、同じキーを持つ
//! `runs.yaml` を機械的に組み立てる**純関数**を提供する(ファイル I/O は
//! `src/bin/logbook2runs.rs` 側)。
//!
//! # 対応表(発注書 082 の入出力節)
//!
//! - `run_id` / `files` の `<TS>` は **`run_stop.files` の graw ファイル名**
//!   (`CoBo{K}_AsAd{A}_{TS}_{idx}.graw`、CLAUDE.md の命名規約)から取る。
//!   `run_stop` が無い(進行中/クラッシュ)run は `run_start.ts` からオフセットを落として
//!   代用する。
//! - `n_events` = `run_stop.counters.events_built`(取得できれば)、無ければ
//!   `frames` の合計。
//! - `geometry` = `run_start.geometry.path` のベースネーム。
//! - `reco.hypothesis` / `conditions.{gas,temperature_K,sampling_rate_MHz}` は
//!   条件ファイル(`runs_defaults.toml`)の `[defaults]` から。
//! - `conditions.{pressure_mbar,drift_hv_V}` と `source` は同ファイルの `[runs.<N>]`
//!   による run 毎の上書き(079/B-4 が logbook に載せるまでの暫定置き場)。
//! - `drift_velocity_cm_us` は **CO₂ かつ pressure・drift_hv が両方揃うときだけ**
//!   `0.7233 × (drift_hv_V / 20) / pressure_mbar` で埋め、それ以外は `null  # unknown`。
//! - `notes` = `run_start.comment` + 同 run 中の `comment` レコード + (`run_stop.reason`
//!   が `"normal"` 以外ならそれも) + (`run_stop` が無ければ `"no run_stop"`)。
//!
//! # silent failure を作らない(CLAUDE.md)
//!
//! [`build_runs_doc`] は純関数なので IO(stderr)を持たない代わりに、異常
//! (run_stop に対応する run_start が無い/AsAd graw が見つからない/run_stop 無しで
//! 次の run_start が来た)を `Vec<String>` の警告として返す。呼び手([`crate::bin`]側)が
//! これを stderr に出す。壊れた JSONL 行も同様に、[`parse_logbook_lines`] が
//! 例外を投げる代わりに行番号付きで数える。

use serde::Deserialize;
use std::collections::BTreeMap;

use crate::logbook::{LogbookLine, LogbookRecord};

// ---------------------------------------------------------------------
// 条件ファイル(`runs_defaults.toml`)
// ---------------------------------------------------------------------

/// `runs_defaults.toml` の解決済み中身(発注書 082 の入出力節)。
///
/// すべて省略可 —— ファイル自体が「任意」であり(`Defaults::default()` が
/// 「何も分からない」を表す)、各キーも独立に省略できる。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Defaults {
    pub gas: Option<String>,
    pub temperature_k: Option<f64>,
    pub sampling_rate_mhz: Option<f64>,
    /// 発注書の例 TOML には載っているが、出力節は `geometry:` の出典を
    /// `run_start.geometry.path` のベースネームと明記しており、この値の参照先が
    /// 無い。パースはして(未知キー扱いで弾かない)、出力には使わない
    /// (082 の疑問点 — 最終報告参照)。
    pub geometry: Option<String>,
    pub hypothesis: Option<String>,
    /// `[runs.<N>]` の run 番号 → 上書き。
    pub runs: BTreeMap<u32, RunOverride>,
}

/// `[runs.<N>]` 1 個分(発注書 082: 「圧力・HV は 079/B-4 が logbook に載せるまでここで与える」)。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RunOverride {
    pub pressure_mbar: Option<f64>,
    pub drift_hv_v: Option<f64>,
    pub source: Option<SourceToml>,
}

/// `source`(出力の `source: {primary, background}` にそのまま乗る)。
///
/// 発注書の例 TOML は `source = "文字列"`(フラット)1 個しか見せていないが、出力は
/// `primary`/`background` の 2 値を持つ。**この場に判断が必要**(発注書に明記が無い):
/// フラット文字列は `primary` のみの略記として受理し(`background` は `null`)、
/// 両方欲しいときは `source = { primary = "...", background = "..." }` のテーブル形を
/// 使えるようにした(`config.rs::RawConfigId` と同じ untagged enum の流儀 — 新しい
/// 抽象を持ち込まず、既存パターンを踏襲)。最終報告の「逸脱」節に記載。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SourceToml {
    pub primary: Option<String>,
    pub background: Option<String>,
}

/// `runs_defaults.toml` のパース失敗。
#[derive(Debug, thiserror::Error)]
pub enum DefaultsError {
    #[error("TOML parse error: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("invalid run number key in [runs.<N>]: {0:?}")]
    InvalidRunKey(String),
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
struct RawDefaultsFile {
    defaults: RawDefaultsSection,
    runs: BTreeMap<String, RawRunOverride>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
struct RawDefaultsSection {
    gas: Option<String>,
    #[serde(rename = "temperature_K")]
    temperature_k: Option<f64>,
    #[serde(rename = "sampling_rate_MHz")]
    sampling_rate_mhz: Option<f64>,
    geometry: Option<String>,
    hypothesis: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
struct RawRunOverride {
    pressure_mbar: Option<f64>,
    #[serde(rename = "drift_hv_V")]
    drift_hv_v: Option<f64>,
    source: Option<RawSource>,
}

/// `source` の 2 通りの書き方(モジュール doc の説明参照)。
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawSource {
    Flat(String),
    Table {
        #[serde(default)]
        primary: Option<String>,
        #[serde(default)]
        background: Option<String>,
    },
}

impl From<RawSource> for SourceToml {
    fn from(raw: RawSource) -> Self {
        match raw {
            RawSource::Flat(s) => SourceToml {
                primary: Some(s),
                background: None,
            },
            RawSource::Table {
                primary,
                background,
            } => SourceToml {
                primary,
                background,
            },
        }
    }
}

/// `runs_defaults.toml` の中身(文字列)を読む。
pub fn parse_defaults_toml(s: &str) -> Result<Defaults, DefaultsError> {
    let raw: RawDefaultsFile = toml::from_str(s)?;
    let mut runs = BTreeMap::new();
    for (key, value) in raw.runs {
        let run: u32 = key
            .parse()
            .map_err(|_| DefaultsError::InvalidRunKey(key.clone()))?;
        runs.insert(
            run,
            RunOverride {
                pressure_mbar: value.pressure_mbar,
                drift_hv_v: value.drift_hv_v,
                source: value.source.map(SourceToml::from),
            },
        );
    }
    Ok(Defaults {
        gas: raw.defaults.gas,
        temperature_k: raw.defaults.temperature_k,
        sampling_rate_mhz: raw.defaults.sampling_rate_mhz,
        geometry: raw.defaults.geometry,
        hypothesis: raw.defaults.hypothesis,
        runs,
    })
}

// ---------------------------------------------------------------------
// JSONL ログブックの寛容な読み込み(silent failure を作らない)
// ---------------------------------------------------------------------

/// パースできなかった 1 行(行番号は 1 始まり)。
#[derive(Debug, Clone, PartialEq)]
pub struct CorruptLine {
    pub line_no: usize,
    pub message: String,
}

/// JSONL 全体を寛容に読む: パースできる行はすべて拾い、できない行は**末尾に限らず**
/// 行番号付きで数える(`crate::logbook::read_since` は末尾行だけ許容して他は `Err` だが、
/// この変換ツールは「読めるだけ読んで壊れ行を可視化する」— 発注書「parse できない行は
/// 行番号付きで stderr に数える」)。空行はスキップする。
pub fn parse_logbook_lines(content: &str) -> (Vec<LogbookLine>, Vec<CorruptLine>) {
    let mut lines = Vec::new();
    let mut corrupt = Vec::new();
    for (idx, raw) in content.lines().enumerate() {
        if raw.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<LogbookLine>(raw) {
            Ok(parsed) => lines.push(parsed),
            Err(e) => corrupt.push(CorruptLine {
                line_no: idx + 1,
                message: e.to_string(),
            }),
        }
    }
    (lines, corrupt)
}

// ---------------------------------------------------------------------
// runs.yaml の中身(公開データ)
// ---------------------------------------------------------------------

/// `runs.yaml` の 1 run 分(発注書 082 の出力節のキーそのまま)。
#[derive(Debug, Clone, PartialEq)]
pub struct RunEntry {
    pub run_id: String,
    pub files: String,
    pub format: String,
    pub n_events: u64,
    pub geometry: String,
    pub hypothesis: Option<String>,
    pub gas: Option<String>,
    pub pressure_mbar: Option<f64>,
    pub temperature_k: Option<f64>,
    pub drift_hv_v: Option<f64>,
    pub drift_velocity_cm_us: Option<f64>,
    pub sampling_rate_mhz: Option<f64>,
    pub source_primary: Option<String>,
    pub source_background: Option<String>,
    pub notes: String,
}

/// `runs.yaml` 全体。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RunsDoc {
    pub runs: Vec<RunEntry>,
}

// ---------------------------------------------------------------------
// drift_velocity_cm_us(発注書 082: 100 mbar / 1600 V → 0.5786 がオラクル)
// ---------------------------------------------------------------------

/// SURVEY §2 の係数。ドリフトケージ長 20 cm で HV を電界(V/cm)に変換する。
const DRIFT_VELOCITY_COEFF: f64 = 0.7233;
const DRIFT_CAGE_LENGTH_CM: f64 = 20.0;

/// `gas == "CO2"` かつ `pressure_mbar` と `drift_hv_v` が両方揃うときだけ計算する。
///
/// 手計算(発注書のオラクル): 100 mbar / 1600 V → E = 1600/20 = 80 V/cm →
/// 0.7233 × 80 / 100 = 0.57864 → 4 桁丸めで **0.5786**。
fn compute_drift_velocity_cm_us(
    gas: Option<&str>,
    pressure_mbar: Option<f64>,
    drift_hv_v: Option<f64>,
) -> Option<f64> {
    if gas != Some("CO2") {
        return None;
    }
    let (pressure, hv) = match (pressure_mbar, drift_hv_v) {
        (Some(p), Some(hv)) => (p, hv),
        _ => return None,
    };
    let e_field_v_per_cm = hv / DRIFT_CAGE_LENGTH_CM;
    let raw = DRIFT_VELOCITY_COEFF * e_field_v_per_cm / pressure;
    Some((raw * 10_000.0).round() / 10_000.0)
}

// ---------------------------------------------------------------------
// ファイル名からの TS 抽出(CLAUDE.md 命名規約)
// ---------------------------------------------------------------------

/// `CoBo{K}_AsAd{A}_{TS}_{idx}.graw` の basename から `{TS}` を取り出す。
///
/// マッチしなければ `None`(例: listen-before-start の「ctrl」プローブ接続が残す
/// `CoBo{K}_{TS}_{idx}.graw` は AsAd 区画が無いので該当しない — 041 実測、
/// `reference/_spike/demo/out/logbook_BC_saved.jsonl` の `.../ctrl/CoBo0_..._0000.graw`)。
fn asad_graw_ts(path: &str) -> Option<String> {
    let name = std::path::Path::new(path).file_name()?.to_str()?;
    let stem = name.strip_suffix(".graw")?;
    let parts: Vec<&str> = stem.split('_').collect();
    if parts.len() != 4 {
        return None;
    }
    if !parts[0].starts_with("CoBo") || !parts[1].starts_with("AsAd") {
        return None;
    }
    Some(parts[2].to_string())
}

/// `run_start.ts`(RFC3339・ミリ秒・オフセット付き、固定 29 バイト —
/// `crate::logbook` テストの `looks_like_rfc3339_millis_with_offset` と同じ前提)から
/// オフセットを落として、ファイル名 TS 形式(`YYYY-MM-DDTHH:MM:SS.mmm`、23 バイト)にする。
/// `run_stop` の無い(進行中/クラッシュ)run の `run_id`/`files` フォールバック用。
///
/// 想定外の長さ・形式ならそのまま返す(壊れた ts をここで panic させない —— 上位の
/// 表示はそのまま怪しい値になるが、silent には失敗しない)。
fn ts_without_offset(ts: &str) -> String {
    if ts.len() == 29 && ts.as_bytes().get(19) == Some(&b'.') {
        ts[0..23].to_string()
    } else {
        ts.to_string()
    }
}

/// basename(拡張子を含む素朴な実装 —— このツールが扱うのは常に `.dat`/短い文字列なので
/// 依存を増やさない)。
fn basename(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
        .to_string()
}

// ---------------------------------------------------------------------
// 組み立て本体
// ---------------------------------------------------------------------

/// 1 run を組み立てる途中状態。
struct RunAcc {
    geometry_path: String,
    start_comment: String,
    start_ts: String,
    /// run 中に見た `comment` レコードのテキスト(出現順)。
    comments: Vec<String>,
    stop: Option<RunStopSnapshot>,
}

struct RunStopSnapshot {
    reason: String,
    n_events: u64,
    /// `run_stop.files` から抽出できた AsAd graw の TS(見つからなければ `None`)。
    ts_from_files: Option<String>,
}

/// `logbook.jsonl` の行列(ファイル順のまま)+ 条件ファイルから `RunsDoc` を組み立てる。
///
/// **`lines` はファイル出現順であること**(seq の数値そのものは見ない —— run の
/// 「開いている/閉じている」状態はこの出現順だけで追う)。異常は例外にせず、
/// 2 つ目の戻り値(警告)に積む(silent failure を作らない — CLAUDE.md)。
pub fn build_runs_doc(
    lines: &[LogbookLine],
    defaults: &Defaults,
    data_root: &str,
) -> (RunsDoc, Vec<String>) {
    let mut accs: BTreeMap<u32, RunAcc> = BTreeMap::new();
    let mut current_run: Option<u32> = None;
    let mut warnings = Vec::new();

    for line in lines {
        match &line.record {
            LogbookRecord::RunStart {
                run,
                geometry,
                comment,
                ..
            } => {
                if let Some(prev) = current_run {
                    let prev_open = accs.get(&prev).map(|a| a.stop.is_none()).unwrap_or(false);
                    if prev != *run && prev_open {
                        warnings.push(format!(
                            "run {prev}: run_start for run {run} arrived before run {prev} \
                             got a run_stop (treated as incomplete)"
                        ));
                    }
                }
                accs.insert(
                    *run,
                    RunAcc {
                        geometry_path: geometry.path.clone(),
                        start_comment: comment.clone(),
                        start_ts: line.ts.clone(),
                        comments: Vec::new(),
                        stop: None,
                    },
                );
                current_run = Some(*run);
            }
            LogbookRecord::RunStop {
                run,
                reason,
                counters,
                files,
                ..
            } => {
                match accs.get_mut(run) {
                    Some(acc) => {
                        let n_events = counters
                            .events_built
                            .unwrap_or_else(|| counters.frames.values().sum());
                        let ts_from_files = files.iter().find_map(|f| asad_graw_ts(&f.path));
                        if ts_from_files.is_none() {
                            warnings.push(format!(
                                "run {run}: no CoBo*_AsAd*_<ts>_*.graw entry in run_stop.files; \
                                 falling back to run_start timestamp for run_id/files"
                            ));
                        }
                        acc.stop = Some(RunStopSnapshot {
                            reason: reason.clone(),
                            n_events,
                            ts_from_files,
                        });
                    }
                    None => {
                        warnings.push(format!(
                            "run {run}: run_stop with no matching run_start (skipped)"
                        ));
                    }
                }
                if current_run == Some(*run) {
                    current_run = None;
                }
            }
            LogbookRecord::Comment { text, .. } => {
                if let Some(run) = current_run {
                    if let Some(acc) = accs.get_mut(&run) {
                        acc.comments.push(text.clone());
                    }
                }
            }
            // audit / psu はこの変換の対象外(run 条件を持たない)。
            LogbookRecord::Audit { .. } | LogbookRecord::Psu { .. } => {}
        }
    }

    let runs = accs
        .into_iter()
        .map(|(run, acc)| build_entry(run, acc, defaults, data_root))
        .collect();

    (RunsDoc { runs }, warnings)
}

fn build_entry(run: u32, acc: RunAcc, defaults: &Defaults, data_root: &str) -> RunEntry {
    let geometry = basename(&acc.geometry_path);

    let (ts, n_events) = match &acc.stop {
        Some(stop) => {
            let ts = stop
                .ts_from_files
                .clone()
                .unwrap_or_else(|| ts_without_offset(&acc.start_ts));
            (ts, stop.n_events)
        }
        None => (ts_without_offset(&acc.start_ts), 0),
    };

    let files = format!("{data_root}/CoBo*_{ts}_*.graw");

    let over = defaults.runs.get(&run);
    let pressure_mbar = over.and_then(|o| o.pressure_mbar);
    let drift_hv_v = over.and_then(|o| o.drift_hv_v);
    let source = over.and_then(|o| o.source.clone());

    let gas = defaults.gas.clone();
    let drift_velocity_cm_us =
        compute_drift_velocity_cm_us(gas.as_deref(), pressure_mbar, drift_hv_v);

    let mut notes_parts: Vec<String> = Vec::new();
    let normalize = |s: &str| s.replace(['\n', '\r'], " ");
    if !acc.start_comment.is_empty() {
        notes_parts.push(normalize(&acc.start_comment));
    }
    for c in &acc.comments {
        notes_parts.push(normalize(c));
    }
    match &acc.stop {
        Some(stop) if stop.reason != "normal" => {
            notes_parts.push(format!("stop reason: {}", stop.reason));
        }
        None => notes_parts.push("no run_stop".to_string()),
        _ => {}
    }
    let notes = notes_parts.join("; ");

    RunEntry {
        run_id: ts,
        files,
        format: "graw".to_string(),
        n_events,
        geometry,
        hypothesis: defaults.hypothesis.clone(),
        gas,
        pressure_mbar,
        temperature_k: defaults.temperature_k,
        drift_hv_v,
        drift_velocity_cm_us,
        sampling_rate_mhz: defaults.sampling_rate_mhz,
        source_primary: source.as_ref().and_then(|s| s.primary.clone()),
        source_background: source.as_ref().and_then(|s| s.background.clone()),
        notes,
    }
}

// ---------------------------------------------------------------------
// 自前 YAML エミッタ(この固定スキーマだけ、依存追加なし)
// ---------------------------------------------------------------------

/// 二重引用符文字列(バックスラッシュ・引用符・制御文字をエスケープ)。
fn yaml_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn yaml_opt_str(s: &Option<String>) -> String {
    match s {
        Some(v) => yaml_str(v),
        None => "null".to_string(),
    }
}

/// `unknown_comment` は `drift_velocity_cm_us` の `null` にだけ `# unknown` を付ける
/// (発注書: 「それ以外は null とコメント `# unknown` で埋める」— この 1 フィールドの
/// 話であり、他の数値フィールドの null には付けない)。
fn yaml_opt_f64(value: Option<f64>, unknown_comment: bool) -> String {
    match value {
        Some(v) => format!("{v}"),
        None if unknown_comment => "null   # unknown".to_string(),
        None => "null".to_string(),
    }
}

/// `RunsDoc` を発注書 082 の固定スキーマ YAML 文字列にする。
pub fn render(doc: &RunsDoc) -> String {
    if doc.runs.is_empty() {
        return "runs: []\n".to_string();
    }
    let mut out = String::from("runs:\n");
    for r in &doc.runs {
        out.push_str(&format!("  - run_id: {}\n", yaml_str(&r.run_id)));
        out.push_str(&format!("    files: {}\n", yaml_str(&r.files)));
        out.push_str(&format!("    format: {}\n", yaml_str(&r.format)));
        out.push_str(&format!("    n_events: {}\n", r.n_events));
        out.push_str(&format!("    geometry: {}\n", yaml_str(&r.geometry)));
        out.push_str("    reco:\n");
        out.push_str(&format!(
            "      hypothesis: {}\n",
            yaml_opt_str(&r.hypothesis)
        ));
        out.push_str("    conditions:\n");
        out.push_str(&format!("      gas: {}\n", yaml_opt_str(&r.gas)));
        out.push_str(&format!(
            "      pressure_mbar: {}\n",
            yaml_opt_f64(r.pressure_mbar, false)
        ));
        out.push_str(&format!(
            "      temperature_K: {}\n",
            yaml_opt_f64(r.temperature_k, false)
        ));
        out.push_str(&format!(
            "      drift_hv_V: {}\n",
            yaml_opt_f64(r.drift_hv_v, false)
        ));
        out.push_str(&format!(
            "      drift_velocity_cm_us: {}\n",
            yaml_opt_f64(r.drift_velocity_cm_us, true)
        ));
        out.push_str(&format!(
            "      sampling_rate_MHz: {}\n",
            yaml_opt_f64(r.sampling_rate_mhz, false)
        ));
        out.push_str("    source:\n");
        out.push_str(&format!(
            "      primary: {}\n",
            yaml_opt_str(&r.source_primary)
        ));
        out.push_str(&format!(
            "      background: {}\n",
            yaml_opt_str(&r.source_background)
        ));
        if r.notes.is_empty() {
            out.push_str("    notes: \"\"\n");
        } else {
            out.push_str("    notes: >\n");
            out.push_str(&format!("      {}\n", r.notes));
        }
    }
    out
}

// ---------------------------------------------------------------------
// テスト(仕様書 — 先に書く)
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::logbook::{CoboListen, Counters, Geometry, OutputFile};

    // -------------------------------------------------------------
    // 受け入れ 1: SPEC §9.2 run_start 例 + logbook.rs run_stop オラクルを流用した
    // 文字列ゴールデン(改行・インデント込み)。
    //
    // 値の出典: run_start は SPEC §9.2 の「例:」ブロックそのもの
    // (`src/logbook.rs::tests::run_start_matches_the_spec_example_verbatim` の
    // オラクル文字列と同一)。run_stop は SPEC に文字列例が無いので
    // `src/logbook.rs::tests::run_stop_matches_a_hand_assembled_oracle` のオラクルを
    // 流用し、**run を 58→57 に変更**(run_start と対応させるため。このツールは
    // run 番号で run_start/run_stop を突き合わせるので、由来の異なる 2 つのオラクルを
    // 1 run として組むにはここだけ変える必要がある)+ **files のパスを
    // 実タイムスタンプ入りに変更**(TS 抽出を実演するため。元オラクルはワイヤ表現の
    // 検証が目的で `.../CoBo0_AsAd0_ts_0000.graw` という placeholder だった)。
    // comment は SPEC §9.2 の「例:」ブロック 2 行目そのもの。
    // -------------------------------------------------------------

    fn spec_run_start_line() -> LogbookLine {
        LogbookLine {
            ts: "2026-08-12T15:04:05.123+03:00".to_string(),
            seq: 1042,
            record: LogbookRecord::RunStart {
                actor: "controller".to_string(),
                run: 57,
                config_id: "default".to_string(),
                config_ids: None,
                geometry: Geometry {
                    path: "config/geometry_mini_eTPC.dat".to_string(),
                    sha256: "ab12".to_string(),
                },
                cobos: vec![CoboListen {
                    id: 0,
                    listen: "0.0.0.0:46005".to_string(),
                }],
                operator: "aogaki".to_string(),
                comment: "gas test".to_string(),
                expected_fragments: vec![(0, 0)],
            },
        }
    }

    fn spec_comment_line() -> LogbookLine {
        LogbookLine {
            ts: "2026-08-12T16:10:00.001+03:00".to_string(),
            seq: 1043,
            record: LogbookRecord::Comment {
                actor: "ui".to_string(),
                author: "aogaki".to_string(),
                text: "beam tuned, 4 Hz".to_string(),
            },
        }
    }

    /// logbook.rs の run_stop オラクル(`run_stop_matches_a_hand_assembled_oracle`)を
    /// run=57・実タイムスタンプ入り files で流用したもの。
    fn logbook_run_stop_line_paired_with_run_57() -> LogbookLine {
        let mut frames = BTreeMap::new();
        frames.insert("0".to_string(), 3852u64);
        frames.insert("1".to_string(), 3849u64);
        LogbookLine {
            ts: "2026-08-12T18:00:00.000+03:00".to_string(),
            seq: 1044,
            record: LogbookRecord::RunStop {
                actor: "controller".to_string(),
                run: 57,
                duration_s: 12.5,
                ok: true,
                reason: "normal".to_string(),
                forced_eos: true,
                eos_closed: true,
                counters: Counters {
                    events_built: None,
                    events_incomplete: None,
                    late_fragments: None,
                    frames,
                    overflow_frames: Some(3),
                    malformed: Some(4),
                },
                files: vec![
                    OutputFile {
                        path: "run57/CoBo0_AsAd0_2026-08-12T18:00:00.050_0000.graw".to_string(),
                        bytes: 30_108_672,
                    },
                    OutputFile {
                        path: "run57/run57.root".to_string(),
                        bytes: 1_234_567,
                    },
                ],
            },
        }
    }

    fn spec_defaults() -> Defaults {
        let mut runs = BTreeMap::new();
        runs.insert(
            57,
            RunOverride {
                pressure_mbar: Some(100.0),
                drift_hv_v: Some(1600.0),
                source: Some(SourceToml {
                    primary: Some("Am (5.5 MeV alpha), on beam-entrance axis".to_string()),
                    background: Some(
                        "alphas from welds, emitted from everywhere on the chamber".to_string(),
                    ),
                }),
            },
        );
        Defaults {
            gas: Some("CO2".to_string()),
            temperature_k: Some(293.15),
            sampling_rate_mhz: Some(25.0),
            geometry: None,
            hypothesis: Some("alpha".to_string()),
            runs,
        }
    }

    #[test]
    fn run_start_run_stop_comment_render_matches_the_golden_string_verbatim() {
        let lines = vec![
            spec_run_start_line(),
            spec_comment_line(),
            logbook_run_stop_line_paired_with_run_57(),
        ];
        let defaults = spec_defaults();

        let (doc, warnings) = build_runs_doc(&lines, &defaults, "/data/tpcdaq/run57");
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");

        let yaml = render(&doc);

        // n_events 手計算: events_built が null(root-sink は REP を持たない)なので
        // frames 合計 = 3852 + 3849 = 7701。
        // drift_velocity 手計算: E = 1600/20 = 80 V/cm、0.7233*80/100 = 0.57864 →
        // 4 桁丸め 0.5786(発注書のオラクル値そのもの)。
        // pressure_mbar/drift_hv_V/sampling_rate_MHz は整数値の f64 なので Rust の
        // Display は小数点以下を出さない(100.0 → "100"、実測: rustc で確認済み)。
        assert_eq!(
            yaml,
            "runs:\n\
             \x20 - run_id: \"2026-08-12T18:00:00.050\"\n\
             \x20   files: \"/data/tpcdaq/run57/CoBo*_2026-08-12T18:00:00.050_*.graw\"\n\
             \x20   format: \"graw\"\n\
             \x20   n_events: 7701\n\
             \x20   geometry: \"geometry_mini_eTPC.dat\"\n\
             \x20   reco:\n\
             \x20     hypothesis: \"alpha\"\n\
             \x20   conditions:\n\
             \x20     gas: \"CO2\"\n\
             \x20     pressure_mbar: 100\n\
             \x20     temperature_K: 293.15\n\
             \x20     drift_hv_V: 1600\n\
             \x20     drift_velocity_cm_us: 0.5786\n\
             \x20     sampling_rate_MHz: 25\n\
             \x20   source:\n\
             \x20     primary: \"Am (5.5 MeV alpha), on beam-entrance axis\"\n\
             \x20     background: \"alphas from welds, emitted from everywhere on the chamber\"\n\
             \x20   notes: >\n\
             \x20     gas test; beam tuned, 4 Hz\n"
        );
    }

    // -------------------------------------------------------------
    // 受け入れ 1: null 経路(pressure/drift_hv の上書きが無い run は
    // drift_velocity_cm_us が `null  # unknown`)。
    // -------------------------------------------------------------

    #[test]
    fn drift_velocity_is_null_with_unknown_comment_when_no_override_given() {
        let lines = vec![
            spec_run_start_line(),
            logbook_run_stop_line_paired_with_run_57(),
        ];
        // defaults に [runs.57] を積まない = pressure/drift_hv とも無し。
        let defaults = Defaults {
            gas: Some("CO2".to_string()),
            temperature_k: Some(293.15),
            sampling_rate_mhz: Some(25.0),
            geometry: None,
            hypothesis: Some("alpha".to_string()),
            runs: BTreeMap::new(),
        };

        let (doc, warnings) = build_runs_doc(&lines, &defaults, "/data/tpcdaq/run57");
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        let yaml = render(&doc);

        assert!(yaml.contains("      pressure_mbar: null\n"), "{yaml}");
        assert!(yaml.contains("      drift_hv_V: null\n"), "{yaml}");
        assert!(
            yaml.contains("      drift_velocity_cm_us: null   # unknown\n"),
            "{yaml}"
        );
        // 他の null フィールドには # unknown が付かない(この 1 項目だけの規則)。
        assert!(!yaml.contains("pressure_mbar: null   # unknown"));
    }

    #[test]
    fn drift_velocity_is_null_when_gas_is_not_co2_even_with_full_override() {
        let lines = vec![
            spec_run_start_line(),
            logbook_run_stop_line_paired_with_run_57(),
        ];
        let mut runs = BTreeMap::new();
        runs.insert(
            57,
            RunOverride {
                pressure_mbar: Some(100.0),
                drift_hv_v: Some(1600.0),
                source: None,
            },
        );
        let defaults = Defaults {
            gas: Some("Ar".to_string()),
            temperature_k: None,
            sampling_rate_mhz: None,
            geometry: None,
            hypothesis: None,
            runs,
        };

        let (doc, _warnings) = build_runs_doc(&lines, &defaults, "/data/tpcdaq/run57");
        assert_eq!(doc.runs.len(), 1);
        assert_eq!(doc.runs[0].drift_velocity_cm_us, None);
    }

    // -------------------------------------------------------------
    // 受け入れ 2: 壊れ末尾行(parse_logbook_lines は行番号付きで数え、他の行は読める)。
    // -------------------------------------------------------------

    #[test]
    fn parse_logbook_lines_counts_a_corrupt_trailing_line_with_its_line_number() {
        let mut content = String::new();
        content.push_str(&serde_json::to_string(&spec_run_start_line()).unwrap());
        content.push('\n');
        content.push_str(&serde_json::to_string(&spec_comment_line()).unwrap());
        content.push('\n');
        // torn write を模した壊れた末尾行(改行なし)。
        content.push_str(r#"{"ts":"2026-08-12T18:00:00.000+03:00","seq":1044,"type":"run_s"#);

        let (lines, corrupt) = parse_logbook_lines(&content);

        assert_eq!(lines.len(), 2);
        assert_eq!(
            corrupt,
            vec![CorruptLine {
                line_no: 3,
                message: corrupt[0].message.clone(),
            }]
        );
        assert!(!corrupt[0].message.is_empty());
    }

    /// 末尾に限らず、途中の壊れ行も数える(発注書は「末尾行だけ許容」の logbook.rs と
    /// 違い、読めるだけ読んで壊れ行を可視化する変換ツール向けの寛容な読み)。
    #[test]
    fn parse_logbook_lines_counts_an_interior_corrupt_line_too() {
        let mut content = String::new();
        content.push_str(&serde_json::to_string(&spec_run_start_line()).unwrap());
        content.push('\n');
        content.push_str("not json at all\n");
        content.push_str(&serde_json::to_string(&spec_comment_line()).unwrap());
        content.push('\n');

        let (lines, corrupt) = parse_logbook_lines(&content);

        assert_eq!(lines.len(), 2);
        assert_eq!(corrupt.len(), 1);
        assert_eq!(corrupt[0].line_no, 2);
    }

    // -------------------------------------------------------------
    // run_stop の無い run(進行中/クラッシュ)。
    // -------------------------------------------------------------

    #[test]
    fn a_run_with_no_run_stop_gets_no_run_stop_in_notes_and_falls_back_to_run_start_ts() {
        let lines = vec![spec_run_start_line()];
        let defaults = Defaults::default();

        let (doc, warnings) = build_runs_doc(&lines, &defaults, "/data/tpcdaq");
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        assert_eq!(doc.runs.len(), 1);
        let entry = &doc.runs[0];

        // run_start.ts = "2026-08-12T15:04:05.123+03:00"(29 バイト)→ オフセット落とし
        // = "2026-08-12T15:04:05.123"(先頭 23 バイト)。
        assert_eq!(entry.run_id, "2026-08-12T15:04:05.123");
        assert_eq!(
            entry.files,
            "/data/tpcdaq/CoBo*_2026-08-12T15:04:05.123_*.graw"
        );
        assert_eq!(entry.n_events, 0);
        assert_eq!(entry.notes, "gas test; no run_stop");
    }

    /// `run_stop` に対応する `run_start` が無いと `RunsDoc` には現れず、警告が立つ
    /// (geometry 等が取れないので静かに落とさず可視化する)。
    #[test]
    fn a_run_stop_with_no_matching_run_start_is_skipped_with_a_warning() {
        let lines = vec![logbook_run_stop_line_paired_with_run_57()];
        let defaults = Defaults::default();

        let (doc, warnings) = build_runs_doc(&lines, &defaults, "/data/tpcdaq");
        assert!(doc.runs.is_empty());
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].contains("no matching run_start"),
            "{warnings:?}"
        );
    }

    /// AsAd graw が `run_stop.files` に見つからない(ctrl プローブ接続だけ)場合は
    /// run_start.ts へフォールバックし、警告を積む(041 の実 fixture が実証する経路)。
    #[test]
    fn falls_back_to_run_start_ts_and_warns_when_no_asad_graw_file_is_present() {
        let lines = vec![
            spec_run_start_line(),
            LogbookLine {
                ts: "2026-08-12T18:00:00.000+03:00".to_string(),
                seq: 1044,
                record: LogbookRecord::RunStop {
                    actor: "controller".to_string(),
                    run: 57,
                    duration_s: 1.0,
                    ok: true,
                    reason: "normal".to_string(),
                    forced_eos: true,
                    eos_closed: true,
                    counters: Counters {
                        events_built: None,
                        events_incomplete: None,
                        late_fragments: None,
                        frames: BTreeMap::new(),
                        overflow_frames: Some(0),
                        malformed: Some(0),
                    },
                    // ctrl プローブ接続のみ(AsAd 区画が無い = asad_graw_ts が None)。
                    files: vec![OutputFile {
                        path: "run57/ctrl/CoBo0_2026-08-12T18:00:00.010_0000.graw".to_string(),
                        bytes: 12,
                    }],
                },
            },
        ];
        let defaults = Defaults::default();

        let (doc, warnings) = build_runs_doc(&lines, &defaults, "/data/tpcdaq");
        assert_eq!(doc.runs.len(), 1);
        assert_eq!(doc.runs[0].run_id, "2026-08-12T15:04:05.123");
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].contains("no CoBo*_AsAd*_<ts>_*.graw entry"),
            "{warnings:?}"
        );
    }

    // -------------------------------------------------------------
    // runs_defaults.toml のパース(発注書の例 TOML、改行区切りの正しい TOML に直したもの)。
    // -------------------------------------------------------------

    #[test]
    fn parses_the_order_example_toml() {
        let toml_str = r#"
[defaults]
gas = "CO2"
temperature_K = 293.15
sampling_rate_MHz = 25.0
geometry = "geometry_mini_eTPC.dat"
hypothesis = "alpha"

[runs.57]
pressure_mbar = 100.0
drift_hv_V = 1600.0
source = "Am (5.5 MeV alpha), on beam-entrance axis"
"#;
        let defaults = parse_defaults_toml(toml_str).unwrap();

        assert_eq!(defaults.gas.as_deref(), Some("CO2"));
        assert_eq!(defaults.temperature_k, Some(293.15));
        assert_eq!(defaults.sampling_rate_mhz, Some(25.0));
        assert_eq!(defaults.geometry.as_deref(), Some("geometry_mini_eTPC.dat"));
        assert_eq!(defaults.hypothesis.as_deref(), Some("alpha"));

        let over = defaults.runs.get(&57).expect("runs.57 missing");
        assert_eq!(over.pressure_mbar, Some(100.0));
        assert_eq!(over.drift_hv_v, Some(1600.0));
        // フラット文字列 = primary のみの略記、background は None。
        assert_eq!(
            over.source,
            Some(SourceToml {
                primary: Some("Am (5.5 MeV alpha), on beam-entrance axis".to_string()),
                background: None,
            })
        );
    }

    #[test]
    fn source_can_be_written_as_a_table_with_both_primary_and_background() {
        let toml_str = r#"
[runs.3]
source = { primary = "Am (5.5 MeV alpha)", background = "welds" }
"#;
        let defaults = parse_defaults_toml(toml_str).unwrap();
        let over = defaults.runs.get(&3).expect("runs.3 missing");
        assert_eq!(
            over.source,
            Some(SourceToml {
                primary: Some("Am (5.5 MeV alpha)".to_string()),
                background: Some("welds".to_string()),
            })
        );
    }

    #[test]
    fn an_empty_defaults_file_parses_to_all_none() {
        let defaults = parse_defaults_toml("").unwrap();
        assert_eq!(defaults, Defaults::default());
    }

    #[test]
    fn an_invalid_run_key_is_an_error_not_a_silent_skip() {
        // TOML の `[runs.abc]` は文字列キー "abc" になる —— u32 として不正なので Err。
        let toml_str = "[runs.abc]\npressure_mbar = 1.0\n";
        let err = parse_defaults_toml(toml_str).unwrap_err();
        assert!(matches!(err, DefaultsError::InvalidRunKey(ref k) if k == "abc"));
    }

    #[test]
    fn unknown_top_level_key_is_rejected() {
        let toml_str = "bogus = 1\n";
        assert!(parse_defaults_toml(toml_str).is_err());
    }

    // -------------------------------------------------------------
    // ヘルパ単体(private だが同一クレート内なのでテストできる)。
    // -------------------------------------------------------------

    #[test]
    fn asad_graw_ts_extracts_the_ts_segment() {
        assert_eq!(
            asad_graw_ts("run57/CoBo0_AsAd0_2026-08-12T18:00:00.050_0000.graw"),
            Some("2026-08-12T18:00:00.050".to_string())
        );
    }

    #[test]
    fn asad_graw_ts_rejects_ctrl_probe_filenames_without_an_asad_segment() {
        assert_eq!(
            asad_graw_ts("run57/ctrl/CoBo0_2026-08-12T18:00:00.010_0000.graw"),
            None
        );
    }

    #[test]
    fn ts_without_offset_strips_a_well_formed_rfc3339_offset() {
        assert_eq!(
            ts_without_offset("2026-08-12T15:04:05.123+03:00"),
            "2026-08-12T15:04:05.123"
        );
    }

    #[test]
    fn ts_without_offset_returns_input_unchanged_when_not_well_formed() {
        assert_eq!(ts_without_offset("not-a-timestamp"), "not-a-timestamp");
    }
}
