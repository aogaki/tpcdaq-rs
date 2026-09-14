//! null_sink —— root-sink 不在環境(ROOT が無い DAQ ホスト等)で decoder の PUSH を受ける
//! **ベンチ専用の PULL ドレイン**。受信メッセージを数えて捨てるだけで、保存はしない。
//!
//! 保存系の検証対象は graw-writer(受信バイト列そのまま)であり、本 bin は decoder →
//! root-sink のロスレスリンク(SPEC §1.2 ZMQ_IMMEDIATE + 有界 HWM)が下流不在で詰まって
//! receiver の内部キューを溢れさせる(§1.4-3 = Error)のを防ぐための代役。
//! 本番構成では root-sink がこの位置に立つ。
//!
//! 使い方: `null_sink [ENDPOINT]`(既定 `tcp://*:47003` = SPEC §3.2 root-sink PULL bind)。
use std::error::Error;
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn Error>> {
    let endpoint = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "tcp://*:47003".to_string());
    let ctx = zmq::Context::new();
    let sock = ctx.socket(zmq::PULL)?;
    sock.bind(&endpoint)?;
    eprintln!("null_sink: PULL bind {endpoint} (counting and discarding)");

    let (mut msgs, mut bytes) = (0u64, 0u64);
    let mut last_report = Instant::now();
    loop {
        let parts = sock.recv_multipart(0)?;
        msgs += 1;
        bytes += parts.iter().map(|p| p.len() as u64).sum::<u64>();
        if last_report.elapsed() >= Duration::from_secs(5) {
            eprintln!("null_sink: {msgs} msgs, {bytes} bytes");
            last_report = Instant::now();
        }
    }
}
