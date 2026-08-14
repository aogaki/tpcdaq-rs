//! root-sink のモニタ PUB(C++、TODO/022)の実配線試験。SPEC §5.1〜5.3 / §6.5 / §12-9。
//!
//! Rust から **本番エンコーダ**で作った Fragments を PUSH で流し込み、root_sink が
//! PUB(47004)へ出すものを SUB で受けて rmp-serde の **named struct** で読む。
//! ここで使うパーサは **023(monitor コンポーネント)の本番パーサの先行形**でもある ——
//! SPEC §5.3 の map 形式(自己記述)がそのまま構造体で受かることを、ここで固定する。
//!
//! 主張すること:
//!   * status は run 外でも 1 Hz で流れる(§5.3)。値は投入した数と一致する。
//!   * hist_snapshot のビンが **Rust 側で独立に再計算した期待値と全一致**
//!     (§4.5 と同じ二重実装照合 —— C++ 実装の写しではなく SPEC §5.2 から書き起こす)。
//!   * built_event が §2.4 の Fragment として読み戻せて、間引きは publish_drops に出る。
//!   * `--snapshot-hz 0` で hist_snapshot が止まっても status は続く。
//!   * EOS から 10 秒以内に `run{run:04}_monitor.root` が出来る(R10 = §12-9)。
//!
//! **env `TPCDAQ_ROOT_SINK_BIN` 未設定なら skip**(C++ ビルドを cargo test の前提に
//! しない —— root_sink_intake.rs と同じ流儀):
//!
//! ```text
//! make -C tools/root_sink
//! TPCDAQ_ROOT_SINK_BIN=$PWD/tools/root_sink/root_sink cargo test --test root_sink_monitor_pub
//! ```

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::io::Read;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant, SystemTime};

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_bytes::ByteBuf;

use tpcdaq::geometry::{self, ChannelRole, Geometry};
use tpcdaq::msg::{self, Batch, Fragment, Fragments, Message};

/// root-sink の期待ソース(SPEC §3.2: decoder = 100)。
const DECODER_SOURCE_ID: u32 = 100;
/// root-sink のモニタ PUB の source_id(SPEC §3.2 v1.9)。
const MONITOR_SOURCE_ID: u32 = 101;
const RUN_NUMBER: u32 = 7;

/// 合成ジオメトリ(リポ内フィクスチャ。実 .dat は入れない — CLAUDE.md)。
/// U 最大 4 / V 最大 3 / W 最大 4、AUX 2 本、cobo0・asad0 のみ。
const GEOMETRY: &str = "tests/fixtures/geometry_mini_reduced.dat";

// ---------------------------------------------------------------------
// SPEC §5.3 の payload(rmp-serde の named struct = 023 本番パーサの先行形)
// ---------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct KindProbe {
    kind: String,
}

#[derive(Debug, Deserialize)]
struct SaturationPayload {
    saturated: u64,
    counted: u64,
}

#[derive(Debug, Deserialize)]
struct StatusPayload {
    kind: String,
    run: u32,
    state: String,
    events_built: u64,
    events_incomplete: u64,
    late_fragments: u64,
    frames_per_cobo: BTreeMap<String, u64>,
    bytes_written: u64,
    saturation: BTreeMap<String, SaturationPayload>,
    publish_drops: u64,
}

#[derive(Debug, Deserialize)]
struct HistPayload {
    id: u8,
    name: String,
    nx: u32,
    ny: u32,
    /// f64 LE の生バイト列(長さ nx*ny*8)。
    bins: ByteBuf,
}

impl HistPayload {
    fn values(&self) -> Vec<f64> {
        assert_eq!(
            self.bins.len(),
            (self.nx as usize) * (self.ny as usize) * 8,
            "hist {} の bins 長が nx*ny*8 でない",
            self.name
        );
        self.bins
            .chunks_exact(8)
            .map(|c| f64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
            .collect()
    }
}

#[derive(Debug, Deserialize)]
struct HistSnapshotPayload {
    kind: String,
    run: u32,
    hists: Vec<HistPayload>,
}

#[derive(Debug, Deserialize)]
struct BuiltEventPayload {
    kind: String,
    run: u32,
    event_idx: u32,
    complete: bool,
    fragments: Vec<Fragment>,
}

/// 受け取った 1 通(エンベロープ = SPEC §2.2 + payload の kind + 生バイト)。
struct Received {
    source_id: u32,
    run: u32,
    seq: u64,
    created_ns: u64,
    kind: String,
    raw: Vec<u8>,
}

impl Received {
    fn parse<T: DeserializeOwned>(&self) -> T {
        match Message::<T>::from_msgpack(&self.raw) {
            Ok(Message::Data(batch)) => batch.payload,
            other => panic!(
                "payload をその型で読めない: {other:?}",
                other = other.is_ok()
            ),
        }
    }
}

fn decode_received(raw: Vec<u8>) -> Received {
    let Ok(Message::Data(batch)) = Message::<KindProbe>::from_msgpack(&raw) else {
        panic!(
            "モニタ PUB の 1 通が §2.2 の Data エンベロープでない ({} B)",
            raw.len()
        );
    };
    Received {
        source_id: batch.source_id,
        run: batch.run_number,
        seq: batch.sequence_number,
        created_ns: batch.created_ns,
        kind: batch.payload.kind,
        raw,
    }
}

// ---------------------------------------------------------------------
// skip ゲートとプロセス操作(root_sink_intake.rs の流儀)
// ---------------------------------------------------------------------

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

/// 起動した root_sink。**Drop で必ず殺す** —— assert で落ちたテストが子プロセスを
/// 置き去りにすると、継承した stderr パイプが閉じず `cargo test` ごと固まる
/// (root_sink_intake.rs にはこの守りが無い。ここでは新規なので入れてある)。
struct Sink {
    child: Child,
    pid: u32,
}

impl Drop for Sink {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

impl Sink {
    /// `--pub` は**必ず空きポート**を渡す(既定の 47004 を掴むと並列テストが互いを弾く)。
    fn spawn(bin: &Path, data_ep: &str, pub_ep: &str, extra: &[&str]) -> Sink {
        let child = Command::new(bin)
            .arg("--bind")
            .arg(data_ep)
            .arg("--pub")
            .arg(pub_ep)
            .args(extra)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn root_sink");
        let pid = child.id();
        Sink { child, pid }
    }

    /// SIGTERM → 終了待ち → 終了時の JSON 1 行。
    ///
    /// **libc を依存に足さない**ため kill(1) を呼ぶ(`Child::kill` は SIGKILL なので
    /// JSON を出す前に死んでしまい使えない)—— root_sink_intake.rs と同じ。
    fn terminate(&mut self, timeout: Duration) -> serde_json::Value {
        let status = Command::new("kill")
            .arg("-TERM")
            .arg(self.pid.to_string())
            .status()
            .expect("run kill(1)");
        assert!(status.success(), "kill -TERM {} failed", self.pid);
        let exit = self.wait_for_exit(timeout);
        assert!(
            exit.success(),
            "SIGTERM は正常終了(exit 0)であるべき: {exit:?}"
        );
        let mut out = String::new();
        self.child
            .stdout
            .take()
            .expect("piped stdout")
            .read_to_string(&mut out)
            .expect("read stdout");
        let line = out.lines().last().unwrap_or_default();
        serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("stdout is not one JSON line: {out:?} ({e})"))
    }

    fn wait_for_exit(&mut self, timeout: Duration) -> ExitStatus {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait().expect("try_wait") {
                return status;
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                panic!("root_sink did not exit within {timeout:?}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn count(counts: &serde_json::Value, key: &str) -> u64 {
    counts[key]
        .as_u64()
        .unwrap_or_else(|| panic!("counter {key} missing in {counts}"))
}

fn connect_push(ctx: &zmq::Context, endpoint: &str) -> zmq::Socket {
    let push = ctx.socket(zmq::PUSH).expect("PUSH socket");
    push.set_sndhwm(100).expect("set sndhwm");
    push.set_sndtimeo(15_000).expect("set sndtimeo");
    push.set_linger(2_000).expect("set linger");
    push.connect(endpoint).expect("connect PUSH");
    push
}

/// SUB を張って**購読が届くまで待つ**(slow joiner: connect 直後は購読が届いていない)。
fn connect_sub(ctx: &zmq::Context, endpoint: &str) -> zmq::Socket {
    let sub = ctx.socket(zmq::SUB).expect("SUB socket");
    sub.set_subscribe(b"").expect("subscribe all"); // トピックフレームなし(SPEC §2.1)
    sub.set_rcvtimeo(200).expect("set rcvtimeo");
    sub.set_linger(0).expect("set linger");
    sub.connect(endpoint).expect("connect SUB");
    std::thread::sleep(Duration::from_millis(300));
    sub
}

/// `until` まで受け続ける。
fn collect_until(sub: &zmq::Socket, until: Instant) -> Vec<Received> {
    let mut out = Vec::new();
    while Instant::now() < until {
        match sub.recv_bytes(0) {
            Ok(raw) => out.push(decode_received(raw)),
            Err(_) => continue, // EAGAIN(rcvtimeo)
        }
    }
    out
}

fn of_kind<'a>(msgs: &'a [Received], kind: &str) -> Vec<&'a Received> {
    msgs.iter().filter(|m| m.kind == kind).collect()
}

// ---------------------------------------------------------------------
// 投入するメッセージ(本番エンコーダ)
// ---------------------------------------------------------------------

/// 1 サンプル = (aget, raw_ch, bucket, adc)。
type Sample = (u8, u8, u16, u16);

fn fragment_of(event_idx: u32, samples: &[Sample]) -> Fragment {
    let words: Vec<u32> = samples
        .iter()
        .map(|(aget, chan, bucket, adc)| {
            msg::pack_item(*aget, *chan, *bucket, *adc).expect("pack_item within range")
        })
        .collect();
    Fragment {
        event_idx,
        event_time: 0x0000_1234_5678_9abc + u64::from(event_idx),
        cobo: 0,
        asad: 0,
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

// ---------------------------------------------------------------------
// 期待ヒストの独立再計算(SPEC §5.2 v1.9 から書き起こす = 二重実装)
// ---------------------------------------------------------------------

const BUCKETS: usize = 512;
const CHARGE_BINS: usize = 512;
const CHARGE_BIN_WIDTH: u32 = 8;
const SATURATION_ADC: u16 = 4095;

struct Expected {
    /// 添字 0..8 = StripTime{U,V,W} / Charge{U,V,W} / ChargeMax{U,V,W}(SPEC §5.2 の順)。
    bins: Vec<Vec<f64>>,
    saturated: [u64; 3],
    counted: [u64; 3],
    /// Strip 割り付けチャンネルの生 ADC 総和(StripTime 3 枚の総和と一致するはず)。
    strip_adc_total: f64,
}

/// 1 イベント = フラグメントの列。SPEC §5.2 のフィル規則をそのまま素直に書く。
fn recompute(geo: &Geometry, events: &[Vec<Fragment>]) -> Expected {
    let nstrip = [
        geo.max_strip[0] as usize,
        geo.max_strip[1] as usize,
        geo.max_strip[2] as usize,
    ];
    let mut bins: Vec<Vec<f64>> = Vec::new();
    for n in nstrip {
        bins.push(vec![0.0; n * BUCKETS]);
    }
    for _ in 0..6 {
        bins.push(vec![0.0; CHARGE_BINS]);
    }
    let mut saturated = [0u64; 3];
    let mut counted = [0u64; 3];
    let mut strip_adc_total = 0.0f64;

    for fragments in events {
        // 面毎の最大波高(ChargeMax 用。イベント単位)。
        let mut plane_peak: [i32; 3] = [-1, -1, -1];
        for f in fragments {
            // チャンネル毎の波高(そのイベントでサンプル ≥1 のものだけ)。
            let mut peak = [[-1i32; 68]; 4];
            for word in msg::items_from_bytes(&f.items).expect("items") {
                let item = msg::unpack_item(word);
                if item.chan as usize >= 68 {
                    continue;
                }
                let role = geo.lookup(
                    u32::from(f.cobo),
                    u32::from(f.asad),
                    u32::from(item.aget),
                    u32::from(item.chan),
                );
                let ChannelRole::Strip { plane, strip, .. } = role else {
                    continue; // FPN / AUX / 未記載はヒストに入れない
                };
                let p = plane as usize;
                bins[p][(strip as usize - 1) * BUCKETS + item.bucket as usize] +=
                    f64::from(item.adc);
                strip_adc_total += f64::from(item.adc);
                let slot = &mut peak[item.aget as usize][item.chan as usize];
                if i32::from(item.adc) > *slot {
                    *slot = i32::from(item.adc);
                }
            }
            for aget in 0..4u32 {
                for chan in 0..68u32 {
                    let pk = peak[aget as usize][chan as usize];
                    if pk < 0 {
                        continue;
                    }
                    let ChannelRole::Strip { plane, .. } =
                        geo.lookup(u32::from(f.cobo), u32::from(f.asad), aget, chan)
                    else {
                        continue;
                    };
                    let p = plane as usize;
                    counted[p] += 1;
                    if pk >= i32::from(SATURATION_ADC) {
                        saturated[p] += 1;
                    }
                    bins[3 + p][(pk as u32 / CHARGE_BIN_WIDTH) as usize] += 1.0;
                    if pk > plane_peak[p] {
                        plane_peak[p] = pk;
                    }
                }
            }
        }
        for p in 0..3 {
            if plane_peak[p] < 0 {
                continue; // 計数チャンネルが 1 本も無い面には入れない
            }
            bins[6 + p][(plane_peak[p] as u32 / CHARGE_BIN_WIDTH) as usize] += 1.0;
        }
    }

    Expected {
        bins,
        saturated,
        counted,
        strip_adc_total,
    }
}

const HIST_NAMES: [&str; 9] = [
    "StripTimeU",
    "StripTimeV",
    "StripTimeW",
    "ChargeU",
    "ChargeV",
    "ChargeW",
    "ChargeMaxU",
    "ChargeMaxV",
    "ChargeMaxW",
];

/// スナップショット 1 通と期待値を全ビン照合する。
fn assert_snapshot_matches(snap: &HistSnapshotPayload, expected: &Expected, geo: &Geometry) {
    assert_eq!(snap.hists.len(), 9, "9 枚(SPEC §5.2)");
    for (i, h) in snap.hists.iter().enumerate() {
        assert_eq!(h.id, (i + 1) as u8, "id は 1..9(表の順)");
        assert_eq!(h.name, HIST_NAMES[i], "名前は SPEC §5.2 の表そのもの");
        if i < 3 {
            assert_eq!(h.nx, u32::from(geo.max_strip[i]), "nx = Nstrip({})", h.name);
            assert_eq!(h.ny, BUCKETS as u32, "2D の ny = 512");
        } else {
            assert_eq!(h.nx, CHARGE_BINS as u32, "1D は 512 ビン");
            assert_eq!(h.ny, 1, "1D の ny = 1");
        }
        let got = h.values();
        assert_eq!(
            got.len(),
            expected.bins[i].len(),
            "{} のビン数が期待と違う",
            h.name
        );
        for (b, (g, e)) in got.iter().zip(expected.bins[i].iter()).enumerate() {
            assert_eq!(g, e, "{} のビン {b} が違う(独立再計算との不一致)", h.name);
        }
    }
}

// ---------------------------------------------------------------------
// 1. status は run 外でも流れる(SPEC §5.3「1 Hz、run 外でも常時」)
// ---------------------------------------------------------------------

#[test]
fn status_is_published_at_one_hertz_even_while_idle() {
    let Some(bin) = sink_bin() else {
        eprintln!("SKIP: TPCDAQ_ROOT_SINK_BIN not set (C++ build is not a cargo test dependency)");
        return;
    };
    let data_ep = free_endpoint();
    let pub_ep = free_endpoint();
    let mut sink = Sink::spawn(
        &bin,
        &data_ep,
        &pub_ep,
        &["--no-root", "--geometry", GEOMETRY],
    );

    let ctx = zmq::Context::new();
    let sub = connect_sub(&ctx, &pub_ep);
    // データを 1 通も送らずに 2.5 秒покой —— それでも status は来る。
    let msgs = collect_until(&sub, Instant::now() + Duration::from_millis(2500));

    let statuses = of_kind(&msgs, "status");
    assert!(
        statuses.len() >= 2,
        "idle でも 1 Hz で status が来るはず(got {} 通 / 全 {} 通)",
        statuses.len(),
        msgs.len()
    );
    for m in &statuses {
        assert_eq!(m.source_id, MONITOR_SOURCE_ID, "source_id = 101(SPEC §3.2)");
        assert!(
            m.created_ns > 1_600_000_000_000_000_000,
            "created_ns が unix ns でない"
        );
    }
    let s: StatusPayload = statuses.last().unwrap().parse();
    assert_eq!(s.kind, "status");
    assert_eq!(s.state, "idle", "run が開いていないので idle");
    assert_eq!(s.run, 0, "まだ run が無いので 0");
    assert_eq!(s.events_built, 0);
    assert_eq!(s.events_incomplete, 0);
    assert_eq!(s.late_fragments, 0);
    assert_eq!(s.bytes_written, 0, "--no-root なので 0");
    assert_eq!(s.publish_drops, 0);
    assert!(s.frames_per_cobo.is_empty(), "1 フレームも来ていない");
    for plane in ["U", "V", "W"] {
        let sat = s.saturation.get(plane).expect("saturation に U/V/W が要る");
        assert_eq!((sat.saturated, sat.counted), (0, 0), "{plane} 面");
    }

    // seq は全 kind 単一系列で単調増加(SPEC §5.3 v1.9 ③)
    let mut last = None;
    for m in &msgs {
        if let Some(prev) = last {
            assert!(m.seq > prev, "seq が単調でない: {prev} -> {}", m.seq);
        }
        last = Some(m.seq);
    }

    let counts = sink.terminate(Duration::from_secs(10));
    assert_eq!(counts["fatal"], "", "counts={counts}");
    assert_eq!(
        count(&counts, "pub_bind_failed"),
        0,
        "PUB が bind できていない"
    );
    assert!(
        count(&counts, "snapshots_published") >= 2,
        "counts={counts}"
    );
}

// ---------------------------------------------------------------------
// 2. EOS 後の hist_snapshot が独立再計算と全一致(§4.5 と同じ二重実装照合)
// ---------------------------------------------------------------------

/// 投入イベント(1 イベント = 1 フラグメント = 既定の期待集合 (0,0))。
///
/// 手計算の出典(フィクスチャ tests/fixtures/geometry_mini_reduced.dat):
///   aget0 raw0..3 = U strip1..4 / aget1 raw0..2 = V strip1..3 / aget1 raw3..6 = W strip1..4
///   aget0 raw4 と aget1 raw7 = AUX、raw{11,22,45,56} = FPN(どちらもヒストに入らない)
///
///   event 0: U strip1 に 100(b5)と 250(b6)、W strip1 に 4095(b2、飽和)、
///            AUX 900 と FPN 800(除外されるので総和が動かないこと自体がテスト)
///   event 1: V strip1 に 40(b0)、V strip2 に 41(b1)
///   event 2: U strip4 に 4095(b9、飽和)、W strip4 に 7(b9)
fn synthetic_events() -> Vec<Vec<Fragment>> {
    vec![
        vec![fragment_of(
            0,
            &[
                (0, 0, 5, 100),
                (0, 0, 6, 250),
                (1, 3, 2, 4095),
                (0, 4, 2, 900),
                (0, 11, 2, 800),
            ],
        )],
        vec![fragment_of(1, &[(1, 0, 0, 40), (1, 1, 1, 41)])],
        vec![fragment_of(2, &[(0, 3, 9, 4095), (1, 6, 9, 7)])],
    ]
}

#[test]
fn hist_snapshot_bins_match_an_independent_recomputation() {
    let Some(bin) = sink_bin() else {
        eprintln!("SKIP: TPCDAQ_ROOT_SINK_BIN not set");
        return;
    };
    let data_ep = free_endpoint();
    let pub_ep = free_endpoint();
    let mut sink = Sink::spawn(
        &bin,
        &data_ep,
        &pub_ep,
        &["--no-root", "--geometry", GEOMETRY],
    );

    let ctx = zmq::Context::new();
    let sub = connect_sub(&ctx, &pub_ep);
    let push = connect_push(&ctx, &data_ep);

    let events = synthetic_events();
    for (seq, fragments) in events.iter().enumerate() {
        push.send(batch_of(seq as u64, fragments.clone()), 0)
            .expect("send Data batch");
    }
    push.send(end_of_stream(), 0).expect("send EndOfStream");

    // EOS 後のスナップショットを 2 通は見たい(1 Hz)。
    let msgs = collect_until(&sub, Instant::now() + Duration::from_millis(2500));

    let geo = geometry::load(GEOMETRY).expect("load geometry fixture");
    let expected = recompute(&geo, &events);

    // --- 手計算の抜き取り(この 3 つが合わないなら上の再計算自体が疑わしい)---
    assert_eq!(expected.bins[0][5], 100.0, "StripTimeU strip1 bucket5");
    assert_eq!(expected.bins[0][6], 250.0, "StripTimeU strip1 bucket6");
    assert_eq!(
        expected.bins[0][3 * BUCKETS + 9],
        4095.0,
        "StripTimeU strip4 bucket9"
    );
    // 計数チャンネル(そのイベントでサンプル ≥1 の Strip ch):
    //   U = event0 の (aget0,raw0) + event2 の (aget0,raw3)              = 2
    //   V = event1 の (aget1,raw0) + (aget1,raw1)                        = 2
    //   W = event0 の (aget1,raw3) + event2 の (aget1,raw6)              = 2
    // 飽和(波高 ≥ 4095): U = event2 の 4095、W = event0 の 4095、V は無し。
    assert_eq!(expected.counted, [2, 2, 2], "計数チャンネル数 U2/V2/W2");
    assert_eq!(expected.saturated, [1, 0, 1], "飽和 U1/V0/W1");
    // ChargeU は 250→bin31、4095→bin511 の 2 エントリ(1 ch 1 エントリ)。
    assert_eq!(expected.bins[3][31], 1.0, "ChargeU bin31 = 250/8");
    assert_eq!(expected.bins[3][511], 1.0, "ChargeU bin511 = 4095/8");
    // ChargeMaxU はイベント毎に 1 エントリ(event0 の 250、event2 の 4095)。
    assert_eq!(expected.bins[6][31], 1.0, "ChargeMaxU bin31");
    assert_eq!(expected.bins[6][511], 1.0, "ChargeMaxU bin511");

    let snapshots = of_kind(&msgs, "hist_snapshot");
    assert!(
        snapshots.len() >= 2,
        "hist_snapshot が来ていない(got {})",
        snapshots.len()
    );
    let last = snapshots.last().unwrap();
    let snap: HistSnapshotPayload = last.parse();
    assert_eq!(snap.kind, "hist_snapshot");
    assert_eq!(snap.run, RUN_NUMBER, "EOS 後は直近クローズした run が載る");
    assert_snapshot_matches(&snap, &expected, &geo);

    // status も同じ数を主張していること
    let statuses = of_kind(&msgs, "status");
    let s: StatusPayload = statuses.last().unwrap().parse();
    assert_eq!(s.state, "idle", "EOS で run は閉じている");
    assert_eq!(s.run, RUN_NUMBER);
    assert_eq!(s.events_built, 3);
    assert_eq!(s.events_incomplete, 0);
    assert_eq!(s.late_fragments, 0);
    assert_eq!(
        s.frames_per_cobo.get("0").copied(),
        Some(3),
        "CoBo 0 に 3 フレーム"
    );
    for (i, plane) in ["U", "V", "W"].iter().enumerate() {
        let sat = s.saturation.get(*plane).unwrap();
        assert_eq!(sat.saturated, expected.saturated[i], "{plane} 面の飽和分子");
        assert_eq!(sat.counted, expected.counted[i], "{plane} 面の飽和分母");
    }

    let counts = sink.terminate(Duration::from_secs(10));
    assert_eq!(count(&counts, "events_complete"), 3, "counts={counts}");
    assert_eq!(count(&counts, "runs"), 1, "counts={counts}");
    assert_eq!(counts["fatal"], "", "counts={counts}");
}

// ---------------------------------------------------------------------
// 3. built_event(最新優先 + 間引きの可視化)
// ---------------------------------------------------------------------

#[test]
fn built_events_are_published_and_throttling_is_counted() {
    let Some(bin) = sink_bin() else {
        eprintln!("SKIP: TPCDAQ_ROOT_SINK_BIN not set");
        return;
    };
    let data_ep = free_endpoint();
    let pub_ep = free_endpoint();
    // 送出は速く・配信は 2 Hz —— 間引きが必ず起きる設定(publish_drops > 0 を主張する)。
    let mut sink = Sink::spawn(
        &bin,
        &data_ep,
        &pub_ep,
        &[
            "--no-root",
            "--geometry",
            GEOMETRY,
            "--snapshot-hz",
            "0",
            "--event-publish-hz",
            "2",
        ],
    );

    let ctx = zmq::Context::new();
    let sub = connect_sub(&ctx, &pub_ep);
    let push = connect_push(&ctx, &data_ep);

    // 3 秒かけて 300 イベント(5 イベント/バッチ × 60 バッチ、50 ms 間隔)。
    let mut event_idx = 0u32;
    let collector = std::thread::spawn(move || {
        collect_until(&sub, Instant::now() + Duration::from_millis(3600))
    });
    for seq in 0..60u64 {
        let payload: Fragments = (0..5)
            .map(|_| {
                let f = fragment_of(event_idx, &[(0, 0, 3, 100 + (event_idx % 100) as u16)]);
                event_idx += 1;
                f
            })
            .collect();
        push.send(batch_of(seq, payload), 0).expect("send batch");
        std::thread::sleep(Duration::from_millis(50));
    }
    push.send(end_of_stream(), 0).expect("send EndOfStream");
    let msgs = collector.join().expect("collector thread");

    let events = of_kind(&msgs, "built_event");
    assert!(
        events.len() >= 2,
        "2 Hz なら 3.6 秒で 2 通以上は来るはず(got {})",
        events.len()
    );
    assert!(
        events.len() < 300,
        "間引かれずに全部来ている(最新優先になっていない): {}",
        events.len()
    );
    assert!(
        of_kind(&msgs, "hist_snapshot").is_empty(),
        "--snapshot-hz 0 なのに hist_snapshot が来ている"
    );

    let mut last_idx: Option<u32> = None;
    for m in &events {
        let ev: BuiltEventPayload = m.parse();
        assert_eq!(ev.kind, "built_event");
        assert_eq!(ev.run, RUN_NUMBER);
        assert_eq!(m.run, RUN_NUMBER, "エンベロープの run_number も載る");
        assert!(ev.complete, "期待集合 (0,0) は 1 フラグメントで complete");
        assert_eq!(ev.fragments.len(), 1, "mini = 1 イベント 1 フラグメント");
        // §2.4 の Fragment として読み戻せて、投入した値と一致する
        let f = &ev.fragments[0];
        assert_eq!(f.event_idx, ev.event_idx);
        assert_eq!((f.cobo, f.asad), (0, 0));
        assert_eq!(f.frame_type, 2);
        assert_eq!(f.revision, 5);
        assert_eq!(f.mult, [68, 0, 17, 3]);
        assert_eq!(f.last_cell, [7, 9, 11, 13]);
        let words = msg::items_from_bytes(&f.items).expect("items");
        assert_eq!(words.len(), 1);
        let item = msg::unpack_item(words[0]);
        assert_eq!((item.aget, item.chan, item.bucket), (0, 0, 3));
        assert_eq!(item.adc, 100 + (ev.event_idx % 100) as u16);
        if let Some(prev) = last_idx {
            assert!(
                ev.event_idx > prev,
                "event_idx が単調でない: {prev} -> {}",
                ev.event_idx
            );
        }
        last_idx = Some(ev.event_idx);
    }

    let counts = sink.terminate(Duration::from_secs(10));
    assert_eq!(count(&counts, "events_complete"), 300, "counts={counts}");
    assert_eq!(count(&counts, "snapshots_published"), 0, "counts={counts}");
    let published = count(&counts, "events_published");
    let drops = count(&counts, "publish_drops");
    assert!(published >= 2, "counts={counts}");
    assert!(
        drops > 0,
        "間引きが publish_drops に出ていない: counts={counts}"
    );
    assert_eq!(
        published + drops,
        300,
        "配信 + 間引き = 全イベント: counts={counts}"
    );
    assert_eq!(counts["fatal"], "", "counts={counts}");
}

// ---------------------------------------------------------------------
// 4. `--event-publish-hz 0` は「設定による停止」= ドロップとして数えない
// ---------------------------------------------------------------------

#[test]
fn event_publish_hz_zero_is_off_and_not_counted_as_drops() {
    let Some(bin) = sink_bin() else {
        eprintln!("SKIP: TPCDAQ_ROOT_SINK_BIN not set");
        return;
    };
    let data_ep = free_endpoint();
    let pub_ep = free_endpoint();
    let mut sink = Sink::spawn(
        &bin,
        &data_ep,
        &pub_ep,
        &[
            "--no-root",
            "--geometry",
            GEOMETRY,
            "--event-publish-hz",
            "0",
        ],
    );

    let ctx = zmq::Context::new();
    let sub = connect_sub(&ctx, &pub_ep);
    let push = connect_push(&ctx, &data_ep);
    for (seq, fragments) in synthetic_events().iter().enumerate() {
        push.send(batch_of(seq as u64, fragments.clone()), 0)
            .expect("send batch");
    }
    push.send(end_of_stream(), 0).expect("send EndOfStream");
    let msgs = collect_until(&sub, Instant::now() + Duration::from_millis(2200));

    assert!(
        of_kind(&msgs, "built_event").is_empty(),
        "--event-publish-hz 0 なのに built_event が来ている"
    );
    assert!(!of_kind(&msgs, "status").is_empty(), "status は続くはず");
    assert!(
        !of_kind(&msgs, "hist_snapshot").is_empty(),
        "hist_snapshot は既定の 1 Hz で続くはず"
    );

    let counts = sink.terminate(Duration::from_secs(10));
    assert_eq!(count(&counts, "events_published"), 0, "counts={counts}");
    // 設定による停止はドロップではない(発注書 §3)
    assert_eq!(count(&counts, "publish_drops"), 0, "counts={counts}");
}

// ---------------------------------------------------------------------
// 5. R10 — EOS から 10 秒以内に run{run:04}_monitor.root(SPEC §12-9 / §6.5)
// ---------------------------------------------------------------------

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tpcdaq_root_sink_monitor_{}_{tag}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

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

#[test]
fn monitor_root_is_written_within_ten_seconds_of_the_eos() {
    let Some(bin) = sink_bin() else {
        eprintln!("SKIP: TPCDAQ_ROOT_SINK_BIN not set");
        return;
    };
    let out_root = scratch_dir("r10");
    let data_ep = free_endpoint();
    let pub_ep = free_endpoint();
    // TTree 側は GDataFrame(1 エントリ = 1 フラグメント)—— 既存回帰と同じ形のまま、
    // モニタ経路が保存側に干渉していないことを見る。
    let mut sink = Sink::spawn(
        &bin,
        &data_ep,
        &pub_ep,
        &[
            "--format",
            "gdataframe",
            "--geometry",
            GEOMETRY,
            "--output-root",
            &out_root.to_string_lossy(),
        ],
    );

    let ctx = zmq::Context::new();
    let push = connect_push(&ctx, &data_ep);
    std::thread::sleep(Duration::from_millis(300)); // bind が済むまで
    let events = synthetic_events();
    for (seq, fragments) in events.iter().enumerate() {
        push.send(batch_of(seq as u64, fragments.clone()), 0)
            .expect("send batch");
    }
    let eos_wall = SystemTime::now();
    let eos_at = Instant::now();
    push.send(end_of_stream(), 0).expect("send EndOfStream");

    let run_dir = out_root.join(format!("run{RUN_NUMBER:04}"));
    let monitor_root = run_dir.join(format!("run{RUN_NUMBER:04}_monitor.root"));
    let deadline = eos_at + Duration::from_secs(10);
    while Instant::now() < deadline && !monitor_root.is_file() {
        std::thread::sleep(Duration::from_millis(50));
    }
    let elapsed = eos_at.elapsed();
    assert!(
        monitor_root.is_file(),
        "R10 違反: {} が EOS から 10 秒以内に出来ていない(dir={:?})",
        monitor_root.display(),
        names_starting_with(&run_dir, "")
    );
    // mtime − EOS 送信時刻 ≤ 10 s(§12-9 の数値化そのもの)
    let mtime = std::fs::metadata(&monitor_root)
        .expect("stat monitor.root")
        .modified()
        .expect("mtime");
    let lag = mtime
        .duration_since(eos_wall)
        .unwrap_or(Duration::from_secs(0));
    assert!(
        lag <= Duration::from_secs(10),
        "mtime − EOS = {lag:?} が 10 秒を超えている"
    );
    eprintln!("R10: monitor.root は EOS から {elapsed:?}(mtime 差 {lag:?})");

    // --- run.root(TTree)側の既存回帰が無影響 ---
    let run_file = run_dir.join(format!("run{RUN_NUMBER:04}.root"));
    assert!(run_file.is_file(), "{} が無い", run_file.display());
    assert!(
        names_starting_with(&run_dir, "run_inprogress_").is_empty(),
        "inprogress が残っている: {:?}",
        names_starting_with(&run_dir, "run_inprogress_")
    );
    assert!(monitor_root.metadata().expect("stat").len() > 0);

    let counts = sink.terminate(Duration::from_secs(20));
    // 1 エントリ = 1 フラグメント(SPEC §6.4 の GDataFrame 回帰)。モニタ経路が
    // 保存側の数を 1 つも変えていないこと。
    assert_eq!(count(&counts, "fragments"), 3, "counts={counts}");
    assert_eq!(count(&counts, "entries_written"), 3, "counts={counts}");
    assert_eq!(count(&counts, "runs"), 1, "counts={counts}");
    assert_eq!(counts["fatal"], "", "counts={counts}");
    // root_files 台帳は run.root だけ(monitor.root は保存系の台帳に混ぜない)
    let files = counts["root_files"].as_array().expect("root_files");
    assert_eq!(files.len(), 1, "counts={counts}");
    assert!(files[0]["path"]
        .as_str()
        .unwrap()
        .ends_with(&format!("run{RUN_NUMBER:04}.root")));

    let _ = std::fs::remove_dir_all(&out_root);
}

// ---------------------------------------------------------------------
// 6. 実データ E2E(env `TPCDAQ_REAL_GRAW` gate)
// ---------------------------------------------------------------------

/// 実 .graw をリプレイ(この場ではデコードして Fragments として投入)し、
/// **StripTime の総和 = Strip 割り付けチャンネルの生 ADC 総和**(Rust 側で独立計算)
/// と一致することを見る。ジオメトリはリポ内の合成フィクスチャ(実 .dat は入れない)。
///
/// P1 オラクル(SPEC §12-1): 実 .graw 1 本 = 108 イベント / 15,040,512 items。
#[test]
fn real_graw_hist_totals_match_an_independent_sum() {
    let Some(bin) = sink_bin() else {
        eprintln!("SKIP: TPCDAQ_ROOT_SINK_BIN not set");
        return;
    };
    let Ok(path) = std::env::var("TPCDAQ_REAL_GRAW") else {
        eprintln!("SKIP: TPCDAQ_REAL_GRAW が未設定(実 .graw はローカルのみ)");
        return;
    };

    // --- デコード(本番の framer + decoder)---
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
    let mut framer = tpcdaq::framer::Framer::new();
    let mut decoder = tpcdaq::decode::Decoder::new();
    let mut events: Vec<Vec<Fragment>> = Vec::new();
    for chunk in bytes.chunks(1 << 16) {
        framer.push(chunk);
        while let Some(frame) = framer.next() {
            if let Some(f) = decoder.decode(frame) {
                events.push(vec![f]); // mini = 1 CoBo × 1 AsAd = 1 イベント 1 フラグメント
            }
        }
    }
    assert_eq!(events.len(), 108, "P1 オラクル(events)");
    assert_eq!(decoder.items(), 15_040_512, "P1 オラクル(items)");

    let data_ep = free_endpoint();
    let pub_ep = free_endpoint();
    let mut sink = Sink::spawn(
        &bin,
        &data_ep,
        &pub_ep,
        &["--no-root", "--geometry", GEOMETRY],
    );

    let ctx = zmq::Context::new();
    let sub = connect_sub(&ctx, &pub_ep);
    let push = connect_push(&ctx, &data_ep);

    let started = Instant::now();
    for (seq, fragments) in events.iter().enumerate() {
        push.send(batch_of(seq as u64, fragments.clone()), 0)
            .expect("send batch");
    }
    push.send(end_of_stream(), 0).expect("send EndOfStream");
    let msgs = collect_until(&sub, Instant::now() + Duration::from_millis(4000));
    eprintln!("real graw replay + aggregate: {:?}", started.elapsed());

    // --- 独立計算(SPEC §5.2 のフィル規則を Rust で再現)---
    let geo = geometry::load(GEOMETRY).expect("load geometry fixture");
    let expected = recompute(&geo, &events);

    let snapshots = of_kind(&msgs, "hist_snapshot");
    assert!(!snapshots.is_empty(), "hist_snapshot が来ていない");
    let snap: HistSnapshotPayload = snapshots.last().unwrap().parse();
    assert_snapshot_matches(&snap, &expected, &geo);

    // StripTime 3 枚の総和 = Strip 割り付けチャンネルの生 ADC 総和
    let strip_time_total: f64 = snap.hists[0..3]
        .iter()
        .map(|h| h.values().iter().sum::<f64>())
        .sum();
    assert_eq!(
        strip_time_total, expected.strip_adc_total,
        "StripTime 総和が Strip チャンネルの生 ADC 総和と違う"
    );
    eprintln!("StripTime 総和 = {strip_time_total} (Strip 割り付け ch の生 ADC 総和と一致)");

    let statuses = of_kind(&msgs, "status");
    let s: StatusPayload = statuses.last().unwrap().parse();
    assert_eq!(s.events_built, 108, "events_built");
    assert_eq!(s.events_incomplete, 0);
    assert_eq!(s.late_fragments, 0);
    assert_eq!(s.frames_per_cobo.get("0").copied(), Some(108));

    let counts = sink.terminate(Duration::from_secs(30));
    assert_eq!(count(&counts, "fragments"), 108, "counts={counts}");
    assert_eq!(count(&counts, "items"), 15_040_512, "counts={counts}");
    assert_eq!(count(&counts, "events_complete"), 108, "counts={counts}");
    assert_eq!(counts["fatal"], "", "counts={counts}");
}

// ---------------------------------------------------------------------
// 7. 非干渉の計測ハーネス(SPEC §12-8 の素材。**自動テストにしない**)
// ---------------------------------------------------------------------
//
// 「スナップショット配信を止めても/倍にしても保存スループットが変わらない」を測るための
// 素材。`#[ignore]` なので `cargo test` では走らない —— 正式ゲートは P3 E2E。
//
//   TPCDAQ_ROOT_SINK_BIN=... TPCDAQ_REAL_GRAW=... TPCDAQ_BENCH_SNAPSHOT_HZ=0|1|2 \
//     cargo test --release --test root_sink_monitor_pub bench_storage -- --ignored --nocapture
//
// 測るのは「実 graw 4 周分(432 イベント ≈ 120 MB)を全速で投入し、run.root が
// finalize されるまで」の wall time。保存経路(TTree 書き出し)を含む。
#[test]
#[ignore]
fn bench_storage_throughput_vs_snapshot_rate() {
    let Some(bin) = sink_bin() else {
        eprintln!("SKIP: TPCDAQ_ROOT_SINK_BIN not set");
        return;
    };
    let Ok(path) = std::env::var("TPCDAQ_REAL_GRAW") else {
        eprintln!("SKIP: TPCDAQ_REAL_GRAW が未設定");
        return;
    };
    let hz = std::env::var("TPCDAQ_BENCH_SNAPSHOT_HZ").unwrap_or_else(|_| "1".to_string());
    const LOOPS: u32 = 4;

    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
    let mut framer = tpcdaq::framer::Framer::new();
    let mut decoder = tpcdaq::decode::Decoder::new();
    let mut base: Vec<Fragment> = Vec::new();
    for chunk in bytes.chunks(1 << 16) {
        framer.push(chunk);
        while let Some(frame) = framer.next() {
            if let Some(f) = decoder.decode(frame) {
                base.push(f);
            }
        }
    }
    // event_idx を周回ごとにずらす(ビルダは昇順 emit なので、同じ番号を繰り返せない)。
    let mut batches: Vec<Vec<u8>> = Vec::new();
    let mut seq = 0u64;
    for loop_idx in 0..LOOPS {
        for f in &base {
            let mut g = f.clone();
            g.event_idx += loop_idx * 1000;
            batches.push(batch_of(seq, vec![g]));
            seq += 1;
        }
    }
    let total_bytes: usize = batches.iter().map(|b| b.len()).sum();

    let out_root = scratch_dir(&format!("bench{hz}"));
    let data_ep = free_endpoint();
    let pub_ep = free_endpoint();
    let mut sink = Sink::spawn(
        &bin,
        &data_ep,
        &pub_ep,
        &[
            "--format",
            "gdataframe",
            "--geometry",
            GEOMETRY,
            "--output-root",
            &out_root.to_string_lossy(),
            "--snapshot-hz",
            &hz,
        ],
    );
    let ctx = zmq::Context::new();
    let push = connect_push(&ctx, &data_ep);
    std::thread::sleep(Duration::from_millis(400)); // bind が済むまで

    let started = Instant::now();
    for b in &batches {
        push.send(b.clone(), 0).expect("send batch");
    }
    push.send(end_of_stream(), 0).expect("send EndOfStream");
    let run_file = out_root
        .join(format!("run{RUN_NUMBER:04}"))
        .join(format!("run{RUN_NUMBER:04}.root"));
    let deadline = started + Duration::from_secs(300);
    while Instant::now() < deadline && !run_file.is_file() {
        std::thread::sleep(Duration::from_millis(10));
    }
    let elapsed = started.elapsed();
    assert!(run_file.is_file(), "run.root が出来ていない");

    let counts = sink.terminate(Duration::from_secs(60));
    assert_eq!(
        count(&counts, "fragments"),
        u64::from(LOOPS) * base.len() as u64,
        "counts={counts}"
    );
    assert_eq!(counts["fatal"], "", "counts={counts}");
    println!(
        "BENCH snapshot_hz={hz} wall={:.3}s wire={:.1}MB snapshots={} events_published={} publish_drops={}",
        elapsed.as_secs_f64(),
        total_bytes as f64 / 1e6,
        count(&counts, "snapshots_published"),
        count(&counts, "events_published"),
        count(&counts, "publish_drops"),
    );
    let _ = std::fs::remove_dir_all(&out_root);
}
