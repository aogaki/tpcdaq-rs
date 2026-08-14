//! receiver — CoBo 毎の TCP 受信コンポーネント(drain のみ、never-stop)。
//! SPEC §1.1(責務)/ §1.3(状態機械)/ §1.4(過負荷規約)/ §2.2・§2.3(Batch/EOS/バッチ詰め)。
//!
//! # タスク分離(delila-rs `docs/component_architecture.md` の流儀)
//!
//! ```text
//!            TCP(CoBo)                有界 sync_channel            PUSH ×2
//!   CoBo ──────────────▶ [drain タスク] ─────────────────▶ [送信タスク] ──▶ graw-writer
//!                         tokio / 非同期    queue_frames 段    専用 OS スレッド ──▶ decoder
//!                              │                                     ▲
//!                              │ overflow は数えるだけ(破棄)          │ ここが詰まっても
//!                              ▼                                     │ drain には波及しない
//!                        Arc<Metrics>(lock-free)              [コマンド REP タスク]
//! ```
//!
//! - **drain タスクは絶対に止めない**。内部キューが満杯なら当該フレームを破棄して
//!   [`Metrics::overflow_frames`] を進め、コンポーネントを Error として報告する
//!   (SPEC §1.4-3)。ソケット読みをブロックさせない = 下流の遅延を CoBo へ逆圧しない。
//! - 送信タスクは **専用 OS スレッド**に置く。ZMQ の送信は HWM 到達でブロックする
//!   (= ロスレスの背圧そのもの)ので、tokio のワーカを塞がない場所で待たせる。
//! - `EndOfStream` は間引き規則から除外(SPEC §2.2)。キューが満杯でも捨てず、空くまで待つ。
//!
//! # 状態機械(SPEC §1.3)
//!
//! `Configure`(run 番号確定)→ `Arm`(**bind + listen** = listen-before-start。実 bind
//! アドレスを `CommandResponse.metrics` で返す — controller が DataLinkSet に使う、SPEC §8.2)
//! → `Start{run}`(accept 開始・seq 0 リセット)→ `Stop` / `Reset`。

use std::net::{SocketAddr, TcpListener as StdTcpListener};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_bytes::ByteBuf;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, oneshot};
use tracing::{debug, error, info, warn};

use crate::command::{run_command_task, Command, CommandResponse, ComponentState, RunConfig};
use crate::config::{Config, DEFAULT_RECEIVER_SEND_TIMEOUT_MS, RECEIVER_COMMAND_PORT_BASE};
use crate::framer::Framer;
use crate::msg::{Batch, Message, RawFrames};
use crate::zmq_helper;

/// TCP から 1 回に読むバイト数。実 .graw の 1 フレーム(278,784 B)に対して数回の read で
/// 済む大きさ。読みバッファは接続毎に 1 回だけ確保する(per-frame 確保をしない)。
const READ_CHUNK_BYTES: usize = 256 * 1024;

/// EOS がキュー満杯で入らないときの再試行間隔。
const CONTROL_RETRY_MS: u64 = 1;

/// accept が失敗したときのバックオフ(エラーで busy loop にしない)。
const ACCEPT_BACKOFF_MS: u64 = 100;

/// 送信タスクが有界キューを待つ上限(= 打ち切り合図に気づくまでの粒度)。
/// decoder スレッドの `POLL_TIMEOUT_MS` と同じ 100 ms 級。
const SENDER_POLL_INTERVAL: Duration = Duration::from_millis(100);

// ---------------------------------------------------------------------
// 起動パラメタ
// ---------------------------------------------------------------------

/// receiver 1 プロセス分の起動パラメタ(= 設定 + CoBo 番号の解決結果)。
///
/// 設定ファイルからは [`ReceiverParams::from_config`] で作る。テストは
/// すべてのアドレスにポート 0 を入れて動的ポートを使う。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiverParams {
    /// この receiver が担当する CoBo(Batch の `source_id` でもある、SPEC §3.2)。
    pub cobo_id: u32,
    /// CoBo からの TCP 着信アドレス(例 `0.0.0.0:46005`)。
    pub listen: String,
    /// コマンド REP の bind アドレス(例 `tcp://*:47110`)。
    pub command_listen: String,
    /// graw-writer の PULL bind への接続先。
    pub graw_writer_endpoint: String,
    /// decoder の PULL bind への接続先。
    pub decoder_endpoint: String,
    /// バッチを閉じるバイト数(SPEC §2.3)。
    pub batch_max_bytes: usize,
    /// バッチを閉じる経過時間(ミリ秒、SPEC §2.3)。
    pub batch_max_ms: u64,
    /// drain → 送信タスクの有界キュー段数(SPEC §1.4-1/2)。
    pub queue_frames: usize,
    /// アイドル時 Heartbeat の周期(ミリ秒、SPEC §2.2)。
    pub heartbeat_ms: u64,
    /// PUSH の HWM(SPEC §1.4-2)。
    pub hwm: i32,
}

impl ReceiverParams {
    /// 設定と CoBo 番号から起動パラメタを解決する。
    ///
    /// コマンド REP アドレスは SPEC §3.2 の規約 `47110 + cobo_id` で決まる。
    /// `cobo_id` が設定に無ければ `None`(呼び手が明示的に失敗させる)。
    pub fn from_config(config: &Config, cobo_id: u32) -> Option<Self> {
        let cobo = config.cobo.iter().find(|c| c.id == cobo_id)?;
        let receiver = &config.receiver;
        Some(Self {
            cobo_id,
            listen: cobo.listen.clone(),
            command_listen: format!("tcp://*:{}", RECEIVER_COMMAND_PORT_BASE + cobo_id),
            graw_writer_endpoint: receiver.graw_writer_endpoint.clone(),
            decoder_endpoint: receiver.decoder_endpoint.clone(),
            batch_max_bytes: receiver.batch_max_bytes,
            batch_max_ms: receiver.batch_max_ms,
            queue_frames: receiver.queue_frames,
            heartbeat_ms: receiver.heartbeat_ms,
            hwm: receiver.hwm,
        })
    }
}

// ---------------------------------------------------------------------
// カウンタ
// ---------------------------------------------------------------------

/// run 毎のカウンタ(`Start` でリセット、`CommandResponse.metrics` で返す)。
///
/// ホットパスから触るので lock-free(delila-rs の統計方針)。順序は `Relaxed` で十分
/// ——値の因果関係ではなく「増えたこと」だけを見るため。
#[derive(Debug, Default)]
struct Metrics {
    bytes: AtomicU64,
    frames: AtomicU64,
    batches: AtomicU64,
    overflow_frames: AtomicU64,
    framer_resets: AtomicU64,
    /// 送出を打ち切ったメッセージ数(TODO/023-2 = R-P2-3。SPEC §1.3 v1.6「abandon は
    /// 可視カウント」の receiver 側実装 — 打ち切りは `Reset` / 畳み込みでのみ起きる)。
    messages_abandoned: AtomicU64,
    /// MessagePack へ符号化できずに送れなかったメッセージ数(以前は error ログのみ・無カウント)。
    encode_errors: AtomicU64,
    /// ETERM 以外の ZMQ send 失敗数(以前は error ログのみ・無カウント)。
    send_errors: AtomicU64,
    /// 内部キュー満杯を一度でも踏んだか(= Error 報告のラッチ、SPEC §1.4-3)。
    overflowed: AtomicBool,
    /// 打ち切りの warn を出したか(初回だけ出す — 以降はカウンタが担う)。
    logged_abandoned: AtomicBool,
    /// framer リセットの warn を出したか(同上、TODO/023-4 = R-P2-10)。
    logged_framer_reset: AtomicBool,
}

impl Metrics {
    fn reset(&self) {
        self.bytes.store(0, Ordering::Relaxed);
        self.frames.store(0, Ordering::Relaxed);
        self.batches.store(0, Ordering::Relaxed);
        self.overflow_frames.store(0, Ordering::Relaxed);
        self.framer_resets.store(0, Ordering::Relaxed);
        self.messages_abandoned.store(0, Ordering::Relaxed);
        self.encode_errors.store(0, Ordering::Relaxed);
        self.send_errors.store(0, Ordering::Relaxed);
        self.overflowed.store(false, Ordering::Relaxed);
        self.logged_abandoned.store(false, Ordering::Relaxed);
        self.logged_framer_reset.store(false, Ordering::Relaxed);
    }

    /// フレーム 1 個を落としたことを記録する。初回だけログに出す
    /// (ホットパスでログ整形を繰り返さない — CLAUDE.md)。
    fn record_dropped_frame(&self, cobo_id: u32, reason: &str) {
        self.overflow_frames.fetch_add(1, Ordering::Relaxed);
        if !self.overflowed.swap(true, Ordering::Relaxed) {
            warn!(
                cobo_id,
                reason,
                "internal queue overflow — dropping frames and reporting Error (SPEC §1.4-3)"
            );
        }
    }

    /// 送出打ち切りを記録する。**初回だけ** warn(以降はカウンタが担う)。
    /// 戻り値 = このとき warn を出したか(「一度だけ」を機械照合するため)。
    fn record_abandoned(&self, cobo_id: u32, link: &str, kind: &str) -> bool {
        self.messages_abandoned.fetch_add(1, Ordering::Relaxed);
        if self.logged_abandoned.swap(true, Ordering::Relaxed) {
            return false;
        }
        warn!(
            cobo_id,
            link,
            kind,
            "send abandoned while stopping (Reset / component teardown) — this message is lost \
             and counted, never silently dropped (SPEC §1.3 v1.6); further occurrences are \
             counted only"
        );
        true
    }

    /// 符号化失敗を記録する(送れなかった = 喪失なので必ず数える)。
    fn record_encode_error(&self, cobo_id: u32, link: &str, error: &crate::msg::MsgError) {
        self.encode_errors.fetch_add(1, Ordering::Relaxed);
        error!(cobo_id, link, %error, "cannot encode message — this message is lost and counted");
    }

    /// ETERM 以外の send 失敗を記録する(同上)。
    fn record_send_error(&self, cobo_id: u32, link: &str, error: zmq::Error) {
        self.send_errors.fetch_add(1, Ordering::Relaxed);
        error!(cobo_id, link, %error, "ZMQ send failed — this message is lost and counted");
    }

    /// framer リセット(MFM ヘッダ崩れ)の**増分**を記録する(TODO/023-4 = R-P2-10)。
    /// GetStatus をポーリングしていなくても気づけるよう**初回だけ** warn する。
    /// 戻り値 = このとき warn を出したか。
    fn note_framer_resets(&self, cobo_id: u32, resets: u64) -> bool {
        self.framer_resets.store(resets, Ordering::Relaxed);
        if self.logged_framer_reset.swap(true, Ordering::Relaxed) {
            return false;
        }
        warn!(
            cobo_id,
            framer_resets = resets,
            "MFM framing lost — the framer resynchronised on the next valid header; the CoBo link \
             may be corrupting frames. Further occurrences are counted only (metrics.framer_resets)"
        );
        true
    }

    fn json(&self) -> serde_json::Value {
        let get = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        serde_json::json!({
            "bytes": get(&self.bytes),
            "frames": get(&self.frames),
            "batches": get(&self.batches),
            "overflow_frames": get(&self.overflow_frames),
            "framer_resets": get(&self.framer_resets),
            "messages_abandoned": get(&self.messages_abandoned),
            "encode_errors": get(&self.encode_errors),
            "send_errors": get(&self.send_errors),
        })
    }
}

// ---------------------------------------------------------------------
// drain タスク(TCP → framer → 有界キュー)
// ---------------------------------------------------------------------

/// drain タスクから送信タスクへ渡すもの。
enum FrameMsg {
    /// MFM フレーム 1 個(バイト列そのまま)。
    Frame(Vec<u8>),
    /// ストリーム終端(TCP EOF もしくは Stop)。**落としてはならない**(SPEC §2.2)。
    EndOfStream,
}

/// ソケットを読み続けてフレームを切り出し、有界キューへ流す。
///
/// ここが「never-stop」の実体。キューが満杯でも `try_send` の失敗を数えて捨てるだけで、
/// 読みを止めない(= CoBo へ逆圧しない)。
async fn drain_task(
    listener: TcpListener,
    tx: SyncSender<FrameMsg>,
    metrics: Arc<Metrics>,
    cobo_id: u32,
    mut stop: broadcast::Receiver<()>,
) {
    let mut buf = vec![0u8; READ_CHUNK_BYTES];
    // run 開始時点で「この run はまだ EOS を出していない」。EOF で出したら降ろす。
    let mut owes_eos = true;

    'accept: loop {
        let accepted = tokio::select! {
            biased;
            _ = stop.recv() => break 'accept,
            result = listener.accept() => result,
        };

        let mut stream = match accepted {
            Ok((stream, peer)) => {
                info!(cobo_id, %peer, "CoBo connected");
                owes_eos = true;
                stream
            }
            Err(e) => {
                error!(cobo_id, error = %e, "accept failed — retrying");
                tokio::time::sleep(Duration::from_millis(ACCEPT_BACKOFF_MS)).await;
                continue 'accept;
            }
        };

        let mut framer = Framer::new();
        // この接続で観測済みの framer リセット数(増分検知用 — TODO/023-4 = R-P2-10)。
        let mut seen_resets = 0u64;
        loop {
            let read = tokio::select! {
                biased;
                _ = stop.recv() => break 'accept,
                result = stream.read(&mut buf) => result,
            };

            match read {
                // TCP EOF = run 境界(SPEC §1.3 の run 停止シーケンス)
                Ok(0) => {
                    info!(cobo_id, "TCP EOF — end of stream");
                    send_end_of_stream(&tx, cobo_id).await;
                    owes_eos = false;
                    break;
                }
                Ok(n) => {
                    metrics.bytes.fetch_add(n as u64, Ordering::Relaxed);
                    framer.push(&buf[..n]);
                    while let Some(frame) = framer.next() {
                        let owned = frame.to_vec();
                        metrics.frames.fetch_add(1, Ordering::Relaxed);
                        match tx.try_send(FrameMsg::Frame(owned)) {
                            Ok(()) => {}
                            Err(TrySendError::Full(_)) => {
                                metrics.record_dropped_frame(cobo_id, "queue full")
                            }
                            Err(TrySendError::Disconnected(_)) => {
                                metrics.record_dropped_frame(cobo_id, "sender task gone")
                            }
                        }
                    }
                    // MFM ヘッダ崩れは**増分を検知した瞬間に**一度だけ warn する
                    // (TODO/023-4 = R-P2-10。以前はカウンタ転写だけで、GetStatus を
                    // ポーリングしない限りフレーミング崩れの継続に気づけなかった)。
                    let resets = framer.reset_count();
                    if resets != seen_resets {
                        seen_resets = resets;
                        metrics.note_framer_resets(cobo_id, resets);
                    }
                }
                Err(e) => {
                    warn!(cobo_id, error = %e, "read failed — treating as end of stream");
                    send_end_of_stream(&tx, cobo_id).await;
                    owes_eos = false;
                    break;
                }
            }
        }
    }

    // Stop / shutdown で抜けたときの取りこぼし防止(SPEC §1.3: 強制 EOS)。
    if owes_eos {
        send_end_of_stream(&tx, cobo_id).await;
    }
    info!(cobo_id, "drain task stopped");
}

/// EOS をキューへ入れる。**満杯でも捨てず**空くまで待つ(SPEC §2.2)。
/// 待っている間もソケット読みは止まらない ——この関数を呼ぶのは EOF/Stop の直後だけ。
async fn send_end_of_stream(tx: &SyncSender<FrameMsg>, cobo_id: u32) {
    let mut pending = FrameMsg::EndOfStream;
    let mut warned = false;
    loop {
        match tx.try_send(pending) {
            Ok(()) => return,
            Err(TrySendError::Full(back)) => {
                if !warned {
                    warn!(
                        cobo_id,
                        "queue full — EndOfStream is waiting (never dropped)"
                    );
                    warned = true;
                }
                pending = back;
                tokio::time::sleep(Duration::from_millis(CONTROL_RETRY_MS)).await;
            }
            Err(TrySendError::Disconnected(_)) => {
                error!(
                    cobo_id,
                    "sender task is gone — EndOfStream could not be sent"
                );
                return;
            }
        }
    }
}

// ---------------------------------------------------------------------
// 送信タスク(バッチ詰め + PUSH ×2)
// ---------------------------------------------------------------------

/// 1 本の下流リンク。**ソケットも sequence_number も独立**(SPEC §2.3)。
struct Link {
    name: &'static str,
    socket: zmq::Socket,
    sequence_number: u64,
}

/// 送信タスクの run 毎パラメタ。
struct SenderConfig {
    /// Batch の `source_id` = 担当 CoBo の番号そのもの(SPEC §3.2)。ログ・カウンタの
    /// `cobo_id` にもこれをそのまま使う(同じ値を 2 つ持たない)。
    source_id: u32,
    run_number: u32,
    batch_max_bytes: usize,
    batch_max: Duration,
    heartbeat: Duration,
}

/// 1 メッセージ送出の結末(decoder の [`crate::decoder`] と同じ切り方)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SendOutcome {
    Sent,
    /// 符号化失敗 / 一時的でない send 失敗。**この 1 通は失われた**(カウント + error 済み)が、
    /// リンクとしては生きているので送出は続ける。
    Failed,
    /// 打ち切り(`Reset` / 畳み込み)または ZMQ コンテキスト終了。以後このリンクへは送らない。
    Stopped,
}

/// 有界キューからフレームを取り、「`batch_max_bytes` 到達 or `batch_max` 経過」の早い方で
/// バッチを閉じて両リンクへ送る(SPEC §2.3)。アイドル時は Heartbeat を出す(SPEC §2.2)。
///
/// 専用 OS スレッドで走る前提。ZMQ 送信のブロック(= 背圧)はここで受け止める。
///
/// # 停止設計(TODO/023-2 = R-P2-3。decoder の `send_lossless` と同じ形)
///
/// 送出はロスレス(下流の背圧で待つ)。`abandon` が立つのは **`Reset` とコンポーネント
/// 畳み込み**のときだけで、そこで初めて送出待ちを打ち切り `messages_abandoned` として
/// 可視化する。`Stop` では立てない —— receiver の `Stop` は「強制 EOS を流す」経路
/// (SPEC §1.3 v1.6)であり、ここで打ち切ると下流が run を閉じられなくなる。
///
/// 待ち時間は [`SENDER_POLL_INTERVAL`] で必ず頭打ちにする: `heartbeat_ms` をいくら長く
/// しても打ち切り合図に気づくまでが鈍らない = スレッドが残らない。
fn sender_loop(
    rx: Receiver<FrameMsg>,
    mut links: Vec<Link>,
    cfg: SenderConfig,
    metrics: Arc<Metrics>,
    abandon: Arc<AtomicBool>,
) {
    // バッチ用のバッファは使い回す(per-batch の再確保をしない)。
    let mut batch: Vec<ByteBuf> = Vec::new();
    let mut batch_bytes = 0usize;
    let mut batch_deadline: Option<Instant> = None;
    let mut heartbeat_counter = 0u64;
    let mut last_send = Instant::now();

    loop {
        if abandon.load(Ordering::Relaxed) {
            // Reset / 畳み込み: 送出待ちで居座らない(打ち切りは既にカウント済み)。
            break;
        }

        let now = Instant::now();
        let wake = batch_deadline.unwrap_or(last_send + cfg.heartbeat);
        let wait = wake
            .saturating_duration_since(now)
            .min(SENDER_POLL_INTERVAL);
        let alive = match rx.recv_timeout(wait) {
            Ok(FrameMsg::Frame(frame)) => {
                batch_bytes += frame.len();
                batch.push(ByteBuf::from(frame));
                if batch_deadline.is_none() {
                    batch_deadline = Some(now + cfg.batch_max);
                }
                if batch_bytes >= cfg.batch_max_bytes {
                    let alive = flush(
                        &mut links,
                        &mut batch,
                        &mut batch_bytes,
                        &cfg,
                        &metrics,
                        &abandon,
                    );
                    batch_deadline = None;
                    last_send = Instant::now();
                    alive
                } else {
                    true
                }
            }
            Ok(FrameMsg::EndOfStream) => {
                let mut alive = flush(
                    &mut links,
                    &mut batch,
                    &mut batch_bytes,
                    &cfg,
                    &metrics,
                    &abandon,
                );
                batch_deadline = None;
                let eos = Message::<RawFrames>::EndOfStream {
                    source_id: cfg.source_id,
                    run_number: cfg.run_number,
                };
                for link in &mut links {
                    alive &= send_on(link, &eos, "EndOfStream", &cfg, &metrics, &abandon)
                        != SendOutcome::Stopped;
                }
                info!(
                    source_id = cfg.source_id,
                    run_number = cfg.run_number,
                    "EndOfStream sent to both links"
                );
                last_send = Instant::now();
                alive
            }
            // 待ちを頭打ちにしたので、タイムアウト = 期限到達とは限らない。期限は自分で見る。
            Err(RecvTimeoutError::Timeout) => {
                let now = Instant::now();
                if batch_deadline.is_some_and(|deadline| now >= deadline) {
                    let alive = flush(
                        &mut links,
                        &mut batch,
                        &mut batch_bytes,
                        &cfg,
                        &metrics,
                        &abandon,
                    );
                    batch_deadline = None;
                    last_send = Instant::now();
                    alive
                } else if batch.is_empty() && now >= last_send + cfg.heartbeat {
                    let heartbeat = Message::<RawFrames>::Heartbeat {
                        source_id: cfg.source_id,
                        run_number: cfg.run_number,
                        counter: heartbeat_counter,
                    };
                    let mut alive = true;
                    for link in &mut links {
                        alive &= send_on(link, &heartbeat, "Heartbeat", &cfg, &metrics, &abandon)
                            != SendOutcome::Stopped;
                    }
                    heartbeat_counter += 1;
                    last_send = Instant::now();
                    alive
                } else {
                    true
                }
            }
            // drain タスクが終わった = run 終了。残りを吐いて畳む。
            Err(RecvTimeoutError::Disconnected) => {
                flush(
                    &mut links,
                    &mut batch,
                    &mut batch_bytes,
                    &cfg,
                    &metrics,
                    &abandon,
                );
                false
            }
        };

        if !alive {
            break;
        }
    }

    info!(
        source_id = cfg.source_id,
        run_number = cfg.run_number,
        messages_abandoned = metrics.messages_abandoned.load(Ordering::Relaxed),
        "sender task stopped"
    );
}

/// 溜まったフレームを 1 バッチとして両リンクへ送る。
///
/// リンク毎に sequence_number が違うので符号化もリンク毎(ペイロードは借用で共有し、
/// バッチのコピーは作らない)。
fn flush(
    links: &mut [Link],
    batch: &mut Vec<ByteBuf>,
    batch_bytes: &mut usize,
    cfg: &SenderConfig,
    metrics: &Metrics,
    abandon: &AtomicBool,
) -> bool {
    if batch.is_empty() {
        return true;
    }
    let created_ns = unix_nanos();
    let mut alive = true;
    let mut delivered = false;
    for link in links.iter_mut() {
        let message = Message::Data(Batch {
            source_id: cfg.source_id,
            run_number: cfg.run_number,
            sequence_number: link.sequence_number,
            created_ns,
            payload: &*batch,
        });
        // 送信の成否によらず番号は進める: 失敗はギャップとして下流に見えるべき
        // (詰めると「落ちていない」と誤認される — silent failure を作らない)。
        link.sequence_number += 1;
        let outcome = send_on(link, &message, "Data", cfg, metrics, abandon);
        delivered |= outcome == SendOutcome::Sent;
        alive &= outcome != SendOutcome::Stopped;
    }
    // `batches` は「プロセスから出て行ったバッチ」だけを数える。打ち切り/失敗したものを
    // ここに混ぜると「送れたこと」に化ける(下流不在で batches が伸びない、が契約 —
    // SPEC §1.2 v1.4。喪失は messages_abandoned / send_errors 側に出る)。
    if delivered {
        metrics.batches.fetch_add(1, Ordering::Relaxed);
    }
    debug!(frames = batch.len(), bytes = *batch_bytes, "batch flushed");
    batch.clear();
    *batch_bytes = 0;
    alive
}

/// 1 リンクへ 1 メッセージ送る(ロスレス。EAGAIN では諦めずに待つ)。
///
/// **捨てない・ただし中断可能**(TODO/023-2 = R-P2-3): SNDTIMEO
/// ([`crate::config::DEFAULT_RECEIVER_SEND_TIMEOUT_MS`])で目を覚ましては `abandon` を見る。
/// 立っていなければ再試行し続ける(下流の背圧で待つ = ロスレス)。立っていれば
/// `messages_abandoned` として数えて打ち切る —— 破棄が可視化されていることが、
/// 打ち切りを許す唯一の条件(SPEC §1.3 v1.6 / §1.4)。
///
/// 失敗はすべて数える: 符号化失敗 = `encode_errors`、ETERM 以外の send 失敗 = `send_errors`。
/// ETERM だけはコンテキスト終了(正常な畳み込み)なのでカウントせず `Stopped` を返す。
fn send_on<P: Serialize>(
    link: &mut Link,
    message: &Message<P>,
    kind: &'static str,
    cfg: &SenderConfig,
    metrics: &Metrics,
    abandon: &AtomicBool,
) -> SendOutcome {
    let bytes = match message.to_msgpack() {
        Ok(bytes) => bytes,
        Err(e) => {
            metrics.record_encode_error(cfg.source_id, link.name, &e);
            return SendOutcome::Failed;
        }
    };
    let mut waited = false;
    loop {
        // 打ち切り合図は **送る前に**見る: 1 通あたり最大でも SNDTIMEO 1 回分しか
        // 待たない(2 リンク分が直列に積み上がらない)。
        if abandon.load(Ordering::Relaxed) {
            metrics.record_abandoned(cfg.source_id, link.name, kind);
            return SendOutcome::Stopped;
        }
        match link.socket.send(&bytes[..], 0) {
            Ok(()) => {
                if waited {
                    info!(
                        cobo_id = cfg.source_id,
                        link = link.name,
                        "downstream backpressure released — message sent"
                    );
                }
                return SendOutcome::Sent;
            }
            Err(zmq::Error::EAGAIN) => {
                if !waited {
                    waited = true;
                    info!(
                        cobo_id = cfg.source_id,
                        link = link.name,
                        kind,
                        "downstream is full or absent — blocking on backpressure (lossless; only \
                         Reset / teardown may abandon it)"
                    );
                }
            }
            Err(zmq::Error::ETERM) => {
                info!(
                    cobo_id = cfg.source_id,
                    link = link.name,
                    "ZMQ context terminated — sender stopping"
                );
                return SendOutcome::Stopped;
            }
            Err(e) => {
                metrics.record_send_error(cfg.source_id, link.name, e);
                return SendOutcome::Failed;
            }
        }
    }
}

fn unix_nanos() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => u64::try_from(d.as_nanos()).unwrap_or(u64::MAX),
        Err(_) => 0,
    }
}

// ---------------------------------------------------------------------
// コマンドハンドラ(状態機械)
// ---------------------------------------------------------------------

/// コマンド 1 件を処理する状態持ち。`run_command_task` の `FnMut` として使う。
struct Handler {
    params: ReceiverParams,
    context: zmq::Context,
    metrics: Arc<Metrics>,
    state: ComponentState,
    run_number: Option<u32>,
    /// Arm で確保した listener(Start で drain タスクへ渡す)。
    listener: Option<StdTcpListener>,
    bind_address: Option<SocketAddr>,
    /// 走行中の run を畳む合図(drain タスク宛)。
    run_stop: Option<broadcast::Sender<()>>,
    /// 送出待ちの打ち切り合図(送信スレッド宛)。`Reset` と畳み込みでのみ立てる。
    abandon: Option<Arc<AtomicBool>>,
    /// 送信スレッド。**join できることが「スレッドを残さない」の担保**(TODO/023-2)。
    sender_handle: Option<std::thread::JoinHandle<()>>,
}

impl Handler {
    fn new(params: ReceiverParams) -> Self {
        Self {
            params,
            context: zmq::Context::new(),
            metrics: Arc::new(Metrics::default()),
            state: ComponentState::Idle,
            run_number: None,
            listener: None,
            bind_address: None,
            run_stop: None,
            abandon: None,
            sender_handle: None,
        }
    }

    fn handle(&mut self, cmd: Command) -> CommandResponse {
        self.latch_overload();

        let Some(target) = cmd.target_state() else {
            return self.respond(true, "status"); // GetStatus は遷移しない
        };
        if !self.state.can_transition_to(target) {
            let message = format!("illegal transition {} -> {target}", self.state);
            warn!(cobo_id = self.params.cobo_id, %cmd, "{message}");
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
                info!(cobo_id = self.params.cobo_id, state = %self.state, %cmd, "command applied");
                self.respond(true, format!("{cmd} ok"))
            }
            Err(message) => {
                // 状態は変えない(= 失敗しても現在の状態を正直に返す)
                error!(cobo_id = self.params.cobo_id, %cmd, "{message}");
                self.respond(false, message)
            }
        }
    }

    /// 内部キューが溢れていたら Error として報告する(SPEC §1.4-3)。
    /// 溢れの検出自体は drain タスク(ホットパス)側で、状態への反映はここで行う。
    fn latch_overload(&mut self) {
        if self.state == ComponentState::Error
            || !self.metrics.overflowed.load(Ordering::Relaxed)
            || !self.state.can_transition_to(ComponentState::Error)
        {
            return;
        }
        warn!(
            cobo_id = self.params.cobo_id,
            overflow_frames = self.metrics.overflow_frames.load(Ordering::Relaxed),
            "entering Error: receiver internal queue overflowed (SPEC §1.4-3)"
        );
        self.state = ComponentState::Error;
    }

    fn do_configure(&mut self, cfg: &RunConfig) -> Result<(), String> {
        // 設定値そのものは起動時の TOML が正本。ここでは run 番号だけを確定させる。
        self.run_number = Some(cfg.run_number);
        Ok(())
    }

    /// listen-before-start(SPEC §1.3): ここで bind + listen する。
    fn do_arm(&mut self) -> Result<(), String> {
        let listener = StdTcpListener::bind(&self.params.listen)
            .map_err(|e| format!("bind {} failed: {e}", self.params.listen))?;
        let address = listener
            .local_addr()
            .map_err(|e| format!("cannot read local address: {e}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("cannot set non-blocking: {e}"))?;
        self.listener = Some(listener);
        self.bind_address = Some(address);
        info!(
            cobo_id = self.params.cobo_id,
            %address,
            "armed — listening before ecc start (SPEC §1.3)"
        );
        Ok(())
    }

    fn do_start(&mut self, run_number: u32) -> Result<(), String> {
        if self.listener.is_none() {
            return Err("no listener: Arm again before Start".to_string());
        }
        // 先に下流を張る: ここで失敗しても Armed(listener を握ったまま)に留まり、
        // 同じアドレスのまま Start を再試行できる。
        let links = self.build_links()?;
        let Some(listener) = self.listener.take() else {
            return Err("no listener: Arm again before Start".to_string());
        };
        let listener = TcpListener::from_std(listener)
            .map_err(|e| format!("cannot hand the listener to tokio: {e}"))?;

        self.metrics.reset();
        self.run_number = Some(run_number);

        let (tx, rx) = sync_channel::<FrameMsg>(self.params.queue_frames);
        let cfg = SenderConfig {
            source_id: self.params.cobo_id,
            run_number,
            batch_max_bytes: self.params.batch_max_bytes,
            batch_max: Duration::from_millis(self.params.batch_max_ms),
            heartbeat: Duration::from_millis(self.params.heartbeat_ms),
        };
        let sender_metrics = Arc::clone(&self.metrics);
        let abandon = Arc::new(AtomicBool::new(false));
        self.abandon = Some(Arc::clone(&abandon));
        let handle = std::thread::Builder::new()
            .name(format!("receiver{}-sender", self.params.cobo_id))
            .spawn(move || sender_loop(rx, links, cfg, sender_metrics, abandon))
            .map_err(|e| format!("cannot spawn sender thread: {e}"))?;
        // 前 run のスレッドは既に畳まれている(drain タスク終了でキューが切れる)。
        self.sender_handle = Some(handle);

        let (stop_tx, stop_rx) = broadcast::channel(1);
        tokio::spawn(drain_task(
            listener,
            tx,
            Arc::clone(&self.metrics),
            self.params.cobo_id,
            stop_rx,
        ));
        self.run_stop = Some(stop_tx);
        Ok(())
    }

    /// `Stop` は送出を打ち切らない(未送バッチと強制 EOS を送り切る = ロスレス)。
    /// SPEC §1.3 v1.6 の中止経路は「EOS を流して閉じる」なので、ここで打ち切ると
    /// 下流が run を閉じられなくなる。畳むのは drain タスクだけ。
    fn do_stop(&mut self) -> Result<(), String> {
        // drain タスクが listener を落とし、未送 EOS を出してから畳む。
        self.stop_run();
        self.bind_address = None;
        Ok(())
    }

    /// `Reset` はオペレータの明示オーバーライド(decoder の `do_reset` と同じ位置づけ)。
    /// 送出待ちを打ち切れる唯一の経路で、破棄は `messages_abandoned` として可視化される。
    /// **カウンタは消さない** —— 破棄が後から読めなければ打ち切りを許す条件を満たさない。
    fn do_reset(&mut self) -> Result<(), String> {
        self.abandon_sends();
        self.stop_run();
        // 打ち切り合図を出した後なので join は有限時間で返る(SNDTIMEO / poll 間隔で目覚める)。
        self.join_sender();
        self.listener = None;
        self.bind_address = None;
        self.run_number = None;
        // 過負荷ラッチも降ろす(Error → Idle が唯一の脱出路、SPEC §1.3)。
        self.metrics.overflowed.store(false, Ordering::Relaxed);
        Ok(())
    }

    fn stop_run(&mut self) {
        if let Some(stop) = self.run_stop.take() {
            let _ = stop.send(()); // 受け手が既に居なくても構わない
        }
    }

    /// 送信スレッドへ「送出待ちを打ち切ってよい」と伝える(`Reset` / 畳み込み専用)。
    fn abandon_sends(&mut self) {
        if let Some(abandon) = self.abandon.take() {
            abandon.store(true, Ordering::Relaxed);
        }
    }

    /// 送信スレッドの終了を待つ。**ここが「スレッドがリークしない」の担保**で、
    /// 呼ぶ前に必ず [`Handler::abandon_sends`] を済ませておくこと(そうでないと
    /// 下流不在のときに永久にブロックする)。
    fn join_sender(&mut self) {
        let Some(handle) = self.sender_handle.take() else {
            return;
        };
        if handle.join().is_err() {
            // スレッドが panic して死んでいた。カウンタが途中で止まっている可能性がある。
            error!(
                cobo_id = self.params.cobo_id,
                "sender thread panicked — its counters may be incomplete"
            );
        }
    }

    /// 下流 2 本の PUSH を張る。HWM と `ZMQ_IMMEDIATE` は必ず [`zmq_helper`] 経由
    /// (SPEC §1.4-2 / §1.2 v1.4)。
    ///
    /// IMMEDIATE により**下流(graw-writer / decoder)が居なければ送信タスクの send は
    /// ブロックする**(未接続の相手に「送れたことにする」のを禁じる)。これは never-stop と
    /// 矛盾しない: 詰まるのは送信タスクだけで、drain タスクは有界キューへ `try_send` するだけ
    /// なので止まらず、溢れは `overflow_frames` + Error として可視化される(SPEC §1.4-1/3)。
    fn build_links(&self) -> Result<Vec<Link>, String> {
        let specs = [
            ("graw-writer", &self.params.graw_writer_endpoint),
            ("decoder", &self.params.decoder_endpoint),
        ];
        let mut links = Vec::with_capacity(specs.len());
        for (name, endpoint) in specs {
            let socket = self
                .context
                .socket(zmq::PUSH)
                .map_err(|e| format!("cannot create PUSH socket for {name}: {e}"))?;
            // HWM も IMMEDIATE も connect の前に設定しないと効かない
            zmq_helper::apply_push_hwm_with(&socket, self.params.hwm)
                .map_err(|e| format!("cannot set HWM {} for {name}: {e}", self.params.hwm))?;
            // SNDTIMEO は「打ち切れる粒度」を作るためだけのもの(decoder と同じ出所・
            // 同じ既定 — TODO/023-2)。タイムアウトしても通常は諦めず再試行する。
            socket
                .set_sndtimeo(DEFAULT_RECEIVER_SEND_TIMEOUT_MS)
                .map_err(|e| format!("cannot set send timeout for {name}: {e}"))?;
            socket
                .connect(endpoint)
                .map_err(|e| format!("cannot connect {name} to {endpoint}: {e}"))?;
            info!(
                cobo_id = self.params.cobo_id,
                link = name,
                endpoint,
                "PUSH connected"
            );
            links.push(Link {
                name,
                socket,
                sequence_number: 0, // run 開始で 0 リセット(SPEC §2.2)
            });
        }
        Ok(links)
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
        let mut metrics = self.metrics.json();
        // controller はこの 2 つで DataLinkSet を組む(SPEC §8.2)。
        metrics["cobo_id"] = serde_json::json!(self.params.cobo_id);
        metrics["bind_address"] = match self.bind_address {
            Some(address) => serde_json::json!(address.to_string()),
            None => serde_json::Value::Null,
        };
        metrics["router_port"] = match self.bind_address {
            Some(address) => serde_json::json!(address.port()),
            None => serde_json::Value::Null,
        };
        response.with_metrics(metrics)
    }
}

impl Drop for Handler {
    /// コンポーネントごと畳む(プロセス終了・shutdown)。`Reset` と同じ扱いで送出待ちを
    /// 打ち切り、**送信スレッドを join してから**去る(ブロックしたまま残る OS スレッドを
    /// 作らない — TODO/023-2 の「スレッドがリークしない」)。
    fn drop(&mut self) {
        self.abandon_sends();
        self.stop_run();
        self.join_sender();
    }
}

// ---------------------------------------------------------------------
// エントリポイント
// ---------------------------------------------------------------------

/// receiver を起動し、`shutdown` が来るまでコマンドを処理し続ける。
///
/// `bound_command_endpoint` はコマンド REP の実 bind アドレスを 1 度だけ通知する
/// (`tcp://127.0.0.1:0` を使うテスト用。production は `None`)。
pub async fn run_receiver(
    params: ReceiverParams,
    shutdown: broadcast::Receiver<()>,
    bound_command_endpoint: Option<oneshot::Sender<String>>,
) {
    info!(
        cobo_id = params.cobo_id,
        listen = params.listen,
        command_listen = params.command_listen,
        graw_writer = params.graw_writer_endpoint,
        decoder = params.decoder_endpoint,
        "receiver starting"
    );
    let command_listen = params.command_listen.clone();
    let mut handler = Handler::new(params);
    run_command_task(
        command_listen,
        "receiver",
        shutdown,
        bound_command_endpoint,
        move |cmd| handler.handle(cmd),
    )
    .await;
}

// ---------------------------------------------------------------------
// テスト(仕様書 — 先に書く)。カウンタと「一度だけログ」の純ロジック(ZMQ/IO なし)。
// 実配線での打ち切り検証は tests/receiver_overload.rs。
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// 打ち切りは **毎回数え、ログは初回だけ**(ホットパスでログ整形を繰り返さない
    /// — CLAUDE.md。カウンタが常に進むので silent にはならない)。
    #[test]
    fn every_abandoned_message_is_counted_but_only_the_first_one_is_logged() {
        let m = Metrics::default();
        assert!(
            m.record_abandoned(0, "decoder", "Data"),
            "初回は warn を出す"
        );
        assert!(
            !m.record_abandoned(0, "graw-writer", "Data"),
            "2 回目以降は出さない"
        );
        assert!(!m.record_abandoned(0, "decoder", "EndOfStream"));

        assert_eq!(m.messages_abandoned.load(Ordering::Relaxed), 3);
        assert_eq!(m.json()["messages_abandoned"].as_u64(), Some(3));
    }

    /// framer リセット(MFM ヘッダ崩れ)は増分検知時に一度だけ warn、値は常に metrics へ。
    #[test]
    fn framer_resets_are_stored_every_time_but_warned_only_once() {
        let m = Metrics::default();
        assert!(m.note_framer_resets(1, 1), "最初の増分で warn");
        assert_eq!(m.json()["framer_resets"].as_u64(), Some(1));
        assert!(!m.note_framer_resets(1, 4), "2 回目以降は warn しない");
        assert_eq!(
            m.json()["framer_resets"].as_u64(),
            Some(4),
            "カウンタは黙って進み続ける"
        );
    }

    /// 送出失敗の 2 経路(符号化 / ETERM 以外の send)は数えて metrics に出す。
    ///
    /// **実配線での符号化失敗は事実上到達不能**: ペイロードは `Vec<ByteBuf>` で、
    /// `rmp_serde::to_vec` は `Vec<u8>` へ書くだけ(IO も未対応型も無い)。到達経路を
    /// 作れないので、ここではカウンタの配線だけを固定する(発注書 023 テスト節の
    /// 「不能なら理由を記録してカウンタ配線のみ検査」)。
    #[test]
    fn encode_and_send_failures_are_counted_and_exposed() {
        let m = Metrics::default();
        m.record_encode_error(
            0,
            "decoder",
            &crate::msg::MsgError::ItemFieldRange {
                field: "adc",
                value: 5000,
                max: 4095,
            },
        );
        // 非対称: 符号化 1 件・send 2 件
        m.record_send_error(0, "decoder", zmq::Error::EINVAL);
        m.record_send_error(0, "graw-writer", zmq::Error::EHOSTUNREACH);

        let json = m.json();
        assert_eq!(json["encode_errors"].as_u64(), Some(1));
        assert_eq!(json["send_errors"].as_u64(), Some(2));
        assert_eq!(
            json["messages_abandoned"].as_u64(),
            Some(0),
            "失敗と打ち切りは別のカウンタ"
        );
    }

    /// カウンタは run 毎(`Start` でリセット)。023 で足した 3 本も同じ規約に従う。
    #[test]
    fn start_resets_every_counter_including_the_new_ones() {
        let m = Metrics::default();
        m.record_abandoned(0, "decoder", "Data");
        m.record_encode_error(
            0,
            "decoder",
            &crate::msg::MsgError::ItemFieldRange {
                field: "adc",
                value: 5000,
                max: 4095,
            },
        );
        m.record_send_error(0, "decoder", zmq::Error::EINVAL);
        m.note_framer_resets(0, 3);
        m.record_dropped_frame(0, "queue full");

        m.reset();

        let json = m.json();
        for key in [
            "bytes",
            "frames",
            "batches",
            "overflow_frames",
            "framer_resets",
            "messages_abandoned",
            "encode_errors",
            "send_errors",
        ] {
            assert_eq!(json[key].as_u64(), Some(0), "metrics.{key} が残っている");
        }
        assert!(!m.overflowed.load(Ordering::Relaxed));
        assert!(
            m.record_abandoned(0, "decoder", "Data"),
            "次の run では warn も出し直す"
        );
    }
}
