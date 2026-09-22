//! xcfg 上の位置を指す内部表現(078 §3-5「パスは XPath 風の内部表現 1 種類に固定」)。
//!
//! 採用したのは **CompoundConfig 自身の `Path` 構文**(`CCfg::Path`、`.` 区切り + `[index]`):
//!
//! ```text
//! Node[CoBo].Instance[0].AsAd[1].Aget[2].channel[5].LSBThresholdValue
//! ```
//!
//! こうするとオラクル(`tools/ccfg_resolve`)にそのまま渡せる = 照合のときに
//! 表現を変換しなくて済む。ルート要素 `Setup[Conditions]` はパスに含めない
//! (CConfig の base attribute がルートなので)。

use std::fmt;

/// 要素名の定数(文字列リテラルを散らかさない)。
pub const NODE: &str = "Node";
pub const COBO: &str = "CoBo";
pub const INSTANCE: &str = "Instance";
pub const ASAD: &str = "AsAd";
pub const AGET: &str = "Aget";
pub const CHANNEL: &str = "channel";
/// 既定値ノードの id(xcfg の継承規則 — 072 §2)。
pub const STAR: &str = "*";

/// ノブが住む階層。UI には見せない(078 §2 のノブ定義表「スコープ」列)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
    Cobo,
    Asad,
    Aget,
    Channel,
}

impl Scope {
    /// CoBo を 0 とした深さ。`exceptions[].at` の粗さ判定に使う。
    pub fn depth(self) -> usize {
        match self {
            Scope::Cobo => 0,
            Scope::Asad => 1,
            Scope::Aget => 2,
            Scope::Channel => 3,
        }
    }

    /// 深さ 1..=3 に対応する要素名(`at` の階層 → xcfg の要素)。
    pub fn element_at_depth(depth: usize) -> Option<&'static str> {
        match depth {
            1 => Some(ASAD),
            2 => Some(AGET),
            3 => Some(CHANNEL),
            _ => None,
        }
    }
}

/// パスの 1 段。`index` は xcfg の `id` 属性(`*` を含む)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub name: String,
    pub index: Option<String>,
}

/// `Node[CoBo].Instance[*]....` の並び。
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Path {
    pub steps: Vec<Step>,
}

impl Path {
    pub fn new() -> Self {
        Path::default()
    }

    pub fn push(&mut self, name: &str, index: Option<&str>) {
        self.steps.push(Step {
            name: name.to_string(),
            index: index.map(|s| s.to_string()),
        });
    }
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, step) in self.steps.iter().enumerate() {
            if i > 0 {
                write!(f, ".")?;
            }
            write!(f, "{}", step.name)?;
            if let Some(index) = &step.index {
                write!(f, "[{index}]")?;
            }
        }
        Ok(())
    }
}

/// 1 つのノブインスタンスの完全パスを組み立てる。
///
/// - `instance`: CoBo インスタンスの id(レンダラは常に `*`、照合テストは `0`/`1`…)。
/// - `ids`: スコープの深さ分の id(`[asad]` / `[asad, aget]` / `[asad, aget, ch]`)。
///   余分に渡された分は無視せず**切り詰める**(スコープより深い位置は存在しないため)。
/// - `suffix`: スコープノードの下の要素名の並び(例 `["Module", "triggerDelay"]`)。
pub fn knob_path(instance: &str, scope: Scope, ids: &[&str], suffix: &[&str]) -> Path {
    let mut path = Path::new();
    path.push(NODE, Some(COBO));
    path.push(INSTANCE, Some(instance));
    for depth in 1..=scope.depth() {
        let id = ids.get(depth - 1).copied().unwrap_or(STAR);
        let name = Scope::element_at_depth(depth).unwrap_or(CHANNEL);
        path.push(name, Some(id));
    }
    for name in suffix {
        path.push(name, None);
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_channel_scope_path_prints_like_the_compound_config_path_syntax() {
        let path = knob_path(
            "0",
            Scope::Channel,
            &["1", "2", "5"],
            &["LSBThresholdValue"],
        );
        assert_eq!(
            path.to_string(),
            "Node[CoBo].Instance[0].AsAd[1].Aget[2].channel[5].LSBThresholdValue"
        );
    }

    #[test]
    fn a_cobo_scope_path_has_no_asad_level() {
        let path = knob_path("*", Scope::Cobo, &[], &["Module", "triggerDelay"]);
        assert_eq!(
            path.to_string(),
            "Node[CoBo].Instance[*].Module.triggerDelay"
        );
    }

    #[test]
    fn missing_ids_fall_back_to_the_default_node() {
        // 一様値の書き込み先(`*` の連鎖)を作るときに使う形。
        // 粗い例外は `*` では効かないので展開する(render.rs の (3) を参照)。
        let path = knob_path("*", Scope::Channel, &["1"], &["Gain"]);
        assert_eq!(
            path.to_string(),
            "Node[CoBo].Instance[*].AsAd[1].Aget[*].channel[*].Gain"
        );
    }

    #[test]
    fn an_aget_scope_path_stops_at_the_aget_level() {
        let path = knob_path(
            "0",
            Scope::Aget,
            &["3", "2"],
            &["Global", "Reg1", "GlobalThresholdValue"],
        );
        assert_eq!(
            path.to_string(),
            "Node[CoBo].Instance[0].AsAd[3].Aget[2].Global.Reg1.GlobalThresholdValue"
        );
    }
}
