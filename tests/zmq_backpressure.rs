//! 有限 HWM の PUSH/PULL が背圧を掛けることの実証(SPEC §1.4-2、CLAUDE.md「保存系は
//! 絶対にデータを落とさない」)。
//!
//! delila-rs は HWM=0(無制限バッファ)だが、tpcdaq-rs は有限 HWM を選ぶ。有限だからこそ
//! 「受け手が止まると送り手が止まる」= ロスレスが成立する。ここではそれを実測する。
//!
//! sleep への依存は最小化し、条件待ちループ + タイムアウトで判定する。

#![allow(clippy::unwrap_used)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tpcdaq::zmq_helper;

/// 1 メッセージのサイズ。小さすぎると OS の TCP バッファに数千通吸われて
/// 「詰まらない」偽陰性になるため、あえて大きめにする。
const MSG_BYTES: usize = 256 * 1024;
/// テスト用の極小 HWM(既定 1000 だと詰まるまでに時間がかかるだけで、性質は同じ)。
const TEST_HWM: i32 = 2;
const TOTAL: usize = 64;

fn bind_pull(ctx: &zmq::Context, hwm: i32) -> (zmq::Socket, String) {
    let pull = ctx.socket(zmq::PULL).unwrap();
    pull.set_linger(0).unwrap();
    zmq_helper::apply_hwm(&pull, hwm).unwrap(); // HWM は bind 前に設定する必要がある
    pull.bind("tcp://127.0.0.1:0").unwrap();
    pull.set_rcvtimeo(5_000).unwrap();
    let endpoint = pull.get_last_endpoint().unwrap().unwrap();
    (pull, endpoint)
}

fn connect_push(ctx: &zmq::Context, endpoint: &str, hwm: i32) -> zmq::Socket {
    let push = ctx.socket(zmq::PUSH).unwrap();
    push.set_linger(0).unwrap();
    zmq_helper::apply_hwm(&push, hwm).unwrap();
    push.connect(endpoint).unwrap();
    push
}

fn wait_until(mut cond: impl FnMut() -> bool, timeout: Duration, what: &str) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if cond() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out after {timeout:?} waiting for {what}");
}

/// 有界キューであることの決定的な確認: 受け手が recv を止めると、非ブロッキング送信は
/// 有限回で EAGAIN になる(= 無制限バッファではない)。
#[test]
fn a_stalled_receiver_makes_nonblocking_send_report_eagain() {
    let ctx = zmq::Context::new();
    let (pull, endpoint) = bind_pull(&ctx, TEST_HWM);
    let push = connect_push(&ctx, &endpoint, TEST_HWM);

    // まずリンクが張れていることを確認する(未接続の EAGAIN と取り違えないため)
    push.send(vec![0u8; 16], 0).unwrap();
    assert_eq!(pull.recv_bytes(0).unwrap().len(), 16);

    let payload = vec![0xa5u8; MSG_BYTES];
    let mut accepted = 0usize;
    let mut hit_eagain = false;
    for _ in 0..200 {
        match push.send(&payload[..], zmq::DONTWAIT) {
            Ok(()) => accepted += 1,
            Err(zmq::Error::EAGAIN) => {
                hit_eagain = true;
                break;
            }
            Err(e) => panic!("unexpected send error: {e}"),
        }
    }
    assert!(
        hit_eagain,
        "受け手が止まっているのに {accepted} 通が無条件に受理された = 有界でない"
    );
}

/// ブロッキング送信版: 受け手が recv を止めると送り手の send が止まり、
/// 受信を再開すると解放されること。
#[test]
fn a_stalled_receiver_blocks_the_sender_until_it_drains() {
    let ctx = zmq::Context::new();
    let (pull, endpoint) = bind_pull(&ctx, TEST_HWM);
    let push = connect_push(&ctx, &endpoint, TEST_HWM);

    let sent = Arc::new(AtomicUsize::new(0));
    // 受信完了まで送り手にソケットを持たせ続けるための合図。linger=0 のソケットを
    // 落とすと未送出分が破棄されるため、送信ループを抜けた直後に drop させない。
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let sender = {
        let sent = Arc::clone(&sent);
        std::thread::spawn(move || {
            let payload = vec![0x5au8; MSG_BYTES];
            for _ in 0..TOTAL {
                push.send(&payload[..], 0).unwrap();
                sent.fetch_add(1, Ordering::SeqCst);
            }
            let _ = release_rx.recv();
        })
    };

    // 1 通目が出る = リンクは生きている
    wait_until(
        || sent.load(Ordering::SeqCst) > 0,
        Duration::from_secs(5),
        "the first send to complete",
    );

    // 送信数が止まる(= ブロックしている)ことを、全送信完了より前に観測する
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut stalled_at = None;
    while Instant::now() < deadline {
        let before = sent.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(150));
        let after = sent.load(Ordering::SeqCst);
        if before == after && after < TOTAL {
            stalled_at = Some(after);
            break;
        }
    }
    let stalled_at = stalled_at.expect("sender never blocked — the queue is not bounded");
    assert!(stalled_at < TOTAL);

    // 受信を再開すると解放される
    for _ in 0..TOTAL {
        assert_eq!(pull.recv_bytes(0).unwrap().len(), MSG_BYTES);
    }
    wait_until(
        || sent.load(Ordering::SeqCst) == TOTAL,
        Duration::from_secs(5),
        "the sender to be released after the receiver drained",
    );
    release_tx.send(()).unwrap();
    sender.join().unwrap();
}
