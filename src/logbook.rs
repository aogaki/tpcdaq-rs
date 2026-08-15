//! JSONL ログブック(SPEC §9)。
//!
//! `<output_root>/logbook.jsonl` への追記専用ログ。**書き手は controller ただ一人**
//! (SPEC §9.1)。このモジュールは純粋な読み書きだけを提供し、誰がいつ何を書くかの
//! 判断(REST/監査/LogPost の配線)は 016(controller)の仕事。
//!
//! # ワイヤ表現
//!
//! 1 行 = 1 JSON オブジェクト(改行区切り、JSONL)。共通フィールド `ts` / `seq` / `type` /
//! `actor` の後に、型ごとの追加フィールドが続く(SPEC §9.2 の表そのまま)。
//! `type` は [`LogbookRecord`] の internally-tagged enum(`#[serde(tag = "type")]`)が
//! 自動で出す。フィールド名・順序の正本は [`schema`] の定数表で、ズレはテストが検出する
//! (SPEC §2.5 の方式踏襲)。
//!
//! # クラッシュ耐性(SPEC §9.1)
//!
//! 途中で落ちた write は**末尾行だけ**壊れうる。[`LogbookWriter::open`] はファイル末尾から
//! 遡って最初に parse できた行の `seq` を回復に使い(壊れた末尾行は無視)、
//! [`read_since`] は最終行のみ parse 失敗を許容して警告フラグを立てる。
//! **壊れた末尾行はファイルに残ったまま次の write が続く点に注意**
//! (追記オープンなので巻き戻し・削除はしない)。以後の [`read_since`] 呼び出しがその行に
//! 到達すると、その時点で `Err`(SPEC §9.1 の許容は「クラッシュ直後の末尾行」だけであり、
//! 埋もれた壊れ行を silent に読み飛ばしはしない — CLAUDE.md)。

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::ConfigIds;
use std::collections::BTreeMap;
use tracing::warn;

// ---------------------------------------------------------------------
// エラー
// ---------------------------------------------------------------------

/// ログブックの読み書きで起きうる失敗。
///
/// 破損行を silent に読み飛ばさないための型でもある(CLAUDE.md「silent failure を作らない」)。
#[derive(Debug, thiserror::Error)]
pub enum LogbookError {
    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to encode logbook line: {0}")]
    Encode(#[from] serde_json::Error),

    /// 最終行以外の破損(SPEC §9.1「それ以外の行の破損は Err」)。
    #[error("logbook line {line_no} in {path} is corrupt: {source}")]
    CorruptLine {
        path: PathBuf,
        line_no: usize,
        #[source]
        source: serde_json::Error,
    },
}

// ---------------------------------------------------------------------
// レコード補助型(SPEC §9.2 の入れ子構造)
// ---------------------------------------------------------------------

/// `run_start.geometry`(SPEC §9.2)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Geometry {
    pub path: String,
    pub sha256: String,
}

/// `run_start.cobos` の 1 要素(SPEC §9.2)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoboListen {
    pub id: u32,
    pub listen: String,
}

/// `run_stop.counters`(SPEC §9.2)。`frames` は cobo id(文字列)→ フレーム数。
///
/// JSON のオブジェクトキーはもともと文字列なので `BTreeMap<String, u64>` にしてある
/// (`BTreeMap<u32, _>` にすると、`type` タグ + `#[serde(flatten)]` の内部実装が使う
/// content ベースの deserializer が数値キーへの復元に対応しておらず round-trip できない —
/// 実装時に `cargo test` で確認済み)。`HashMap` ではなく `BTreeMap` にしてあるのは
/// 決定的な順序でシリアライズするため。
///
/// # `Option<u64>`(025、SPEC v1.10 §9.2 — 016 逸脱③の確定処置)
///
/// `frames` を除く 5 項目は **`None` = その時点で取得不能で、`0`(=実測ゼロ)とは別の意味**。
/// 016 では `u64` 非 Option で取得不能を `0` 埋めしていたが、それだと「0 イベント」と
/// 「取得不能」がログブック上で区別できず記録品質を損なう(016 レビュー逸脱③)。
/// serde の既定 `Option<T>` 直列化がそのまま「取得不能 → JSON `null`」になるので追加の
/// 属性は要らない。`controller::counters_from` は root-sink 由来(REP を持たない)
/// `events_built` / `events_incomplete` / `late_fragments` を常に `None` にし、
/// GetStatus から実測できる `overflow_frames` / `malformed` は `Some` にする。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Counters {
    pub events_built: Option<u64>,
    pub events_incomplete: Option<u64>,
    pub late_fragments: Option<u64>,
    pub frames: BTreeMap<String, u64>,
    pub overflow_frames: Option<u64>,
    pub malformed: Option<u64>,
}

/// `run_stop.files` の 1 要素(SPEC §9.2)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutputFile {
    pub path: String,
    pub bytes: u64,
}

/// `psu.values`(SPEC §9.2、P6 で詳細化予定)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PsuValues {
    pub vmon: f64,
    pub imon: f64,
    pub vset: f64,
}

// ---------------------------------------------------------------------
// レコード本体(SPEC §9.2)
// ---------------------------------------------------------------------

/// ログブック 1 行の中身(SPEC §9.2 の 5 型)。`actor` は表では共通列だが、
/// `type` タグの直後に来る必要があるため各バリアントの先頭フィールドとして持つ
/// (internally-tagged enum は「タグ → バリアントのフィールドを宣言順」で出力する)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum LogbookRecord {
    #[serde(rename = "run_start")]
    RunStart {
        actor: String,
        run: u32,
        /// 3 相同値ならその値、非同値なら **configure 相**の id(SPEC §9.2 v1.13)。
        config_id: String,
        /// 3 相が**非同値のときだけ**出る(同値なら省略 —— null / 省略 ≠ 空値の
        /// nullable 規律、SPEC §9.2 v1.13)。v1.13 より前の行には無いので `default`。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        config_ids: Option<ConfigIds>,
        geometry: Geometry,
        cobos: Vec<CoboListen>,
        operator: String,
        comment: String,
        /// 期待 (cobo, asad) 集合(SPEC §9.2)。
        expected_fragments: Vec<(u32, u32)>,
    },

    #[serde(rename = "run_stop")]
    RunStop {
        actor: String,
        run: u32,
        duration_s: f64,
        ok: bool,
        /// `"normal"` / `"error:..."`(SPEC §9.2)。
        reason: String,
        counters: Counters,
        files: Vec<OutputFile>,
    },

    #[serde(rename = "audit")]
    Audit {
        actor: String,
        /// REST エンドポイント名(SPEC §9.2)。
        action: String,
        /// パラメタの要約。`command::RunConfig::config` / `CommandResponse::metrics` と同様、
        /// 呼び手ごとに形が違う自由記述 JSON なので `serde_json::Value`(既存の流儀)。
        params: Value,
        operator: String,
        ok: bool,
        error: Option<String>,
    },

    #[serde(rename = "comment")]
    Comment {
        actor: String,
        author: String,
        text: String,
    },

    #[serde(rename = "psu")]
    Psu {
        actor: String,
        device: String,
        channel: u32,
        /// `"TRIP"` / `"ON"` / `"OFF"` / `"VSET"` / ...(SPEC §9.2、P6 で詳細化)。
        event: String,
        values: PsuValues,
    },
}

/// ログブック 1 行(`ts` + `seq` + レコード本体、SPEC §9.2 共通フィールド)。
///
/// `record` を `#[serde(flatten)]` することで `{"ts":...,"seq":...,"type":...,...}` の
/// フラットな 1 オブジェクトになる([`LogbookRecord`] 単体なら `{"type":...,...}`)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogbookLine {
    /// RFC3339・ミリ秒・ローカルオフセット付き(SPEC §9.1、例 `"2026-08-12T15:04:05.123+03:00"`)。
    pub ts: String,
    /// 単調増加(SPEC §9.1)。
    pub seq: u64,
    #[serde(flatten)]
    pub record: LogbookRecord,
}

// ---------------------------------------------------------------------
// スキーマ漂流ガード(SPEC §2.5 の方式を JSONL へ踏襲)
// ---------------------------------------------------------------------

/// ワイヤ表現(フィールド名・順序)の正本表。実構造体とズレたらテストが落ちる
/// ([`crate::msg::schema`] と同じ考え方)。
pub mod schema {
    /// 全レコード型に共通の先頭 4 フィールド(SPEC §9.2 表頭)。
    pub const COMMON: &[&str] = &["ts", "seq", "type", "actor"];

    /// `run_start` の `actor` 以降(SPEC §9.2)。3 相同値の形 = `config_ids` は出ない。
    pub const RUN_START: &[&str] = &[
        "run",
        "config_id",
        "geometry",
        "cobos",
        "operator",
        "comment",
        "expected_fragments",
    ];

    /// `run_start` の `actor` 以降で、ConfigId の 3 相が**非同値**のとき(v1.13)。
    /// `config_ids` は `config_id` の直後に入る。
    pub const RUN_START_WITH_CONFIG_IDS: &[&str] = &[
        "run",
        "config_id",
        "config_ids",
        "geometry",
        "cobos",
        "operator",
        "comment",
        "expected_fragments",
    ];

    /// `run_stop` の `actor` 以降(SPEC §9.2)。
    pub const RUN_STOP: &[&str] = &["run", "duration_s", "ok", "reason", "counters", "files"];

    /// `audit` の `actor` 以降(SPEC §9.2)。
    pub const AUDIT: &[&str] = &["action", "params", "operator", "ok", "error"];

    /// `comment` の `actor` 以降(SPEC §9.2)。
    pub const COMMENT: &[&str] = &["author", "text"];

    /// `psu` の `actor` 以降(SPEC §9.2)。
    pub const PSU: &[&str] = &["device", "channel", "event", "values"];
}

// ---------------------------------------------------------------------
// 内部ヘルパ
// ---------------------------------------------------------------------

/// SPEC §9.1「RFC3339、ミリ秒、ローカルオフセット付き」で現在時刻を書式化する。
fn now_rfc3339_millis() -> String {
    chrono::Local::now()
        .format("%Y-%m-%dT%H:%M:%S%.3f%:z")
        .to_string()
}

fn parse_line(line: &str) -> Result<LogbookLine, serde_json::Error> {
    serde_json::from_str(line)
}

fn io_err(path: &Path, source: std::io::Error) -> LogbookError {
    LogbookError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// [`recover_next_seq_with_skip`] の内部実装。次に振る `seq` と、末尾から遡って
/// スキップした(parse できなかった)行数を両方返す(025)。
///
/// スキップ数 0 = 末尾行がそのまま parse できた(通常終了)。スキップ数 1 = SPEC §9.1 が
/// 明示的に許容する「末尾 1 行だけ壊れる」経路。2 以上はその前提から外れているので、
/// 呼び出し元([`recover_next_seq`])が warn する。
fn recover_next_seq_with_skip(path: &Path) -> Result<(u64, usize), LogbookError> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((1, 0)),
        Err(e) => return Err(io_err(path, e)),
    };
    let mut skipped = 0usize;
    for line in content.lines().rev() {
        if let Ok(parsed) = parse_line(line) {
            return Ok((parsed.seq + 1, skipped));
        }
        skipped += 1;
    }
    Ok((1, skipped))
}

/// ファイル末尾から遡って最初に parse できた行の `seq + 1` を返す(SPEC §9.1「壊れた最終行は
/// 無視」)。ファイル不在・全行破損は `1` から。**公開挙動は不変**(025 以前と同じ `Result<u64, _>`)
/// —— 内部でスキップした行数だけ追加で見て、SPEC §9.1 の「末尾 1 行だけ壊れる」前提から
/// 外れたとき(スキップ > 1)は `warn!` で可視化する(silent failure を作らない — CLAUDE.md)。
fn recover_next_seq(path: &Path) -> Result<u64, LogbookError> {
    let (next_seq, skipped) = recover_next_seq_with_skip(path)?;
    if skipped > 1 {
        warn!(
            path = %path.display(),
            skipped,
            "logbook recovery skipped more than one trailing corrupt line — \
             SPEC §9.1 only allows the very last line to be corrupt"
        );
    }
    Ok(next_seq)
}

// ---------------------------------------------------------------------
// Writer(SPEC §9.1「書き手は controller ただ一人」)
// ---------------------------------------------------------------------

/// 単一書き手の JSONL 追記器。
pub struct LogbookWriter {
    file: std::fs::File,
    path: PathBuf,
    next_seq: u64,
}

impl LogbookWriter {
    /// 追記オープン。既存ファイルがあれば末尾から `seq` を回復する。
    pub fn open(path: impl AsRef<Path>) -> Result<Self, LogbookError> {
        let path = path.as_ref().to_path_buf();
        let next_seq = recover_next_seq(&path)?;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| io_err(&path, e))?;
        Ok(Self {
            file,
            path,
            next_seq,
        })
    }

    /// 次に振られる `seq`(テスト・診断用)。
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// `ts` と `seq` を付与して 1 行追記する。**1 行 = 1 write(2)**
    /// (行全体を文字列に組んでから `write_all` を 1 回呼ぶ)+ 呼び出し毎に `flush`。
    /// 成功したら割り当てた `seq` を返す。
    pub fn append(&mut self, record: LogbookRecord) -> Result<u64, LogbookError> {
        let seq = self.next_seq;
        let line = LogbookLine {
            ts: now_rfc3339_millis(),
            seq,
            record,
        };
        let mut json = serde_json::to_string(&line)?;
        json.push('\n');
        self.file
            .write_all(json.as_bytes())
            .map_err(|e| io_err(&self.path, e))?;
        self.file.flush().map_err(|e| io_err(&self.path, e))?;
        self.next_seq += 1;
        Ok(seq)
    }
}

// ---------------------------------------------------------------------
// Reader(SPEC §9.1、016 の REST `GET /api/logbook?since_seq=N` 用)
// ---------------------------------------------------------------------

/// [`read_since`] の戻り値。
#[derive(Debug, Clone, PartialEq)]
pub struct ReadResult {
    /// `seq > since_seq` の行のみ、ファイル中の順序のまま。
    pub records: Vec<LogbookLine>,
    /// 末尾行が破損していて無視されたか(SPEC §9.1)。
    pub tail_corrupt: bool,
}

/// `since_seq` より後のレコードを読む。**最終行のみ parse 失敗を許容**する
/// (それ以外の行の破損は `Err` — silent にしない、SPEC §9.1)。
pub fn read_since(path: impl AsRef<Path>, since_seq: u64) -> Result<ReadResult, LogbookError> {
    let path = path.as_ref();
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ReadResult {
                records: Vec::new(),
                tail_corrupt: false,
            });
        }
        Err(e) => return Err(io_err(path, e)),
    };

    let lines: Vec<&str> = content.lines().collect();
    let mut records = Vec::new();
    let mut tail_corrupt = false;

    for (idx, line) in lines.iter().enumerate() {
        match parse_line(line) {
            Ok(parsed) => {
                if parsed.seq > since_seq {
                    records.push(parsed);
                }
            }
            Err(source) => {
                if idx + 1 == lines.len() {
                    tail_corrupt = true;
                } else {
                    return Err(LogbookError::CorruptLine {
                        path: path.to_path_buf(),
                        line_no: idx + 1,
                        source,
                    });
                }
            }
        }
    }

    Ok(ReadResult {
        records,
        tail_corrupt,
    })
}

// ---------------------------------------------------------------------
// テスト(仕様書 — 先に書く)
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// テスト用に一意な一時ファイルパスを作る(既存ファイルなしの状態から始める)。
    fn temp_path(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "tpcdaq-logbook-test-{tag}-{}-{n}.jsonl",
            std::process::id()
        ))
    }

    fn sample_run_start() -> LogbookRecord {
        LogbookRecord::RunStart {
            actor: "controller".to_string(),
            run: 57,
            config_id: "default".to_string(),
            // 3 相同値(既存設定の形)なので出さない —— ワイヤは v1.13 前と 1 バイトも変わらない。
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
        }
    }

    fn sample_run_stop() -> LogbookRecord {
        // 非対称な値(取り違え検出用): frames は 2 CoBo 分、overflow_frames と malformed は
        // 相異なる。
        //
        // 申し送り(025、SPEC v1.10 §9.2 — 016 逸脱③の確定処置): 016 は
        // events_built/events_incomplete/late_fragments(root-sink 由来、REP が無く常に
        // 取得不能)を `0` 埋めしていた。「0 イベント」と「取得不能」が区別できないため、
        // 025 で `Counters` を `Option<u64>` 化し、この 3 項目は `None`(→ JSON `null`)にした。
        // GetStatus から実測できる overflow_frames/malformed は `Some` のまま数値で出る —
        // これが現実の controller(`counters_from`)の出力そのもの。
        let mut frames = BTreeMap::new();
        frames.insert("0".to_string(), 3852);
        frames.insert("1".to_string(), 3849);
        LogbookRecord::RunStop {
            actor: "controller".to_string(),
            run: 58,
            duration_s: 12.5,
            ok: true,
            reason: "normal".to_string(),
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
                    path: "run58/CoBo0_AsAd0_ts_0000.graw".to_string(),
                    bytes: 30_108_672,
                },
                OutputFile {
                    path: "run58/run58.root".to_string(),
                    bytes: 1_234_567,
                },
            ],
        }
    }

    fn sample_audit() -> LogbookRecord {
        LogbookRecord::Audit {
            actor: "controller".to_string(),
            action: "run/start".to_string(),
            params: serde_json::json!({"run_number": 58}),
            operator: "aogaki".to_string(),
            ok: false,
            error: Some("listen-before-start violated".to_string()),
        }
    }

    fn sample_comment() -> LogbookRecord {
        LogbookRecord::Comment {
            actor: "ui".to_string(),
            author: "aogaki".to_string(),
            text: "beam tuned, 4 Hz".to_string(),
        }
    }

    fn sample_psu() -> LogbookRecord {
        LogbookRecord::Psu {
            actor: "psu".to_string(),
            device: "CAEN0".to_string(),
            channel: 3,
            event: "TRIP".to_string(),
            values: PsuValues {
                vmon: 350.2,
                imon: 0.008,
                vset: 350.0,
            },
        }
    }

    // -----------------------------------------------------------------
    // ワイヤ表現(golden 文字列照合)
    // -----------------------------------------------------------------

    /// SPEC §9.2 の `run_start` 例をそのまま転記したオラクル文字列。
    #[test]
    fn run_start_matches_the_spec_example_verbatim() {
        let line = LogbookLine {
            ts: "2026-08-12T15:04:05.123+03:00".to_string(),
            seq: 1042,
            record: sample_run_start(),
        };
        let json = serde_json::to_string(&line).unwrap();
        assert_eq!(
            json,
            r#"{"ts":"2026-08-12T15:04:05.123+03:00","seq":1042,"type":"run_start","actor":"controller","run":57,"config_id":"default","geometry":{"path":"config/geometry_mini_eTPC.dat","sha256":"ab12"},"cobos":[{"id":0,"listen":"0.0.0.0:46005"}],"operator":"aogaki","comment":"gas test","expected_fragments":[[0,0]]}"#
        );
    }

    /// SPEC §9.2 の `comment` 例をそのまま転記したオラクル文字列。
    #[test]
    fn comment_matches_the_spec_example_verbatim() {
        let line = LogbookLine {
            ts: "2026-08-12T16:10:00.001+03:00".to_string(),
            seq: 1043,
            record: sample_comment(),
        };
        let json = serde_json::to_string(&line).unwrap();
        assert_eq!(
            json,
            r#"{"ts":"2026-08-12T16:10:00.001+03:00","seq":1043,"type":"comment","actor":"ui","author":"aogaki","text":"beam tuned, 4 Hz"}"#
        );
    }

    /// `run_stop` は SPEC に文字列例がないので、§9.2 の表のフィールド順に手で組んだオラクル。
    ///
    /// 025 で更新(golden 変更点): counters の `events_built`/`events_incomplete`/
    /// `late_fragments` が `0` から `null` に変わった(`Option<u64>` 化、`sample_run_stop`
    /// の申し送りコメント参照)。`overflow_frames`/`malformed` は `Some` なので数値のまま
    /// (`Option<u64>` の既定直列化は `Some(n)` → `n` そのもの、`None` → `null`)。
    #[test]
    fn run_stop_matches_a_hand_assembled_oracle() {
        let line = LogbookLine {
            ts: "2026-08-12T18:00:00.000+03:00".to_string(),
            seq: 7,
            record: sample_run_stop(),
        };
        let json = serde_json::to_string(&line).unwrap();
        assert_eq!(
            json,
            r#"{"ts":"2026-08-12T18:00:00.000+03:00","seq":7,"type":"run_stop","actor":"controller","run":58,"duration_s":12.5,"ok":true,"reason":"normal","counters":{"events_built":null,"events_incomplete":null,"late_fragments":null,"frames":{"0":3852,"1":3849},"overflow_frames":3,"malformed":4},"files":[{"path":"run58/CoBo0_AsAd0_ts_0000.graw","bytes":30108672},{"path":"run58/run58.root","bytes":1234567}]}"#
        );
    }

    // -----------------------------------------------------------------
    // config_ids(SPEC §9.2 v1.13、TODO/042)
    // -----------------------------------------------------------------

    /// 3 相非同値の `run_start`。`config_id` は **configure 相**、`config_ids` に 3 相全部。
    /// 3 値は非対称(相の取り違え検出用)。
    fn sample_run_start_per_phase() -> LogbookRecord {
        LogbookRecord::RunStart {
            actor: "controller".to_string(),
            run: 57,
            config_id: "pulser".to_string(),
            config_ids: Some(ConfigIds {
                describe: "zCobo-ZC706".to_string(),
                prepare: "prep-xcfg".to_string(),
                configure: "pulser".to_string(),
            }),
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
        }
    }

    /// 3 相同値なら `config_ids` は**出ない**(省略 ≠ 空値 —— nullable 規律、SPEC §9.2)。
    /// `run_start_matches_the_spec_example_verbatim` の golden が無変更で通ることと合わせて、
    /// v1.13 が既存のワイヤ表現を 1 バイトも動かしていないことの証明。
    #[test]
    fn run_start_omits_config_ids_when_the_three_phases_agree() {
        let line = LogbookLine {
            ts: "2026-08-12T15:04:05.123+03:00".to_string(),
            seq: 1042,
            record: sample_run_start(),
        };
        let json = serde_json::to_string(&line).unwrap();
        assert!(!json.contains("config_ids"), "{json}");
    }

    /// 非同値なら `config_id` の**直後**に `config_ids` が入る(SPEC §9.2 の表順)。
    #[test]
    fn run_start_carries_config_ids_when_the_phases_differ() {
        let line = LogbookLine {
            ts: "2026-08-12T15:04:05.123+03:00".to_string(),
            seq: 1042,
            record: sample_run_start_per_phase(),
        };
        let json = serde_json::to_string(&line).unwrap();
        assert_eq!(
            json,
            r#"{"ts":"2026-08-12T15:04:05.123+03:00","seq":1042,"type":"run_start","actor":"controller","run":57,"config_id":"pulser","config_ids":{"describe":"zCobo-ZC706","prepare":"prep-xcfg","configure":"pulser"},"geometry":{"path":"config/geometry_mini_eTPC.dat","sha256":"ab12"},"cobos":[{"id":0,"listen":"0.0.0.0:46005"}],"operator":"aogaki","comment":"gas test","expected_fragments":[[0,0]]}"#
        );
        assert_eq!(
            top_level_keys(&json),
            expected_keys(schema::RUN_START_WITH_CONFIG_IDS)
        );
    }

    /// v1.13 より前に書かれた行(`config_ids` 無し)もそのまま読める(前方互換)。
    #[test]
    fn run_start_without_config_ids_still_parses() {
        let line = r#"{"ts":"2026-08-12T15:04:05.123+03:00","seq":1042,"type":"run_start","actor":"controller","run":57,"config_id":"default","geometry":{"path":"config/geometry_mini_eTPC.dat","sha256":"ab12"},"cobos":[{"id":0,"listen":"0.0.0.0:46005"}],"operator":"aogaki","comment":"gas test","expected_fragments":[[0,0]]}"#;
        let parsed: LogbookLine = serde_json::from_str(line).unwrap();
        assert_eq!(parsed.record, sample_run_start());
    }

    // -----------------------------------------------------------------
    // Counters の Option<u64> 直列化(025、SPEC v1.10 §9.2)
    // -----------------------------------------------------------------

    /// `None` は `null`(`0` と混同しない — Counters の doc コメント参照)。5 項目とも
    /// 相異なる値ではなく全部 `None` にして「取得不能」経路をまとめて機械照合する。
    #[test]
    fn counters_none_serializes_as_null_not_zero() {
        let counters = Counters {
            events_built: None,
            events_incomplete: None,
            late_fragments: None,
            frames: BTreeMap::new(),
            overflow_frames: None,
            malformed: None,
        };
        let json = serde_json::to_string(&counters).unwrap();
        assert_eq!(
            json,
            r#"{"events_built":null,"events_incomplete":null,"late_fragments":null,"frames":{},"overflow_frames":null,"malformed":null}"#
        );
        let back: Counters = serde_json::from_str(&json).unwrap();
        assert_eq!(back, counters);
    }

    /// `Some` は中身の数値がそのまま出る(`null` にならない)— Option 化が既存の
    /// 「取れた値」経路を壊していないことの確認。非対称な値(取り違え検出用)。
    #[test]
    fn counters_some_serializes_as_the_plain_number_not_null() {
        let counters = Counters {
            events_built: Some(11),
            events_incomplete: Some(22),
            late_fragments: Some(33),
            frames: BTreeMap::new(),
            overflow_frames: Some(44),
            malformed: Some(55),
        };
        let json = serde_json::to_string(&counters).unwrap();
        assert_eq!(
            json,
            r#"{"events_built":11,"events_incomplete":22,"late_fragments":33,"frames":{},"overflow_frames":44,"malformed":55}"#
        );
        let back: Counters = serde_json::from_str(&json).unwrap();
        assert_eq!(back, counters);
    }

    /// `audit` も同様に手組みオラクル。
    #[test]
    fn audit_matches_a_hand_assembled_oracle() {
        let line = LogbookLine {
            ts: "2026-08-12T18:00:01.000+03:00".to_string(),
            seq: 8,
            record: sample_audit(),
        };
        let json = serde_json::to_string(&line).unwrap();
        assert_eq!(
            json,
            r#"{"ts":"2026-08-12T18:00:01.000+03:00","seq":8,"type":"audit","actor":"controller","action":"run/start","params":{"run_number":58},"operator":"aogaki","ok":false,"error":"listen-before-start violated"}"#
        );
    }

    /// `psu` も同様に手組みオラクル。
    #[test]
    fn psu_matches_a_hand_assembled_oracle() {
        let line = LogbookLine {
            ts: "2026-08-12T18:00:02.000+03:00".to_string(),
            seq: 9,
            record: sample_psu(),
        };
        let json = serde_json::to_string(&line).unwrap();
        assert_eq!(
            json,
            r#"{"ts":"2026-08-12T18:00:02.000+03:00","seq":9,"type":"psu","actor":"psu","device":"CAEN0","channel":3,"event":"TRIP","values":{"vmon":350.2,"imon":0.008,"vset":350.0}}"#
        );
    }

    #[test]
    fn all_five_record_types_roundtrip_through_json() {
        for record in [
            sample_run_start(),
            sample_run_stop(),
            sample_audit(),
            sample_comment(),
            sample_psu(),
        ] {
            let line = LogbookLine {
                ts: "2026-08-12T00:00:00.000+00:00".to_string(),
                seq: 1,
                record,
            };
            let json = serde_json::to_string(&line).unwrap();
            let back: LogbookLine = serde_json::from_str(&json).unwrap();
            assert_eq!(back, line);
        }
    }

    // -----------------------------------------------------------------
    // スキーマ漂流ガード(SPEC §2.5 の方式)
    // -----------------------------------------------------------------

    /// トップレベルの JSON オブジェクトのキーを出現順に取り出す(このテスト専用の
    /// 最小スキャナ。ネストしたオブジェクト・配列の中身は数えない。値の文字列と
    /// キーの文字列は「直後が `:`か」で区別する)。
    fn top_level_keys(json: &str) -> Vec<String> {
        let bytes = json.as_bytes();
        let mut keys = Vec::new();
        let mut i = 0usize;
        let mut depth = 0i32;
        while i < bytes.len() {
            match bytes[i] {
                b'{' | b'[' => {
                    depth += 1;
                    i += 1;
                }
                b'}' | b']' => {
                    depth -= 1;
                    i += 1;
                }
                b'"' => {
                    let (text, next) = read_json_string(bytes, i);
                    let mut k = next;
                    while k < bytes.len() && (bytes[k] as char).is_whitespace() {
                        k += 1;
                    }
                    if depth == 1 && k < bytes.len() && bytes[k] == b':' {
                        keys.push(text);
                    }
                    i = next;
                }
                _ => i += 1,
            }
        }
        keys
    }

    /// `bytes[start]` が開き `"` を指す前提で文字列を読み、(中身, 閉じ `"` の次の index) を返す。
    fn read_json_string(bytes: &[u8], start: usize) -> (String, usize) {
        assert_eq!(bytes[start], b'"');
        let mut i = start + 1;
        let mut escape = false;
        while i < bytes.len() {
            if escape {
                escape = false;
            } else if bytes[i] == b'\\' {
                escape = true;
            } else if bytes[i] == b'"' {
                let text = std::str::from_utf8(&bytes[start + 1..i])
                    .unwrap()
                    .to_string();
                return (text, i + 1);
            }
            i += 1;
        }
        panic!("unterminated JSON string in {bytes:?}");
    }

    #[test]
    fn top_level_keys_ignores_nested_object_and_array_keys() {
        let json = r#"{"a":1,"b":{"nested_key":2},"c":[{"array_key":3}],"d":"literal: not a key"}"#;
        assert_eq!(top_level_keys(json), vec!["a", "b", "c", "d"]);
    }

    fn expected_keys(extra: &[&str]) -> Vec<String> {
        schema::COMMON
            .iter()
            .chain(extra)
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn run_start_field_order_matches_the_schema_table() {
        let line = LogbookLine {
            ts: "2026-08-12T00:00:00.000+00:00".to_string(),
            seq: 1,
            record: sample_run_start(),
        };
        let json = serde_json::to_string(&line).unwrap();
        assert_eq!(top_level_keys(&json), expected_keys(schema::RUN_START));
    }

    #[test]
    fn run_stop_field_order_matches_the_schema_table() {
        let line = LogbookLine {
            ts: "2026-08-12T00:00:00.000+00:00".to_string(),
            seq: 1,
            record: sample_run_stop(),
        };
        let json = serde_json::to_string(&line).unwrap();
        assert_eq!(top_level_keys(&json), expected_keys(schema::RUN_STOP));
    }

    #[test]
    fn audit_field_order_matches_the_schema_table() {
        let line = LogbookLine {
            ts: "2026-08-12T00:00:00.000+00:00".to_string(),
            seq: 1,
            record: sample_audit(),
        };
        let json = serde_json::to_string(&line).unwrap();
        assert_eq!(top_level_keys(&json), expected_keys(schema::AUDIT));
    }

    #[test]
    fn comment_field_order_matches_the_schema_table() {
        let line = LogbookLine {
            ts: "2026-08-12T00:00:00.000+00:00".to_string(),
            seq: 1,
            record: sample_comment(),
        };
        let json = serde_json::to_string(&line).unwrap();
        assert_eq!(top_level_keys(&json), expected_keys(schema::COMMENT));
    }

    #[test]
    fn psu_field_order_matches_the_schema_table() {
        let line = LogbookLine {
            ts: "2026-08-12T00:00:00.000+00:00".to_string(),
            seq: 1,
            record: sample_psu(),
        };
        let json = serde_json::to_string(&line).unwrap();
        assert_eq!(top_level_keys(&json), expected_keys(schema::PSU));
    }

    /// 表が実際のフィールド数より短い場合に確実に落ちること(ガード自身の健全性)。
    #[test]
    fn drift_guard_rejects_a_wrong_table() {
        let line = LogbookLine {
            ts: "2026-08-12T00:00:00.000+00:00".to_string(),
            seq: 1,
            record: sample_comment(),
        };
        let json = serde_json::to_string(&line).unwrap();
        let short = &schema::COMMENT[..1]; // "author" だけ、"text" が欠けた表
        assert_ne!(top_level_keys(&json), expected_keys(short));
    }

    // -----------------------------------------------------------------
    // ts 形式(RFC3339・ミリ秒・オフセット付き)
    // -----------------------------------------------------------------

    /// `regex` crate を増やさず(発注書「依存追加なし」)固定長の手書きチェックで判定する。
    fn looks_like_rfc3339_millis_with_offset(ts: &str) -> bool {
        let b = ts.as_bytes();
        // "YYYY-MM-DDTHH:MM:SS.mmm+HH:MM" = 29 バイト固定長。
        if b.len() != 29 {
            return false;
        }
        let digits = |range: std::ops::Range<usize>| b[range].iter().all(u8::is_ascii_digit);
        digits(0..4)
            && b[4] == b'-'
            && digits(5..7)
            && b[7] == b'-'
            && digits(8..10)
            && b[10] == b'T'
            && digits(11..13)
            && b[13] == b':'
            && digits(14..16)
            && b[16] == b':'
            && digits(17..19)
            && b[19] == b'.'
            && digits(20..23)
            && (b[23] == b'+' || b[23] == b'-')
            && digits(24..26)
            && b[26] == b':'
            && digits(27..29)
    }

    #[test]
    fn looks_like_rfc3339_millis_with_offset_accepts_the_spec_examples() {
        assert!(looks_like_rfc3339_millis_with_offset(
            "2026-08-12T15:04:05.123+03:00"
        ));
        assert!(looks_like_rfc3339_millis_with_offset(
            "2026-08-12T16:10:00.001-05:30"
        ));
    }

    #[test]
    fn looks_like_rfc3339_millis_with_offset_rejects_malformed_input() {
        // "Z" サフィックス(数値オフセットではない)/ ミリ秒欠落 / 空文字列。
        assert!(!looks_like_rfc3339_millis_with_offset(
            "2026-08-12T15:04:05.123Z"
        ));
        assert!(!looks_like_rfc3339_millis_with_offset(
            "2026-08-12T15:04:05+03:00"
        ));
        assert!(!looks_like_rfc3339_millis_with_offset(""));
    }

    #[test]
    fn append_stamps_a_valid_rfc3339_millis_ts_with_offset() {
        let path = temp_path("ts-format");
        let mut writer = LogbookWriter::open(&path).unwrap();
        writer.append(sample_comment()).unwrap();
        let result = read_since(&path, 0).unwrap();
        assert_eq!(result.records.len(), 1);
        assert!(looks_like_rfc3339_millis_with_offset(&result.records[0].ts));
        let _ = std::fs::remove_file(&path);
    }

    // -----------------------------------------------------------------
    // seq 単調増加 / open の回復
    // -----------------------------------------------------------------

    #[test]
    fn seq_starts_at_1_and_increments_monotonically() {
        let path = temp_path("seq-monotonic");
        let mut writer = LogbookWriter::open(&path).unwrap();
        assert_eq!(writer.next_seq(), 1);
        assert_eq!(writer.append(sample_comment()).unwrap(), 1);
        assert_eq!(writer.append(sample_comment()).unwrap(), 2);
        assert_eq!(writer.append(sample_comment()).unwrap(), 3);
        assert_eq!(writer.next_seq(), 4);
        let _ = std::fs::remove_file(&path);
    }

    /// 再オープン(プロセス再起動を模す)は前回の続きの seq を回復する。
    #[test]
    fn reopening_the_file_recovers_the_next_seq() {
        let path = temp_path("seq-recovery");
        {
            let mut writer = LogbookWriter::open(&path).unwrap();
            writer.append(sample_comment()).unwrap();
            writer.append(sample_comment()).unwrap();
        }
        let writer2 = LogbookWriter::open(&path).unwrap();
        assert_eq!(writer2.next_seq(), 3);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn opening_a_nonexistent_file_starts_at_seq_1() {
        let path = temp_path("seq-fresh");
        let _ = std::fs::remove_file(&path); // 確実に存在しない状態から
        let writer = LogbookWriter::open(&path).unwrap();
        assert_eq!(writer.next_seq(), 1);
        let _ = std::fs::remove_file(&path);
    }

    // -----------------------------------------------------------------
    // read_since のフィルタ
    // -----------------------------------------------------------------

    #[test]
    fn read_since_returns_only_records_after_the_given_seq() {
        let path = temp_path("read-since-filter");
        let mut writer = LogbookWriter::open(&path).unwrap();
        for _ in 0..5 {
            writer.append(sample_comment()).unwrap();
        }
        let result = read_since(&path, 2).unwrap();
        assert_eq!(
            result.records.iter().map(|r| r.seq).collect::<Vec<_>>(),
            vec![3, 4, 5]
        );
        assert!(!result.tail_corrupt);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_since_0_returns_everything() {
        let path = temp_path("read-since-zero");
        let mut writer = LogbookWriter::open(&path).unwrap();
        writer.append(sample_comment()).unwrap();
        writer.append(sample_comment()).unwrap();
        let result = read_since(&path, 0).unwrap();
        assert_eq!(result.records.len(), 2);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_since_a_missing_file_returns_empty_not_corrupt() {
        let path = temp_path("read-since-missing");
        let _ = std::fs::remove_file(&path);
        let result = read_since(&path, 0).unwrap();
        assert!(result.records.is_empty());
        assert!(!result.tail_corrupt);
    }

    // -----------------------------------------------------------------
    // 耐久(SPEC §12-11 の再現)
    // -----------------------------------------------------------------

    /// ①行の途中で切れたファイル(手で truncate)。
    /// リーダは最終行以外全部 parse + 警告フラグ、writer は正しい seq で再開。
    #[test]
    fn truncated_tail_line_is_tolerated_by_reader_and_writer_resumes_correctly() {
        let path = temp_path("truncated-tail");
        {
            let mut writer = LogbookWriter::open(&path).unwrap();
            writer.append(sample_comment()).unwrap(); // seq 1
            writer.append(sample_comment()).unwrap(); // seq 2
            writer.append(sample_run_start()).unwrap(); // seq 3
        }
        // seq 4 の途中(write(2) が torn write した想定)を手で追記 — 改行なし。
        {
            use std::io::Write as _;
            let mut f = OpenOptions::new().append(true).open(&path).unwrap();
            f.write_all(br#"{"ts":"2026-08-12T00:00:03.000+00:00","seq":4,"type":"com"#)
                .unwrap();
        }

        // リーダ: 最終行(壊れた seq=4 相当)以外の 3 行が読め、警告フラグが立つ。
        let result = read_since(&path, 0).unwrap();
        assert_eq!(
            result.records.iter().map(|r| r.seq).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert!(result.tail_corrupt);

        // writer: 壊れた seq=4 を無視し、seq=4 として再開する(seq 5 からではない)。
        let mut writer = LogbookWriter::open(&path).unwrap();
        assert_eq!(writer.next_seq(), 4);
        assert_eq!(writer.append(sample_comment()).unwrap(), 4);

        let _ = std::fs::remove_file(&path);
    }

    /// ②末尾から連続して 2 行破損(SPEC §9.1 が許容する「末尾 1 行だけ」を超えるケース、
    /// 025 R-P2-13)。復旧そのものは成功する(2 行遡って見つかる)が、内部のスキップ数は 2 —
    /// `recover_next_seq` はこの経路で `warn!` する(CLAUDE.md「silent failure を作らない」)。
    /// 公開 API(`LogbookWriter::open`)の挙動は不変であることも合わせて確認する。
    #[test]
    fn two_trailing_corrupt_lines_still_recover_but_report_a_skip_of_two() {
        let path = temp_path("two-trailing-corrupt");
        {
            let mut writer = LogbookWriter::open(&path).unwrap();
            writer.append(sample_comment()).unwrap(); // seq 1
            writer.append(sample_comment()).unwrap(); // seq 2
        }
        // seq 3 相当・seq 4 相当のつもりだった 2 行を、どちらも壊れた形で手で追記する
        // (二重クラッシュ相当の異常系 — 通常の torn write は末尾 1 行だけのはず)。
        {
            use std::io::Write as _;
            let mut f = OpenOptions::new().append(true).open(&path).unwrap();
            f.write_all(b"not json at all, line one\n").unwrap();
            f.write_all(b"not json at all, line two\n").unwrap();
        }

        let (next_seq, skipped) = recover_next_seq_with_skip(&path).unwrap();
        assert_eq!(next_seq, 3, "seq 2 の行まで遡って見つかった値から復旧する");
        assert_eq!(skipped, 2, "末尾から 2 行スキップした(warn 経路の機械照合)");

        // 公開挙動は不変: LogbookWriter::open は同じ next_seq で再開する。
        let writer = LogbookWriter::open(&path).unwrap();
        assert_eq!(writer.next_seq(), 3);

        let _ = std::fs::remove_file(&path);
    }

    /// 途中(最終行より前)の破損は silent に無視せず Err(SPEC §9.1「それ以外の行の破損は Err」)。
    #[test]
    fn corruption_before_the_last_line_is_an_error_not_silently_skipped() {
        let path = temp_path("interior-corruption");
        let mut content = String::new();
        content.push_str(r#"{"ts":"2026-08-12T00:00:00.000+00:00","seq":1,"type":"comment","actor":"ui","author":"a","text":"ok"}"#);
        content.push('\n');
        content.push_str("this line is not json at all\n"); // 中間行の破損
        content.push_str(r#"{"ts":"2026-08-12T00:00:02.000+00:00","seq":3,"type":"comment","actor":"ui","author":"a","text":"ok"}"#);
        content.push('\n');
        std::fs::write(&path, content).unwrap();

        let err = read_since(&path, 0).unwrap_err();
        assert!(matches!(err, LogbookError::CorruptLine { line_no: 2, .. }));

        let _ = std::fs::remove_file(&path);
    }

    /// ③同一 writer で 1000 行追記 → 全行 parse 可・seq 連続。
    #[test]
    fn writer_appends_1000_lines_all_parseable_with_contiguous_seq() {
        let path = temp_path("durability-1000");
        {
            let mut writer = LogbookWriter::open(&path).unwrap();
            for i in 0..1000u64 {
                let seq = writer
                    .append(LogbookRecord::Comment {
                        actor: "ui".to_string(),
                        author: "aogaki".to_string(),
                        text: format!("line {i}"),
                    })
                    .unwrap();
                assert_eq!(seq, i + 1);
            }
        }

        let result = read_since(&path, 0).unwrap();
        assert!(!result.tail_corrupt);
        assert_eq!(result.records.len(), 1000);
        let seqs: Vec<u64> = result.records.iter().map(|r| r.seq).collect();
        let expected: Vec<u64> = (1..=1000).collect();
        assert_eq!(seqs, expected);

        let _ = std::fs::remove_file(&path);
    }
}
