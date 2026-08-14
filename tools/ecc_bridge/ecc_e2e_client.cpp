// ecc_e2e_client.cpp — run_ecc_e2e.sh の中身(TODO/017 §テスト「統合」)。
//
// controller(Rust)の立場で ecc_bridge を ZMQ REQ で叩き、fake-ECC までの往復を機械照合する:
//
//   status(Off) → start(順序違反 = エラー) → describe → prepare
//   → configure(誰も listen していないポート) → **start = "Could not establish data link"**
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

// レスポンス(SPEC §8.2)の 3 フィールドを機械照合する。
void expect(const std::string& what, const std::string& reply, bool want_ok,
            const char* want_state, const char* want_error_substring = nullptr) {
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
  if (ok == nullptr || state == nullptr || error == nullptr) {
    std::printf("FAIL %s: reply misses ok/state/error [%s]\n", what.c_str(), reply.c_str());
    ++tpccheck::g_fail;
    return;
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

  // 0. 初期状態(fake-ECC は Off から始まる)
  expect("status(initial)", req.call("{\"action\":\"status\"}"), true, "Off");

  // 1. 順序違反: describe もせずに start —— ECC がエラーを返し、状態は動かない
  expect("start before describe", req.call("{\"action\":\"start\"}"), false, "Off",
         "invalid transition");

  // 2. 正順で Ready まで(listen していないポートを指す DataLinkSet)
  expect("describe", req.call("{\"action\":\"describe\",\"config_id\":\"e2e\"}"), true,
         "Described");
  expect("prepare", req.call("{\"action\":\"prepare\",\"config_id\":\"e2e\"}"), true, "Prepared");

  int closed_port = 0;
  const int probe = listen_ephemeral(closed_port);
  CHECK(probe >= 0);
  if (probe >= 0) ::close(probe);  // ポート番号だけ借りて即座に閉じる = 誰も listen していない
  expect("configure(no listener)", req.call(configure_request(closed_port)), true, "Ready");

  // 3. **listen-before-start の負性テスト**(SPEC §8.3)
  expect("start without listener", req.call("{\"action\":\"start\"}"), false, "Ready",
         "Could not establish data link");

  // 4. 受信ポートを用意してから configure → start
  int listen_port = 0;
  const int server = listen_ephemeral(listen_port);
  CHECK(server >= 0);
  if (server < 0) return tpccheck::report("ecc_e2e");

  // 直前の configure 失敗で ECC は Active(Ready)に居る。**実 ECC の configure は
  // `ST_PREPARED` からしか効かない**(BackEnd.cpp:955-962 のガード。それ以外は黙ってスキップ)
  // ので、張り替えの前に breakup で Prepared へ戻す(SPEC v1.12 §1.3 / TODO/036)。
  expect("breakup(before re-configure)", req.call("{\"action\":\"breakup\"}"), true, "Prepared");
  expect("configure(with listener)", req.call(configure_request(listen_port)), true, "Ready");
  expect("start", req.call("{\"action\":\"start\"}"), true, "Running");

  // CoBo(fake-ECC)が実際に繋いで来たか —— data link の実在確認。
  CHECK(readable(server, 2000));
  const int conn = ::accept(server, nullptr, nullptr);
  CHECK(conn >= 0);

  // 走行中でも 1 バイトも来ない(fake-ECC はデータを送らない。送出は graw_replay の仕事)。
  CHECK(!readable(conn, 200));

  // 5. stop → 接続が閉じる(受信側から見ると EOF = run 境界)
  expect("stop", req.call("{\"action\":\"stop\"}"), true, "Ready");
  CHECK(readable(conn, 2000));
  char scratch[16];
  const ssize_t n = ::recv(conn, scratch, sizeof(scratch), 0);
  CHECK_EQ(n, 0);  // 0 = EOF(データではなく切断)
  if (conn >= 0) ::close(conn);
  ::close(server);

  // 6. 残りの遷移
  expect("breakup", req.call("{\"action\":\"breakup\"}"), true, "Prepared");
  // 実 ECC の reset は `EV_UNDO` = **1 段戻す**(BackEnd.cpp:250-270)。Prepared → Idle は 2 段。
  expect("reset(1)", req.call("{\"action\":\"reset\"}"), true, "Described");
  expect("reset(2)", req.call("{\"action\":\"reset\"}"), true, "Idle");

  // 7. 壊れた入力でブリッジは死なない(状態は返る、次のリクエストも通る)
  expect("malformed json", req.call("{\"action\":"), false, "Idle", "parse error");
  expect("unknown action", req.call("{\"action\":\"launch\"}"), false, "Idle", "unknown action");
  expect("configure without links", req.call("{\"action\":\"configure\"}"), false, "Idle",
         "links");
  expect("status(final)", req.call("{\"action\":\"status\"}"), true, "Idle");

  return tpccheck::report("ecc_e2e");
}
