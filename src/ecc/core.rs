//! ecc-bridge の **Ice 非依存**な核(SPEC §8.2)。`tools/ecc_bridge/ecc_core.hpp` の 1:1 移植
//! (TODO/076 Phase 2)。
//!
//!   * [`State`] と文字列の対応(Off/Idle/Described/Prepared/Ready/Running/Paused/Unknown)
//!   * [`Error`](GET の error フラグ)と文字列の対応 + その set/clear 規則(043、SPEC §8.2 v1.14)
//!   * DataLinkSet XML の生成(**実機の罠を仕様として固定**: DataSender id = "CoBo[k]" 形式、
//!     flowType = 大文字 "TCP")
//!   * JSON リクエストの parse とレスポンスの生成(never panic)
//!   * fake-ECC 用の状態遷移表(実 ECC の SM を模す。bridge 本体は使わない)
//!
//! Ice も ZMQ も知らない —— ここだけで `cargo test --lib` が回る。
//!
//! **移植の写し方**(C++ の out 引数 → Rust の戻り値):
//!   * `Request{valid,error,...}` → `Result<Request, String>`(`valid` = `is_ok`、
//!     `error` = `Err` の中身)
//!   * `next_state(from, action, &to, &error)` → [`Transition`] を返す

use serde_json::Value;

/// `get::rc::State` のミラー + `Unknown`(取得失敗 / ECC 不達)。
///
/// 流用元: `~/test/get/tpcdaq/include/tpcdaq/control/ecc_controller.hpp` の enum。
/// **ワイヤ上の値(順序)は [`crate::ecc::proxy::WIRE_STATES`] が持つ** —— ここは文字列だけ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Off,
    Idle,
    Described,
    Prepared,
    Ready,
    Running,
    Paused,
    Unknown,
}

impl State {
    /// C++ `ecc::to_string(State)`。
    pub fn as_str(self) -> &'static str {
        match self {
            State::Off => "Off",
            State::Idle => "Idle",
            State::Described => "Described",
            State::Prepared => "Prepared",
            State::Ready => "Ready",
            State::Running => "Running",
            State::Paused => "Paused",
            State::Unknown => "Unknown",
        }
    }

    /// C++ `ecc::state_from_string`。知らない綴りは `Unknown` に落とす(エラーにしない)。
    pub fn from_name(name: &str) -> State {
        match name {
            "Off" => State::Off,
            "Idle" => State::Idle,
            "Described" => State::Described,
            "Prepared" => State::Prepared,
            "Ready" => State::Ready,
            "Running" => State::Running,
            "Paused" => State::Paused,
            _ => State::Unknown,
        }
    }
}

impl std::fmt::Display for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// `get::rc::Error` のミラー + `Unknown`(= 我々が状態を取れなかった。実 ECC の enum には無い)。
///
/// 一次資料(reference/20190315_patched):
///   * `GetBench/src/get/rc/SM.h:58-70` … enum Error の全 10 値(NO_ERR .. WHEN_RESET)
///   * `GetBench/src/get/GetEcc.ice:56-60` … ワイヤ上の綴り(NoErr, WhenDescribe, ...)
///   * `GetBench/src/get/GetEccImpl.cpp:431-464` … SM::Error → Ice enum の変換(1 対 1)
///
/// **文字列は SM.h 側の綴り(UPPER_SNAKE)を出す** —— SPEC §8.2 v1.14 が `NO_ERR` /
/// `WHEN_DESCRIBE` と書いており、実 ECC 自身のログもこの綴り(rc/SM.cpp:73-108 の `operator<<`)。
/// オペレータが ECC のログと突き合わせる文字列なので、Ice 側の綴りに寄せない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    NoErr,
    WhenDescribe,
    WhenPrepare,
    WhenConfigure,
    WhenStart,
    WhenStop,
    WhenPause,
    WhenResume,
    WhenBreakup,
    WhenReset,
    Unknown,
}

impl Error {
    /// C++ `ecc::to_string(Error)`。
    pub fn as_str(self) -> &'static str {
        match self {
            Error::NoErr => "NO_ERR",
            Error::WhenDescribe => "WHEN_DESCRIBE",
            Error::WhenPrepare => "WHEN_PREPARE",
            Error::WhenConfigure => "WHEN_CONFIGURE",
            Error::WhenStart => "WHEN_START",
            Error::WhenStop => "WHEN_STOP",
            Error::WhenPause => "WHEN_PAUSE",
            Error::WhenResume => "WHEN_RESUME",
            Error::WhenBreakup => "WHEN_BREAKUP",
            Error::WhenReset => "WHEN_RESET",
            // 状態が取れていない(ECC 不達 / getStatus 失敗)。`NO_ERR` と**言い切らない** ——
            // 「ECC はエラー無しと申告した」と「聞けていない」は別物である
            // (CLAUDE.md: silent failure 禁止)。
            Error::Unknown => "Unknown",
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// 実 ECC の error フラグの **set / clear 規則**(043。一次資料を行番号で示す。
// テストダブルのコメントではなく reference/20190315_patched のソースが根拠)
//
//   初期値 : `BackEnd::BackEnd()`(rc/BackEnd.cpp:132)が `smErrorFlag = SM::NO_ERR`。
//
//   clear  : `BackEnd::resetErrorFlag()`(rc/BackEnd.cpp:1146-1149、NO_ERR を代入)が
//            **11 本の遷移すべての 1 番目のアクション**として登録されている
//            (rc/BackEnd.cpp:249-290)。つまり「遷移を渡り始めた瞬間」に消える。
//            逆に、その状態にその遷移が**無い**場合は dhsm が `Ignored` を返して
//            アクションを 1 つも実行しない(StateMachine/src/dhsm/Engine.cpp:338-353)ので
//            **フラグは残る**。`BackEnd::configure` の `ST_PREPARED` ガード
//            (rc/BackEnd.cpp:955-962)も同様 —— engine.step すら呼ばれないので残る。
//
//   set    : `BackEnd::handleStateMachineException()`(rc/BackEnd.cpp:329-333)が
//            `smErrorFlag = errorCode` してから SM::Exception を投げる。呼ぶのは
//            `CATCH_SM_EXCEPTIONS`(rc/BackEnd.cpp:74-99)で、各 on* ハンドラの末尾に
//            **その相のコード**で置かれている:
//              onDescribe   :808  WHEN_DESCRIBE     onPrepare   :918  WHEN_PREPARE
//              onUnDescribe :940  WHEN_RESET        onConfigure :1025 WHEN_CONFIGURE
//              onStart      :1052 WHEN_START        onStop      :1080 WHEN_STOP
//              onPause      :1101 WHEN_PAUSE        onResume    :1121 WHEN_RESUME
//              onBreakup    :1143 WHEN_BREAKUP
//
//   状態   : 失敗しても **state は動かない**。SM::Exception は std::exception を継承しない
//            (rc/SM.h:92-96 —— `struct Exception : public Status`)ので dhsm の
//            `catch (const std::exception&)`(Engine.cpp:361)に掛からず halt もされず、
//            `sm.currentState_ = transition.targetState()`(Engine.cpp:298)に**到達しないまま**
//            step() の外へ抜ける。→ 041 D-1 で実測した `IDLE / WHEN_DESCRIBE` はこの形。
//
//   読出し : `getStatus` は `smStatus()`(rc/BackEnd.cpp:321-327)を読むだけで**消さない**
//            (GetEccImpl.cpp:468-473)。フラグは次の遷移が渡るまで残り続ける。
//
// 帰結(fake_ecc / 我々のダブルが従うべき形):
//   遷移が渡った(Applied)     → まず NO_ERR にする。その後の副作用が失敗したら相のコードを立て、
//                                state は元へ戻す(渡り切っていないから)。
//   遷移が無い(Ignored/Denied)→ フラグに**触らない**。
//   status を読む               → フラグに**触らない**。
// ---------------------------------------------------------------------------

/// DataLinkSet の 1 本。CoBo 毎に 1 本(SPEC §8.2)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// ECC の NodeId 表記そのまま。実機は `"CoBo[0]"`(`.AsAd[..]` は付かない)。
    pub sender: String,
    /// 我々(receiver)の受信アドレス。
    pub router_ip: String,
    /// receiver が**実際に bind した**ポート(controller が Arm 応答から渡す)。
    pub router_port: u16,
    /// 大文字必須(sender factory が case-sensitive)。
    pub flow_type: String,
}

impl Link {
    /// flowType 既定 = `"TCP"`(C++ の `std::string flow_type = "TCP"` メンバ初期化子)。
    pub fn new(sender: impl Into<String>, router_ip: impl Into<String>, router_port: u16) -> Link {
        Link {
            sender: sender.into(),
            router_ip: router_ip.into(),
            router_port,
            flow_type: "TCP".to_string(),
        }
    }
}

/// XML 属性値のエスケープ。値は実質 ASCII だが、壊れた XML を黙って作らない。
pub fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            other => out.push(other),
        }
    }
    out
}

/// [`xml_escape`] の逆。属性値に現れうる 5 実体だけ戻す(数値参照は使わないので見ない)。
pub fn xml_unescape(s: &str) -> String {
    const ENTITIES: [(&str, char); 5] = [
        ("&quot;", '"'),
        ("&apos;", '\''),
        ("&lt;", '<'),
        ("&gt;", '>'),
        ("&amp;", '&'),
    ];
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while !rest.is_empty() {
        let mut matched = false;
        if rest.starts_with('&') {
            for (entity, ch) in ENTITIES {
                if rest.starts_with(entity) {
                    out.push(ch);
                    rest = &rest[entity.len()..];
                    matched = true;
                    break;
                }
            }
        }
        if !matched {
            let mut chars = rest.chars();
            match chars.next() {
                Some(c) => {
                    out.push(c);
                    rest = chars.as_str();
                }
                None => break,
            }
        }
    }
    out
}

/// `configure()` に注入する DataLinkSet XML。
///
/// 流用元: `tpcdaq::control::EccController::data_link_set()`
/// (`~/test/get/tpcdaq/src/control/ecc_controller.cpp:57-66`)を N リンクへ拡張したもの。
/// 受理側の正は `reference/20190315_patched/GetBench/src/get/rc/{DataLinkSet,DataLink,
/// DataSenderId,DataRouterId}.cpp`(tinyxml パースなので空白・属性順は不問。
/// DataRouter の name 属性は fromXml で optional なので出さない)。
pub fn data_link_set_xml(links: &[Link]) -> String {
    let mut xml = String::from("<DataLinkSet>");
    for l in links {
        xml.push_str("<DataLink><DataSender id=\"");
        xml.push_str(&xml_escape(&l.sender));
        xml.push_str("\"/><DataRouter ipAddress=\"");
        xml.push_str(&xml_escape(&l.router_ip));
        xml.push_str("\" port=\"");
        xml.push_str(&l.router_port.to_string());
        xml.push_str("\" type=\"");
        xml.push_str(&xml_escape(&l.flow_type));
        xml.push_str("\"/></DataLink>");
    }
    xml.push_str("</DataLinkSet>");
    xml
}

/// DataLinkSet XML を [`Link`] 配列に戻す(**fake-ECC 専用**の最小読み取り。
/// 実 ECC は tinyxml で読む —— こちらは自分が作った XML を読み返すだけなので属性走査で足りる)。
/// 壊れた入力では取れたぶんだけ返す(panic しない)。
pub fn parse_data_link_set(xml: &str) -> Vec<Link> {
    // `[from, to)` の範囲にある `name="..."` の値(見つからなければ空文字)。
    fn attr(xml: &str, from: usize, to: usize, name: &str) -> String {
        let key = format!("{name}=\"");
        let Some(rel) = xml[from..].find(&key) else {
            return String::new();
        };
        let p = from + rel;
        if p >= to {
            return String::new();
        }
        let b = p + key.len();
        let Some(rel_end) = xml[b..].find('"') else {
            return String::new();
        };
        let e = b + rel_end;
        if e > to {
            return String::new();
        }
        xml_unescape(&xml[b..e])
    }

    let mut links = Vec::new();
    let mut pos = 0usize;
    while let Some(rel) = xml[pos..].find("<DataLink>") {
        let begin = pos + rel;
        let end = match xml[begin..].find("</DataLink>") {
            Some(rel_end) => begin + rel_end,
            None => xml.len(),
        };
        let port = attr(xml, begin, end, "port");
        links.push(Link {
            sender: attr(xml, begin, end, "id"),
            router_ip: attr(xml, begin, end, "ipAddress"),
            // C++ は strtol。壊れた値は 0(= 「読めなかった」)に落とす。
            router_port: port.parse::<u16>().unwrap_or(0),
            flow_type: attr(xml, begin, end, "type"),
        });
        if end + 1 >= xml.len() {
            break;
        }
        pos = end + 1;
    }
    links
}

// ---------------------------------------------------------------------------
// JSON リクエスト / レスポンス(SPEC §8.2)
// ---------------------------------------------------------------------------

/// 受け付ける action(SPEC §8.2 の列挙そのまま)。pause/resume は §8.2 に無いので受けない。
pub fn is_known_action(action: &str) -> bool {
    matches!(
        action,
        "describe" | "prepare" | "configure" | "start" | "stop" | "breakup" | "reset" | "status"
    )
}

/// 受理済みのリクエスト(C++ `ecc::Request` の `valid == true` の姿)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub action: String,
    pub config_id: String,
    /// configure のみ。
    pub links: Vec<Link>,
}

/// JSON リクエストの parse。理由つきで拒否する(C++ は `Request::valid=false` + `error`)。
pub fn parse_request(json: &str) -> Result<Request, String> {
    let value: Value = serde_json::from_str(json).map_err(|e| format!("parse error: {e}"))?;
    let Value::Object(map) = value else {
        return Err("top-level JSON value must be an object".to_string());
    };

    let action = match map.get("action") {
        Some(Value::String(a)) if !a.is_empty() => a.clone(),
        _ => return Err("missing action".to_string()),
    };
    if !is_known_action(&action) {
        return Err(format!("unknown action: {action}"));
    }

    let config_id = match map.get("config_id") {
        None => "default".to_string(),
        Some(Value::String(c)) if !c.is_empty() => c.clone(),
        Some(_) => return Err("config_id must be a non-empty string".to_string()),
    };

    let mut links = Vec::new();
    if let Some(value) = map.get("links") {
        let Value::Array(items) = value else {
            return Err("links must be an array".to_string());
        };
        for item in items {
            let Value::Object(item) = item else {
                return Err("each entry of links must be an object".to_string());
            };
            let sender = match item.get("sender") {
                Some(Value::String(s)) if !s.is_empty() => s.clone(),
                _ => return Err("link needs a non-empty sender (e.g. \"CoBo[0]\")".to_string()),
            };
            let router_ip = match item.get("router_ip") {
                Some(Value::String(s)) if !s.is_empty() => s.clone(),
                _ => return Err("link needs a non-empty router_ip".to_string()),
            };
            let port = match item.get("router_port") {
                // C++ `jsonmin::Value::as_int()` は number をそのまま整数化する
                // (`-2.5e1` → `-25`)ので、小数も同じく切り捨てで受けて範囲検査に回す。
                Some(Value::Number(n)) => match n.as_i64() {
                    Some(i) => i,
                    None => n.as_f64().unwrap_or(0.0) as i64,
                },
                _ => return Err("link needs a numeric router_port".to_string()),
            };
            if !(1..=65535).contains(&port) {
                return Err(format!("router_port out of range (1..65535): {port}"));
            }
            let mut link = Link::new(sender, router_ip, port as u16);
            if let Some(value) = item.get("type") {
                let Value::String(flow_type) = value else {
                    return Err("link type must be a string".to_string());
                };
                // 実機の罠(SPEC §8.2): flowType は大文字 "TCP"。小文字を**黙って直さない** ——
                // 直してしまうと「設定が間違っていた」事実が消え、本番で同じ罠を踏み直す。
                if flow_type != "TCP" {
                    return Err(format!(
                        "link type must be upper-case \"TCP\", got \"{flow_type}\""
                    ));
                }
                link.flow_type = flow_type.clone();
            }
            links.push(link);
        }
    }

    if action == "configure" && links.is_empty() {
        return Err("configure needs a non-empty links array".to_string());
    }

    Ok(Request {
        action,
        config_id,
        links,
    })
}

/// JSON 文字列値のエスケープ(C++ `jsonmin::escape` の 1:1 移植)。
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// レスポンス(SPEC §8.2)。4 キーを**常に**出す(Rust 側の契約を単純にする)。
///
/// `error` = **輸送・ブリッジ層**のエラー文字列(Ice 例外 / CmdException の本文 /
/// リクエスト不正)。043 で意味も値も変えていない。
///
/// `ecc_error` = **GET の error フラグ**(`NO_ERR`/`WHEN_DESCRIBE`/… — 上の set/clear 規則)。
/// SPEC §8.2 v1.14 は `{"action":"status"}` の応答について定めているが、ここでは**全 action の
/// 応答に**載せる —— bridge はどの action の後にも getStatus を打つので追加コストが無く、
/// 失敗した describe の応答でそのまま `WHEN_DESCRIBE` が読めるほうが運用で嬉しい。
///
/// **キー順は C++ と同じ**(`ok`/`state`/`error`/`ecc_error`)。serde_json の
/// `Map` は既定でキーを並べ替えるので、ここは手で組む。
pub fn make_response(ok: bool, state: State, error: &str, ecc_error: Error) -> String {
    format!(
        "{{\"ok\":{},\"state\":\"{}\",\"error\":\"{}\",\"ecc_error\":\"{}\"}}",
        if ok { "true" } else { "false" },
        state.as_str(),
        json_escape(error),
        ecc_error.as_str()
    )
}

// ---------------------------------------------------------------------------
// fake-ECC の状態遷移表(TODO/017 §2 →**036 で実 ECC の SM に合わせて厳格化**)
//
// **この表の前身は「reset はどこからでも Idle(復旧手段は塞がない)」だった。それが
// TODO/034 の誤実装(`Ready` から reset 1 発で Idle に戻るつもりの run 開始)を green で
// 通してしまった。** テストダブルのコメントを一次資料として扱ってはならない —— 一次資料は
// 実 ECC のソース(reference/20190315_patched)だけである(036 の教訓)。
//
// 実 ECC の SM(出典を行番号まで書く):
//   GetBench/src/get/rc/BackEnd.cpp:250-270  … SM の全遷移
//       EV_DESCRIBE  : Idle      -> Described
//       EV_UNDO      : Described -> Idle / Prepared -> Described   ← **この 2 本だけ**
//       EV_PREPARE   : Described -> Prepared
//       EV_CONFIGURE : Prepared  -> Active(初期部分状態 = Ready)
//       EV_BREAK     : Active    -> Prepared      (Active = Ready/Running/Paused の複合状態)
//       EV_START/EV_STOP : Ready <-> Running(Active 部分機械。EV_STOP は Paused からも)
//   GetBench/src/get/rc/BackEnd.cpp:924      … reset() は engine.step(EV_UNDO) **のみ**
//   GetBench/src/get/rc/BackEnd.cpp:955-962  … configure() は `if (state == ST_PREPARED)`
//                                              ガード付き = それ以外は**黙ってスキップ**
//   StateMachine/src/dhsm/Engine.cpp:334-352 … 未定義遷移は例外を投げず **Ignored**(完全な無音)
//
// 帰結: 前の run の `ecc stop` が置いた `Ready` から `Idle` へ戻るには
// **breakup → reset → reset** の歩き戻しが要る(SPEC §1.3 v1.12。歩くのは controller)。
//
// **実機より辛くしてある 1 点**: describe / prepare / start / stop の順序違反を、実機は
// 無音の Ignored にするがここでは Denied(= CmdException)にする。テストダブルは実機より
// **甘くしてはならない**が、辛い分には「run が可視に失敗する」だけで事故にならない。
// この辛さがあるおかげで、歩き戻しを省いた実装は E2E で必ず落ちる。
// **bridge 本体はこの表を使わない**(状態の正は常に実 ECC の getStatus)。
// ---------------------------------------------------------------------------

/// 1 遷移の結末。実 ECC の `dhsm::Engine::ProcessingResult` に対応する
/// (`Applied` = crossTransition した / `Ignored` = 遷移が無く**無音で**何も起きない)。
/// `Denied` は本テストダブル固有の厳しさ(上記「実機より辛くしてある 1 点」)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Applied,
    Ignored,
    Denied,
}

/// [`next_state`] の結果(C++ の out 引数 `to` / `error` をまとめたもの)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub step: Step,
    /// 次状態。`Applied` のときだけ動く。
    pub next: State,
    /// `Denied` のときだけ理由が入る(他は空)。
    pub error: String,
}

/// fake-ECC の遷移表。C++ `ecc::next_state`。
pub fn next_state(from: State, action: &str) -> Transition {
    // 既定は「動かない」。Applied のときだけ書き換える。
    let stay = |step: Step| Transition {
        step,
        next: from,
        error: String::new(),
    };
    let applied = |next: State| Transition {
        step: Step::Applied,
        next,
        error: String::new(),
    };
    let denied = || Transition {
        step: Step::Denied,
        next: from,
        error: format!("invalid transition: {action} in state {from}"),
    };

    // ST_ACTIVE(複合状態)に居るか。EV_UNDO はここからは定義されていない。
    let active = matches!(from, State::Ready | State::Running | State::Paused);

    // status は観測であって遷移ではない(状態を変えない)。
    if action == "status" {
        return stay(Step::Applied);
    }
    // EV_UNDO = **1 段戻すだけ**。Active からは遷移が無いので無音で無視される。
    if action == "reset" {
        return match from {
            State::Described => applied(State::Idle),
            State::Prepared => applied(State::Described),
            _ => stay(Step::Ignored),
        };
    }
    // EV_BREAK = Active(Ready/Running/Paused)→ Prepared。それ以外からは無音。
    if action == "breakup" {
        return if active {
            applied(State::Prepared)
        } else {
            stay(Step::Ignored)
        };
    }
    // EV_CONFIGURE = Prepared からのみ。BackEnd::configure の ST_PREPARED ガードにより
    // **それ以外の状態では黙ってスキップされる**(エラーにならない = 一番危ない挙動)。
    if action == "configure" {
        return if from == State::Prepared {
            applied(State::Ready)
        } else {
            stay(Step::Ignored)
        };
    }
    if action == "describe" {
        // 実 SM は Idle からのみ。`Off`(= 我々が付けた「ECC 未初期化」状態。fake の初期値)と
        // `Described` からの撃ち直しは 017 から受けている —— 実機より甘くはならない側なので残す。
        return if matches!(from, State::Off | State::Idle | State::Described) {
            applied(State::Described)
        } else {
            denied()
        };
    }
    if action == "prepare" {
        return if matches!(from, State::Described | State::Prepared) {
            applied(State::Prepared)
        } else {
            denied()
        };
    }
    if action == "start" {
        return if from == State::Ready {
            applied(State::Running)
        } else {
            denied()
        };
    }
    if action == "stop" {
        return if matches!(from, State::Running | State::Paused) {
            applied(State::Ready)
        } else {
            denied()
        };
    }

    Transition {
        step: Step::Denied,
        next: from,
        error: format!("unknown action: {action}"),
    }
}

// ---------------------------------------------------------------------------
// テスト(`tools/ecc_bridge/test_ecc_bridge.cpp` の 457 CHECK の移植。
// 節番号・コメント・期待値は C++ 側と 1 対 1。assert の数を落とさない)
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // 1. DataLinkSet XML(実機の罠を仕様として固定 — SPEC §8.2)
    // -----------------------------------------------------------------------

    /// C++ `test_data_link_set_single_cobo`(1 CHECK)。
    #[test]
    fn data_link_set_single_cobo() {
        // mini eTPC = 1 CoBo。router_port は receiver が実際に bind したポート(§8.2)。
        let links = [Link::new("CoBo[0]", "192.168.10.5", 46005)];

        // 期待値の出典: 流用元 tpcdaq::control::EccController::data_link_set()
        //   (~/test/get/tpcdaq/src/control/ecc_controller.cpp:57-66)を 1 対 1 で写したもの。
        // GET 側の受理形は reference/20190315_patched/GetBench/src/get/rc/DataLink.cpp(tinyxml
        // なので空白は不問)、属性名は同 DataSenderId.cpp / DataRouterId.cpp が正
        //   (name 属性は DataRouterId::fromXml で optional なので出さない)。
        assert_eq!(
            data_link_set_xml(&links),
            "<DataLinkSet><DataLink>\
             <DataSender id=\"CoBo[0]\"/>\
             <DataRouter ipAddress=\"192.168.10.5\" port=\"46005\" type=\"TCP\"/>\
             </DataLink></DataLinkSet>"
        );
    }

    /// C++ `test_data_link_set_two_cobo`(1 CHECK)。
    #[test]
    fn data_link_set_two_cobo() {
        // ELITPC = 2 CoBo。非対称に(IP も port も sender も別)して取り違えを検出する。
        let links = [
            Link::new("CoBo[0]", "192.168.10.5", 46005),
            Link::new("CoBo[1]", "192.168.10.6", 46006),
        ];
        assert_eq!(
            data_link_set_xml(&links),
            "<DataLinkSet>\
             <DataLink><DataSender id=\"CoBo[0]\"/>\
             <DataRouter ipAddress=\"192.168.10.5\" port=\"46005\" type=\"TCP\"/></DataLink>\
             <DataLink><DataSender id=\"CoBo[1]\"/>\
             <DataRouter ipAddress=\"192.168.10.6\" port=\"46006\" type=\"TCP\"/></DataLink>\
             </DataLinkSet>"
        );
    }

    /// C++ `test_data_link_set_escapes_attributes`(1 CHECK)。
    #[test]
    fn data_link_set_escapes_attributes() {
        // 属性値に XML メタ文字が来ても壊れた XML を作らない(黙って壊す方が悪い)。
        let links = [Link::new("CoBo[\"x\"&<0>]", "127.0.0.1", 1)];
        assert_eq!(
            data_link_set_xml(&links),
            "<DataLinkSet><DataLink>\
             <DataSender id=\"CoBo[&quot;x&quot;&amp;&lt;0&gt;]\"/>\
             <DataRouter ipAddress=\"127.0.0.1\" port=\"1\" type=\"TCP\"/>\
             </DataLink></DataLinkSet>"
        );
    }

    /// C++ `test_data_link_set_round_trip`(13 CHECK)。
    #[test]
    fn data_link_set_round_trip() {
        // fake-ECC は start() で XML を読み返して接続先を決める(TODO/017 §2)。
        // 生成 → 読み返しで全フィールドが戻ること = fake の接続先が仕様どおりであること。
        let links = [
            Link::new("CoBo[0]", "10.0.0.1", 46005),
            // instance は数字とは限らない
            Link::new("CoBo[Crate00_Slot01]", "10.0.0.2", 60000),
        ];
        let back = parse_data_link_set(&data_link_set_xml(&links));
        assert_eq!(back.len(), links.len()); // 1
        if back.len() == links.len() {
            for (i, want) in links.iter().enumerate() {
                assert_eq!(back[i].sender, want.sender); // 2,6
                assert_eq!(back[i].router_ip, want.router_ip); // 3,7
                assert_eq!(back[i].router_port, want.router_port); // 4,8
                assert_eq!(back[i].flow_type, want.flow_type); // 5,9
            }
        }
        // エスケープした属性も往復する。
        let odd = [Link::new("CoBo[\"&<0>\"]", "127.0.0.1", 1)];
        let odd_back = parse_data_link_set(&data_link_set_xml(&odd));
        assert_eq!(odd_back.len(), 1); // 10
        if !odd_back.is_empty() {
            assert_eq!(odd_back[0].sender, odd[0].sender); // 11
        }
        // 空 / 壊れた XML でも落ちない(0 本を返す)。
        assert_eq!(parse_data_link_set("").len(), 0); // 12
        assert_eq!(parse_data_link_set("<DataLinkSet></DataLinkSet>").len(), 0);
        // 13
    }

    // -----------------------------------------------------------------------
    // 2. JSON リクエスト parse(SPEC §8.2 のリクエスト形)
    // -----------------------------------------------------------------------

    /// C++ `test_parse_configure_request`(12 CHECK)。
    #[test]
    fn parse_configure_request() {
        // SPEC §8.2 の例そのもの(2 CoBo に拡張、値は全部バラバラ)。
        let json = r#"{"action": "configure", "config_id": "mini-eTPC", "links": [
            {"sender": "CoBo[0]", "router_ip": "192.168.10.5", "router_port": 46005, "type": "TCP"},
            {"sender": "CoBo[1]", "router_ip": "192.168.10.6", "router_port": 46006, "type": "TCP"}]}"#;

        let parsed = parse_request(json);
        assert!(parsed.is_ok(), "{parsed:?}"); // 1 (C++: CHECK(r.valid))
        assert_eq!(parsed.as_ref().err(), None); // 2 (C++: CHECK_STR(r.error, ""))
        let r = parsed.unwrap();
        assert_eq!(r.action, "configure"); // 3
        assert_eq!(r.config_id, "mini-eTPC"); // 4
        assert_eq!(r.links.len(), 2); // 5
        if r.links.len() == 2 {
            assert_eq!(r.links[0].sender, "CoBo[0]"); // 6
            assert_eq!(r.links[0].router_ip, "192.168.10.5"); // 7
            assert_eq!(r.links[0].router_port, 46005); // 8
            assert_eq!(r.links[0].flow_type, "TCP"); // 9
            assert_eq!(r.links[1].sender, "CoBo[1]"); // 10
            assert_eq!(r.links[1].router_ip, "192.168.10.6"); // 11
            assert_eq!(r.links[1].router_port, 46006); // 12
        }
    }

    /// C++ `test_parse_simple_actions`(7 CHECK)。
    #[test]
    fn parse_simple_actions() {
        let r = parse_request(r#"{"action":"status"}"#);
        assert!(r.is_ok(), "{r:?}"); // 1
        let r = r.unwrap();
        assert_eq!(r.action, "status"); // 2
        assert_eq!(r.config_id, "default"); // 3 — 既定 config_id(省略時)
        assert_eq!(r.links.len(), 0); // 4

        let s = parse_request(r#"{"action":"describe","config_id":"elitpc"}"#);
        assert!(s.is_ok(), "{s:?}"); // 5
        let s = s.unwrap();
        assert_eq!(s.action, "describe"); // 6
        assert_eq!(s.config_id, "elitpc"); // 7
    }

    /// C++ `test_parse_rejects_bad_input`(11 ケース × 2 = 22 CHECK)。
    #[test]
    fn parse_rejects_bad_input() {
        // 壊れた JSON / 未知 action / links 欠落 / 小文字 flowType —— **黙って通さない**。
        let cases: [(&str, &str); 11] = [
            ("", "parse error"),
            (r#"{"action":"#, "parse error"),
            ("[1,2,3]", "top-level"),
            (r#"{"config_id":"x"}"#, "missing action"),
            (r#"{"action":"launch"}"#, "unknown action"),
            (r#"{"action":"configure"}"#, "links"),
            (r#"{"action":"configure","links":[]}"#, "links"),
            (
                r#"{"action":"configure","links":[{"router_ip":"1.2.3.4","router_port":1}]}"#,
                "sender",
            ),
            (
                r#"{"action":"configure","links":[{"sender":"CoBo[0]","router_port":1}]}"#,
                "router_ip",
            ),
            (
                r#"{"action":"configure","links":[{"sender":"CoBo[0]","router_ip":"1.2.3.4","router_port":0}]}"#,
                "router_port",
            ),
            // 実機の罠: flowType は大文字 TCP。小文字を**黙って直さない**(直すと本番で沈黙する)。
            (
                r#"{"action":"configure","links":[{"sender":"CoBo[0]","router_ip":"1.2.3.4","router_port":46005,"type":"tcp"}]}"#,
                "TCP",
            ),
        ];
        for (json, want_error_substring) in cases {
            let r = parse_request(json);
            assert!(r.is_err(), "accepted bad input: [{json}]"); // 1
            let error = r.err().unwrap_or_default();
            assert!(
                error.contains(want_error_substring),
                "(input={json} error={error})"
            ); // 2
        }
    }

    /// C++ `test_parse_defaults_flow_type`(3 CHECK)。
    #[test]
    fn parse_defaults_flow_type() {
        // type 省略は "TCP" 既定(既定値であって黙った修正ではない)。
        let r = parse_request(
            r#"{"action":"configure","links":[{"sender":"CoBo[0]","router_ip":"127.0.0.1","router_port":46005}]}"#,
        );
        assert!(r.is_ok(), "{r:?}"); // 1
        let r = r.unwrap();
        assert_eq!(r.links.len(), 1); // 2
        if !r.links.is_empty() {
            assert_eq!(r.links[0].flow_type, "TCP"); // 3
        }
    }

    // -----------------------------------------------------------------------
    // 3. レスポンス生成(SPEC §8.2 v1.14: {"ok", "state", "error", "ecc_error"})
    // -----------------------------------------------------------------------

    /// C++ `test_make_response`(2 CHECK)。
    #[test]
    fn make_response_has_the_four_keys() {
        assert_eq!(
            make_response(true, State::Running, "", Error::NoErr),
            r#"{"ok":true,"state":"Running","error":"","ecc_error":"NO_ERR"}"#
        );
        // ECC 不達 = ok:false + state Unknown(§8.2)。error は JSON エスケープされる。
        // 状態が取れていないので `ecc_error` も Unknown —— 「NO_ERR」と言い切ってはならない。
        assert_eq!(
            make_response(
                false,
                State::Unknown,
                "connect failed: \"refused\"\n",
                Error::Unknown
            ),
            r#"{"ok":false,"state":"Unknown","error":"connect failed: \"refused\"\n","ecc_error":"Unknown"}"#
        );
    }

    /// C++ `test_make_response_carries_ecc_error_beside_state`(2 CHECK)。
    ///
    /// **043**: 041 D-1 の形 —— state は正常(Idle)なのに error フラグだけ立っている。
    /// この 2 軸が独立に運ばれることを全文照合で固定する(片方に丸められたら落ちる)。
    #[test]
    fn make_response_carries_ecc_error_beside_state() {
        assert_eq!(
            make_response(true, State::Idle, "", Error::WhenDescribe),
            r#"{"ok":true,"state":"Idle","error":"","ecc_error":"WHEN_DESCRIBE"}"#
        );
        // 輸送層 error と GET の error フラグは**別物**。両方載る(片方が他方を上書きしない)。
        assert_eq!(
            make_response(
                false,
                State::Ready,
                "Could not establish data link. refused",
                Error::WhenStart
            ),
            r#"{"ok":false,"state":"Ready","error":"Could not establish data link. refused","ecc_error":"WHEN_START"}"#
        );
    }

    // -----------------------------------------------------------------------
    // 3b. error フラグの文字列(SM.h:58-70 の 10 値 + 我々の Unknown)
    //
    // 綴りは **SM.h / rc/SM.cpp:73-108 の operator<<** と同じ UPPER_SNAKE(SPEC §8.2 v1.14)。
    // オペレータが ECC のログと突き合わせる文字列なので、Ice 側の綴り(NoErr, WhenDescribe)
    // には寄せない。順序は SM.h の enum 定義順そのまま(NO_ERR=0 .. WHEN_RESET=9)。
    // -----------------------------------------------------------------------

    /// C++ `test_error_strings`(11 CHECK)。
    #[test]
    fn error_strings() {
        let all = [
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
            Error::Unknown,
        ];
        let names = [
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
            "Unknown",
        ];
        for i in 0..11 {
            assert_eq!(all[i].as_str(), names[i]);
        }
    }

    // -----------------------------------------------------------------------
    // 4. 状態文字列マップ
    // -----------------------------------------------------------------------

    /// C++ `test_state_strings`(18 CHECK)。
    #[test]
    fn state_strings() {
        let all = [
            State::Off,
            State::Idle,
            State::Described,
            State::Prepared,
            State::Ready,
            State::Running,
            State::Paused,
            State::Unknown,
        ];
        let names = [
            "Off",
            "Idle",
            "Described",
            "Prepared",
            "Ready",
            "Running",
            "Paused",
            "Unknown",
        ];
        for i in 0..8 {
            assert_eq!(all[i].as_str(), names[i]); // 1..8
            assert_eq!(State::from_name(names[i]), all[i]); // 9..16
        }
        // 知らない綴りは Unknown に落とす(エラーにしない)。
        assert_eq!(State::from_name("running"), State::Unknown); // 17
        assert_eq!(State::from_name(""), State::Unknown); // 18
    }

    // -----------------------------------------------------------------------
    // 5. fake-ECC の状態遷移表 = **実 ECC の SM**(036)
    //
    // 出典は reference/20190315_patched:
    //   GetBench/src/get/rc/BackEnd.cpp:250-270 / :924 / :955-962、
    //   StateMachine/src/dhsm/Engine.cpp:334-352。
    // -----------------------------------------------------------------------

    /// C++ `test_state_machine_happy_path`(18 CHECK)。
    #[test]
    fn state_machine_happy_path() {
        let mut s = State::Off;
        // 一周: Off →(describe/prepare/configure/start)→ Running →(stop)→ Ready →
        //       **歩き戻し breakup → reset → reset** → Idle。
        // `reset` が 2 回要るのが 036 の眼目 —— `EV_UNDO` は 1 段しか戻さない。
        let steps = [
            ("describe", State::Described),
            ("prepare", State::Prepared),
            ("configure", State::Ready),
            ("start", State::Running),
            ("stop", State::Ready),
            ("breakup", State::Prepared),
            ("reset", State::Described),
            ("reset", State::Idle),
        ];
        for (action, want) in steps {
            let t = next_state(s, action);
            assert_eq!(
                t.step,
                Step::Applied,
                "(action={action} from={s} err={})",
                t.error
            );
            assert_eq!(t.next, want);
            s = t.next;
        }
        // status は状態を変えない(いつでも可)。
        let t = next_state(State::Running, "status");
        assert_eq!(t.step, Step::Applied); // 17
        assert_eq!(t.next, State::Running); // 18
    }

    /// C++ `test_reset_walks_back_one_step_at_a_time`(31 CHECK)。
    ///
    /// **036 の核心**: `reset`(= EV_UNDO)は 1 段戻すだけで、`Active`(Ready/Running/Paused)
    /// からは遷移が無く**無音で無視**される。`Ready → Idle` には breakup → reset → reset が要る。
    #[test]
    fn reset_walks_back_one_step_at_a_time() {
        // EV_UNDO の 2 本(BackEnd.cpp:250-270)。
        let t = next_state(State::Prepared, "reset");
        assert_eq!(t.step, Step::Applied); // 1
        assert_eq!(t.next, State::Described); // 2
        let t = next_state(State::Described, "reset");
        assert_eq!(t.step, Step::Applied); // 3
        assert_eq!(t.next, State::Idle); // 4

        // Active からは**遷移が無い** = Ignored(例外も出ない・状態も動かない)。
        for active in [State::Ready, State::Running, State::Paused] {
            let t = next_state(active, "reset");
            assert_eq!(t.step, Step::Ignored); // 5,8,11
            assert_eq!(t.next, active); // 6,9,12
            assert_eq!(t.error, "", "**無音**: エラー文言すら出ない"); // 7,10,13
        }
        // 底(Off / Idle)でも同じく無音(これ以上戻れない)。
        let t = next_state(State::Idle, "reset");
        assert_eq!(t.step, Step::Ignored); // 14
        assert_eq!(t.next, State::Idle); // 15
        let t = next_state(State::Off, "reset");
        assert_eq!(t.step, Step::Ignored); // 16
        assert_eq!(t.next, State::Off); // 17

        // EV_BREAK は Active のどこからでも Prepared へ(ST_ACTIVE は複合状態)。
        for active in [State::Ready, State::Running, State::Paused] {
            let t = next_state(active, "breakup");
            assert_eq!(t.step, Step::Applied); // 18,20,22
            assert_eq!(t.next, State::Prepared); // 19,21,23
        }
        let t = next_state(State::Described, "breakup");
        assert_eq!(t.step, Step::Ignored); // 24
        assert_eq!(t.next, State::Described); // 25

        // `Ready → Idle` の歩き戻し 3 手を通しで(controller が実装する列そのもの)。
        let mut s = State::Ready;
        let walk = ["breakup", "reset", "reset"];
        let want = [State::Prepared, State::Described, State::Idle];
        for i in 0..3 {
            let t = next_state(s, walk[i]);
            assert_eq!(t.step, Step::Applied); // 26,28,30
            assert_eq!(t.next, want[i]); // 27,29,31
            s = t.next;
        }
    }

    /// C++ `test_configure_is_silently_skipped_outside_prepared`(20 CHECK)。
    ///
    /// **036**: `configure` は `ST_PREPARED` ガード付き(BackEnd.cpp:955-962)。
    /// Prepared 以外では**黙ってスキップ**される —— エラーにならないのが一番危ないところ。
    #[test]
    fn configure_is_silently_skipped_outside_prepared() {
        let t = next_state(State::Prepared, "configure");
        assert_eq!(t.step, Step::Applied); // 1
        assert_eq!(t.next, State::Ready); // 2

        for from in [
            State::Off,
            State::Idle,
            State::Described,
            State::Ready,
            State::Running,
            State::Paused,
        ] {
            let t = next_state(from, "configure");
            assert_eq!(t.step, Step::Ignored);
            // DataLinkSet は張り替わらない = CoBo は古い宛先のまま
            assert_eq!(t.next, from);
            assert_eq!(t.error, "");
        }
    }

    /// C++ `test_state_machine_rejects_out_of_order`(11 CHECK)。
    #[test]
    fn state_machine_rejects_out_of_order() {
        // listen 以前の問題: describe していないのに start
        let t = next_state(State::Off, "start");
        assert_eq!(t.step, Step::Denied); // 1
        assert!(t.error.contains("start")); // 2
        assert!(t.error.contains("Off")); // 3

        assert_eq!(next_state(State::Described, "start").step, Step::Denied); // 4
        assert_eq!(next_state(State::Ready, "prepare").step, Step::Denied); // 5
                                                                            // 二重 start
        assert_eq!(next_state(State::Running, "start").step, Step::Denied); // 6
                                                                            // 走っていない
        assert_eq!(next_state(State::Ready, "stop").step, Step::Denied); // 7
                                                                         // 未知 action
        assert_eq!(next_state(State::Off, "chirp").step, Step::Denied); // 8

        // **034 の事故の再現**: 前の run の `ecc stop` 後(= Ready)に reset 1 発では Idle に
        // 届かず(Ignored)、続く describe は順序違反になる。この 2 行が「歩き戻しを省いた
        // 実装は必ず落ちる」ことの保証である。
        let t = next_state(State::Ready, "reset");
        assert_eq!(t.step, Step::Ignored); // 9
        assert_eq!(t.next, State::Ready); // 10
        assert_eq!(next_state(State::Ready, "describe").step, Step::Denied); // 11
    }

    // -----------------------------------------------------------------------
    // 6. 共有 golden 遷移表とのパリティ(049)
    //
    // ECC の状態機械は **意図的に 2 実装**ある(036 の事故に由来する負荷分散された防御。
    // 044 で「統一しない」と裁定済み)。残るリスクは**将来の片側だけの修正によるドリフト**
    // なので、`tests/fixtures/ecc_transitions.txt` の golden 表を **controller 側
    // (src/controller.rs の同名テスト)と同じ 1 ファイルから**読み、全 64 行を照合する。
    //
    // 表の各列の意味はファイル冒頭のヘッダに書いてある。**表と実装が食い違ったら、表を直す前に
    // 一次資料(reference/20190315_patched/GetBench/src/get/rc/BackEnd.cpp)を見ること** ——
    // 意味論の正本はこのファイル上部の 2 つのコメントブロック(048-C)であって、表ではない。
    // -----------------------------------------------------------------------

    /// C++ `test_golden_transition_table`(64 行 × 4 + 行数 = 257 CHECK)。
    #[test]
    fn golden_transition_table() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/ecc_transitions.txt");
        // **黙って skip しない**(CLAUDE.md: silent failure を作らない)。表が読めない =
        // パリティを検証できていない、であって「異常なし」ではない。
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot open golden table [{}]: {e}", path.display()));

        let mut rows = 0;
        for (index, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let cols: Vec<&str> = line.split_whitespace().collect();
            let at = format!("{}:{}", path.display(), index + 1);
            assert_eq!(cols.len(), 4, "{at}: 4 列でない: {raw}");
            let (state, action, outcome, flag) = (cols[0], cols[1], cols[2], cols[3]);
            let at = format!("{at} [{state} {action}]");

            // from_name は知らない綴りを黙って Unknown に落とすので、往復で綴りを確かめる
            // (表の typo が「Unknown 行が 1 本増えた」に化けない)。
            let from = State::from_name(state);
            assert_eq!(state, from.as_str(), "{at}: 未知の state");

            let (want_step, want_next, want_err_empty) = match outcome {
                "Ignored" => (Step::Ignored, from, true),
                // Denied だけは理由が出る(実機より辛くしてある 1 点)
                "Denied" => (Step::Denied, from, false),
                // status は next_state では Applied を返すが state は動かない
                // (観測であって遷移でない)。
                "Observed" => (Step::Applied, from, true),
                other => {
                    let next_name = other
                        .strip_prefix("Applied:")
                        .unwrap_or_else(|| panic!("{at}: 未知の outcome [{other}]"));
                    let next = State::from_name(next_name);
                    assert_eq!(next_name, next.as_str(), "{at}: 未知の次状態");
                    (Step::Applied, next, true)
                }
            };
            assert!(flag == "clear" || flag == "keep", "{at}: 未知の error_flag");

            let t = next_state(from, action);

            // error フラグが消えるのは fake_ecc::step() が Applied を受けたときだけ
            // (fake_ecc.cpp:236-240)。`status` は getStatus に降りて step() を通らないので
            // 触られない(fake_ecc.cpp:202-208)。
            let clears = action != "status" && t.step == Step::Applied;

            assert_eq!(t.step, want_step, "{at}: step"); // 1
            assert_eq!(t.next, want_next, "{at}: 次状態"); // 2
            assert_eq!(
                t.error.is_empty(),
                want_err_empty,
                "{at}: 理由 [{}]",
                t.error
            ); // 3
            assert_eq!(clears, flag == "clear", "{at}: error フラグ"); // 4
            rows += 1;
        }
        // 表が空 / 半分しか読めていないのに green になるのを防ぐ(8 state × 8 action)。
        assert_eq!(rows, 64);
    }

    // -----------------------------------------------------------------------
    // 7. JSON の土台(C++ は自前 json_min 273 行、Rust は serde_json)
    //
    // C++ `test_json_min` の 27 CHECK をそのまま移す。**土台が替わっても同じ判定になる**
    // ことを確かめるためのもので、serde_json を疑っているのではなく「parse_request が
    // 立っている地面」の性質(数・真偽・null・エスケープ・壊れた入力の拒否・深さ上限)を
    // 仕様として固定する。
    // -----------------------------------------------------------------------

    /// C++ `test_json_min`(27 CHECK)。
    #[test]
    fn the_json_substrate_behaves_like_json_min() {
        let parsed: Result<Value, _> = serde_json::from_str(
            r#"{"a": "x\ty", "b": [1, -2.5e1, true, false, null], "c": {"d": 7}}"#,
        );
        assert!(parsed.is_ok(), "{parsed:?}"); // 1
        let v = parsed.unwrap_or(Value::Null);
        assert!(v.is_object()); // 2
        assert_eq!(v.get("a").and_then(Value::as_str), Some("x\ty")); // 3
        let b = v.get("b").and_then(Value::as_array);
        assert_eq!(b.map(Vec::len), Some(5)); // 4
        if let Some(b) = b {
            assert_eq!(b[0].as_i64(), Some(1)); // 5
                                                // C++ json_min の `as_int()` は -2.5e1 を -25 にする。serde_json は f64 のまま
                                                // 持つので、ここは f64 で照合する(値は同じ -25)。
            assert_eq!(b[1].as_f64(), Some(-25.0)); // 6
            assert_eq!(b[2].as_bool(), Some(true)); // 7
            assert_eq!(b[3].as_bool(), Some(false)); // 8
            assert!(b[4].is_null()); // 9
        }
        assert_eq!(
            v.get("c").and_then(|c| c.get("d")).and_then(Value::as_i64),
            Some(7)
        ); // 10

        // 壊れた入力は Err + 理由(panic しない)。
        let bad = [
            "",
            "{",
            r#"{"a"}"#,
            r#"{"a":}"#,
            "[1,]",
            "tru",
            "\"unterminated",
            r#"{"a":1}x"#,
        ];
        for s in bad {
            let parsed: Result<Value, _> = serde_json::from_str(s);
            assert!(parsed.is_err(), "accepted bad input: [{s}]"); // 11,13,..25
            assert!(!parsed
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default()
                .is_empty()); // 12,14,..26
        }
        // 深すぎるネストで stack を焼かない(深さ上限で弾く)。
        let deep = "[".repeat(200);
        let parsed: Result<Value, _> = serde_json::from_str(&deep);
        assert!(parsed.is_err()); // 27
    }
}
