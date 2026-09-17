//! Ice クライアントの失敗型。TODO/076 §3 Phase 1(`error.rs`)。

use super::proto::{Identity, ReplyStatus};

/// 接続・プロトコル層の失敗(Ice の LocalException 相当)。
///
/// `Connect` / `Lost` / `Protocol` は原因文字列を持つ(CLAUDE.md「silent failure を
/// 作らない」— ブリッジのログに「何で落ちたか」が残らないと現場で切り分けられない)。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LocalError {
    /// 接続を確立できなかった(TCP connect 失敗 / ValidateConnection 待ちのタイムアウト)。
    #[error("connect failed: {0}")]
    Connect(String),
    /// 確立済みの接続が落ちた(CloseConnection 受信 / EOF / 送受信の IO エラー)。
    #[error("connection lost: {0}")]
    Lost(String),
    /// ワイヤが仕様に合わない(magic 違い / size 矛盾 / 未知のメッセージ型 /
    /// requestId 不一致 / 壊れたエンコーディング)。
    #[error("protocol violation: {0}")]
    Protocol(String),
}

/// Ice 呼び出しの失敗。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IceError {
    /// 輸送・プロトコル層(呼び出しが届いたかどうかは変種の意味に従う)。
    #[error("ice local error: {0}")]
    Local(#[from] LocalError),
    /// slice 定義に載っている例外(replyStatus 1)。`body` は最初の slice の
    /// type id の後ろから末尾までの生バイト(メンバの解釈は呼び手の仕事)。
    #[error("ice user exception {type_id} ({} bytes of members)", body.len())]
    UserException { type_id: String, body: Vec<u8> },
    /// replyStatus 2/3/4(ObjectNotExist / FacetNotExist / OperationNotExist)。
    #[error(
        "ice request failed ({kind}): identity={identity} facet={facet:?} operation={operation}"
    )]
    RequestFailed {
        kind: ReplyStatus,
        identity: Identity,
        facet: String,
        operation: String,
    },
    /// replyStatus 5/6/7(UnknownLocalException / UnknownUserException / UnknownException)。
    #[error("ice unknown exception ({kind}): {message}")]
    Unknown { kind: ReplyStatus, message: String },
}

impl IceError {
    pub(crate) fn protocol(message: impl Into<String>) -> Self {
        IceError::Local(LocalError::Protocol(message.into()))
    }

    pub(crate) fn lost(message: impl Into<String>) -> Self {
        IceError::Local(LocalError::Lost(message.into()))
    }

    pub(crate) fn connect(message: impl Into<String>) -> Self {
        IceError::Local(LocalError::Connect(message.into()))
    }
}
