// test_ecc_bridge.cpp — ecc-bridge の **Ice 非依存部分**の単体テスト(TODO/017 §テスト)。
//
//   * DataLinkSet XML 生成の**文字列全文照合**(CoBo[0] 形式 / 大文字 TCP / 2 CoBo = DataLink 2 本)
//   * JSON リクエスト parse とレスポンス生成(never throw — 壊れた入力も Result 化)
//   * 状態文字列マップ(Off..Paused/Unknown)
//   * fake-ECC の状態遷移表 = **実 ECC の SM**(036): `reset` は 1 段戻すだけ・`Active` からは
//     無音、`configure` は Prepared 以外で無音スキップ、`breakup` は Active→Prepared、
//     順序違反(describe/prepare/start/stop)はエラー
//
// ここには Ice も ZMQ も要らない(`make test` は g++ 一発)。Ice を跨ぐ検証は run_ecc_e2e.sh。

#include "check.hpp"
#include "ecc_core.hpp"
#include "json_min.hpp"

#include <string>
#include <vector>

using ecc::Link;
using ecc::State;

namespace {

// ---------------------------------------------------------------------------
// 1. DataLinkSet XML(実機の罠を仕様として固定 — SPEC §8.2)
// ---------------------------------------------------------------------------

void test_data_link_set_single_cobo() {
  // mini eTPC = 1 CoBo。router_port は receiver が実際に bind したポート(§8.2)。
  std::vector<Link> links = {Link{"CoBo[0]", "192.168.10.5", 46005, "TCP"}};

  // 期待値の出典: 流用元 tpcdaq::control::EccController::data_link_set()
  //   (~/test/get/tpcdaq/src/control/ecc_controller.cpp:57-66)を 1 対 1 で写したもの。
  // GET 側の受理形は reference/20190315_patched/GetBench/src/get/rc/DataLink.cpp(tinyxml
  // なので空白は不問)、属性名は同 DataSenderId.cpp / DataRouterId.cpp が正
  //   (name 属性は DataRouterId::fromXml で optional なので出さない)。
  CHECK_STR(ecc::data_link_set_xml(links),
            "<DataLinkSet><DataLink>"
            "<DataSender id=\"CoBo[0]\"/>"
            "<DataRouter ipAddress=\"192.168.10.5\" port=\"46005\" type=\"TCP\"/>"
            "</DataLink></DataLinkSet>");
}

void test_data_link_set_two_cobo() {
  // ELITPC = 2 CoBo。非対称に(IP も port も sender も別)して取り違えを検出する。
  std::vector<Link> links = {
      Link{"CoBo[0]", "192.168.10.5", 46005, "TCP"},
      Link{"CoBo[1]", "192.168.10.6", 46006, "TCP"},
  };
  CHECK_STR(ecc::data_link_set_xml(links),
            "<DataLinkSet>"
            "<DataLink><DataSender id=\"CoBo[0]\"/>"
            "<DataRouter ipAddress=\"192.168.10.5\" port=\"46005\" type=\"TCP\"/></DataLink>"
            "<DataLink><DataSender id=\"CoBo[1]\"/>"
            "<DataRouter ipAddress=\"192.168.10.6\" port=\"46006\" type=\"TCP\"/></DataLink>"
            "</DataLinkSet>");
}

void test_data_link_set_escapes_attributes() {
  // 属性値に XML メタ文字が来ても壊れた XML を作らない(黙って壊す方が悪い)。
  std::vector<Link> links = {Link{"CoBo[\"x\"&<0>]", "127.0.0.1", 1, "TCP"}};
  CHECK_STR(ecc::data_link_set_xml(links),
            "<DataLinkSet><DataLink>"
            "<DataSender id=\"CoBo[&quot;x&quot;&amp;&lt;0&gt;]\"/>"
            "<DataRouter ipAddress=\"127.0.0.1\" port=\"1\" type=\"TCP\"/>"
            "</DataLink></DataLinkSet>");
}

void test_data_link_set_round_trip() {
  // fake-ECC は start() で XML を読み返して接続先を決める(TODO/017 §2)。
  // 生成 → 読み返しで全フィールドが戻ること = fake の接続先が仕様どおりであること。
  std::vector<Link> links = {
      Link{"CoBo[0]", "10.0.0.1", 46005, "TCP"},
      Link{"CoBo[Crate00_Slot01]", "10.0.0.2", 60000, "TCP"},  // instance は数字とは限らない
  };
  const std::vector<Link> back = ecc::parse_data_link_set(ecc::data_link_set_xml(links));
  CHECK_EQ(back.size(), links.size());
  if (back.size() == links.size()) {
    for (std::size_t i = 0; i < links.size(); ++i) {
      CHECK_STR(back[i].sender, links[i].sender);
      CHECK_STR(back[i].router_ip, links[i].router_ip);
      CHECK_EQ(back[i].router_port, links[i].router_port);
      CHECK_STR(back[i].flow_type, links[i].flow_type);
    }
  }
  // エスケープした属性も往復する。
  std::vector<Link> odd = {Link{"CoBo[\"&<0>\"]", "127.0.0.1", 1, "TCP"}};
  const std::vector<Link> odd_back = ecc::parse_data_link_set(ecc::data_link_set_xml(odd));
  CHECK_EQ(odd_back.size(), 1);
  if (!odd_back.empty()) CHECK_STR(odd_back[0].sender, odd[0].sender);
  // 空 / 壊れた XML でも落ちない(0 本を返す)。
  CHECK_EQ(ecc::parse_data_link_set("").size(), 0);
  CHECK_EQ(ecc::parse_data_link_set("<DataLinkSet></DataLinkSet>").size(), 0);
}

// ---------------------------------------------------------------------------
// 2. JSON リクエスト parse(SPEC §8.2 のリクエスト形)
// ---------------------------------------------------------------------------

void test_parse_configure_request() {
  // SPEC §8.2 の例そのもの(2 CoBo に拡張、値は全部バラバラ)。
  const std::string json =
      "{\"action\": \"configure\", \"config_id\": \"mini-eTPC\", \"links\": ["
      "{\"sender\": \"CoBo[0]\", \"router_ip\": \"192.168.10.5\", \"router_port\": 46005,"
      " \"type\": \"TCP\"},"
      "{\"sender\": \"CoBo[1]\", \"router_ip\": \"192.168.10.6\", \"router_port\": 46006,"
      " \"type\": \"TCP\"}]}";

  ecc::Request r = ecc::parse_request(json);
  CHECK(r.valid);
  CHECK_STR(r.error, "");
  CHECK_STR(r.action, "configure");
  CHECK_STR(r.config_id, "mini-eTPC");
  CHECK_EQ(r.links.size(), 2);
  if (r.links.size() == 2) {
    CHECK_STR(r.links[0].sender, "CoBo[0]");
    CHECK_STR(r.links[0].router_ip, "192.168.10.5");
    CHECK_EQ(r.links[0].router_port, 46005);
    CHECK_STR(r.links[0].flow_type, "TCP");
    CHECK_STR(r.links[1].sender, "CoBo[1]");
    CHECK_STR(r.links[1].router_ip, "192.168.10.6");
    CHECK_EQ(r.links[1].router_port, 46006);
  }
}

void test_parse_simple_actions() {
  ecc::Request r = ecc::parse_request("{\"action\":\"status\"}");
  CHECK(r.valid);
  CHECK_STR(r.action, "status");
  CHECK_STR(r.config_id, "default");  // 既定 config_id(省略時)
  CHECK_EQ(r.links.size(), 0);

  ecc::Request s = ecc::parse_request("{\"action\":\"describe\",\"config_id\":\"elitpc\"}");
  CHECK(s.valid);
  CHECK_STR(s.action, "describe");
  CHECK_STR(s.config_id, "elitpc");
}

void test_parse_rejects_bad_input() {
  // 壊れた JSON / 未知 action / links 欠落 / 小文字 flowType —— **黙って通さない**。
  const struct {
    const char* json;
    const char* want_error_substring;
  } cases[] = {
      {"", "parse error"},
      {"{\"action\":", "parse error"},
      {"[1,2,3]", "top-level"},
      {"{\"config_id\":\"x\"}", "missing action"},
      {"{\"action\":\"launch\"}", "unknown action"},
      {"{\"action\":\"configure\"}", "links"},
      {"{\"action\":\"configure\",\"links\":[]}", "links"},
      {"{\"action\":\"configure\",\"links\":[{\"router_ip\":\"1.2.3.4\",\"router_port\":1}]}",
       "sender"},
      {"{\"action\":\"configure\",\"links\":[{\"sender\":\"CoBo[0]\",\"router_port\":1}]}",
       "router_ip"},
      {"{\"action\":\"configure\",\"links\":[{\"sender\":\"CoBo[0]\",\"router_ip\":\"1.2.3.4\","
       "\"router_port\":0}]}",
       "router_port"},
      // 実機の罠: flowType は大文字 TCP。小文字を**黙って直さない**(直すと本番で沈黙する)。
      {"{\"action\":\"configure\",\"links\":[{\"sender\":\"CoBo[0]\",\"router_ip\":\"1.2.3.4\","
       "\"router_port\":46005,\"type\":\"tcp\"}]}",
       "TCP"},
  };
  for (const auto& c : cases) {
    ecc::Request r = ecc::parse_request(c.json);
    CHECK(!r.valid);
    const bool has = r.error.find(c.want_error_substring) != std::string::npos;
    if (!has) std::printf("  (input=%s error=%s)\n", c.json, r.error.c_str());
    CHECK(has);
  }
}

void test_parse_defaults_flow_type() {
  // type 省略は "TCP" 既定(既定値であって黙った修正ではない)。
  ecc::Request r = ecc::parse_request(
      "{\"action\":\"configure\",\"links\":[{\"sender\":\"CoBo[0]\",\"router_ip\":\"127.0.0.1\","
      "\"router_port\":46005}]}");
  CHECK(r.valid);
  CHECK_EQ(r.links.size(), 1);
  if (!r.links.empty()) CHECK_STR(r.links[0].flow_type, "TCP");
}

// ---------------------------------------------------------------------------
// 3. レスポンス生成(SPEC §8.2: {"ok", "state", "error"})
// ---------------------------------------------------------------------------

void test_make_response() {
  CHECK_STR(ecc::make_response(true, State::Running, ""),
            "{\"ok\":true,\"state\":\"Running\",\"error\":\"\"}");
  // ECC 不達 = ok:false + state Unknown(§8.2)。error は JSON エスケープされる。
  CHECK_STR(ecc::make_response(false, State::Unknown, "connect failed: \"refused\"\n"),
            "{\"ok\":false,\"state\":\"Unknown\","
            "\"error\":\"connect failed: \\\"refused\\\"\\n\"}");
}

// ---------------------------------------------------------------------------
// 4. 状態文字列マップ
// ---------------------------------------------------------------------------

void test_state_strings() {
  const State all[] = {State::Off,     State::Idle,    State::Described, State::Prepared,
                       State::Ready,   State::Running, State::Paused,    State::Unknown};
  const char* names[] = {"Off",   "Idle",    "Described", "Prepared",
                         "Ready", "Running", "Paused",    "Unknown"};
  for (int i = 0; i < 8; ++i) {
    CHECK_STR(ecc::to_string(all[i]), names[i]);
    CHECK(ecc::state_from_string(names[i]) == all[i]);
  }
  // 知らない綴りは Unknown に落とす(例外にしない)。
  CHECK(ecc::state_from_string("running") == State::Unknown);
  CHECK(ecc::state_from_string("") == State::Unknown);
}

// ---------------------------------------------------------------------------
// 5. fake-ECC の状態遷移表 = **実 ECC の SM**(036)
//
// 出典は reference/20190315_patched:
//   GetBench/src/get/rc/BackEnd.cpp:250-270 / :924 / :955-962、
//   StateMachine/src/dhsm/Engine.cpp:334-352。
// ---------------------------------------------------------------------------

void test_state_machine_happy_path() {
  State s = State::Off;
  std::string err;
  // 一周: Off →(describe/prepare/configure/start)→ Running →(stop)→ Ready →
  //       **歩き戻し breakup → reset → reset** → Idle。
  // `reset` が 2 回要るのが 036 の眼目 —— `EV_UNDO` は 1 段しか戻さない。
  const struct {
    const char* action;
    State want;
  } steps[] = {
      {"describe", State::Described}, {"prepare", State::Prepared},  {"configure", State::Ready},
      {"start", State::Running},      {"stop", State::Ready},        {"breakup", State::Prepared},
      {"reset", State::Described},    {"reset", State::Idle},
  };
  for (const auto& st : steps) {
    State next = State::Unknown;
    const ecc::Step step = ecc::next_state(s, st.action, next, err);
    if (step != ecc::Step::Applied) {
      std::printf("  (action=%s from=%s err=%s)\n", st.action, ecc::to_string(s), err.c_str());
    }
    CHECK(step == ecc::Step::Applied);
    CHECK(next == st.want);
    s = next;
  }
  // status は状態を変えない(いつでも可)。
  State next = State::Unknown;
  CHECK(ecc::next_state(State::Running, "status", next, err) == ecc::Step::Applied);
  CHECK(next == State::Running);
}

// **036 の核心**: `reset`(= EV_UNDO)は 1 段戻すだけで、`Active`(Ready/Running/Paused)
// からは遷移が無く**無音で無視**される。`Ready → Idle` には breakup → reset → reset が要る。
void test_reset_walks_back_one_step_at_a_time() {
  std::string err;
  State next = State::Unknown;

  // EV_UNDO の 2 本(BackEnd.cpp:250-270)。
  CHECK(ecc::next_state(State::Prepared, "reset", next, err) == ecc::Step::Applied);
  CHECK(next == State::Described);
  CHECK(ecc::next_state(State::Described, "reset", next, err) == ecc::Step::Applied);
  CHECK(next == State::Idle);

  // Active からは**遷移が無い** = Ignored(例外も出ない・状態も動かない)。
  for (const State active : {State::Ready, State::Running, State::Paused}) {
    CHECK(ecc::next_state(active, "reset", next, err) == ecc::Step::Ignored);
    CHECK(next == active);
    CHECK_STR(err, "");  // **無音**: エラー文言すら出ない
  }
  // 底(Off / Idle)でも同じく無音(これ以上戻れない)。
  CHECK(ecc::next_state(State::Idle, "reset", next, err) == ecc::Step::Ignored);
  CHECK(next == State::Idle);
  CHECK(ecc::next_state(State::Off, "reset", next, err) == ecc::Step::Ignored);
  CHECK(next == State::Off);

  // EV_BREAK は Active のどこからでも Prepared へ(ST_ACTIVE は複合状態)。
  for (const State active : {State::Ready, State::Running, State::Paused}) {
    CHECK(ecc::next_state(active, "breakup", next, err) == ecc::Step::Applied);
    CHECK(next == State::Prepared);
  }
  CHECK(ecc::next_state(State::Described, "breakup", next, err) == ecc::Step::Ignored);
  CHECK(next == State::Described);

  // `Ready → Idle` の歩き戻し 3 手を通しで(controller が実装する列そのもの)。
  State s = State::Ready;
  const char* walk[] = {"breakup", "reset", "reset"};
  const State want[] = {State::Prepared, State::Described, State::Idle};
  for (int i = 0; i < 3; ++i) {
    CHECK(ecc::next_state(s, walk[i], next, err) == ecc::Step::Applied);
    CHECK(next == want[i]);
    s = next;
  }
}

// **036**: `configure` は `ST_PREPARED` ガード付き(BackEnd.cpp:955-962)。
// Prepared 以外では**黙ってスキップ**される —— エラーにならないのが一番危ないところ。
void test_configure_is_silently_skipped_outside_prepared() {
  std::string err;
  State next = State::Unknown;

  CHECK(ecc::next_state(State::Prepared, "configure", next, err) == ecc::Step::Applied);
  CHECK(next == State::Ready);

  for (const State from : {State::Off, State::Idle, State::Described, State::Ready,
                           State::Running, State::Paused}) {
    CHECK(ecc::next_state(from, "configure", next, err) == ecc::Step::Ignored);
    CHECK(next == from);  // DataLinkSet は張り替わらない = CoBo は古い宛先のまま
    CHECK_STR(err, "");
  }
}

void test_state_machine_rejects_out_of_order() {
  std::string err;
  State next = State::Unknown;

  // listen 以前の問題: describe していないのに start
  CHECK(ecc::next_state(State::Off, "start", next, err) == ecc::Step::Denied);
  CHECK(err.find("start") != std::string::npos);
  CHECK(err.find("Off") != std::string::npos);

  CHECK(ecc::next_state(State::Described, "start", next, err) == ecc::Step::Denied);
  CHECK(ecc::next_state(State::Ready, "prepare", next, err) == ecc::Step::Denied);
  CHECK(ecc::next_state(State::Running, "start", next, err) == ecc::Step::Denied);  // 二重 start
  CHECK(ecc::next_state(State::Ready, "stop", next, err) == ecc::Step::Denied);  // 走っていない
  CHECK(ecc::next_state(State::Off, "chirp", next, err) == ecc::Step::Denied);   // 未知 action

  // **034 の事故の再現**: 前の run の `ecc stop` 後(= Ready)に reset 1 発では Idle に
  // 届かず(Ignored)、続く describe は順序違反になる。この 2 行が「歩き戻しを省いた
  // 実装は必ず落ちる」ことの保証である。
  CHECK(ecc::next_state(State::Ready, "reset", next, err) == ecc::Step::Ignored);
  CHECK(next == State::Ready);
  CHECK(ecc::next_state(State::Ready, "describe", next, err) == ecc::Step::Denied);
}

// ---------------------------------------------------------------------------
// 6. 最小 JSON パーサ(ecc_core が乗っている土台)
// ---------------------------------------------------------------------------

void test_json_min() {
  jsonmin::Value v;
  std::string err;
  CHECK(jsonmin::parse(
      "{\"a\": \"x\\ty\", \"b\": [1, -2.5e1, true, false, null], \"c\": {\"d\": 7}}", v, err));
  CHECK(v.type == jsonmin::Value::Type::Object);
  const jsonmin::Value* a = v.find("a");
  CHECK(a != nullptr && a->str == "x\ty");
  const jsonmin::Value* b = v.find("b");
  CHECK(b != nullptr && b->arr.size() == 5);
  if (b != nullptr && b->arr.size() == 5) {
    CHECK_EQ(b->arr[0].as_int(), 1);
    CHECK_EQ(b->arr[1].as_int(), -25);  // -2.5e1 = -25
    CHECK(b->arr[2].type == jsonmin::Value::Type::Bool && b->arr[2].boolean);
    CHECK(b->arr[3].type == jsonmin::Value::Type::Bool && !b->arr[3].boolean);
    CHECK(b->arr[4].type == jsonmin::Value::Type::Null);
  }
  const jsonmin::Value* c = v.find("c");
  CHECK(c != nullptr && c->find("d") != nullptr && c->find("d")->as_int() == 7);

  // 壊れた入力は false + 理由(throw しない)。
  const char* bad[] = {"", "{", "{\"a\"}", "{\"a\":}", "[1,]", "tru", "\"unterminated",
                       "{\"a\":1}x"};
  for (const char* s : bad) {
    jsonmin::Value bv;
    std::string berr;
    const bool ok = jsonmin::parse(s, bv, berr);
    if (ok) std::printf("  (accepted bad input: [%s])\n", s);
    CHECK(!ok);
    CHECK(!berr.empty());
  }
  // 深すぎるネストで stack を焼かない(深さ上限で弾く)。
  std::string deep(200, '[');
  jsonmin::Value dv;
  std::string derr;
  CHECK(!jsonmin::parse(deep, dv, derr));
}

}  // namespace

int main() {
  test_data_link_set_single_cobo();
  test_data_link_set_two_cobo();
  test_data_link_set_escapes_attributes();
  test_data_link_set_round_trip();
  test_parse_configure_request();
  test_parse_simple_actions();
  test_parse_rejects_bad_input();
  test_parse_defaults_flow_type();
  test_make_response();
  test_state_strings();
  test_state_machine_happy_path();
  test_reset_walks_back_one_step_at_a_time();
  test_configure_is_silently_skipped_outside_prepared();
  test_state_machine_rejects_out_of_order();
  test_json_min();
  return tpccheck::report("test_ecc_bridge");
}
