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
