//! `get::rc::StateMachine`(GetEcc.ice:68-82)の型付きプロキシ。TODO/076 Phase 2、SPEC §8.2。
//!
//! 移植元は `tools/ecc_bridge/ecc_bridge.cpp` の `EccClient`(Ice 3.8 の C++ マッピング)。
//! 実使用は **8 op + `getStatus` + 組込み `ice_ping`** だけで、引数は string が最大 2 本、
//! 戻りは void か `Status{State s; Error e}`、例外は `mdaq::utl::CmdException{string reason}`
//! の 1 種類しかない(TODO/076 §2.1)。
//!
//! # 再接続(C++ と同じ規則)
//!
//! Ice の C++ マッピングで LocalException になるもの —— 接続断・プロトコル違反・
//! ObjectNotExist 系・Unknown\* 系 —— を受けたら**接続を捨て、次の呼び出しで張り直す**。
//! **再送はしない**(describe/configure/start は非冪等。二重に効いたら run が壊れる)。
//! `CmdException` は ECC が正常に返した業務エラーなので接続は保つ。

use std::net::{SocketAddr, ToSocketAddrs};
use std::time::Duration;

use super::core::{Error, State};
use crate::ice::codec::{InputStream, OutputStream};
use crate::ice::proto::{Identity, OperationMode};
use crate::ice::{IceConnection, IceError};

/// `GetEcc.ice:54` の `enum State`(**順序がワイヤ上の値**)。
///
/// `enum State { Off, Idle, Described, Prepared, Ready, Running, Paused };`
/// —— 7 値。`State::Unknown` は**我々が足した**もので、ワイヤには存在しない
/// (§2.5 スキーマ漂流ガードと同じ流儀: 表と enum を突き合わせるテストを置く)。
pub const WIRE_STATES: [State; 7] = [
    State::Off,
    State::Idle,
    State::Described,
    State::Prepared,
    State::Ready,
    State::Running,
    State::Paused,
];

/// `GetEcc.ice:56-60` の `enum Error`(**順序がワイヤ上の値**)。10 値。
/// `Error::Unknown` は我々が足したもの(ワイヤには無い)。
pub const WIRE_ERRORS: [Error; 10] = [
    Error::NoErr,
    Error::WhenDescribe,
    Error::WhenPrepare,
    Error::WhenConfigure,
    Error::WhenStart,
    Error::WhenStop,
    Error::WhenPause,
    Error::WhenResume,
    Error::WhenBreakup,
    Error::WhenReset,
];

/// `MDaq/src/mdaq/utl/CmdException.ice` の型 id(継承なし = slice 1 枚)。
pub const CMD_EXCEPTION_TYPE_ID: &str = "::mdaq::utl::CmdException";

/// TCP connect + ValidateConnection 待ちの上限。
///
/// 呼び出し自体には効かない(C++ bridge は invocation timeout 未設定 = 無期限。
/// 正は controller 側の 60 s/REQ —— SPEC §8.2 v1.20)。その 60 s より十分短くして、
/// 「ECC が居ない」ときに controller が待ちきる前にブリッジが答えられるようにする。
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// ワイヤ上の `State` 値 → 我々の [`State`]。範囲外は `Unknown`
/// (C++ `map_state` の `default:` と同じ。知らない値を `Off` などに丸めない)。
pub fn state_from_wire(value: u32) -> State {
    match usize::try_from(value) {
        Ok(index) if index < WIRE_STATES.len() => WIRE_STATES[index],
        _ => State::Unknown,
    }
}

/// ワイヤ上の `Error` 値 → 我々の [`Error`]。範囲外は `Unknown`
/// (C++ `map_error` の `default:`。「知らない値を `NO_ERR` に丸めない」= 嘘をつかない)。
pub fn error_from_wire(value: u32) -> Error {
    match usize::try_from(value) {
        Ok(index) if index < WIRE_ERRORS.len() => WIRE_ERRORS[index],
        _ => Error::Unknown,
    }
}

/// 1 回の `getStatus` で取れる ECC の申告(state と error は**別の軸**。041 D-1 の
/// `IDLE / WHEN_DESCRIBE` のように、状態が正常でもフラグが立っていることがある)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observed {
    pub state: State,
    pub error: Error,
}

impl Default for Observed {
    /// 取得できなかったときの姿(SPEC §8.2「ECC 不達 = state Unknown」)。
    fn default() -> Self {
        Observed {
            state: State::Unknown,
            error: Error::Unknown,
        }
    }
}

/// Ice プロキシ文字列 `"Ecc:tcp -h HOST -p PORT"` を分解したもの。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxySpec {
    pub identity: String,
    pub host: String,
    pub port: u16,
}

/// プロキシ文字列を読む(我々が要る部分だけ: identity / `tcp` / `-h` / `-p`)。
///
/// `-e`(encoding)や `-t`(timeout)などの付随オプションは**読み飛ばす** ——
/// encoding は [`crate::ice`] が 1.1 固定、invocation timeout は controller 側が持つ。
/// 読み飛ばしたものは呼び手が [`ProxySpec`] と原文を突き合わせられるよう、
/// ここでは捨てるだけで**黙らない**(bin が起動時に原文を 1 行出す)。
pub fn parse_proxy_string(text: &str) -> Result<ProxySpec, String> {
    let mut parts = text.split(':');
    let head = parts.next().unwrap_or("").trim();
    let mut head_tokens = head.split_whitespace();
    let identity = match head_tokens.next() {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => return Err(format!("proxy string has no identity: {text:?}")),
    };
    if identity.contains('/') {
        // category 付き identity(`cat/name`)は実 ECC では使われない(実測は `Ecc`、
        // TODO/038)。黙って名前の一部として扱うと**別のオブジェクト**に話しかけるので拒否する。
        return Err(format!(
            "proxy identity with a category is not supported: {identity:?}"
        ));
    }

    let Some(endpoint) = parts.next() else {
        return Err(format!("proxy string has no endpoint: {text:?}"));
    };
    let mut tokens = endpoint.split_whitespace();
    match tokens.next() {
        Some("tcp") => {}
        Some(other) => {
            return Err(format!(
                "only tcp endpoints are supported, got {other:?} in {text:?}"
            ))
        }
        None => return Err(format!("proxy string has an empty endpoint: {text:?}")),
    }

    let mut host = None;
    let mut port = None;
    while let Some(token) = tokens.next() {
        match token {
            "-h" => host = tokens.next().map(str::to_string),
            "-p" => port = tokens.next().map(str::to_string),
            // -t(timeout)/ -e(encoding)/ -z(compress)など。値を取るものは値も飛ばす。
            "-t" | "-e" => {
                tokens.next();
            }
            _ => {}
        }
    }

    let Some(host) = host else {
        return Err(format!("proxy endpoint has no -h HOST: {text:?}"));
    };
    let Some(port) = port else {
        return Err(format!("proxy endpoint has no -p PORT: {text:?}"));
    };
    let port: u16 = port
        .parse()
        .map_err(|e| format!("bad -p PORT {port:?} in {text:?}: {e}"))?;
    if port == 0 {
        return Err(format!("port 0 is not a destination: {text:?}"));
    }

    Ok(ProxySpec {
        identity,
        host,
        port,
    })
}

/// ECC の状態機械プロキシ。接続は遅延して張り、LocalException 相当で捨てる。
#[derive(Debug)]
pub struct EccProxy {
    spec: ProxySpec,
    identity: Identity,
    timeout: Duration,
    connection: Option<IceConnection>,
}

impl EccProxy {
    pub fn new(spec: ProxySpec, timeout: Duration) -> EccProxy {
        let identity = Identity::new(spec.identity.clone());
        EccProxy {
            spec,
            identity,
            timeout,
            connection: None,
        }
    }

    /// `"Ecc:tcp -h H -p P"` から作る(CLI の `--ecc-proxy`)。
    pub fn from_proxy_string(text: &str, timeout: Duration) -> Result<EccProxy, String> {
        Ok(EccProxy::new(parse_proxy_string(text)?, timeout))
    }

    pub fn spec(&self) -> &ProxySpec {
        &self.spec
    }

    /// 接続を張っているか(テストと運用ログ用)。
    pub fn is_connected(&self) -> bool {
        self.connection.is_some()
    }

    /// 接続 + `ice_ping`。既に張れていれば何もしない(C++ `EccClient::connect`)。
    pub fn connect(&mut self) -> Result<(), String> {
        if self.connection.is_some() {
            return Ok(());
        }
        let peer = self.resolve()?;
        let mut connection =
            IceConnection::connect(peer, self.timeout).map_err(|e| e.to_string())?;
        // 到達不能ならここで分かる。mode 2(Idempotent)・context 空は Phase 0 の実測
        // (fake_ecc 3.8 / 実 ECC 3.7 で同一バイト。tests/fixtures/ice/README.md)。
        connection
            .invoke(&self.identity, "ice_ping", OperationMode::Idempotent, &[])
            .map_err(|e| e.to_string())?;
        self.connection = Some(connection);
        Ok(())
    }

    /// `void describe(string configId)`。
    pub fn describe(&mut self, config_id: &str) -> Result<(), String> {
        let params = string_params(&[config_id])?;
        self.call("describe", OperationMode::Normal, &params)
            .map(drop)
    }

    /// `void prepare(string configId)`。
    pub fn prepare(&mut self, config_id: &str) -> Result<(), String> {
        let params = string_params(&[config_id])?;
        self.call("prepare", OperationMode::Normal, &params)
            .map(drop)
    }

    /// `void configure(string configId, string dataRouter)` —— 第 2 引数が DataLinkSet XML。
    pub fn configure(&mut self, config_id: &str, data_link_set: &str) -> Result<(), String> {
        let params = string_params(&[config_id, data_link_set])?;
        self.call("configure", OperationMode::Normal, &params)
            .map(drop)
    }

    /// `void start()`。
    pub fn start(&mut self) -> Result<(), String> {
        self.call("start", OperationMode::Normal, &[]).map(drop)
    }

    /// `void stop()`。
    pub fn stop(&mut self) -> Result<(), String> {
        self.call("stop", OperationMode::Normal, &[]).map(drop)
    }

    /// `void breakup()`。
    pub fn breakup(&mut self) -> Result<(), String> {
        self.call("breakup", OperationMode::Normal, &[]).map(drop)
    }

    /// `void reset()`。
    pub fn reset(&mut self) -> Result<(), String> {
        self.call("reset", OperationMode::Normal, &[]).map(drop)
    }

    /// `void getStatus(out Status s)` —— out パラメータは enum 2 つ(合計 2 B)。
    pub fn get_status(&mut self) -> Result<Observed, String> {
        let out = self.call("getStatus", OperationMode::Normal, &[])?;
        let mut input = InputStream::new(&out);
        let state = input
            .read_enum()
            .map_err(|e| format!("getStatus reply: cannot read Status::s: {e}"))?;
        let error = input
            .read_enum()
            .map_err(|e| format!("getStatus reply: cannot read Status::e: {e}"))?;
        if input.remaining() != 0 {
            // 構造が違うものを「読めた分だけ」で通さない(silent failure 禁止)。
            return Err(format!(
                "getStatus reply has {} unexpected trailing bytes",
                input.remaining()
            ));
        }
        Ok(Observed {
            state: state_from_wire(state),
            error: error_from_wire(error),
        })
    }

    /// 呼び出し 1 本。未接続なら繋いでから実行する(ECC が後から起動しても回復する)。
    fn call(
        &mut self,
        operation: &str,
        mode: OperationMode,
        params: &[u8],
    ) -> Result<Vec<u8>, String> {
        self.connect()?;
        let Some(connection) = self.connection.as_mut() else {
            return Err("ECC proxy not available".to_string());
        };
        let result = connection.invoke(&self.identity, operation, mode, params);
        match result {
            Ok(out) => Ok(out),
            Err(err) => Err(self.fail(err)),
        }
    }

    /// Ice の失敗 → ブリッジが返す 1 行(SPEC §8.2 の `error`)。
    fn fail(&mut self, err: IceError) -> String {
        match err {
            IceError::UserException { type_id, body } if type_id == CMD_EXCEPTION_TYPE_ID => {
                // `CmdException { string reason }`。ECC が正常に返した業務エラーなので
                // 接続は保つ(C++ も CmdException では proxy を捨てない)。
                match InputStream::new(&body).read_string() {
                    Ok(reason) => reason,
                    Err(e) => format!("{type_id} with an unreadable reason: {e}"),
                }
            }
            IceError::UserException { type_id, .. } => {
                // StateMachine の op は CmdException しか投げない(GetEcc.ice)。
                format!("unexpected user exception {type_id}")
            }
            other => {
                // Ice の C++ マッピングではここに来るものは全部 LocalException
                // (RequestFailed / Unknown\* も LocalException 派生)。接続を捨てて次回張り直す。
                self.connection = None;
                other.to_string()
            }
        }
    }

    /// `-h HOST -p PORT` を解決する(名前でも IP でも通す)。
    fn resolve(&self) -> Result<SocketAddr, String> {
        let target = (self.spec.host.as_str(), self.spec.port);
        let mut addrs = target
            .to_socket_addrs()
            .map_err(|e| format!("cannot resolve {}:{}: {e}", self.spec.host, self.spec.port))?;
        addrs.next().ok_or_else(|| {
            format!(
                "{}:{} resolved to no address",
                self.spec.host, self.spec.port
            )
        })
    }
}

/// string 引数だけを詰めた in パラメータ(= encapsulation の中身)。
fn string_params(values: &[&str]) -> Result<Vec<u8>, String> {
    let mut out = OutputStream::new();
    for value in values {
        out.write_string(value)
            .map_err(|e| format!("cannot marshal a string argument: {e}"))?;
    }
    Ok(out.into_bytes())
}

// ---------------------------------------------------------------------
// テスト(偽 ECC は std::net::TcpListener でインプロセスに立てる。
// バイト列の正は tests/fixtures/ice/ の Phase 0 ゴールデン = 実バイナリの実測)
// ---------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::ice::proto::HEADER_SIZE;
    use crate::ice::proto::{
        validate_connection_message, MessageHeader, Reply, ReplyBody, Request,
    };
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::thread;

    const SHORT: Duration = Duration::from_millis(500);
    const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/ice");

    /// Phase 0 ゴールデン(実 C++ bridge が出した実バイト列)を 1 本読む。
    fn golden(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(GOLDEN).join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("missing golden {}: {e}", path.display()))
    }

    fn golden_request(name: &str) -> Request {
        Request::parse(&golden(name)).unwrap()
    }

    /// ヘッダ 14 B + 本体で 1 メッセージ読む(EOF は `None`)。
    fn read_message(stream: &mut TcpStream) -> Option<Vec<u8>> {
        let mut head = [0u8; HEADER_SIZE];
        stream.read_exact(&mut head).ok()?;
        let header = MessageHeader::parse(&head).ok()?;
        let mut message = head.to_vec();
        message.resize(header.size, 0);
        stream.read_exact(&mut message[HEADER_SIZE..]).ok()?;
        Some(message)
    }

    fn ok_reply(request_id: i32, payload: Vec<u8>) -> Vec<u8> {
        Reply {
            request_id,
            body: ReplyBody::Ok(payload),
        }
        .to_bytes()
        .unwrap()
    }

    fn user_exception_reply(request_id: i32, type_id: &str, reason: &str) -> Vec<u8> {
        let mut body = OutputStream::new();
        body.write_string(reason).unwrap();
        Reply {
            request_id,
            body: ReplyBody::UserException {
                type_id: type_id.to_string(),
                body: body.into_bytes(),
            },
        }
        .to_bytes()
        .unwrap()
    }

    /// 偽 ECC。接続を**何本でも**受け(再接続の検証に要る)、n 本目のリクエストに
    /// `reply(n, request)` を返す。`None` なら**その接続を閉じる**(サーバ発の切断)。
    /// 受けたリクエストは戻り値の `Vec` に溜まる。
    fn spawn_fake_ecc<F>(reply: F) -> (SocketAddr, Arc<Mutex<Vec<Request>>>)
    where
        F: Fn(usize, &Request) -> Option<Vec<u8>> + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let server_log = Arc::clone(&log);
        thread::spawn(move || {
            let mut seen = 0usize;
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                if stream.write_all(&validate_connection_message()).is_err() {
                    continue;
                }
                while let Some(message) = read_message(&mut stream) {
                    let Ok(request) = Request::parse(&message) else {
                        break;
                    };
                    if let Ok(mut log) = server_log.lock() {
                        log.push(request.clone());
                    }
                    let answer = reply(seen, &request);
                    seen += 1;
                    match answer {
                        Some(bytes) => {
                            if stream.write_all(&bytes).is_err() {
                                break;
                            }
                        }
                        None => break,
                    }
                }
            }
        });
        (addr, log)
    }

    /// すべての呼び出しに「空の OK」を返す偽 ECC(`getStatus` だけ `payload` を載せる)。
    fn always_ok(payload: Vec<u8>) -> (SocketAddr, Arc<Mutex<Vec<Request>>>) {
        spawn_fake_ecc(move |_, request| {
            let body = if request.operation == "getStatus" {
                payload.clone()
            } else {
                Vec::new()
            };
            Some(ok_reply(request.request_id, body))
        })
    }

    fn proxy_at(addr: SocketAddr) -> EccProxy {
        EccProxy::new(
            ProxySpec {
                identity: "Ecc".to_string(),
                host: addr.ip().to_string(),
                port: addr.port(),
            },
            SHORT,
        )
    }

    fn requests(log: &Arc<Mutex<Vec<Request>>>) -> Vec<Request> {
        log.lock().map(|l| l.clone()).unwrap_or_default()
    }

    // -----------------------------------------------------------------
    // 1. スキーマ漂流ガード(表 ↔ enum)
    // -----------------------------------------------------------------

    /// `State`(7)/ `Error`(10)の**順序が値**である以上、表が 1 つずれたら run 制御が
    /// 静かに嘘をつく。`GetEcc.ice:54-60` の綴りをここに書き写して突き合わせる。
    #[test]
    fn the_wire_tables_match_the_geteccice_enum_order() {
        // GetEcc.ice:54 `enum State { Off, Idle, Described, Prepared, Ready, Running, Paused };`
        let state_names = [
            "Off",
            "Idle",
            "Described",
            "Prepared",
            "Ready",
            "Running",
            "Paused",
        ];
        assert_eq!(WIRE_STATES.len(), 7, "GetEcc.ice の State は 7 値");
        for (value, name) in state_names.iter().enumerate() {
            assert_eq!(WIRE_STATES[value].as_str(), *name, "State の値 {value}");
            assert_eq!(state_from_wire(value as u32), WIRE_STATES[value]);
        }
        // GetEcc.ice:56-60 の 10 値(綴りは Ice 側、文字列は SM.h 側 = ecc::core)。
        let error_names = [
            "NO_ERR",
            "WHEN_DESCRIBE",
            "WHEN_PREPARE",
            "WHEN_CONFIGURE",
            "WHEN_START",
            "WHEN_STOP",
            "WHEN_PAUSE",
            "WHEN_RESUME",
            "WHEN_BREAKUP",
            "WHEN_RESET",
        ];
        assert_eq!(WIRE_ERRORS.len(), 10, "GetEcc.ice の Error は 10 値");
        for (value, name) in error_names.iter().enumerate() {
            assert_eq!(WIRE_ERRORS[value].as_str(), *name, "Error の値 {value}");
            assert_eq!(error_from_wire(value as u32), WIRE_ERRORS[value]);
        }
        // 範囲外は Unknown(`Off` や `NO_ERR` に丸めない — 嘘をつかない)。
        assert_eq!(state_from_wire(7), State::Unknown);
        assert_eq!(state_from_wire(u32::MAX), State::Unknown);
        assert_eq!(error_from_wire(10), Error::Unknown);
        assert_eq!(error_from_wire(u32::MAX), Error::Unknown);
    }

    // -----------------------------------------------------------------
    // 2. プロキシ文字列
    // -----------------------------------------------------------------

    #[test]
    fn proxy_strings_are_parsed_like_ice() {
        // fake_ecc が stdout に出す形(`ice_toString()`、実測)。
        assert_eq!(
            parse_proxy_string("Ecc:tcp -h 127.0.0.1 -p 58250").unwrap(),
            ProxySpec {
                identity: "Ecc".to_string(),
                host: "127.0.0.1".to_string(),
                port: 58250,
            }
        );
        // SPEC §3.1 の既定。
        assert_eq!(
            parse_proxy_string("Ecc:tcp -h 192.168.144.101 -p 46002")
                .unwrap()
                .port,
            46002
        );
        // `-t`(timeout)と `-e`(encoding)は読み飛ばす —— encoding は src/ice が 1.1 固定、
        // invocation timeout は controller の 60 s/REQ が正(SPEC §8.2 v1.20)。
        let spec = parse_proxy_string("Ecc -e 1.1:tcp -h ecc.local -p 46002 -t 60000").unwrap();
        assert_eq!(spec.identity, "Ecc");
        assert_eq!(spec.host, "ecc.local");
        assert_eq!(spec.port, 46002);
        // 属性の順序は Ice と同じく不問。
        assert_eq!(
            parse_proxy_string("Ecc:tcp -p 46002 -h 10.0.0.9")
                .unwrap()
                .host,
            "10.0.0.9"
        );
    }

    #[test]
    fn proxy_strings_we_cannot_serve_are_rejected_with_a_reason() {
        for (text, want) in [
            ("", "no identity"),
            ("Ecc", "no endpoint"),
            ("Ecc:udp -h 127.0.0.1 -p 46002", "only tcp"),
            ("Ecc:tcp -p 46002", "-h HOST"),
            ("Ecc:tcp -h 127.0.0.1", "-p PORT"),
            ("Ecc:tcp -h 127.0.0.1 -p nope", "bad -p PORT"),
            ("Ecc:tcp -h 127.0.0.1 -p 0", "port 0"),
            ("cat/Ecc:tcp -h 127.0.0.1 -p 46002", "category"),
        ] {
            let err = parse_proxy_string(text).unwrap_err();
            assert!(err.contains(want), "{text:?} -> {err:?} (want {want:?})");
        }
    }

    // -----------------------------------------------------------------
    // 3. ワイヤ(Phase 0 ゴールデンとの照合)
    // -----------------------------------------------------------------

    /// C++ bridge と同じく、接続直後に `ice_ping`(mode 2 = Idempotent・引数なし)を打つ。
    #[test]
    fn connecting_pings_the_servant_like_the_cpp_bridge() {
        let (addr, log) = always_ok(Vec::new());
        let mut proxy = proxy_at(addr);
        proxy.connect().unwrap();
        assert!(proxy.is_connected());
        // 2 度目の connect は何もしない(C++ `if (proxy_) return success`)。
        proxy.connect().unwrap();

        let seen = requests(&log);
        assert_eq!(seen.len(), 1, "ice_ping は 1 回だけ: {seen:?}");
        let want = golden_request("ice_ping_req.bin");
        assert_eq!(seen[0].operation, want.operation);
        assert_eq!(seen[0].mode, want.mode); // = Idempotent
        assert_eq!(seen[0].mode, OperationMode::Idempotent);
        assert_eq!(seen[0].identity, want.identity); // Identity{name:"Ecc", category:""}
        assert!(seen[0].params.is_empty());
        assert!(seen[0].context.is_empty());
    }

    /// `describe` / `prepare` は configId 1 本。ゴールデン(実 C++ bridge の実バイト)と
    /// **同じ operation / mode / params** になる。
    #[test]
    fn describe_and_prepare_send_the_config_id_like_the_cpp_bridge() {
        let (addr, log) = always_ok(Vec::new());
        let mut proxy = proxy_at(addr);
        proxy.describe("pulser").unwrap();
        proxy.prepare("pulser").unwrap();

        let seen = requests(&log);
        assert_eq!(seen.len(), 3, "ice_ping + describe + prepare: {seen:?}");
        for (index, name) in [(1, "describe_req.bin"), (2, "prepare_req.bin")] {
            let want = golden_request(name);
            assert_eq!(seen[index].operation, want.operation, "{name}");
            assert_eq!(seen[index].mode, want.mode, "{name}"); // = Normal
            assert_eq!(seen[index].params, want.params, "{name}");
            assert_eq!(seen[index].identity, want.identity, "{name}");
        }
    }

    /// `configure` は configId + DataLinkSet XML(283 B = string size の 0xFF+int32 形式)。
    /// **`ecc::core::data_link_set_xml` が組んだ XML が、実 C++ bridge の送ったバイト列と
    /// 一致する**ところまで通しで確かめる(核 → プロキシ → ワイヤ)。
    #[test]
    fn configure_marshals_exactly_the_bytes_the_cpp_bridge_sent() {
        use crate::ecc::core::{data_link_set_xml, Link};

        // configure_req.bin の 2 リンク(tests/fixtures/ice/README.md)。
        let xml = data_link_set_xml(&[
            Link::new("CoBo[0]", "cobo0-recv.tpcdaq-fixture.local", 47401),
            Link::new("CoBo[1]", "cobo1-recv.tpcdaq-fixture.local", 47402),
        ]);
        assert_eq!(xml.len(), 283, "255 B 超 = 0xFF+int32 形式を踏む");

        let (addr, log) = always_ok(Vec::new());
        let mut proxy = proxy_at(addr);
        proxy.configure("pulser", &xml).unwrap();

        let seen = requests(&log);
        assert_eq!(seen.len(), 2);
        let want = golden_request("configure_req.bin");
        assert_eq!(seen[1].operation, "configure");
        assert_eq!(seen[1].mode, want.mode);
        assert_eq!(seen[1].params, want.params, "configure の in パラメータ");
    }

    /// 引数なしの 4 op(start/stop/breakup/reset)は空の encapsulation。
    #[test]
    fn the_argument_less_operations_match_the_golden_requests() {
        let (addr, log) = always_ok(Vec::new());
        let mut proxy = proxy_at(addr);
        proxy.start().unwrap();
        proxy.stop().unwrap();
        proxy.breakup().unwrap();
        proxy.reset().unwrap();

        let seen = requests(&log);
        assert_eq!(seen.len(), 5, "ice_ping + 4 op");
        for (index, name) in [
            (1, "start_req.bin"),
            (2, "stop_req.bin"),
            (3, "breakup_req.bin"),
            (4, "reset_req.bin"),
        ] {
            let want = golden_request(name);
            assert_eq!(seen[index].operation, want.operation, "{name}");
            assert_eq!(seen[index].mode, want.mode, "{name}");
            assert!(seen[index].params.is_empty(), "{name}");
        }
    }

    // -----------------------------------------------------------------
    // 4. getStatus(out Status{State s; Error e})
    // -----------------------------------------------------------------

    /// 実測の 2 点で固定する: fake_ecc の `02 00` = Described/NoErr、
    /// 実 ECC(3.7.11)の `01 01` = Idle/WhenDescribe(041 D-1 の形)。
    #[test]
    fn get_status_decodes_the_two_enums() {
        let (addr, log) = always_ok(vec![0x02, 0x00]);
        let mut proxy = proxy_at(addr);
        assert_eq!(
            proxy.get_status().unwrap(),
            Observed {
                state: State::Described,
                error: Error::NoErr,
            }
        );
        let seen = requests(&log);
        let want = golden_request("getstatus_req.bin");
        assert_eq!(seen[1].operation, want.operation);
        assert_eq!(seen[1].mode, want.mode);

        let (addr, _) = always_ok(vec![0x01, 0x01]);
        let mut proxy = proxy_at(addr);
        assert_eq!(
            proxy.get_status().unwrap(),
            Observed {
                state: State::Idle,
                error: Error::WhenDescribe,
            }
        );

        // 知らない値は Unknown に落ちる(`Off` / `NO_ERR` と言い張らない)。
        let (addr, _) = always_ok(vec![0x07, 0x0a]);
        let mut proxy = proxy_at(addr);
        assert_eq!(proxy.get_status().unwrap(), Observed::default());
    }

    /// 形の違うリプライを「読めた分だけ」で通さない。
    #[test]
    fn get_status_rejects_a_reply_that_is_not_two_enums() {
        // 1 B しか無い = Status::e が読めない。
        let (addr, _) = always_ok(vec![0x02]);
        let mut proxy = proxy_at(addr);
        let err = proxy.get_status().unwrap_err();
        assert!(err.contains("Status::e"), "{err}");

        // 余分な 1 B。
        let (addr, _) = always_ok(vec![0x02, 0x00, 0x00]);
        let mut proxy = proxy_at(addr);
        let err = proxy.get_status().unwrap_err();
        assert!(err.contains("trailing"), "{err}");
    }

    // -----------------------------------------------------------------
    // 5. 例外と再接続
    // -----------------------------------------------------------------

    /// `CmdException` は **reason だけ**を返す(Ice 3.8 の `what()` は型 id しか出さないので
    /// C++ 側も個別に catch していた —— ecc_bridge.cpp:177-178)。接続は**保つ**。
    #[test]
    fn a_cmd_exception_becomes_its_reason_and_keeps_the_connection() {
        // 実 ECC(3.7.11)が返した実物(tests/fixtures/ice/real_ecc_describe_cmdexception_rep.bin)。
        const REASON: &str = "Exception: Configuration 'bogus-config-id-999' does not exist.";
        let (addr, log) = spawn_fake_ecc(move |_, request| {
            Some(if request.operation == "describe" {
                user_exception_reply(request.request_id, CMD_EXCEPTION_TYPE_ID, REASON)
            } else {
                ok_reply(request.request_id, Vec::new())
            })
        });

        let mut proxy = proxy_at(addr);
        assert_eq!(proxy.describe("bogus-config-id-999").unwrap_err(), REASON);
        // 業務エラーであって接続の問題ではない = 張ったまま次を打てる。
        assert!(proxy.is_connected());
        proxy.stop().unwrap();
        let seen = requests(&log);
        assert_eq!(seen.len(), 3, "再接続していない(ice_ping は 1 回だけ)");
    }

    /// slice に載っていない例外(あり得ないが、来たら型 id を出して黙らない)。
    #[test]
    fn an_unexpected_user_exception_is_reported_with_its_type_id() {
        let (addr, _) = spawn_fake_ecc(move |_, request| {
            Some(if request.operation == "start" {
                user_exception_reply(request.request_id, "::get::rc::SomethingElse", "why")
            } else {
                ok_reply(request.request_id, Vec::new())
            })
        });
        let mut proxy = proxy_at(addr);
        let err = proxy.start().unwrap_err();
        assert!(err.contains("::get::rc::SomethingElse"), "{err}");
    }

    /// 接続断(ACM のアイドル切断など)は**捨てて次の呼び出しで張り直す**。再送はしない。
    #[test]
    fn a_lost_connection_is_dropped_and_the_next_call_reconnects() {
        // 2 本目のリクエスト(= describe)で接続を閉じる。
        let (addr, log) = spawn_fake_ecc(|seen, request| {
            if seen == 1 {
                None // サーバ発の切断
            } else {
                Some(ok_reply(request.request_id, Vec::new()))
            }
        });

        let mut proxy = proxy_at(addr);
        let err = proxy.describe("pulser").unwrap_err();
        assert!(!err.is_empty(), "理由を残さずに失敗しない");
        assert!(!proxy.is_connected(), "LocalException 相当は接続を捨てる");

        // 次の呼び出しで張り直す(= ice_ping からやり直す)。**describe は再送しない**。
        proxy.prepare("pulser").unwrap();
        assert!(proxy.is_connected());
        let seen = requests(&log);
        let ops: Vec<&str> = seen.iter().map(|r| r.operation.as_str()).collect();
        assert_eq!(ops, vec!["ice_ping", "describe", "ice_ping", "prepare"]);
    }

    /// ECC 不達は panic ではなく理由つきの `Err`(SPEC §8.2 = `{"ok":false,"state":"Unknown"}`)。
    #[test]
    fn an_unreachable_ecc_fails_with_a_reason() {
        // 誰も listen していないポートを借りて即座に手放す。
        let probe = TcpListener::bind("127.0.0.1:0").unwrap();
        let dead = probe.local_addr().unwrap();
        drop(probe);

        let mut proxy = proxy_at(dead);
        let err = proxy.get_status().unwrap_err();
        assert!(err.contains("connect"), "{err}");
        assert!(!proxy.is_connected());
        // 2 度目も同じく答える(死なない)。
        assert!(proxy.describe("x").is_err());
    }
}
