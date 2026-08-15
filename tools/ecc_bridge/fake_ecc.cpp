// fake_ecc.cpp — 実 `.ice` 定義(reference/20190315_patched)の servant を最小実装した
//                テスト用 ECC(TODO/017 §2、SPEC §8.3「二層検証の軽い層」)。
//
// **流用元**: ~/test/get/tpcdaq/test/support/fake_ecc.{hpp,cpp}(ユーザー自身のコード)。
// 017 で変えた点:
//   (a) 状態遷移を **ecc_core.hpp の表で強制**(順序違反は CmdException)。流用元は無検査だった。
//   (b) start() は DataLinkSet の**全リンク**へ実 TCP connect を試み、繋がらなければ
//       CmdException("Could not establish data link. ...") を投げる —— listen-before-start の
//       負性テストの実体。実 ECC の同じ症状は
//       reference/20190315_patched/GetBench/src/get/daq/DaqCtrlNodeI.cpp:399 が出典。
//   (c) 繋がったら **1 バイトも送らず**保持し、stop() で閉じる(データ送出は graw_replay の仕事)。
//   (d) `--no-data-link`(既定 OFF、TODO/030 の裁定①): start() で **TCP を一切張らない**。
//       実機では「CoBo が張る 1 本の TCP がそのままデータリンク」なので、リプレイ側
//       (graw_replay)が CoBo 役を務める全通し試験では、この servant が別途 connect すると
//       **CoBo が 2 台**いる状態になる。receiver の accept ループは 1 接続ずつしか処理しない
//       ため、保持されたこの「幻の CoBo」が受け口を占有してデータが 1 バイトも読まれない。
//       接続してすぐ close する案は使えない —— receiver は `Ok(0)` を run 境界と解釈して
//       EOS を送るので run が空で閉じてしまう。よって「張らない」を選ぶ。
//       **既定は従来どおり張る**ので、016/017 の listen-before-start 負性テストは不変。
//   (e) **043**: `getStatus` の error フラグを実 ECC の set/clear 規則どおりに持つ
//       (規則と一次資料の行番号は ecc_core.hpp の「set / clear 規則」ブロック)。
//       要点だけ再掲: 遷移が渡ったら NO_ERR に消え、その後の副作用が失敗したら**その相の
//       コード**が立って state は動かない。遷移が無い(Ignored/Denied)ときは触らない。
//       getStatus では消えない。
//
// このプロセスは制御プレーンの模擬だけを行う。GET のビルドは要らない(Ice のみ)。

#include "ecc_core.hpp"

#include <Ice/Ice.h>

#include "get/GetEcc.h"
#include "mdaq/utl/CmdException.h"

#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <netinet/in.h>
#include <sys/select.h>
#include <sys/socket.h>
#include <unistd.h>

#include <chrono>
#include <csignal>
#include <cstdio>
#include <cstring>
#include <memory>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

namespace {

volatile std::sig_atomic_t g_stop = 0;
void on_signal(int) { g_stop = 1; }

constexpr int kConnectTimeoutMs = 2000;  // 到達しない相手でも制御プレーンを固めない

// 非ブロッキング connect + タイムアウト。成功なら fd(>=0)、失敗なら -1 + why。
int connect_with_timeout(const std::string& ip, int port, std::string& why) {
  const int fd = ::socket(AF_INET, SOCK_STREAM, 0);
  if (fd < 0) {
    why = std::string("socket(): ") + std::strerror(errno);
    return -1;
  }
  sockaddr_in addr{};
  addr.sin_family = AF_INET;
  addr.sin_port = htons(static_cast<std::uint16_t>(port));
  if (::inet_pton(AF_INET, ip.c_str(), &addr.sin_addr) != 1) {
    why = "bad router ipAddress '" + ip + "'";
    ::close(fd);
    return -1;
  }

  const int flags = ::fcntl(fd, F_GETFL, 0);
  ::fcntl(fd, F_SETFL, flags | O_NONBLOCK);

  int rc = ::connect(fd, reinterpret_cast<sockaddr*>(&addr), sizeof(addr));
  if (rc != 0 && errno == EINPROGRESS) {
    fd_set wset;
    FD_ZERO(&wset);
    FD_SET(fd, &wset);
    timeval tv{kConnectTimeoutMs / 1000, (kConnectTimeoutMs % 1000) * 1000};
    const int sel = ::select(fd + 1, nullptr, &wset, nullptr, &tv);
    if (sel == 0) {
      why = "connect to " + ip + ":" + std::to_string(port) + " timed out";
      ::close(fd);
      return -1;
    }
    if (sel < 0) {
      why = std::string("select(): ") + std::strerror(errno);
      ::close(fd);
      return -1;
    }
    int soerr = 0;
    socklen_t len = sizeof(soerr);
    ::getsockopt(fd, SOL_SOCKET, SO_ERROR, &soerr, &len);
    rc = (soerr == 0) ? 0 : -1;
    errno = soerr;
  }
  if (rc != 0) {
    why = "connect to " + ip + ":" + std::to_string(port) + " failed: " + std::strerror(errno);
    ::close(fd);
    return -1;
  }
  ::fcntl(fd, F_SETFL, flags);  // ブロッキングに戻す(保持するだけなので実害はないが素直に)
  return fd;
}

class FakeEcc : public get::rc::StateMachine {
 public:
  explicit FakeEcc(bool no_data_link) : no_data_link_(no_data_link) {}
  ~FakeEcc() override { close_links(); }

  void describe(std::string configId, const Ice::Current&) override {
    std::lock_guard<std::mutex> lk(mu_);
    step("describe");
    config_id_ = std::move(configId);
  }
  void prepare(std::string configId, const Ice::Current&) override {
    std::lock_guard<std::mutex> lk(mu_);
    step("prepare");
    config_id_ = std::move(configId);
  }
  void configure(std::string configId, std::string dataRouter, const Ice::Current&) override {
    std::lock_guard<std::mutex> lk(mu_);
    // **036**: 実 ECC の BackEnd::configure は `if (state == ST_PREPARED)` ガード付きで、
    // Prepared 以外では**何も起きない**(BackEnd.cpp:955-962)。DataLinkSet も更新されない ——
    // つまり CoBo は**古い宛先のまま**になる。ここで早期 return するのがその再現である
    // (更新してしまうと「歩き戻しを省いても新しいポートに繋がる」嘘の green を作る)。
    if (step("configure") != ecc::Step::Applied) return;
    config_id_ = std::move(configId);
    data_link_set_ = std::move(dataRouter);  // DataLinkSet XML をそのまま保持
  }

  // 実 ECC の daqStart 相当。DataLinkSet の向き先へ **CoBo として** outbound TCP する。
  void start(const Ice::Current&) override {
    std::lock_guard<std::mutex> lk(mu_);
    step("start");  // Ready でなければここで CmdException(順序違反)

    const std::vector<ecc::Link> links = ecc::parse_data_link_set(data_link_set_);
    if (links.empty()) {
      fail_transition(ecc::State::Ready, ecc::Error::WhenStart);
      throw mdaq::utl::CmdException("Could not establish data link. no DataLink in DataLinkSet");
    }
    // --no-data-link: DataLinkSet の中身は検査するが TCP は張らない(冒頭 (d) の理由)。
    if (no_data_link_) {
      for (const ecc::Link& l : links) {
        std::fprintf(stderr, "fake_ecc: data link SKIPPED (--no-data-link): %s -> %s:%d\n",
                     l.sender.c_str(), l.router_ip.c_str(), l.router_port);
      }
      return;
    }
    std::vector<int> opened;
    for (const ecc::Link& l : links) {
      std::string why;
      const int fd = connect_with_timeout(l.router_ip, l.router_port, why);
      if (fd < 0) {
        for (const int f : opened) ::close(f);
        fail_transition(ecc::State::Ready, ecc::Error::WhenStart);  // start は成立しなかった
        std::fprintf(stderr, "fake_ecc: data link failed for %s: %s\n", l.sender.c_str(),
                     why.c_str());
        throw mdaq::utl::CmdException("Could not establish data link. " + why);
      }
      std::fprintf(stderr, "fake_ecc: data link up: %s -> %s:%d\n", l.sender.c_str(),
                   l.router_ip.c_str(), l.router_port);
      opened.push_back(fd);
    }
    // 繋いだだけ。**1 バイトも送らない**(データ送出は graw_replay の担当)。
    links_ = std::move(opened);
  }

  void stop(const Ice::Current&) override {
    std::lock_guard<std::mutex> lk(mu_);
    step("stop");
    close_links();  // 受信側から見ると EOF = run 境界
  }
  void pause(const Ice::Current&) override {
    std::lock_guard<std::mutex> lk(mu_);
    if (state_ != ecc::State::Running) throw invalid("pause");
    state_ = ecc::State::Paused;
  }
  void resume(const Ice::Current&) override {
    std::lock_guard<std::mutex> lk(mu_);
    if (state_ != ecc::State::Paused) throw invalid("resume");
    state_ = ecc::State::Running;
  }
  void breakup(const Ice::Current&) override {
    std::lock_guard<std::mutex> lk(mu_);
    step("breakup");
  }
  void reset(const Ice::Current&) override {
    std::lock_guard<std::mutex> lk(mu_);
    // **036**: 実 ECC の reset は EV_UNDO = **1 段戻すだけ**(BackEnd.cpp:924)。遷移が
    // 無ければ(= Active に居れば)**副作用も含めて完全に無音**。ここで早期 return しないと
    // 「reset は効かないのに DataLinkSet だけ消える」という実機に無い状態を作ってしまう。
    if (step("reset") != ecc::Step::Applied) return;
    close_links();
    data_link_set_.clear();
  }
  void getStatus(get::rc::Status& s, const Ice::Current&) override {
    std::lock_guard<std::mutex> lk(mu_);
    // 読むだけ。**フラグは消さない**(実 ECC も smStatus を読むだけ —— BackEnd.cpp:321-327 /
    // GetEccImpl.cpp:468-473)。次の遷移が渡るまで残り続ける。
    s.s = to_ice(state_);
    s.e = to_ice(error_);
  }
  std::string getDataLinks(const Ice::Current&) override {
    std::lock_guard<std::mutex> lk(mu_);
    return data_link_set_;
  }

 private:
  // ecc_core の遷移表(実 ECC の SM、一次資料の行番号込み)で状態を進める(036)。
  // Applied/Ignored/Denied の出典・意味は ecc_core.hpp の遷移表コメントを見よ
  // (048 でここから一元化)。ここでは呼び出し箇所固有の事実だけ:
  //   Ignored でも**呼び手には ok を返す**(実機準拠)。controller はこれを
  //   「状態が動かなかった」ことで検出する(src/controller.rs の歩き戻し)。
  //   Ignored は**この fake のログには出す**(実機は完全な無音。ログは Ice の外なので
  //   controller からは見えない — テストが落ちたときの原因追跡用)。
  ecc::Step step(const char* action) {
    ecc::State next = ecc::State::Unknown;
    std::string err;
    const ecc::Step result = ecc::next_state(state_, action, next, err);
    if (result == ecc::Step::Denied) {
      std::fprintf(stderr, "fake_ecc: %s\n", err.c_str());
      throw mdaq::utl::CmdException(err);
    }
    if (result == ecc::Step::Ignored) {
      std::fprintf(stderr,
                   "fake_ecc: %s IGNORED in state %s (real ECC: no such transition = silence)\n",
                   action, ecc::to_string(state_));
      return result;
    }
    // エラーフラグの set/clear 規則の出典は ecc_core.hpp の規則ブロックを見よ(043)。
    // ここは clear 側の実装箇所 —— 副作用より先に、遷移を渡り始めた時点で消す。
    error_ = ecc::Error::NoErr;
    state_ = next;
    return result;
  }

  // 遷移のアクションが失敗したときの後始末。set 規則・state 巻き戻しの出典は
  // ecc_core.hpp の規則ブロックを見よ(043)。ここはその実装箇所 —— フラグにその相の
  // コードを立て、state を渡り始める前へ戻す。
  void fail_transition(ecc::State back_to, ecc::Error code) {
    state_ = back_to;
    error_ = code;
  }
  mdaq::utl::CmdException invalid(const char* action) const {
    return mdaq::utl::CmdException("invalid transition: " + std::string(action) + " in state " +
                                   ecc::to_string(state_));
  }
  void close_links() {
    for (const int fd : links_) ::close(fd);
    links_.clear();
  }
  static get::rc::Error to_ice(ecc::Error e) {
    switch (e) {
      case ecc::Error::NoErr: return get::rc::Error::NoErr;
      case ecc::Error::WhenDescribe: return get::rc::Error::WhenDescribe;
      case ecc::Error::WhenPrepare: return get::rc::Error::WhenPrepare;
      case ecc::Error::WhenConfigure: return get::rc::Error::WhenConfigure;
      case ecc::Error::WhenStart: return get::rc::Error::WhenStart;
      case ecc::Error::WhenStop: return get::rc::Error::WhenStop;
      case ecc::Error::WhenPause: return get::rc::Error::WhenPause;
      case ecc::Error::WhenResume: return get::rc::Error::WhenResume;
      case ecc::Error::WhenBreakup: return get::rc::Error::WhenBreakup;
      case ecc::Error::WhenReset: return get::rc::Error::WhenReset;
      // Unknown は .ice の enum に無い(この fake は Unknown を作らない)。
      default: return get::rc::Error::NoErr;
    }
  }

  static get::rc::State to_ice(ecc::State s) {
    switch (s) {
      case ecc::State::Off: return get::rc::State::Off;
      case ecc::State::Idle: return get::rc::State::Idle;
      case ecc::State::Described: return get::rc::State::Described;
      case ecc::State::Prepared: return get::rc::State::Prepared;
      case ecc::State::Ready: return get::rc::State::Ready;
      case ecc::State::Running: return get::rc::State::Running;
      case ecc::State::Paused: return get::rc::State::Paused;
      // Unknown は .ice の enum に無い。遷移表が Unknown を作らないので到達しないが、
      // 万一来たら Off に落とす(「実は Paused でした」と嘘をつく方が悪い)。
      default: return get::rc::State::Off;
    }
  }

  std::mutex mu_;
  ecc::State state_ = ecc::State::Off;
  // 実 ECC の初期値も NO_ERR(BackEnd::BackEnd()、BackEnd.cpp:132)。
  ecc::Error error_ = ecc::Error::NoErr;
  std::string config_id_;
  std::string data_link_set_;
  std::vector<int> links_;  // start() で張った(そして黙って保持する)接続
  bool no_data_link_ = false;
};

void usage() {
  std::printf(
      "usage: fake_ecc [--host IP] [--port N] [--identity NAME] [--no-data-link]\n"
      "\n"
      "  --host IP        endpoint address (default 127.0.0.1)\n"
      "  --port N         endpoint port, 0 = pick a free one (default 0)\n"
      "  --identity NAME  Ice object identity (default Ecc)\n"
      "  --no-data-link   start() does not open (nor hold) any TCP connection to the\n"
      "                   DataLinkSet targets. Use it when something else already plays\n"
      "                   the CoBo and streams the data (graw_replay in the P3 e2e):\n"
      "                   a receiver serves one connection at a time, so a held probe\n"
      "                   connection would starve the real data stream. Default: off,\n"
      "                   i.e. links are opened (listen-before-start negative test).\n"
      "\n"
      "Prints one line \"PROXY <stringified proxy>\" on stdout once the adapter is\n"
      "active; feed it to ecc_bridge --ecc-proxy. Runs until SIGINT/SIGTERM.\n");
}

}  // namespace

int main(int argc, char** argv) {
  std::string host = "127.0.0.1";
  std::string port = "0";
  std::string identity = "Ecc";
  bool no_data_link = false;
  for (int i = 1; i < argc; ++i) {
    const std::string a = argv[i];
    const bool has_value = (i + 1 < argc);
    if (a == "--help" || a == "-h") {
      usage();
      return 0;
    }
    if (a == "--host" && has_value) {
      host = argv[++i];
    } else if (a == "--port" && has_value) {
      port = argv[++i];
    } else if (a == "--identity" && has_value) {
      identity = argv[++i];
    } else if (a == "--no-data-link") {
      no_data_link = true;
    } else {
      std::fprintf(stderr, "fake_ecc: bad argument '%s'\n", a.c_str());
      usage();
      return 2;
    }
  }

  std::signal(SIGINT, on_signal);
  std::signal(SIGTERM, on_signal);

  Ice::CommunicatorPtr comm;
  try {
    comm = Ice::initialize();
    auto adapter = comm->createObjectAdapterWithEndpoints(
        "FakeEcc", "tcp -h " + host + " -p " + port);
    auto servant = std::make_shared<FakeEcc>(no_data_link);
    auto prx = adapter->add(servant, Ice::stringToIdentity(identity));
    adapter->activate();
    std::printf("PROXY %s\n", prx->ice_toString().c_str());
    std::fflush(stdout);
  } catch (const std::exception& e) {
    std::fprintf(stderr, "fake_ecc: startup failed: %s\n", e.what());
    if (comm) {
      try {
        comm->destroy();
      } catch (...) {
      }
    }
    return 1;
  }

  while (g_stop == 0) std::this_thread::sleep_for(std::chrono::milliseconds(50));

  std::fprintf(stderr, "fake_ecc: shutting down\n");
  try {
    comm->destroy();
  } catch (...) {
  }
  return 0;
}
