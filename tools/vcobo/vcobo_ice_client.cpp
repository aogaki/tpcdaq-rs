// vcobo_ice_client — 走っている vcobo_daq を **encoding 1.0** で機械照合するテストクライアント。
//
//   ./vcobo_ice_client --host 127.0.0.1 --ctrl-port 47001 --daq-port 47004 \
//                      --graw FIXTURE.graw [--data-port 47005]
//
// TODO/040 の受け入れ ②③④⑤ をこの 1 本で満たす:
//   ② 46001: create → 魔法値 read → powerON write → alarm read
//      46004: connect → start → stop → start → stop → stop(冪等)→ disconnect
//   ③ リプレイ出力がソース .graw と**バイト一致**
//   ④ stop → start でリンクが維持される(同じソケットで 2 回目の run が届く)
//   ⑤ 背圧でブロックするだけでドロップしない
//
// 実 ECC がハードノードへ使うプロキシ形式と同じ `"<Identity> -e 1.0 :default -h IP -p PORT"`
// を使う(MDaq/src/mdaq/EccBackEnd.cpp:59-65、HardwareNode.cpp:133-139)。
#include <Ice/Ice.h>
#include <arpa/inet.h>
#include <netinet/in.h>
#include <sys/socket.h>
#include <unistd.h>

#include <atomic>
#include <chrono>
#include <cstring>
#include <string>
#include <thread>
#include <vector>

#include "check.hpp"
#include "get/cobo/CtrlNode.h"
#include "get/mt/AlarmService.h"
#include "mdaq/DaqControl.h"
#include "mdaq/hw/Control.h"

namespace {

// ---------------------------------------------------------------------------
// テスト用の最小ノード記述 —— 実 xcfg(hardwareDescription_fullCoBoStandAlone)の
// 該当部分だけを写したもの。ここに書いた offset/width がそのまま期待値の根拠になる。
// ---------------------------------------------------------------------------
mdaq::hw::FieldDescription field(const std::string& name, int offset, int width) {
  mdaq::hw::FieldDescription f;
  f.name = name;
  f.offset = ::Ice::Short(offset);
  f.width = ::Ice::Short(width);
  f.readOnly = false;
  return f;
}

mdaq::hw::RegisterConfig reg(const std::string& name, int offset,
                             const std::vector<mdaq::hw::FieldDescription>& fields) {
  mdaq::hw::RegisterConfig r;
  r.descr.name = name;
  r.descr.offset = offset;
  r.fields = fields;
  return r;
}

mdaq::hw::NodeConfig make_node_config() {
  mdaq::hw::NodeConfig cfg;
  cfg.id = "HardwareNode@127.0.0.1:47001";

  mdaq::hw::DeviceConfig ctrl;
  ctrl.descr.name = "ctrl";
  ctrl.descr.baseAddress = 0x80000000;
  ctrl.descr.registerAccess = "MemBus";
  ctrl.descr.registerWidth = 4;
  ctrl.registers.push_back(reg("asadConnection", 0x4,
                               {field("PLG", 0, 4), field("PLG0", 0, 1), field("PLG1", 1, 1),
                                field("PLG2", 2, 1), field("PLG3", 3, 1)}));
  ctrl.registers.push_back(reg("asadEnable", 0x8,
                               {field("powerON", 0, 4), field("powerON0", 0, 1),
                                field("powerON1", 1, 1), field("powerON2", 2, 1),
                                field("powerON3", 3, 1)}));
  ctrl.registers.push_back(reg("asadStatus", 0xC, {field("alarm", 0, 4)}));
  ctrl.registers.push_back(reg("mutantConfig", 0x40, {field("mode", 8, 12)}));
  cfg.devices.push_back(ctrl);

  mdaq::hw::DeviceConfig asad;
  asad.descr.name = "asad";
  asad.descr.baseAddress = 0x0;
  asad.descr.registerAccess = "AsAdBus";
  asad.descr.registerWidth = 1;
  asad.registers.push_back(reg("monitorID", 0x0, {}));
  asad.registers.push_back(reg("VDD", 0x8, {}));
  cfg.devices.push_back(asad);

  mdaq::hw::DeviceConfig aget;
  aget.descr.name = "aget";
  aget.descr.baseAddress = 0x0;
  aget.descr.registerAccess = "AGetBus";
  aget.descr.registerWidth = 4;
  aget.registers.push_back(reg("reg5", 0x5, {}));
  cfg.devices.push_back(aget);

  return cfg;
}

// ---------------------------------------------------------------------------
// データリンクの受け側(receiver の代わり)。
// ---------------------------------------------------------------------------
class Receiver {
 public:
  /// ephemeral port で listen して実際の port を返す。0 なら失敗。
  int listen_any(const std::string& host) {
    listen_fd_ = ::socket(AF_INET, SOCK_STREAM, 0);
    if (listen_fd_ < 0) return 0;
    int on = 1;
    ::setsockopt(listen_fd_, SOL_SOCKET, SO_REUSEADDR, &on, sizeof(on));
    sockaddr_in addr;
    std::memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_port = 0;
    if (::inet_pton(AF_INET, host.c_str(), &addr.sin_addr) != 1) return 0;
    if (::bind(listen_fd_, reinterpret_cast<sockaddr*>(&addr), sizeof(addr)) != 0) return 0;
    if (::listen(listen_fd_, 4) != 0) return 0;
    socklen_t len = sizeof(addr);
    if (::getsockname(listen_fd_, reinterpret_cast<sockaddr*>(&addr), &len) != 0) return 0;
    return ntohs(addr.sin_port);
  }

  /// accept を別スレッドで待つ。listen-before-start の再現。
  void accept_async() {
    accepted_ = false;
    th_ = std::thread([this] {
      const int fd = ::accept(listen_fd_, nullptr, nullptr);
      if (fd >= 0) {
        data_fd_ = fd;
        accepted_ = true;
      }
    });
  }

  bool wait_accepted(int ms) {
    if (th_.joinable()) th_.join();
    (void)ms;
    return accepted_;
  }

  /// `want` バイト読むまでブロックする。EOF なら読めた分で返す。
  size_t read_exactly(std::vector<uint8_t>& out, size_t want, int timeout_ms) {
    const auto deadline = std::chrono::steady_clock::now() + std::chrono::milliseconds(timeout_ms);
    uint8_t buf[64 * 1024];
    while (out.size() < want && std::chrono::steady_clock::now() < deadline) {
      const ssize_t n = recv_with_timeout(buf, sizeof(buf), 200);
      if (n == 0) break;  // EOF
      if (n > 0) out.insert(out.end(), buf, buf + n);
    }
    return out.size();
  }

  /// 最初の 1 バイトが来るまで待って、来た分だけ返す(-1 = timeout / 0 = EOF)。
  ssize_t read_some(std::vector<uint8_t>& out, int timeout_ms) {
    const auto deadline = std::chrono::steady_clock::now() + std::chrono::milliseconds(timeout_ms);
    uint8_t buf[64 * 1024];
    while (std::chrono::steady_clock::now() < deadline) {
      const ssize_t n = recv_with_timeout(buf, sizeof(buf), 100);
      if (n == 0) return 0;
      if (n > 0) {
        out.insert(out.end(), buf, buf + n);
        return n;
      }
    }
    return -1;
  }

  /// EOF(正常 FIN)が来るか確かめる。
  bool wait_eof(int timeout_ms) {
    const auto deadline = std::chrono::steady_clock::now() + std::chrono::milliseconds(timeout_ms);
    uint8_t buf[4096];
    while (std::chrono::steady_clock::now() < deadline) {
      const ssize_t n = recv_with_timeout(buf, sizeof(buf), 200);
      if (n == 0) return true;
    }
    return false;
  }

  void close_all() {
    if (data_fd_ >= 0) {
      ::close(data_fd_);
      data_fd_ = -1;
    }
    if (listen_fd_ >= 0) {
      ::close(listen_fd_);
      listen_fd_ = -1;
    }
  }

 private:
  ssize_t recv_with_timeout(uint8_t* buf, size_t len, int ms) {
    timeval tv;
    tv.tv_sec = ms / 1000;
    tv.tv_usec = (ms % 1000) * 1000;
    ::setsockopt(data_fd_, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));
    const ssize_t n = ::recv(data_fd_, buf, len, 0);
    if (n < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) return -1;
    return n;
  }

  int listen_fd_ = -1;
  int data_fd_ = -1;
  std::thread th_;
  std::atomic<bool> accepted_{false};
};

std::vector<uint8_t> read_file_bytes(const std::string& path) {
  std::vector<uint8_t> out;
  std::FILE* f = std::fopen(path.c_str(), "rb");
  if (f == nullptr) return out;
  uint8_t buf[64 * 1024];
  size_t n = 0;
  while ((n = std::fread(buf, 1, sizeof(buf), f)) > 0) out.insert(out.end(), buf, buf + n);
  std::fclose(f);
  return out;
}

std::string proxy_string(const std::string& identity, const std::string& host, int port) {
  // 実 ECC がハードノードへ使う形式をそのまま真似る(**encoding 1.0**)。
  return identity + " -e 1.0 :default -h " + host + " -p " + std::to_string(port);
}

// ---------------------------------------------------------------------------
// 46001 — HwNode / Device / AlarmService
// ---------------------------------------------------------------------------
void test_hw_node(const Ice::CommunicatorPtr& ic, const std::string& host, int port) {
  Ice::ObjectPrx base = ic->stringToProxy(proxy_string("HwNode", host, port));
  // ECC は checkedCast(= ice_isA)でハードノードを掴む。CtrlNode 継承ツリーが立つこと。
  get::cobo::CtrlNodePrx ctrl_node = get::cobo::CtrlNodePrx::checkedCast(base);
  CHECK(!!ctrl_node);
  if (!ctrl_node) return;

  // encoding 1.0 で喋っていることをプロキシ側からも確認する。
  CHECK_EQ(ctrl_node->ice_getEncodingVersion().major, 1);
  CHECK_EQ(ctrl_node->ice_getEncodingVersion().minor, 0);

  // mdaq::hw::Node / LedManager / AsAdPulserMgr としても ice_isA が立つ。
  CHECK(!!mdaq::hw::NodePrx::checkedCast(base));
  CHECK(!!get::cobo::LedManagerPrx::checkedCast(base));
  CHECK(!!get::cobo::AsAdPulserMgrPrx::checkedCast(base));

  // --- describe シーケンス ---
  ctrl_node->destroy();  // 前の run の残骸を捨てる(ECC も describe 毎に destroy する)
  const mdaq::hw::NodeConfig cfg = make_node_config();
  ctrl_node->create(cfg);

  mdaq::hw::DeviceMap devices;
  ctrl_node->getMapOfDevices(devices);
  CHECK_EQ(devices.size(), 3u);
  CHECK(devices.count("ctrl") == 1);
  CHECK(devices.count("asad") == 1);
  CHECK(devices.count("aget") == 1);
  if (devices.size() != 3) return;

  ctrl_node->setName("CoBo[0]");
  CHECK_STR(ctrl_node->name(), "CoBo[0]");

  mdaq::hw::DevicePrx ctrl = devices["ctrl"];
  mdaq::hw::DevicePrx asad = devices["asad"];
  mdaq::hw::DevicePrx aget = devices["aget"];

  // --- 魔法値(039 実測の必須 4 種)---
  CHECK_EQ(asad->readRegister("monitorID"), 0x41);  // 'A'
  CHECK_EQ(asad->readRegister("VDD"), 0xCD);
  CHECK_EQ(aget->readRegister("reg5"), 0x201);
  for (int i = 0; i < 4; ++i) {
    CHECK_EQ(ctrl->readField("asadConnection", "PLG" + std::to_string(i)), 1);
  }

  // --- alarm 意味論: alarm ビット i = !powerON{i} ---
  // 起動直後は全 AsAd power off → alarm = 0b1111 = 0xF
  CHECK_EQ(ctrl->readField("asadStatus", "alarm"), 0xF);
  // AsAd2 だけ power on → 0b1011 = 0xB(非対称: 0 番ではなく 2 番)
  ctrl->writeField("asadEnable", "powerON2", 1);
  CHECK_EQ(ctrl->readField("asadStatus", "alarm"), 0xB);
  // AsAd0 も power on → 0b1010 = 0xA
  ctrl->writeField("asadEnable", "powerON0", 1);
  CHECK_EQ(ctrl->readField("asadStatus", "alarm"), 0xA);
  // AsAd2 を落とす → 0b1110 = 0xE
  ctrl->writeField("asadEnable", "powerON2", 0);
  CHECK_EQ(ctrl->readField("asadStatus", "alarm"), 0xE);
  // レジスタ丸ごとでも追従(powerON=0xF → alarm=0)
  ctrl->writeRegister("asadEnable", 0xF);
  CHECK_EQ(ctrl->readField("asadStatus", "alarm"), 0x0);
  // 元に戻す
  ctrl->writeRegister("asadEnable", 0x0);
  CHECK_EQ(ctrl->readField("asadStatus", "alarm"), 0xF);

  // --- ビットフィールドの抽出・挿入 ---
  ctrl->writeRegister("mutantConfig", 0xFF0000FF);
  ctrl->writeField("mutantConfig", "mode", 0x456);
  // mode = bits[19:8] → 0xFF0000FF の該当ビットを 0x456 に → 0xFF0456FF
  CHECK_EQ(ctrl->readRegister("mutantConfig"), 0xFF0456FF);
  CHECK_EQ(ctrl->readField("mutantConfig", "mode"), 0x456);

  // --- 未知の名前: read は 0、write は受理して格納 ---
  CHECK_EQ(ctrl->readRegister("noSuchRegister"), 0);
  ctrl->writeRegister("brandNewRegister", 0x5A5A);
  CHECK_EQ(ctrl->readRegister("brandNewRegister"), 0x5A5A);

  // --- 記述の参照 ---
  CHECK_STR(ctrl->registerAccess(), "MemBus");
  CHECK_EQ(ctrl->baseAddress(), 0x80000000LL);
  CHECK_EQ(ctrl->registerWidth(), 4);
  mdaq::hw::FieldList fields;
  ctrl->getListOfFields("asadEnable", fields);
  CHECK_EQ(fields.size(), 5u);  // powerON + powerON0..3

  // --- execBatch は実装しない(batchProcessing=false 固定)---
  bool threw = false;
  try {
    mdaq::hw::RegCmdSeq in;
    mdaq::hw::RegCmdSeq out;
    ctrl->execBatch(in, out);
  } catch (const mdaq::utl::CmdException&) {
    threw = true;
  }
  CHECK(threw);

  // --- AlarmService(受理するだけ)---
  get::mt::AlarmServicePrx alarm = ctrl_node->getAlarmService();
  CHECK(!!alarm);
  if (alarm) {
    alarm->reset();
    alarm->subscribe("ECC", get::mt::AlarmCallbackPrx());
    alarm->unsubscribe("ECC");
  }

  // --- LedManager / AsAdPulserMgr は no-op(例外を投げない)---
  ctrl_node->setLEDs(true);
  ctrl_node->modifyLED(get::cobo::LedP, 0, get::cobo::LedOn);
  ctrl_node->stopPeriodicPulser();
  ctrl_node->setAsAdAlarmMonitoringEnabled(true);

  // --- destroy でデバイスが消え、再 create で魔法値がまた乗る(039: シードは毎回死ぬ)---
  ctrl_node->destroy();
  mdaq::hw::DeviceList after;
  ctrl_node->getListOfDevices(after);
  CHECK_EQ(after.size(), 0u);
  ctrl_node->create(cfg);
  mdaq::hw::DeviceMap again;
  ctrl_node->getMapOfDevices(again);
  CHECK_EQ(again["asad"]->readRegister("monitorID"), 0x41);
  CHECK_EQ(again["ctrl"]->readField("asadStatus", "alarm"), 0xF);
}

// ---------------------------------------------------------------------------
// 46004 — DaqCtrlNode + データリンク
// ---------------------------------------------------------------------------
void test_daq_ctrl_node(const Ice::CommunicatorPtr& ic, const std::string& host, int port,
                        const std::string& graw_path) {
  Ice::ObjectPrx base = ic->stringToProxy(proxy_string("DaqCtrlNode", host, port));
  mdaq::DaqCtrlNodePrx daq = mdaq::DaqCtrlNodePrx::checkedCast(base);
  CHECK(!!daq);
  if (!daq) return;
  CHECK_EQ(daq->ice_getEncodingVersion().major, 1);
  CHECK_EQ(daq->ice_getEncodingVersion().minor, 0);

  const std::vector<uint8_t> source = read_file_bytes(graw_path);
  CHECK(!source.empty());
  if (source.empty()) return;

  // --- ⑩ dataRouterType は "TCP" 完全一致のみ ---
  bool threw = false;
  try {
    daq->connect("Tcp", "127.0.0.1:1");
  } catch (const mdaq::utl::CmdException&) {
    threw = true;
  }
  CHECK(threw);
  threw = false;
  try {
    daq->connect("FDT", "127.0.0.1:1");
  } catch (const mdaq::utl::CmdException&) {
    threw = true;
  }
  CHECK(threw);

  // --- 接続先が居なければ例外(リトライを勝手に足さない)---
  threw = false;
  try {
    daq->connect("TCP", "127.0.0.1:9");  // discard port: 誰も listen していない
  } catch (const mdaq::utl::CmdException&) {
    threw = true;
  }
  CHECK(threw);

  // --- listen-before-start: 受け側を先に立てる ---
  Receiver rx;
  const int data_port = rx.listen_any(host);
  CHECK(data_port > 0);
  if (data_port <= 0) return;
  rx.accept_async();

  daq->connect("TCP", host + ":" + std::to_string(data_port));
  CHECK(rx.wait_accepted(2000));

  daq->setCircularBuffersEnabled(0x1);
  daq->setAlwaysFlushData(false);

  // --- ③ run 1: リプレイ出力がソース .graw とバイト一致 ---
  daq->daqStart();
  std::vector<uint8_t> run1;
  rx.read_exactly(run1, source.size(), 30000);
  daq->daqStop();
  CHECK_EQ(run1.size(), source.size());
  CHECK(run1 == source);

  // --- ④ stop → start でリンクが維持される(同じソケットに 2 回目の run が届く)---
  daq->daqStart();
  std::vector<uint8_t> run2;
  rx.read_exactly(run2, source.size(), 30000);
  daq->daqStop();
  CHECK_EQ(run2.size(), source.size());
  CHECK(run2 == source);

  // --- daqStop は冪等(breakup で 2 度目が来る)---
  daq->daqStop();
  daq->daqStop();

  // --- ② 再 connect は旧センダを自分で破棄する(disconnect 無しで来る)---
  Receiver rx2;
  const int data_port2 = rx2.listen_any(host);
  CHECK(data_port2 > 0);
  rx2.accept_async();
  daq->connect("TCP", host + ":" + std::to_string(data_port2));
  CHECK(rx2.wait_accepted(2000));
  // 旧リンクは CoBo 側から閉じられている(正常 FIN)。
  CHECK(rx.wait_eof(2000));

  // --- ⑤ 背圧: 受け側が読まない間、送信はブロックするだけでドロップしない ---
  daq->daqStart();
  std::this_thread::sleep_for(std::chrono::milliseconds(1500));  // わざと読まない
  std::vector<uint8_t> run3;
  rx2.read_exactly(run3, source.size(), 30000);
  daq->daqStop();
  CHECK_EQ(run3.size(), source.size());
  CHECK(run3 == source);

  // --- ⑤ disconnect は shutdown+close(正常 FIN)---
  daq->disconnect();
  CHECK(rx2.wait_eof(2000));

  // disconnect は冪等(リンク無しでも例外を投げない)
  daq->disconnect();

  // リンク無しの daqStart は黙って成功せずエラーになる(silent failure 禁止)
  threw = false;
  try {
    daq->daqStart();
  } catch (const mdaq::utl::CmdException&) {
    threw = true;
  }
  CHECK(threw);

  rx.close_all();
  rx2.close_all();
}

// ---------------------------------------------------------------------------
// バイト一致リプレイだけを 1 パスで見るモード(`--mode replay`)。
// 実 .graw(TPCDAQ_REAL_GRAW_DIR)の任意回帰で使う —— 大きいファイルを 3 周しない。
// ---------------------------------------------------------------------------
void test_replay_only(const Ice::CommunicatorPtr& ic, const std::string& host, int port,
                      const std::string& graw_path) {
  Ice::ObjectPrx base = ic->stringToProxy(proxy_string("DaqCtrlNode", host, port));
  mdaq::DaqCtrlNodePrx daq = mdaq::DaqCtrlNodePrx::checkedCast(base);
  CHECK(!!daq);
  if (!daq) return;

  const std::vector<uint8_t> source = read_file_bytes(graw_path);
  CHECK(!source.empty());
  if (source.empty()) return;

  Receiver rx;
  const int data_port = rx.listen_any(host);
  CHECK(data_port > 0);
  if (data_port <= 0) return;
  rx.accept_async();
  daq->connect("TCP", host + ":" + std::to_string(data_port));
  CHECK(rx.wait_accepted(2000));

  daq->daqStart();
  std::vector<uint8_t> got;
  rx.read_exactly(got, source.size(), 300000);
  daq->daqStop();
  daq->disconnect();
  rx.close_all();

  CHECK_EQ(got.size(), source.size());
  CHECK(got == source);
  std::printf("  replayed %zu bytes from %s\n", got.size(), graw_path.c_str());
}

// ---------------------------------------------------------------------------
// ⑥ 3 秒端数 flush —— 新しいフレームが flush_bytes に届かなくても、ハートビートで
// バッファの端数が吐き出されること。`--mode heartbeat` で走る別インスタンス相手に
// 使う(flush_bytes を巨大に、rate_hz を低くした設定が要る)。
// ---------------------------------------------------------------------------
void test_heartbeat_flush(const Ice::CommunicatorPtr& ic, const std::string& host, int port,
                          int heartbeat_ms) {
  Ice::ObjectPrx base = ic->stringToProxy(proxy_string("DaqCtrlNode", host, port));
  mdaq::DaqCtrlNodePrx daq = mdaq::DaqCtrlNodePrx::checkedCast(base);
  CHECK(!!daq);
  if (!daq) return;

  Receiver rx;
  const int data_port = rx.listen_any(host);
  CHECK(data_port > 0);
  if (data_port <= 0) return;
  rx.accept_async();
  daq->connect("TCP", host + ":" + std::to_string(data_port));
  CHECK(rx.wait_accepted(2000));

  const auto t0 = std::chrono::steady_clock::now();
  daq->daqStart();
  std::vector<uint8_t> first;
  const ssize_t n = rx.read_some(first, heartbeat_ms * 3);
  const auto elapsed_ms =
      std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now() - t0)
          .count();
  daq->daqStop();
  daq->disconnect();
  rx.close_all();

  CHECK(n > 0);
  // flush_bytes に届いていないので、最初の 1 バイトはハートビートで出てくる。
  // 下限はハートビートの 2/3、上限は 2 倍(タイマ精度と OS のばらつきの余裕)。
  CHECK(elapsed_ms >= heartbeat_ms * 2 / 3);
  CHECK(elapsed_ms <= heartbeat_ms * 2);
  if (elapsed_ms < heartbeat_ms * 2 / 3 || elapsed_ms > heartbeat_ms * 2) {
    std::printf("  (heartbeat flush arrived after %lld ms, expected ~%d ms)\n",
                static_cast<long long>(elapsed_ms), heartbeat_ms);
  }
}

}  // namespace

int main(int argc, char* argv[]) {
  std::string host = "127.0.0.1";
  int ctrl_port = 46001;
  int daq_port = 46004;
  std::string graw;
  std::string mode = "full";
  int heartbeat_ms = 3000;

  for (int i = 1; i < argc; ++i) {
    const std::string a = argv[i];
    const bool has_value = (i + 1 < argc);
    if (a == "--host" && has_value) {
      host = argv[++i];
    } else if (a == "--ctrl-port" && has_value) {
      ctrl_port = std::atoi(argv[++i]);
    } else if (a == "--daq-port" && has_value) {
      daq_port = std::atoi(argv[++i]);
    } else if (a == "--graw" && has_value) {
      graw = argv[++i];
    } else if (a == "--mode" && has_value) {
      mode = argv[++i];
    } else if (a == "--heartbeat-ms" && has_value) {
      heartbeat_ms = std::atoi(argv[++i]);
    } else {
      std::fprintf(stderr,
                   "usage: vcobo_ice_client [--host H] [--ctrl-port N] [--daq-port N] "
                   "[--mode full|heartbeat] [--heartbeat-ms N] --graw FILE\n");
      return 2;
    }
  }
  if ((mode == "full" || mode == "replay") && graw.empty()) {
    std::fprintf(stderr, "--graw FILE is required in --mode %s\n", mode.c_str());
    return 2;
  }

  Ice::CommunicatorPtr ic;
  try {
    Ice::InitializationData init;
    init.properties = Ice::createProperties();
    init.properties->setProperty("Ice.IPv6", "0");
    ic = Ice::initialize(init);
    if (mode == "heartbeat") {
      test_heartbeat_flush(ic, host, daq_port, heartbeat_ms);
    } else if (mode == "replay") {
      test_replay_only(ic, host, daq_port, graw);
    } else {
      test_hw_node(ic, host, ctrl_port);
      test_daq_ctrl_node(ic, host, daq_port, graw);
    }
  } catch (const Ice::Exception& e) {
    std::ostringstream os;
    os << e;
    std::printf("FAIL Ice exception: %s\n", os.str().c_str());
    ++tpccheck::g_fail;
  }
  if (ic) ic->destroy();
  return tpccheck::report(("vcobo_ice_client(" + mode + ")").c_str());
}
