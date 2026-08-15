//! soak_harness のスモーク(TODO/031-3)。**負荷ハーネス自体の回帰**であって、
//! 耐久試験そのものではない —— 一晩の実走(SPEC §12-5 v1.15)や 10 分バースト(§12-6)は
//! CI に入れない。
//!
//! ここで固定するのは「ハーネスの判定パイプライン一式が動く」こと:
//!
//! * 全通し配線(fake-ECC + ecc-bridge + root-sink + monitor + Rust 3 コンポーネント +
//!   controller)を**子プロセス**で上げ、controller REST で run を **2 本**回す。
//! * 1 run 毎の照合(ROOT entries = laps×108 / 生 graw バイト = laps×30,108,684 /
//!   全ロスレスカウンタ 0 / monitor.root 存在)が実際に効く。
//! * 合格した run の**出力が消える**(「書いて検証して消す」)。
//! * メトリクス CSV が 1 行 1 サンプルで書かれ、レポートがその CSV から作られる。
//!
//! env が欠けたら**欠けた変数名を stderr に出して** skip する(030 と同じ 5 本):
//!
//! ```text
//! TPCDAQ_ROOT_SINK_BIN=$PWD/tools/root_sink/root_sink \
//! TPCDAQ_ECC_BRIDGE_BIN=$PWD/tools/ecc_bridge/ecc_bridge \
//! TPCDAQ_FAKE_ECC_BIN=$PWD/tools/ecc_bridge/fake_ecc \
//! TPCDAQ_REAL_GRAW=$HOME/TPC/CoBo_2025-09-01T08_51_06.203_0000.graw \
//! TPCDAQ_REAL_GEOMETRY_MINI=$HOME/TPC/miniTPC_UVW_pcb_info/new_geometry_mini_eTPC.dat \
//!   cargo test --release --test soak_smoke -- --nocapture
//! ```

#![allow(clippy::unwrap_used)]

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{cleanup, e2e_env, scratch_dir, REAL_GRAW_BYTES, REAL_GRAW_EVENTS};

/// `soak_harness` が兄弟バイナリ(controller / receiver / …)を探す場所。
/// `CARGO_BIN_EXE_*` は同じ target ディレクトリを指すので、その親を渡せばよい。
fn bin_dir() -> PathBuf {
    Path::new(env!("CARGO_BIN_EXE_controller"))
        .parent()
        .expect("CARGO_BIN_EXE_controller has a parent directory")
        .to_path_buf()
}

/// 2 run × 短 lap でハーネス一式を回す。
///
/// `--run-minutes 0.02` = 1.2 s のリプレイ。224 Mbps(= 28 MB/s)では
/// 1 周 30,108,684 B ≈ 1.07 s なので、**lap 境界で止まる規則**により 2 周で終わる
/// (1 周目終了 1.07 s < 1.2 s → 続行、2 周目終了 2.15 s ≥ 1.2 s → 終了)。
/// したがって 1 run = 2 laps = 216 events = 60,217,368 B。
#[test]
fn soak_harness_runs_two_runs_verifies_and_deletes_the_outputs() {
    let Some(env) = e2e_env("soak smoke") else {
        return;
    };
    let scratch = scratch_dir("tpcdaq_soak_smoke", "two_runs");
    let out_dir = scratch.join("soak");

    let output = Command::new(env!("CARGO_BIN_EXE_soak_harness"))
        .args([
            "--mode",
            "soak",
            "--runs",
            "2",
            "--run-minutes",
            "0.02",
            // 時間では打ち切らせない(--runs 2 が先に効く)。
            "--duration-h",
            "1",
            "--metrics-interval-s",
            "2",
        ])
        .arg("--out-dir")
        .arg(&out_dir)
        .arg("--bin-dir")
        .arg(bin_dir())
        .env("TPCDAQ_ROOT_SINK_BIN", &env.root_sink)
        .env("TPCDAQ_ECC_BRIDGE_BIN", &env.ecc_bridge)
        .env("TPCDAQ_FAKE_ECC_BIN", &env.fake_ecc)
        .env("TPCDAQ_REAL_GRAW", &env.graw)
        .env("TPCDAQ_REAL_GEOMETRY_MINI", &env.geometry)
        .output()
        .expect("spawn soak_harness");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    eprintln!("---- soak_harness stderr ----\n{stderr}");
    assert!(
        output.status.success(),
        "soak_harness が非 0 で終わった: {:?}\n--- stdout ---\n{stdout}",
        output.status
    );

    // --- run 2 本が回り、run 毎の照合が通っていること(レポートは CSV から作られる)---
    assert!(
        stdout.contains("run 数 = 2"),
        "レポートに run 2 本が出ていない:\n{stdout}"
    );
    // 1 run = 2 laps = 216 events / 60,217,368 B(手計算: 2 × 108、2 × 30,108,684)。
    let want_bytes = 2 * REAL_GRAW_BYTES;
    let want_entries = 2 * REAL_GRAW_EVENTS;
    for run in 1..=2u32 {
        let want = format!("run {run:04}: laps=2 bytes={want_bytes} entries={want_entries}");
        assert!(
            stdout.contains(&want),
            "run {run} の照合行 {want:?} がレポートに無い:\n{stdout}"
        );
    }

    // --- 「書いて検証して消す」: 合格した run の出力は残っていない ---
    for run in 1..=2u32 {
        let dir = out_dir.join("data").join(format!("run{run:04}"));
        assert!(
            !dir.exists(),
            "合格した run の出力が消えていない: {}",
            dir.display()
        );
    }
    // ログブックは run の台帳なので残る(消すのは run 出力だけ)。
    assert!(
        out_dir.join("data").join("logbook.jsonl").is_file(),
        "logbook.jsonl が無い"
    );

    // --- メトリクス CSV(判定の一次データ)---
    let csv = std::fs::read_to_string(out_dir.join("metrics.csv")).expect("read metrics.csv");
    let mut lines = csv.lines();
    let header = lines.next().expect("CSV header");
    for column in [
        "elapsed_s",
        "run_number",
        "rss_kib_root_sink",
        "rss_kib_controller",
        "fd_receiver0",
        "recv_overflow_frames",
        "dec_malformed",
        "gw_write_errors",
        "rs_events_built",
        "mon_ws_dropped",
        "free_gib",
    ] {
        assert!(
            header.split(',').any(|c| c == column),
            "CSV に列 {column} が無い: {header}"
        );
    }
    let rows: Vec<&str> = lines.collect();
    assert!(
        rows.len() >= 2,
        "CSV のサンプルが {} 行しかない(2 s 毎に取っているはず)",
        rows.len()
    );
    for row in &rows {
        assert_eq!(
            row.split(',').count(),
            header.split(',').count(),
            "CSV の列数が揃っていない: {row}"
        );
    }

    // --- レポートはファイルにも残る(クラッシュしても証拠が残る形)---
    let report = std::fs::read_to_string(out_dir.join("report.txt")).expect("read report.txt");
    assert!(
        report.contains("RSS 単調性"),
        "レポートに RSS 判定が無い:\n{report}"
    );
    assert!(
        report.contains("モニタ系 drop"),
        "レポートにモニタ系 drop が無い:\n{report}"
    );

    cleanup(&scratch);
}

/// env が欠けたときの skip と同じ流儀で、**引数の誤りは即エラー**(silent に走らない)。
#[test]
fn soak_harness_rejects_a_missing_mode() {
    let output = Command::new(env!("CARGO_BIN_EXE_soak_harness"))
        .output()
        .expect("spawn soak_harness");
    assert!(!output.status.success(), "--mode 無しで成功してはいけない");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--mode is required"),
        "理由が出ていない: {stderr}"
    );
}
