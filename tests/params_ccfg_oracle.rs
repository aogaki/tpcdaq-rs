//! 078 §5-2 — **CompoundConfig オラクル**による照合(一次資料はテストダブルではない)。
//!
//! `tools/ccfg_resolve`(実 GET の `CompoundConfig` をビルド時参照する C++ ツール)で
//! 生成 xcfg の**有効値**(`id="*"` の継承を解決した後の値)を全インスタンス分印字させ、
//! Rust 側の「JSON が正」の解決([`Params::effective_value`]、078 §3-4)と突き合わせる。
//!
//! 走らせ方(env が無ければ skip を出力して緑のまま — 078 §9):
//!
//! ```text
//! $ make -j -C tools/ccfg_resolve CCFG_DIR=$PWD/reference/20190315_patched/CompoundConfig
//! $ TPCDAQ_CCFG_RESOLVE_BIN=$PWD/tools/ccfg_resolve/build/ccfg_resolve cargo test --test params_ccfg_oracle
//! ```
//!
//! `TPCDAQ_XCFG_TEMPLATE` に**実テンプレート**(`reference/config/configure-*.xcfg`、
//! リポには入れない)のパスを入れると、ELITPC 実寸(4 AsAd × 4 AGET × 68 ch = 1,088 ch)でも
//! 同じ照合を行う。

#![allow(clippy::unwrap_used)]

use std::io::Write;
use std::process::{Command, Stdio};

use tpcdaq::params::schema::{knob_spec, value_to_xcfg_text};
use tpcdaq::params::{knob_path, render, Params, Scope, KNOBS};

fn oracle_bin() -> Option<String> {
    match std::env::var("TPCDAQ_CCFG_RESOLVE_BIN") {
        Ok(path) if !path.is_empty() => Some(path),
        _ => {
            eprintln!(
                "SKIP params_ccfg_oracle: TPCDAQ_CCFG_RESOLVE_BIN not set \
                 (CompoundConfig は reference/ にしか無いのでリポ単体では照合しない)"
            );
            None
        }
    }
}

/// `ccfg_resolve <file> < paths` を 1 回呼んで `path -> value` を得る。
fn resolve_all(bin: &str, xcfg_path: &std::path::Path, paths: &[String]) -> Vec<(String, String)> {
    let mut child = Command::new(bin)
        .arg(xcfg_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("cannot run {bin}: {e}"));
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        for path in paths {
            writeln!(stdin, "{path}").expect("write path");
        }
    }
    let output = child.wait_with_output().expect("wait");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    stdout
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(2, '\t');
            let path = parts.next()?.to_string();
            let value = parts.next()?.to_string();
            Some((path, value))
        })
        .collect()
}

/// テンプレート本文から、ある要素名の明示 id(数値)を集める。
/// ch 数をコードに焼き込まない(CLAUDE.md)— テンプレートが唯一の真実。
fn ids_of(xml: &str, element: &str) -> Vec<String> {
    let needle = format!("<{element} id=\"");
    let mut ids: Vec<String> = xml
        .match_indices(&needle)
        .filter_map(|(at, _)| {
            let rest = &xml[at + needle.len()..];
            let end = rest.find('"')?;
            let id = &rest[..end];
            if id == "*" {
                None
            } else {
                Some(id.to_string())
            }
        })
        .collect();
    ids.sort_by_key(|id| id.parse::<i64>().unwrap_or(i64::MAX));
    ids.dedup();
    ids
}

fn write_temp(name: &str, contents: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("tpcdaq_078_{name}"));
    std::fs::write(&path, contents).expect("write temp xcfg");
    path
}

/// 合成テンプレート(リポ内)で、**全インスタンス × 全ノブ**を照合する。
#[test]
fn the_rendered_xcfg_resolves_to_the_json_values_for_every_instance() {
    let Some(bin) = oracle_bin() else { return };

    let template = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/params/template_mini.xcfg"
    ))
    .unwrap();

    // 一様値はテンプレートと変えたうえで、3 スコープすべてに例外を置く。
    let json = r#"{
        "schema": "tpcdaq-params/1",
        "template": "configure-mini-fixture.xcfg",
        "knobs": {
            "triggerDelay": 2000,
            "writingClockFrequency_MHz": 12.5,
            "triggerMode": "periodically",
            "globalThreshold": 3,
            "lsbThreshold": 12,
            "gain": "240fC",
            "triggerInhibition": "none",
            "peakingTime_ns": 502,
            "readout": "partial",
            "zeroSuppression": {"enabled": true, "threshold": 700},
            "pulser": {"enabled": true, "voltage_mV": -300.0, "period_ms": 200}
        },
        "exceptions": [
            {"at": {"asad": 1}, "knob": "lsbThreshold", "value": 7, "note": "AsAd1 全体"},
            {"at": {"asad": 1, "aget": 0, "ch": 3}, "knob": "lsbThreshold", "value": 3},
            {"at": {"asad": 0, "aget": 1, "ch": 4}, "knob": "triggerInhibition", "value": "inhibit_channel"},
            {"at": {"asad": 1, "aget": 1}, "knob": "globalThreshold", "value": 6},
            {"at": {"asad": 0, "aget": 1}, "knob": "peakingTime_ns", "value": 117},
            {"at": {"asad": 0}, "knob": "readout", "value": "full", "note": "合成ノブ × 粗い例外"},
            {"at": {"asad": 1, "aget": 0, "ch": 2}, "knob": "zeroSuppression.threshold", "value": 42}
        ]
    }"#;
    let params = Params::from_json_str(json).unwrap();
    let rendered = render(&params, &template).unwrap();
    let path = write_temp("mini_rendered.xcfg", &rendered.xml);

    let checked = check_all_instances(&bin, &path, &params, &rendered.xml, &["0", "1"]);
    // KNOBS のエントリは CoBo 7 / AGET 4 / channel 4(合成ノブ readout は 2 エントリ)。
    // 2 CoBo instance × (7 + 4×(2 AsAd × 2 AGET) + 4×(2 × 2 × 6 ch)) = 2 × (7 + 16 + 96) = 238
    assert_eq!(checked, 238, "照合したインスタンス数");
    eprintln!("params_ccfg_oracle: mini fixture で {checked} インスタンスを照合");
}

/// 除去規則(§3-1)の負性テストを**オラクルで**確かめる。
/// テンプレートのままなら明示ノードが勝つ(= 罠が本物)。生成物では消えている。
#[test]
fn the_stale_explicit_override_wins_in_the_template_and_is_gone_after_rendering() {
    let Some(bin) = oracle_bin() else { return };

    let template = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/params/template_stale_overrides.xcfg"
    ))
    .unwrap();
    let template_path = write_temp("stale_template.xcfg", &template);

    let trap_paths = vec![
        knob_path("0", Scope::Cobo, &[], &["Module", "triggerDelay"]).to_string(),
        knob_path(
            "0",
            Scope::Aget,
            &["0", "1"],
            &["Global", "Reg1", "GlobalThresholdValue"],
        )
        .to_string(),
        knob_path(
            "0",
            Scope::Channel,
            &["0", "0", "4"],
            &["LSBThresholdValue"],
        )
        .to_string(),
        knob_path(
            "0",
            Scope::Channel,
            &["1", "0", "3"],
            &["LSBThresholdValue"],
        )
        .to_string(),
        knob_path("0", Scope::Channel, &["1", "0", "3"], &["Gain"]).to_string(),
        // 2026-09-22 に追加した 4 ノブの仕込み(5〜8)。
        knob_path(
            "0",
            Scope::Aget,
            &["0", "1"],
            &["Global", "Reg1", "peackingTime"],
        )
        .to_string(),
        knob_path(
            "0",
            Scope::Aget,
            &["0", "1"],
            &["Global", "Reg1", "isAllChannelRead"],
        )
        .to_string(),
        knob_path(
            "0",
            Scope::Channel,
            &["0", "0", "4"],
            &["zeroSuppressionThreshold"],
        )
        .to_string(),
        knob_path("0", Scope::Cobo, &[], &["Module", "enableZeroSuppression"]).to_string(),
        knob_path("0", Scope::Cobo, &[], &["Generator", "periodicModeEnabled"]).to_string(),
    ];

    // 罠が本物であることを一次資料で確認する。
    let before = resolve_all(&bin, &template_path, &trap_paths);
    let before_values: Vec<&str> = before.iter().map(|(_, v)| v.as_str()).collect();
    assert_eq!(
        before_values,
        vec!["999", "6", "0", "2", "1pC", "70", "false", "11", "true", "true"],
        "テンプレートの明示ノードが勝つ(= 除去規則が必要な理由)"
    );

    // 生成物では一様値に戻る。
    let json = r#"{"schema":"tpcdaq-params/1","template":"configure-mini-fixture.xcfg",
        "knobs":{"triggerDelay":1748,"writingClockFrequency_MHz":25.0,"triggerMode":"external",
        "globalThreshold":1,"lsbThreshold":15,"gain":"120fC","triggerInhibition":"none",
        "peakingTime_ns":232,"readout":"full",
        "zeroSuppression":{"enabled":false,"threshold":600},
        "pulser":{"enabled":false,"voltage_mV":-1000.0,"period_ms":1000}},
        "exceptions":[]}"#;
    let params = Params::from_json_str(json).unwrap();
    let rendered = render(&params, &template).unwrap();
    let rendered_path = write_temp("stale_rendered.xcfg", &rendered.xml);

    let after = resolve_all(&bin, &rendered_path, &trap_paths);
    let after_values: Vec<&str> = after.iter().map(|(_, v)| v.as_str()).collect();
    assert_eq!(
        after_values,
        vec!["1748", "1", "15", "15", "120fC", "232", "true", "600", "false", "false"]
    );
}

/// 実テンプレート(env、リポ外)での全数照合。ELITPC 実寸 = 1,088 ch。
#[test]
fn the_real_template_resolves_to_the_json_values_for_every_instance() {
    let Some(bin) = oracle_bin() else { return };
    let Ok(template_path) = std::env::var("TPCDAQ_XCFG_TEMPLATE") else {
        eprintln!(
            "SKIP the_real_template_...: TPCDAQ_XCFG_TEMPLATE not set \
             (実 xcfg は reference/config にしか無い)"
        );
        return;
    };

    let template = std::fs::read_to_string(&template_path).unwrap();
    let json = r#"{
        "schema": "tpcdaq-params/1",
        "template": "configure-physics-real.xcfg",
        "knobs": {
            "triggerDelay": 1748,
            "writingClockFrequency_MHz": 25.0,
            "triggerMode": "external",
            "globalThreshold": 1,
            "lsbThreshold": 15,
            "gain": "120fC",
            "triggerInhibition": "none",
            "peakingTime_ns": 232,
            "readout": "full",
            "zeroSuppression": {"enabled": false, "threshold": 600},
            "pulser": {"enabled": false, "voltage_mV": -1000.0, "period_ms": 1000}
        },
        "exceptions": [
            {"at": {"asad": 2, "aget": 3, "ch": 47}, "knob": "lsbThreshold", "value": 4},
            {"at": {"asad": 2, "aget": 3, "ch": 47}, "knob": "triggerInhibition", "value": "inhibit_channel"},
            {"at": {"asad": 3}, "knob": "gain", "value": "240fC"},
            {"at": {"asad": 0, "aget": 2}, "knob": "globalThreshold", "value": 5},
            {"at": {"asad": 1, "aget": 2}, "knob": "peakingTime_ns", "value": 1014},
            {"at": {"asad": 0}, "knob": "readout", "value": "partial"},
            {"at": {"asad": 3, "aget": 0, "ch": 11}, "knob": "zeroSuppression.threshold", "value": 4095}
        ]
    }"#;
    let params = Params::from_json_str(json).unwrap();
    let rendered = render(&params, &template).unwrap();
    let path = write_temp("real_rendered.xcfg", &rendered.xml);

    // 実テンプレートは CoBo インスタンスが 0..11 あるが、ELITPC は 1 論理 CoBo なので 0 だけ見る。
    let checked = check_all_instances(&bin, &path, &params, &rendered.xml, &["0"]);
    eprintln!("params_ccfg_oracle: 実テンプレート {template_path} で {checked} インスタンスを照合");
    assert!(checked > 1000, "照合数が少なすぎる: {checked}");
}

/// §5-1 の往復を**実テンプレート**(831 行)で。オラクルは要らないので
/// `TPCDAQ_XCFG_TEMPLATE` だけで走る。
#[test]
fn the_real_template_round_trips_byte_for_byte() {
    let Ok(template_path) = std::env::var("TPCDAQ_XCFG_TEMPLATE") else {
        eprintln!("SKIP the_real_template_round_trips_byte_for_byte: TPCDAQ_XCFG_TEMPLATE not set");
        return;
    };
    let template = std::fs::read_to_string(&template_path).unwrap();

    // 実テンプレート(physics 25 MHz)の現行値 = docs/PARAMS_KNOBS_ja.md §3 の「現行値」列。
    let json = r#"{
        "schema": "tpcdaq-params/1",
        "template": "configure-physics-real.xcfg",
        "knobs": {
            "triggerDelay": 1748,
            "writingClockFrequency_MHz": 25.0,
            "triggerMode": "external",
            "globalThreshold": 1,
            "lsbThreshold": 15,
            "gain": "120fC",
            "triggerInhibition": "none",
            "peakingTime_ns": 232,
            "readout": "full",
            "zeroSuppression": {"enabled": false, "threshold": 600},
            "pulser": {"enabled": false, "voltage_mV": -1000.0, "period_ms": 1000}
        },
        "exceptions": []
    }"#;
    let params = Params::from_json_str(json).unwrap();

    let rendered = render(&params, &template).unwrap();

    assert_eq!(
        rendered.xml, template,
        "実テンプレートの往復がバイト一致しない"
    );
}

/// 全スコープ・全 id のパスを作ってオラクルと突き合わせる。返り値は照合したインスタンス数。
fn check_all_instances(
    bin: &str,
    xcfg_path: &std::path::Path,
    params: &Params,
    rendered_xml: &str,
    instances: &[&str],
) -> usize {
    let asads = ids_of(rendered_xml, "AsAd");
    let agets = ids_of(rendered_xml, "Aget");
    let channels = ids_of(rendered_xml, "channel");
    assert!(!asads.is_empty() && !agets.is_empty() && !channels.is_empty());

    // path -> 期待値。
    let mut expected: Vec<(String, String)> = Vec::new();
    for instance in instances {
        for spec in KNOBS {
            match spec.scope {
                Scope::Cobo => {
                    let path = knob_path(instance, spec.scope, &[], spec.suffix).to_string();
                    let value = params.effective_value(spec.name, 0, 0, 0).unwrap();
                    expected.push((path, value_to_xcfg_text(spec, value).unwrap()));
                }
                Scope::Asad => unreachable!("AsAd スコープのノブは未実装(PARAMS_KNOBS_ja.md)"),
                Scope::Aget => {
                    for asad in &asads {
                        for aget in &agets {
                            let path = knob_path(
                                instance,
                                spec.scope,
                                &[asad.as_str(), aget.as_str()],
                                spec.suffix,
                            )
                            .to_string();
                            let value = params
                                .effective_value(
                                    spec.name,
                                    asad.parse().unwrap(),
                                    aget.parse().unwrap(),
                                    0,
                                )
                                .unwrap();
                            expected.push((path, value_to_xcfg_text(spec, value).unwrap()));
                        }
                    }
                }
                Scope::Channel => {
                    for asad in &asads {
                        for aget in &agets {
                            for ch in &channels {
                                let path = knob_path(
                                    instance,
                                    spec.scope,
                                    &[asad.as_str(), aget.as_str(), ch.as_str()],
                                    spec.suffix,
                                )
                                .to_string();
                                let value = params
                                    .effective_value(
                                        spec.name,
                                        asad.parse().unwrap(),
                                        aget.parse().unwrap(),
                                        ch.parse().unwrap(),
                                    )
                                    .unwrap();
                                expected.push((path, value_to_xcfg_text(spec, value).unwrap()));
                            }
                        }
                    }
                }
            }
        }
    }

    let paths: Vec<String> = expected.iter().map(|(p, _)| p.clone()).collect();
    let resolved = resolve_all(bin, xcfg_path, &paths);
    assert_eq!(
        resolved.len(),
        expected.len(),
        "オラクルが返した行数がパス数と違う"
    );

    let mut mismatches: Vec<String> = Vec::new();
    for ((path, want), (got_path, got)) in expected.iter().zip(resolved.iter()) {
        assert_eq!(path, got_path, "オラクルの出力順が入力順と違う");
        if want != got {
            mismatches.push(format!("{path}: want {want}, got {got}"));
        }
    }
    assert!(
        mismatches.is_empty(),
        "オラクルと食い違い {} 件:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );

    // knob_spec が引けることも一応確かめておく(KNOBS と名前表の整合)。
    for spec in KNOBS {
        assert!(knob_spec(spec.name).is_some());
    }

    expected.len()
}
