// ecc_bridge.cpp — ECC(GET の run 制御 Ice サーバ)への薄い JSON REQ/REP ブリッジ。
//                  SPEC §8.2 / §3.2(REP = tcp://*:47200)、TODO/017。
//
//   controller(Rust) --ZMQ REQ/REP(JSON)--> ecc_bridge --Ice(encoding 1.1)--> ECC
//
// * リクエスト: {"action":"describe|prepare|configure|start|stop|breakup|reset|status",
//                "config_id":"...", "links":[{"sender":"CoBo[0]","router_ip":"...",
//                "router_port":46005,"type":"TCP"}, ...]}
// * レスポンス: {"ok":bool,"state":"Off|Idle|Described|Prepared|Ready|Running|Paused|Unknown",
//                "error":"..."}
// * **never throw**: Ice の例外は全部 Result 化。ECC 不達 = {"ok":false,"state":"Unknown"}。
// * REP は 1 スレッド逐次(コマンドは人間スケール)。**必ず 1 リクエスト 1 レスポンス**を返す
//   —— 返さないと REQ 側が永久に待つ。
//
// Ice を含むのはこの TU と fake_ecc.cpp だけ(核 ecc_core.hpp は Ice も ZMQ も知らない)。

#include "ecc_core.hpp"

#include <Ice/Ice.h>
#include <zmq.h>

#include "get/GetEcc.h"        // slice2cpp 生成(get::rc::StateMachine / State / Status)
#include "mdaq/utl/CmdException.h"

#include <csignal>
#include <cstdio>
#include <cstring>
#include <optional>
#include <sstream>
#include <stdexcept>
#include <string>
#include <vector>

namespace {

volatile std::sig_atomic_t g_stop = 0;
void on_signal(int) { g_stop = 1; }

// ---------------------------------------------------------------------------
// EccClient —— **流用元**: tpcdaq::control::EccController
//   (~/test/get/tpcdaq/{include/tpcdaq/control/ecc_controller.hpp,src/control/ecc_controller.cpp})
// ユーザー自身のコードなので third_party 隔離は不要(TODO/017 §1)。
// 変更点は 3 つだけ:
//   (a) pimpl をやめた(このプロセスが唯一の利用者。Ice を隠す相手がいない)
//   (b) configure() が **N 本の DataLink**(2 CoBo)を受ける
//   (c) CmdException を個別に catch して `reason` を返す(Ice 3.8 の UserException::what() は
//       型 id しか返さないので、"Could not establish data link" が消えてしまう)
// ---------------------------------------------------------------------------
struct Result {
  bool ok = false;
  std::string error;
  static Result success() { return {true, {}}; }
  static Result fail(std::string e) { return {false, std::move(e)}; }
};

ecc::State map_state(get::rc::State s) {
  switch (s) {
    case get::rc::State::Off: return ecc::State::Off;
    case get::rc::State::Idle: return ecc::State::Idle;
    case get::rc::State::Described: return ecc::State::Described;
    case get::rc::State::Prepared: return ecc::State::Prepared;
    case get::rc::State::Ready: return ecc::State::Ready;
    case get::rc::State::Running: return ecc::State::Running;
    case get::rc::State::Paused: return ecc::State::Paused;
    default: return ecc::State::Unknown;
  }
}

class EccClient {
 public:
  explicit EccClient(std::string proxy_str) : proxy_str_(std::move(proxy_str)) {}
  EccClient(const EccClient&) = delete;
  EccClient& operator=(const EccClient&) = delete;
  ~EccClient() {
    if (comm_) {
      try {
        comm_->destroy();
      } catch (...) {
      }
    }
  }

  // communicator + proxy を作り ice_ping で疎通確認。既に張れていれば何もしない。
  Result connect() {
    if (proxy_) return Result::success();
    return guarded([&] {
      if (!comm_) comm_ = Ice::initialize();
      // **encoding 1.1 を明示強制**(SPEC §8.2 / CLAUDE.md)。手元の Ice は 3.8 でも
      // 実 getEccServer は 3.6/3.7 系 —— 既定エンコーディングの差で marshalling が
      // こける余地を潰す。1.1 は Ice 3.5+ で共通。
      proxy_ = get::rc::StateMachinePrx(comm_, proxy_str_).ice_encodingVersion(Ice::Encoding_1_1);
      proxy_->ice_ping();  // 到達不能なら Ice::LocalException
      // 主張ではなく**実際の値**を出す(P5 の実 ECC 検証で読む材料)。
      std::ostringstream enc;
      enc << proxy_->ice_getEncodingVersion();
      std::fprintf(stderr, "ecc_bridge: connected to %s (encoding %s)\n", proxy_str_.c_str(),
                   enc.str().c_str());
    });
  }

  Result describe(const std::string& config_id) {
    return run([&](get::rc::StateMachinePrx& p) { p.describe(config_id); });
  }
  Result prepare(const std::string& config_id) {
    return run([&](get::rc::StateMachinePrx& p) { p.prepare(config_id); });
  }
  Result configure(const std::string& config_id, const std::string& data_link_set) {
    return run([&](get::rc::StateMachinePrx& p) { p.configure(config_id, data_link_set); });
  }
  Result start() {
    return run([](get::rc::StateMachinePrx& p) { p.start(); });
  }
  Result stop() {
    return run([](get::rc::StateMachinePrx& p) { p.stop(); });
  }
  Result breakup() {
    return run([](get::rc::StateMachinePrx& p) { p.breakup(); });
  }
  Result reset() {
    return run([](get::rc::StateMachinePrx& p) { p.reset(); });
  }

  // 取得できなければ out = Unknown + ok=false(§8.2「ECC 不達 = state Unknown」)。
  Result status(ecc::State& out) {
    out = ecc::State::Unknown;
    ecc::State& o = out;
    return run([&](get::rc::StateMachinePrx& p) {
      get::rc::Status st;
      p.getStatus(st);
      o = map_state(st.s);
    });
  }

  // 応答に載せる状態(取得失敗は Unknown。理由は直前の操作の error が語る)。
  ecc::State state() {
    ecc::State s = ecc::State::Unknown;
    status(s);
    return s;
  }

 private:
  // 例外 → Result。LocalException(= 接続断)なら proxy を捨てて次回張り直す。
  template <class F>
  Result guarded(F&& fn) {
    try {
      fn();
      return Result::success();
    } catch (const mdaq::utl::CmdException& e) {
      return Result::fail(e.reason);  // ECC が返す本文(例 "Could not establish data link. ...")
    } catch (const Ice::LocalException& e) {
      proxy_.reset();  // 次のリクエストで再接続を試みる
      return Result::fail(e.what());
    } catch (const Ice::Exception& e) {
      return Result::fail(e.what());
    } catch (const std::exception& e) {
      return Result::fail(e.what());
    }
  }

  // 未接続なら繋いでから本体を実行(ECC が後から起動しても回復する)。
  template <class F>
  Result run(F&& fn) {
    const Result c = connect();
    if (!c.ok) return c;
    return guarded([&] {
      if (!proxy_) throw std::runtime_error("ECC proxy not available");
      fn(*proxy_);
    });
  }

  std::string proxy_str_;
  Ice::CommunicatorPtr comm_;
  std::optional<get::rc::StateMachinePrx> proxy_;  // Ice 3.8 の既定 ctor は protected
};

// ---------------------------------------------------------------------------
// ZMQ REP サーバ
// ---------------------------------------------------------------------------

struct Options {
  std::string bind = "tcp://*:47200";                      // SPEC §3.2
  std::string ecc_proxy = "GetEcc:tcp -h 127.0.0.1 -p 46002";  // SPEC §3.1 の既定
};

void usage() {
  std::printf(
      "usage: ecc_bridge [--bind ENDPOINT] [--ecc-proxy PROXY]\n"
      "\n"
      "  --bind ENDPOINT   ZMQ REP bind (default tcp://*:47200)\n"
      "  --ecc-proxy PROXY Ice proxy of the ECC state machine\n"
      "                    (default \"GetEcc:tcp -h 127.0.0.1 -p 46002\")\n"
      "\n"
      "Prints one line \"BIND <endpoint>\" on stdout once the REP socket is bound\n"
      "(so a test can use an ephemeral port), then serves JSON requests (SPEC 8.2)\n"
      "until SIGINT/SIGTERM.\n");
}

// action を ECC 呼び出しに割り付ける(status は handle() が直接扱う)。
Result dispatch(EccClient& ecc, const ecc::Request& req) {
  if (req.action == "describe") return ecc.describe(req.config_id);
  if (req.action == "prepare") return ecc.prepare(req.config_id);
  if (req.action == "configure") {
    return ecc.configure(req.config_id, ecc::data_link_set_xml(req.links));
  }
  if (req.action == "start") return ecc.start();
  if (req.action == "stop") return ecc.stop();
  if (req.action == "breakup") return ecc.breakup();
  if (req.action == "reset") return ecc.reset();
  return Result::fail("unknown action: " + req.action);  // parse_request が弾くので到達しない
}

std::string handle(EccClient& ecc, const std::string& request_json) {
  const ecc::Request req = ecc::parse_request(request_json);
  if (!req.valid) {
    // 壊れたリクエストでも ECC の状態は返す(状態が見えないと運用が止まる)。
    std::fprintf(stderr, "ecc_bridge: rejected request: %s\n", req.error.c_str());
    return ecc::make_response(false, ecc.state(), req.error);
  }
  // status は「状態が取れたか」がそのまま ok(ECC 不達なら ok=false + Unknown)。
  ecc::State st = ecc::State::Unknown;
  if (req.action == "status") {
    const Result r = ecc.status(st);
    std::fprintf(stderr, "ecc_bridge: status -> ok=%d state=%s %s\n", r.ok ? 1 : 0,
                 ecc::to_string(st), r.error.c_str());
    return ecc::make_response(r.ok, st, r.error);
  }

  const Result r = dispatch(ecc, req);
  st = ecc.state();
  std::fprintf(stderr, "ecc_bridge: %s -> ok=%d state=%s %s\n", req.action.c_str(), r.ok ? 1 : 0,
               ecc::to_string(st), r.error.c_str());
  return ecc::make_response(r.ok, st, r.error);
}

}  // namespace

int main(int argc, char** argv) {
  Options opt;
  for (int i = 1; i < argc; ++i) {
    const std::string a = argv[i];
    const bool has_value = (i + 1 < argc);
    if (a == "--help" || a == "-h") {
      usage();
      return 0;
    }
    if (a == "--bind" && has_value) {
      opt.bind = argv[++i];
    } else if (a == "--ecc-proxy" && has_value) {
      opt.ecc_proxy = argv[++i];
    } else {
      std::fprintf(stderr, "ecc_bridge: bad argument '%s'\n", a.c_str());
      usage();
      return 2;
    }
  }

  std::signal(SIGINT, on_signal);
  std::signal(SIGTERM, on_signal);

  void* ctx = zmq_ctx_new();
  if (ctx == nullptr) {
    std::fprintf(stderr, "ecc_bridge: zmq_ctx_new failed: %s\n", zmq_strerror(zmq_errno()));
    return 1;
  }
  void* rep = zmq_socket(ctx, ZMQ_REP);
  if (rep == nullptr) {
    std::fprintf(stderr, "ecc_bridge: zmq_socket failed: %s\n", zmq_strerror(zmq_errno()));
    zmq_ctx_term(ctx);
    return 1;
  }
  const int linger = 0;
  zmq_setsockopt(rep, ZMQ_LINGER, &linger, sizeof(linger));
  if (zmq_bind(rep, opt.bind.c_str()) != 0) {
    std::fprintf(stderr, "ecc_bridge: zmq_bind(%s) failed: %s\n", opt.bind.c_str(),
                 zmq_strerror(zmq_errno()));
    zmq_close(rep);
    zmq_ctx_term(ctx);
    return 1;
  }

  // 実際に bind したエンドポイント(ポート 0 を指定したときの実ポート)を 1 行で出す。
  char endpoint[256];
  std::size_t endpoint_len = sizeof(endpoint);
  if (zmq_getsockopt(rep, ZMQ_LAST_ENDPOINT, endpoint, &endpoint_len) != 0) {
    std::snprintf(endpoint, sizeof(endpoint), "%s", opt.bind.c_str());
  }
  std::printf("BIND %s\n", endpoint);
  std::fflush(stdout);
  std::fprintf(stderr, "ecc_bridge: ecc proxy = %s\n", opt.ecc_proxy.c_str());

  EccClient ecc(opt.ecc_proxy);
  // 起動時に一度だけ繋ぎに行く。失敗しても**起動は続ける**(ECC が後から上がる運用がある)。
  const Result initial = ecc.connect();
  if (!initial.ok) {
    std::fprintf(stderr, "ecc_bridge: ECC not reachable yet: %s\n", initial.error.c_str());
  }

  while (g_stop == 0) {
    zmq_pollitem_t item{rep, 0, ZMQ_POLLIN, 0};
    const int rc = zmq_poll(&item, 1, 200);  // ms。SIGINT を 0.2 s 以内に見る
    if (rc < 0) {
      if (zmq_errno() == EINTR) continue;
      std::fprintf(stderr, "ecc_bridge: zmq_poll failed: %s\n", zmq_strerror(zmq_errno()));
      break;
    }
    if (rc == 0) continue;

    zmq_msg_t msg;
    zmq_msg_init(&msg);
    if (zmq_msg_recv(&msg, rep, 0) < 0) {
      zmq_msg_close(&msg);
      if (zmq_errno() == EINTR) continue;
      std::fprintf(stderr, "ecc_bridge: zmq_msg_recv failed: %s\n", zmq_strerror(zmq_errno()));
      break;
    }
    const std::string request(static_cast<const char*>(zmq_msg_data(&msg)), zmq_msg_size(&msg));
    zmq_msg_close(&msg);

    const std::string reply = handle(ecc, request);
    if (zmq_send(rep, reply.data(), reply.size(), 0) < 0) {
      std::fprintf(stderr, "ecc_bridge: zmq_send failed: %s\n", zmq_strerror(zmq_errno()));
      break;
    }
  }

  std::fprintf(stderr, "ecc_bridge: shutting down\n");
  zmq_close(rep);
  zmq_ctx_term(ctx);
  return 0;
}
