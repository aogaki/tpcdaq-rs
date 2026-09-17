//! Ice クライアント(twoway の部分集合)。TODO/076 §2.2/§3 Phase 1、SPEC §8.2。
//!
//! ECC(GET software 20190315_patched)と話すのに必要な分だけを自前で実装する:
//! プロトコル **1.0**(ヘッダの encoding も常に 1.0)、リクエスト/リプライの
//! エンコーディングは **1.1 固定**。クラス・optional・バッチ・oneway・IceStorm・SSL は
//! 対象外(要るものが増えたら発注書を起こすこと)。
//!
//! - [`codec`] — エンコーディング 1.1 の基本型(byte/bool/int/long/string/seq/dict/enum/encapsulation)
//! - [`proto`] — メッセージ(ヘッダ 14 B、Request、Reply、ValidateConnection、CloseConnection)
//! - [`client`] — 同期 `std::net::TcpStream` の twoway 呼び出し(1 接続 = 逐次)
//! - [`error`] — 失敗の型。**壊れ入力は全部 `Err`**(silent failure を作らない)

pub mod client;
pub mod codec;
pub mod error;
pub mod proto;

pub use client::IceConnection;
pub use error::{IceError, LocalError};
pub use proto::{Identity, OperationMode, ReplyStatus};
