// ecc_core.hpp — ecc-bridge の **Ice 非依存**な核(SPEC §8.2)。
//
//   * State と文字列の対応(Off/Idle/Described/Prepared/Ready/Running/Paused/Unknown)
//   * DataLinkSet XML の生成(**実機の罠を仕様として固定**: DataSender id = "CoBo[k]" 形式、
//     flowType = 大文字 "TCP")
//   * JSON リクエストの parse とレスポンスの生成(never throw)
//   * fake-ECC 用の状態遷移表(実 ECC の SM を模す。bridge 本体は使わない)
//
// Ice も ZMQ も include しない —— ここだけで `make test` が回る(TODO/017 §テスト)。

#ifndef TPCDAQ_ECC_BRIDGE_ECC_CORE_HPP
#define TPCDAQ_ECC_BRIDGE_ECC_CORE_HPP

#include "json_min.hpp"

#include <cstddef>
#include <cstdlib>
#include <string>
#include <utility>
#include <vector>

namespace ecc {

// get::rc::State のミラー + Unknown(取得失敗 / ECC 不達)。
// 流用元: ~/test/get/tpcdaq/include/tpcdaq/control/ecc_controller.hpp の enum。
enum class State { Off, Idle, Described, Prepared, Ready, Running, Paused, Unknown };

inline const char* to_string(State s) {
  switch (s) {
    case State::Off: return "Off";
    case State::Idle: return "Idle";
    case State::Described: return "Described";
    case State::Prepared: return "Prepared";
    case State::Ready: return "Ready";
    case State::Running: return "Running";
    case State::Paused: return "Paused";
    default: return "Unknown";
  }
}

inline State state_from_string(const std::string& s) {
  if (s == "Off") return State::Off;
  if (s == "Idle") return State::Idle;
  if (s == "Described") return State::Described;
  if (s == "Prepared") return State::Prepared;
  if (s == "Ready") return State::Ready;
  if (s == "Running") return State::Running;
  if (s == "Paused") return State::Paused;
  return State::Unknown;
}

// DataLinkSet の 1 本。CoBo 毎に 1 本(SPEC §8.2)。
struct Link {
  std::string sender;      // ECC の NodeId 表記そのまま。実機は "CoBo[0]"(`.AsAd[..]` は付かない)
  std::string router_ip;   // 我々(receiver)の受信アドレス
  int router_port = 0;     // receiver が**実際に bind した**ポート(controller が Arm 応答から渡す)
  std::string flow_type = "TCP";  // 大文字必須(sender factory が case-sensitive)
};

// XML 属性値のエスケープ。値は実質 ASCII だが、壊れた XML を黙って作らない。
inline std::string xml_escape(const std::string& s) {
  std::string out;
  out.reserve(s.size());
  for (const char c : s) {
    switch (c) {
      case '&': out += "&amp;"; break;
      case '<': out += "&lt;"; break;
      case '>': out += "&gt;"; break;
      case '"': out += "&quot;"; break;
      default: out += c;
    }
  }
  return out;
}

// configure() に注入する DataLinkSet XML。
// 流用元: tpcdaq::control::EccController::data_link_set()
//   (~/test/get/tpcdaq/src/control/ecc_controller.cpp:57-66)を N リンクへ拡張したもの。
// 受理側の正は reference/20190315_patched/GetBench/src/get/rc/{DataLinkSet,DataLink,
//   DataSenderId,DataRouterId}.cpp(tinyxml パースなので空白・属性順は不問。
//   DataRouter の name 属性は fromXml で optional なので出さない)。
inline std::string data_link_set_xml(const std::vector<Link>& links) {
  std::string xml = "<DataLinkSet>";
  for (const Link& l : links) {
    xml += "<DataLink><DataSender id=\"";
    xml += xml_escape(l.sender);
    xml += "\"/><DataRouter ipAddress=\"";
    xml += xml_escape(l.router_ip);
    xml += "\" port=\"";
    xml += std::to_string(l.router_port);
    xml += "\" type=\"";
    xml += xml_escape(l.flow_type);
    xml += "\"/></DataLink>";
  }
  xml += "</DataLinkSet>";
  return xml;
}

// xml_escape の逆。属性値に現れうる 5 実体だけ戻す(数値参照は使わないので見ない)。
inline std::string xml_unescape(const std::string& s) {
  static const std::pair<const char*, char> kEntities[] = {
      {"&quot;", '"'}, {"&apos;", '\''}, {"&lt;", '<'}, {"&gt;", '>'}, {"&amp;", '&'}};
  std::string out;
  out.reserve(s.size());
  for (std::size_t i = 0; i < s.size();) {
    bool matched = false;
    if (s[i] == '&') {
      for (const auto& e : kEntities) {
        const std::size_t n = std::char_traits<char>::length(e.first);
        if (s.compare(i, n, e.first) == 0) {
          out += e.second;
          i += n;
          matched = true;
          break;
        }
      }
    }
    if (!matched) out += s[i++];
  }
  return out;
}

// DataLinkSet XML を Link 配列に戻す(**fake-ECC 専用**の最小読み取り。
// 実 ECC は tinyxml で読む —— こちらは自分が作った XML を読み返すだけなので属性走査で足りる)。
// 壊れた入力では取れたぶんだけ返す(fake の中で例外を投げない)。
inline std::vector<Link> parse_data_link_set(const std::string& xml) {
  auto attr = [&xml](std::size_t from, std::size_t to, const char* name) -> std::string {
    const std::string key = std::string(name) + "=\"";
    const std::size_t p = xml.find(key, from);
    if (p == std::string::npos || p >= to) return std::string();
    const std::size_t b = p + key.size();
    const std::size_t e = xml.find('"', b);
    if (e == std::string::npos || e > to) return std::string();
    return xml_unescape(xml.substr(b, e - b));
  };

  std::vector<Link> links;
  std::size_t pos = 0;
  for (;;) {
    const std::size_t begin = xml.find("<DataLink>", pos);
    if (begin == std::string::npos) break;
    std::size_t end = xml.find("</DataLink>", begin);
    if (end == std::string::npos) end = xml.size();
    Link l;
    l.sender = attr(begin, end, "id");
    l.router_ip = attr(begin, end, "ipAddress");
    const std::string port = attr(begin, end, "port");
    l.router_port = port.empty() ? 0 : static_cast<int>(std::strtol(port.c_str(), nullptr, 10));
    l.flow_type = attr(begin, end, "type");
    links.push_back(l);
    pos = end + 1;
  }
  return links;
}

// ---------------------------------------------------------------------------
// JSON リクエスト / レスポンス(SPEC §8.2)
// ---------------------------------------------------------------------------

// 受け付ける action(SPEC §8.2 の列挙そのまま)。pause/resume は §8.2 に無いので受けない。
inline bool is_known_action(const std::string& a) {
  return a == "describe" || a == "prepare" || a == "configure" || a == "start" || a == "stop" ||
         a == "breakup" || a == "reset" || a == "status";
}

struct Request {
  bool valid = false;
  std::string error;  // valid=false のときの理由(そのままレスポンスの error になる)
  std::string action;
  std::string config_id = "default";
  std::vector<Link> links;  // configure のみ
};

inline Request parse_request(const std::string& json) {
  Request r;
  jsonmin::Value v;
  std::string err;
  if (!jsonmin::parse(json, v, err)) {
    r.error = err;
    return r;
  }
  if (v.type != jsonmin::Value::Type::Object) {
    r.error = "top-level JSON value must be an object";
    return r;
  }

  const jsonmin::Value* action = v.find("action");
  if (action == nullptr || action->type != jsonmin::Value::Type::String || action->str.empty()) {
    r.error = "missing action";
    return r;
  }
  if (!is_known_action(action->str)) {
    r.error = "unknown action: " + action->str;
    return r;
  }
  r.action = action->str;

  const jsonmin::Value* cfg = v.find("config_id");
  if (cfg != nullptr) {
    if (cfg->type != jsonmin::Value::Type::String || cfg->str.empty()) {
      r.error = "config_id must be a non-empty string";
      return r;
    }
    r.config_id = cfg->str;
  }

  const jsonmin::Value* links = v.find("links");
  if (links != nullptr) {
    if (links->type != jsonmin::Value::Type::Array) {
      r.error = "links must be an array";
      return r;
    }
    for (const jsonmin::Value& item : links->arr) {
      if (item.type != jsonmin::Value::Type::Object) {
        r.error = "each entry of links must be an object";
        return r;
      }
      Link l;
      const jsonmin::Value* sender = item.find("sender");
      if (sender == nullptr || sender->type != jsonmin::Value::Type::String ||
          sender->str.empty()) {
        r.error = "link needs a non-empty sender (e.g. \"CoBo[0]\")";
        return r;
      }
      l.sender = sender->str;

      const jsonmin::Value* ip = item.find("router_ip");
      if (ip == nullptr || ip->type != jsonmin::Value::Type::String || ip->str.empty()) {
        r.error = "link needs a non-empty router_ip";
        return r;
      }
      l.router_ip = ip->str;

      const jsonmin::Value* port = item.find("router_port");
      if (port == nullptr || port->type != jsonmin::Value::Type::Number) {
        r.error = "link needs a numeric router_port";
        return r;
      }
      const long long p = port->as_int();
      if (p < 1 || p > 65535) {
        r.error = "router_port out of range (1..65535): " + std::to_string(p);
        return r;
      }
      l.router_port = static_cast<int>(p);

      const jsonmin::Value* type = item.find("type");
      if (type != nullptr) {
        if (type->type != jsonmin::Value::Type::String) {
          r.error = "link type must be a string";
          return r;
        }
        // 実機の罠(SPEC §8.2): flowType は大文字 "TCP"。小文字を**黙って直さない** ——
        // 直してしまうと「設定が間違っていた」事実が消え、本番で同じ罠を踏み直す。
        if (type->str != "TCP") {
          r.error = "link type must be upper-case \"TCP\", got \"" + type->str + "\"";
          return r;
        }
        l.flow_type = type->str;
      }
      r.links.push_back(l);
    }
  }

  if (r.action == "configure" && r.links.empty()) {
    r.error = "configure needs a non-empty links array";
    return r;
  }

  r.valid = true;
  return r;
}

// レスポンス(SPEC §8.2)。3 キーを**常に**出す(Rust 側の契約を単純にする)。
inline std::string make_response(bool ok, State s, const std::string& error) {
  std::string out = "{\"ok\":";
  out += ok ? "true" : "false";
  out += ",\"state\":\"";
  out += to_string(s);
  out += "\",\"error\":\"";
  out += jsonmin::escape(error);
  out += "\"}";
  return out;
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

// 1 遷移の結末。実 ECC の dhsm::Engine::ProcessingResult に対応する
// (Applied = crossTransition した / Ignored = 遷移が無く**無音で**何も起きない)。
// Denied は本テストダブル固有の厳しさ(上記「実機より辛くしてある 1 点」)。
enum class Step { Applied, Ignored, Denied };

inline Step next_state(State from, const std::string& action, State& to, std::string& error) {
  to = from;  // 既定は「動かない」。Applied のときだけ書き換える。
  error.clear();
  // ST_ACTIVE(複合状態)に居るか。EV_UNDO はここからは定義されていない。
  const bool active =
      (from == State::Ready || from == State::Running || from == State::Paused);

  auto applied = [&to](State next) {
    to = next;
    return Step::Applied;
  };
  auto denied = [&]() {
    error = "invalid transition: " + action + " in state " + to_string(from);
    return Step::Denied;
  };

  // status は観測であって遷移ではない(状態を変えない)。
  if (action == "status") return Step::Applied;

  // EV_UNDO = **1 段戻すだけ**。Active からは遷移が無いので無音で無視される。
  if (action == "reset") {
    if (from == State::Described) return applied(State::Idle);
    if (from == State::Prepared) return applied(State::Described);
    return Step::Ignored;
  }
  // EV_BREAK = Active(Ready/Running/Paused)→ Prepared。それ以外からは無音。
  if (action == "breakup") return active ? applied(State::Prepared) : Step::Ignored;
  // EV_CONFIGURE = Prepared からのみ。BackEnd::configure の ST_PREPARED ガードにより
  // **それ以外の状態では黙ってスキップされる**(エラーにならない = 一番危ない挙動)。
  if (action == "configure") {
    return (from == State::Prepared) ? applied(State::Ready) : Step::Ignored;
  }

  if (action == "describe") {
    // 実 SM は Idle からのみ。`Off`(= 我々が付けた「ECC 未初期化」状態。fake の初期値)と
    // `Described` からの撃ち直しは 017 から受けている —— 実機より甘くはならない側なので残す。
    return (from == State::Off || from == State::Idle || from == State::Described)
               ? applied(State::Described)
               : denied();
  }
  if (action == "prepare") {
    return (from == State::Described || from == State::Prepared) ? applied(State::Prepared)
                                                                 : denied();
  }
  if (action == "start") return (from == State::Ready) ? applied(State::Running) : denied();
  if (action == "stop") {
    return (from == State::Running || from == State::Paused) ? applied(State::Ready) : denied();
  }

  error = "unknown action: " + action;
  return Step::Denied;
}

}  // namespace ecc

#endif  // TPCDAQ_ECC_BRIDGE_ECC_CORE_HPP
