//! テンプレート xcfg + パラメータ JSON → 生成 xcfg(078 §3)。
//!
//! # 規則(078 §3)
//!
//! 1. **管理ノブ**: テンプレートから当該ノブの**全インスタンスノード**(`*` と明示 id の両方)を
//!    除去 → 一様値を `*` ノードに書く → 例外を明示 id ノードで書く。
//!    **除去を省くと、テンプレートに残った明示ノードが JSON の一様値に勝つ**(最重要の罠)。
//!    実装では「`*` の連鎖上のノード」だけ除去せず、その場で値を書き換える
//!    (= 位置と `unit`/`range` 属性が保たれ、往復がバイト一致になる)。
//! 2. **非管理ノブ**: 1 バイトも変えない。パーサは各イベントの**生スライス**を持ち回り、
//!    書き出しでそのまま再生する(コメント・改行・タブ・属性の引用符まで保つ)。
//! 3. **出力**: `configure-<id>.xcfg` と `prepare-<id>.xcfg`(同内容)。
//!    `<id>` = `<テンプレート stem>__<JSON の sha256 先頭 8 桁>`。
//!
//! # 除去も書き込みも「パス」で行う
//!
//! `triggerDelay` は `Module`(10ns 単位)と `AsAd/Generator`(WCK periods)の**両方にある**。
//! 要素名で消すと別物を壊す。スコープ + suffix の完全パスだけを見る。

use std::collections::BTreeSet;

use quick_xml::events::Event;
use quick_xml::Reader;

use super::paths::{Scope, AGET, ASAD, CHANNEL, COBO, INSTANCE, NODE, STAR};
use super::schema::{
    knob_spec, knob_specs, lookup_knob, value_to_xcfg_text, At, KnobSpec, Params, ParamsError,
    KNOBS,
};

/// 生成結果。configure と prepare は**同内容**(078 §3-3)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    /// ECC に渡す config id(= `<テンプレート stem>__<hash8>`)。
    pub config_id: String,
    pub configure_file_name: String,
    pub prepare_file_name: String,
    /// 生成した xcfg の中身。
    pub xml: String,
}

/// パラメータ JSON をテンプレートに適用する。**純関数**(ファイル IO はしない)。
pub fn render(params: &Params, template_xml: &str) -> Result<Rendered, ParamsError> {
    let mut doc = parse(template_xml)?;

    {
        let cobo = find_cobo_node(&mut doc)?;
        // テンプレートに実在する id(ch 数をコードに焼き込まない — CLAUDE.md)。
        // **書き換える前**に集める。
        let ids = TemplateIds::collect(cobo);
        check_exception_targets(&ids, params)?;

        for spec in KNOBS {
            let value =
                lookup_knob(&params.knobs, spec.name).ok_or_else(|| ParamsError::MissingKnob {
                    knob: spec.name.to_string(),
                })?;
            let text = value_to_xcfg_text(spec, value)?;

            // (1) 除去: `*` の連鎖上以外の全インスタンスノードから当該ノブを消す。
            remove_knob_from_explicit_nodes(cobo, spec);
            // (2) 一様値: `*` の連鎖に書く。
            write_knob(cobo, spec, &[], &text)?;
        }

        // (3) 例外: **スコープの深さまで明示 id に展開して**書く。
        //
        // 078 §2 は「`{"asad":1}` → `<AsAd id="1"><Aget id="*"><channel id="*">`」と
        // 書いているが、**それでは効かない**(2026-09-22、ccfg_resolve で実測)。
        // CompoundConfig の探索(View/Search.hpp `search`/`backtrack`)は、ある段で
        // 明示 id が見つからないとき、その場で `*` に落ちるのは「より深い段で既に
        // 実体を見つけている」ときだけ(`if(iInstance >= iPath and index != "*")`)。
        // `AsAd[1]` の下に `Aget[*]` を置いても、`Aget[0]` が無ければ探索は
        // `AsAd[1]` ごと諦めて `AsAd[*]` の連鎖へ戻ってしまう(= 一様値が勝つ)。
        // テンプレートが `<channel id="0" />` のような空の明示ノードを全部並べているのは
        // この探索を成り立たせるため。
        // よって、スコープより粗い `at` はテンプレートに実在する id の直積に展開する。
        for exception in &params.exceptions {
            if knob_spec(&exception.knob).is_none() {
                return Err(ParamsError::UnknownKnob {
                    knob: exception.knob.clone(),
                });
            }
            // 合成ノブ(`readout`)は 1 つの例外が複数の要素に効く。
            for spec in knob_specs(&exception.knob) {
                let text = value_to_xcfg_text(spec, &exception.value)?;
                for target in ids.expand(&exception.at, spec, &exception.knob)? {
                    let target: Vec<&str> = target.iter().map(String::as_str).collect();
                    write_knob(cobo, spec, &target, &text)?;
                }
            }
        }
    }

    let stem = template_stem(&params.template);
    let config_id = format!("{stem}__{}", params.json_hash8());
    Ok(Rendered {
        configure_file_name: format!("configure-{config_id}.xcfg"),
        prepare_file_name: format!("prepare-{config_id}.xcfg"),
        config_id,
        xml: serialize(&doc),
    })
}

/// テンプレートのファイル名 → `<id>` に使う stem。
///
/// 相のプレフィクス(`configure-` / `prepare-`)は落とす。ECC は
/// `<相>-<configId>.xcfg` という名前でファイルを読むので、残すと
/// `configure-configure-…` になってしまう。
fn template_stem(template: &str) -> String {
    let name = template.rsplit('/').next().unwrap_or(template);
    let stem = name.strip_suffix(".xcfg").unwrap_or(name);
    for prefix in ["configure-", "prepare-", "describe-"] {
        if let Some(rest) = stem.strip_prefix(prefix) {
            return rest.to_string();
        }
    }
    stem.to_string()
}

// ---------------------------------------------------------------------
// 最小の XML 木(生スライスを持つ)
// ---------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Node {
    Element(Element),
    /// テキスト・コメント・宣言など、**そのまま出すもの**(生スライス)。
    Raw(String),
}

#[derive(Debug, Clone)]
struct Element {
    name: String,
    id: Option<String>,
    /// `<Module>` や `<channel id="0" />` の生スライス。
    open_raw: String,
    /// `</Module>`(空要素は空文字)。
    close_raw: String,
    empty: bool,
    children: Vec<Node>,
}

impl Element {
    fn new_virtual_root() -> Element {
        Element {
            name: String::new(),
            id: None,
            open_raw: String::new(),
            close_raw: String::new(),
            empty: false,
            children: Vec::new(),
        }
    }

    /// 指定名(と id)の子のインデックス。
    fn find_child(&self, name: &str, id: Option<&str>) -> Option<usize> {
        self.children.iter().position(|child| match child {
            Node::Element(element) => {
                element.name == name && (id.is_none() || element.id.as_deref() == id)
            }
            Node::Raw(_) => false,
        })
    }

    fn child_indices(&self, name: &str) -> Vec<usize> {
        self.children
            .iter()
            .enumerate()
            .filter_map(|(i, child)| match child {
                Node::Element(element) if element.name == name => Some(i),
                _ => None,
            })
            .collect()
    }

    fn element_at(&mut self, index: usize) -> Option<&mut Element> {
        match self.children.get_mut(index) {
            Some(Node::Element(element)) => Some(element),
            _ => None,
        }
    }

    /// 子要素を 1 個消す(見つからなければ何もしない)。直前の空白テキストも一緒に消して
    /// 空行が残らないようにする。
    fn remove_child(&mut self, index: usize) {
        self.children.remove(index);
        if index > 0 {
            if let Some(Node::Raw(raw)) = self.children.get(index - 1) {
                if raw.trim().is_empty() && raw.contains('\n') {
                    self.children.remove(index - 1);
                }
            }
        }
    }

    /// 値(テキスト)を差し替える。空要素だったら開始/終了タグ形式に直す。
    fn set_text(&mut self, text: &str) {
        if self.empty {
            let inner = self
                .open_raw
                .trim_start_matches('<')
                .trim_end_matches("/>")
                .trim_end();
            self.open_raw = format!("<{inner}>");
            self.close_raw = format!("</{}>", self.name);
            self.empty = false;
        }
        self.children = vec![Node::Raw(text.to_string())];
    }

    /// 子要素を末尾(閉じタグ直前の空白の前)に足す。
    fn append_child(&mut self, element: Element, depth: usize) {
        let indent = format!("\n{}", "\t".repeat(depth));
        if self.empty {
            self.open_raw = format!(
                "<{}>",
                self.open_raw
                    .trim_start_matches('<')
                    .trim_end_matches("/>")
                    .trim_end()
            );
            self.close_raw = format!("</{}>", self.name);
            self.empty = false;
            self.children.clear();
        }
        // 末尾が閉じタグ用の空白ならその前に入れる。
        let closing_ws = matches!(self.children.last(), Some(Node::Raw(raw)) if raw.trim().is_empty() && raw.contains('\n'));
        let at = if closing_ws {
            self.children.len() - 1
        } else {
            self.children.len()
        };
        self.children.insert(at, Node::Element(element));
        self.children.insert(at, Node::Raw(indent));
        if !closing_ws {
            // 閉じタグ用の空白が無かった(= 1 行で書かれていた)ので足す。
            let closing = format!("\n{}", "\t".repeat(depth.saturating_sub(1)));
            self.children.push(Node::Raw(closing));
        }
    }
}

fn new_element(name: &str, id: Option<&str>) -> Element {
    let open_raw = match id {
        Some(id) => format!("<{name} id=\"{id}\">"),
        None => format!("<{name}>"),
    };
    Element {
        name: name.to_string(),
        id: id.map(|s| s.to_string()),
        open_raw,
        close_raw: format!("</{name}>"),
        empty: false,
        children: Vec::new(),
    }
}

// ---------------------------------------------------------------------
// パース / 書き出し
// ---------------------------------------------------------------------

fn parse(xml: &str) -> Result<Element, ParamsError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().expand_empty_elements = false;

    let mut stack: Vec<Element> = vec![Element::new_virtual_root()];
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader
            .read_event()
            .map_err(|e| ParamsError::Xml(e.to_string()))?;
        let end = reader.buffer_position() as usize;
        let raw = xml.get(start..end).unwrap_or_default();

        match event {
            Event::Eof => break,
            Event::Start(e) => {
                let name = e.name().as_ref().to_string();
                stack.push(Element {
                    id: attribute_id(&e),
                    name,
                    open_raw: raw.to_string(),
                    close_raw: String::new(),
                    empty: false,
                    children: Vec::new(),
                });
            }
            Event::Empty(e) => {
                let name = e.name().as_ref().to_string();
                let element = Element {
                    id: attribute_id(&e),
                    name,
                    open_raw: raw.to_string(),
                    close_raw: String::new(),
                    empty: true,
                    children: Vec::new(),
                };
                push_child(&mut stack, Node::Element(element))?;
            }
            Event::End(_) => {
                let mut element = stack
                    .pop()
                    .ok_or_else(|| ParamsError::Xml("unbalanced end tag".to_string()))?;
                element.close_raw = raw.to_string();
                push_child(&mut stack, Node::Element(element))?;
            }
            _ => push_child(&mut stack, Node::Raw(raw.to_string()))?,
        }
    }

    if stack.len() != 1 {
        return Err(ParamsError::Xml("unbalanced start tag".to_string()));
    }
    stack
        .pop()
        .ok_or_else(|| ParamsError::Xml("empty document".to_string()))
}

fn push_child(stack: &mut [Element], node: Node) -> Result<(), ParamsError> {
    let parent = stack
        .last_mut()
        .ok_or_else(|| ParamsError::Xml("no open element".to_string()))?;
    parent.children.push(node);
    Ok(())
}

fn attribute_id(start: &quick_xml::events::BytesStart<'_>) -> Option<String> {
    start
        .attributes()
        .flatten()
        .find(|attr| attr.key.as_ref() == "id")
        .map(|attr| attr.value.to_string())
}

fn serialize(root: &Element) -> String {
    let mut out = String::new();
    write_children(root, &mut out);
    out
}

fn write_children(element: &Element, out: &mut String) {
    for child in &element.children {
        match child {
            Node::Raw(raw) => out.push_str(raw),
            Node::Element(element) => {
                out.push_str(&element.open_raw);
                if !element.empty {
                    write_children(element, out);
                    out.push_str(&element.close_raw);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------
// 木の操作(除去と書き込み)
// ---------------------------------------------------------------------

/// `Setup` の下の `Node id="CoBo"` を取る。
fn find_cobo_node(doc: &mut Element) -> Result<&mut Element, ParamsError> {
    let setup =
        doc.child_indices("Setup")
            .first()
            .copied()
            .ok_or_else(|| ParamsError::TemplateShape {
                what: "ルート要素 Setup が無い".to_string(),
            })?;
    let setup = doc
        .element_at(setup)
        .ok_or_else(|| ParamsError::TemplateShape {
            what: "ルート要素 Setup が無い".to_string(),
        })?;
    let cobo = setup
        .find_child(NODE, Some(COBO))
        .ok_or_else(|| ParamsError::TemplateShape {
            what: format!("{NODE}[{COBO}] が無い"),
        })?;
    setup
        .element_at(cobo)
        .ok_or_else(|| ParamsError::TemplateShape {
            what: format!("{NODE}[{COBO}] が無い"),
        })
}

/// スコープノードを走査する。`all_star` は「ここまでの id が全て `*`」。
fn for_each_scope_node(
    node: &mut Element,
    current_depth: usize,
    scope: Scope,
    all_star: bool,
    f: &mut dyn FnMut(&mut Element, bool),
) {
    if current_depth == scope.depth() {
        f(node, all_star);
        return;
    }
    let Some(child_name) = Scope::element_at_depth(current_depth + 1) else {
        return;
    };
    for index in node.child_indices(child_name) {
        let Some(child) = node.element_at(index) else {
            continue;
        };
        let star = all_star && child.id.as_deref() == Some(STAR);
        for_each_scope_node(child, current_depth + 1, scope, star, f);
    }
}

/// 078 §3-1 の除去。`*` の連鎖上のノードだけは残す(そこに一様値を書くため)。
fn remove_knob_from_explicit_nodes(cobo: &mut Element, spec: &KnobSpec) {
    for index in cobo.child_indices(INSTANCE) {
        let Some(instance) = cobo.element_at(index) else {
            continue;
        };
        let instance_is_star = instance.id.as_deref() == Some(STAR);
        for_each_scope_node(
            instance,
            0,
            spec.scope,
            instance_is_star,
            &mut |node, all_star| {
                if !all_star {
                    remove_suffix(node, spec.suffix);
                }
            },
        );
    }
}

/// スコープノードの下の `suffix` の葉を消す(中間ノードは残す = 非管理ノブに触らない)。
fn remove_suffix(node: &mut Element, suffix: &[&str]) {
    let Some((leaf, parents)) = suffix.split_last() else {
        return;
    };
    let mut current = node;
    for name in parents {
        let Some(index) = current.find_child(name, None) else {
            return;
        };
        let Some(next) = current.element_at(index) else {
            return;
        };
        current = next;
    }
    if let Some(index) = current.find_child(leaf, None) {
        current.remove_child(index);
    }
}

/// 一様値 / 例外を書く。`ids` が空なら `*` の連鎖、そうでなければ明示 id +
/// 足りない段を `*` で埋める。
fn write_knob(
    cobo: &mut Element,
    spec: &KnobSpec,
    ids: &[&str],
    text: &str,
) -> Result<(), ParamsError> {
    // Setup(0) > Node(1) > Instance(2) > AsAd(3) > Aget(4) > channel(5)
    let mut depth = 2;
    let instance_index =
        cobo.find_child(INSTANCE, Some(STAR))
            .ok_or_else(|| ParamsError::TemplateShape {
                what: format!("{INSTANCE}[{STAR}] が無い"),
            })?;
    let mut current =
        cobo.element_at(instance_index)
            .ok_or_else(|| ParamsError::TemplateShape {
                what: format!("{INSTANCE}[{STAR}] が無い"),
            })?;

    for level in 1..=spec.scope.depth() {
        let name = Scope::element_at_depth(level).unwrap_or(CHANNEL);
        let id = ids.get(level - 1).copied().unwrap_or(STAR);
        depth += 1;
        let index = match current.find_child(name, Some(id)) {
            Some(index) => index,
            None => {
                current.append_child(new_element(name, Some(id)), depth);
                current
                    .find_child(name, Some(id))
                    .ok_or_else(|| ParamsError::TemplateShape {
                        what: format!("{name}[{id}] を作れなかった"),
                    })?
            }
        };
        current = current
            .element_at(index)
            .ok_or_else(|| ParamsError::TemplateShape {
                what: format!("{name}[{id}] が要素でない"),
            })?;
    }

    set_suffix_value(current, depth, spec.suffix, text)
}

fn set_suffix_value(
    node: &mut Element,
    depth: usize,
    suffix: &[&str],
    text: &str,
) -> Result<(), ParamsError> {
    let Some((leaf, parents)) = suffix.split_last() else {
        return Ok(());
    };
    let mut current = node;
    let mut depth = depth;
    for name in parents {
        depth += 1;
        let index = match current.find_child(name, None) {
            Some(index) => index,
            None => {
                current.append_child(new_element(name, None), depth);
                current
                    .find_child(name, None)
                    .ok_or_else(|| ParamsError::TemplateShape {
                        what: format!("{name} を作れなかった"),
                    })?
            }
        };
        current = current
            .element_at(index)
            .ok_or_else(|| ParamsError::TemplateShape {
                what: format!("{name} が要素でない"),
            })?;
    }
    depth += 1;
    let escaped = quick_xml::escape::escape(text).to_string();
    match current.find_child(leaf, None) {
        Some(index) => {
            if let Some(element) = current.element_at(index) {
                element.set_text(&escaped);
            }
        }
        None => {
            let mut element = new_element(leaf, None);
            element.children.push(Node::Raw(escaped));
            current.append_child(element, depth);
        }
    }
    Ok(())
}

/// テンプレートに実在する明示 id(`*` は含まない)。
///
/// ch 数や AsAd 数を**コードに焼き込まない**(CLAUDE.md)ため、id の集合は
/// テンプレート自身から集める。存在しない id への例外は黙って無視せずエラーにする。
struct TemplateIds {
    /// 深さ 1..=3(AsAd / Aget / channel)の id。数値順。
    by_depth: [Vec<String>; 3],
}

impl TemplateIds {
    fn collect(cobo: &Element) -> TemplateIds {
        let mut by_depth = [Vec::new(), Vec::new(), Vec::new()];
        for (index, name) in [ASAD, AGET, CHANNEL].iter().enumerate() {
            let mut ids = BTreeSet::new();
            collect_ids(cobo, name, &mut ids);
            let mut ids: Vec<String> = ids.into_iter().collect();
            ids.sort_by_key(|id| id.parse::<i64>().unwrap_or(i64::MAX));
            by_depth[index] = ids;
        }
        TemplateIds { by_depth }
    }

    fn at_depth(&self, depth: usize) -> &[String] {
        self.by_depth
            .get(depth - 1)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// `at` を**ノブのスコープの深さまで**展開する(足りない段はテンプレートの全 id)。
    fn expand(
        &self,
        at: &At,
        spec: &KnobSpec,
        knob: &str,
    ) -> Result<Vec<Vec<String>>, ParamsError> {
        let mut targets: Vec<Vec<String>> = vec![at.ids()];
        for depth in (at.depth() + 1)..=spec.scope.depth() {
            let ids = self.at_depth(depth);
            if ids.is_empty() {
                return Err(ParamsError::CannotExpandException {
                    knob: knob.to_string(),
                    at: at.to_string(),
                    level: Scope::element_at_depth(depth)
                        .unwrap_or(CHANNEL)
                        .to_string(),
                });
            }
            targets = targets
                .into_iter()
                .flat_map(|prefix| {
                    ids.iter().map(move |id| {
                        let mut next = prefix.clone();
                        next.push(id.clone());
                        next
                    })
                })
                .collect();
        }
        Ok(targets)
    }
}

/// 例外の宛先がテンプレートに実在する id かを確かめる。
fn check_exception_targets(ids: &TemplateIds, params: &Params) -> Result<(), ParamsError> {
    for exception in &params.exceptions {
        for (depth, value) in [
            (1, exception.at.asad),
            (2, exception.at.aget),
            (3, exception.at.ch),
        ] {
            let Some(value) = value else { continue };
            if !ids
                .at_depth(depth)
                .iter()
                .any(|id| id == &value.to_string())
            {
                return Err(ParamsError::TargetNotInTemplate {
                    level: Scope::element_at_depth(depth)
                        .unwrap_or(CHANNEL)
                        .to_string(),
                    id: value.to_string(),
                });
            }
        }
    }
    Ok(())
}

fn collect_ids(element: &Element, name: &str, out: &mut BTreeSet<String>) {
    for child in &element.children {
        if let Node::Element(child) = child {
            if child.name == name {
                if let Some(id) = &child.id {
                    if id != STAR {
                        out.insert(id.clone());
                    }
                }
            }
            collect_ids(child, name, out);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// パーサ + 書き出しは**生バイトの往復**(非管理ノブを 1 バイトも変えないための土台)。
    #[test]
    fn parsing_and_serializing_round_trips_byte_for_byte() {
        let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Setup id=\"Conditions\">\n\t<!-- comment -->\n\t<Node id=\"CoBo\">\n\t\t<Instance id=\"*\">\n\t\t\t<Module>\n\t\t\t\t<triggerDelay unit=\"10ns\">1748</triggerDelay>\n\t\t\t</Module>\n\t\t</Instance>\n\t\t<Instance id=\"0\" />\n\t</Node>\n</Setup>\n";

        let doc = parse(xml).unwrap();

        assert_eq!(serialize(&doc), xml);
    }

    #[test]
    fn the_template_stem_drops_the_phase_prefix_and_the_extension() {
        assert_eq!(
            template_stem("configure-physics-zCobo1k_25MHz.xcfg"),
            "physics-zCobo1k_25MHz"
        );
        assert_eq!(template_stem("prepare-mini.xcfg"), "mini");
        assert_eq!(template_stem("mini.xcfg"), "mini");
    }
}
