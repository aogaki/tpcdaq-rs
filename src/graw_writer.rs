//! graw-writer — 生 GRAW フレームを AsAd 毎ファイルへバイト一致 append するコンポーネント。
//! SPEC §1.1(責務)/ §1.3(状態機械)/ §1.4(過負荷カスケード)/ §2.2・§2.3(Batch/RawFrames/EOS)/
//! §6.5(出力配置)/ §7(全部、v1.2 — AsAd 毎ファイル + 実機 DataRouter 命名 + 非 AsAd 制御フレーム
//! の ctrl/ 保全)。
//!
//! # 構成
//!
//! [`RunWriter`] が「フレームを受け取って正しいファイルへ書く」純粋なコア(IO はするが ZMQ は
//! 知らない — decode/framer と同じ Clean Architecture の切り方)。ZMQ 配線([`run_graw_writer`])
//! は [`RunWriter`] を専用 OS スレッド上で駆動するだけの薄い層にする。
//!
//! # 状態機械(SPEC §1.3/§7)
//!
//! `Configure`(run 番号確定)→ `Arm`(**PULL bind**)→ `Start{run}`([`RunWriter`] を新設し
//! 受信スレッドを起動、消費開始)→ `Stop` / `Reset`。ファイルの実際の close は **全期待ソースの
//! EndOfStream 受領**で駆動される(データ駆動、root-sink の RunState と同じ発想)。`Stop` は
//! 受信スレッドへ停止合図を送るだけで、スレッド側は合図の有無によらず終了直前に必ず
//! [`RunWriter::finalize`] を呼ぶ(開きっぱなしのハンドルを残さない)。
//!
//! # 非 AsAd 制御フレーム(SPEC §7 v1.2)
//!
//! [`crate::decode::peek_asad`] は **frameType 1/2 かつヘッダ 28 B 以上のときだけ** `Some(asad)`
//! を返す。それ以外(短小フレーム、frameType ∉ {1,2} の制御フレーム — 実 2025 run 先頭の
//! frameType 7・12 B が実例)は `None` になり、graw-writer は **Error にせず**
//! `run{run:04}/ctrl/CoBo{K}_{TS}_{idx:04}.graw` へバイトそのまま保全する(意図的ドロップ禁止の
//! 絶対ルールを優先 — 実機 FrameStorage は同フレームを警告して捨てるが、tpcdaq-rs は捨てない)。
//! Error になるのは write 失敗・seq ギャップ・EOS 前の run 変更のみ。

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver as StdReceiver, Sender as StdSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Local;
use serde_bytes::ByteBuf;
use tokio::sync::{broadcast, oneshot};
use tracing::{error, info, warn};

use crate::command::{run_command_task, Command, CommandResponse, ComponentState, RunConfig};
use crate::config::{Config, GRAW_WRITER_COMMAND_LISTEN};
use crate::decode::peek_asad;
use crate::msg::{Message, RawFrames};
use crate::zmq_helper;

/// PULL recv のタイムアウト(ms)。停止合図の確認と定期 flush のための目覚まし。
const POLL_TIMEOUT_MS: i32 = 200;

// ---------------------------------------------------------------------
// 起動パラメタ
// ---------------------------------------------------------------------

/// graw-writer 1 プロセス分の起動パラメタ(= 設定の解決結果)。
///
/// テストはすべてのアドレスにポート 0 を入れて動的ポートを使う。
#[derive(Debug, Clone, PartialEq)]
pub struct GrawWriterParams {
    /// PULL bind アドレス(例 `tcp://*:47001`、SPEC §3.2)。
    pub pull_bind: String,
    /// コマンド REP の bind アドレス(SPEC §3.2)。
    pub command_listen: String,
    /// 出力の根(`<output_root>/run{run:04}/...`、SPEC §6.5)。
    pub output_root: PathBuf,
    /// ファイルローテーションの閾値(バイト、SPEC §7)。
    pub max_file_bytes: u64,
    /// flush 周期(ミリ秒、SPEC §7)。
    pub flush_interval_ms: u64,
    /// 期待ソース集合(設定の `[[cobo]]` 全 id、SPEC §7 の EOS 判定)。
    pub expected_sources: Vec<u32>,
}

impl GrawWriterParams {
    /// 設定から起動パラメタを解決する。
    pub fn from_config(config: &Config) -> Self {
        Self {
            pull_bind: config.graw_writer.pull_bind.clone(),
            command_listen: GRAW_WRITER_COMMAND_LISTEN.to_string(),
            output_root: config.system.output_root.clone(),
            max_file_bytes: config.graw_writer.max_file_bytes,
            flush_interval_ms: config.graw_writer.flush_interval_ms,
            expected_sources: config.cobo.iter().map(|c| c.id).collect(),
        }
    }
}

// ---------------------------------------------------------------------
// エラー
// ---------------------------------------------------------------------

/// ファイル IO で起きうる失敗(silent に握り潰さないためのエラー型、CLAUDE.md)。
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

// ---------------------------------------------------------------------
// ファイル鍵(per-AsAd / ctrl 共通、SPEC §7 v1.2)
// ---------------------------------------------------------------------

/// 1 出力ファイルの識別子。per-AsAd(`CoBo{K}_AsAd{A}_...`)と ctrl(`ctrl/CoBo{K}_...`、
/// v1.2)は命名以外(TS・idx・ローテーション・遅延作成・EOS finalize)の規則が完全に同一なので、
/// 同じ `HashMap` 上でこの鍵だけを切り替えて扱う(実装の重複を避ける)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum FileKey {
    /// per-AsAd ファイル(cobo, asad)。
    Asad(u32, u8),
    /// ctrl ファイル(cobo のみ — asadIdx を持たない制御フレーム用、SPEC §7 v1.2)。
    Ctrl(u32),
}

impl FileKey {
    fn cobo(self) -> u32 {
        match self {
            FileKey::Asad(cobo, _) | FileKey::Ctrl(cobo) => cobo,
        }
    }

    fn asad(self) -> Option<u8> {
        match self {
            FileKey::Asad(_, asad) => Some(asad),
            FileKey::Ctrl(_) => None,
        }
    }
}

// ---------------------------------------------------------------------
// ファイル実績(metrics の材料、SPEC §7)
// ---------------------------------------------------------------------

/// 1 ファイル分の実績。`asad` は per-AsAd ファイルなら `Some`、ctrl ファイルなら `None`
/// (v1.2)。
#[derive(Debug, Clone, PartialEq)]
pub struct FileReport {
    pub cobo: u32,
    pub asad: Option<u8>,
    pub idx: u32,
    pub path: PathBuf,
    pub bytes: u64,
    pub frames: u64,
    /// flush + fsync + close 済みか。`false` の間は `bytes` が BufWriter 内バッファ分だけ
    /// 実ディスクより先行していることがある(flush はローテーションと close 時のみ、
    /// ホットパスで fsync しない — SPEC §7)。ディスク上の内容を検証するテストは
    /// `closed == true` を待ってから読むこと。
    pub closed: bool,
}

// ---------------------------------------------------------------------
// RunWriter — フレームを正しいファイルへ書く純粋なコア(IO あり、ZMQ 非依存)
// ---------------------------------------------------------------------

/// 開きっぱなしの 1 出力ファイル(per-AsAd / ctrl 共通)。
struct OutputFile {
    key: FileKey,
    ts: String,
    idx: u32,
    cur_bytes: u64,
    frames: u64,
    file: BufWriter<File>,
    path: PathBuf,
}

impl OutputFile {
    fn report(&self, closed: bool) -> FileReport {
        FileReport {
            cobo: self.key.cobo(),
            asad: self.key.asad(),
            idx: self.idx,
            path: self.path.clone(),
            bytes: self.cur_bytes,
            frames: self.frames,
            closed,
        }
    }
}

/// 1 run 分の書き込み状態。receiver からの `RawFrames` バッチを受け取り、フレームを
/// (cobo, asad) 毎のファイル(または非 AsAd フレームなら ctrl/ ファイル)へバイトそのまま
/// append する(SPEC §7)。
///
/// ZMQ を知らない純粋なコア(実ファイル IO はする)。ホットパスの規約(per-frame の
/// open/close をしない)を守るため、ファイルハンドルは [`RunWriter`] が run の間ずっと保持する。
pub struct RunWriter {
    output_root: PathBuf,
    run_number: u32,
    max_file_bytes: u64,
    run_dir_created: bool,
    ctrl_dir_created: bool,
    files: HashMap<FileKey, OutputFile>,
    closed_files: Vec<FileReport>,
    asad_bytes: HashMap<(u32, u8), u64>,
    asad_frames: HashMap<(u32, u8), u64>,
    batches: HashMap<u32, u64>,
    next_seq: HashMap<u32, u64>,
    seq_gaps: u64,
    run_mismatches: u64,
    /// 非 AsAd フレーム(peek_asad = None)を ctrl/ へ保全した回数(SPEC §7 v1.2)。
    /// **Error 状態には遷移しない**(malformed とは違い、run 先頭に来る正常な制御フレーム)。
    ctrl_frames: u64,
    write_errors: u64,
    errored: bool,
}

impl RunWriter {
    pub fn new(output_root: PathBuf, run_number: u32, max_file_bytes: u64) -> Self {
        Self {
            output_root,
            run_number,
            max_file_bytes,
            run_dir_created: false,
            ctrl_dir_created: false,
            files: HashMap::new(),
            closed_files: Vec::new(),
            asad_bytes: HashMap::new(),
            asad_frames: HashMap::new(),
            batches: HashMap::new(),
            next_seq: HashMap::new(),
            seq_gaps: 0,
            run_mismatches: 0,
            ctrl_frames: 0,
            write_errors: 0,
            errored: false,
        }
    }

    /// 過負荷/プロトコル違反を一度でも踏んだか(SPEC §7「Error 状態 + カウント」)。
    /// write 失敗・seq ギャップ・EOS 前の run 変更のみが対象(v1.2: 非 AsAd 制御フレームは対象外)。
    pub fn errored(&self) -> bool {
        self.errored
    }

    pub fn seq_gaps(&self) -> u64 {
        self.seq_gaps
    }

    pub fn run_mismatches(&self) -> u64 {
        self.run_mismatches
    }

    /// ctrl/ へ保全した非 AsAd フレーム数(SPEC §7 v1.2)。
    pub fn ctrl_frames(&self) -> u64 {
        self.ctrl_frames
    }

    pub fn write_errors(&self) -> u64 {
        self.write_errors
    }

    /// 1 Batch(RawFrames)分を処理する: ソース毎 sequence_number 連続性 → run_number 一致 →
    /// 各フレームを frameType 1/2 の asadIdx で振り分け、それ以外(非 AsAd)は ctrl/ へ保全する
    /// (SPEC §7 v1.2)。
    ///
    /// 書き込み失敗のみ `Err` として呼び出し側へ伝える(SPEC §7「PULL 消費停止」の起点)。
    /// seq ギャップ・run 不一致は Error 状態への遷移材料としてカウントするだけで、処理は継続する
    /// (取れるデータは取りこぼさない)。非 AsAd フレームは Error にしない(v1.2)。
    pub fn handle_batch(
        &mut self,
        source_id: u32,
        run_number: u32,
        sequence_number: u64,
        frames: &[ByteBuf],
    ) -> Result<(), WriteError> {
        if run_number != self.run_number {
            self.run_mismatches += 1;
            self.errored = true;
            warn!(
                source_id,
                expected_run = self.run_number,
                got_run = run_number,
                "graw-writer: run_number changed before EndOfStream — protocol violation (SPEC §7)"
            );
            return Ok(());
        }

        let expected = self.next_seq.entry(source_id).or_insert(0);
        if sequence_number != *expected {
            self.seq_gaps += 1;
            self.errored = true;
            warn!(
                source_id,
                expected = *expected,
                got = sequence_number,
                "graw-writer: sequence_number gap on a lossless link (SPEC §2.2/§7)"
            );
        }
        *expected = sequence_number + 1;
        *self.batches.entry(source_id).or_insert(0) += 1;

        for frame in frames {
            let bytes: &[u8] = frame;
            match peek_asad(bytes) {
                Some(asad) => self.write_to(FileKey::Asad(source_id, asad), bytes)?,
                None => {
                    self.ctrl_frames += 1;
                    info!(
                        source_id,
                        frame_len = bytes.len(),
                        "graw-writer: non-AsAd control frame preserved under ctrl/ (SPEC §7 v1.2, \
                         not an Error)"
                    );
                    self.write_to(FileKey::Ctrl(source_id), bytes)?;
                }
            }
        }
        Ok(())
    }

    /// 開いている全ファイルを flush する(fsync なし — ホットパスで fsync しない、SPEC §7)。
    pub fn flush_all(&mut self) -> Result<(), WriteError> {
        let mut first_err: Option<(PathBuf, std::io::Error)> = None;
        for entry in self.files.values_mut() {
            if let Err(source) = entry.file.flush() {
                if first_err.is_none() {
                    first_err = Some((entry.path.clone(), source));
                }
            }
        }
        match first_err {
            Some((path, source)) => {
                self.write_errors += 1;
                self.errored = true;
                error!(path = %path.display(), error = %source, "graw-writer: periodic flush failed");
                Err(WriteError::Io { path, source })
            }
            None => Ok(()),
        }
    }

    /// 開いている全ファイルを flush + fsync + close する(SPEC §7「全 EOS 受領 → close」)。
    /// per-AsAd / ctrl のどちらも同一規則で閉じる(v1.2)。個々のファイルの失敗は最初の 1 件を
    /// 返しつつ、残りのファイルも close を試みる(silent にしないが、1 本の失敗で他を巻き込まない)。
    pub fn finalize(&mut self) -> Result<(), WriteError> {
        let keys: Vec<FileKey> = self.files.keys().copied().collect();
        let mut first_err = None;
        for key in keys {
            if let Err(e) = self.close_file(key) {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// ファイル実績(閉じたもの + 現在開いているもの、per-AsAd/ctrl 共通)。
    /// `(cobo, asad, idx)` 順(`asad` は `None`(ctrl)が `Some`(per-AsAd)より先)。
    pub fn file_report(&self) -> Vec<FileReport> {
        let mut reports = self.closed_files.clone();
        reports.extend(self.files.values().map(|f| f.report(false)));
        reports.sort_by_key(|r| (r.cobo, r.asad, r.idx));
        reports
    }

    /// `CommandResponse.metrics` に載せる JSON(SPEC §7 のカウンタ一式、v1.2)。
    pub fn metrics_json(&self) -> serde_json::Value {
        let files: Vec<_> = self
            .file_report()
            .into_iter()
            .map(|f| {
                serde_json::json!({
                    "cobo": f.cobo,
                    "asad": f.asad,
                    "idx": f.idx,
                    "path": f.path.display().to_string(),
                    "bytes": f.bytes,
                    "frames": f.frames,
                    "closed": f.closed,
                })
            })
            .collect();

        let mut asad_keys: Vec<(u32, u8)> = self.asad_bytes.keys().copied().collect();
        asad_keys.sort();
        let asad: Vec<_> = asad_keys
            .into_iter()
            .map(|k| {
                serde_json::json!({
                    "cobo": k.0,
                    "asad": k.1,
                    "bytes": self.asad_bytes.get(&k).copied().unwrap_or(0),
                    "frames": self.asad_frames.get(&k).copied().unwrap_or(0),
                })
            })
            .collect();

        let mut batches = serde_json::Map::new();
        for (cobo, n) in &self.batches {
            batches.insert(cobo.to_string(), serde_json::json!(n));
        }

        serde_json::json!({
            "seq_gaps": self.seq_gaps,
            "run_mismatches": self.run_mismatches,
            "ctrl_frames": self.ctrl_frames,
            "write_errors": self.write_errors,
            "batches": serde_json::Value::Object(batches),
            "asad": asad,
            "files": files,
            // まだ開いている(flush 待ちの可能性がある)/ 既に flush+fsync+close 済みの本数。
            // 「全 EOS 受領 → close」(SPEC §7)を外部から観測する material — ディスク上の
            // 内容を検証する側は files_open == 0 を待ってから読むこと。
            "files_open": self.files.len(),
            "files_closed": self.closed_files.len(),
        })
    }

    // -------------------------------------------------------------
    // 内部実装
    // -------------------------------------------------------------

    fn run_dir(&self) -> PathBuf {
        self.output_root.join(format!("run{:04}", self.run_number))
    }

    /// ctrl ファイルの置き場所(SPEC §7/§6.5 v1.2)。run ディレクトリ配下のサブディレクトリ
    /// なので、オフラインの `CoBo*` glob には見えない(per-AsAd ファイルは実機と完全一致のまま)。
    fn ctrl_dir(&self) -> PathBuf {
        self.run_dir().join("ctrl")
    }

    /// run ディレクトリは当該 run の最初のフレーム到着時に遅延作成する(SPEC §7)。
    fn ensure_run_dir(&mut self) -> Result<(), WriteError> {
        if self.run_dir_created {
            return Ok(());
        }
        let dir = self.run_dir();
        if let Err(source) = std::fs::create_dir_all(&dir) {
            self.write_errors += 1;
            self.errored = true;
            return Err(WriteError::Io { path: dir, source });
        }
        self.run_dir_created = true;
        Ok(())
    }

    /// ctrl/ サブディレクトリは最初の非 AsAd フレーム到着時に遅延作成する(SPEC §7 v1.2)。
    fn ensure_ctrl_dir(&mut self) -> Result<(), WriteError> {
        if self.ctrl_dir_created {
            return Ok(());
        }
        let dir = self.ctrl_dir();
        if let Err(source) = std::fs::create_dir_all(&dir) {
            self.write_errors += 1;
            self.errored = true;
            return Err(WriteError::Io { path: dir, source });
        }
        self.ctrl_dir_created = true;
        Ok(())
    }

    /// `key` が指す出力ファイルのパスを組む(命名は per-AsAd / ctrl で異なる、SPEC §6.5/§7)。
    fn file_path(&self, key: FileKey, ts: &str, idx: u32) -> PathBuf {
        match key {
            FileKey::Asad(cobo, asad) => self
                .run_dir()
                .join(format!("CoBo{cobo}_AsAd{asad}_{ts}_{idx:04}.graw")),
            FileKey::Ctrl(cobo) => self
                .ctrl_dir()
                .join(format!("CoBo{cobo}_{ts}_{idx:04}.graw")),
        }
    }

    /// `key` の出力ファイルへ 1 フレームを書く(per-AsAd / ctrl 共通のホットパス)。
    fn write_to(&mut self, key: FileKey, frame: &[u8]) -> Result<(), WriteError> {
        self.ensure_run_dir()?;

        let must_rotate = self
            .files
            .get(&key)
            .map(|f| f.cur_bytes > 0 && f.cur_bytes + frame.len() as u64 > self.max_file_bytes)
            .unwrap_or(false);
        if must_rotate {
            self.rotate(key)?;
        }
        if !self.files.contains_key(&key) {
            self.create_file(key)?;
        }

        let write_result = {
            let entry = self
                .files
                .get_mut(&key)
                .expect("create_file/rotate just ensured the entry exists");
            entry.file.write_all(frame)
        };

        let entry = self
            .files
            .get_mut(&key)
            .expect("entry still present after write_all");
        match write_result {
            Ok(()) => {
                entry.cur_bytes += frame.len() as u64;
                entry.frames += 1;
                if let FileKey::Asad(cobo, asad) = key {
                    *self.asad_bytes.entry((cobo, asad)).or_insert(0) += frame.len() as u64;
                    *self.asad_frames.entry((cobo, asad)).or_insert(0) += 1;
                }
                Ok(())
            }
            Err(source) => {
                let path = entry.path.clone();
                self.write_errors += 1;
                self.errored = true;
                error!(
                    cobo = key.cobo(), asad = ?key.asad(), path = %path.display(), error = %source,
                    "graw-writer: write_all failed — stopping PULL consumption (SPEC §7 backpressure cascade)"
                );
                Err(WriteError::Io { path, source })
            }
        }
    }

    /// **ローテーション**: TS は据え置き、idx++ のみ(FrameStorage `createNewFile(newTimeStamp=false)`
    /// と同一挙動、SPEC §7)。per-AsAd / ctrl 共通。呼び出し元(`write_to`)が `cur_bytes > 0`
    /// の場合だけ呼ぶので、空ファイルへの巨大単発フレームはローテーションされない(同じガード)。
    fn rotate(&mut self, key: FileKey) -> Result<(), WriteError> {
        let (ts, path_for_err, flush_result) = {
            let entry = self.files.get_mut(&key).expect("rotate: file must exist");
            let path_for_err = entry.path.clone();
            let flush_result = entry
                .file
                .flush()
                .and_then(|()| entry.file.get_ref().sync_all());
            (entry.ts.clone(), path_for_err, flush_result)
        };
        if let Err(source) = flush_result {
            self.write_errors += 1;
            self.errored = true;
            error!(
                cobo = key.cobo(), asad = ?key.asad(), path = %path_for_err.display(), error = %source,
                "graw-writer: flush/fsync failed during rotation"
            );
            return Err(WriteError::Io {
                path: path_for_err,
                source,
            });
        }

        let finished = self.files.remove(&key).expect("just flushed above");
        let next_idx = finished.idx + 1;
        self.closed_files.push(finished.report(true));
        self.open_new_file(key, ts, next_idx)
    }

    fn create_file(&mut self, key: FileKey) -> Result<(), WriteError> {
        let ts = local_timestamp();
        self.open_new_file(key, ts, 0)
    }

    fn open_new_file(&mut self, key: FileKey, ts: String, idx: u32) -> Result<(), WriteError> {
        if matches!(key, FileKey::Ctrl(_)) {
            self.ensure_ctrl_dir()?;
        }
        let path = self.file_path(key, &ts, idx);
        let file = match File::create(&path) {
            Ok(file) => file,
            Err(source) => {
                self.write_errors += 1;
                self.errored = true;
                return Err(WriteError::Io { path, source });
            }
        };
        self.files.insert(
            key,
            OutputFile {
                key,
                ts,
                idx,
                cur_bytes: 0,
                frames: 0,
                file: BufWriter::new(file),
                path,
            },
        );
        Ok(())
    }

    fn close_file(&mut self, key: FileKey) -> Result<(), WriteError> {
        let Some(mut entry) = self.files.remove(&key) else {
            return Ok(());
        };
        let flush_result = entry
            .file
            .flush()
            .and_then(|()| entry.file.get_ref().sync_all());
        self.closed_files.push(entry.report(true));
        if let Err(source) = flush_result {
            self.write_errors += 1;
            self.errored = true;
            let path = entry.path.clone();
            error!(
                cobo = key.cobo(), asad = ?key.asad(), path = %path.display(), error = %source,
                "graw-writer: flush/fsync failed while closing (SPEC §7 EOS finalize)"
            );
            return Err(WriteError::Io { path, source });
        }
        Ok(())
    }
}

/// 実機 DataRouter 形式のタイムスタンプ: **localtime** の ISO 8601 拡張 + ミリ秒 3 桁
/// (例 `2022-04-12T08:03:44.531`)。出自: GET `utl::buildTimeStamp()`(SPEC §7)。
fn local_timestamp() -> String {
    Local::now().format("%Y-%m-%dT%H:%M:%S%.3f").to_string()
}

// ---------------------------------------------------------------------
// ZMQ 配線(受信スレッド + コマンド REP)
// ---------------------------------------------------------------------

/// PULL recv → [`RunWriter`] への書き込みを 1 本の専用 OS スレッドで行う(SPEC §7「受信スレッド
/// (専用 OS スレッド、同期 zmq)→ 書き込み処理」)。
///
/// 終了条件は 2 つ: (1) 期待ソース全部から EndOfStream を受け取った(データ駆動の正常終了)、
/// (2) `stop_rx` に停止合図が来た(`Stop`/`Reset` コマンド)。どちらの経路でも抜ける直前に
/// 必ず [`RunWriter::finalize`] を呼び、開きっぱなしのハンドルを残さない。
///
/// 書き込み失敗が起きたら **即座に PULL の消費を止める**(SPEC §7: HWM が詰まり上流receiver へ
/// 背圧が伝わり、overflow として可視化される — SPEC §1.4 のカスケード)。
fn run_writer_thread(
    pull: zmq::Socket,
    writer: Arc<Mutex<RunWriter>>,
    expected_sources: HashSet<u32>,
    run_number: u32,
    flush_interval: Duration,
    stop_rx: StdReceiver<()>,
) {
    if let Err(e) = pull.set_rcvtimeo(POLL_TIMEOUT_MS) {
        error!(error = %e, "graw-writer: cannot set PULL recv timeout — stop/flush polling degraded");
    }

    let mut eos_received: HashSet<u32> = HashSet::new();
    let mut last_flush = Instant::now();

    'consume: loop {
        if stop_rx.try_recv().is_ok() {
            info!(run_number, "graw-writer: Stop received — finalizing");
            break 'consume;
        }

        match pull.recv_bytes(0) {
            Ok(raw) => match Message::<RawFrames>::from_msgpack(&raw) {
                Ok(Message::Data(batch)) => {
                    let write_result = {
                        let Ok(mut w) = writer.lock() else {
                            error!("graw-writer: RunWriter mutex poisoned — stopping");
                            break 'consume;
                        };
                        w.handle_batch(
                            batch.source_id,
                            batch.run_number,
                            batch.sequence_number,
                            &batch.payload,
                        )
                    };
                    if write_result.is_err() {
                        // カウント/ログは RunWriter 内で既に行っている。ここでは消費を止めるだけ。
                        break 'consume;
                    }
                }
                Ok(Message::EndOfStream {
                    source_id,
                    run_number: eos_run,
                }) => {
                    if eos_run == run_number {
                        eos_received.insert(source_id);
                    } else {
                        warn!(
                            source_id,
                            expected_run = run_number,
                            got_run = eos_run,
                            "graw-writer: EndOfStream run_number mismatch"
                        );
                    }
                    if expected_sources.is_subset(&eos_received) {
                        info!(
                            run_number,
                            "graw-writer: all expected sources reached EndOfStream"
                        );
                        break 'consume;
                    }
                }
                Ok(Message::Heartbeat { .. }) => {}
                Err(e) => {
                    warn!(error = %e, "graw-writer: cannot decode message — skipped");
                }
            },
            Err(zmq::Error::EAGAIN) => {}
            Err(zmq::Error::ETERM) => break 'consume,
            Err(e) => {
                warn!(error = %e, "graw-writer: PULL recv failed");
            }
        }

        if last_flush.elapsed() >= flush_interval {
            if let Ok(mut w) = writer.lock() {
                let _ = w.flush_all();
            }
            last_flush = Instant::now();
        }
    }

    match writer.lock() {
        Ok(mut w) => {
            if let Err(e) = w.finalize() {
                error!(run_number, error = %e, "graw-writer: finalize failed");
            }
        }
        Err(_) => error!(
            run_number,
            "graw-writer: RunWriter mutex poisoned at finalize"
        ),
    }
    info!(run_number, "graw-writer thread stopped");
}

// ---------------------------------------------------------------------
// コマンドハンドラ(状態機械、SPEC §1.3/§7)
// ---------------------------------------------------------------------

struct Handler {
    params: GrawWriterParams,
    context: zmq::Context,
    state: ComponentState,
    run_number: Option<u32>,
    pull_socket: Option<zmq::Socket>,
    bind_address: Option<String>,
    writer: Option<Arc<Mutex<RunWriter>>>,
    stop_tx: Option<StdSender<()>>,
}

impl Handler {
    fn new(params: GrawWriterParams) -> Self {
        Self {
            params,
            context: zmq::Context::new(),
            state: ComponentState::Idle,
            run_number: None,
            pull_socket: None,
            bind_address: None,
            writer: None,
            stop_tx: None,
        }
    }

    fn handle(&mut self, cmd: Command) -> CommandResponse {
        self.latch_error();

        let Some(target) = cmd.target_state() else {
            return self.respond(true, "status"); // GetStatus は遷移しない
        };
        if !self.state.can_transition_to(target) {
            let message = format!("illegal transition {} -> {target}", self.state);
            warn!(%cmd, "{message}");
            return self.respond(false, message);
        }

        let outcome = match &cmd {
            Command::Configure(cfg) => self.do_configure(cfg),
            Command::Arm => self.do_arm(),
            Command::Start { run_number } => self.do_start(*run_number),
            Command::Stop => self.do_stop(),
            Command::Reset => self.do_reset(),
            Command::GetStatus => Ok(()),
        };

        match outcome {
            Ok(()) => {
                self.state = target;
                info!(state = %self.state, %cmd, "command applied");
                self.respond(true, format!("{cmd} ok"))
            }
            Err(message) => {
                error!(%cmd, "{message}");
                self.respond(false, message)
            }
        }
    }

    /// [`RunWriter`] が Error を報告していたら状態へ反映する(SPEC §7、receiver の
    /// overflow latch と同じ流儀)。
    fn latch_error(&mut self) {
        if self.state == ComponentState::Error
            || !self.state.can_transition_to(ComponentState::Error)
        {
            return;
        }
        let is_errored = self
            .writer
            .as_ref()
            .and_then(|w| w.lock().ok().map(|g| g.errored()))
            .unwrap_or(false);
        if !is_errored {
            return;
        }
        warn!("graw-writer: entering Error (protocol/write violation, SPEC §7)");
        self.state = ComponentState::Error;
    }

    fn do_configure(&mut self, cfg: &RunConfig) -> Result<(), String> {
        self.run_number = Some(cfg.run_number);
        Ok(())
    }

    /// listen-before-start と同じ理屈: Arm で PULL を bind しておく(SPEC §7)。
    fn do_arm(&mut self) -> Result<(), String> {
        let socket = self
            .context
            .socket(zmq::PULL)
            .map_err(|e| format!("cannot create PULL socket: {e}"))?;
        zmq_helper::apply_pull_hwm(&socket).map_err(|e| format!("cannot set PULL HWM: {e}"))?;
        socket
            .bind(&self.params.pull_bind)
            .map_err(|e| format!("bind {} failed: {e}", self.params.pull_bind))?;
        let endpoint = match socket.get_last_endpoint() {
            Ok(Ok(endpoint)) => endpoint,
            Ok(Err(raw)) => return Err(format!("last_endpoint is not utf-8: {raw:?}")),
            Err(e) => return Err(format!("cannot read last_endpoint: {e}")),
        };
        info!(%endpoint, "graw-writer: armed — PULL bound (SPEC §7)");
        self.bind_address = Some(endpoint);
        self.pull_socket = Some(socket);
        Ok(())
    }

    fn do_start(&mut self, run_number: u32) -> Result<(), String> {
        let Some(socket) = self.pull_socket.take() else {
            return Err("no PULL socket: Arm again before Start".to_string());
        };
        self.run_number = Some(run_number);

        let writer = Arc::new(Mutex::new(RunWriter::new(
            self.params.output_root.clone(),
            run_number,
            self.params.max_file_bytes,
        )));
        self.writer = Some(Arc::clone(&writer));

        let expected: HashSet<u32> = self.params.expected_sources.iter().copied().collect();
        let (stop_tx, stop_rx) = std::sync::mpsc::channel();
        self.stop_tx = Some(stop_tx);
        let flush_interval = Duration::from_millis(self.params.flush_interval_ms);

        std::thread::Builder::new()
            .name("graw-writer".to_string())
            .spawn(move || {
                run_writer_thread(
                    socket,
                    writer,
                    expected,
                    run_number,
                    flush_interval,
                    stop_rx,
                )
            })
            .map_err(|e| format!("cannot spawn writer thread: {e}"))?;
        Ok(())
    }

    fn do_stop(&mut self) -> Result<(), String> {
        self.stop_writer();
        self.bind_address = None;
        Ok(())
    }

    fn do_reset(&mut self) -> Result<(), String> {
        self.stop_writer();
        self.pull_socket = None;
        self.bind_address = None;
        self.run_number = None;
        self.writer = None;
        Ok(())
    }

    fn stop_writer(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(()); // 受信スレッドが既に居なくても構わない
        }
    }

    fn respond(&self, success: bool, message: impl Into<String>) -> CommandResponse {
        let mut response = if success {
            CommandResponse::success(self.state, message)
        } else {
            CommandResponse::error(self.state, message)
        };
        if let Some(run_number) = self.run_number {
            response = response.with_run_number(run_number);
        }
        let mut metrics = self
            .writer
            .as_ref()
            .and_then(|w| w.lock().ok())
            .map(|g| g.metrics_json())
            .unwrap_or_else(|| serde_json::json!({}));
        metrics["bind_address"] = match &self.bind_address {
            Some(addr) => serde_json::json!(addr),
            None => serde_json::Value::Null,
        };
        response.with_metrics(metrics)
    }
}

impl Drop for Handler {
    fn drop(&mut self) {
        self.stop_writer();
    }
}

// ---------------------------------------------------------------------
// エントリポイント
// ---------------------------------------------------------------------

/// graw-writer を起動し、`shutdown` が来るまでコマンドを処理し続ける。
///
/// `bound_command_endpoint` はコマンド REP の実 bind アドレスを 1 度だけ通知する
/// (`tcp://127.0.0.1:0` を使うテスト用。production は `None`)。
pub async fn run_graw_writer(
    params: GrawWriterParams,
    shutdown: broadcast::Receiver<()>,
    bound_command_endpoint: Option<oneshot::Sender<String>>,
) {
    info!(
        pull_bind = params.pull_bind,
        command_listen = params.command_listen,
        output_root = %params.output_root.display(),
        max_file_bytes = params.max_file_bytes,
        flush_interval_ms = params.flush_interval_ms,
        expected_sources = ?params.expected_sources,
        "graw-writer starting"
    );
    let command_listen = params.command_listen.clone();
    let mut handler = Handler::new(params);
    run_command_task(
        command_listen,
        "graw-writer",
        shutdown,
        bound_command_endpoint,
        move |cmd| handler.handle(cmd),
    )
    .await;
}

// ---------------------------------------------------------------------
// テスト(仕様書 — 先に書く)。RunWriter 単体(ZMQ なし、実ファイル IO はする)。
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// テスト専用の一意な出力先ディレクトリ(config.rs の `make_temp_geometry_file` と同じ流儀)。
    fn temp_output_root(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "tpcdaq-graw-writer-test-{tag}-{}-{}",
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn frame(bytes: &[u8]) -> ByteBuf {
        ByteBuf::from(bytes.to_vec())
    }

    /// asadIdx = offset 27 の最小 MFM フレームを組む(peek_asad が読める最小長 28 バイト)。
    /// frameType=1 を big-endian で offset 5..7 に明示する(v1.2 — `peek_asad` が
    /// frameType-aware になったため、asadIdx を読ませるには frameType 1/2 が必須)。
    /// 内容は非対称(offset 依存の値)にしておく — バイト一致テストで取り違えを検出しやすくする。
    fn make_frame(asad: u8, extra_len: usize) -> Vec<u8> {
        let mut b = vec![0u8; 28 + extra_len];
        // b[0] = 0x00 のまま(big-endian、bit7=0)。
        b[6] = 0x01; // frameType=1(offset 5..7 big-endian、offset5 は 0 のまま)
        b[27] = asad;
        for (i, byte) in b.iter_mut().enumerate().skip(28) {
            *byte = (i % 251) as u8;
        }
        b
    }

    /// frameType が 1/2 以外の(= 非 CoBo)制御フレームを組む。`len` バイトのうち offset 27 に
    /// 「asadIdx らしきバイト」を置いても、frameType で弾かれて `peek_asad` が `None` を返す
    /// (v1.2 の契約)ことを確かめるためのフィクスチャ。実 2025 run 先頭の frameType 7 相当。
    fn make_non_cobo_frame(frame_type: u16, len: usize, fill: u8) -> Vec<u8> {
        assert!(
            len >= 28,
            "テストの前提: 28 B 以上(短小フレームとの混同を避ける)"
        );
        let mut b = vec![fill; len];
        b[0] = 0x00; // big-endian
        b[5] = ((frame_type >> 8) & 0xFF) as u8;
        b[6] = (frame_type & 0xFF) as u8;
        b
    }

    /// SPEC §7 の実機 DataRouter 命名(per-AsAd)を検証する(regex 相当。regex クレートは
    /// 追加しないので手書きで文法を確認する — chrono でタイムスタンプ部だけパース可否を確かめる)。
    fn assert_matches_datarouter_naming(filename: &str) {
        let parts: Vec<&str> = filename.split('_').collect();
        assert_eq!(
            parts.len(),
            4,
            "CoBo{{K}}_AsAd{{A}}_{{TS}}_{{idx}}.graw の 4 区画のはず: {filename}"
        );

        let cobo = parts[0]
            .strip_prefix("CoBo")
            .unwrap_or_else(|| panic!("CoBo 接頭辞なし: {filename}"));
        assert!(
            !cobo.is_empty() && cobo.bytes().all(|b| b.is_ascii_digit()),
            "K は 10 進数のはず: {filename}"
        );

        let asad = parts[1]
            .strip_prefix("AsAd")
            .unwrap_or_else(|| panic!("AsAd 接頭辞なし: {filename}"));
        assert!(
            !asad.is_empty() && asad.bytes().all(|b| b.is_ascii_digit()),
            "A は 10 進数のはず: {filename}"
        );

        assert_matches_ts_and_idx(parts[2], parts[3]);
    }

    /// SPEC §6.5/§7 v1.2 の ctrl 命名(`CoBo{K}_{TS}_{idx:04}.graw`、AsAd 区画なし)を検証する。
    fn assert_matches_ctrl_naming(filename: &str) {
        let parts: Vec<&str> = filename.split('_').collect();
        assert_eq!(
            parts.len(),
            3,
            "CoBo{{K}}_{{TS}}_{{idx}}.graw の 3 区画のはず(AsAd なし): {filename}"
        );
        let cobo = parts[0]
            .strip_prefix("CoBo")
            .unwrap_or_else(|| panic!("CoBo 接頭辞なし: {filename}"));
        assert!(
            !cobo.is_empty() && cobo.bytes().all(|b| b.is_ascii_digit()),
            "K は 10 進数のはず: {filename}"
        );
        assert_matches_ts_and_idx(parts[1], parts[2]);
    }

    fn assert_matches_ts_and_idx(ts: &str, idx_and_ext: &str) {
        assert!(
            chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%S%.3f").is_ok(),
            "TS が ISO 8601 拡張 + ミリ秒 3 桁でない: {ts}"
        );
        let idx = idx_and_ext
            .strip_suffix(".graw")
            .unwrap_or_else(|| panic!(".graw 拡張子なし: {idx_and_ext}"));
        assert_eq!(idx.len(), 4, "idx は 4 桁ゼロ埋めのはず: {idx_and_ext}");
        assert!(
            idx.bytes().all(|b| b.is_ascii_digit()),
            "idx は 10 進数のはず: {idx_and_ext}"
        );
    }

    // -----------------------------------------------------------------
    // ファイル命名
    // -----------------------------------------------------------------

    #[test]
    fn file_naming_matches_the_real_datarouter_format() {
        let root = temp_output_root("naming");
        let mut w = RunWriter::new(root.clone(), 7, 10 * 1024 * 1024);
        w.handle_batch(0, 7, 0, &[frame(&make_frame(1, 4))])
            .unwrap();

        let files = w.file_report();
        assert_eq!(files.len(), 1);
        let name = files[0].path.file_name().and_then(|n| n.to_str()).unwrap();
        assert_matches_datarouter_naming(name);
        assert!(name.starts_with("CoBo0_AsAd1_"), "{name}");
        assert!(name.ends_with("_0000.graw"), "{name}");
        assert_eq!(
            files[0].path.parent().and_then(|p| p.file_name()),
            Some(std::ffi::OsStr::new("run0007")),
            "run ディレクトリは run{{run:04}}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    // -----------------------------------------------------------------
    // ローテーション: TS 不変・idx++
    // -----------------------------------------------------------------

    #[test]
    fn rotation_keeps_the_same_ts_and_increments_idx_only() {
        let root = temp_output_root("rotation-ts");
        let one_frame = make_frame(0, 100); // 128 B
        let max_file_bytes = one_frame.len() as u64; // ちょうど 1 フレーム分で毎回ローテーション
        let mut w = RunWriter::new(root.clone(), 3, max_file_bytes);

        for seq in 0..3u64 {
            w.handle_batch(0, 3, seq, &[frame(&one_frame)]).unwrap();
        }

        let files = w.file_report();
        assert_eq!(files.len(), 3, "1 フレームごとに新ファイル");
        let name0 = files[0].path.file_name().and_then(|n| n.to_str()).unwrap();
        let ts0 = name0.split('_').nth(2).unwrap().to_string();
        for (i, f) in files.iter().enumerate() {
            let name = f.path.file_name().and_then(|n| n.to_str()).unwrap();
            let ts = name.split('_').nth(2).unwrap();
            assert_eq!(ts, ts0, "TS はローテーションで据え置き: {name}");
            assert_eq!(f.idx, i as u32, "idx は連番: {name}");
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn ctrl_files_rotate_with_ts_unchanged_and_idx_incrementing_like_asad_files() {
        let root = temp_output_root("ctrl-rotation");
        let ctrl = vec![0xCCu8; 20]; // 短小(< 28 B)= 無条件で ctrl 行き
        let max_file_bytes = ctrl.len() as u64; // 毎回ローテーション
        let mut w = RunWriter::new(root.clone(), 6, max_file_bytes);
        for seq in 0..3u64 {
            w.handle_batch(0, 6, seq, &[frame(&ctrl)]).unwrap();
        }

        assert_eq!(w.ctrl_frames(), 3);
        assert!(!w.errored());
        let files = w.file_report();
        assert_eq!(files.len(), 3);
        let name0 = files[0].path.file_name().and_then(|n| n.to_str()).unwrap();
        let ts0 = name0.split('_').nth(1).unwrap().to_string();
        for (i, f) in files.iter().enumerate() {
            let name = f.path.file_name().and_then(|n| n.to_str()).unwrap();
            let ts = name.split('_').nth(1).unwrap();
            assert_eq!(ts, ts0, "ctrl も TS はローテーションで据え置き: {name}");
            assert_eq!(f.idx, i as u32);
            assert_eq!(f.asad, None);
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    // -----------------------------------------------------------------
    // asadIdx 振り分け(合成 2 AsAd 混在バッチ)
    // -----------------------------------------------------------------

    #[test]
    fn frames_are_routed_by_asad_index_within_a_mixed_batch() {
        let root = temp_output_root("asad-routing");
        let mut w = RunWriter::new(root.clone(), 1, 10 * 1024 * 1024);
        let f_a0 = make_frame(0, 10);
        let f_a1 = make_frame(1, 20);
        let f_a0b = make_frame(0, 10);
        w.handle_batch(2, 1, 0, &[frame(&f_a0), frame(&f_a1), frame(&f_a0b)])
            .unwrap();
        w.flush_all().unwrap();

        let files = w.file_report();
        assert_eq!(files.len(), 2, "asad 0 と asad 1 で別ファイル");
        let asad0 = files.iter().find(|f| f.asad == Some(0)).unwrap();
        let asad1 = files.iter().find(|f| f.asad == Some(1)).unwrap();
        assert_eq!(asad0.frames, 2);
        assert_eq!(asad1.frames, 1);
        assert_eq!(asad0.bytes, (f_a0.len() + f_a0b.len()) as u64);
        assert_eq!(asad1.bytes, f_a1.len() as u64);

        // バイト一致: asad0 のファイル内容 = 入力を asadIdx で分別した列と同一(f_a0 ++ f_a0b)
        let mut expected0 = f_a0.clone();
        expected0.extend_from_slice(&f_a0b);
        assert_eq!(std::fs::read(&asad0.path).unwrap(), expected0);
        assert_eq!(std::fs::read(&asad1.path).unwrap(), f_a1);

        let _ = std::fs::remove_dir_all(&root);
    }

    // -----------------------------------------------------------------
    // 非 AsAd フレーム(peek_asad = None)は ctrl/ へ保全、Error にしない(SPEC §7 v1.2)
    // -----------------------------------------------------------------

    #[test]
    fn a_non_asad_frame_is_preserved_under_ctrl_and_does_not_latch_error() {
        let root = temp_output_root("ctrl-short-frame");
        let mut w = RunWriter::new(root.clone(), 1, 10 * 1024 * 1024);
        // 実 2025 run 先頭の 12 B 制御フレーム相当(短小 — offset 27 が読めない)。
        let ctrl = vec![0xEEu8; 12];
        w.handle_batch(0, 1, 0, &[frame(&ctrl)]).unwrap();
        w.flush_all().unwrap();

        assert_eq!(w.ctrl_frames(), 1);
        assert!(
            !w.errored(),
            "非 AsAd フレームは Error 状態にしない(SPEC §7 v1.2)"
        );

        let files = w.file_report();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].asad, None);
        assert_eq!(
            std::fs::read(&files[0].path).unwrap(),
            ctrl,
            "ctrl ファイルの内容がバイト一致"
        );
        assert_eq!(
            files[0].path.parent().and_then(|p| p.file_name()),
            Some(std::ffi::OsStr::new("ctrl")),
            "ctrl/ サブディレクトリに置かれる: {:?}",
            files[0].path
        );
        let name = files[0].path.file_name().and_then(|n| n.to_str()).unwrap();
        assert_matches_ctrl_naming(name);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// v1.2 の核心: frameType を見ずにオフセット 27 だけを読むと、28 B を超える非 CoBo
    /// 制御フレームが誤った AsAd ファイルへ混入する。frameType-aware な `peek_asad` により
    /// このフレームは(28 B を超えていても)ctrl/ に振り分けられ、AsAd ファイルへは書かれない。
    #[test]
    fn a_long_non_cobo_frame_is_routed_to_ctrl_not_misinterpreted_as_asad_data() {
        let root = temp_output_root("ctrl-long-frame");
        let mut w = RunWriter::new(root.clone(), 3, 10 * 1024 * 1024);
        let ctrl = make_non_cobo_frame(7, 88, 0xAB); // 88 B、frameType=7(非 1/2)
        w.handle_batch(0, 3, 0, &[frame(&ctrl)]).unwrap();
        w.flush_all().unwrap();

        assert_eq!(w.ctrl_frames(), 1);
        assert!(!w.errored());
        let files = w.file_report();
        assert_eq!(files.len(), 1, "AsAd ファイルへ誤って混入していない");
        assert_eq!(
            files[0].asad, None,
            "ctrl ファイルとして書かれる(28 B 超でも frameType で弾く)"
        );
        assert_eq!(std::fs::read(&files[0].path).unwrap(), ctrl);

        let _ = std::fs::remove_dir_all(&root);
    }

    // -----------------------------------------------------------------
    // ローテーション境界: フレーム非分割・連結一致
    // -----------------------------------------------------------------

    #[test]
    fn rotation_boundary_frames_are_not_split_and_concatenation_matches() {
        let root = temp_output_root("rotation-boundary");
        let f1 = make_frame(0, 40); // 68 B
        let f2 = make_frame(0, 40); // 68 B
        let max_file_bytes = f1.len() as u64 + 10; // f1 単独は収まるが f1+f2 は収まらない
        let mut w = RunWriter::new(root.clone(), 5, max_file_bytes);
        w.handle_batch(0, 5, 0, &[frame(&f1), frame(&f2)]).unwrap();
        w.flush_all().unwrap();

        let files = w.file_report();
        assert_eq!(files.len(), 2);
        let idx0 = files.iter().find(|f| f.idx == 0).unwrap();
        let idx1 = files.iter().find(|f| f.idx == 1).unwrap();
        assert_eq!(
            std::fs::read(&idx0.path).unwrap(),
            f1,
            "idx0 は f1 のみ(分割されない)"
        );
        assert_eq!(
            std::fs::read(&idx1.path).unwrap(),
            f2,
            "idx1 は f2 のみ(分割されない)"
        );

        let mut concatenated = std::fs::read(&idx0.path).unwrap();
        concatenated.extend_from_slice(&std::fs::read(&idx1.path).unwrap());
        let mut expected = f1.clone();
        expected.extend_from_slice(&f2);
        assert_eq!(concatenated, expected, "連結すれば入力と一致");

        let _ = std::fs::remove_dir_all(&root);
    }

    // -----------------------------------------------------------------
    // 巨大フレーム(cur_bytes>0 ガード)
    // -----------------------------------------------------------------

    #[test]
    fn a_single_frame_larger_than_the_rotation_limit_is_written_whole() {
        let root = temp_output_root("huge-frame");
        let max_file_bytes = 32u64;
        let huge = make_frame(0, 200); // 228 B > 32
        let mut w = RunWriter::new(root.clone(), 9, max_file_bytes);
        w.handle_batch(0, 9, 0, &[frame(&huge)]).unwrap();
        w.flush_all().unwrap();

        let files = w.file_report();
        assert_eq!(
            files.len(),
            1,
            "cur_bytes==0 ガードで単発巨大フレームは分割されない"
        );
        assert_eq!(files[0].bytes, huge.len() as u64);
        assert_eq!(std::fs::read(&files[0].path).unwrap(), huge);

        // 続くフレームは(既に cur_bytes>0 なので)通常どおりローテーションされる
        let next = make_frame(0, 5);
        w.handle_batch(0, 9, 1, &[frame(&next)]).unwrap();
        w.flush_all().unwrap();
        assert_eq!(
            w.file_report().len(),
            2,
            "巨大フレームの後続フレームはローテーションされる"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    // -----------------------------------------------------------------
    // 遅延作成
    // -----------------------------------------------------------------

    #[test]
    fn files_are_created_lazily_on_first_frame_of_each_asad() {
        let root = temp_output_root("delayed-creation");
        let run_dir = root.join("run0004");
        let mut w = RunWriter::new(root.clone(), 4, 10 * 1024 * 1024);
        assert!(!run_dir.exists(), "run ディレクトリはまだ作られない");

        w.handle_batch(0, 4, 0, &[frame(&make_frame(2, 4))])
            .unwrap();
        assert!(
            run_dir.exists(),
            "最初のフレーム到着で run ディレクトリが作られる"
        );
        let files = w.file_report();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].asad, Some(2));

        let _ = std::fs::remove_dir_all(&root);
    }

    // -----------------------------------------------------------------
    // finalize: flush + fsync + close、以後は書けない
    // -----------------------------------------------------------------

    #[test]
    fn finalize_closes_every_open_file_and_keeps_the_file_report() {
        let root = temp_output_root("finalize");
        let mut w = RunWriter::new(root.clone(), 2, 10 * 1024 * 1024);
        let f = make_frame(0, 8);
        w.handle_batch(0, 2, 0, &[frame(&f)]).unwrap();
        w.finalize().unwrap();

        let files = w.file_report();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].bytes, f.len() as u64);
        assert_eq!(std::fs::read(&files[0].path).unwrap(), f);

        let _ = std::fs::remove_dir_all(&root);
    }
}
