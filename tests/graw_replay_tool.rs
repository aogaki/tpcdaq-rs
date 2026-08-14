//! graw_replay の E2E テスト(TODO/005)。
//!
//! バイナリを `env!("CARGO_BIN_EXE_graw_replay")` で直接起動し、`127.0.0.1:0` で listen した
//! TCP 受け口に対して合成バイト列(実 GRAW 形式である必要はない — バイト単位の配送を検証するだけ)
//! をリプレイさせ、受信側での一致・ペーシング・loop 挙動を確認する。004 (framer/decoder) には依存しない。

#![allow(clippy::unwrap_used)]

use std::io::Read;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// ビルド済み graw_replay バイナリのパス。
fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_graw_replay")
}

/// テスト用の一時ファイルパスを発行する(pid + 経過ナノ秒で並列テスト同士の衝突を避ける)。
fn temp_path(tag: &str) -> PathBuf {
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("tpcdaq_graw_replay_{tag}_{pid}_{nanos}"))
}

/// 手計算で決めた非対称パターンの合成バイト列(全ゼロ/全 0xFF のような退化ケースを避ける)。
fn synth_bytes(len: usize) -> Vec<u8> {
    (0..len).map(|i| ((i * 7 + 3) % 251) as u8).collect()
}

/// バイト一致: `--loop` なしでファイル全体を送り切ったら接続を閉じ、受信側は EOF まで
/// 読み切った内容がソースファイルと完全一致すること(TODO/005 受け入れ#1)。
/// chunk-bytes 既定 65536 の非倍数長にして、途中半端なチャンク送出も突いておく。
#[test]
fn replays_file_bytes_exactly_over_tcp() {
    let file_len = 200_003usize; // 65536 の倍数ではない
    let content = synth_bytes(file_len);
    let path = temp_path("bytes");
    std::fs::write(&path, &content).expect("write fixture file");

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let addr = listener.local_addr().expect("local_addr");

    let mut child = Command::new(bin())
        .arg(addr.to_string())
        .arg(&path)
        .spawn()
        .expect("spawn graw_replay");

    let (mut sock, _peer) = listener.accept().expect("accept connection");
    let mut received = Vec::new();
    sock.read_to_end(&mut received)
        .expect("read until sender closes the connection");

    let status = child.wait().expect("wait for graw_replay to exit");
    assert!(
        status.success(),
        "graw_replay should exit 0 on a clean full-speed replay, got {status:?}"
    );
    assert_eq!(
        received, content,
        "bytes received over TCP must match the source file exactly"
    );

    let _ = std::fs::remove_file(&path);
}

/// ペーシング smoke: `--rate-mbps` 指定時の実効経過時間が理論値の ±30% に収まること
/// (TODO/005 受け入れ: フレークしない大きめマージンでの短時間 smoke。厳密な精度試験はしない)。
///
/// rate_mbps=4.0 → 4,000,000 bit/s / 8 = 500,000 byte/s。file_len を 500,000 byte に
/// 合わせているので理論経過時間はちょうど 1.0 秒(手計算: 500,000 / 500,000 = 1.0)。
#[test]
fn rate_mbps_paces_within_30_percent_margin() {
    let rate_mbps = 4.0_f64;
    let file_len = 500_000usize;
    let expected_secs = 1.0_f64;

    let content = synth_bytes(file_len);
    let path = temp_path("pace");
    std::fs::write(&path, &content).expect("write fixture file");

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let addr = listener.local_addr().expect("local_addr");

    let mut child = Command::new(bin())
        .arg(addr.to_string())
        .arg(&path)
        .arg("--rate-mbps")
        .arg(rate_mbps.to_string())
        .spawn()
        .expect("spawn graw_replay");

    let (mut sock, _peer) = listener.accept().expect("accept connection");
    // 受信側の読み出しが遅れて送出側へ逆圧をかけないよう、別スレッドで即座に drain する。
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        sock.read_to_end(&mut buf).expect("read until close");
        buf
    });

    let start = Instant::now();
    let status = child.wait().expect("wait for graw_replay to exit");
    let elapsed = start.elapsed().as_secs_f64();
    let received = reader.join().expect("reader thread panicked");

    assert!(
        status.success(),
        "graw_replay should exit 0 on a clean paced replay, got {status:?}"
    );
    assert_eq!(
        received.len(),
        file_len,
        "receiver must get the full file regardless of pacing"
    );

    let lower = expected_secs * 0.7;
    let upper = expected_secs * 1.3;
    assert!(
        (lower..=upper).contains(&elapsed),
        "paced replay took {elapsed:.3}s, want within [{lower:.3}, {upper:.3}]s \
         for --rate-mbps {rate_mbps} over {file_len} bytes"
    );

    let _ = std::fs::remove_file(&path);
}

/// `--loop`: EOF に達したらファイル先頭へ戻って送り続けること。2 周分以上を受信できたら
/// 内容を検証して打ち切る(TODO/005 受け入れ: 「2 周分以上受けたら切る」形での確認)。
#[test]
fn loop_flag_repeats_file_at_least_twice() {
    let file_len = 37usize; // chunk-bytes 既定より十分小さい単純なケース
    let content = synth_bytes(file_len);
    let path = temp_path("loop");
    std::fs::write(&path, &content).expect("write fixture file");

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let addr = listener.local_addr().expect("local_addr");

    let mut child = Command::new(bin())
        .arg(addr.to_string())
        .arg(&path)
        .arg("--loop")
        .spawn()
        .expect("spawn graw_replay");

    let (mut sock, _peer) = listener.accept().expect("accept connection");

    let mut received = vec![0u8; file_len * 2];
    sock.read_exact(&mut received)
        .expect("read at least two loops worth of bytes");

    let mut expected = content.clone();
    expected.extend_from_slice(&content);
    assert_eq!(
        received, expected,
        "--loop must replay the file bytes verbatim, twice back to back"
    );

    // 受信側を閉じて 3 周目以降の送出を失敗させ、`--loop` の無限送出を確実に止める。
    drop(sock);
    let _ = child.kill();
    let _ = child.wait();

    let _ = std::fs::remove_file(&path);
}

// ---------------------------------------------------------------------------
// 複数ファイルの eventIdx マージ送出(TODO/021)
// ---------------------------------------------------------------------------

/// 最小の合成 CoBo フレーム(blkSize=1・big-endian・frameType 1、96 B)。
/// Framer が必要とするのは metaType + frameSize、マージが必要とするのは
/// frameType(5..7)+ eventIdx(22..26)+ asadIdx(27)だけ。残りは `fill` の
/// 埋め草にして内容一致の検証を効かせる。
fn synth_frame(event_idx: u32, asad: u8, fill: u8) -> Vec<u8> {
    let mut b = vec![fill; 96];
    b[0] = 0x00; // blkSize=2^0=1、big-endian
    b[1] = 0;
    b[2] = 0;
    b[3] = 96; // frameSize = 96 blocks × 1 B
    b[5] = 0;
    b[6] = 1; // frameType 1
    b[22..26].copy_from_slice(&event_idx.to_be_bytes());
    b[26] = 0;
    b[27] = asad;
    b
}

/// 12 B の非 AsAd 制御フレーム(実 2025 run 先頭の frameType 7 と同型)。
fn synth_ctrl_frame(fill: u8) -> Vec<u8> {
    let mut b = vec![fill; 12];
    b[0] = 0x00;
    b[1] = 0;
    b[2] = 0;
    b[3] = 12;
    b[5] = 0;
    b[6] = 7; // frameType 7(eventIdx を持たない)
    b
}

/// 複数ファイル指定 = eventIdx 昇順・同 idx 内は引数順のインターリーブ送出であること。
/// 制御フレームは遭遇時に即時送出。ファイル長の不揃い(末尾の余りイベント)も流れ切ること。
/// 期待列は手組み: [C.ctrl, A0, B0, C0, A1, B1, C1, A2, B2, C2, A3](B/C は 3 イベントで枯渇)。
#[test]
fn multiple_files_are_interleaved_by_event_idx() {
    // A(asad 0)= events 0..=3、B(asad 1)= 0..=2、C(asad 2)= 先頭 ctrl + 0..=2
    let a: Vec<Vec<u8>> = (0..4).map(|e| synth_frame(e, 0, 0xA0)).collect();
    let b: Vec<Vec<u8>> = (0..3).map(|e| synth_frame(e, 1, 0xB0)).collect();
    let mut c: Vec<Vec<u8>> = vec![synth_ctrl_frame(0xCC)];
    c.extend((0..3).map(|e| synth_frame(e, 2, 0xC0)));

    let mut paths = Vec::new();
    for (tag, frames) in [("ma", &a), ("mb", &b), ("mc", &c)] {
        let path = temp_path(tag);
        std::fs::write(&path, frames.concat()).expect("write fixture");
        paths.push(path);
    }

    // 手組みの期待列(マージ規則そのもの): ctrl 最優先 → (eventIdx, 引数順) 昇順。
    let expected: Vec<u8> = [
        &c[0], // ctrl(C の先頭に居るので最初の 1 手で出る)
        &a[0], &b[0], &c[1], // event 0
        &a[1], &b[1], &c[2], // event 1
        &a[2], &b[2], &c[3], // event 2
        &a[3], // event 3(A のみ)
    ]
    .iter()
    .flat_map(|f| f.iter().copied())
    .collect();

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let addr = listener.local_addr().expect("local_addr");

    let mut child = Command::new(bin())
        .arg(addr.to_string())
        .args(paths.iter().map(|p| p.as_os_str().to_owned()))
        .spawn()
        .expect("spawn graw_replay (merged)");

    let (mut sock, _peer) = listener.accept().expect("accept connection");
    let mut received = Vec::new();
    sock.read_to_end(&mut received).expect("read until close");

    let status = child.wait().expect("wait for graw_replay");
    assert!(
        status.success(),
        "merged replay must exit 0, got {status:?}"
    );
    assert_eq!(
        received.len(),
        expected.len(),
        "merged stream must carry every byte of every input file"
    );
    assert_eq!(
        received, expected,
        "merged stream must be the exact eventIdx-ascending interleave"
    );

    for p in &paths {
        let _ = std::fs::remove_file(p);
    }
}

/// 引数不備は明確なエラー + 非 0 exit であること(TODO/005: 「接続失敗・途中切断は明確な
/// エラー + 非 0 exit」の一環として、まず引数レベルの誤りを確認しておく)。
#[test]
fn missing_arguments_exit_non_zero_with_usage_message() {
    let output = Command::new(bin())
        .output()
        .expect("run graw_replay with no args");

    assert!(
        !output.status.success(),
        "graw_replay with no arguments must exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.to_lowercase().contains("usage"),
        "stderr should show a usage message, got: {stderr}"
    );
}

/// 接続できない場合(誰も listen していないポート)は明確なエラー + 非 0 exit であること。
#[test]
fn connect_failure_exits_non_zero_with_clear_message() {
    // listen だけして accept しない listener を潰してポートを解放し、直後にそこへ繋ぎに行かせる。
    // 127.0.0.1 上の未使用ポートへ繋ぎに行けば ECONNREFUSED になる想定。
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let addr = listener.local_addr().expect("local_addr");
    drop(listener); // listen をやめてポートを空ける

    let file_len = 16usize;
    let content = synth_bytes(file_len);
    let path = temp_path("connect_fail");
    std::fs::write(&path, &content).expect("write fixture file");

    let output = Command::new(bin())
        .arg(addr.to_string())
        .arg(&path)
        .output()
        .expect("run graw_replay against a closed port");

    let _ = std::fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "graw_replay must exit non-zero when it cannot connect"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.trim().is_empty(),
        "graw_replay must print a clear error to stderr on connect failure"
    );
}
