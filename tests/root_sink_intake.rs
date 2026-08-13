//! root-sink 取り込み骨格(C++)の実配線試験(TODO/008 §4)。
//!
//! Rust 側から **本番エンコーダ**で作った ZMQ メッセージを PUSH で流し込み、C++ バイナリが
//! 数えた結果(終了時の JSON 1 行)を機械検証する。SPEC §6.2 のロスレス化チェックリストの
//! うち、プロセス境界を跨がないと確かめられない項目がここの担当:
//!
//! * 2/3. 有限 RCVHWM + 有界内部キュー → **送り手がブロックする**(背圧の実在)
//! * 5.   sequence_number ギャップ = fatal
//! * 6.   malformed バッチ = 即 exit 非 0(黙って捨てない)
//!
//! **env `TPCDAQ_ROOT_SINK_BIN` 未設定なら skip** —— C++ ビルド(libzmq 必須)を
//! `cargo test` の前提にしない。設定して走らせる手順:
//!
//! ```text
//! make -C tools/root_sink
//! TPCDAQ_ROOT_SINK_BIN=$PWD/tools/root_sink/root_sink cargo test --test root_sink_intake
//! ```

#![allow(clippy::unwrap_used)]

use std::io::Read;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use serde_bytes::ByteBuf;
use tpcdaq::msg::{self, Batch, Fragment, Fragments, Message};

// E2E(§6 の最後の 1 本)だけが使う。実 .graw が無い環境では丸ごと skip される。
use tokio::sync::{broadcast, oneshot};
use tpcdaq::command::{Command as DaqCommand, CommandResponse, RunConfig};
use tpcdaq::decoder::{run_decoder, DecoderParams};
use tpcdaq::msg::RawFrames;
use tpcdaq::receiver::{run_receiver, ReceiverParams};
use tpcdaq::zmq_helper;

/// root-sink の期待ソース(SPEC §3.2: decoder = 100)。
const DECODER_SOURCE_ID: u32 = 100;
const RUN_NUMBER: u32 = 7;

/// C++ 側の終了コード表(root_sink.cxx と対)。
const EXIT_MALFORMED: i32 = 2;
const EXIT_SEQ: i32 = 3;

// ---------------------------------------------------------------------
// skip ゲートとプロセス操作
// ---------------------------------------------------------------------

/// `TPCDAQ_ROOT_SINK_BIN` が指すバイナリ。未設定なら `None`(= テストは skip)。
fn sink_bin() -> Option<PathBuf> {
    std::env::var_os("TPCDAQ_ROOT_SINK_BIN").map(PathBuf::from)
}

/// 空きポートを 1 つ確保して即座に手放す(固定ポートを書かない = 並列実行に耐える)。
fn free_endpoint() -> String {
    let probe = TcpListener::bind("127.0.0.1:0").expect("bind probe listener");
    let port = probe.local_addr().expect("local_addr").port();
    drop(probe);
    format!("tcp://127.0.0.1:{port}")
}

/// root_sink を起動する。stdout はパイプ(終了時の JSON を読む)、stderr は継承
/// (進捗ログはテスト出力にそのまま出す —— パイプを溜めて詰まらせない)。
fn spawn_sink_raw(bin: &Path, endpoint: &str, extra: &[&str]) -> Child {
    Command::new(bin)
        .arg("--bind")
        .arg(endpoint)
        .args(extra)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn root_sink")
}

/// カウンタ系テストの起動口 —— **`--no-root`**(TTree を書かない高速経路、TODO/011 §2)。
/// 数え方・背圧・fatal の検査に ROOT IO は要らないし、テストの cwd(リポジトリ直下)に
/// run ディレクトリを作らせない。ROOT 書き出しそのものは
/// `writes_a_run_root_file_and_finalizes_it` と C++ 側 `make test-root` が受け持つ。
fn spawn_sink(bin: &Path, endpoint: &str, extra: &[&str]) -> Child {
    let mut args: Vec<&str> = vec!["--no-root"];
    args.extend_from_slice(extra);
    spawn_sink_raw(bin, endpoint, &args)
}

/// SIGTERM を送る。**libc を依存に足さない**ため kill(1) を呼ぶ
/// (Cargo.toml への依存追加は発注書で禁止。std::process::Child::kill は SIGKILL なので
///  JSON を出す前に死んでしまい使えない)。
fn send_term(pid: u32) {
    let status = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()
        .expect("run kill(1)");
    assert!(status.success(), "kill -TERM {pid} failed");
}

fn wait_for_exit(child: &mut Child, timeout: Duration) -> ExitStatus {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("root_sink did not exit within {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// 終了時に stdout へ出る JSON 1 行を読む。
fn read_counts(child: &mut Child) -> serde_json::Value {
    let mut out = String::new();
    child
        .stdout
        .take()
        .expect("piped stdout")
        .read_to_string(&mut out)
        .expect("read stdout");
    let line = out.lines().last().unwrap_or_default();
    serde_json::from_str(line)
        .unwrap_or_else(|e| panic!("stdout is not one JSON line: {out:?} ({e})"))
}

fn count(counts: &serde_json::Value, key: &str) -> u64 {
    counts[key]
        .as_u64()
        .unwrap_or_else(|| panic!("counter {key} missing in {counts}"))
}

// ---------------------------------------------------------------------
// メッセージ生成(本番エンコーダを使う)
// ---------------------------------------------------------------------

/// 非対称な合成 Fragment。`items` 個のサンプルを詰める(値は取り違えが見えるようずらす)。
fn fragment_at(event_idx: u32, cobo: u8, asad: u8, items: usize) -> Fragment {
    let words: Vec<u32> = (0..items)
        .map(|k| {
            msg::pack_item(
                (k % 4) as u8,
                ((k * 7) % 68) as u8,
                ((k * 13) % 512) as u16,
                ((k * 37) % 4096) as u16,
            )
            .expect("pack_item within range")
        })
        .collect();
    Fragment {
        event_idx,
        event_time: 0x0000_1234_5678_9abc + u64::from(event_idx),
        cobo,
        asad,
        frame_type: 2,
        revision: 5,
        read_offset: 136,
        status: 0,
        mult: [68, 0, 17, 3],
        window_out: 512,
        last_cell: [7, 9, 11, 13],
        items: ByteBuf::from(msg::items_to_bytes(&words)),
    }
}

/// 既定の期待集合(`--expect 0:0` = mini)向けの Fragment。
fn fragment(event_idx: u32, items: usize) -> Fragment {
    fragment_at(event_idx, 0, (event_idx % 2) as u8, items)
}

/// decoder → root-sink の Data バッチ 1 通(`fragment_items` = フラグメント毎の item 数)。
fn data_batch(sequence_number: u64, fragment_items: &[usize]) -> Vec<u8> {
    let payload: Fragments = fragment_items
        .iter()
        .enumerate()
        .map(|(i, n)| fragment(i as u32, *n))
        .collect();
    batch_of(sequence_number, payload)
}

/// 任意の Fragment 列を 1 バッチに詰める。
fn batch_of(sequence_number: u64, payload: Fragments) -> Vec<u8> {
    let message: Message<Fragments> = Message::Data(Batch {
        source_id: DECODER_SOURCE_ID,
        run_number: RUN_NUMBER,
        sequence_number,
        created_ns: 1_755_000_000_000_000_000 + sequence_number,
        payload,
    });
    message.to_msgpack().expect("encode Data batch")
}

fn end_of_stream() -> Vec<u8> {
    let message: Message<Fragments> = Message::EndOfStream {
        source_id: DECODER_SOURCE_ID,
        run_number: RUN_NUMBER,
    };
    message.to_msgpack().expect("encode EndOfStream")
}

/// PUSH を張る。sndtimeo を入れておく(受け側が上がらなかったときに永久に固まらない)。
fn connect_push(ctx: &zmq::Context, endpoint: &str, sndhwm: i32) -> zmq::Socket {
    let push = ctx.socket(zmq::PUSH).expect("PUSH socket");
    push.set_sndhwm(sndhwm).expect("set sndhwm"); // HWM は connect の前
    push.set_sndtimeo(15_000).expect("set sndtimeo");
    push.set_linger(2_000).expect("set linger"); // 残りを吐き出してから閉じる
    push.connect(endpoint).expect("connect PUSH");
    push
}

// ---------------------------------------------------------------------
// 1. 正常系 — Data×N + EOS を数え切る
// ---------------------------------------------------------------------

/// 手計算の出典:
///   バッチ 0: フラグメント 2 個(3 items, 5 items)= 8 items
///   バッチ 1: フラグメント 1 個(7 items)        = 7 items
///   バッチ 2: フラグメント 3 個(2, 4, 6 items)  = 12 items
///   合計 batches=3 / fragments=6 / items=27、EOS 1 本で runs=1。
#[test]
fn counts_data_batches_and_closes_the_run_on_eos() {
    let Some(bin) = sink_bin() else {
        eprintln!("SKIP: TPCDAQ_ROOT_SINK_BIN not set (C++ build is not a cargo test dependency)");
        return;
    };
    let endpoint = free_endpoint();
    let mut child = spawn_sink(&bin, &endpoint, &[]);
    let pid = child.id();

    let ctx = zmq::Context::new();
    let push = connect_push(&ctx, &endpoint, 100);
    let shapes: [&[usize]; 3] = [&[3, 5], &[7], &[2, 4, 6]];
    for (seq, shape) in shapes.iter().enumerate() {
        push.send(data_batch(seq as u64, shape), 0)
            .expect("send Data batch");
    }
    push.send(end_of_stream(), 0).expect("send EndOfStream");

    // 飛行中のメッセージを吸わせる猶予(sink 自身も停止後 200 ms のドレイン猶予を持つ)。
    std::thread::sleep(Duration::from_millis(500));
    send_term(pid);
    let status = wait_for_exit(&mut child, Duration::from_secs(10));
    assert!(
        status.success(),
        "SIGTERM は正常終了(exit 0)であるべき: {status:?}"
    );

    let counts = read_counts(&mut child);
    assert_eq!(count(&counts, "batches"), 3, "counts={counts}");
    assert_eq!(count(&counts, "fragments"), 6, "counts={counts}");
    assert_eq!(count(&counts, "items"), 27, "counts={counts}");
    assert_eq!(count(&counts, "eos"), 1, "counts={counts}");
    assert_eq!(count(&counts, "runs"), 1, "counts={counts}");
    // 異常カウンタはすべて 0(黙って何かを弾いていない)
    assert_eq!(count(&counts, "unknown"), 0, "counts={counts}");
    assert_eq!(count(&counts, "stale_eos"), 0, "counts={counts}");
    assert_eq!(count(&counts, "unexpected_sources"), 0, "counts={counts}");
    assert_eq!(count(&counts, "run_number_mismatch"), 0, "counts={counts}");
    assert_eq!(counts["fatal"], "", "counts={counts}");
}

// ---------------------------------------------------------------------
// 2. malformed = 即 exit 非 0(SPEC §6.2-6)
// ---------------------------------------------------------------------

/// ロスレス契約では「warn して捨てる」をしない。壊れた 1 通でプロセスごと落ちる。
#[test]
fn a_malformed_message_kills_the_process_instead_of_being_dropped() {
    let Some(bin) = sink_bin() else {
        eprintln!("SKIP: TPCDAQ_ROOT_SINK_BIN not set");
        return;
    };
    let endpoint = free_endpoint();
    let mut child = spawn_sink(&bin, &endpoint, &[]);

    let ctx = zmq::Context::new();
    let push = connect_push(&ctx, &endpoint, 100);
    // Message は fixmap(1) のはず。fixarray(3) を送りつける = 構造が壊れている。
    push.send(vec![0x93u8, 0x01, 0x02, 0x03], 0)
        .expect("send malformed message");

    let status = wait_for_exit(&mut child, Duration::from_secs(10));
    assert_eq!(
        status.code(),
        Some(EXIT_MALFORMED),
        "malformed は exit {EXIT_MALFORMED}(非 0)であるべき: {status:?}"
    );
    let counts = read_counts(&mut child);
    assert_eq!(count(&counts, "batches"), 0, "counts={counts}");
    assert_eq!(counts["fatal"], "malformed-message", "counts={counts}");
}

// ---------------------------------------------------------------------
// 3. sequence_number ギャップ = fatal(SPEC §6.2-5)
// ---------------------------------------------------------------------

#[test]
fn a_sequence_number_gap_is_fatal() {
    let Some(bin) = sink_bin() else {
        eprintln!("SKIP: TPCDAQ_ROOT_SINK_BIN not set");
        return;
    };
    let endpoint = free_endpoint();
    let mut child = spawn_sink(&bin, &endpoint, &[]);

    let ctx = zmq::Context::new();
    let push = connect_push(&ctx, &endpoint, 100);
    push.send(data_batch(0, &[3]), 0).expect("send seq 0");
    push.send(data_batch(2, &[3]), 0).expect("send seq 2"); // 1 が消えた

    let status = wait_for_exit(&mut child, Duration::from_secs(10));
    assert_eq!(
        status.code(),
        Some(EXIT_SEQ),
        "seq ギャップは exit {EXIT_SEQ} であるべき: {status:?}"
    );
    let counts = read_counts(&mut child);
    // ギャップの手前までは数え切っている(落ちた瞬間の状態が見える)
    assert_eq!(count(&counts, "batches"), 1, "counts={counts}");
    assert_eq!(counts["fatal"], "sequence-break", "counts={counts}");
}

/// decoder リンクは **厳格モード**(TODO/010 §2): run の初回 sequence_number は 0 のみ受理。
/// 先頭バッチを取りこぼしたまま黙って走り出さない(SPEC §2.2「run 開始で 0 リセット」)。
#[test]
fn a_run_whose_first_batch_is_missing_is_fatal() {
    let Some(bin) = sink_bin() else {
        eprintln!("SKIP: TPCDAQ_ROOT_SINK_BIN not set");
        return;
    };
    let endpoint = free_endpoint();
    let mut child = spawn_sink(&bin, &endpoint, &[]);

    let ctx = zmq::Context::new();
    let push = connect_push(&ctx, &endpoint, 100);
    push.send(data_batch(3, &[3]), 0).expect("send seq 3"); // 0,1,2 が消えた

    let status = wait_for_exit(&mut child, Duration::from_secs(10));
    assert_eq!(
        status.code(),
        Some(EXIT_SEQ),
        "初回 seq≠0 は exit {EXIT_SEQ} であるべき: {status:?}"
    );
    let counts = read_counts(&mut child);
    assert_eq!(count(&counts, "batches"), 0, "counts={counts}");
    assert_eq!(counts["fatal"], "sequence-break", "counts={counts}");
}

// ---------------------------------------------------------------------
// 4. 背圧の実在(SPEC §6.2-2/3)
// ---------------------------------------------------------------------

/// 受け手を詰まらせる(rcvhwm=1・内部キュー 1・1 通あたり 50 ms 消費)と、送り手の
/// 非ブロッキング送信が**有限回で EAGAIN になる** = 無制限バッファではない。
/// そのうえで残りをブロッキング送信すると **1 通も落ちない**(背圧 = ロスレスの手段)。
///
/// 1 通 = 65536 items = 256 KiB。小さすぎると OS の TCP バッファに吸われて偽陰性になる
/// (tests/zmq_backpressure.rs と同じ理由)。
#[test]
fn a_throttled_sink_makes_the_sender_block_without_losing_anything() {
    let Some(bin) = sink_bin() else {
        eprintln!("SKIP: TPCDAQ_ROOT_SINK_BIN not set");
        return;
    };
    const ITEMS_PER_BATCH: usize = 65_536; // 256 KiB
    const TOTAL: usize = 16;
    const THROTTLE_MS: u64 = 50;

    let endpoint = free_endpoint();
    let mut child = spawn_sink(
        &bin,
        &endpoint,
        &["--rcvhwm", "1", "--queue", "1", "--throttle-ms", "50"],
    );
    let pid = child.id();

    let ctx = zmq::Context::new();
    let push = connect_push(&ctx, &endpoint, 2);

    let batches: Vec<Vec<u8>> = (0..TOTAL)
        .map(|seq| data_batch(seq as u64, &[ITEMS_PER_BATCH]))
        .collect();

    // まず非ブロッキングで送れるだけ送る。詰まった時点(EAGAIN)が背圧の証拠。
    let mut sent = 0usize;
    let mut blocked_at: Option<usize> = None;
    while sent < TOTAL {
        match push.send(batches[sent].clone(), zmq::DONTWAIT) {
            Ok(()) => sent += 1,
            Err(zmq::Error::EAGAIN) => {
                blocked_at = Some(sent);
                break;
            }
            Err(e) => panic!("unexpected send error: {e}"),
        }
    }
    let blocked_at = blocked_at.unwrap_or_else(|| {
        panic!("送り手が {TOTAL} 通すべて非ブロッキングで送れてしまった = 背圧がない")
    });
    assert!(
        blocked_at < TOTAL,
        "EAGAIN は {TOTAL} 通に届く前に起きるはず(実測 {blocked_at} 通目)"
    );

    // 残りはブロッキングで送る(待たされるが、捨てられはしない)。
    for batch in &batches[sent..] {
        push.send(batch.clone(), 0).expect("blocking send");
    }
    push.send(end_of_stream(), 0).expect("send EndOfStream");

    // 絞っている分の消化を待つ(TOTAL × throttle + 余裕)。
    std::thread::sleep(Duration::from_millis(TOTAL as u64 * THROTTLE_MS + 800));
    send_term(pid);
    let status = wait_for_exit(&mut child, Duration::from_secs(30));
    assert!(status.success(), "SIGTERM は正常終了であるべき: {status:?}");

    let counts = read_counts(&mut child);
    // ロスレス: 送った分がそのまま届いている(1 通も落ちない)
    assert_eq!(count(&counts, "batches"), TOTAL as u64, "counts={counts}");
    assert_eq!(count(&counts, "fragments"), TOTAL as u64, "counts={counts}");
    assert_eq!(
        count(&counts, "items"),
        (TOTAL * ITEMS_PER_BATCH) as u64,
        "counts={counts}"
    );
    assert_eq!(count(&counts, "eos"), 1, "counts={counts}");
    assert_eq!(count(&counts, "runs"), 1, "counts={counts}");
    eprintln!("back-pressure: EAGAIN after {blocked_at} non-blocking sends, all {TOTAL} delivered");
}

// ---------------------------------------------------------------------
// 5. イベントビルダ(SPEC §6.3、TODO/010)
// ---------------------------------------------------------------------

/// ELITPC 相当の期待フラグメント集合(2 CoBo × 2 AsAd)。
const ELITPC_EXPECT: &str = "0:0,0:1,1:0,1:1";
/// 投入するソースの順(意図的に (0,0)→(1,1)→(0,1)→(1,0) と並べ替えてある)。
const ELITPC_SOURCES: [(u8, u8); 4] = [(0, 0), (1, 1), (0, 1), (1, 0)];
/// タイムアウトを効かせない(このテストで見たいのは「揃ったら出る」「EOS で出る」の方)。
const NO_TIMEOUT_MS: &str = "60000";

/// 投入順(event_idx, cobo, asad)を作る。ソース毎に 1 周(ラウンド)し、ラウンド毎に
/// event_idx の向きを反転させる。**どのイベントも最終ラウンドまで揃わない**ので、
/// 「先頭 event_idx が揃うまで 1 個も出さない」(head-of-line)と「揃った瞬間に
/// 昇順で出る」を同時に踏む。
fn shuffled_delivery(events: u32) -> Vec<(u32, u8, u8)> {
    let mut out = Vec::new();
    for (round, (cobo, asad)) in ELITPC_SOURCES.iter().enumerate() {
        let mut order: Vec<u32> = (0..events).collect();
        if round % 2 == 1 {
            order.reverse();
        }
        for event_idx in order {
            out.push((event_idx, *cobo, *asad));
        }
    }
    out
}

/// 投入計画をバッチに詰めて送る。フラグメント毎の item 数は `1 + event_idx`
/// (イベント毎に違う値 = items の合計が手計算できる)。戻り値 = (batches, fragments, items)。
fn send_plan(push: &zmq::Socket, plan: &[(u32, u8, u8)], per_batch: usize) -> (u64, u64, u64) {
    let mut batches = 0u64;
    let mut items = 0u64;
    for chunk in plan.chunks(per_batch) {
        let payload: Fragments = chunk
            .iter()
            .map(|(event_idx, cobo, asad)| {
                let n = 1 + *event_idx as usize;
                items += n as u64;
                fragment_at(*event_idx, *cobo, *asad, n)
            })
            .collect();
        push.send(batch_of(batches, payload), 0)
            .expect("send Data batch");
        batches += 1;
    }
    (batches, plan.len() as u64, items)
}

/// 2 CoBo × 2 AsAd を順序シャッフルで投入 → 全イベントが complete で組み上がる。
///
/// 手計算の出典:
///   イベント 6 個(idx 0–5)× ソース 4 個 = 24 フラグメント、4 個ずつ詰めて 6 バッチ。
///   items = Σ_e 4 × (1 + e) = 4 × (1+2+3+4+5+6) = 4 × 21 = **84**。
///   全イベントが 4 ソース揃うので events_complete = **6**、incomplete / late は 0。
#[test]
fn two_cobo_two_asad_shuffled_fragments_build_complete_events() {
    let Some(bin) = sink_bin() else {
        eprintln!("SKIP: TPCDAQ_ROOT_SINK_BIN not set");
        return;
    };
    const EVENTS: u32 = 6;

    let endpoint = free_endpoint();
    let mut child = spawn_sink(
        &bin,
        &endpoint,
        &[
            "--expect",
            ELITPC_EXPECT,
            "--build-timeout-ms",
            NO_TIMEOUT_MS,
        ],
    );
    let pid = child.id();

    let ctx = zmq::Context::new();
    let push = connect_push(&ctx, &endpoint, 100);
    let plan = shuffled_delivery(EVENTS);
    let (batches, fragments, items) = send_plan(&push, &plan, 4);
    assert_eq!(
        (batches, fragments, items),
        (6, 24, 84),
        "投入計画の自己検算"
    );
    push.send(end_of_stream(), 0).expect("send EndOfStream");

    std::thread::sleep(Duration::from_millis(500));
    send_term(pid);
    let status = wait_for_exit(&mut child, Duration::from_secs(10));
    assert!(status.success(), "SIGTERM は正常終了であるべき: {status:?}");

    let counts = read_counts(&mut child);
    assert_eq!(count(&counts, "batches"), batches, "counts={counts}");
    assert_eq!(count(&counts, "fragments"), fragments, "counts={counts}");
    assert_eq!(count(&counts, "items"), items, "counts={counts}");
    assert_eq!(
        count(&counts, "events_complete"),
        u64::from(EVENTS),
        "counts={counts}"
    );
    assert_eq!(count(&counts, "events_incomplete"), 0, "counts={counts}");
    assert_eq!(count(&counts, "late_fragments"), 0, "counts={counts}");
    assert_eq!(count(&counts, "unexpected_fragments"), 0, "counts={counts}");
    assert_eq!(count(&counts, "duplicate_fragments"), 0, "counts={counts}");
    assert_eq!(count(&counts, "pending_events"), 0, "counts={counts}");
    assert_eq!(count(&counts, "runs"), 1, "counts={counts}");
    assert_eq!(counts["fatal"], "", "counts={counts}");
    eprintln!("event builder (2 CoBo x 2 AsAd, shuffled): {counts}");
}

/// 1 フラグメント欠けの run: EOS の flush で **incomplete フラグ付きで出る**(捨てない)。
///
/// 手計算の出典:
///   イベント 4 個(idx 0–3)× 4 ソース = 16 のうち、(event 2, cobo 1, asad 0) を 1 個抜く
///   → 15 フラグメント、4 個ずつ詰めて 4 バッチ(最後は 3 個)。
///   items = 4×1 + 4×2 + **3**×3 + 4×4 = 4 + 8 + 9 + 16 = **37**。
///   揃うのは event 0/1/3 の 3 個 = events_complete 3、event 2 が events_incomplete 1。
///   build-timeout は 60 s なので、この 1 個を出すのは **EOS の flush**(タイムアウトではない)。
#[test]
fn a_missing_fragment_is_emitted_as_an_incomplete_event_on_eos() {
    let Some(bin) = sink_bin() else {
        eprintln!("SKIP: TPCDAQ_ROOT_SINK_BIN not set");
        return;
    };
    const EVENTS: u32 = 4;
    const MISSING: (u32, u8, u8) = (2, 1, 0);

    let endpoint = free_endpoint();
    let mut child = spawn_sink(
        &bin,
        &endpoint,
        &[
            "--expect",
            ELITPC_EXPECT,
            "--build-timeout-ms",
            NO_TIMEOUT_MS,
        ],
    );
    let pid = child.id();

    let ctx = zmq::Context::new();
    let push = connect_push(&ctx, &endpoint, 100);
    let plan: Vec<(u32, u8, u8)> = shuffled_delivery(EVENTS)
        .into_iter()
        .filter(|f| *f != MISSING)
        .collect();
    let (batches, fragments, items) = send_plan(&push, &plan, 4);
    assert_eq!(
        (batches, fragments, items),
        (4, 15, 37),
        "投入計画の自己検算"
    );
    push.send(end_of_stream(), 0).expect("send EndOfStream");

    std::thread::sleep(Duration::from_millis(500));
    send_term(pid);
    let status = wait_for_exit(&mut child, Duration::from_secs(10));
    assert!(status.success(), "SIGTERM は正常終了であるべき: {status:?}");

    let counts = read_counts(&mut child);
    assert_eq!(count(&counts, "fragments"), fragments, "counts={counts}");
    assert_eq!(count(&counts, "items"), items, "counts={counts}");
    assert_eq!(count(&counts, "events_incomplete"), 1, "counts={counts}");
    assert_eq!(count(&counts, "events_complete"), 3, "counts={counts}");
    assert_eq!(count(&counts, "late_fragments"), 0, "counts={counts}");
    assert_eq!(count(&counts, "pending_events"), 0, "counts={counts}");
    assert_eq!(count(&counts, "runs"), 1, "counts={counts}");
    assert_eq!(counts["fatal"], "", "counts={counts}");
    eprintln!("event builder (one fragment missing): {counts}");
}

// ---------------------------------------------------------------------------
// 6. ROOT 書き出し(SPEC §6.4/§6.5、TODO/011)
// ---------------------------------------------------------------------------

/// テスト毎の使い捨てディレクトリ(固定パスを書かない = 並列実行・再実行に耐える)。
fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tpcdaq_root_sink_intake_{}_{tag}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// `dir` 直下で `prefix` 始まりのファイル名(存在しないディレクトリは空扱い)。
fn names_starting_with(dir: &Path, prefix: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(prefix))
        .collect();
    out.sort();
    out
}

/// run 全体(Data×3 + EOS)を流し、**`run{run:04}.root` が出来て inprogress が
/// 残っていない**ことと、JSON の `entries_written` が投入フラグメント数と一致することを見る。
///
/// 手計算の出典:
///   バッチ 0: event 0(2 items)+ event 1(3 items)
///   バッチ 1: event 2(4 items)
///   バッチ 2: event 3(5 items)+ event 4(6 items)
///   すべて (cobo,asad) = (0,0) = 既定の期待集合なので **1 イベント = 1 フラグメント**で
///   即 complete。fragments = **5**、items = 2+3+4+5+6 = **20**、events_complete = **5**。
///   **1 エントリ = 1 フラグメント**(SPEC §6.4)なので entries_written = **5**。
#[test]
fn writes_a_run_root_file_and_finalizes_it() {
    let Some(bin) = sink_bin() else {
        eprintln!("SKIP: TPCDAQ_ROOT_SINK_BIN not set");
        return;
    };
    let out_root = scratch_dir("finalize");
    let endpoint = free_endpoint();
    // v1.8: このテストは GDataFrame 出力(entries == fragments)の回帰なので
    // テスト専用モードを明示する(既定は PEventTPC — SPEC §6.4 v1.8 / TODO/020)。
    let mut child = spawn_sink_raw(
        &bin,
        &endpoint,
        &[
            "--format",
            "gdataframe",
            "--output-root",
            &out_root.to_string_lossy(),
        ],
    );
    let pid = child.id();

    let ctx = zmq::Context::new();
    let push = connect_push(&ctx, &endpoint, 100);
    // event_idx を昇順・単調にし、(cobo,asad) は既定の期待集合 (0,0) に固定する。
    let plan: [&[(u32, usize)]; 3] = [&[(0, 2), (1, 3)], &[(2, 4)], &[(3, 5), (4, 6)]];
    let mut items_total = 0u64;
    let mut fragments_total = 0u64;
    for (seq, batch) in plan.iter().enumerate() {
        let payload: Fragments = batch
            .iter()
            .map(|(event_idx, n)| {
                items_total += *n as u64;
                fragments_total += 1;
                fragment_at(*event_idx, 0, 0, *n)
            })
            .collect();
        push.send(batch_of(seq as u64, payload), 0)
            .expect("send Data batch");
    }
    assert_eq!(
        (fragments_total, items_total),
        (5, 20),
        "投入計画の自己検算"
    );
    push.send(end_of_stream(), 0).expect("send EndOfStream");

    std::thread::sleep(Duration::from_millis(700));
    send_term(pid);
    let status = wait_for_exit(&mut child, Duration::from_secs(20));
    assert!(status.success(), "SIGTERM は正常終了であるべき: {status:?}");

    let counts = read_counts(&mut child);
    assert_eq!(
        count(&counts, "fragments"),
        fragments_total,
        "counts={counts}"
    );
    assert_eq!(count(&counts, "items"), items_total, "counts={counts}");
    assert_eq!(count(&counts, "events_complete"), 5, "counts={counts}");
    assert_eq!(count(&counts, "runs"), 1, "counts={counts}");
    // 1 エントリ = 1 フラグメント(SPEC §6.4)
    assert_eq!(
        count(&counts, "entries_written"),
        fragments_total,
        "counts={counts}"
    );
    assert_eq!(count(&counts, "items_out_of_range"), 0, "counts={counts}");
    assert_eq!(counts["fatal"], "", "counts={counts}");

    // --- ファイルライフサイクル(SPEC §6.5)---
    let run_dir = out_root.join(format!("run{RUN_NUMBER:04}"));
    let run_file = run_dir.join(format!("run{RUN_NUMBER:04}.root"));
    assert!(
        run_file.is_file(),
        "{} が無い(finalize されていない)。dir={:?}",
        run_file.display(),
        names_starting_with(&run_dir, "")
    );
    assert!(
        names_starting_with(&run_dir, "run_inprogress_").is_empty(),
        "inprogress が残っている: {:?}",
        names_starting_with(&run_dir, "run_inprogress_")
    );

    // --- JSON の root_files 台帳が実ファイルと一致する ---
    let files = counts["root_files"]
        .as_array()
        .unwrap_or_else(|| panic!("root_files missing in {counts}"));
    assert_eq!(files.len(), 1, "counts={counts}");
    assert_eq!(
        files[0]["path"].as_str(),
        Some(run_file.to_string_lossy().as_ref()),
        "counts={counts}"
    );
    assert_eq!(files[0]["entries"].as_u64(), Some(5), "counts={counts}");
    let bytes = files[0]["bytes"].as_u64().expect("bytes");
    assert_eq!(
        bytes,
        std::fs::metadata(&run_file).expect("stat run file").len(),
        "JSON の bytes が実サイズと違う"
    );

    eprintln!("root recorder: {counts}");
    let _ = std::fs::remove_dir_all(&out_root);
}

// ---------------------------------------------------------------------------
// 7. E2E — graw_replay → receiver(006)→ decoder(009)→ root_sink(011)
// ---------------------------------------------------------------------------
//
// **env `TPCDAQ_REAL_GRAW` + `TPCDAQ_ROOT_SINK_BIN` の両方が要る**(実 .graw は
// リポに入れない / C++ ビルドを cargo test の前提にしない —— CLAUDE.md)。
//
// `tests/decoder_pipeline_real_graw.rs` の配線から、終端のテスト用 PULL を
// **本物の root_sink プロセス**に差し替えたもの。あちらが「ワイヤに 108 フラグメント /
// 15,040,512 items が出る」を主張するのに対し、こちらは
// **それが ROOT ファイルの 108 エントリになる**(SPEC §6.4: 1 エントリ = 1 CoBo フレーム)を
// 主張する —— カウンタ JSON と、書かれた .root を読み戻した実測の両方で。

/// receiver / decoder の command REP に 1 往復。
async fn rpc(endpoint: &str, cmd: &DaqCommand) -> CommandResponse {
    let ctx = tmq::Context::new();
    let sender = tmq::request(&ctx).connect(endpoint).unwrap();
    {
        use tmq::AsZmqSocket;
        sender.get_socket().set_linger(0).unwrap();
    }
    let receiver = sender
        .send(vec![cmd.to_json().unwrap()].into())
        .await
        .unwrap();
    let (mut reply, _sender) = receiver.recv().await.unwrap();
    CommandResponse::from_json(&reply.pop_front().unwrap()).unwrap()
}

/// Configure → Arm。戻り値は Arm が報告する bind アドレス。
async fn configure_arm(endpoint: &str, run: u32, comment: &str) -> String {
    rpc(
        endpoint,
        &DaqCommand::Configure(RunConfig {
            run_number: run,
            comment: comment.to_string(),
            config: serde_json::Value::Null,
        }),
    )
    .await;
    let armed = rpc(endpoint, &DaqCommand::Arm).await;
    assert!(armed.success, "Arm failed: {}", armed.message);
    armed.metrics.as_ref().unwrap()["bind_address"]
        .as_str()
        .unwrap()
        .to_string()
}

fn bind_pull(ctx: &zmq::Context, timeout_ms: i32) -> (zmq::Socket, String) {
    let sock = ctx.socket(zmq::PULL).unwrap();
    sock.set_linger(0).unwrap();
    zmq_helper::apply_pull_hwm(&sock).unwrap();
    sock.bind("tcp://127.0.0.1:0").unwrap();
    sock.set_rcvtimeo(timeout_ms).unwrap();
    let endpoint = sock.get_last_endpoint().unwrap().unwrap();
    (sock, endpoint)
}

/// `path` が現れるまで待つ(現れなければ false)。
fn wait_for_file(path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.is_file() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// P1 オラクル(SPEC §12-1): 実 .graw 1 本 = 108 イベント / 15,040,512 items。
/// mini は 1 CoBo × 1 AsAd なので **1 イベント = 1 フレーム = TTree 1 エントリ**。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_graw_replayed_end_to_end_writes_108_entries() {
    let Some(bin) = sink_bin() else {
        eprintln!("SKIP: TPCDAQ_ROOT_SINK_BIN not set");
        return;
    };
    let Ok(graw) = std::env::var("TPCDAQ_REAL_GRAW") else {
        eprintln!("SKIP: TPCDAQ_REAL_GRAW が未設定(実 .graw はローカルのみ)");
        return;
    };
    const RUN: u32 = RUN_NUMBER;
    let out_root = scratch_dir("e2e");
    let ctx = zmq::Context::new();

    // --- root_sink(本物のプロセス)を先に上げる。listen-before-start と同じ理屈 ---
    let sink_ep = free_endpoint();
    let mut sink = spawn_sink_raw(
        &bin,
        &sink_ep,
        // v1.8: mini 実データオラクル(entries=108、GDataFrame)の回帰なのでテスト専用モード。
        &[
            "--format",
            "gdataframe",
            "--output-root",
            &out_root.to_string_lossy(),
            "--expect",
            "0:0",
        ],
    );
    let sink_pid = sink.id();
    std::thread::sleep(Duration::from_millis(300)); // bind が済むまで

    // --- decoder(009): PUSH 先は root_sink ---
    let decoder_params = DecoderParams {
        pull_bind: "tcp://127.0.0.1:0".to_string(),
        push_connect: sink_ep.clone(),
        command_listen: "tcp://127.0.0.1:0".to_string(),
        batch_max_bytes: tpcdaq::config::DEFAULT_BATCH_MAX_BYTES,
        batch_max_ms: tpcdaq::config::DEFAULT_BATCH_MAX_MS,
        heartbeat_ms: tpcdaq::config::DEFAULT_HEARTBEAT_MS,
        send_timeout_ms: tpcdaq::config::DEFAULT_DECODER_SEND_TIMEOUT_MS,
        workers: 1,
        expected_sources: vec![0],
    };
    let (dec_shutdown_tx, dec_shutdown_rx) = broadcast::channel(1);
    let (dec_ep_tx, dec_ep_rx) = oneshot::channel();
    let dec_task = tokio::spawn(run_decoder(
        decoder_params,
        dec_shutdown_rx,
        Some(dec_ep_tx),
    ));
    let dec_cmd_ep = dec_ep_rx.await.expect("decoder command endpoint");
    let decoder_pull_bind = configure_arm(&dec_cmd_ep, RUN, "root_sink e2e").await;
    let started = rpc(&dec_cmd_ep, &DaqCommand::Start { run_number: RUN }).await;
    assert!(started.success, "{}", started.message);

    // --- graw-writer 行きは drain するだけ(HWM で receiver を止めない。007 E2E の流儀) ---
    let (writer_pull, writer_ep) = bind_pull(&ctx, 60_000);
    let writer_drain = std::thread::spawn(move || {
        while let Ok(raw) = writer_pull.recv_bytes(0) {
            if let Ok(Message::<RawFrames>::EndOfStream { .. }) =
                Message::<RawFrames>::from_msgpack(&raw)
            {
                break;
            }
        }
    });

    // --- receiver(006) ---
    let recv_params = ReceiverParams {
        cobo_id: 0,
        listen: "127.0.0.1:0".to_string(),
        command_listen: "tcp://127.0.0.1:0".to_string(),
        graw_writer_endpoint: writer_ep,
        decoder_endpoint: decoder_pull_bind,
        batch_max_bytes: tpcdaq::config::DEFAULT_BATCH_MAX_BYTES,
        batch_max_ms: tpcdaq::config::DEFAULT_BATCH_MAX_MS,
        queue_frames: tpcdaq::config::DEFAULT_QUEUE_FRAMES,
        heartbeat_ms: tpcdaq::config::DEFAULT_HEARTBEAT_MS,
        hwm: zmq_helper::DEFAULT_HWM,
    };
    let (recv_shutdown_tx, recv_shutdown_rx) = broadcast::channel(1);
    let (recv_ep_tx, recv_ep_rx) = oneshot::channel();
    let recv_task = tokio::spawn(run_receiver(
        recv_params,
        recv_shutdown_rx,
        Some(recv_ep_tx),
    ));
    let recv_cmd_ep = recv_ep_rx.await.expect("receiver command endpoint");
    let data_addr = configure_arm(&recv_cmd_ep, RUN, "root_sink e2e").await;
    rpc(&recv_cmd_ep, &DaqCommand::Start { run_number: RUN }).await;

    // --- graw_replay(全速) ---
    let started_at = Instant::now();
    let status = Command::new(env!("CARGO_BIN_EXE_graw_replay"))
        .arg(&data_addr)
        .arg(&graw)
        .status()
        .expect("spawn graw_replay");
    assert!(status.success(), "graw_replay failed: {status:?}");

    // --- run が finalize される(= rename が済む)まで待つ ---
    let run_dir = out_root.join(format!("run{RUN:04}"));
    let run_file = run_dir.join(format!("run{RUN:04}.root"));
    let finalized = wait_for_file(&run_file, Duration::from_secs(60));
    let elapsed = started_at.elapsed();
    assert!(
        finalized,
        "{} が 60 s 以内に出来なかった(dir={:?})",
        run_file.display(),
        names_starting_with(&run_dir, "")
    );
    assert!(
        names_starting_with(&run_dir, "run_inprogress_").is_empty(),
        "inprogress が残っている: {:?}",
        names_starting_with(&run_dir, "run_inprogress_")
    );

    send_term(sink_pid);
    let sink_status = wait_for_exit(&mut sink, Duration::from_secs(30));
    assert!(sink_status.success(), "root_sink: {sink_status:?}");
    let counts = read_counts(&mut sink);

    // --- P1 オラクル(SPEC §12-1)---
    assert_eq!(count(&counts, "fragments"), 108, "counts={counts}");
    assert_eq!(count(&counts, "items"), 15_040_512, "counts={counts}");
    assert_eq!(count(&counts, "events_complete"), 108, "counts={counts}");
    assert_eq!(count(&counts, "events_incomplete"), 0, "counts={counts}");
    assert_eq!(count(&counts, "late_fragments"), 0, "counts={counts}");
    assert_eq!(count(&counts, "runs"), 1, "counts={counts}");
    // **1 エントリ = 1 CoBo フレーム**(SPEC §6.4)
    assert_eq!(count(&counts, "entries_written"), 108, "counts={counts}");
    assert_eq!(count(&counts, "items_out_of_range"), 0, "counts={counts}");
    assert_eq!(counts["fatal"], "", "counts={counts}");
    let files = counts["root_files"].as_array().expect("root_files");
    assert_eq!(files.len(), 1, "run 1 本 = ファイル 1 本(1 GiB 未満)");
    assert_eq!(files[0]["entries"].as_u64(), Some(108), "counts={counts}");

    // --- 書かれた .root を読み戻して照合(test_recorder の inspect モード)---
    let inspector = bin.with_file_name("test_recorder");
    if inspector.is_file() {
        let out = Command::new(&inspector)
            .arg(&run_file)
            .output()
            .expect("run test_recorder inspect");
        let text = String::from_utf8_lossy(&out.stdout);
        eprintln!("read-back: {}", text.trim());
        assert!(out.status.success(), "inspect failed: {text}");
        assert!(
            text.contains("entries=108"),
            "読み戻しが 108 エントリでない: {text}"
        );
    } else {
        eprintln!(
            "NOTE: {} が無いので読み戻し照合は skip(make -C tools/root_sink test-root で作られる)",
            inspector.display()
        );
    }

    eprintln!(
        "e2e replay+decode+record: {:.3} s, {} bytes -> {}",
        elapsed.as_secs_f64(),
        std::fs::metadata(&graw).map(|m| m.len()).unwrap_or(0),
        run_file.display()
    );
    eprintln!("root_sink counters: {counts}");

    let _ = recv_shutdown_tx.send(());
    let _ = tokio::time::timeout(Duration::from_secs(5), recv_task).await;
    let _ = dec_shutdown_tx.send(());
    let _ = tokio::time::timeout(Duration::from_secs(5), dec_task).await;
    let _ = writer_drain.join();
    let _ = std::fs::remove_dir_all(&out_root);
}
