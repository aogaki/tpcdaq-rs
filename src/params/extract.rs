//! テンプレート xcfg → パラメータ JSON の**抽出**(render の逆、TODO/111 §5.0)。
//!
//! Parameters UI の「Load defaults from template」が、ノブの初期値をテンプレート自身から読むために使う
//! (直書きの既定値で mini の 100 MHz が 25 MHz に化ける穴を塞ぐ — 111 §0)。
//!
//! # 規則(111 §5.0)
//!
//! - `knobs`: `Instance[*]` から `*` の連鎖をたどって読めて、かつ [`KnobKind`] の型・範囲に収まった葉だけ。
//! - `unset`: `*` の連鎖に要素が無い葉。**黙って既定値を入れない**(CLAUDE.md)。
//! - `unsupported`: 要素はあるが型・範囲外、合成ノブの構成要素が 1 つのモードに写らない、
//!   または明示 id ノードの値が `exceptions` で表せない(`*` の下の明示 id / CoBo の明示インスタンス)。
//! - `exceptions`: `Instance[*]` の下で明示 id が**先頭から連続する**ノードの値。`at` はその明示 id の接頭辞。
//!
//! パーサは `render.rs` のものを使う(二重に持たない)。値の検査は `schema.rs` の検査をそのまま使うので、
//! ここで `knobs` に入った値は `Params::from_json_str` でも必ず通る。

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{json, Map, Number, Value};

use super::paths::{Scope, INSTANCE, STAR};
use super::render::{find_cobo_node, parse, Element, Node};
use super::schema::{
    check_value, knob_specs, At, Exception, KnobKind, KnobSpec, ParamsError, KNOBS,
};

/// `*` の下に明示 id が来る形(`AsAd[*].Aget[1]…`)。CompoundConfig の探索規則上の意味が
/// render の例外と一致しない(111 §5.0)。
pub const WHY_BELOW_WILDCARD: &str = "explicit id below a wildcard";
/// `Instance[0]` など CoBo の明示インスタンスにある値。`at` は CoBo を区別できない。
pub const WHY_EXPLICIT_INSTANCE: &str = "explicit CoBo instance";
/// 複数要素の合成ノブで、構成要素が同じノードに揃っていない(2026-10-10 以降の KNOBS には無いが、
/// Derived の複数エントリはスキーマ上まだ許されるので残す)。
pub const WHY_PARTIAL_COMPOSITE: &str = "not all parts of the composite knob at the same node";

/// 抽出結果(111 §5.0 の JSON は [`Extracted::to_json`])。
#[derive(Debug, Clone)]
pub struct Extracted {
    pub template: String,
    /// `tpcdaq-params/1` の `knobs` と同じ入れ子(`pulser` → `{enabled, …}`)。
    pub knobs: BTreeMap<String, Value>,
    pub exceptions: Vec<Exception>,
    /// `.` 区切りの葉の名前(例 `pulser.period_ms`)。
    pub unset: Vec<String>,
    pub unsupported: Vec<Unsupported>,
}

/// 読めたが表せなかった値 1 件。`found` は xcfg のテキストそのまま
/// (構成要素が複数の合成ノブは `a=true,b=false` の形)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Unsupported {
    pub knob: String,
    pub found: String,
    pub why: String,
}

impl Extracted {
    /// 111 §5.0 の応答本体。
    pub fn to_json(&self) -> Value {
        let exceptions: Vec<Value> = self
            .exceptions
            .iter()
            .map(|e| json!({"at": at_json(&e.at), "knob": e.knob, "value": e.value}))
            .collect();
        json!({
            "template": self.template,
            "knobs": self.knobs,
            "exceptions": exceptions,
            "unset": self.unset,
            "unsupported": self.unsupported,
        })
    }
}

/// テンプレート xcfg からノブの値を抽出する。**純関数**(ファイル IO はしない)。
///
/// エラーになるのは XML として読めない / `Setup` > `Node[CoBo]` が無いときだけ。
/// 個々のノブの問題は `unset` / `unsupported` に入る。
pub fn extract(template_name: &str, template_xml: &str) -> Result<Extracted, ParamsError> {
    let mut doc = parse(template_xml)?;
    let cobo: &Element = find_cobo_node(&mut doc)?;

    let mut out = Extracted {
        template: template_name.to_string(),
        knobs: BTreeMap::new(),
        exceptions: Vec::new(),
        unset: Vec::new(),
        unsupported: Vec::new(),
    };

    for name in knob_names() {
        let specs: Vec<&KnobSpec> = knob_specs(name).collect();
        extract_uniform(cobo, name, &specs, &mut out);
        extract_explicit(cobo, name, &specs, &mut out);
    }
    Ok(out)
}

/// 一様値 = `*` の連鎖の値。
fn extract_uniform(cobo: &Element, name: &str, specs: &[&KnobSpec], out: &mut Extracted) {
    let texts: Option<Vec<String>> = specs
        .iter()
        .map(|spec| star_chain_text(cobo, spec))
        .collect();
    let Some(texts) = texts else {
        out.unset.push(name.to_string());
        return;
    };
    let entries: Vec<(&KnobSpec, &str)> = specs
        .iter()
        .copied()
        .zip(texts.iter().map(String::as_str))
        .collect();
    match decode(&entries) {
        Ok(value) => insert_knob(&mut out.knobs, name, value),
        Err(why) => out.unsupported.push(Unsupported {
            knob: name.to_string(),
            found: found_text(&entries),
            why,
        }),
    }
}

/// 例外 = `*` の連鎖以外のノードにある値。合成ノブは同じ `at` の構成要素をまとめて 1 件にする。
fn extract_explicit(cobo: &Element, name: &str, specs: &[&KnobSpec], out: &mut Extracted) {
    // (at, 構成要素ごとのテキスト)。非合成ノブは構成要素 1 つ。
    let mut groups: Vec<(At, Vec<Option<String>>)> = Vec::new();
    for (i, spec) in specs.iter().enumerate() {
        for hit in explicit_hits(cobo, spec) {
            match hit.at {
                Err(why) => out.unsupported.push(Unsupported {
                    knob: name.to_string(),
                    found: found_text(&[(*spec, hit.text.as_str())]),
                    why,
                }),
                Ok(at) => {
                    // 同じ at の空き枠に入れる。無ければ新しい組(重複ノードは別の例外になり、
                    // Params の検証が DuplicateException で拒否する — 黙って上書きしない)。
                    match groups
                        .iter_mut()
                        .find(|(a, slots)| *a == at && slots[i].is_none())
                    {
                        Some((_, slots)) => slots[i] = Some(hit.text),
                        None => {
                            let mut slots = vec![None; specs.len()];
                            slots[i] = Some(hit.text);
                            groups.push((at, slots));
                        }
                    }
                }
            }
        }
    }

    for (at, slots) in groups {
        let entries: Vec<(&KnobSpec, &str)> = specs
            .iter()
            .zip(&slots)
            .filter_map(|(spec, text)| text.as_deref().map(|text| (*spec, text)))
            .collect();
        if entries.len() < specs.len() {
            out.unsupported.push(Unsupported {
                knob: name.to_string(),
                found: found_text(&entries),
                why: WHY_PARTIAL_COMPOSITE.to_string(),
            });
            continue;
        }
        match decode(&entries) {
            Ok(value) => out.exceptions.push(Exception {
                at,
                knob: name.to_string(),
                value,
                note: None,
            }),
            Err(why) => out.unsupported.push(Unsupported {
                knob: name.to_string(),
                found: found_text(&entries),
                why,
            }),
        }
    }
}

/// KNOBS に出てくる JSON キー(合成ノブの重複を除いた、表の順)。
fn knob_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = Vec::new();
    for spec in KNOBS {
        if !names.contains(&spec.name) {
            names.push(spec.name);
        }
    }
    names
}

// ---------------------------------------------------------------------
// 木の読み出し
// ---------------------------------------------------------------------

fn child<'a>(element: &'a Element, name: &str, id: Option<&str>) -> Option<&'a Element> {
    match element.children.get(element.find_child(name, id)?)? {
        Node::Element(child) => Some(child),
        Node::Raw(_) => None,
    }
}

fn element_children<'a>(element: &'a Element, name: &'a str) -> impl Iterator<Item = &'a Element> {
    element.children.iter().filter_map(move |node| match node {
        Node::Element(child) if child.name == name => Some(child),
        _ => None,
    })
}

/// スコープノードの下の `suffix` の葉のテキスト(生のまま)。無ければ `None`。
fn leaf_text(node: &Element, suffix: &[&str]) -> Option<String> {
    let mut current = node;
    for name in suffix {
        current = child(current, name, None)?;
    }
    Some(
        current
            .children
            .iter()
            .map(|node| match node {
                Node::Raw(raw) => raw.as_str(),
                // 葉の下に要素があるのは想定外。テキストだけ拾い、型検査で unsupported になる。
                Node::Element(_) => "",
            })
            .collect(),
    )
}

/// `Instance[*]` からスコープの深さまで `*` をたどった先の葉。
fn star_chain_text(cobo: &Element, spec: &KnobSpec) -> Option<String> {
    let mut node = child(cobo, INSTANCE, Some(STAR))?;
    for depth in 1..=spec.scope.depth() {
        node = child(node, Scope::element_at_depth(depth)?, Some(STAR))?;
    }
    leaf_text(node, spec.suffix)
}

/// `*` の連鎖以外のスコープノードで見つかった値 1 件。
struct Hit {
    /// 例外として表せるなら `at`、表せなければ理由。
    at: Result<At, String>,
    text: String,
}

fn explicit_hits(cobo: &Element, spec: &KnobSpec) -> Vec<Hit> {
    let mut hits = Vec::new();
    for instance in element_children(cobo, INSTANCE) {
        let instance_is_star = instance.id.as_deref() == Some(STAR);
        walk(instance, spec, instance_is_star, &mut Vec::new(), &mut hits);
    }
    hits
}

/// スコープの深さまで全ノードを下り、`*` の連鎖以外で葉を持つものを集める。
fn walk<'a>(
    node: &'a Element,
    spec: &KnobSpec,
    instance_is_star: bool,
    ids: &mut Vec<&'a str>,
    hits: &mut Vec<Hit>,
) {
    if ids.len() == spec.scope.depth() {
        if instance_is_star && ids.iter().all(|id| *id == STAR) {
            return; // 一様値(extract_uniform が読む)
        }
        if let Some(text) = leaf_text(node, spec.suffix) {
            hits.push(Hit {
                at: classify(instance_is_star, ids),
                text,
            });
        }
        return;
    }
    let Some(name) = Scope::element_at_depth(ids.len() + 1) else {
        return;
    };
    for child in element_children(node, name) {
        ids.push(child.id.as_deref().unwrap_or(""));
        walk(child, spec, instance_is_star, ids, hits);
        ids.pop();
    }
}

/// 明示 id の並び → `at`。明示 id は先頭から連続していなければならない(111 §5.0)。
fn classify(instance_is_star: bool, ids: &[&str]) -> Result<At, String> {
    if !instance_is_star {
        return Err(WHY_EXPLICIT_INSTANCE.to_string());
    }
    let explicit = ids.iter().take_while(|id| **id != STAR).count();
    if ids[explicit..].iter().any(|id| *id != STAR) {
        return Err(WHY_BELOW_WILDCARD.to_string());
    }
    let mut numbers = Vec::new();
    for id in &ids[..explicit] {
        numbers.push(
            id.parse::<u32>()
                .map_err(|_| format!("non-numeric id {id:?}"))?,
        );
    }
    Ok(At {
        asad: numbers.first().copied(),
        aget: numbers.get(1).copied(),
        ch: numbers.get(2).copied(),
    })
}

// ---------------------------------------------------------------------
// テキスト → JSON(`value_to_xcfg_text` の逆)
// ---------------------------------------------------------------------

/// 1 ノブ分のテキスト(合成ノブは構成要素すべて)を JSON の値にする。`Err` は unsupported の理由。
fn decode(entries: &[(&KnobSpec, &str)]) -> Result<Value, String> {
    match entries {
        [(spec, text)] if !matches!(spec.kind, KnobKind::Derived(_)) => text_to_value(spec, text),
        _ => derived_mode(entries),
    }
}

fn text_to_value(spec: &KnobSpec, raw: &str) -> Result<Value, String> {
    let unescaped = quick_xml::escape::unescape(raw).map_err(|e| e.to_string())?;
    let text = unescaped.trim();
    let value = match spec.kind {
        KnobKind::Int { .. } | KnobKind::IntEnum(_) => text
            .parse::<i64>()
            .map(Value::from)
            .map_err(|_| "not an integer".to_string())?,
        KnobKind::Real { .. } | KnobKind::RealEnum(_) => text
            .parse::<f64>()
            .ok()
            .and_then(Number::from_f64)
            .map(Value::Number)
            .ok_or_else(|| "not a number".to_string())?,
        KnobKind::StrEnum(_) => Value::from(text),
        KnobKind::Bool => match text {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => return Err("not a bool (true/false)".to_string()),
        },
        KnobKind::Derived(_) => return derived_mode(&[(spec, raw)]),
    };
    // 範囲・許容値は schema の検査そのもの(Params::from_json_str と同じ基準)。
    check_value(spec, &value).map_err(|e| e.to_string())?;
    Ok(value)
}

/// 合成ノブ: 全構成要素のテキストが**同じモード**に写るときだけそのモード。
fn derived_mode(entries: &[(&KnobSpec, &str)]) -> Result<Value, String> {
    let mut mode: Option<&str> = None;
    for (spec, raw) in entries {
        let KnobKind::Derived(table) = spec.kind else {
            return Err(format!("{} is not a composite knob", spec.name));
        };
        let text = raw.trim();
        let modes = || {
            table
                .iter()
                .map(|(json, _)| *json)
                .collect::<Vec<_>>()
                .join(", ")
        };
        let this = table
            .iter()
            .find(|(_, xcfg)| *xcfg == text)
            .map(|(json, _)| *json)
            .ok_or_else(|| format!("parts do not map to one of {{{}}}", modes()))?;
        if mode.is_some_and(|previous| previous != this) {
            return Err(format!("parts do not map to one of {{{}}}", modes()));
        }
        mode = Some(this);
    }
    mode.map(Value::from)
        .ok_or_else(|| "no parts of the composite knob".to_string())
}

/// `found` の文字列。構成要素が複数のノブは `要素名=テキスト` を `,` でつなぐ(1 つならテキストそのまま)。
fn found_text(entries: &[(&KnobSpec, &str)]) -> String {
    // KNOBS に同じ名前のエントリが複数ある(= 構成要素が複数の)ノブだけ要素名を付ける。
    let composite = entries
        .first()
        .is_some_and(|(spec, _)| knob_specs(spec.name).count() > 1);
    entries
        .iter()
        .map(|(spec, text)| {
            if composite {
                format!("{}={text}", spec.suffix.last().copied().unwrap_or_default())
            } else {
                (*text).to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// `.` 区切りの名前で入れ子の `knobs` に入れる(`pulser.enabled` → `{"pulser":{"enabled":…}}`)。
fn insert_knob(knobs: &mut BTreeMap<String, Value>, name: &str, value: Value) {
    match name.split_once('.') {
        None => {
            knobs.insert(name.to_string(), value);
        }
        Some((outer, inner)) => {
            let entry = knobs
                .entry(outer.to_string())
                .or_insert_with(|| Value::Object(Map::new()));
            if let Value::Object(map) = entry {
                map.insert(inner.to_string(), value);
            }
        }
    }
}

fn at_json(at: &At) -> Value {
    let mut map = Map::new();
    for (key, value) in [("asad", at.asad), ("aget", at.aget), ("ch", at.ch)] {
        if let Some(value) = value {
            map.insert(key.to_string(), Value::from(value));
        }
    }
    Value::Object(map)
}
