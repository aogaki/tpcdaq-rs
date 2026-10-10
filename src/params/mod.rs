//! パラメータ設定(**JSON が正 → xcfg は生成物**)。TODO/078 レーン A。
//!
//! ECC は改変せず、実験側の正典 xcfg を**テンプレート**として丸ごと引き継ぐ。
//! 管理ノブ以外は一切触らない(`render` は非管理ノブの生バイトをそのまま再生する)。
//!
//! - [`schema`]: JSON スキーマ `tpcdaq-params/1` と検証(ノブ表 = [`schema::KNOBS`])。
//! - [`render`]: テンプレート xcfg への適用(除去規則 → 一様値 → 例外)。
//! - [`extract`]: その逆 — テンプレート xcfg からノブの値を読む(TODO/111、UI の初期値)。
//! - [`paths`]: xcfg 上の位置の内部表現(CompoundConfig の `Path` 構文)。
//!
//! 実装済みのノブ(12 ノブ・葉 15)と、078 §2 の表と実 xcfg の照合結果は
//! `docs/PARAMS_KNOBS_ja.md`(§7 が一覧)を参照。

pub mod extract;
pub mod paths;
pub mod render;
pub mod schema;

pub use extract::{extract, Extracted, Unsupported};
pub use paths::{knob_path, Scope};
pub use render::{render, Rendered};
pub use schema::{
    knob_spec, knob_specs, At, Exception, KnobKind, KnobSpec, Params, ParamsError, KNOBS,
    PEAKING_TIME_NS, SCHEMA_ID,
};
