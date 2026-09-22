//! ECC(GET の run 制御 Ice サーバ)との対話。TODO/076 Phase 2、SPEC §8.2。
//!
//! - [`core`] — Ice も ZMQ も知らない核(状態・error フラグ・DataLinkSet XML・JSON 契約・
//!   fake-ECC の遷移表)。`tools/ecc_bridge/ecc_core.hpp` の 1:1 移植。
//! - [`proxy`] — `get::rc::StateMachine` の型付きプロキシ([`crate::ice`] の上)。
//!
//! ZMQ REP のループ本体は `src/bin/ecc_bridge.rs`(bin)側にある。

/// Ice 非依存の核(状態・error フラグ・DataLinkSet XML・JSON 契約・遷移表)。
pub mod core;

/// `get::rc::StateMachine` の型付きプロキシ(describe / prepare / … / getStatus / ice_ping)。
pub mod proxy;
