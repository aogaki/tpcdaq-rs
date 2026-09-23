//! 実データ回帰(TODO/081 受け入れ条件 2)。
//!
//! `TPCDAQ_EXP_DATA_2026` に `reference/exp_data/2026` のパスが入っているときだけ走る
//! (実 .graw はリポに入れない — CLAUDE.md)。pedestal 1 run + pulser 1 run を
//! `calib::{pedestal,pulser}::compute` に通し、068 の CSV
//! (`reference/_spike/asset_survey/out/{B_pedestal_channel_stats,C_pulser_channel_stats}.csv`)
//! から実装者が読み取ったオラクル値と比較する。
//!
//! 選んだ run(`run_ledger.csv` の列名で選定): 067/068 で既に frameType 1/2 の実データ照合に
//! 使われている 2 本と同じもの(`src/decode.rs` のテストが引用する run と同一)。
//! - pedestal: `2026-08-16T17:37:09.555`(単一ファイル、frameType 2、467 events / 1868 データ
//!   フレーム / 233 バイト飛ばしの topology frame 1 個 — `run_ledger.csv` 実測)。
//! - pulser: `2026-08-17T08:09:11.852`(単一ファイル、frameType 1、76 events / 304 データ
//!   フレーム / topology frame 1 個)。
//!
//! オラクル値は 068 CSV の当該行から実装者が転記した(CSV 自体は gitignore 済みでリポに
//! 無いため、実行時に読むのではなく数値をここに固定する — `decoder_real_graw.rs` が
//! C++ オラクルの `108`/`15_040_512` を固定するのと同じ流儀)。
//!
//! **pedestal は mean/rms とも CSV と丸め誤差内(実測 max diff 5e-5 ADC、1088 ch 全数)で
//! 一致する**(定義が同一の pooled mean/rms なので当然)。
//!
//! **pulser は amplitude_median(本ツールは per-event 中央値)と CSV の `amp_mean`(per-event
//! 平均)を比較しており、統計量そのものが違う**(発注書は「波高中央値」= 中央値を明示している
//! が、068 の CSV には per-run per-channel の中央値列が無く `amp_mean` しか無いため)。
//! 実測(1088 ch 全数、実装者が別途計算): matched=1088, missing=0,
//! **max_diff_amp=8.6337 ADC, max_relative_diff=0.274%**。丸め誤差では説明できない差だが、
//! 中央値と平均という別統計量の差として妥当な大きさに収まっている。**この差分は「中央値」の
//! 解釈が発注書の意図(per-event 中央値)と 068 CSV の実装(per-run 平均)のどちらを指すかという
//! 未確定点に起因する — 最終報告で明示し、レビューを仰ぐ**(黙って通さない — CLAUDE.md)。

use std::path::PathBuf;

use tpcdaq::calib::{median, pedestal, pulser, scan};

fn exp_data_root() -> Option<PathBuf> {
    match std::env::var_os("TPCDAQ_EXP_DATA_2026") {
        Some(v) => Some(PathBuf::from(v)),
        None => {
            eprintln!("SKIP calib_real_graw: TPCDAQ_EXP_DATA_2026 not set (local-only regression)");
            None
        }
    }
}

#[test]
fn pedestal_real_run_matches_the_068_csv_oracle() {
    let Some(root) = exp_data_root() else {
        return;
    };
    let file = root.join("pedestal/CoBo_2026-08-16T17:37:09.555_0000.graw");
    assert!(file.is_file(), "missing {}", file.display());

    let started = std::time::Instant::now();
    let table = pedestal::compute(&[file]).expect("pedestal::compute");
    let elapsed = started.elapsed();
    eprintln!(
        "calib_real_graw: pedestal compute took {:.2}s, {} channels, events={}",
        elapsed.as_secs_f64(),
        table.channels.len(),
        table.meta.events
    );

    // run_ledger.csv: pedestal,2026-08-16T17:37:09.555,1,520768524,1869,1868,1,467,...
    assert_eq!(table.meta.events, 467, "events oracle (run_ledger.csv)");
    assert_eq!(table.channels.len(), 1088, "4 asad x 4 aget x 68 raw ch");

    let find = |asad: u8, aget: u8, ch: u8| {
        table
            .channels
            .iter()
            .find(|c| c.asad == asad && c.aget == aget && c.ch == ch)
            .unwrap_or_else(|| panic!("channel ({asad},{aget},{ch}) missing"))
    };

    // B_pedestal_channel_stats.csv, run=2026-08-16T17:37:09.555(実装者が転記、丸め誤差 1e-3 許容):
    //   asad,aget,raw_ch,role,mean,rms
    //   0,0,0,signal,   392.1518, 6.8526
    //   0,0,11,fpn,     415.3693, 2.0144
    //   1,1,14,signal,  512.2531, 6.2811  (068 SUMMARY の恒常異常 ch の 1 つ)
    //   3,0,62,signal,  248.2045, 4.4917  (同上)
    let cases = [
        (0u8, 0u8, 0u8, 392.1518, 6.8526, false),
        (0, 0, 11, 415.3693, 2.0144, true),
        (1, 1, 14, 512.2531, 6.2811, false),
        (3, 0, 62, 248.2045, 4.4917, false),
    ];
    for (asad, aget, ch, mean, rms, is_fpn) in cases {
        let c = find(asad, aget, ch);
        assert!(
            (c.mean - mean).abs() < 1e-3,
            "({asad},{aget},{ch}) mean={} expected={mean}",
            c.mean
        );
        assert!(
            (c.rms - rms).abs() < 1e-3,
            "({asad},{aget},{ch}) rms={} expected={rms}",
            c.rms
        );
        assert_eq!(c.is_fpn, is_fpn);
        assert_eq!(c.runs, 1, "single input file");
    }
}

#[test]
fn pulser_real_run_matches_the_068_csv_oracle_within_the_documented_median_vs_mean_gap() {
    let Some(root) = exp_data_root() else {
        return;
    };
    let file = root.join("pulser/CoBo_2026-08-17T08:09:11.852_0000.graw");
    assert!(file.is_file(), "missing {}", file.display());

    let started = std::time::Instant::now();
    let table = pulser::compute(&[file]).expect("pulser::compute");
    let elapsed = started.elapsed();
    eprintln!(
        "calib_real_graw: pulser compute took {:.2}s, {} channels, events={}",
        elapsed.as_secs_f64(),
        table.channels.len(),
        table.meta.events
    );

    // run_ledger.csv: pulser,2026-08-17T08:09:11.852,1,169422860,305,304,1,76,...
    assert_eq!(table.meta.events, 76, "events oracle (run_ledger.csv)");
    assert_eq!(table.channels.len(), 1088, "4 asad x 4 aget x 68 raw ch");

    let find = |asad: u8, aget: u8, ch: u8| {
        table
            .channels
            .iter()
            .find(|c| c.asad == asad && c.aget == aget && c.ch == ch)
            .unwrap_or_else(|| panic!("channel ({asad},{aget},{ch}) missing"))
    };

    // C_pulser_channel_stats.csv, run=2026-08-17T08:09:11.852(実装者が転記):
    //   asad,aget,raw_ch,role,amp_mean,saturated_events,events
    //   0,0,0, signal, 3264.839, 0,  76
    //   0,0,11,fpn,    3681.103, 76, 76
    // 本ツールの amplitude_median(per-event 中央値)は CSV の amp_mean(per-event 平均)と
    // 統計量が異なる。1088 ch 全数の実測差(実装者が別途計算)は
    // max_diff_amp=8.6337 ADC / max_relative_diff=0.274% — この 2 ch もその範囲に収まる
    // ことを 15 ADC の緩い許容で確認する(丸め誤差ではなく「別統計量としての近さ」の検証)。
    let ch0 = find(0, 0, 0);
    assert!(
        (ch0.amplitude_median - 3264.839).abs() < 15.0,
        "amplitude_median={} vs csv amp_mean=3264.839",
        ch0.amplitude_median
    );
    assert!(!ch0.is_fpn);
    assert_eq!(ch0.saturated, 0.0, "signal ch: csv saturated_events=0/76");
    let gain0 = ch0.relative_gain.expect("signal channel has Some gain");
    assert!(
        (0.9..1.1).contains(&gain0),
        "relative_gain for a near-median signal channel should be close to 1.0, got {gain0}"
    );

    let ch11 = find(0, 0, 11);
    assert!(ch11.is_fpn);
    assert_eq!(ch11.relative_gain, None, "FPN gain is null");
    assert_eq!(
        ch11.saturated, 1.0,
        "csv saturated_events=76/76 → saturation rate 1.0"
    );
    assert!(
        (ch11.amplitude_median - 3681.103).abs() < 15.0,
        "amplitude_median={} vs csv amp_mean=3681.103",
        ch11.amplitude_median
    );
}

/// TODO/091 §1 実データ回帰: D-3 パルサー振幅スキャン(2026-09-23、GDED、実 ECC・実
/// zCoBo、run 6–11、−100/−200/−300/−500/−700/−1000 mV)。
///
/// オラクル値は 091 §1 の発注書に書かれた Fable の先行数値
/// (`calib pulser` を各 run に個別適用した際の、信号 ch 中央値 / FPN 中央値、生 ADC・
/// 減算なし)。ここでは `calib::scan::compute` の per-channel `amplitude`(= 各 run 内で
/// pulser と同じ peak-baseline 定義を再利用した中央値)を is_fpn で分けて中央値を取り、
/// 同じ数字と 1 ADC 以内で一致することを確認する。
#[test]
fn scan_real_d3_pulser_ladder_matches_the_091_fable_pre_numbers_within_1_adc() {
    let Some(root) = exp_data_root() else {
        return;
    };
    let base = root.join("d3_pulser_scan");
    let runs = vec![
        scan::ScanRun {
            amp_mv: -100.0,
            path: base.join("run0006/CoBo0_AsAd0_2026-09-23T12:07:16.744_0000.graw"),
        },
        scan::ScanRun {
            amp_mv: -200.0,
            path: base.join("run0007/CoBo0_AsAd0_2026-09-23T12:08:31.454_0000.graw"),
        },
        scan::ScanRun {
            amp_mv: -300.0,
            path: base.join("run0008/CoBo0_AsAd0_2026-09-23T12:09:46.202_0000.graw"),
        },
        scan::ScanRun {
            amp_mv: -500.0,
            path: base.join("run0009/CoBo0_AsAd0_2026-09-23T12:11:00.883_0000.graw"),
        },
        scan::ScanRun {
            amp_mv: -700.0,
            path: base.join("run0010/CoBo0_AsAd0_2026-09-23T12:12:15.583_0000.graw"),
        },
        scan::ScanRun {
            amp_mv: -1000.0,
            path: base.join("run0011/CoBo0_AsAd0_2026-09-23T12:13:30.285_0000.graw"),
        },
    ];
    for r in &runs {
        assert!(r.path.is_file(), "missing {}", r.path.display());
    }

    let started = std::time::Instant::now();
    let table = scan::compute(&runs).expect("scan::compute");
    eprintln!(
        "calib_real_graw: scan compute took {:.2}s, {} channels",
        started.elapsed().as_secs_f64(),
        table.channels.len()
    );

    assert_eq!(table.channels.len(), 272, "1 asad x 4 aget x 68 raw ch");

    let amp_index = |mv: f64| {
        table
            .amps
            .iter()
            .position(|&a| a == mv)
            .unwrap_or_else(|| panic!("amp {mv} missing"))
    };
    let idx_m100 = amp_index(-100.0);
    let idx_m300 = amp_index(-300.0);
    let idx_m1000 = amp_index(-1000.0);

    let group_median = |idx: usize, want_fpn: bool| -> f64 {
        let vals: Vec<f64> = table
            .channels
            .iter()
            .filter(|c| c.is_fpn == want_fpn)
            .map(|c| c.amplitude[idx])
            .collect();
        median(&vals)
    };

    // 091 §1 発注書: 「先に Fable が calib pulser(081)で見た数字(signal ch 中央値、
    // FPN 除く): −100 mV → 369.0 ADC / −300 → 1074.0 / −1000 → 3239.8(飽和 ch 0)。
    // ... FPN の中央値は 543.6 / 1589.8 / 3427.0」。
    let signal_m100 = group_median(idx_m100, false);
    let signal_m300 = group_median(idx_m300, false);
    let signal_m1000 = group_median(idx_m1000, false);
    assert!(
        (signal_m100 - 369.0).abs() < 1.0,
        "signal median @-100mV={signal_m100}"
    );
    assert!(
        (signal_m300 - 1074.0).abs() < 1.0,
        "signal median @-300mV={signal_m300}"
    );
    assert!(
        (signal_m1000 - 3239.8).abs() < 1.0,
        "signal median @-1000mV={signal_m1000}"
    );

    let fpn_m100 = group_median(idx_m100, true);
    let fpn_m300 = group_median(idx_m300, true);
    let fpn_m1000 = group_median(idx_m1000, true);
    assert!(
        (fpn_m100 - 543.6).abs() < 1.0,
        "fpn median @-100mV={fpn_m100}"
    );
    assert!(
        (fpn_m300 - 1589.8).abs() < 1.0,
        "fpn median @-300mV={fpn_m300}"
    );
    assert!(
        (fpn_m1000 - 3427.0).abs() < 1.0,
        "fpn median @-1000mV={fpn_m1000}"
    );
}
