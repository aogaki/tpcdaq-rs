// ecc_e2e_client.cpp — run_ecc_e2e.sh の中身(TODO/017 §テスト「統合」)。
//
// controller(Rust)の立場で ecc_bridge を ZMQ REQ で叩き、fake-ECC までの往復を機械照合する:
//
//   status(Off) → start(順序違反 = エラー) → describe → prepare
//   → configure(誰も listen していないポート) → **start = "Could not establish data link"**
//     (= GET の error フラグが `WHEN_START` に立ち、state は Ready のまま残る。043)
//   → status を 2 回読んでも消えない / 遷移の無い reset でも消えない / breakup で消える
//   → 受信ポートを listen → configure(その実ポート) → start = 成功 + **実際に繋がってくる**
//   → 走行中は 1 バイトも来ない(データ送出は replay の仕事)
//   → stop(= EOF)→ breakup → reset → 壊れた JSON でもブリッジは死なない
//
// listen-before-start の負性テストがここの主役(SPEC §8.3)。
// Ice は要らない(ZMQ REQ + 生 TCP listen だけ)。

#include "check.hpp"
#include "ecc_core.hpp"
#include "json_min.hpp"

#include <zmq.h>

#include <arpa/inet.h>
#include <errno.h>
#include <netinet/in.h>
#include <sys/select.h>
#include <sys/socket.h>
#include <unistd.h>

#include <cstdio>
#include <cstring>
#include <string>

namespace {

// --- ZMQ REQ ---------------------------------------------------------------
class Req {
 public:
  bool open(const std::string& endpoint) {
    ctx_ = zmq_ctx_new();
    if (ctx_ == nullptr) return false;
    sock_ = zmq_socket(ctx_, ZMQ_REQ);
    if (sock_ == nullptr) return false;
    const int timeout = 5000;  // ms。返事が来ないこと自体を失敗として扱う
    const int linger = 0;
    zmq_setsockopt(sock_, ZMQ_RCVTIMEO, &timeout, sizeof(timeout));
    zmq_setsockopt(sock_, ZMQ_SNDTIMEO, &timeout, sizeof(timeout));
    zmq_setsockopt(sock_, ZMQ_LINGER, &linger, sizeof(linger));
    return zmq_connect(sock_, endpoint.c_str()) == 0;
  }
  ~Req() {
    if (sock_ != nullptr) zmq_close(sock_);
    if (ctx_ != nullptr) zmq_ctx_term(ctx_);
  }

  std::string call(const std::string& request) {
    if (zmq_send(sock_, request.data(), request.size(), 0) < 0) {
      return std::string("<send failed: ") + zmq_strerror(zmq_errno()) + ">";
    }
    zmq_msg_t msg;
    zmq_msg_init(&msg);
    if (zmq_msg_recv(&msg, sock_, 0) < 0) {
      zmq_msg_close(&msg);
      return std::string("<recv failed: ") + zmq_strerror(zmq_errno()) + ">";
    }
    std::string reply(static_cast<const char*>(zmq_msg_data(&msg)), zmq_msg_size(&msg));
    zmq_msg_close(&msg);
    return reply;
  }

 private:
  void* ctx_ = nullptr;
  void* sock_ = nullptr;
};

// レスポンス(SPEC §8.2 v1.14)の 4 フィールドを機械照合する。
//
// `want_ecc_error` は **GET の error フラグ**の期待値(043)。全呼び出しで指定する ——
// フラグは「立ったまま残る」ことが本体なので、見ていない箇所があると set/clear の
// 取り違えを取り逃がす。
void expect(const std::string& what, const std::string& reply, bool want_ok,
            const char* want_state, const char* want_ecc_error,
            const char* want_error_substring = nullptr) {
  jsonmin::Value v;
  std::string err;
  if (!jsonmin::parse(reply, v, err)) {
    std::printf("FAIL %s: reply is not JSON [%s] (%s)\n", what.c_str(), reply.c_str(),
                err.c_str());
    ++tpccheck::g_fail;
    return;
  }
  const jsonmin::Value* ok = v.find("ok");
  const jsonmin::Value* state = v.find("state");
  const jsonmin::Value* error = v.find("error");
  const jsonmin::Value* ecc_error = v.find("ecc_error");
  if (ok == nullptr || state == nullptr || error == nullptr || ecc_error == nullptr) {
    std::printf("FAIL %s: reply misses ok/state/error/ecc_error [%s]\n", what.c_str(),
                reply.c_str());
    ++tpccheck::g_fail;
    return;
  }
  if (ecc_error->str != want_ecc_error) {
    std::printf("FAIL %s: got ecc_error=%s (want %s) [%s]\n", what.c_str(),
                ecc_error->str.c_str(), want_ecc_error, reply.c_str());
    ++tpccheck::g_fail;
  } else {
    ++tpccheck::g_pass;
  }
  if (ok->boolean != want_ok || state->str != want_state) {
    std::printf("FAIL %s: got ok=%d state=%s error=%s (want ok=%d state=%s)\n", what.c_str(),
                ok->boolean ? 1 : 0, state->str.c_str(), error->str.c_str(), want_ok ? 1 : 0,
                want_state);
    ++tpccheck::g_fail;
  } else {
    ++tpccheck::g_pass;
  }
  if (want_error_substring != nullptr) {
    const bool has = error->str.find(want_error_substring) != std::string::npos;
    if (!has) {
      std::printf("FAIL %s: error [%s] does not contain [%s]\n", what.c_str(),
                  error->str.c_str(), want_error_substring);
      ++tpccheck::g_fail;
    } else {
      ++tpccheck::g_pass;
    }
  }
}

// --- 受信側(receiver の代役)------------------------------------------------
// 127.0.0.1 の空きポートを listen する。bound_port に実ポートを返す。
int listen_ephemeral(int& bound_port) {
  const int fd = ::socket(AF_INET, SOCK_STREAM, 0);
  if (fd < 0) return -1;
  const int one = 1;
  ::setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &one, sizeof(one));
  sockaddr_in addr{};
  addr.sin_family = AF_INET;
  addr.sin_port = 0;
  ::inet_pton(AF_INET, "127.0.0.1", &addr.sin_addr);
  if (::bind(fd, reinterpret_cast<sockaddr*>(&addr), sizeof(addr)) != 0 || ::listen(fd, 4) != 0) {
    ::close(fd);
    return -1;
  }
  sockaddr_in actual{};
  socklen_t len = sizeof(actual);
  if (::getsockname(fd, reinterpret_cast<sockaddr*>(&actual), &len) != 0) {
    ::close(fd);
    return -1;
  }
  bound_port = ntohs(actual.sin_port);
  return fd;
}

bool readable(int fd, int timeout_ms) {
  fd_set rset;
  FD_ZERO(&rset);
  FD_SET(fd, &rset);
  timeval tv{timeout_ms / 1000, (timeout_ms % 1000) * 1000};
  return ::select(fd + 1, &rset, nullptr, nullptr, &tv) > 0;
}

std::string configure_request(int port) {
  // SPEC §8.2 のリクエスト形。router_port = receiver が実際に bind したポート。
  char buf[256];
  std::snprintf(buf, sizeof(buf),
                "{\"action\":\"configure\",\"config_id\":\"e2e\",\"links\":"
                "[{\"sender\":\"CoBo[0]\",\"router_ip\":\"127.0.0.1\",\"router_port\":%d,"
                "\"type\":\"TCP\"}]}",
                port);
  return buf;
}

}  // namespace

int main(int argc, char** argv) {
  std::string bridge;
  for (int i = 1; i < argc; ++i) {
    const std::string a = argv[i];
    if (a == "--bridge" && i + 1 < argc) {
      bridge = argv[++i];
    } else {
      std::fprintf(stderr, "usage: ecc_e2e_client --bridge tcp://127.0.0.1:PORT\n");
      return 2;
    }
  }
  if (bridge.empty()) {
    std::fprintf(stderr, "usage: ecc_e2e_client --bridge tcp://127.0.0.1:PORT\n");
    return 2;
  }

  Req req;
  if (!req.open(bridge)) {
    std::fprintf(stderr, "ecc_e2e_client: cannot connect to %s\n", bridge.c_str());
    return 1;
  }

  // 0. 初期状態(fake-ECC は Off から始まる。error フラグの初期値は NO_ERR —— 実 ECC も
  //    BackEnd::BackEnd() で NO_ERR、BackEnd.cpp:132)
  expect("status(initial)", req.call("{\"action\":\"status\"}"), true, "Off", "NO_ERR");

  // 1. 順序違反: describe もせずに start —— ECC がエラーを返し、状態は動かない。
  //    実 ECC ではこれは「遷移が無い」= Ignored でアクションが 1 つも走らないので、
  //    **error フラグは触られない**(NO_ERR のまま)。
  expect("start before describe", req.call("{\"action\":\"start\"}"), false, "Off", "NO_ERR",
         "invalid transition");

  // 2. 正順で Ready まで(listen していないポートを指す DataLinkSet)
  expect("describe", req.call("{\"action\":\"describe\",\"config_id\":\"e2e\"}"), true,
         "Described", "NO_ERR");
  expect("prepare", req.call("{\"action\":\"prepare\",\"config_id\":\"e2e\"}"), true, "Prepared",
         "NO_ERR");

  int closed_port = 0;
  const int probe = listen_ephemeral(closed_port);
  CHECK(probe >= 0);
  if (probe >= 0) ::close(probe);  // ポート番号だけ借りて即座に閉じる = 誰も listen していない
  expect("configure(no listener)", req.call(configure_request(closed_port)), true, "Ready",
         "NO_ERR");

  // 3. **listen-before-start の負性テスト**(SPEC §8.3)+ **error フラグの set**(043)。
  //    遷移のアクション(データリンク確立)が失敗 → CATCH_SM_EXCEPTIONS(SM::WHEN_START)が
  //    フラグを立て(BackEnd.cpp:1052 / 329-333)、state は渡り切らないので Ready のまま
  //    (dhsm/Engine.cpp:298 に到達しない)。041 D-1 の `IDLE / WHEN_DESCRIBE` と同じ形。
  expect("start without listener", req.call("{\"action\":\"start\"}"), false, "Ready",
         "WHEN_START", "Could not establish data link");

  // 3b. **フラグは残る**(043 の眼目 —— これが見えないと 041 D-1 の再来)。
  //     getStatus は smStatus を読むだけでフラグを消さない(BackEnd.cpp:321-327)。
  //     何度読んでも消えないことまで見る。
  expect("status(after failed start)", req.call("{\"action\":\"status\"}"), true, "Ready",
         "WHEN_START");
  expect("status(read twice)", req.call("{\"action\":\"status\"}"), true, "Ready", "WHEN_START");

  // 3c. **遷移が無ければフラグは消えない**。`Ready` からの reset は EV_UNDO が無いので
  //     Ignored = アクションが 1 つも走らない → resetErrorFlag(BackEnd.cpp:249-290 の
  //     1 番目のアクション)も走らない。ok=true(無音)で state も動かないのに、
  //     フラグだけは WHEN_START のまま残る。
  expect("reset(ignored, keeps the flag)", req.call("{\"action\":\"reset\"}"), true, "Ready",
         "WHEN_START");

  // 4. 受信ポートを用意してから configure → start
  int listen_port = 0;
  const int server = listen_ephemeral(listen_port);
  CHECK(server >= 0);
  if (server < 0) return tpccheck::report("ecc_e2e");

  // 直前の configure 失敗で ECC は Active(Ready)に居る。**実 ECC の configure は
  // `ST_PREPARED` からしか効かない**(BackEnd.cpp:955-962 のガード。それ以外は黙ってスキップ)
  // ので、張り替えの前に breakup で Prepared へ戻す(SPEC v1.12 §1.3 / TODO/036)。
  // **error フラグの clear**(043): breakup は Ready から遷移が**ある**ので渡り、
  // その 1 番目のアクション resetErrorFlag が WHEN_START を消す(BackEnd.cpp:268-270)。
  expect("breakup(clears the flag)", req.call("{\"action\":\"breakup\"}"), true, "Prepared",
         "NO_ERR");
  expect("configure(with listener)", req.call(configure_request(listen_port)), true, "Ready",
         "NO_ERR");
  expect("start", req.call("{\"action\":\"start\"}"), true, "Running", "NO_ERR");

  // CoBo(fake-ECC)が実際に繋いで来たか —— data link の実在確認。
  CHECK(readable(server, 2000));
  const int conn = ::accept(server, nullptr, nullptr);
  CHECK(conn >= 0);

  // 走行中でも 1 バイトも来ない(fake-ECC はデータを送らない。送出は graw_replay の仕事)。
  CHECK(!readable(conn, 200));

  // 5. stop → 接続が閉じる(受信側から見ると EOF = run 境界)
  expect("stop", req.call("{\"action\":\"stop\"}"), true, "Ready", "NO_ERR");
  CHECK(readable(conn, 2000));
  char scratch[16];
  const ssize_t n = ::recv(conn, scratch, sizeof(scratch), 0);
  CHECK_EQ(n, 0);  // 0 = EOF(データではなく切断)
  if (conn >= 0) ::close(conn);
  ::close(server);

  // 6. 残りの遷移
  expect("breakup", req.call("{\"action\":\"breakup\"}"), true, "Prepared", "NO_ERR");
  // 実 ECC の reset は `EV_UNDO` = **1 段戻す**(BackEnd.cpp:250-270)。Prepared → Idle は 2 段。
  expect("reset(1)", req.call("{\"action\":\"reset\"}"), true, "Described", "NO_ERR");
  expect("reset(2)", req.call("{\"action\":\"reset\"}"), true, "Idle", "NO_ERR");

  // 7. 壊れた入力でブリッジは死なない(状態は返る、次のリクエストも通る)。
  //    リクエストが ECC に届いてすらいないので `ecc_error` は ECC の現況(NO_ERR)を映す ——
  //    輸送層の `error` と GET の error フラグが**別軸**であることの現れ。
  expect("malformed json", req.call("{\"action\":"), false, "Idle", "NO_ERR", "parse error");
  expect("unknown action", req.call("{\"action\":\"launch\"}"), false, "Idle", "NO_ERR",
         "unknown action");
  expect("configure without links", req.call("{\"action\":\"configure\"}"), false, "Idle",
         "NO_ERR", "links");
  expect("status(final)", req.call("{\"action\":\"status\"}"), true, "Idle", "NO_ERR");

  return tpccheck::report("ecc_e2e");
}
