//! 078 レーン A — JSON スキーマ + xcfg レンダラの受け入れテスト(§5-1 往復 / §3-1 除去規則)。
//!
//! フィクスチャは**合成の縮小テンプレート**(`tests/fixtures/params/`)。実 xcfg は
//! `reference/config/` にしか無く、リポには入れない(CLAUDE.md)。構造は実物の写し
//! (`docs/PARAMS_KNOBS_ja.md` の照合結果)で、AsAd 0..1 / Aget 0..1 / channel 0..5 に縮めてある。
//!
//! テストが仕様書 — ここに書いてある挙動が 078 §2/§3 の実装契約。

#![allow(clippy::unwrap_used)]

use tpcdaq::params::{render, Params, ParamsError, PEAKING_TIME_NS};

fn fixture(name: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/params/{}",
        env!("CARGO_MANIFEST_DIR"),
        name
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"))
}

fn template_mini() -> String {
    fixture("template_mini.xcfg")
}

/// テンプレートの現行値そのままの JSON(例外なし)。
fn params_json_template_values() -> String {
    fixture("params_roundtrip.json")
}

/// 任意の knobs / exceptions を差し込む JSON を組み立てる(テストの可読性のため)。
fn params_json(knobs_and_exceptions: &str) -> String {
    format!(
        r#"{{"schema":"tpcdaq-params/1","template":"configure-mini-fixture.xcfg",{knobs_and_exceptions}}}"#
    )
}

/// 全ノブ(12 個 = 葉 15 個)をテンプレートの現行値にしたうえで、一部だけ差し替えるための素。
/// `KNOBS_BODY` は `{}` を含まないので `format!` で包んで使う(`replace("}")` の誤爆を避ける)。
const KNOBS_BODY: &str = r#""triggerDelay":1748,"writingClockFrequency_MHz":25.0,"triggerMode":"external","globalThreshold":1,"lsbThreshold":15,"gain":"120fC","triggerInhibition":"none","peakingTime_ns":232,"readout":"full","fpnRead":true,"zeroSuppression":{"enabled":false,"threshold":600},"pulser":{"enabled":false,"voltage_mV":-1000.0,"period_ms":1000}"#;

/// `"knobs":{…}` の形。
fn knobs_template_values() -> String {
    format!(r#""knobs":{{{KNOBS_BODY}}}"#)
}

/// 現行値の一部を差し替えた `"knobs":{…}`。
fn knobs_with(from: &str, to: &str) -> String {
    let body = KNOBS_BODY.replace(from, to);
    assert_ne!(body, KNOBS_BODY, "差し替え対象 {from} が見つからない");
    format!(r#""knobs":{{{body}}}"#)
}

// ---------------------------------------------------------------------
// §5-1 往復
// ---------------------------------------------------------------------

/// 例外なし + knobs = テンプレートの現行値 → **生成 xcfg はテンプレートとバイト一致**。
///
/// 078 §5-1 は「意味同一」を求めているが、レンダラは非管理ノブのイベントを
/// 生バイトのまま再生し、管理ノブも `*` ノードの位置と属性(`unit`/`range`)を保つので、
/// バイト一致という強い形で満たせる(= §3-2「1 バイトも変えない」の実証)。
#[test]
fn roundtrip_with_template_values_reproduces_the_template_byte_for_byte() {
    let template = template_mini();
    let params = Params::from_json_str(&params_json_template_values()).unwrap();

    let rendered = render(&params, &template).unwrap();

    assert_eq!(rendered.xml, template);
}

/// 管理ノブを 1 つ変えたら、**その 1 か所の値だけ**が変わる(非管理ノブは無傷)。
#[test]
fn changing_one_knob_changes_exactly_one_value_in_the_template() {
    let template = template_mini();
    let json = params_json(&format!(
        r#"{}, "exceptions":[]"#,
        knobs_with("\"triggerDelay\":1748", "\"triggerDelay\":2000")
    ));
    let params = Params::from_json_str(&json).unwrap();

    let rendered = render(&params, &template).unwrap();

    let expected = template.replace(
        "<triggerDelay unit=\"10ns\">1748</triggerDelay>",
        "<triggerDelay unit=\"10ns\">2000</triggerDelay>",
    );
    assert_eq!(rendered.xml, expected);
}

/// 同名要素の罠: `AsAd/Generator/triggerDelay`(WCK periods)は
/// `Module/triggerDelay`(10ns)と別物。除去も書き込みも**パスで**行うので触られない。
#[test]
fn the_asad_generator_trigger_delay_is_a_different_knob_and_stays_untouched() {
    let template = template_mini();
    let json = params_json(&format!(
        r#"{}, "exceptions":[]"#,
        knobs_with("\"triggerDelay\":1748", "\"triggerDelay\":2000")
    ));
    let params = Params::from_json_str(&json).unwrap();

    let rendered = render(&params, &template).unwrap();

    assert!(rendered
        .xml
        .contains("<triggerDelay unit=\"WCK periods\" range=\"[0, 31]\">0</triggerDelay>"));
    assert_eq!(rendered.xml.matches("<triggerDelay").count(), 2);
}

// ---------------------------------------------------------------------
// §3-1 除去規則(負性テスト)
// ---------------------------------------------------------------------

/// **最重要の罠**: テンプレートに残った明示 id ノードは JSON の一様値に勝ってしまう。
/// レンダラは管理ノブの全インスタンスノードを除去してから `*` に書く。
#[test]
fn stale_explicit_overrides_of_managed_knobs_are_removed() {
    let template = fixture("template_stale_overrides.xcfg");
    let params = Params::from_json_str(&params_json(&format!(
        r#"{}, "exceptions":[]"#,
        knobs_template_values()
    )))
    .unwrap();

    let rendered = render(&params, &template).unwrap();

    // 仕込んだ 4 か所が消えている(フィクスチャ冒頭のコメントにも同じ数字が出るので、
    // 要素の形で確かめる)。
    assert!(
        !rendered
            .xml
            .contains("<triggerDelay unit=\"10ns\">999</triggerDelay>"),
        "Instance[0] の triggerDelay 999 が残っている"
    );
    assert!(
        !rendered
            .xml
            .contains("<GlobalThresholdValue>6</GlobalThresholdValue>"),
        "Aget[1] の GlobalThresholdValue 6 が残っている"
    );
    assert!(
        !rendered
            .xml
            .contains("<LSBThresholdValue>0</LSBThresholdValue>"),
        "channel[4] の LSBThresholdValue 0 が残っている"
    );
    assert!(
        !rendered
            .xml
            .contains("<LSBThresholdValue>2</LSBThresholdValue>"),
        "AsAd[1]/Aget[0]/channel[3] の LSBThresholdValue 2 が残っている"
    );
    assert!(
        !rendered.xml.contains("<Gain>1pC</Gain>"),
        "AsAd[1]/Aget[0]/channel[3] の Gain 1pC が残っている"
    );

    // 管理ノブは `*` ノードに 1 個ずつだけ残る(Module.triggerDelay と AsAd.Generator.triggerDelay で 2)。
    assert_eq!(rendered.xml.matches("<triggerDelay").count(), 2);
    assert_eq!(rendered.xml.matches("<GlobalThresholdValue").count(), 1);
    assert_eq!(rendered.xml.matches("<LSBThresholdValue").count(), 1);
    assert_eq!(rendered.xml.matches("<Gain").count(), 1);

    // 2026-09-22 に追加した 4 ノブの仕込み(5〜8)も消えている。
    assert!(
        !rendered
            .xml
            .contains("<peackingTime unit=\"ns\">70</peackingTime>"),
        "Aget[1] の peackingTime 70 が残っている"
    );
    assert!(
        !rendered
            .xml
            .contains("<isAllChannelRead>false</isAllChannelRead>"),
        "Aget[1] の isAllChannelRead false が残っている"
    );
    assert!(
        !rendered
            .xml
            .contains("<zeroSuppressionThreshold>11</zeroSuppressionThreshold>"),
        "channel[4] の zeroSuppressionThreshold 11 が残っている"
    );
    assert!(
        !rendered
            .xml
            .contains("<enableZeroSuppression>true</enableZeroSuppression>"),
        "Instance[0] の enableZeroSuppression true が残っている"
    );
    assert!(
        !rendered
            .xml
            .contains("<periodicModeEnabled>true</periodicModeEnabled>"),
        "Instance[0] の periodicModeEnabled true が残っている"
    );
    assert_eq!(rendered.xml.matches("<peackingTime").count(), 1);
    assert_eq!(rendered.xml.matches("<isAllChannelRead").count(), 1);
    assert_eq!(rendered.xml.matches("<zeroSuppressionThreshold").count(), 1);
    assert_eq!(rendered.xml.matches("<enableZeroSuppression").count(), 1);
    assert_eq!(rendered.xml.matches("<periodicModeEnabled").count(), 1);

    // 非管理ノブは無傷(明示ノードそのものは消さない — 空になっても残す)。
    assert!(rendered.xml.contains("<coboId>1</coboId>"));
    assert!(rendered
        .xml
        .contains("<zeroSuppressionThreshold range=\"[0, 4095]\">600</zeroSuppressionThreshold>"));
    assert!(rendered.xml.contains("<Aget id=\"1\">"));
    assert!(rendered.xml.contains("<channel id=\"3\">"));
}

// ---------------------------------------------------------------------
// §2 例外リスト
// ---------------------------------------------------------------------

/// ノブのスコープぴったりの例外 → 明示 id ノードに書かれる。
#[test]
fn an_exception_at_the_knob_scope_is_written_as_an_explicit_node() {
    let template = template_mini();
    let json = params_json(&format!(
        r#"{}, "exceptions":[{{"at":{{"asad":1,"aget":0,"ch":3}},"knob":"lsbThreshold","value":3,"note":"noisy channel (068 #4)"}}]"#,
        knobs_template_values()
    ));
    let params = Params::from_json_str(&json).unwrap();

    let rendered = render(&params, &template).unwrap();

    // AsAd[1] は既にある明示ノード、Aget[0]/channel[3] は新設。
    assert!(rendered
        .xml
        .contains("<LSBThresholdValue>3</LSBThresholdValue>"));
    assert_eq!(rendered.xml.matches("<LSBThresholdValue").count(), 2);
    // 一様値は `*` 側に残る。
    assert!(rendered
        .xml
        .contains("<LSBThresholdValue range=\"[0, 15]\">15</LSBThresholdValue>"));
}

/// スコープより**粗い**例外は許す(§2)が、下の段は `*` では**なく**テンプレートに
/// 実在する id へ展開する。
///
/// 078 §2 は「`{"asad":1}` → `<AsAd id="1"><Aget id="*"><channel id="*">`」と書いているが、
/// **その形では効かない**ことを ccfg_resolve(実 CompoundConfig)で実測した
/// (2026-09-22、`tests/params_ccfg_oracle.rs`)。CompoundConfig の探索は、ある段で明示 id が
/// 無いときその場で `*` に落ちてくれないので、`AsAd[1]/Aget[*]` は `Aget[0]` の解決に使われず、
/// 探索は `AsAd[*]` の連鎖へ戻って**一様値が勝って**しまう。
/// そこで粗い例外は (aget, ch) の直積に展開する — 意味(AsAd1 全体)は変わらない。
#[test]
fn an_exception_coarser_than_the_knob_scope_expands_to_explicit_nodes() {
    let template = template_mini();
    let json = params_json(&format!(
        r#"{}, "exceptions":[{{"at":{{"asad":1}},"knob":"lsbThreshold","value":7}}]"#,
        knobs_template_values()
    ));
    let params = Params::from_json_str(&json).unwrap();

    let rendered = render(&params, &template).unwrap();

    let asad1 = rendered
        .xml
        .split("<AsAd id=\"1\">")
        .nth(1)
        .expect("AsAd[1] が無い");
    assert!(
        !asad1.contains("<Aget id=\"*\">"),
        "粗い例外を `*` で書いてはいけない:\n{asad1}"
    );
    // フィクスチャは Aget 0..1 × channel 0..5 → 12 個の明示ノードに展開される。
    assert_eq!(
        rendered
            .xml
            .matches("<LSBThresholdValue>7</LSBThresholdValue>")
            .count(),
        12
    );
    assert!(asad1.contains("<Aget id=\"0\">"));
    assert!(asad1.contains("<channel id=\"5\">"));
}

/// スコープより**細かい**例外は拒否(黙って丸めない — §2)。
#[test]
fn an_exception_finer_than_the_knob_scope_is_rejected() {
    let json = params_json(&format!(
        r#"{}, "exceptions":[{{"at":{{"asad":1}},"knob":"triggerDelay","value":800}}]"#,
        knobs_template_values()
    ));

    let err = Params::from_json_str(&json).unwrap_err();

    assert!(
        matches!(err, ParamsError::ExceptionTooFine { .. }),
        "unexpected error: {err}"
    );
}

/// `at` は (asad, aget, ch) の**接頭辞**でなければならない(粗さ = 末尾の切り落とし)。
#[test]
fn an_exception_target_that_is_not_a_prefix_is_rejected() {
    let json = params_json(&format!(
        r#"{}, "exceptions":[{{"at":{{"ch":3}},"knob":"lsbThreshold","value":3}}]"#,
        knobs_template_values()
    ));

    let err = Params::from_json_str(&json).unwrap_err();

    assert!(
        matches!(err, ParamsError::ExceptionAtNotPrefix { .. }),
        "unexpected error: {err}"
    );
}

/// 空の `at` も拒否(一様値と区別が付かない)。
#[test]
fn an_empty_exception_target_is_rejected() {
    let json = params_json(&format!(
        r#"{}, "exceptions":[{{"at":{{}},"knob":"lsbThreshold","value":3}}]"#,
        knobs_template_values()
    ));

    let err = Params::from_json_str(&json).unwrap_err();

    assert!(
        matches!(err, ParamsError::ExceptionAtNotPrefix { .. }),
        "unexpected error: {err}"
    );
}

/// テンプレートに存在しない id を指す例外は**レンダラが**拒否する
/// (ch 数をコードに焼き込まない — CLAUDE.md。テンプレートが唯一の真実)。
#[test]
fn an_exception_target_absent_from_the_template_is_rejected() {
    let template = template_mini();
    let json = params_json(&format!(
        r#"{}, "exceptions":[{{"at":{{"asad":7}},"knob":"lsbThreshold","value":3}}]"#,
        knobs_template_values()
    ));
    let params = Params::from_json_str(&json).unwrap();

    let err = render(&params, &template).unwrap_err();

    assert!(
        matches!(err, ParamsError::TargetNotInTemplate { .. }),
        "unexpected error: {err}"
    );
}

/// §3-4: 有効値の解決は JSON 内で完結する(most specific wins)。
#[test]
fn effective_value_resolution_prefers_the_most_specific_exception() {
    let json = params_json(&format!(
        r#"{}, "exceptions":[
            {{"at":{{"asad":1}},"knob":"lsbThreshold","value":7}},
            {{"at":{{"asad":1,"aget":0,"ch":3}},"knob":"lsbThreshold","value":3}}
        ]"#,
        knobs_template_values()
    ));
    let params = Params::from_json_str(&json).unwrap();

    // 一様値 15 / AsAd1 全体 7 / AsAd1-Aget0-ch3 だけ 3。
    assert_eq!(
        params.effective_value("lsbThreshold", 0, 0, 0).unwrap(),
        &serde_json::json!(15)
    );
    assert_eq!(
        params.effective_value("lsbThreshold", 1, 1, 5).unwrap(),
        &serde_json::json!(7)
    );
    assert_eq!(
        params.effective_value("lsbThreshold", 1, 0, 3).unwrap(),
        &serde_json::json!(3)
    );
}

// ---------------------------------------------------------------------
// §2 スキーマ検証
// ---------------------------------------------------------------------

#[test]
fn an_unknown_schema_id_is_rejected() {
    let json = r#"{"schema":"tpcdaq-params/2","template":"configure-mini-fixture.xcfg","knobs":{},"exceptions":[]}"#;

    let err = Params::from_json_str(json).unwrap_err();

    assert!(
        matches!(err, ParamsError::UnknownSchema { .. }),
        "unexpected error: {err}"
    );
}

#[test]
fn out_of_range_and_unknown_enum_values_are_rejected() {
    // globalThreshold の range は [0, 7](xcfg 実物)。
    let too_big = params_json(&format!(
        r#"{}, "exceptions":[]"#,
        knobs_with("\"globalThreshold\":1", "\"globalThreshold\":8")
    ));
    assert!(matches!(
        Params::from_json_str(&too_big).unwrap_err(),
        ParamsError::OutOfRange { .. }
    ));

    // Gain の enum は {10pC, 120fC, 1pC, 240fC}。
    let bad_gain = params_json(&format!(
        r#"{}, "exceptions":[]"#,
        knobs_with("\"gain\":\"120fC\"", "\"gain\":\"5pC\"")
    ));
    assert!(matches!(
        Params::from_json_str(&bad_gain).unwrap_err(),
        ParamsError::NotInEnum { .. }
    ));

    // triggerMode は zCoBo FW が扱える 5 値だけ(§2.1 裁定 1)。
    let mult = params_json(&format!(
        r#"{}, "exceptions":[]"#,
        knobs_with(
            "\"triggerMode\":\"external\"",
            "\"triggerMode\":\"onMultiplicity\""
        )
    ));
    assert!(matches!(
        Params::from_json_str(&mult).unwrap_err(),
        ParamsError::NotInEnum { .. }
    ));

    // writingClockFrequency_MHz は enum {12.5, 25, 50, 100}。
    let odd_clock = params_json(&format!(
        r#"{}, "exceptions":[]"#,
        knobs_with(
            "\"writingClockFrequency_MHz\":25.0",
            "\"writingClockFrequency_MHz\":33.0"
        )
    ));
    assert!(matches!(
        Params::from_json_str(&odd_clock).unwrap_err(),
        ParamsError::NotInEnum { .. }
    ));
}

#[test]
fn a_missing_or_unknown_knob_is_rejected() {
    let missing = params_json(
        r#""knobs":{"triggerDelay":1748,"writingClockFrequency_MHz":25.0,"triggerMode":"external","globalThreshold":1,"lsbThreshold":15,"gain":"120fC"}, "exceptions":[]"#,
    );
    assert!(matches!(
        Params::from_json_str(&missing).unwrap_err(),
        ParamsError::MissingKnob { .. }
    ));

    // 078 §2.1 裁定 1: multiplicity / 2p 系はスキーマ違反として拒否する。
    let knobs = format!(r#""knobs":{{{KNOBS_BODY},"multiplicityThreshold":50}}"#);
    let unknown = params_json(&format!(r#"{knobs}, "exceptions":[]"#));
    assert!(matches!(
        Params::from_json_str(&unknown).unwrap_err(),
        ParamsError::UnknownKnob { .. }
    ));
}

#[test]
fn an_exception_on_an_unknown_knob_is_rejected() {
    let json = params_json(&format!(
        r#"{}, "exceptions":[{{"at":{{"asad":1}},"knob":"multiplicityThreshold","value":50}}]"#,
        knobs_template_values()
    ));

    assert!(matches!(
        Params::from_json_str(&json).unwrap_err(),
        ParamsError::UnknownKnob { .. }
    ));
}

#[test]
fn an_exception_value_is_range_checked_like_the_uniform_value() {
    let json = params_json(&format!(
        r#"{}, "exceptions":[{{"at":{{"asad":1,"aget":0,"ch":3}},"knob":"lsbThreshold","value":16}}]"#,
        knobs_template_values()
    ));

    assert!(matches!(
        Params::from_json_str(&json).unwrap_err(),
        ParamsError::OutOfRange { .. }
    ));
}

// ---------------------------------------------------------------------
// §3-3 生成物の名前
// ---------------------------------------------------------------------

/// `<id>` = `<テンプレート stem(相のプレフィクスを除く)>__<JSON の sha256 先頭 8 桁>`、
/// configure / prepare は同内容。
///
/// 期待値の出典:
/// ```text
/// $ shasum -a 256 tests/fixtures/params/params_roundtrip.json
/// 1f04446c4dd2c8f27f02eb08208378537d2f48d9b27598956684f428af4327ba
/// ```
#[test]
fn the_config_id_is_the_template_stem_plus_the_json_hash() {
    let template = template_mini();
    let params = Params::from_json_str(&params_json_template_values()).unwrap();

    let rendered = render(&params, &template).unwrap();

    assert_eq!(rendered.config_id, "mini-fixture__1f04446c");
    assert_eq!(
        rendered.configure_file_name,
        "configure-mini-fixture__1f04446c.xcfg"
    );
    assert_eq!(
        rendered.prepare_file_name,
        "prepare-mini-fixture__1f04446c.xcfg"
    );
}

/// 同じ JSON → 同じ id(= 既存ファイルを壊さない)。違う JSON → 違う id。
#[test]
fn the_config_id_changes_when_the_json_changes() {
    let template = template_mini();
    let a = Params::from_json_str(&params_json(&format!(
        r#"{}, "exceptions":[]"#,
        knobs_template_values()
    )))
    .unwrap();
    let b = Params::from_json_str(&params_json(&format!(
        r#"{}, "exceptions":[]"#,
        knobs_with("\"triggerDelay\":1748", "\"triggerDelay\":1749")
    )))
    .unwrap();

    let ra = render(&a, &template).unwrap();
    let rb = render(&b, &template).unwrap();

    assert_ne!(ra.config_id, rb.config_id);
    assert_eq!(ra.config_id, render(&a, &template).unwrap().config_id);
}

// ---------------------------------------------------------------------
// 2026-09-22 レビュー裁定で追加した 4 ノブ
// ---------------------------------------------------------------------

/// `peakingTime_ns` は **AGET スコープ**(§2 表の channel は誤り)。要素名は GET の綴り。
#[test]
fn peaking_time_is_an_aget_scope_knob_with_the_get_spelling() {
    let template = template_mini();
    let json = params_json(&format!(
        r#"{}, "exceptions":[{{"at":{{"asad":1,"aget":1}},"knob":"peakingTime_ns","value":117}}]"#,
        knobs_with("\"peakingTime_ns\":232", "\"peakingTime_ns\":334")
    ));
    let params = Params::from_json_str(&json).unwrap();

    let rendered = render(&params, &template).unwrap();

    // 一様値は `*` ノードで属性ごと保たれる。
    assert!(rendered.xml.contains(
        "<peackingTime unit=\"ns\" range=\"{70, 117, 232, 280, 334, 383, 502, 541, 568, 632, 721, 760, 831, 870, 976, 1014}\">334</peackingTime>"
    ));
    // 例外は AsAd[1]/Aget[1] の明示ノードへ(AGET スコープなので展開は起きない)。
    let asad1 = rendered.xml.split("<AsAd id=\"1\">").nth(1).unwrap();
    assert!(asad1.contains("<peackingTime>117</peackingTime>"));
    assert_eq!(rendered.xml.matches("<peackingTime").count(), 2);
}

/// AGET 仕様の 16 値だけを受ける。定数表 `PEAKING_TIME_NS` が唯一の定義で、
/// テンプレートの `range` 属性と一致することをここで確かめる(焼き込みの検算)。
#[test]
fn the_peaking_time_table_matches_the_template_range_attribute() {
    let template = template_mini();
    let range: String = PEAKING_TIME_NS
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    assert!(
        template.contains(&format!("range=\"{{{range}}}\"")),
        "PEAKING_TIME_NS がテンプレートの range と違う: {range}"
    );

    // 16 値に無い 200 ns は拒否。
    let bad = params_json(&format!(
        r#"{}, "exceptions":[]"#,
        knobs_with("\"peakingTime_ns\":232", "\"peakingTime_ns\":200")
    ));
    assert!(matches!(
        Params::from_json_str(&bad).unwrap_err(),
        ParamsError::NotInEnum { .. }
    ));
}

/// `readout` は `isAllChannelRead` だけ(full → true、partial → false)。`isFPNRead` は別ノブ
/// `fpnRead`(TODO/111 §5.4 — mini の現用 = false/true を表すため 2026-10-10 に分けた)。
#[test]
fn readout_and_fpn_read_each_flip_only_their_own_aget_element() {
    let template = template_mini();

    let full = Params::from_json_str(&params_json(&format!(
        r#"{}, "exceptions":[]"#,
        knobs_template_values()
    )))
    .unwrap();
    let rendered_full = render(&full, &template).unwrap();
    assert!(rendered_full
        .xml
        .contains("<isAllChannelRead>true</isAllChannelRead>"));
    assert!(rendered_full.xml.contains("<isFPNRead>true</isFPNRead>"));

    let partial = Params::from_json_str(&params_json(&format!(
        r#"{}, "exceptions":[]"#,
        knobs_with("\"readout\":\"full\"", "\"readout\":\"partial\"")
    )))
    .unwrap();
    let rendered_partial = render(&partial, &template).unwrap();
    assert!(rendered_partial
        .xml
        .contains("<isAllChannelRead>false</isAllChannelRead>"));
    // full との差は isAllChannelRead だけ(isFPNRead も非管理ノブも無傷)。
    assert_eq!(
        rendered_partial.xml,
        rendered_full.xml.replace(
            "<isAllChannelRead>true</isAllChannelRead>",
            "<isAllChannelRead>false</isAllChannelRead>"
        )
    );

    // mini の現用の組(isAllChannelRead=false + isFPNRead=true)は partial + fpnRead=true。
    let no_fpn = Params::from_json_str(&params_json(&format!(
        r#"{}, "exceptions":[]"#,
        knobs_with("\"fpnRead\":true", "\"fpnRead\":false")
    )))
    .unwrap();
    let rendered_no_fpn = render(&no_fpn, &template).unwrap();
    assert_eq!(
        rendered_no_fpn.xml,
        rendered_full.xml.replace(
            "<isFPNRead>true</isFPNRead>",
            "<isFPNRead>false</isFPNRead>"
        )
    );
}

#[test]
fn fpn_read_must_be_a_bool() {
    let json = params_json(&format!(
        r#"{}, "exceptions":[]"#,
        knobs_with("\"fpnRead\":true", "\"fpnRead\":\"yes\"")
    ));
    assert!(matches!(
        Params::from_json_str(&json).unwrap_err(),
        ParamsError::BadType { .. }
    ));
}

#[test]
fn an_unknown_readout_mode_is_rejected() {
    let json = params_json(&format!(
        r#"{}, "exceptions":[]"#,
        knobs_with("\"readout\":\"full\"", "\"readout\":\"zeroSuppressed\"")
    ));
    assert!(matches!(
        Params::from_json_str(&json).unwrap_err(),
        ParamsError::NotInEnum { .. }
    ));
}

/// `zeroSuppression` は JSON では 1 オブジェクトだが、xcfg ではスコープの違う 2 要素。
/// 例外は threshold(channel)にだけ意味があり、enabled(CoBo)への例外は拒否される。
#[test]
fn zero_suppression_is_two_knobs_with_different_scopes() {
    let template = template_mini();
    let json = params_json(&format!(
        r#"{}, "exceptions":[{{"at":{{"asad":0,"aget":1,"ch":2}},"knob":"zeroSuppression.threshold","value":42}}]"#,
        knobs_with(
            "\"zeroSuppression\":{\"enabled\":false,\"threshold\":600}",
            "\"zeroSuppression\":{\"enabled\":true,\"threshold\":700}"
        )
    ));
    let params = Params::from_json_str(&json).unwrap();

    let rendered = render(&params, &template).unwrap();

    assert!(rendered
        .xml
        .contains("<enableZeroSuppression>true</enableZeroSuppression>"));
    assert!(rendered
        .xml
        .contains("<zeroSuppressionThreshold range=\"[0, 4095]\">700</zeroSuppressionThreshold>"));
    assert!(rendered
        .xml
        .contains("<zeroSuppressionThreshold>42</zeroSuppressionThreshold>"));

    // enabled は CoBo スコープ = どんな `at` もスコープより細かい → 拒否。
    let bad = params_json(&format!(
        r#"{}, "exceptions":[{{"at":{{"asad":0}},"knob":"zeroSuppression.enabled","value":true}}]"#,
        knobs_template_values()
    ));
    assert!(matches!(
        Params::from_json_str(&bad).unwrap_err(),
        ParamsError::ExceptionTooFine { .. }
    ));
}

/// `pulser` は **CoBo スコープ**の `Instance.Generator`(AsAd の Generator ではない)。
/// `voltage_mV` は `amplitudeStart` と同値(GET の `asadPulserVoltage_mV` がそのまま読む)。
#[test]
fn pulser_fields_go_to_the_cobo_generator() {
    let template = template_mini();
    let json = params_json(&format!(
        r#"{}, "exceptions":[]"#,
        knobs_with(
            "\"pulser\":{\"enabled\":false,\"voltage_mV\":-1000.0,\"period_ms\":1000}",
            "\"pulser\":{\"enabled\":true,\"voltage_mV\":-300.0,\"period_ms\":200}"
        )
    ));
    let params = Params::from_json_str(&json).unwrap();

    let rendered = render(&params, &template).unwrap();

    assert!(rendered
        .xml
        .contains("<periodicModeEnabled>true</periodicModeEnabled>"));
    assert!(rendered.xml.contains(
        "<amplitudeStart unit=\"mV\" range=\"[-1000.0, 1000.0]\">-300.0</amplitudeStart>"
    ));
    assert!(rendered
        .xml
        .contains("<period_ms unit=\"ms\">200</period_ms>"));
    // AsAd 配下の Generator(isCalibMode / triggerDelay)は触らない。
    assert!(rendered.xml.contains("<isCalibMode>false</isCalibMode>"));
    assert_eq!(rendered.xml.matches("<periodicModeEnabled").count(), 1);
}

/// pulser の電圧は xcfg の range [-1000, 1000] mV の外を拒否する。
#[test]
fn a_pulser_voltage_outside_the_xcfg_range_is_rejected() {
    let json = params_json(&format!(
        r#"{}, "exceptions":[]"#,
        knobs_with("\"voltage_mV\":-1000.0", "\"voltage_mV\":-1200.0")
    ));
    assert!(matches!(
        Params::from_json_str(&json).unwrap_err(),
        ParamsError::OutOfRange { .. }
    ));
}

/// 入れ子キーの打ち間違いは弾く(黙って既定値にしない)。
#[test]
fn a_misspelled_nested_knob_key_is_rejected() {
    let json = params_json(&format!(
        r#"{}, "exceptions":[]"#,
        knobs_with("\"period_ms\":1000", "\"periodMs\":1000")
    ));
    let err = Params::from_json_str(&json).unwrap_err();
    assert!(
        matches!(
            err,
            ParamsError::MissingKnob { .. } | ParamsError::UnknownKnob { .. }
        ),
        "unexpected error: {err}"
    );
}
