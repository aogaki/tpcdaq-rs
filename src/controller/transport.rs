use super::*;

// ---------------------------------------------------------------------
// ecc-bridge の応答(SPEC §8.2)
// ---------------------------------------------------------------------

/// ecc-bridge の 4 フィールド応答。`state` は `Off/Idle/Described/Prepared/Ready/Running/
/// Paused/Unknown`(ECC 不達は `Unknown`)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EccReply {
    pub ok: bool,
    pub state: String,
    /// **輸送・ブリッジ層**のエラー文字列(Ice 例外 / ECC の CmdException 本文 /
    /// リクエスト不正)。043 で意味も値も変えていない。
    #[serde(default)]
    pub error: String,
    /// **GET の error フラグ**(`NO_ERR`/`WHEN_DESCRIBE`/…、SPEC §8.2 v1.14 = TODO/043)。
    ///
    /// `state` とは**別の軸**である —— 実 ECC は describe に失敗しても state は `Idle` の
    /// ままで、フラグにだけ `WHEN_DESCRIBE` を抱える(041 D-1 実測。仕組みは
    /// `tools/ecc_bridge/ecc_core.hpp` の set/clear 規則ブロック)。controller はこれを
    /// **解釈せず素通しする**(表示は P4)。
    ///
    /// `#[serde(default)]`: このフィールドを知らない古い ecc-bridge が相手でも
    /// 応答の parse は落とさない(その場合は空文字 = 「載っていなかった」)。
    #[serde(default)]
    pub ecc_error: String,
}

// ---------------------------------------------------------------------
// Transport(外への口 = テストの差し替え点)
// ---------------------------------------------------------------------

/// controller が外へ触る唯一の口。[`Sequencer`] はこれ以外の IO を知らない。
pub trait Transport {
    /// コンポーネントへ 1 コマンド(JSON REQ/REP、SPEC §2.6)。`Err` は**不達**
    /// (`success: false` の応答は `Ok` で返る — 断り方の区別は呼び手の仕事)。
    fn command(&mut self, endpoint: &str, command: &Command) -> Result<CommandResponse, String>;

    /// ecc-bridge へ 1 リクエスト(JSON REQ/REP、SPEC §8.2)。
    fn ecc(&mut self, request: &Value) -> Result<EccReply, String>;

    /// EOS 待ちなどの休止。モックは仮想時間を進めるだけでよい。
    fn sleep(&mut self, duration: Duration);
}

/// 実物の ZMQ Transport。ソケットは**呼び出し毎に作って捨てる**
/// ([`crate::command::request`] の doc コメント参照 — REQ の状態機械を持ち越さないため)。
#[derive(Clone)]
pub struct ZmqTransport {
    pub(super) context: zmq::Context,
    ecc_endpoint: String,
    command_timeout: Duration,
    ecc_timeout: Duration,
}

impl ZmqTransport {
    pub fn new(ecc_endpoint: String, command_timeout: Duration, ecc_timeout: Duration) -> Self {
        Self {
            context: zmq::Context::new(),
            ecc_endpoint,
            command_timeout,
            ecc_timeout,
        }
    }

    /// タイムアウトだけ差し替えた複製(status ポーリング用)。
    pub(super) fn with_command_timeout(&self, timeout: Duration) -> Self {
        Self {
            context: self.context.clone(),
            ecc_endpoint: self.ecc_endpoint.clone(),
            command_timeout: timeout,
            ecc_timeout: timeout,
        }
    }
}

impl Transport for ZmqTransport {
    fn command(&mut self, endpoint: &str, command: &Command) -> Result<CommandResponse, String> {
        crate::command::request(&self.context, endpoint, command, self.command_timeout)
            .map_err(|e| e.to_string())
    }

    fn ecc(&mut self, request: &Value) -> Result<EccReply, String> {
        let timeout_ms = i32::try_from(self.ecc_timeout.as_millis()).unwrap_or(i32::MAX);
        let endpoint = self.ecc_endpoint.clone();
        let fail = |what: &str, e: zmq::Error| format!("ecc-bridge {endpoint}: {what}: {e}");

        let socket = self
            .context
            .socket(zmq::REQ)
            .map_err(|e| fail("cannot create REQ socket", e))?;
        socket
            .set_rcvtimeo(timeout_ms)
            .map_err(|e| fail("cannot set rcvtimeo", e))?;
        socket
            .set_sndtimeo(timeout_ms)
            .map_err(|e| fail("cannot set sndtimeo", e))?;
        socket
            .set_linger(0)
            .map_err(|e| fail("cannot set linger", e))?;
        socket
            .connect(&endpoint)
            .map_err(|e| fail("cannot connect", e))?;

        let payload = serde_json::to_vec(request)
            .map_err(|e| format!("ecc-bridge {endpoint}: cannot encode request: {e}"))?;
        socket
            .send(payload, 0)
            .map_err(|e| fail("send failed", e))?;
        let reply = socket
            .recv_bytes(0)
            .map_err(|e| fail(&format!("no reply within {timeout_ms} ms"), e))?;
        serde_json::from_slice(&reply).map_err(|e| {
            format!(
                "ecc-bridge {endpoint}: reply is not {{ok,state,error}}: {e} ({})",
                String::from_utf8_lossy(&reply)
            )
        })
    }

    fn sleep(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }
}
