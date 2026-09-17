//! 同期 Ice クライアント接続。TODO/076 §3 Phase 1(`client.rs`)。
//!
//! ブリッジは ZMQ REP の逐次処理なので **`std::net::TcpStream` 1 本の直列**で足りる
//! (tokio 不要 — ユーザー裁定 2026-09-14)。1 接続 = 同時に 1 リクエストだけ。
//!
//! 手順:
//! 1. TCP connect(`timeout` 付き)。
//! 2. サーバからの **ValidateConnection**(ヘッダのみ 14 B)を待つ(同じ `timeout`)。
//! 3. 以後は [`IceConnection::invoke`]。**呼び出しには期限を設けない**
//!    (SPEC §8.2 v1.20: 実 ECC の configure は 261 s かかった実績。期限は
//!    controller の REQ 60 s が持つ)。
//! 4. リプライ待ちの間に届く type 3 は ACM のハートビートとして捨てる。
//!    type 4(CloseConnection)/ EOF は [`LocalError::Lost`]。
//!
//! **再接続はここではしない**(TODO/076 §2.2)。`Lost` を受けた持ち主(Phase 2 の
//! プロキシ)が接続を捨てて次回張り直す。CloseConnection は Ice の規約上
//! 「保留中のリクエストを 1 つも処理していない」ときにしか来ないので、その
//! 呼び出しは**未実行**であり再送してよい — 判断できるよう原因文字列に残す。

use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use super::error::{IceError, LocalError};
use super::proto::{
    Identity, MessageHeader, MessageType, OperationMode, Reply, Request, HEADER_SIZE,
};

/// ECC との TCP 接続 1 本。
#[derive(Debug)]
pub struct IceConnection {
    stream: TcpStream,
    peer: SocketAddr,
    next_request_id: i32,
}

impl IceConnection {
    /// 接続して ValidateConnection を待つ。`timeout` は TCP connect と
    /// ValidateConnection 待ちの両方に効く(呼び出し自体には効かない)。
    pub fn connect(peer: SocketAddr, timeout: Duration) -> Result<Self, IceError> {
        let stream = TcpStream::connect_timeout(&peer, timeout)
            .map_err(|err| IceError::connect(format!("tcp connect to {peer} failed: {err}")))?;
        // 制御面の要求/応答は小さく往復するので Nagle を切る(Ice の既定と同じ)。
        stream
            .set_nodelay(true)
            .map_err(|err| IceError::connect(format!("set_nodelay on {peer} failed: {err}")))?;
        stream.set_read_timeout(Some(timeout)).map_err(|err| {
            IceError::connect(format!("set_read_timeout on {peer} failed: {err}"))
        })?;

        let mut connection = IceConnection {
            stream,
            peer,
            next_request_id: 1,
        };

        let (header, _message) = connection.read_message().map_err(|err| match err {
            IceError::Local(LocalError::Lost(cause)) => IceError::connect(format!(
                "no ValidateConnection from {peer} within {timeout:?}: {cause}"
            )),
            other => other,
        })?;
        if header.message_type != MessageType::ValidateConnection {
            return Err(IceError::protocol(format!(
                "first message from {peer} is {:?}, expected ValidateConnection",
                header.message_type
            )));
        }

        // 以後は無期限(SPEC §8.2 v1.20)。
        connection.stream.set_read_timeout(None).map_err(|err| {
            IceError::connect(format!("clearing the read timeout on {peer} failed: {err}"))
        })?;
        Ok(connection)
    }

    pub fn peer(&self) -> SocketAddr {
        self.peer
    }

    /// twoway 呼び出し 1 本。返り値は **out encapsulation の中身**
    /// (6 B のヘッダを除いたパラメータ列。void なら空)。
    pub fn invoke(
        &mut self,
        identity: &Identity,
        operation: &str,
        mode: OperationMode,
        params: &[u8],
    ) -> Result<Vec<u8>, IceError> {
        let request_id = self.take_request_id();
        let request = Request::new(
            request_id,
            identity.clone(),
            operation,
            mode,
            params.to_vec(),
        );
        let bytes = request.to_bytes()?;
        self.stream.write_all(&bytes).map_err(|err| {
            IceError::lost(format!(
                "sending {operation} (request {request_id}) to {} failed: {err}",
                self.peer
            ))
        })?;
        self.stream.flush().map_err(|err| {
            IceError::lost(format!(
                "flushing {operation} (request {request_id}) to {} failed: {err}",
                self.peer
            ))
        })?;

        loop {
            let (header, message) = self.read_message()?;
            match header.message_type {
                MessageType::Reply => {
                    let reply = Reply::parse(&message)?;
                    if reply.request_id != request_id {
                        return Err(IceError::protocol(format!(
                            "reply carries request id {} while waiting for {request_id}",
                            reply.request_id
                        )));
                    }
                    return reply.into_result();
                }
                MessageType::ValidateConnection => {
                    // ACM のハートビート。落とすが黙らない。
                    tracing::debug!(
                        peer = %self.peer,
                        operation,
                        request_id,
                        "ice heartbeat while waiting for a reply"
                    );
                }
                MessageType::CloseConnection => {
                    return Err(IceError::lost(format!(
                        "{} sent CloseConnection while {operation} (request {request_id}) was \
                         outstanding; the request was not dispatched",
                        self.peer
                    )));
                }
                other => {
                    return Err(IceError::protocol(format!(
                        "unexpected {other:?} message from {}",
                        self.peer
                    )));
                }
            }
        }
    }

    /// requestId は 1 から連番(0 は oneway 予約)。i32 の端で 1 に巻き戻す。
    fn take_request_id(&mut self) -> i32 {
        let id = self.next_request_id;
        self.next_request_id = if id == i32::MAX { 1 } else { id + 1 };
        id
    }

    /// メッセージ 1 本(ヘッダ + 本体)を読む。
    fn read_message(&mut self) -> Result<(MessageHeader, Vec<u8>), IceError> {
        let mut head = [0u8; HEADER_SIZE];
        self.stream
            .read_exact(&mut head)
            .map_err(|err| self.read_error("message header", &err))?;
        let header = MessageHeader::parse(&head)?;

        let mut message = Vec::with_capacity(header.size);
        message.extend_from_slice(&head);
        if header.size > HEADER_SIZE {
            message.resize(header.size, 0);
            self.stream
                .read_exact(&mut message[HEADER_SIZE..])
                .map_err(|err| self.read_error("message body", &err))?;
        }
        Ok((header, message))
    }

    fn read_error(&self, what: &str, err: &std::io::Error) -> IceError {
        if err.kind() == ErrorKind::UnexpectedEof {
            return IceError::lost(format!(
                "{} closed the connection while reading {what}",
                self.peer
            ));
        }
        IceError::lost(format!("reading {what} from {} failed: {err}", self.peer))
    }
}

// ---------------------------------------------------------------------
// テスト(仕様書 — 偽サーバは std::net::TcpListener でインプロセスに立てる)
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::ice::error::LocalError;
    use crate::ice::proto::{
        close_connection_message, validate_connection_message, MessageHeader, Reply, ReplyBody,
        ReplyStatus, Request, HEADER_SIZE,
    };
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::thread;

    const SHORT: Duration = Duration::from_millis(500);

    /// 偽 ECC。127.0.0.1 の空きポートで **1 接続だけ**受けて `script` を回す。
    fn spawn_server<F>(script: F) -> SocketAddr
    where
        F: FnOnce(&mut TcpStream) + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            script(&mut stream);
        });
        addr
    }

    /// ヘッダ 14 B + 本体で 1 メッセージ読む。
    fn read_message(stream: &mut TcpStream) -> Vec<u8> {
        let mut head = [0u8; HEADER_SIZE];
        stream.read_exact(&mut head).unwrap();
        let header = MessageHeader::parse(&head).unwrap();
        let mut message = head.to_vec();
        message.resize(header.size, 0);
        stream.read_exact(&mut message[HEADER_SIZE..]).unwrap();
        message
    }

    fn ok_reply(request_id: i32, payload: Vec<u8>) -> Vec<u8> {
        Reply {
            request_id,
            body: ReplyBody::Ok(payload),
        }
        .to_bytes()
        .unwrap()
    }

    #[test]
    fn connect_waits_for_the_validate_connection_message_then_invokes() {
        let (tx, rx): (Sender<Request>, Receiver<Request>) = mpsc::channel();
        let addr = spawn_server(move |stream| {
            stream.write_all(&validate_connection_message()).unwrap();
            let message = read_message(stream);
            let request = Request::parse(&message).unwrap();
            let reply = ok_reply(request.request_id, vec![0x02, 0x05]);
            tx.send(request).unwrap();
            stream.write_all(&reply).unwrap();
            // クライアントが読み終えるまで接続を開けておく。
            let mut sink = [0u8; 1];
            let _ = stream.read(&mut sink);
        });

        let mut conn = IceConnection::connect(addr, SHORT).unwrap();
        let out = conn
            .invoke(
                &Identity::new("Ecc"),
                "getStatus",
                OperationMode::Normal,
                &[],
            )
            .unwrap();
        assert_eq!(out, vec![0x02, 0x05]);

        let request = rx.recv().unwrap();
        assert_eq!(request.request_id, 1); // requestId は 1 から(0 は oneway 予約)
        assert_eq!(request.identity, Identity::new("Ecc"));
        assert_eq!(request.operation, "getStatus");
        assert_eq!(request.mode, OperationMode::Normal);
        assert!(request.facet.is_empty());
        assert!(request.context.is_empty());
        assert!(request.params.is_empty());
    }

    #[test]
    fn the_parameters_reach_the_server_unchanged() {
        let (tx, rx) = mpsc::channel();
        let addr = spawn_server(move |stream| {
            stream.write_all(&validate_connection_message()).unwrap();
            let request = Request::parse(&read_message(stream)).unwrap();
            let reply = ok_reply(request.request_id, Vec::new());
            tx.send(request).unwrap();
            stream.write_all(&reply).unwrap();
            let mut sink = [0u8; 1];
            let _ = stream.read(&mut sink);
        });

        // configure の実引数を模した 255 B 超のペイロード。
        let mut params = crate::ice::codec::OutputStream::new();
        params.write_string("configId").unwrap();
        params.write_string(&"x".repeat(300)).unwrap();
        let params = params.into_bytes();

        let mut conn = IceConnection::connect(addr, SHORT).unwrap();
        let out = conn
            .invoke(
                &Identity::new("Ecc"),
                "configure",
                OperationMode::Idempotent,
                &params,
            )
            .unwrap();
        assert!(out.is_empty()); // void オペレーション = 空 encapsulation

        let request = rx.recv().unwrap();
        assert_eq!(request.operation, "configure");
        assert_eq!(request.mode, OperationMode::Idempotent);
        assert_eq!(request.params, params);
    }

    #[test]
    fn request_ids_start_at_one_and_increase_by_one() {
        let (tx, rx) = mpsc::channel();
        let addr = spawn_server(move |stream| {
            stream.write_all(&validate_connection_message()).unwrap();
            for _ in 0..3 {
                let request = Request::parse(&read_message(stream)).unwrap();
                let reply = ok_reply(request.request_id, Vec::new());
                tx.send(request.request_id).unwrap();
                stream.write_all(&reply).unwrap();
            }
            let mut sink = [0u8; 1];
            let _ = stream.read(&mut sink);
        });

        let mut conn = IceConnection::connect(addr, SHORT).unwrap();
        for _ in 0..3 {
            conn.invoke(&Identity::new("Ecc"), "start", OperationMode::Normal, &[])
                .unwrap();
        }
        assert_eq!(
            [rx.recv().unwrap(), rx.recv().unwrap(), rx.recv().unwrap()],
            [1, 2, 3]
        );
    }

    #[test]
    fn a_heartbeat_between_the_request_and_the_reply_is_skipped() {
        let addr = spawn_server(|stream| {
            stream.write_all(&validate_connection_message()).unwrap();
            let request = Request::parse(&read_message(stream)).unwrap();
            // ACM のハートビート(type 3)がリプライの前に割り込む。
            stream.write_all(&validate_connection_message()).unwrap();
            stream
                .write_all(&ok_reply(request.request_id, vec![0x01]))
                .unwrap();
            let mut sink = [0u8; 1];
            let _ = stream.read(&mut sink);
        });

        let mut conn = IceConnection::connect(addr, SHORT).unwrap();
        let out = conn
            .invoke(
                &Identity::new("Ecc"),
                "describe",
                OperationMode::Normal,
                &[],
            )
            .unwrap();
        assert_eq!(out, vec![0x01]);
    }

    #[test]
    fn a_close_connection_while_waiting_is_reported_as_lost() {
        let addr = spawn_server(|stream| {
            stream.write_all(&validate_connection_message()).unwrap();
            let _ = read_message(stream);
            stream.write_all(&close_connection_message()).unwrap();
            let mut sink = [0u8; 1];
            let _ = stream.read(&mut sink);
        });

        let mut conn = IceConnection::connect(addr, SHORT).unwrap();
        let err = conn
            .invoke(
                &Identity::new("Ecc"),
                "describe",
                OperationMode::Normal,
                &[],
            )
            .unwrap_err();
        match err {
            IceError::Local(LocalError::Lost(cause)) => {
                assert!(cause.contains("CloseConnection"), "{cause}");
            }
            other => panic!("expected Lost, got {other:?}"),
        }
    }

    #[test]
    fn an_abrupt_disconnect_while_waiting_is_reported_as_lost() {
        let addr = spawn_server(|stream| {
            stream.write_all(&validate_connection_message()).unwrap();
            let _ = read_message(stream);
            // リプライを返さずに切る(ECC の異常終了 / ネットワーク断)。
        });

        let mut conn = IceConnection::connect(addr, SHORT).unwrap();
        let err = conn
            .invoke(
                &Identity::new("Ecc"),
                "describe",
                OperationMode::Normal,
                &[],
            )
            .unwrap_err();
        assert!(
            matches!(err, IceError::Local(LocalError::Lost(_))),
            "{err:?}"
        );
    }

    #[test]
    fn a_reply_for_another_request_id_is_a_protocol_error() {
        let addr = spawn_server(|stream| {
            stream.write_all(&validate_connection_message()).unwrap();
            let request = Request::parse(&read_message(stream)).unwrap();
            stream
                .write_all(&ok_reply(request.request_id + 100, Vec::new()))
                .unwrap();
            let mut sink = [0u8; 1];
            let _ = stream.read(&mut sink);
        });

        let mut conn = IceConnection::connect(addr, SHORT).unwrap();
        let err = conn
            .invoke(&Identity::new("Ecc"), "stop", OperationMode::Normal, &[])
            .unwrap_err();
        match err {
            IceError::Local(LocalError::Protocol(cause)) => {
                assert!(cause.contains("101"), "{cause}");
                assert!(cause.contains("1"), "{cause}");
            }
            other => panic!("expected a protocol error, got {other:?}"),
        }
    }

    #[test]
    fn a_user_exception_reply_surfaces_as_a_user_exception() {
        let addr = spawn_server(|stream| {
            stream.write_all(&validate_connection_message()).unwrap();
            let request = Request::parse(&read_message(stream)).unwrap();
            let mut reason = crate::ice::codec::OutputStream::new();
            reason.write_string("no such config").unwrap();
            let reply = Reply {
                request_id: request.request_id,
                body: ReplyBody::UserException {
                    type_id: "::mdaq::utl::CmdException".to_string(),
                    body: reason.into_bytes(),
                },
            }
            .to_bytes()
            .unwrap();
            stream.write_all(&reply).unwrap();
            let mut sink = [0u8; 1];
            let _ = stream.read(&mut sink);
        });

        let mut conn = IceConnection::connect(addr, SHORT).unwrap();
        let err = conn
            .invoke(
                &Identity::new("Ecc"),
                "describe",
                OperationMode::Normal,
                &[],
            )
            .unwrap_err();
        match err {
            IceError::UserException { type_id, body } => {
                assert_eq!(type_id, "::mdaq::utl::CmdException");
                let mut reason = crate::ice::codec::InputStream::new(&body);
                assert_eq!(reason.read_string().unwrap(), "no such config");
            }
            other => panic!("expected a user exception, got {other:?}"),
        }
    }

    #[test]
    fn an_operation_not_exist_reply_surfaces_as_request_failed() {
        let addr = spawn_server(|stream| {
            stream.write_all(&validate_connection_message()).unwrap();
            let request = Request::parse(&read_message(stream)).unwrap();
            let reply = Reply {
                request_id: request.request_id,
                body: ReplyBody::RequestFailed {
                    status: ReplyStatus::OperationNotExist,
                    identity: Identity::new("Ecc"),
                    facet: String::new(),
                    operation: "breakup".to_string(),
                },
            }
            .to_bytes()
            .unwrap();
            stream.write_all(&reply).unwrap();
            let mut sink = [0u8; 1];
            let _ = stream.read(&mut sink);
        });

        let mut conn = IceConnection::connect(addr, SHORT).unwrap();
        let err = conn
            .invoke(&Identity::new("Ecc"), "breakup", OperationMode::Normal, &[])
            .unwrap_err();
        match err {
            IceError::RequestFailed {
                kind, operation, ..
            } => {
                assert_eq!(kind, ReplyStatus::OperationNotExist);
                assert_eq!(operation, "breakup");
            }
            other => panic!("expected a request-failed error, got {other:?}"),
        }
    }

    #[test]
    fn a_server_that_never_validates_fails_the_connect_with_a_timeout() {
        let addr = spawn_server(|stream| {
            // 何も書かずに抱えたまま(ValidateConnection を送ってこないサーバ)。
            let mut sink = [0u8; 1];
            let _ = stream.read(&mut sink);
        });

        let err = IceConnection::connect(addr, Duration::from_millis(150)).unwrap_err();
        match err {
            IceError::Local(LocalError::Connect(cause)) => {
                assert!(cause.contains("ValidateConnection"), "{cause}");
            }
            other => panic!("expected a connect error, got {other:?}"),
        }
    }

    #[test]
    fn the_first_message_from_the_server_must_be_validate_connection() {
        let addr = spawn_server(|stream| {
            stream.write_all(&ok_reply(1, Vec::new())).unwrap();
            let mut sink = [0u8; 1];
            let _ = stream.read(&mut sink);
        });

        let err = IceConnection::connect(addr, SHORT).unwrap_err();
        match err {
            IceError::Local(LocalError::Protocol(cause)) => {
                assert!(cause.contains("ValidateConnection"), "{cause}");
            }
            other => panic!("expected a protocol error, got {other:?}"),
        }
    }

    #[test]
    fn a_server_that_closes_before_validating_fails_the_connect() {
        let addr = spawn_server(|_stream| {});
        let err = IceConnection::connect(addr, SHORT).unwrap_err();
        assert!(
            matches!(err, IceError::Local(LocalError::Connect(_))),
            "{err:?}"
        );
    }
}
