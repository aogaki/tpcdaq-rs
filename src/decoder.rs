//! decoder — receiver 群の RawFrames を Fragments の**単一ストリーム**へ変換するコンポーネント。
//! SPEC §1.1(責務)/ §1.3(状態機械)/ §1.4(過負荷)/ §2.2(Batch/EOS/Heartbeat)/
//! §2.3(decoder のソース性 = 本ユニットの核)/ §2.4(Fragment)/ §3.1・§3.2([decoder]・
//! PULL 47002・REP 47101)。
//!
//! # 構成
//!
//! ```text
//!   receiver 0 ──PUSH─┐
//!   receiver 1 ──PUSH─┼▶ (PULL bind) [decoder 専用 OS スレッド] ──PUSH──▶ (PULL bind) root-sink
//!            …        ┘        │  RunDecoder(純コア: 検証・decode・バッチ詰め・EOS 集約)
//!                              ▼
//!                       [コマンド REP タスク](003 run_command_task)
//! ```
//!
//! [`RunDecoder`] が「入力バッチを受けて何を送るべきかを決める」純コア(ZMQ も IO も知らない
//! — decode/framer/graw_writer と同じ Clean Architecture の切り方)。ZMQ 配線
//! ([`run_decoder`])は [`RunDecoder`] を専用 OS スレッド上で駆動し、[`Emit`] の指示どおりに
//! 送るだけの薄い層にする。**変換そのものは既存の [`crate::decode::Decoder`]** が全部やる
//! (再実装しない)。
//!
//! # decoder のソース性(SPEC §2.3)
//!
//! 出力は `source_id = 100`・自前 sequence_number の**単一ストリーム**で、上流全 CoBo の
//! EOS 受領 + 対応 Fragment の送出完了後に**自分の EOS を 1 本**送る。CoBo の識別は
//! `Fragment.cobo` が担うので、下流(root-sink)は上流の数を知らなくてよい。
//!
//! # 停止設計(ロスレスと「畳めること」の両立)
//!
//! 通常運転の送出はブロッキング(ロスレス — 下流の背圧で待つ)。PUSH の sndtimeo は有限
//! ([`crate::config::DEFAULT_DECODER_SEND_TIMEOUT_MS`])だが、タイムアウトしても
//! **通常は諦めずに再試行する**。`Reset` コマンド処理中に限り送出待ちを打ち切り、
//! `batches_abandoned` / `eos_abandoned` としてカウント + warn する(Reset はオペレータの
//! 明示オーバーライド = 破棄が可視化されていれば許される唯一の経路)。`Stop` は打ち切らない。

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver as StdReceiver, Sender as StdSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_bytes::ByteBuf;
use tokio::sync::{broadcast, oneshot};
use tracing::{error, info, warn};

use crate::command::{run_command_task, Command, CommandResponse, ComponentState, RunConfig};
use crate::config::{
    Config, DECODER_COMMAND_LISTEN, DECODER_SOURCE_ID, DEFAULT_DECODER_SEND_TIMEOUT_MS,
    DEFAULT_HEARTBEAT_MS,
};
use crate::decode::{parse_topology, Decoder};
use crate::msg::{Batch, Fragment, Message, RawFrames};
use crate::zmq_helper;

/// PULL poll の待ち上限(ms)。停止合図の確認と Heartbeat の目覚まし
/// (receiver の `sender_loop` / root_sink と同じ 100 ms 級)。
///
/// 実際の待ち時間はこれではなく **「次のバッチ期限まで」をこの値で頭打ちにしたもの**なので、
/// SPEC §2.3 の「10 ms 経過で close」はこの上限を上げても鈍らない
/// ([`RunDecoder::next_deadline`] — receiver の `wake` と同じ流儀)。
const POLL_TIMEOUT_MS: i64 = 100;

// ---------------------------------------------------------------------
// 起動パラメタ
// ---------------------------------------------------------------------

/// decoder 1 プロセス分の起動パラメタ(= 設定の解決結果)。
///
/// テストはすべてのアドレスにポート 0 を入れて動的ポートを使う。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecoderParams {
    /// PULL bind アドレス(例 `tcp://*:47002`、SPEC §3.2)。receiver 群が PUSH connect する。
    pub pull_bind: String,
    /// root-sink の PULL bind への接続先(例 `tcp://127.0.0.1:47003`、SPEC §3.2)。
    pub push_connect: String,
    /// コマンド REP の bind アドレス(SPEC §3.2 v1.2: `tcp://*:47101`)。
    pub command_listen: String,
    /// 出力バッチを閉じるバイト数(SPEC §2.3)。
    pub batch_max_bytes: usize,
    /// 出力バッチを閉じる経過時間(ミリ秒、SPEC §2.3)。
    pub batch_max_ms: u64,
    /// アイドル時 Heartbeat の周期(ミリ秒、SPEC §2.2)。
    pub heartbeat_ms: u64,
    /// PUSH の送信タイムアウト(ミリ秒)。Reset 中の打ち切り粒度になる。
    pub send_timeout_ms: i32,
    /// 内部ワーカー数。**本ユニットでは未使用**(受理して > 1 なら info ログ)。
    pub workers: u32,
    /// 期待ソース集合(設定の `[[cobo]]` 全 id、SPEC §2.3 の EOS 集約)。
    pub expected_sources: Vec<u32>,
}

impl DecoderParams {
    /// 設定から起動パラメタを解決する。
    pub fn from_config(config: &Config) -> Self {
        Self {
            pull_bind: config.decoder.pull_bind.clone(),
            push_connect: config.decoder.push_connect.clone(),
            command_listen: DECODER_COMMAND_LISTEN.to_string(),
            batch_max_bytes: config.decoder.batch_max_bytes,
            batch_max_ms: config.decoder.batch_max_ms,
            heartbeat_ms: DEFAULT_HEARTBEAT_MS,
            send_timeout_ms: DEFAULT_DECODER_SEND_TIMEOUT_MS,
            workers: config.decoder.workers,
            expected_sources: config.cobo.iter().map(|c| c.id).collect(),
        }
    }
}

// ---------------------------------------------------------------------
// RunDecoder — 入力を検証・デコードし「何を送るか」を決める純コア(ZMQ 非依存)
// ---------------------------------------------------------------------

/// [`RunDecoder`] が呼び出し側に返す送出指示。
///
/// 実際の送信は ZMQ 層の仕事。コアはバッファと番号だけを持ち、送信の成否は
/// [`RunDecoder::batch_sent`] / [`RunDecoder::batch_abandoned`] などで戻してもらう。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Emit {
    /// 送るものはない。
    Nothing,
    /// 溜まった Fragments を 1 バッチとして送る。
    Batch,
    /// 残バッチを送ってから(空なら送らずに)**自分の EOS を 1 本**送る(SPEC §2.3)。
    FlushAndEndOfStream,
}

/// 1 run 分のデコード状態。受け取った `RawFrames` バッチを検証し、
/// [`crate::decode::Decoder`] で Fragment 化し、`batch_max_bytes` / `batch_max` で
/// 出力バッチを閉じ、上流全ソースの EOS を集約する(SPEC §2.3)。
///
/// ZMQ も時計も持たない(`now` は呼び出し側が渡す)ので、そのまま単体テストできる。
pub struct RunDecoder {
    run_number: u32,
    /// 直前に自分の EOS を出して再武装した状態。次の Batch/EOS の run 番号を採用する。
    awaiting_run: bool,
    expected_sources: HashSet<u32>,
    eos_received: HashSet<u32>,
    /// 変換の本体。004 の純コアをそのまま使う(再実装しない)。
    decoder: Decoder,
    /// Fragment 蓄積バッファ。**送出後も capacity を保って再利用**する(ホットパスで
    /// 確保し直さない — CLAUDE.md)。
    pending: Vec<Fragment>,
    /// `pending` の items バイト数合計(バッチ close 判定の材料)。
    pending_bytes: usize,
    batch_deadline: Option<Instant>,
    batch_max_bytes: usize,
    batch_max: Duration,
    out_seq: u64,
    next_seq: HashMap<u32, u64>,
    batches_in: HashMap<u32, u64>,
    frames_in: u64,
    fragments_out: u64,
    batches_out: u64,
    items_out: u64,
    seq_gaps: u64,
    run_mismatches: u64,
    eos_in: u64,
    heartbeats_in: u64,
    heartbeats_out: u64,
    heartbeats_abandoned: u64,
    /// **自分の** EOS を送り終えた回数(TODO/033-C)。`eos_abandoned` と対で、
    /// チェーンの**最後のホップ**(decoder → root-sink)を controller から観測できるようにする。
    /// `RunDecoder` は `Start` 毎に新規生成されるので run 内の値であり、`rearm` でも消さない
    /// (「この run の EOS を送り終えたか」を停止シーケンスが読む)。
    eos_out: u64,
    eos_abandoned: u64,
    batches_abandoned: u64,
    /// `Batch.source_id` と `Fragment.cobo`(GRAW ヘッダ実値)の不一致数(TODO/013-4)。
    /// 誤設定の早期検出用で、**Error にはしない**。
    cobo_mismatch: u64,
    errored: bool,
    /// unsupported / malformed のログはホットパスなので**初回だけ**出す
    /// (カウンタは常に進み metrics に出るので silent にはならない — receiver の
    /// `record_dropped_frame` と同じ流儀)。
    /// topology frame(frameType 7)の受領数(TODO/067-A)。`unsupported` の内数。
    /// CoBo は FDT データリンクの daqStart で 1 回だけ送るので、通常は run あたり
    /// リンク数ぶん(= 1)になる。
    topology_frames: u64,
    logged_unsupported: bool,
    logged_malformed: bool,
    logged_topology: bool,
    logged_cobo_mismatch: bool,
    logged_heartbeat_abandoned: bool,
}

impl RunDecoder {
    pub fn new(
        run_number: u32,
        expected_sources: HashSet<u32>,
        batch_max_bytes: usize,
        batch_max: Duration,
    ) -> Self {
        Self {
            run_number,
            awaiting_run: false,
            expected_sources,
            eos_received: HashSet::new(),
            decoder: Decoder::new(),
            pending: Vec::new(),
            pending_bytes: 0,
            batch_deadline: None,
            batch_max_bytes,
            batch_max,
            out_seq: 0,
            next_seq: HashMap::new(),
            batches_in: HashMap::new(),
            frames_in: 0,
            fragments_out: 0,
            batches_out: 0,
            items_out: 0,
            seq_gaps: 0,
            run_mismatches: 0,
            eos_in: 0,
            heartbeats_in: 0,
            heartbeats_out: 0,
            heartbeats_abandoned: 0,
            eos_out: 0,
            eos_abandoned: 0,
            batches_abandoned: 0,
            cobo_mismatch: 0,
            errored: false,
            topology_frames: 0,
            logged_unsupported: false,
            logged_malformed: false,
            logged_topology: false,
            logged_cobo_mismatch: false,
            logged_heartbeat_abandoned: false,
        }
    }

    /// 1 入力 Batch(`RawFrames`)を処理する。
    ///
    /// ソース毎 sequence_number 連続性と run_number 一致を検証し(違反は
    /// **カウント + Error ラッチ**、ただし**消費・送出は継続**する)、各フレームを
    /// [`crate::decode::Decoder`] へ渡して Fragment を蓄積する。
    ///
    /// 出力バッチの close 判定は**入力バッチ 1 通を処理し終えた時点**で行う。上流(receiver)も
    /// 同じ 8 MiB 規則でバッチを閉じるので、1 通の出力は概ね「入力バッチ相当 + 1 Fragment」に
    /// 収まる(フレーム途中で切って複数通に分けるより単純 — KISS)。
    pub fn handle_batch(
        &mut self,
        source_id: u32,
        run_number: u32,
        sequence_number: u64,
        frames: &[ByteBuf],
        now: Instant,
    ) -> Emit {
        self.check_run(source_id, run_number, "Data");

        let expected = self.next_seq.entry(source_id).or_insert(0);
        if sequence_number != *expected {
            self.seq_gaps += 1;
            self.errored = true;
            warn!(
                source_id,
                expected = *expected,
                got = sequence_number,
                "decoder: sequence_number gap on a lossless link (SPEC §2.2/§1.4-3)"
            );
        }
        *expected = sequence_number + 1;
        *self.batches_in.entry(source_id).or_insert(0) += 1;

        for frame in frames {
            self.frames_in += 1;
            let malformed_before = self.decoder.malformed();
            let unsupported_before = self.decoder.unsupported();
            match self.decoder.decode(frame) {
                Some(fragment) => {
                    if u32::from(fragment.cobo) != source_id {
                        self.cobo_mismatch += 1;
                        if !self.logged_cobo_mismatch {
                            self.logged_cobo_mismatch = true;
                            warn!(
                                source_id,
                                cobo = fragment.cobo,
                                "decoder: Batch.source_id != Fragment.cobo — likely a misconfigured \
                                 DataLinkSet or receiver --cobo-id. Not an error: the data is kept \
                                 and downstream identifies CoBo by Fragment.cobo (SPEC §2.3); \
                                 further occurrences are counted only"
                            );
                        }
                    }
                    self.pending_bytes += fragment.items.len();
                    self.pending.push(fragment);
                    if self.batch_deadline.is_none() {
                        self.batch_deadline = Some(now + self.batch_max);
                    }
                }
                None if self.decoder.malformed() > malformed_before => {
                    self.errored = true;
                    if !self.logged_malformed {
                        self.logged_malformed = true;
                        warn!(
                            source_id,
                            frame_len = frame.len(),
                            "decoder: malformed frame (frameType 1/2 but broken) — counting and \
                             entering Error; further occurrences are counted only"
                        );
                    }
                }
                None if self.decoder.unsupported() > unsupported_before => {
                    // topology frame(frameType 7)は CoBo が daqStart で必ず送る正常系。
                    // run 先頭に 1 回しか来ないので payload をデコードして INFO に出す
                    // (silent drop 禁止 — CLAUDE.md)。ログは run あたり初回だけ。
                    if let Some(topology) = parse_topology(frame) {
                        self.topology_frames += 1;
                        if !self.logged_topology {
                            self.logged_topology = true;
                            info!(
                                source_id,
                                cobo = topology.cobo,
                                asad_mask = format_args!("0x{:02x}", topology.asad_mask),
                                active_asads = topology.active_asads(),
                                two_p_mode = topology.two_p_mode,
                                "decoder: CoBo topology frame received (frameType 7) — skipped, \
                                 not an Error; it carries no event data (GET MemRead::sendTopology)"
                            );
                        }
                    } else if !self.logged_unsupported {
                        self.logged_unsupported = true;
                        info!(
                            source_id,
                            frame_len = frame.len(),
                            "decoder: unsupported frame (frameType not in {{1,2}}) — skipped, not \
                             an Error (graw-writer preserves it under ctrl/, SPEC v1.2 §7); \
                             further occurrences are counted only"
                        );
                    }
                }
                None => {
                    // decode が None を返しつつどちらのカウンタも動かないことは無い。
                    // 起きたら黙って落とさず可視化する(CLAUDE.md)。
                    self.errored = true;
                    warn!(
                        source_id,
                        frame_len = frame.len(),
                        "decoder: frame dropped without a decoder counter moving — unexpected"
                    );
                }
            }
        }

        if self.pending_bytes >= self.batch_max_bytes {
            Emit::Batch
        } else {
            Emit::Nothing
        }
    }

    /// 上流 1 ソースの EndOfStream を受ける(SPEC §2.3 の EOS 集約)。
    ///
    /// 期待ソース全部が揃ったときだけ [`Emit::FlushAndEndOfStream`] を返す。
    pub fn handle_eos(&mut self, source_id: u32, run_number: u32) -> Emit {
        self.eos_in += 1;
        self.check_run(source_id, run_number, "EndOfStream");
        if !self.expected_sources.contains(&source_id) {
            info!(
                source_id,
                expected = ?self.expected_sources,
                "decoder: EndOfStream from a source outside the expected set"
            );
        }
        self.eos_received.insert(source_id);
        if !self.expected_sources.is_subset(&self.eos_received) {
            return Emit::Nothing;
        }
        info!(
            run_number = self.run_number,
            sources = ?self.expected_sources,
            "decoder: all upstream sources reached EndOfStream — flushing and sending our own EOS"
        );
        Emit::FlushAndEndOfStream
    }

    /// 上流 Heartbeat は**数えるだけ**(転送しない、SPEC §2.2)。
    pub fn handle_heartbeat(&mut self, source_id: u32) {
        let _ = source_id;
        self.heartbeats_in += 1;
    }

    /// 時間経過による close 判定(SPEC §2.3「10 ms 経過」)。
    pub fn tick(&mut self, now: Instant) -> Emit {
        match self.batch_deadline {
            Some(deadline) if now >= deadline => Emit::Batch,
            _ => Emit::Nothing,
        }
    }

    /// 送出待ちの Fragments(ZMQ 層がそのまま `payload` として借用する)。
    pub fn pending(&self) -> &[Fragment] {
        &self.pending
    }

    /// 送出待ちのバッチがあるか(Heartbeat は**アイドル時のみ**なので必要)。
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// 溜まっているバッチの close 期限(空なら `None`)。ZMQ 層はこれを poll の待ち時間に使い、
    /// 「10 ms 経過で close」(SPEC §2.3)を待ち上限に鈍らされずに守る。
    pub fn next_deadline(&self) -> Option<Instant> {
        self.batch_deadline
    }

    /// 次に送る出力バッチの sequence_number(run 開始 0 から自前で単調増加、SPEC §2.2)。
    pub fn out_sequence_number(&self) -> u64 {
        self.out_seq
    }

    /// 現在の run 番号(出力 Batch/EOS に載せる)。
    pub fn run_number(&self) -> u32 {
        self.run_number
    }

    /// バッチ送出に成功した。カウンタを進めてバッファを空にする(capacity は保つ)。
    pub fn batch_sent(&mut self) {
        self.batches_out += 1;
        self.fragments_out += self.pending.len() as u64;
        // 1 item = 4 バイト(SPEC §2.4 の u32 パック)。
        self.items_out += (self.pending_bytes / 4) as u64;
        self.out_seq += 1;
        self.clear_pending();
    }

    /// バッチ送出を打ち切った。破棄は必ず数えて可視化する。
    ///
    /// 打ち切りが起きるのは 2 経路: **Reset 中の明示オーバーライド**(送出待ちの中断)と
    /// **そもそも送れないソケット**(符号化失敗 / ETERM / 一般 ZMQ エラー)。前者だけと
    /// 書いてあったのは実装との乖離だった(TODO/023-6 = P2 レビュー R-P2-13 の L3)。
    ///
    /// 自前 seq は進める — 下流にはギャップとして見えるべきで、詰めると
    /// 「落ちていない」と誤認される(receiver の flush と同じ理屈)。
    pub fn batch_abandoned(&mut self) {
        self.batches_abandoned += 1;
        self.out_seq += 1;
        warn!(
            run_number = self.run_number,
            fragments = self.pending.len(),
            batches_abandoned = self.batches_abandoned,
            "decoder: Fragments batch abandoned (Reset override or unsendable socket) — \
             this data is lost and counted, never silently dropped"
        );
        self.clear_pending();
    }

    /// 自分の EOS を送り終えた。run 状態(EOS 集合・入力 seq 検証・自前 seq)を再武装する。
    pub fn eos_sent(&mut self) {
        self.eos_out += 1;
        info!(
            run_number = self.run_number,
            "decoder: own EndOfStream sent — rearming for the next run"
        );
        self.rearm();
    }

    /// 自分の EOS 送出を打ち切った(経路は [`RunDecoder::batch_abandoned`] と同じ 2 つ)。
    pub fn eos_abandoned(&mut self) {
        self.eos_abandoned += 1;
        warn!(
            run_number = self.run_number,
            "decoder: own EndOfStream abandoned (Reset override or unsendable socket) — \
             downstream will not see this run's barrier; counted, never silently dropped"
        );
        self.rearm();
    }

    /// アイドル Heartbeat を送出できた(SPEC §2.2)。Batch/EOS と同じく out/abandoned を
    /// 対にして数える(TODO/023-5 = R-P2-12。以前は `let _ =` で結果を捨てていた)。
    pub fn heartbeat_sent(&mut self) {
        self.heartbeats_out += 1;
    }

    /// Heartbeat の送出を打ち切った(符号化失敗・Reset 中の中断・送れないソケット)。
    /// Heartbeat は 1 Hz で繰り返し来るのでログは**初回だけ**(以降はカウンタが担う)。
    pub fn heartbeat_abandoned(&mut self) {
        self.heartbeats_abandoned += 1;
        if !self.logged_heartbeat_abandoned {
            self.logged_heartbeat_abandoned = true;
            info!(
                run_number = self.run_number,
                "decoder: Heartbeat abandoned (unsendable socket or Reset override) — counted; \
                 further occurrences are counted only"
            );
        }
    }

    /// プロトコル違反(seq ギャップ・EOS 前 run 変更・malformed)を一度でも踏んだか。
    pub fn errored(&self) -> bool {
        self.errored
    }

    pub fn frames_in(&self) -> u64 {
        self.frames_in
    }

    pub fn batches_in_for(&self, source_id: u32) -> u64 {
        self.batches_in.get(&source_id).copied().unwrap_or(0)
    }

    pub fn fragments_out(&self) -> u64 {
        self.fragments_out
    }

    pub fn batches_out(&self) -> u64 {
        self.batches_out
    }

    pub fn items_out(&self) -> u64 {
        self.items_out
    }

    pub fn unsupported(&self) -> u64 {
        self.decoder.unsupported()
    }

    /// 受領した topology frame(frameType 7)の数(TODO/067-A)。`unsupported` の内数。
    pub fn topology_frames(&self) -> u64 {
        self.topology_frames
    }

    pub fn malformed(&self) -> u64 {
        self.decoder.malformed()
    }

    pub fn seq_gaps(&self) -> u64 {
        self.seq_gaps
    }

    pub fn run_mismatches(&self) -> u64 {
        self.run_mismatches
    }

    pub fn eos_in(&self) -> u64 {
        self.eos_in
    }

    /// 自分の EOS を送り終えた回数(TODO/033-C)。停止シーケンスの `eos_complete` 判定の
    /// 3 点目 —— これが 0 のままなら run のバリアは root-sink に届いていない。
    pub fn eos_out(&self) -> u64 {
        self.eos_out
    }

    pub fn batches_abandoned(&self) -> u64 {
        self.batches_abandoned
    }

    pub fn heartbeats_out(&self) -> u64 {
        self.heartbeats_out
    }

    pub fn heartbeats_abandoned(&self) -> u64 {
        self.heartbeats_abandoned
    }

    /// `Batch.source_id` と `Fragment.cobo` が食い違ったフレーム数(TODO/013-4)。
    pub fn cobo_mismatch(&self) -> u64 {
        self.cobo_mismatch
    }

    /// `CommandResponse.metrics` に載せる JSON(TODO/009 のカウンタ一式)。
    pub fn metrics_json(&self) -> serde_json::Value {
        let mut batches_in = serde_json::Map::new();
        for (source_id, n) in &self.batches_in {
            batches_in.insert(source_id.to_string(), serde_json::json!(n));
        }
        serde_json::json!({
            "run_number": self.run_number,
            "batches_in": serde_json::Value::Object(batches_in),
            "frames_in": self.frames_in,
            "fragments_out": self.fragments_out,
            "batches_out": self.batches_out,
            "items_out": self.items_out,
            "unsupported": self.decoder.unsupported(),
            "topology_frames": self.topology_frames,
            "malformed": self.decoder.malformed(),
            "seq_gaps": self.seq_gaps,
            "run_mismatches": self.run_mismatches,
            "eos_in": self.eos_in,
            "heartbeats_in": self.heartbeats_in,
            "heartbeats_out": self.heartbeats_out,
            "heartbeats_abandoned": self.heartbeats_abandoned,
            "eos_out": self.eos_out,
            "eos_abandoned": self.eos_abandoned,
            "batches_abandoned": self.batches_abandoned,
            "cobo_mismatch": self.cobo_mismatch,
        })
    }

    // -------------------------------------------------------------
    // 内部実装
    // -------------------------------------------------------------

    /// run 番号の一致を見る。再武装直後(自分の EOS 送出後)は新しい run 番号を採用し、
    /// それ以外で食い違えば **EOS 前の run 変更 = プロトコル違反**として数える。
    fn check_run(&mut self, source_id: u32, run_number: u32, kind: &'static str) {
        if self.awaiting_run {
            self.awaiting_run = false;
            if run_number != self.run_number {
                info!(
                    previous = self.run_number,
                    run_number, "decoder: new run adopted after rearming"
                );
                self.run_number = run_number;
            }
            return;
        }
        if run_number != self.run_number {
            self.run_mismatches += 1;
            self.errored = true;
            warn!(
                source_id,
                kind,
                expected = self.run_number,
                got = run_number,
                "decoder: run_number changed before EndOfStream — protocol violation (SPEC §2.2)"
            );
        }
    }

    fn clear_pending(&mut self) {
        self.pending.clear(); // capacity は保つ(バッファ再利用)
        self.pending_bytes = 0;
        self.batch_deadline = None;
    }

    fn rearm(&mut self) {
        self.eos_received.clear();
        self.next_seq.clear();
        self.out_seq = 0;
        self.awaiting_run = true;
        self.clear_pending();
    }
}

// ---------------------------------------------------------------------
// ZMQ 配線(受信・デコード・送出を 1 本の専用 OS スレッドで、006/007 と同じ流儀)
// ---------------------------------------------------------------------

/// 1 メッセージ送出の結末。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SendOutcome {
    Sent,
    /// Reset 中の打ち切り(または送れないソケット)。破棄はカウント + warn 済み。
    Abandoned,
}

/// ロスレス送出: sndtimeo に達しても **Reset 中でなければ諦めない**(下流の背圧で待つ)。
fn send_lossless(push: &zmq::Socket, bytes: &[u8], abandon: &AtomicBool) -> SendOutcome {
    let mut waited = false;
    loop {
        match push.send(bytes, 0) {
            Ok(()) => {
                if waited {
                    info!("decoder: downstream backpressure released — batch sent");
                }
                return SendOutcome::Sent;
            }
            Err(zmq::Error::EAGAIN) => {
                if abandon.load(Ordering::Relaxed) {
                    return SendOutcome::Abandoned;
                }
                if !waited {
                    waited = true;
                    info!(
                        "decoder: downstream is full — blocking on backpressure (lossless; only \
                         Reset may abandon it)"
                    );
                }
            }
            Err(zmq::Error::ETERM) => {
                info!("decoder: ZMQ context terminated — cannot send");
                return SendOutcome::Abandoned;
            }
            Err(e) => {
                error!(error = %e, "decoder: ZMQ send failed");
                return SendOutcome::Abandoned;
            }
        }
    }
}

/// 溜まっている Fragments を 1 バッチとして送る(空なら何もしない)。
///
/// **符号化はコアのロックを持ったまま・送信はロックを離してから**行う。送信は下流の背圧で
/// 長時間ブロックしうるので、その間もコマンド REP(GetStatus)がメトリクスを読めるようにする。
fn send_pending(push: &zmq::Socket, core: &Mutex<RunDecoder>, abandon: &AtomicBool) -> bool {
    let bytes = {
        let Ok(c) = core.lock() else {
            error!("decoder: RunDecoder mutex poisoned");
            return false;
        };
        if !c.has_pending() {
            return true;
        }
        let message = Message::Data(Batch {
            source_id: DECODER_SOURCE_ID,
            run_number: c.run_number(),
            sequence_number: c.out_sequence_number(),
            created_ns: unix_nanos(),
            payload: c.pending(),
        });
        match message.to_msgpack() {
            Ok(bytes) => bytes,
            Err(e) => {
                error!(error = %e, "decoder: cannot encode Fragments batch");
                drop(c);
                if let Ok(mut c) = core.lock() {
                    c.batch_abandoned();
                }
                return false;
            }
        }
    };

    let outcome = send_lossless(push, &bytes, abandon);
    match core.lock() {
        Ok(mut c) => match outcome {
            SendOutcome::Sent => c.batch_sent(),
            SendOutcome::Abandoned => c.batch_abandoned(),
        },
        Err(_) => error!("decoder: RunDecoder mutex poisoned after send"),
    }
    outcome == SendOutcome::Sent
}

/// 自分の EOS を 1 本送る(SPEC §2.3)。EOS はいかなる間引き規則からも除外される
/// (SPEC §2.2)ので、Reset 中以外は送れるまで待つ。
fn send_own_eos(push: &zmq::Socket, core: &Mutex<RunDecoder>, abandon: &AtomicBool) {
    let (bytes, run_number) = {
        let Ok(c) = core.lock() else {
            error!("decoder: RunDecoder mutex poisoned");
            return;
        };
        let run_number = c.run_number();
        let eos = Message::<RawFrames>::EndOfStream {
            source_id: DECODER_SOURCE_ID,
            run_number,
        };
        match eos.to_msgpack() {
            Ok(bytes) => (bytes, run_number),
            Err(e) => {
                error!(error = %e, "decoder: cannot encode EndOfStream");
                drop(c);
                if let Ok(mut c) = core.lock() {
                    c.eos_abandoned();
                }
                return;
            }
        }
    };
    let outcome = send_lossless(push, &bytes, abandon);
    match core.lock() {
        Ok(mut c) => match outcome {
            SendOutcome::Sent => {
                info!(
                    run_number,
                    source_id = DECODER_SOURCE_ID,
                    "decoder: EndOfStream sent"
                );
                c.eos_sent();
            }
            SendOutcome::Abandoned => c.eos_abandoned(),
        },
        Err(_) => error!("decoder: RunDecoder mutex poisoned after EOS send"),
    }
}

/// アイドル時 Heartbeat(SPEC §2.2、1 Hz)。落としてよいものではないが、下流が詰まって
/// いるときは通常の送出と同じくブロックする(Reset のみ打ち切り可)。
///
/// 送出の成否は **必ずカウンタへ戻す**(TODO/023-5 = R-P2-12。Batch/EOS が out/abandoned の
/// 対を持つのに Heartbeat だけ結果を捨てていた = 死活判定の材料が無音で消えていた)。
fn send_heartbeat(
    push: &zmq::Socket,
    core: &Mutex<RunDecoder>,
    abandon: &AtomicBool,
    counter: u64,
) {
    let bytes = {
        let Ok(c) = core.lock() else {
            error!("decoder: RunDecoder mutex poisoned");
            return;
        };
        let heartbeat = Message::<RawFrames>::Heartbeat {
            source_id: DECODER_SOURCE_ID,
            run_number: c.run_number(),
            counter,
        };
        match heartbeat.to_msgpack() {
            Ok(bytes) => bytes,
            Err(e) => {
                error!(error = %e, "decoder: cannot encode Heartbeat");
                drop(c);
                if let Ok(mut c) = core.lock() {
                    c.heartbeat_abandoned();
                }
                return;
            }
        }
    };
    let outcome = send_lossless(push, &bytes, abandon);
    match core.lock() {
        Ok(mut c) => match outcome {
            SendOutcome::Sent => c.heartbeat_sent(),
            SendOutcome::Abandoned => c.heartbeat_abandoned(),
        },
        Err(_) => error!("decoder: RunDecoder mutex poisoned after Heartbeat send"),
    }
}

/// PULL poll → recv → decode → PUSH send を 1 本の専用 OS スレッドで回す(006/007 と同じ流儀)。
///
/// 終了条件は停止合図(`Stop` / `Reset`)のみ。全ソース EOS を受けても run は再武装され、
/// 次の run のデータをそのまま処理できる(コンポーネントを畳むのは controller の仕事)。
///
/// # なぜ recv の前に必ず poll するのか(TODO/013 — 計測で確定した 2 ソース飢餓の修正)
///
/// PULL の fair-queue(libzmq `fq_t`)は「自分の番で空だったパイプ」を**非活性化**して
/// ラウンドロビンから外す。外れたパイプの**再活性化はコマンドとして**ソケットのメールボックスへ
/// 届き、`process_commands()` でしか取り込まれない。ところが `zmq_recv` がそれを呼ぶのは
/// **成功 100 回に 1 度**(libzmq の `inbound_poll_rate`)か **EAGAIN を返したとき**だけである。
///
/// decoder が入力に追いつけないときは残ったパイプに常にデータがあるので `recv` は EAGAIN を
/// 返さない。つまり **一度外れた上流は 100 通ぶん戻ってこられない**。ELITPC(2 CoBo)の実測では
/// これが「片 CoBo を run 丸ごと飢餓させ、下流のイベントビルダが全イベントを incomplete にする」
/// という形で表面化した(TODO/012 の「発見した不具合」)。
///
/// `zmq_poll` は対象ソケットの `ZMQ_EVENTS` を読み、その中で `process_commands()` が走る。
/// だから **recv の前に poll を 1 回挟むだけ**で外れたパイプが必ず戻る。**受信予算は設けない**
/// (読めるなら読み続ける)— 公平性を担保しているのは「何通で区切るか」ではなく
/// 「recv と recv の間で `process_commands()` が走るか」だけである(TODO/013 の計測: 1 回の
/// wake で EAGAIN まで drain しても、途中に poll が無ければ飢餓は消えない)。
fn run_decoder_thread(
    pull: zmq::Socket,
    push: zmq::Socket,
    core: Arc<Mutex<RunDecoder>>,
    abandon: Arc<AtomicBool>,
    heartbeat_interval: Duration,
    stop_rx: StdReceiver<()>,
) {
    let mut heartbeat_counter = 0u64;
    let mut last_send = Instant::now();

    'consume: loop {
        if stop_rx.try_recv().is_ok() {
            info!("decoder: Stop/Reset received — flushing what is left");
            break 'consume;
        }

        // 次の目覚まし = 溜まっているバッチの close 期限(SPEC §2.3 の 10 ms)。
        // 何も溜まっていなければ上限まで寝る(tight loop にしない — receiver と同じ流儀)。
        //
        // ミリ秒への丸めは**切り上げ**る。切り捨てると期限の手前 1 ms 未満で `poll(0)` を
        // 回し続ける busy loop になり、バッチ 1 通ごとに CPU を焼く(CLAUDE.md: ホットパスで
        // 無駄をしない)。1 ms 遅く閉じるほうが害がない。
        let wait_ms = {
            let Ok(c) = core.lock() else {
                error!("decoder: RunDecoder mutex poisoned — stopping");
                break 'consume;
            };
            match c.next_deadline() {
                Some(deadline) => {
                    const NANOS_PER_MS: u128 = 1_000_000;
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    i64::try_from(remaining.as_nanos().div_ceil(NANOS_PER_MS))
                        .unwrap_or(POLL_TIMEOUT_MS)
                        .min(POLL_TIMEOUT_MS)
                }
                None => POLL_TIMEOUT_MS,
            }
        };

        // ここが 013 の修正本体(理由は関数ドキュメント)。
        let readable = {
            let mut items = [pull.as_poll_item(zmq::POLLIN)];
            match zmq::poll(&mut items, wait_ms) {
                Ok(_) => items[0].is_readable(),
                Err(zmq::Error::ETERM) => break 'consume,
                // シグナルによる中断は失敗ではない(次の周回で待ち直す)。
                Err(zmq::Error::EINTR) => false,
                Err(e) => {
                    warn!(error = %e, "decoder: PULL poll failed");
                    false
                }
            }
        };

        let received = if readable {
            // 待つのは poll の仕事なので recv はノンブロッキング。
            match pull.recv_bytes(zmq::DONTWAIT) {
                Ok(raw) => Some(raw),
                Err(zmq::Error::EAGAIN) => None,
                Err(zmq::Error::ETERM) => break 'consume,
                Err(e) => {
                    warn!(error = %e, "decoder: PULL recv failed");
                    None
                }
            }
        } else {
            None
        };

        let emit = {
            let Ok(mut c) = core.lock() else {
                error!("decoder: RunDecoder mutex poisoned — stopping");
                break 'consume;
            };
            let from_message = match received {
                Some(raw) => match Message::<RawFrames>::from_msgpack(&raw) {
                    Ok(Message::Data(batch)) => c.handle_batch(
                        batch.source_id,
                        batch.run_number,
                        batch.sequence_number,
                        &batch.payload,
                        Instant::now(),
                    ),
                    Ok(Message::EndOfStream {
                        source_id,
                        run_number,
                    }) => c.handle_eos(source_id, run_number),
                    Ok(Message::Heartbeat { source_id, .. }) => {
                        c.handle_heartbeat(source_id);
                        Emit::Nothing
                    }
                    Err(e) => {
                        warn!(error = %e, "decoder: cannot decode message — skipped");
                        Emit::Nothing
                    }
                },
                None => Emit::Nothing,
            };
            // サイズで閉じなかったときは必ず時間の期限も見る。入力が途切れない間は recv が
            // EAGAIN を返さないので、ここで見ないと「10 ms 経過で close」(SPEC §2.3)が
            // 高レート時だけ効かなくなる。
            match from_message {
                Emit::Nothing => c.tick(Instant::now()),
                other => other,
            }
        };

        match emit {
            Emit::Nothing => {}
            Emit::Batch => {
                send_pending(&push, &core, &abandon);
                last_send = Instant::now();
            }
            Emit::FlushAndEndOfStream => {
                send_pending(&push, &core, &abandon);
                send_own_eos(&push, &core, &abandon);
                last_send = Instant::now();
            }
        }

        // アイドル時のみ Heartbeat(データ到着中は不要 — SPEC §2.2)。
        if last_send.elapsed() >= heartbeat_interval {
            let idle = core.lock().map(|c| !c.has_pending()).unwrap_or(false);
            if idle {
                send_heartbeat(&push, &core, &abandon, heartbeat_counter);
                heartbeat_counter += 1;
                last_send = Instant::now();
            }
        }
    }

    // 抜ける前に残バッチを出す(ロスレス。Reset 中なら打ち切って可視化される)。
    send_pending(&push, &core, &abandon);
    info!("decoder thread stopped");
}

fn unix_nanos() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => u64::try_from(d.as_nanos()).unwrap_or(u64::MAX),
        Err(_) => 0,
    }
}

// ---------------------------------------------------------------------
// コマンドハンドラ(状態機械、SPEC §1.3)
// ---------------------------------------------------------------------

struct Handler {
    params: DecoderParams,
    context: zmq::Context,
    state: ComponentState,
    run_number: Option<u32>,
    pull_socket: Option<zmq::Socket>,
    bind_address: Option<String>,
    core: Option<Arc<Mutex<RunDecoder>>>,
    abandon: Option<Arc<AtomicBool>>,
    stop_tx: Option<StdSender<()>>,
}

impl Handler {
    fn new(params: DecoderParams) -> Self {
        Self {
            params,
            context: zmq::Context::new(),
            state: ComponentState::Idle,
            run_number: None,
            pull_socket: None,
            bind_address: None,
            core: None,
            abandon: None,
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

    /// [`RunDecoder`] がプロトコル違反を報告していたら状態へ反映する(007 と同じ流儀)。
    ///
    /// **poisoned Mutex は「エラーなし」に丸めない**(TODO/023-1 = R-P2-8): ロックが毒され
    /// ているのは「デコードスレッドがロック保持中に panic して死んだ」ときだけで、それを
    /// `unwrap_or(false)` で握り潰すと **スレッド全停止でも state=Running のまま GetStatus が
    /// success を返し続ける**(delila-rs 2026-05-04 事案と同型の不可視化)。即 Error にする。
    fn latch_error(&mut self) {
        if self.state == ComponentState::Error
            || !self.state.can_transition_to(ComponentState::Error)
        {
            return;
        }
        let is_errored = match self.core.as_ref() {
            None => false,
            Some(core) => match core.lock() {
                Ok(guard) => guard.errored(),
                Err(_) => {
                    warn!(
                        "decoder: worker thread panicked while holding the core lock (poisoned \
                         mutex) — entering Error"
                    );
                    true
                }
            },
        };
        if !is_errored {
            return;
        }
        warn!("decoder: entering Error (protocol violation, SPEC §1.4-3)");
        self.state = ComponentState::Error;
    }

    fn do_configure(&mut self, cfg: &RunConfig) -> Result<(), String> {
        self.run_number = Some(cfg.run_number);
        Ok(())
    }

    /// listen-before-start と同じ理屈: Arm で PULL を bind しておく(SPEC §1.3)。
    fn do_arm(&mut self) -> Result<(), String> {
        let (socket, endpoint) = zmq_helper::bind_pull(&self.context, &self.params.pull_bind)?;
        info!(%endpoint, "decoder: armed — PULL bound (SPEC §3.2)");
        self.bind_address = Some(endpoint);
        self.pull_socket = Some(socket);
        Ok(())
    }

    fn do_start(&mut self, run_number: u32) -> Result<(), String> {
        let Some(pull) = self.pull_socket.take() else {
            return Err("no PULL socket: Arm again before Start".to_string());
        };
        let push = match self.build_push() {
            Ok(push) => push,
            Err(e) => {
                // PUSH が張れないなら PULL は握ったまま Armed に留まり、Start を再試行できる。
                self.pull_socket = Some(pull);
                return Err(e);
            }
        };
        self.run_number = Some(run_number);

        let core = Arc::new(Mutex::new(RunDecoder::new(
            run_number,
            self.params.expected_sources.iter().copied().collect(),
            self.params.batch_max_bytes,
            Duration::from_millis(self.params.batch_max_ms),
        )));
        self.core = Some(Arc::clone(&core));
        let abandon = Arc::new(AtomicBool::new(false));
        self.abandon = Some(Arc::clone(&abandon));

        let (stop_tx, stop_rx) = std::sync::mpsc::channel();
        self.stop_tx = Some(stop_tx);
        let heartbeat_interval = Duration::from_millis(self.params.heartbeat_ms);

        std::thread::Builder::new()
            .name("decoder".to_string())
            .spawn(move || {
                run_decoder_thread(pull, push, core, abandon, heartbeat_interval, stop_rx)
            })
            .map_err(|e| format!("cannot spawn decoder thread: {e}"))?;
        Ok(())
    }

    /// `Stop` は送出を打ち切らない(EOS/残バッチの送出完了まで待つ = ロスレス)。
    fn do_stop(&mut self) -> Result<(), String> {
        self.signal_stop();
        self.bind_address = None;
        Ok(())
    }

    /// `Reset` はオペレータの明示オーバーライド。送出待ちを打ち切れる唯一の経路で、
    /// 破棄は `batches_abandoned` / `eos_abandoned` として可視化される。
    ///
    /// **コア(= カウンタ)は捨てない**: 破棄が可視化されていることが Reset を許す条件なので、
    /// Reset 後の `GetStatus` でも実績を読めなければ意味がない。
    fn do_reset(&mut self) -> Result<(), String> {
        if let Some(abandon) = &self.abandon {
            abandon.store(true, Ordering::Relaxed);
        }
        self.signal_stop();
        self.pull_socket = None;
        self.bind_address = None;
        self.run_number = None;
        Ok(())
    }

    fn signal_stop(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(()); // スレッドが既に居なくても構わない
        }
    }

    /// 下流(root-sink)への PUSH を張る。HWM と `ZMQ_IMMEDIATE`(下流不在なら送出は
    /// ブロックする、SPEC §1.2 v1.4)は必ず [`zmq_helper`] 経由 — ロスレス PUSH の設定は
    /// あそこ 1 箇所に集約されており、ここで個別に焼き込まない。sndtimeo だけは
    /// このコンポーネント固有(Reset 時に打ち切れる粒度)なのでここで設定する。
    /// すべて connect の前に設定する。
    fn build_push(&self) -> Result<zmq::Socket, String> {
        let socket = self
            .context
            .socket(zmq::PUSH)
            .map_err(|e| format!("cannot create PUSH socket: {e}"))?;
        zmq_helper::apply_push_hwm(&socket).map_err(|e| format!("cannot set PUSH HWM: {e}"))?;
        socket
            .set_sndtimeo(self.params.send_timeout_ms)
            .map_err(|e| format!("cannot set PUSH send timeout: {e}"))?;
        socket
            .connect(&self.params.push_connect)
            .map_err(|e| format!("cannot connect to {}: {e}", self.params.push_connect))?;
        info!(
            endpoint = self.params.push_connect,
            "decoder: PUSH connected to root-sink"
        );
        Ok(socket)
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
            .core
            .as_ref()
            .and_then(|c| c.lock().ok())
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
        self.signal_stop();
    }
}

// ---------------------------------------------------------------------
// エントリポイント
// ---------------------------------------------------------------------

/// decoder を起動し、`shutdown` が来るまでコマンドを処理し続ける。
///
/// `bound_command_endpoint` はコマンド REP の実 bind アドレスを 1 度だけ通知する
/// (`tcp://127.0.0.1:0` を使うテスト用。production は `None`)。
pub async fn run_decoder(
    params: DecoderParams,
    shutdown: broadcast::Receiver<()>,
    bound_command_endpoint: Option<oneshot::Sender<String>>,
) {
    info!(
        pull_bind = params.pull_bind,
        push_connect = params.push_connect,
        command_listen = params.command_listen,
        batch_max_bytes = params.batch_max_bytes,
        batch_max_ms = params.batch_max_ms,
        expected_sources = ?params.expected_sources,
        "decoder starting"
    );
    if params.workers > 1 {
        info!(
            workers = params.workers,
            "decoder: [decoder] workers is accepted but unused — single-threaded until measured \
             on the real system (P5, SPEC §13)"
        );
    }
    let command_listen = params.command_listen.clone();
    let mut handler = Handler::new(params);
    run_command_task(
        command_listen,
        "decoder",
        shutdown,
        bound_command_endpoint,
        move |cmd| handler.handle(cmd),
    )
    .await;
}

// ---------------------------------------------------------------------
// テスト(仕様書 — 先に書く)。RunDecoder 単体(ZMQ なし・IO なしの純コア)。
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_bytes::ByteBuf;

    const HEADER_BYTES: usize = 88;

    /// `bytes` へ `value` を `n` バイトのビッグエンディアン符号なし整数として書く
    /// (decode.rs の `read_uint` の逆演算。テストフィクスチャ組み立て専用)。
    fn write_uint(buf: &mut [u8], offset: usize, value: u64, n: usize) {
        for (i, b) in buf[offset..offset + n].iter_mut().enumerate() {
            *b = ((value >> (8 * (n - 1 - i))) & 0xFF) as u8;
        }
    }

    /// frameType 1(4B item、blkSize=1、big-endian)の CoBo フレームを手組みする。
    /// ヘッダの cobo/asad/event_idx を呼び出し側が決められるようにしてあるのは、
    /// **下流が CoBo を Fragment.cobo で識別する**(SPEC §2.3)ことを検証するため。
    fn make_frame(cobo: u8, asad: u8, event_idx: u32, items: &[(u8, u8, u16, u16)]) -> Vec<u8> {
        let total = HEADER_BYTES + items.len() * 4;
        let mut b = vec![0u8; total];
        b[0] = 0x00; // metaType: blkSize = 2^0 = 1、bit7=0 = big-endian
        write_uint(&mut b, 1, total as u64, 3);
        write_uint(&mut b, 5, 1, 2); // frameType = 1
        write_uint(&mut b, 8, HEADER_BYTES as u64, 2); // headerSize(blk 単位、blkSize=1)
        write_uint(&mut b, 10, 4, 2); // itemSize
        write_uint(&mut b, 12, items.len() as u64, 4);
        write_uint(&mut b, 22, u64::from(event_idx), 4);
        b[26] = cobo;
        b[27] = asad;
        for (i, &(aget, chan, bucket, adc)) in items.iter().enumerate() {
            let w = (u32::from(aget & 0x3) << 30)
                | (u32::from(chan & 0x7F) << 23)
                | (u32::from(bucket & 0x1FF) << 14)
                | u32::from(adc & 0xFFF);
            write_uint(&mut b, HEADER_BYTES + i * 4, u64::from(w), 4);
        }
        b
    }

    /// 8 item(= 32 バイト)の非対称なフレーム。1 Fragment = items 32 B になる。
    fn frame_with_8_items(cobo: u8, event_idx: u32) -> Vec<u8> {
        let items: Vec<(u8, u8, u16, u16)> =
            (0..8u16).map(|i| (0u8, i as u8, i, 100 + i * 7)).collect();
        make_frame(cobo, 0, event_idx, &items)
    }

    fn buf(bytes: &[u8]) -> ByteBuf {
        ByteBuf::from(bytes.to_vec())
    }

    /// frameType ∉ {1,2} の制御フレーム(実 2025 run 先頭の frameType 7・12 B が実例)。
    fn control_frame() -> Vec<u8> {
        let mut b = vec![0u8; 12];
        b[0] = 0x00; // big-endian
        write_uint(&mut b, 5, 7, 2); // frameType = 7
        b
    }

    fn core(run: u32, sources: &[u32]) -> RunDecoder {
        RunDecoder::new(
            run,
            sources.iter().copied().collect(),
            8 * 1024 * 1024,
            Duration::from_millis(10),
        )
    }

    // -----------------------------------------------------------------
    // EOS 集約(SPEC §2.3 — decoder のソース性)
    // -----------------------------------------------------------------

    #[test]
    fn eos_is_emitted_only_after_every_expected_source_has_reported() {
        let mut c = core(4, &[0, 1]);
        let now = Instant::now();
        c.handle_batch(0, 4, 0, &[buf(&frame_with_8_items(0, 1))], now);
        c.handle_batch(1, 4, 0, &[buf(&frame_with_8_items(1, 1))], now);

        assert_eq!(
            c.handle_eos(0, 4),
            Emit::Nothing,
            "片方だけでは自分の EOS を出さない"
        );
        assert_eq!(
            c.handle_eos(1, 4),
            Emit::FlushAndEndOfStream,
            "全ソース到達で残バッチ flush + 自分の EOS 1 本"
        );
        assert_eq!(c.eos_in(), 2);
    }

    #[test]
    fn a_repeated_eos_does_not_emit_a_second_one() {
        let mut c = core(4, &[0, 1]);
        assert_eq!(c.handle_eos(0, 4), Emit::Nothing);
        assert_eq!(
            c.handle_eos(0, 4),
            Emit::Nothing,
            "重複 EOS は集合なので無効"
        );
        assert_eq!(c.handle_eos(1, 4), Emit::FlushAndEndOfStream);
        c.eos_sent();
        assert_eq!(
            c.handle_eos(0, 4),
            Emit::Nothing,
            "EOS 送出後は再武装済み — 1 ソースだけでは出ない"
        );
    }

    #[test]
    fn the_run_state_rearms_after_the_eos_so_the_next_run_starts_from_zero() {
        let mut c = core(4, &[0, 1]);
        let now = Instant::now();
        c.handle_batch(0, 4, 0, &[buf(&frame_with_8_items(0, 1))], now);
        c.handle_batch(1, 4, 0, &[buf(&frame_with_8_items(1, 1))], now);
        c.handle_eos(0, 4);
        assert_eq!(c.handle_eos(1, 4), Emit::FlushAndEndOfStream);
        c.batch_sent();
        assert_eq!(c.out_sequence_number(), 1, "1 バッチ送出後は seq = 1");
        c.eos_sent();

        // 再 run: 自前 seq も入力側 seq 検証も 0 から。run 番号は新しいものを採る。
        assert_eq!(c.out_sequence_number(), 0);
        assert_eq!(
            c.run_number(),
            4,
            "次の Batch を見るまでは前の run 番号のまま"
        );
        c.handle_batch(0, 5, 0, &[buf(&frame_with_8_items(0, 2))], now);
        assert_eq!(c.run_number(), 5, "再武装後の最初の Batch で新 run を採用");
        assert_eq!(c.run_mismatches(), 0, "再武装後の run 変化は違反ではない");
        c.handle_batch(1, 5, 0, &[buf(&frame_with_8_items(1, 2))], now);
        c.handle_eos(0, 5);
        assert_eq!(
            c.handle_eos(1, 5),
            Emit::FlushAndEndOfStream,
            "次の run でも EOS 集約が働く"
        );
    }

    // -----------------------------------------------------------------
    // バッチ close 条件(SPEC §2.3「8 MiB 到達 or 10 ms 経過」)
    // -----------------------------------------------------------------

    #[test]
    fn a_batch_closes_when_the_pending_items_reach_the_size_limit() {
        // 手計算: 1 フレーム = 8 item × 4 B = 32 B の items。閾値 64 B なので 2 フレーム目で到達。
        let mut c = RunDecoder::new(1, [0].into_iter().collect(), 64, Duration::from_millis(10));
        let now = Instant::now();
        assert_eq!(
            c.handle_batch(0, 1, 0, &[buf(&frame_with_8_items(0, 1))], now),
            Emit::Nothing
        );
        assert_eq!(c.pending().len(), 1);
        assert_eq!(
            c.handle_batch(0, 1, 1, &[buf(&frame_with_8_items(0, 2))], now),
            Emit::Batch
        );
        assert_eq!(c.pending().len(), 2);
    }

    #[test]
    fn a_batch_closes_when_the_time_limit_elapses() {
        let mut c = RunDecoder::new(
            1,
            [0].into_iter().collect(),
            8 * 1024 * 1024,
            Duration::from_millis(10),
        );
        let t0 = Instant::now();
        assert_eq!(
            c.handle_batch(0, 1, 0, &[buf(&frame_with_8_items(0, 1))], t0),
            Emit::Nothing
        );
        assert_eq!(
            c.tick(t0 + Duration::from_millis(9)),
            Emit::Nothing,
            "期限前は閉じない"
        );
        assert_eq!(
            c.tick(t0 + Duration::from_millis(10)),
            Emit::Batch,
            "10 ms 経過で閉じる"
        );
    }

    #[test]
    fn an_empty_buffer_never_closes_on_a_tick() {
        let mut c = core(1, &[0]);
        let t0 = Instant::now();
        assert_eq!(c.tick(t0 + Duration::from_secs(1)), Emit::Nothing);
    }

    // -----------------------------------------------------------------
    // 入力の検証(seq 連続性・run 変化)
    // -----------------------------------------------------------------

    #[test]
    fn a_sequence_gap_is_counted_latches_error_and_keeps_consuming() {
        let mut c = core(1, &[0]);
        let now = Instant::now();
        c.handle_batch(0, 1, 0, &[buf(&frame_with_8_items(0, 1))], now);
        // seq=1 を飛ばして 2 が来る(ロスレスリンクのプロトコル違反)
        c.handle_batch(0, 1, 2, &[buf(&frame_with_8_items(0, 2))], now);

        assert_eq!(c.seq_gaps(), 1);
        assert!(c.errored(), "ギャップは Error ラッチ(SPEC §1.4-3)");
        assert_eq!(c.pending().len(), 2, "ラッチ後も消費・送出は継続する");
    }

    #[test]
    fn sequence_numbers_are_verified_per_source() {
        let mut c = core(1, &[0, 1]);
        let now = Instant::now();
        // 2 ソースが交互に 0,0,1,1 と来るのは各ソースでは連続 = 違反ではない。
        c.handle_batch(0, 1, 0, &[buf(&frame_with_8_items(0, 1))], now);
        c.handle_batch(1, 1, 0, &[buf(&frame_with_8_items(1, 1))], now);
        c.handle_batch(0, 1, 1, &[buf(&frame_with_8_items(0, 2))], now);
        c.handle_batch(1, 1, 1, &[buf(&frame_with_8_items(1, 2))], now);
        assert_eq!(c.seq_gaps(), 0);
        assert!(!c.errored());
    }

    #[test]
    fn a_run_number_change_before_eos_is_counted_and_latches_error() {
        let mut c = core(1, &[0]);
        let now = Instant::now();
        c.handle_batch(0, 1, 0, &[buf(&frame_with_8_items(0, 1))], now);
        c.handle_batch(0, 2, 1, &[buf(&frame_with_8_items(0, 2))], now);

        assert_eq!(c.run_mismatches(), 1);
        assert!(c.errored());
        assert_eq!(c.run_number(), 1, "EOS 前の run 変更は採用しない");
        assert_eq!(c.pending().len(), 2, "ラッチ後も消費・送出は継続する");
    }

    // -----------------------------------------------------------------
    // unsupported / malformed(SPEC v1.2 §7 と同じ区別)
    // -----------------------------------------------------------------

    #[test]
    fn unsupported_frames_are_counted_without_latching_error() {
        let mut c = core(1, &[0]);
        let now = Instant::now();
        c.handle_batch(
            0,
            1,
            0,
            &[buf(&control_frame()), buf(&frame_with_8_items(0, 1))],
            now,
        );

        assert_eq!(
            c.unsupported(),
            1,
            "frameType 7 は unsupported として数える"
        );
        assert_eq!(c.malformed(), 0);
        assert!(
            !c.errored(),
            "unsupported は Error にしない(graw-writer が ctrl/ に保全済み — SPEC v1.2 §7)"
        );
        assert_eq!(
            c.pending().len(),
            1,
            "Fragment 化されるのは frameType 1/2 のみ"
        );
        assert_eq!(c.frames_in(), 2, "入力フレーム数は跳ばした分も数える");
    }

    // -----------------------------------------------------------------
    // topology frame(frameType 7、TODO/067-A — ELI-NP 地雷の除去)
    // -----------------------------------------------------------------

    /// 実機 topology frame の生バイト列(出典 = MemRead.cpp:362 の `sendTopology()`。
    /// 同じ 12 B を reference/exp_data/2026 の pedestal / pulser 両方の `_0000.graw`
    /// 先頭で実測、2026-08-18)。coboIdx=0 / asadMask=0x0F / 2pMode=0。
    const REAL_TOPOLOGY_FRAME: [u8; 12] = [
        0x40, 0x00, 0x00, 0x0c, 0x00, 0x00, 0x07, 0x00, 0x00, 0x0f, 0x00, 0x00,
    ];

    /// 本ユニットの核心: **run 先頭に topology frame が来ても後続イベントが 1 件も欠けない**。
    ///
    /// 手計算のオラクル: 入力 5 フレーム = topology 1 + データ 4(eventIdx 1..=4、各 8 item)。
    /// → frames_in=5 / pending=4 Fragment / items = 4 × 8 = 32 / topology_frames=1 /
    ///   unsupported=1(topology は内数)/ malformed=0 / Error ラッチ無し。
    #[test]
    fn a_topology_frame_at_the_head_of_a_run_costs_no_events() {
        let mut c = core(7, &[0]);
        let now = Instant::now();
        let frames = vec![
            buf(&REAL_TOPOLOGY_FRAME),
            buf(&frame_with_8_items(0, 1)),
            buf(&frame_with_8_items(0, 2)),
            buf(&frame_with_8_items(0, 3)),
            buf(&frame_with_8_items(0, 4)),
        ];
        c.handle_batch(0, 7, 0, &frames, now);

        assert_eq!(c.frames_in(), 5);
        assert_eq!(
            c.pending().len(),
            4,
            "topology の後ろの 4 イベントが 1 件も欠けない"
        );
        let event_idxs: Vec<u32> = c.pending().iter().map(|f| f.event_idx).collect();
        assert_eq!(event_idxs, vec![1, 2, 3, 4], "順序も保たれる");
        assert_eq!(c.topology_frames(), 1);
        assert_eq!(c.unsupported(), 1, "topology は unsupported の内数");
        assert_eq!(c.malformed(), 0);
        assert!(!c.errored(), "topology frame は正常系(Error にしない)");
    }

    /// topology frame は `metrics` に専用フィールドとして出る(silent にしない — CLAUDE.md)。
    #[test]
    fn topology_frames_are_visible_in_metrics() {
        let mut c = core(7, &[0]);
        c.handle_batch(0, 7, 0, &[buf(&REAL_TOPOLOGY_FRAME)], Instant::now());
        let metrics = c.metrics_json();
        assert_eq!(metrics["topology_frames"], serde_json::json!(1));
        assert_eq!(metrics["unsupported"], serde_json::json!(1));
    }

    /// topology 以外の未知 frameType は `topology_frames` を進めない(取り違えの防止)。
    #[test]
    fn other_control_frames_do_not_count_as_topology() {
        let mut c = core(1, &[0]);
        c.handle_batch(0, 1, 0, &[buf(&control_frame())], Instant::now());
        assert_eq!(
            c.unsupported(),
            1,
            "control_frame() は 12 B・frameType 7 だが payload を持たない合成"
        );
        // control_frame() は 12 B の frameType 7 なので topology として認識される。
        assert_eq!(c.topology_frames(), 1);

        // frameType 5 は topology ではない。
        let mut c5 = core(1, &[0]);
        let h_none = {
            let mut b = vec![0u8; 12];
            b[0] = 0x00;
            b[3] = 12;
            write_uint(&mut b, 5, 5, 2); // frameType 5
            b
        };
        c5.handle_batch(0, 1, 0, &[buf(&h_none)], Instant::now());
        assert_eq!(c5.unsupported(), 1);
        assert_eq!(c5.topology_frames(), 0);
    }

    #[test]
    fn malformed_frames_are_counted_and_latch_error() {
        let mut c = core(1, &[0]);
        let now = Instant::now();
        // frameType 1 だがヘッダが 88 B に満たない(構造破損)。
        let mut broken = frame_with_8_items(0, 1);
        broken.truncate(40);
        c.handle_batch(0, 1, 0, &[buf(&broken)], now);

        assert_eq!(c.malformed(), 1);
        assert_eq!(c.unsupported(), 0);
        assert!(c.errored(), "malformed は Error ラッチ(P1 オラクルは 0)");
        assert_eq!(c.pending().len(), 0);
    }

    // -----------------------------------------------------------------
    // 送出後のカウンタ(GetStatus の材料)
    // -----------------------------------------------------------------

    #[test]
    fn sending_a_batch_advances_the_output_sequence_and_counters() {
        let mut c = core(1, &[0]);
        let now = Instant::now();
        c.handle_batch(
            0,
            1,
            0,
            &[
                buf(&frame_with_8_items(0, 1)),
                buf(&frame_with_8_items(0, 2)),
            ],
            now,
        );
        assert_eq!(c.out_sequence_number(), 0, "run 開始は 0 から");
        c.batch_sent();

        assert_eq!(c.out_sequence_number(), 1);
        assert_eq!(c.batches_out(), 1);
        assert_eq!(c.fragments_out(), 2);
        // 手計算: 1 フレーム 8 item × 2 フレーム = 16 item
        assert_eq!(c.items_out(), 16);
        assert_eq!(c.pending().len(), 0, "送出後はバッファを空にして再利用する");
        assert_eq!(c.batches_in_for(0), 1);
    }

    /// Reset 中の破棄は「数えて可視化されていれば許される」(TODO/009 停止設計)。
    /// 破棄しても自前 seq は進める — 下流には**ギャップとして見える**べきで、
    /// 詰めてしまうと「落ちていない」と誤認される(receiver の flush と同じ理屈)。
    #[test]
    fn an_abandoned_batch_is_counted_and_leaves_a_visible_sequence_gap() {
        let mut c = core(1, &[0]);
        let now = Instant::now();
        c.handle_batch(0, 1, 0, &[buf(&frame_with_8_items(0, 1))], now);
        c.batch_abandoned();

        assert_eq!(c.batches_abandoned(), 1);
        assert_eq!(c.batches_out(), 0);
        assert_eq!(c.fragments_out(), 0);
        assert_eq!(c.out_sequence_number(), 1, "破棄もギャップとして見せる");
        assert_eq!(c.pending().len(), 0);
    }

    // -----------------------------------------------------------------
    // cobo_mismatch(TODO/013-4 — DataLinkSet / --cobo-id 誤設定の早期検出)
    // -----------------------------------------------------------------

    /// `Batch.source_id`(receiver の `--cobo-id`)と `Fragment.cobo`(GRAW ヘッダ実値)が
    /// 食い違うのは、実機側の DataLinkSet か receiver の起動引数の誤設定である。
    /// **Error にはしない**(データは正しく、下流は `Fragment.cobo` で識別するので run は続く)。
    /// 数えて可視化するだけ — 黙って通すと 2 CoBo 運用で気づけない。
    #[test]
    fn a_cobo_header_that_disagrees_with_the_batch_source_is_counted_but_is_not_an_error() {
        let mut c = core(1, &[0, 1]);
        let now = Instant::now();
        // source 0 のバッチに cobo=1 が 2 本・cobo=0 が 1 本 = 不一致は 2(非対称)。
        c.handle_batch(
            0,
            1,
            0,
            &[
                buf(&frame_with_8_items(1, 1)),
                buf(&frame_with_8_items(0, 2)),
                buf(&frame_with_8_items(1, 3)),
            ],
            now,
        );

        assert_eq!(c.cobo_mismatch(), 2, "フレーム単位で数える");
        assert!(!c.errored(), "誤設定は Error にしない(TODO/013-4)");
        assert_eq!(
            c.pending().len(),
            3,
            "不一致でも Fragment は 1 本も捨てない"
        );
        assert_eq!(c.metrics_json()["cobo_mismatch"].as_u64(), Some(2));
    }

    #[test]
    fn matching_cobo_headers_never_count_a_mismatch() {
        let mut c = core(1, &[0, 1]);
        let now = Instant::now();
        c.handle_batch(0, 1, 0, &[buf(&frame_with_8_items(0, 1))], now);
        c.handle_batch(1, 1, 0, &[buf(&frame_with_8_items(1, 1))], now);
        // Fragment にならないフレーム(制御フレーム)は数えようがないので数えない。
        c.handle_batch(0, 1, 1, &[buf(&control_frame())], now);

        assert_eq!(c.cobo_mismatch(), 0);
        assert_eq!(c.unsupported(), 1);
    }

    // -----------------------------------------------------------------
    // Heartbeat 送出カウンタ(TODO/023-5 = R-P2-12)
    // -----------------------------------------------------------------

    /// Batch/EOS が out/abandoned の対を持つのに Heartbeat だけ結果を捨てていた(`let _ =`)。
    /// 送出成功と打ち切りを別々に数え、どちらも metrics に出す。
    #[test]
    fn heartbeat_send_outcomes_are_counted_separately() {
        let mut c = core(1, &[0]);
        // 非対称: 成功 2 通・打ち切り 1 通(数え違いが目に見えるように)
        c.heartbeat_sent();
        c.heartbeat_sent();
        c.heartbeat_abandoned();

        assert_eq!(c.heartbeats_out(), 2);
        assert_eq!(c.heartbeats_abandoned(), 1);
        let m = c.metrics_json();
        assert_eq!(m["heartbeats_out"].as_u64(), Some(2));
        assert_eq!(m["heartbeats_abandoned"].as_u64(), Some(1));
        assert_eq!(m["heartbeats_in"].as_u64(), Some(0), "受信カウンタとは独立");
    }

    /// TODO/033-C: **最後のホップ**(decoder → root-sink)の観測点。`eos_in` が揃っても
    /// 自分の EOS を送れていなければ run のバリアは root-sink に届いていない ——
    /// controller の `eos_complete` はこの `eos_out` を 3 点目に読む。
    /// `eos_abandoned` と対であること(送れた / 打ち切った のどちらかが必ず立つ)も固定する。
    #[test]
    fn eos_out_counts_only_the_decoders_own_sent_eos() {
        let mut c = core(4, &[0, 1]);
        assert_eq!(c.eos_out(), 0, "起点は 0");

        // 上流 EOS を 2 本受けただけでは **まだ送っていない**(ここが 033-C の穴)。
        c.handle_eos(0, 4);
        assert_eq!(c.handle_eos(1, 4), Emit::FlushAndEndOfStream);
        assert_eq!(c.eos_in(), 2);
        assert_eq!(c.eos_out(), 0, "受けただけでは eos_out は進まない");
        assert_eq!(c.metrics_json()["eos_out"].as_u64(), Some(0));

        c.eos_sent();
        assert_eq!(c.eos_out(), 1);
        assert_eq!(c.metrics_json()["eos_out"].as_u64(), Some(1));
        assert_eq!(
            c.metrics_json()["eos_abandoned"].as_u64(),
            Some(0),
            "送れたのだから打ち切りは 0"
        );

        // 打ち切り経路は `eos_out` を進めない(対の関係)。
        c.eos_abandoned();
        assert_eq!(c.eos_out(), 1, "abandon で eos_out は進まない");
        assert_eq!(c.metrics_json()["eos_abandoned"].as_u64(), Some(1));
    }

    #[test]
    fn metrics_json_carries_every_counter_of_the_ticket() {
        let mut c = core(1, &[0]);
        let now = Instant::now();
        c.handle_batch(0, 1, 0, &[buf(&frame_with_8_items(0, 1))], now);
        c.handle_heartbeat(0);
        c.batch_sent();
        let m = c.metrics_json();

        for key in [
            "frames_in",
            "fragments_out",
            "batches_out",
            "items_out",
            "unsupported",
            "malformed",
            "seq_gaps",
            "run_mismatches",
            "eos_in",
            "heartbeats_in",
            "heartbeats_out",
            "heartbeats_abandoned",
            "eos_out",
            "eos_abandoned",
            "batches_abandoned",
            "cobo_mismatch",
        ] {
            assert!(m.get(key).is_some(), "metrics.{key} が無い: {m}");
        }
        assert_eq!(
            m["batches_in"]["0"].as_u64(),
            Some(1),
            "batches_in は cobo 毎"
        );
        assert_eq!(m["heartbeats_in"].as_u64(), Some(1));
        assert_eq!(m["items_out"].as_u64(), Some(8));
    }

    // -----------------------------------------------------------------
    // poisoned Mutex = スレッド死(TODO/023-1 = R-P2-8)
    // -----------------------------------------------------------------

    fn test_params() -> DecoderParams {
        DecoderParams {
            pull_bind: "tcp://127.0.0.1:0".to_string(),
            push_connect: "tcp://127.0.0.1:0".to_string(),
            command_listen: "tcp://127.0.0.1:0".to_string(),
            batch_max_bytes: 8 * 1024 * 1024,
            batch_max_ms: 10,
            heartbeat_ms: 1_000,
            send_timeout_ms: 100,
            workers: 1,
            expected_sources: vec![0],
        }
    }

    /// 「ワーカースレッドがロック保持中に panic して死んだ」状況をそのまま作る。
    /// (テスト出力に panic のメッセージが 1 行出るのは想定どおり — 握り潰さない。)
    fn poison<T: Send + 'static>(m: &Arc<Mutex<T>>) {
        let m = Arc::clone(m);
        let joined = std::thread::spawn(move || {
            let _guard = m.lock().expect("fresh mutex is not poisoned");
            panic!("simulated worker panic while holding the lock");
        })
        .join();
        assert!(joined.is_err(), "スレッドは panic しているはず");
    }

    #[test]
    fn a_poisoned_core_lock_enters_error_instead_of_looking_healthy() {
        let mut handler = Handler::new(test_params());
        let core = Arc::new(Mutex::new(core(1, &[0])));
        handler.core = Some(Arc::clone(&core));
        handler.state = ComponentState::Running;

        poison(&core);
        assert!(core.lock().is_err(), "前提: Mutex は poisoned");

        handler.latch_error();
        assert_eq!(
            handler.state,
            ComponentState::Error,
            "スレッド死を「エラーなし」に丸めない(R-P2-8)"
        );
    }

    /// 対照: 健全なコアで違反が無ければ Running のまま(上のテストが常に Error を
    /// 返すだけの偽陽性でないこと)。
    #[test]
    fn a_healthy_core_without_violations_stays_running() {
        let mut handler = Handler::new(test_params());
        let core = Arc::new(Mutex::new(core(1, &[0])));
        handler.core = Some(Arc::clone(&core));
        handler.state = ComponentState::Running;

        handler.latch_error();
        assert_eq!(handler.state, ComponentState::Running);
    }
}
