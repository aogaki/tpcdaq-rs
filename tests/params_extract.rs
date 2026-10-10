//! 111 レーン A — テンプレート xcfg → パラメータ JSON の**抽出**(render の逆)の受け入れテスト。
//!
//! 契約は TODO/111 §5.0: `knobs` = `*` 連鎖から読めて型・範囲に収まった葉、`unset` = `*` 連鎖に
//! 要素が無い葉、`unsupported` = 要素はあるが表せないもの、`exceptions` = 明示 id ノードの値。
//!
//! フィクスチャは合成の縮小テンプレート(`tests/fixtures/params/`)。実テンプレート 3 本の
//! バイト一致往復は env `TPCDAQ_PARAMS_REAL_TEMPLATES`(`:` 区切り)がある時だけ走る(リポには入れない)。

#![allow(clippy::unwrap_used)]

use serde_json::{json, Value};
use tpcdaq::params::{extract, render, Extracted, Params, SCHEMA_ID};

fn fixture(name: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/params/{}",
        env!("CARGO_MANIFEST_DIR"),
        name
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"))
}

/// `extract` の結果を `Params` に組む(UI が Save するときと同じ: knobs + exceptions だけ)。
fn params_from(extracted: &Extracted) -> Params {
    let json = extracted.to_json();
    let params = json!({
        "schema": SCHEMA_ID,
        "template": json["template"],
        "knobs": json["knobs"],
        "exceptions": json["exceptions"],
    });
    Params::from_json_str(&params.to_string()).unwrap()
}

/// 文字列置換でフィクスチャを変形する。置換対象が無ければ落とす(黙って素通りしない)。
fn replaced(xml: &str, from: &str, to: &str) -> String {
    assert!(xml.contains(from), "fixture does not contain {from:?}");
    xml.replacen(from, to, 1)
}

// ---------------------------------------------------------------------
// (a) 合成テンプレートの現行値 = 078 の往復オラクル params_roundtrip.json の knobs
// ---------------------------------------------------------------------

#[test]
fn the_mini_template_extracts_to_the_roundtrip_knobs_with_nothing_else() {
    let extracted = extract(
        "configure-mini-fixture.xcfg",
        &fixture("template_mini.xcfg"),
    )
    .unwrap();
    let json = extracted.to_json();

    let oracle: Value = serde_json::from_str(&fixture("params_roundtrip.json")).unwrap();
    assert_eq!(json["template"], json!("configure-mini-fixture.xcfg"));
    assert_eq!(json["knobs"], oracle["knobs"]);
    assert_eq!(json["exceptions"], json!([]));
    assert_eq!(json["unset"], json!([]));
    assert_eq!(json["unsupported"], json!([]));
}

/// 実数ノブは JSON でも実数(`25.0`。UI のフォームと `Params` の検証が整数と取り違えない)。
#[test]
fn real_knobs_come_out_as_json_floats() {
    let extracted = extract("t.xcfg", &fixture("template_mini.xcfg")).unwrap();
    let json = extracted.to_json();

    assert!(json["knobs"]["writingClockFrequency_MHz"].is_f64());
    assert!(json["knobs"]["pulser"]["voltage_mV"].is_f64());
    assert!(json["knobs"]["triggerDelay"].is_i64());
}

// ---------------------------------------------------------------------
// (b) 明示 id ノードの値: 例外 / 表せないもの
// ---------------------------------------------------------------------

/// `template_stale_overrides.xcfg` の仕込み 8 か所(フィクスチャ冒頭のコメント)のうち、
/// 例外として表せるのは 4. の `AsAd[1].Aget[0].channel[3]` の 2 値だけ。
/// `*` の下の明示 id(2, 3, 5, 6, 7)と CoBo の明示インスタンス(1, 8)は `unsupported`。
#[test]
fn explicit_nodes_become_exceptions_and_unrepresentable_ones_are_unsupported() {
    let extracted = extract(
        "configure-mini-fixture.xcfg",
        &fixture("template_stale_overrides.xcfg"),
    )
    .unwrap();
    let json = extracted.to_json();

    // 一様値(`*` 連鎖)は template_mini と同じ。
    let oracle: Value = serde_json::from_str(&fixture("params_roundtrip.json")).unwrap();
    assert_eq!(json["knobs"], oracle["knobs"]);
    assert_eq!(json["unset"], json!([]));

    // 仕込み 4: AsAd[1]/Aget[0]/channel[3]/{LSBThresholdValue=2, Gain=1pC}
    assert_eq!(
        json["exceptions"],
        json!([
            {"at": {"asad": 1, "aget": 0, "ch": 3}, "knob": "lsbThreshold", "value": 2},
            {"at": {"asad": 1, "aget": 0, "ch": 3}, "knob": "gain", "value": "1pC"},
        ])
    );

    let below = "explicit id below a wildcard";
    let instance = "explicit CoBo instance";
    let unsupported: Vec<(String, String, String)> = json["unsupported"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| {
            (
                u["knob"].as_str().unwrap().to_string(),
                u["found"].as_str().unwrap().to_string(),
                u["why"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    // KNOBS の順に並ぶ。
    let expected: Vec<(String, String, String)> = [
        ("triggerDelay", "999", instance),             // 仕込み 1
        ("globalThreshold", "6", below),               // 仕込み 2
        ("lsbThreshold", "0", below),                  // 仕込み 3
        ("peakingTime_ns", "70", below),               // 仕込み 5
        ("readout", "isAllChannelRead=false", below),  // 仕込み 6
        ("zeroSuppression.enabled", "true", instance), // 仕込み 8
        ("zeroSuppression.threshold", "11", below),    // 仕込み 7
        ("pulser.enabled", "true", instance),          // 仕込み 8
    ]
    .iter()
    .map(|(k, f, w)| (k.to_string(), f.to_string(), w.to_string()))
    .collect();
    assert_eq!(unsupported, expected);
}

/// 粗い明示 id(`AsAd[1].Aget[*].channel[*]`)は `at` = 明示 id の接頭辞だけ(§5.0)。
#[test]
fn a_coarse_explicit_node_gives_an_exception_at_the_explicit_prefix() {
    let xml = replaced(
        &fixture("template_mini.xcfg"),
        "\t\t\t<AsAd id=\"1\">\n",
        "\t\t\t<AsAd id=\"1\">\n\t\t\t\t<Aget id=\"*\">\n\t\t\t\t\t<channel id=\"*\">\n\t\t\t\t\t\t<Gain>240fC</Gain>\n\t\t\t\t\t</channel>\n\t\t\t\t</Aget>\n\t\t\t\t<Aget id=\"1\">\n\t\t\t\t\t<Global>\n\t\t\t\t\t\t<Reg1>\n\t\t\t\t\t\t\t<isAllChannelRead>false</isAllChannelRead>\n\t\t\t\t\t\t\t<isFPNRead>false</isFPNRead>\n\t\t\t\t\t\t</Reg1>\n\t\t\t\t\t</Global>\n\t\t\t\t</Aget>\n",
    );

    let json = extract("t.xcfg", &xml).unwrap().to_json();

    // KNOBS の順。readout(合成ノブ)は 2 要素が同じノードで同じモード(partial = false/false)なら 1 件。
    assert_eq!(
        json["exceptions"],
        json!([
            {"at": {"asad": 1}, "knob": "gain", "value": "240fC"},
            {"at": {"asad": 1, "aget": 1}, "knob": "readout", "value": "partial"},
        ])
    );
    assert_eq!(json["unsupported"], json!([]));
}

// ---------------------------------------------------------------------
// (c) 冪等性: extract → Params → render → extract が最初の抽出と一致
// ---------------------------------------------------------------------

fn assert_idempotent(template: &str) -> (Value, Value) {
    let first = extract("configure-mini-fixture.xcfg", template).unwrap();
    let rendered = render(&params_from(&first), template).unwrap();
    let second = extract("configure-mini-fixture.xcfg", &rendered.xml).unwrap();

    let (first, second) = (first.to_json(), second.to_json());
    // Params に入るのは knobs + exceptions だけ(unsupported は render が除去規則で消す)。
    assert_eq!(second["knobs"], first["knobs"]);
    assert_eq!(second["exceptions"], first["exceptions"]);
    assert_eq!(second["unset"], json!([]));
    (first, second)
}

#[test]
fn extracting_the_rendered_mini_template_gives_the_same_result() {
    let (first, second) = assert_idempotent(&fixture("template_mini.xcfg"));
    assert_eq!(second, first);
}

#[test]
fn extracting_the_rendered_stale_template_keeps_the_exceptions_and_drops_the_unsupported() {
    let (first, second) = assert_idempotent(&fixture("template_stale_overrides.xcfg"));
    assert_eq!(first["exceptions"].as_array().unwrap().len(), 2);
    // render の除去規則(078 §3-1)が表せない明示ノードを消している。
    assert_eq!(second["unsupported"], json!([]));
}

// ---------------------------------------------------------------------
// (d)(e)(f) 表せない / 無い値は黙って埋めない
// ---------------------------------------------------------------------

/// `readout` の 2 要素が同じモードに写らない(true + false)→ `unsupported`、`knobs` に無い。
#[test]
fn a_composite_knob_whose_parts_disagree_is_unsupported() {
    let xml = replaced(
        &fixture("template_mini.xcfg"),
        "<isFPNRead>true</isFPNRead>",
        "<isFPNRead>false</isFPNRead>",
    );

    let json = extract("t.xcfg", &xml).unwrap().to_json();

    assert_eq!(json["knobs"].get("readout"), None);
    let unsupported = json["unsupported"].as_array().unwrap();
    assert_eq!(unsupported.len(), 1, "{unsupported:?}");
    assert_eq!(unsupported[0]["knob"], json!("readout"));
    assert_eq!(
        unsupported[0]["found"],
        json!("isAllChannelRead=true,isFPNRead=false")
    );
    assert_eq!(json["unset"], json!([]));
}

/// `Generator.period_ms` が無い → `unset` に `pulser.period_ms`、ほかの pulser の葉は読める。
#[test]
fn a_missing_leaf_is_listed_as_unset_and_not_filled_with_a_default() {
    let xml = replaced(
        &fixture("template_mini.xcfg"),
        "\n\t\t\t\t<period_ms unit=\"ms\">1000</period_ms>",
        "",
    );

    let json = extract("t.xcfg", &xml).unwrap().to_json();

    assert_eq!(json["unset"], json!(["pulser.period_ms"]));
    assert_eq!(
        json["knobs"]["pulser"],
        json!({"enabled": false, "voltage_mV": -1000.0})
    );
    assert_eq!(json["knobs"]["triggerDelay"], json!(1748));
    assert_eq!(json["unsupported"], json!([]));
}

/// zCoBo FW が扱えない `onMultiplicity` → `unsupported`(テキストそのまま)、`knobs` に無い。
#[test]
fn a_value_outside_the_knob_kind_is_unsupported_with_the_text_found() {
    let xml = replaced(
        &fixture("template_mini.xcfg"),
        ">external</triggerMode>",
        ">onMultiplicity</triggerMode>",
    );

    let json = extract("t.xcfg", &xml).unwrap().to_json();

    assert_eq!(json["knobs"].get("triggerMode"), None);
    let unsupported = json["unsupported"].as_array().unwrap();
    assert_eq!(unsupported.len(), 1, "{unsupported:?}");
    assert_eq!(unsupported[0]["knob"], json!("triggerMode"));
    assert_eq!(unsupported[0]["found"], json!("onMultiplicity"));
    // 理由は schema の検証と同じ文言(許容値の一覧を含む)。
    let why = unsupported[0]["why"].as_str().unwrap();
    assert!(why.contains("external"), "{why}");
    assert_eq!(json["unset"], json!([]));
}

/// 整数ノブに整数でないテキスト → `unsupported`(パースできないものを 0 等で埋めない)。
#[test]
fn a_non_numeric_text_in_an_integer_knob_is_unsupported() {
    let xml = replaced(
        &fixture("template_mini.xcfg"),
        ">1748</triggerDelay>",
        ">soon</triggerDelay>",
    );

    let json = extract("t.xcfg", &xml).unwrap().to_json();

    assert_eq!(json["knobs"].get("triggerDelay"), None);
    assert_eq!(json["unsupported"][0]["knob"], json!("triggerDelay"));
    assert_eq!(json["unsupported"][0]["found"], json!("soon"));
}

#[test]
fn a_document_without_the_cobo_node_is_an_error() {
    let err = extract("t.xcfg", "<Setup id=\"Conditions\"></Setup>").unwrap_err();
    assert!(err.to_string().contains("CoBo"), "{err}");
}

#[test]
fn a_broken_xml_is_an_error() {
    assert!(extract("t.xcfg", "<Setup><Node id=\"CoBo\"></Setup>").is_err());
}

// ---------------------------------------------------------------------
// 実テンプレート往復(任意回帰 — 実 xcfg はリポに入れない)
// ---------------------------------------------------------------------

/// `TPCDAQ_PARAMS_REAL_TEMPLATES=<a.xcfg>:<b.xcfg>:...` の各ファイルで
/// extract → render がバイト一致(= 「無変更で保存」しても何も化けない — 111 §0 の穴の塞ぎ)。
#[test]
fn real_templates_round_trip_byte_for_byte_through_extract_and_render() {
    let paths = match std::env::var("TPCDAQ_PARAMS_REAL_TEMPLATES") {
        Ok(paths) if !paths.is_empty() => paths,
        _ => {
            eprintln!(
                "SKIP real_templates_round_trip_byte_for_byte_through_extract_and_render: \
                 TPCDAQ_PARAMS_REAL_TEMPLATES not set (実 xcfg はリポに入れない)"
            );
            return;
        }
    };

    for path in paths.split(':').filter(|p| !p.is_empty()) {
        let template = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let name = std::path::Path::new(path)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();

        let extracted = extract(&name, &template).unwrap();
        let json = extracted.to_json();
        assert_eq!(json["unset"], json!([]), "{path}: unset");
        assert_eq!(json["unsupported"], json!([]), "{path}: unsupported");

        let rendered = render(&params_from(&extracted), &template).unwrap();
        assert!(
            rendered.xml == template,
            "{path}: extract → render がバイト一致しない"
        );
        eprintln!(
            "real template {path}: byte-identical, knobs = {}, exceptions = {}",
            json["knobs"], json["exceptions"]
        );
    }
}
