//! スローコントロール(ガス系 Bronkhorst FLOW-BUS / 環境 Arduino、SPEC §1.1、旧 psu)。
//!
//! **Phase 1(このユニット、TODO/079 §2.5)の範囲 = ProPar ASCII コーデックと同期リンクのみ。**
//! シリアルポート配線・tokio・TOML 設定・component 本体・logbook 出力は Phase 2 以降。
//! 安全境界(書き込みコマンドを実装しない = 監視のみ)は TODO/079 §1.5 / SPEC §11 に従う。

/// ProPar ASCII コーデック(純関数、IO なし)。Bronkhorst 9.17.027 §3.3–§3.9。
pub mod propar;

/// 同期 ProPar リンク(`Read + Write` ジェネリック)。9.17.027 §3.2/§3.7。
pub mod link;

/// インメモリ ProPar 応答器(テスト専用、または `slowctl-fake` feature)。
#[cfg(any(test, feature = "slowctl-fake"))]
pub mod fake;
