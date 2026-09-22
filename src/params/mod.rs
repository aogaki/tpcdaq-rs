//! パラメータ設定(**JSON が正 → xcfg は生成物**)。TODO/078 レーン A。
//!
//! ECC は改変せず、実験側の正典 xcfg を**テンプレート**として丸ごと引き継ぐ。
//! 管理ノブ以外は一切触らない(`render` は非管理ノブの生バイトをそのまま再生する)。
//!
//! - [`schema`]: JSON スキーマ `tpcdaq-params/1` と検証(ノブ表 = [`schema::KNOBS`])。
//! - [`render`]: テンプレート xcfg への適用(除去規則 → 一様値 → 例外)。
//! - [`paths`]: xcfg 上の位置の内部表現(CompoundConfig の `Path` 構文)。
//!
//! 実装したノブと、078 §2 の表と実 xcfg が食い違って**未実装**のノブは
//! `docs/PARAMS_KNOBS_ja.md` を参照。

pub mod paths;
pub mod render;
pub mod schema;

pub use paths::{knob_path, Scope};
pub use render::{render, Rendered};
pub use schema::{
    knob_spec, At, Exception, KnobKind, KnobSpec, Params, ParamsError, KNOBS, SCHEMA_ID,
};
