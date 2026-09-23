//! `calib pedestal` / `calib pulser`(TODO/081)の合成 .graw 回帰。
//!
//! 各テストが自前で MFM フレーム(frameType 2 = pedestal の実データ規約、frameType 1 = pulser
//! の実データ規約)を手組みして一時ファイルへ書き、`tpcdaq::calib::{pedestal,pulser}::compute`
//! を通す(既存 `tests/*.rs` の流儀 — フレームビルダは各テストファイルにローカルに持つ。
//! `src/decode.rs` のテストヘルパは private なので再利用できない)。
//!
//! merge の flags 分岐の網羅は `src/calib/table.rs` の単体テスト
//! (`merge_flags_six_channels_hit_every_branch`)で済ませてある(発注書「表 5 ch 程度の小さな
//! JSON で分岐を全部踏む」)。ここでは「pedestal/pulser を .graw から通して merge まで繋がる」
//! ことを確認する。

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;

use tpcdaq::calib::{pedestal, pulser, table};

// ---------------------------------------------------------------------
// フレームビルダ(blkSize=1、big-endian。src/decode.rs のテストと同じ規約)。
// ---------------------------------------------------------------------

const HEADER_BYTES: usize = 88;

fn write_uint(buf: &mut [u8], offset: usize, value: u64, n: usize) {
    for (i, b) in buf[offset..offset + n].iter_mut().enumerate() {
        *b = ((value >> (8 * (n - 1 - i))) & 0xFF) as u8;
    }
}

/// frameType 2(compact、2B item、pedestal の実データ規約)の CoBo フレームを手組みする。
/// `items` = (aget, adc) の生順序。ch/bucket は decoder が per-AGET カーソルで復元する
/// (`src/decode.rs` `decode_items` と同じ規約: aget 毎に ch 0..67 → bucket が進む)。
fn make_frame_type2(asad: u8, event_idx: u32, items: &[(u8, u16)]) -> Vec<u8> {
    let total = HEADER_BYTES + items.len() * 2;
    let mut b = vec![0u8; total];
    b[0] = 0x00; // big-endian, blkSize=1
    write_uint(&mut b, 1, total as u64, 3);
    write_uint(&mut b, 5, 2, 2); // frameType = 2
    write_uint(&mut b, 8, HEADER_BYTES as u64, 2); // headerSize(blk)
    write_uint(&mut b, 10, 2, 2); // itemSize
    write_uint(&mut b, 12, items.len() as u64, 4);
    write_uint(&mut b, 22, u64::from(event_idx), 4);
    b[27] = asad;
    for (i, &(aget, adc)) in items.iter().enumerate() {
        let w = (u16::from(aget & 0x3) << 14) | (adc & 0xFFF);
        write_uint(&mut b, HEADER_BYTES + i * 2, u64::from(w), 2);
    }
    b
}

/// frameType 1(4B item、明示 aget/chan/bucket/adc、pulser の実データ規約)の CoBo フレームを
/// 手組みする。
fn make_frame_type1(asad: u8, event_idx: u32, items: &[(u8, u8, u16, u16)]) -> Vec<u8> {
    let total = HEADER_BYTES + items.len() * 4;
    let mut b = vec![0u8; total];
    b[0] = 0x00;
    write_uint(&mut b, 1, total as u64, 3);
    write_uint(&mut b, 5, 1, 2); // frameType = 1
    write_uint(&mut b, 8, HEADER_BYTES as u64, 2);
    write_uint(&mut b, 10, 4, 2); // itemSize
    write_uint(&mut b, 12, items.len() as u64, 4);
    write_uint(&mut b, 22, u64::from(event_idx), 4);
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

// ---------------------------------------------------------------------
// 使い捨てスクラッチディレクトリ(新規依存を増やさない — `tempdir`/`tempfile` は Cargo.toml
// に無いので、`std::env::temp_dir` 直下に自前で作って Drop で消す)。
// ---------------------------------------------------------------------

struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "tpcdaq_calib_synthetic_{}_{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        Self(dir)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `frames` を連結して 1 つの .graw ファイルとして書き出す。
fn write_graw(dir: &ScratchDir, name: &str, frames: &[Vec<u8>]) -> PathBuf {
    let path = dir.path().join(name);
    let mut bytes = Vec::new();
    for f in frames {
        bytes.extend_from_slice(f);
    }
    std::fs::write(&path, bytes).unwrap();
    path
}

// ---------------------------------------------------------------------
// pedestal: 定数 ADC → mean = 定数、rms = 0(発注書のオラクル)+ 非対称データ + runs 集計。
// ---------------------------------------------------------------------

#[test]
fn pedestal_compute_matches_hand_calculation_across_two_files() {
    let dir = ScratchDir::new("pedestal");

    // ファイル A: aget0 の ch0..11 を敷き詰め、ch0/ch1 は 300、ch11(FPN)は 400、それ以外
    // (ch2..10)は検証しないダミー値。2 イベントとも同一値(全 bucket・全イベント一定 →
    // rms=0 のオラクル)。
    let items_a: Vec<(u8, u16)> = (0..12u16)
        .map(|c| {
            if c == 11 {
                (0u8, 400u16) // FPN
            } else if c < 2 {
                (0u8, 300u16) // ch0, ch1(信号)
            } else {
                (0u8, 999u16) // 検証しない ch
            }
        })
        .collect();
    let file_a = write_graw(
        &dir,
        "a.graw",
        &[
            make_frame_type2(0, 0, &items_a),
            make_frame_type2(0, 1, &items_a),
        ],
    );

    // ファイル B: 同じ asad0/aget0 の ch0 だけ、値を変えて非対称データにする。
    // 手計算: ch0 は全体で [300,300 (file A 2 events), 302,298 (file B 2 events)] の 4 サンプル。
    // mean=(300+300+302+298)/4=300, var=((0)^2+(0)^2+(2)^2+(-2)^2)/4=8/4=2, rms=sqrt(2)。
    let file_b = write_graw(
        &dir,
        "b.graw",
        &[
            make_frame_type2(0, 0, &[(0, 302)]),
            make_frame_type2(0, 1, &[(0, 298)]),
        ],
    );

    let computed = pedestal::compute(&[file_a, file_b]).unwrap();

    let find = |ch: u8| {
        computed
            .channels
            .iter()
            .find(|c| c.asad == 0 && c.aget == 0 && c.ch == ch)
            .unwrap_or_else(|| panic!("ch{ch} missing"))
    };

    // ch0: 4 サンプル(file A ×2 = 300,300、file B ×2 = 302,298)。手計算: mean=300, rms=sqrt(2)。
    let ch0 = find(0);
    assert!((ch0.mean - 300.0).abs() < 1e-9, "mean={}", ch0.mean);
    assert!((ch0.rms - 2.0f64.sqrt()).abs() < 1e-9, "rms={}", ch0.rms);
    assert_eq!(ch0.runs, 2, "ch0 は file A と file B の両方に出現する");
    assert!(!ch0.is_fpn);

    // ch1: file A のみ(2 イベント、定数 300)。mean=300, rms=0, runs=1。
    let ch1 = find(1);
    assert!((ch1.mean - 300.0).abs() < 1e-9);
    assert_eq!(ch1.rms, 0.0);
    assert_eq!(ch1.runs, 1, "ch1 は file A にしか出現しない");
    assert!(!ch1.is_fpn);

    // ch11: FPN、定数 400(file A のみ)。mean=400, rms=0, is_fpn=true。
    let ch11 = find(11);
    assert!((ch11.mean - 400.0).abs() < 1e-9);
    assert_eq!(ch11.rms, 0.0);
    assert!(ch11.is_fpn);
    assert_eq!(ch11.runs, 1);

    // meta: 入力ファイル一覧・イベント数(distinct event_idx = {0,1} → 2)。
    assert_eq!(computed.meta.inputs.len(), 2);
    assert_eq!(computed.meta.events, 2);
    assert!(!computed.meta.tool_version.is_empty());
}

// ---------------------------------------------------------------------
// pulser: baseline 差引・波高中央値・relative_gain・FPN 飽和(発注書のオラクル)。
// ---------------------------------------------------------------------

#[test]
fn pulser_compute_matches_hand_calculation_for_baseline_median_gain_and_saturation() {
    let dir = ScratchDir::new("pulser");

    // 3 イベント。ch0(信号): baseline(bucket0)=100 固定、ピークは bucket100 で
    // それぞれ 500, 520, 480 → amp = 400, 420, 380。中央値(奇数 3 個)= 400。
    // ch1(信号): 常に amp=800(baseline=100固定、ピーク=900) → 相対ゲインの分母を作る。
    // ch11(FPN): baseline(bucket0)=100、ピークは常に 4095(飽和)→ amp=3995、saturated=1.0、gain=None。
    let mut frames = Vec::new();
    for (event_idx, ch0_peak) in [500u16, 520, 480].into_iter().enumerate() {
        let items = vec![
            (0u8, 0u8, 0u16, 100u16),     // ch0 baseline
            (0u8, 0u8, 100u16, ch0_peak), // ch0 peak
            (0u8, 1u8, 0u16, 100u16),     // ch1 baseline
            (0u8, 1u8, 100u16, 900u16),   // ch1 peak(常に一定)
            (0u8, 11u8, 0u16, 100u16),    // FPN baseline
            (0u8, 11u8, 100u16, 4095u16), // FPN peak(飽和)
        ];
        frames.push(make_frame_type1(0, event_idx as u32, &items));
    }
    let file = write_graw(&dir, "pulser.graw", &frames);

    let computed = pulser::compute(&[file]).unwrap();
    let find = |ch: u8| {
        computed
            .channels
            .iter()
            .find(|c| c.asad == 0 && c.aget == 0 && c.ch == ch)
            .unwrap_or_else(|| panic!("ch{ch} missing"))
    };

    let ch0 = find(0);
    assert!(
        (ch0.amplitude_median - 400.0).abs() < 1e-9,
        "amp_median={}",
        ch0.amplitude_median
    );
    assert!(!ch0.is_fpn);
    assert_eq!(ch0.saturated, 0.0);
    // 全信号 ch の中央値 = median([400, 800]) = 600(2 個なので平均)。
    // ch0.relative_gain = 400/600 = 0.6666...
    let gain0 = ch0.relative_gain.expect("signal ch has Some gain");
    assert!((gain0 - (400.0 / 600.0)).abs() < 1e-9, "gain0={gain0}");

    let ch1 = find(1);
    assert!((ch1.amplitude_median - 800.0).abs() < 1e-9);
    let gain1 = ch1.relative_gain.expect("signal ch has Some gain");
    assert!((gain1 - (800.0 / 600.0)).abs() < 1e-9, "gain1={gain1}");

    let ch11 = find(11);
    assert!(ch11.is_fpn);
    assert_eq!(ch11.relative_gain, None, "FPN の gain は null");
    assert_eq!(ch11.saturated, 1.0, "全 3 イベントが 4095 に到達");
    assert!((ch11.amplitude_median - (4095.0 - 100.0)).abs() < 1e-9);

    assert_eq!(computed.meta.events, 3);
    assert_eq!(computed.meta.inputs.len(), 1);
}

// ---------------------------------------------------------------------
// pedestal → pulser → merge を通す(発注書「pedestal / pulser / merge を通し」)。
// flags 分岐の網羅は src/calib/table.rs の単体テストで別途済み。
// ---------------------------------------------------------------------

#[test]
fn pedestal_and_pulser_tables_flow_through_merge() {
    let dir = ScratchDir::new("merge_flow");

    // pedestal: ch0(信号, mean=300, rms=0)、ch11(FPN, mean=400, rms=0)。
    let items_a: Vec<(u8, u16)> = (0..12u16)
        .map(|c| {
            if c == 11 {
                (0u8, 400u16)
            } else if c == 0 {
                (0u8, 300u16)
            } else {
                (0u8, 999u16)
            }
        })
        .collect();
    let ped_file = write_graw(&dir, "ped.graw", &[make_frame_type2(0, 0, &items_a)]);
    let ped_table = pedestal::compute(&[ped_file]).unwrap();

    // pulser: ch0(信号)と ch11(FPN、飽和)だけ用意する(merge は共通チャンネルの交差だけを
    // 出力するので、pedestal に無いチャンネルは自然に落ちる — 別テストで確認済み)。
    let pulser_items = vec![
        (0u8, 0u8, 0u16, 100u16),
        (0u8, 0u8, 100u16, 500u16),
        (0u8, 11u8, 0u16, 100u16),
        (0u8, 11u8, 100u16, 4095u16),
    ];
    let pul_file = write_graw(&dir, "pul.graw", &[make_frame_type1(0, 0, &pulser_items)]);
    let pul_table = pulser::compute(&[pul_file]).unwrap();

    let calib = table::merge(&ped_table, &pul_table, table::DEFAULT_K);

    // ch0 と ch11 の 2 ch だけが merge に残る(join は交差)。
    assert_eq!(calib.channels.len(), 2);
    let ch0 = calib
        .channels
        .iter()
        .find(|c| c.ch == 0)
        .expect("ch0 present");
    assert_eq!(ch0.baseline, 300.0);
    assert_eq!(ch0.noise, 0.0);
    assert_eq!(
        ch0.gain,
        Some(1.0),
        "全信号 ch が ch0 だけなので中央値正規化で 1.0"
    );
    // threshold_suggest = round(300 + 5*0) = 300。
    assert_eq!(ch0.threshold_suggest, 300);
    assert!(!ch0.fpn);

    let ch11 = calib
        .channels
        .iter()
        .find(|c| c.ch == 11)
        .expect("ch11 present");
    assert!(ch11.fpn);
    assert_eq!(ch11.gain, None);
    assert!(ch11.flags.contains(&"saturated".to_string()));
}

// ---------------------------------------------------------------------
// write_json / read_json の往復(bin の merge サブコマンドが依存する経路)。
// ---------------------------------------------------------------------

#[test]
fn pedestal_table_round_trips_through_a_json_file() {
    let dir = ScratchDir::new("roundtrip");
    let ped_file = write_graw(&dir, "p.graw", &[make_frame_type2(0, 0, &[(0, 300)])]);
    let ped_table = pedestal::compute(&[ped_file]).unwrap();

    let out = dir.path().join("pedestal.json");
    pedestal::write(&ped_table, &out).unwrap();
    let read_back = pedestal::read(&out).unwrap();
    assert_eq!(read_back.channels, ped_table.channels);
    assert_eq!(read_back.meta, ped_table.meta);
}

// ---------------------------------------------------------------------
// scan: 振幅ラダー(既存 pulser と同じ peak-baseline 定義)+ 平均波形(TODO/091 §1)。
// ---------------------------------------------------------------------

/// aget/chan の 512 バケット全部に `baseline` を書き、`spike_bucket` だけ `spike_adc` に
/// 差し替える(full-readout を模す。未書き込みバケットを残すと「ペデスタル差引後は
/// ほぼ 0」という平均波形オラクルが汚れるので避ける)。
fn flat_waveform_with_spike(
    aget: u8,
    chan: u8,
    baseline: u16,
    spike_bucket: u16,
    spike_adc: u16,
) -> Vec<(u8, u8, u16, u16)> {
    (0u16..512)
        .map(|b| {
            (
                aget,
                chan,
                b,
                if b == spike_bucket {
                    spike_adc
                } else {
                    baseline
                },
            )
        })
        .collect()
}

#[test]
fn scan_compute_matches_hand_calculation_for_slope_intercept_residual_and_average_waveform() {
    use tpcdaq::calib::scan::{self, ScanRun};

    let dir = ScratchDir::new("scan");

    // 3 振幅(−100/−300/−1000 mV)× 2 イベント。ch0(信号, aget0, ch0): baseline=50、
    // bucket100 のピークが中心値 ±10 で振れる(平均・中央値とも中心値に一致するので
    // 手計算が単純)。中心値は 100/300/700(−1000 だけ線形予測 1000 を大きく下回る =
    // 圧縮を模す)。ch1(信号, aget0, ch1): baseline=50、中心値は 50/150/500(圧縮なし、
    // 線形のまま、±5 で振れる)。ch11(FPN, aget0): baseline=100、ピークは常に 4095
    // (飽和、振幅に依らず一定)。
    let make_run = |ch0_center: u16, ch1_center: u16, name: &str| -> PathBuf {
        let ch0_lo = flat_waveform_with_spike(0, 0, 50, 100, 50 + ch0_center - 10);
        let ch0_hi = flat_waveform_with_spike(0, 0, 50, 100, 50 + ch0_center + 10);
        let ch1_lo = flat_waveform_with_spike(0, 1, 50, 100, 50 + ch1_center - 5);
        let ch1_hi = flat_waveform_with_spike(0, 1, 50, 100, 50 + ch1_center + 5);
        let ch11 = flat_waveform_with_spike(0, 11, 100, 100, 4095);

        let mut event0 = ch0_lo;
        event0.extend(ch1_lo);
        event0.extend(ch11.clone());
        let mut event1 = ch0_hi;
        event1.extend(ch1_hi);
        event1.extend(ch11);

        write_graw(
            &dir,
            name,
            &[
                make_frame_type1(0, 0, &event0),
                make_frame_type1(0, 1, &event1),
            ],
        )
    };

    let runs = vec![
        ScanRun {
            amp_mv: -100.0,
            path: make_run(100, 50, "amp_m100.graw"),
        },
        ScanRun {
            amp_mv: -300.0,
            path: make_run(300, 150, "amp_m300.graw"),
        },
        ScanRun {
            amp_mv: -1000.0,
            path: make_run(700, 500, "amp_m1000.graw"),
        },
    ];

    let table = scan::compute(&runs).unwrap();

    assert_eq!(table.amps, vec![-100.0, -300.0, -1000.0]);
    assert_eq!(table.meta.inputs.len(), 3);
    assert_eq!(table.meta.events, 6, "2 events x 3 runs");

    let find = |ch: u8| {
        table
            .channels
            .iter()
            .find(|c| c.asad == 0 && c.aget == 0 && c.ch == ch)
            .unwrap_or_else(|| panic!("ch{ch} missing"))
    };

    // ch0: 振幅ラダー = [100, 300, 700](中央値。±10 の 2 イベント平均/中央値は中心値に一致)。
    let ch0 = find(0);
    assert!(!ch0.is_fpn);
    assert_eq!(ch0.amplitude, vec![100.0, 300.0, 700.0]);
    // 手計算: フィットは |mV|<=700 の 2 点 (-100,100),(-300,300) のみ使う(発注書「振幅
    // ≤ −700mV(圧縮前)」の解釈 — src/calib/scan.rs のモジュール doc 参照。全振幅が負なので
    // 文字どおりの実数比較では圧縮側の 2 点しか残らず「圧縮前」の意図と逆になるため、
    // 絶対値 <=700mV と読む)。
    // slope=(300-100)/(-300-(-100))=-1.0, intercept=100-(-1.0*-100)=0。
    assert!(
        (ch0.fit.slope - (-1.0)).abs() < 1e-9,
        "slope={}",
        ch0.fit.slope
    );
    assert!(
        (ch0.fit.intercept - 0.0).abs() < 1e-9,
        "intercept={}",
        ch0.fit.intercept
    );
    // -1000 での予測 = -1.0*-1000+0 = 1000、実測 700 → 残差 (700-1000)/1000 = -0.3。
    assert!(
        (ch0.fit.max_resid_frac - 0.3).abs() < 1e-9,
        "max_resid_frac={}",
        ch0.fit.max_resid_frac
    );
    // 平均波形: bucket100 だけ中心値、他は 0(baseline=pedestal なので差引後 0)。
    assert_eq!(ch0.waveform.len(), 3);
    for (wave, expected_peak) in ch0.waveform.iter().zip([100.0, 300.0, 700.0]) {
        assert_eq!(wave.len(), 512);
        assert!(
            (wave[100] - expected_peak).abs() < 1e-9,
            "wave[100]={}",
            wave[100]
        );
        assert!((wave[0] - 0.0).abs() < 1e-9, "wave[0]={}", wave[0]);
        assert!((wave[511] - 0.0).abs() < 1e-9, "wave[511]={}", wave[511]);
    }

    // ch1: 圧縮なし(線形)。3 点目(-1000)まで外挿しても残差 0。
    let ch1 = find(1);
    assert_eq!(ch1.amplitude, vec![50.0, 150.0, 500.0]);
    assert!(
        (ch1.fit.slope - (-0.5)).abs() < 1e-9,
        "slope={}",
        ch1.fit.slope
    );
    assert!(
        (ch1.fit.intercept - 0.0).abs() < 1e-9,
        "intercept={}",
        ch1.fit.intercept
    );
    assert!(
        ch1.fit.max_resid_frac < 1e-9,
        "max_resid_frac={} (linear, no compression)",
        ch1.fit.max_resid_frac
    );

    // ch11: FPN、常時飽和(4095)。振幅ラダーは全振幅で 3995 一定 → slope=0, intercept=3995。
    let ch11 = find(11);
    assert!(ch11.is_fpn);
    assert_eq!(ch11.amplitude, vec![3995.0, 3995.0, 3995.0]);
    assert!(
        (ch11.fit.slope - 0.0).abs() < 1e-9,
        "slope={}",
        ch11.fit.slope
    );
    assert!(
        (ch11.fit.intercept - 3995.0).abs() < 1e-9,
        "intercept={}",
        ch11.fit.intercept
    );
    assert!(ch11.fit.max_resid_frac < 1e-9);
    for wave in &ch11.waveform {
        assert!((wave[100] - 3995.0).abs() < 1e-9);
    }
}

#[test]
fn scan_table_round_trips_through_a_json_file() {
    use tpcdaq::calib::scan::{self, ScanRun};

    let dir = ScratchDir::new("scan_roundtrip");
    let wave_a = flat_waveform_with_spike(0, 0, 50, 100, 150); // amp=100
    let wave_b = flat_waveform_with_spike(0, 0, 50, 100, 350); // amp=300
    let path_a = write_graw(&dir, "a.graw", &[make_frame_type1(0, 0, &wave_a)]);
    let path_b = write_graw(&dir, "b.graw", &[make_frame_type1(0, 0, &wave_b)]);
    let table = scan::compute(&[
        ScanRun {
            amp_mv: -100.0,
            path: path_a,
        },
        ScanRun {
            amp_mv: -300.0,
            path: path_b,
        },
    ])
    .unwrap();

    let out = dir.path().join("scan.json");
    scan::write(&table, &out).unwrap();
    let read_back = scan::read(&out).unwrap();
    assert_eq!(read_back.amps, table.amps);
    assert_eq!(read_back.channels, table.channels);
}
