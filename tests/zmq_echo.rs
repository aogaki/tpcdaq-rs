//! echo コンポーネントの最小配線デモ(REQ/REP コマンド + PUSH/PULL データ)。
//!
//! TODO/003 の受け入れ条件「echo デモが Configure→Arm→Start→Stop の全遷移を通る」。
//! ポートは `tcp://127.0.0.1:0` で bind し、実エンドポイントを last_endpoint から得る
//! (固定ポート禁止 — 並列テスト・CI 安全)。

#![allow(clippy::unwrap_used)]

use std::time::Duration;

use tokio::sync::{broadcast, oneshot};
use tpcdaq::command::{run_command_task, Command, CommandResponse, ComponentState, RunConfig};
use tpcdaq::msg::{Batch, Message, RawFrames};
use tpcdaq::zmq_helper;

/// `tcp://127.0.0.1:0` に bind し、実エンドポイント(ポート確定後)を返す。
fn bind_ephemeral_pull(ctx: &zmq::Context) -> (zmq::Socket, String) {
    let sock = ctx.socket(zmq::PULL).unwrap();
    sock.set_linger(0).unwrap();
    zmq_helper::apply_pull_hwm(&sock).unwrap();
    sock.bind("tcp://127.0.0.1:0").unwrap();
    sock.set_rcvtimeo(5_000).unwrap(); // ハングではなく失敗で終わらせる
    let endpoint = sock.get_last_endpoint().unwrap().unwrap();
    (sock, endpoint)
}

/// テスト用 echo コンポーネント。コマンドで状態機械を回し、Start で 1 バッチ、
/// Stop で EndOfStream をデータリンクへ流す。
struct Echo {
    state: ComponentState,
    run_number: Option<u32>,
    sequence_number: u64,
    push: zmq::Socket,
}

impl Echo {
    fn handle(&mut self, cmd: Command) -> CommandResponse {
        let Some(target) = cmd.target_state() else {
            // GetStatus: 遷移しない
            let mut resp = CommandResponse::success(self.state, "status");
            if let Some(run) = self.run_number {
                resp = resp.with_run_number(run);
            }
            return resp;
        };
        if !self.state.can_transition_to(target) {
            return CommandResponse::error(
                self.state,
                format!("illegal transition {} -> {target}", self.state),
            );
        }
        self.state = target;
        match &cmd {
            Command::Configure(cfg) => self.run_number = Some(cfg.run_number),
            Command::Start { run_number } => {
                self.run_number = Some(*run_number);
                self.sequence_number = 0;
                self.send(Message::Data(Batch {
                    source_id: 0, // receiver cobo_id=0(SPEC §3.2)
                    run_number: *run_number,
                    sequence_number: 0,
                    created_ns: 1_755_000_000_000_000_001,
                    payload: vec![serde_bytes::ByteBuf::from(vec![0x08, 0x05, 0x00, 0x01])],
                }));
            }
            Command::Stop => {
                let run = self.run_number.unwrap_or(0);
                self.send(Message::<RawFrames>::EndOfStream {
                    source_id: 0,
                    run_number: run,
                });
            }
            Command::Reset => self.run_number = None,
            _ => {}
        }
        let mut resp = CommandResponse::success(self.state, format!("{cmd} ok"));
        if let Some(run) = self.run_number {
            resp = resp.with_run_number(run);
        }
        resp
    }

    fn send(&mut self, msg: Message<RawFrames>) {
        let bytes = msg.to_msgpack().unwrap();
        self.push.send(&bytes[..], 0).unwrap();
        self.sequence_number += 1;
    }
}

/// REQ で 1 往復する(毎回新しい REQ ソケット = REQ の厳密な送受交互制約を避ける)。
async fn rpc_bytes(endpoint: &str, request: Vec<u8>) -> Vec<u8> {
    let ctx = tmq::Context::new();
    let sender = tmq::request(&ctx).connect(endpoint).unwrap();
    {
        use tmq::AsZmqSocket;
        sender.get_socket().set_linger(0).unwrap();
    }
    let receiver = sender.send(vec![request].into()).await.unwrap();
    let (mut reply, _sender) = receiver.recv().await.unwrap();
    reply.pop_front().unwrap().to_vec()
}

async fn rpc(endpoint: &str, cmd: &Command) -> CommandResponse {
    let bytes = rpc_bytes(endpoint, cmd.to_json().unwrap()).await;
    CommandResponse::from_json(&bytes).unwrap()
}

#[tokio::test]
async fn echo_component_runs_the_full_configure_arm_start_stop_sequence() {
    let ctx = zmq::Context::new();
    let (pull, data_endpoint) = bind_ephemeral_pull(&ctx);

    let push = ctx.socket(zmq::PUSH).unwrap();
    push.set_linger(0).unwrap();
    zmq_helper::apply_push_hwm(&push).unwrap();
    push.connect(&data_endpoint).unwrap();

    let echo = Echo {
        state: ComponentState::Idle,
        run_number: None,
        sequence_number: 0,
        push,
    };

    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    let (ep_tx, ep_rx) = oneshot::channel();
    let mut echo = echo;
    let task = tokio::spawn(run_command_task(
        "tcp://127.0.0.1:0".to_string(),
        "echo",
        shutdown_rx,
        Some(ep_tx),
        move |cmd| echo.handle(cmd),
    ));
    let cmd_endpoint = ep_rx.await.unwrap();
    assert!(
        cmd_endpoint.starts_with("tcp://127.0.0.1:"),
        "unexpected endpoint {cmd_endpoint}"
    );

    // Idle → Configured
    let resp = rpc(
        &cmd_endpoint,
        &Command::Configure(RunConfig {
            run_number: 17,
            comment: "echo demo".to_string(),
            config: serde_json::Value::Null,
        }),
    )
    .await;
    assert!(resp.success, "{}", resp.message);
    assert_eq!(resp.state, ComponentState::Configured);
    assert_eq!(resp.run_number, Some(17));

    // 非合法: Configured から Start は通らず、状態も変わらない
    let resp = rpc(&cmd_endpoint, &Command::Start { run_number: 17 }).await;
    assert!(!resp.success);
    assert_eq!(resp.state, ComponentState::Configured);

    // 壊れた JSON でもコンポーネントは死なず、状態を保ったまま失敗を返す
    let bytes = rpc_bytes(&cmd_endpoint, b"{ not a command }".to_vec()).await;
    let resp = CommandResponse::from_json(&bytes).unwrap();
    assert!(!resp.success);
    assert_eq!(resp.state, ComponentState::Configured);

    // Configured → Armed → Running
    let resp = rpc(&cmd_endpoint, &Command::Arm).await;
    assert!(resp.success, "{}", resp.message);
    assert_eq!(resp.state, ComponentState::Armed);

    let resp = rpc(&cmd_endpoint, &Command::Start { run_number: 17 }).await;
    assert!(resp.success, "{}", resp.message);
    assert_eq!(resp.state, ComponentState::Running);

    // GetStatus は遷移しない
    let resp = rpc(&cmd_endpoint, &Command::GetStatus).await;
    assert!(resp.success);
    assert_eq!(resp.state, ComponentState::Running);
    assert_eq!(resp.run_number, Some(17));

    // Running → Configured
    let resp = rpc(&cmd_endpoint, &Command::Stop).await;
    assert!(resp.success, "{}", resp.message);
    assert_eq!(resp.state, ComponentState::Configured);

    // Configured → Idle
    let resp = rpc(&cmd_endpoint, &Command::Reset).await;
    assert!(resp.success, "{}", resp.message);
    assert_eq!(resp.state, ComponentState::Idle);

    // データリンク: Start の 1 バッチ → Stop の EndOfStream の順で届く
    let first = pull.recv_bytes(0).unwrap();
    let msg: Message<RawFrames> = Message::from_msgpack(&first).unwrap();
    match msg {
        Message::Data(batch) => {
            assert_eq!(batch.source_id, 0);
            assert_eq!(batch.run_number, 17);
            assert_eq!(batch.sequence_number, 0);
            assert_eq!(batch.payload.len(), 1);
            assert_eq!(batch.payload[0].as_ref(), &[0x08, 0x05, 0x00, 0x01]);
        }
        other => panic!("expected Data, got {other:?}"),
    }

    let second = pull.recv_bytes(0).unwrap();
    let msg: Message<RawFrames> = Message::from_msgpack(&second).unwrap();
    assert_eq!(
        msg,
        Message::EndOfStream {
            source_id: 0,
            run_number: 17
        }
    );
    assert!(!msg.is_droppable(), "EOS は間引き対象にしてはならない");

    // shutdown でタスクが素直に終わる
    shutdown_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("command task did not stop within 5 s")
        .unwrap();
}
