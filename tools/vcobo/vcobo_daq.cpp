// vcobo_daq — 仮想 zCoBo(CoBo を板ごと偽る唯一の自作プロセス)。
//
//   [実 getEccServer] --Ice(-e 1.0)--> vcobo_daq
//        port 46001: HwNode(get::cobo::CtrlNode)/ AlarmService / Device 群
//        port 46004: DaqCtrlNode(connect/disconnect/daqStart/daqStop/setCircularBuffersEnabled)
//        graw リプレイ --plain TCP--> receiver
//
// 仕様の正: docs/VIRTUAL_ZCOBO_ja.md v1.1(§4.2 最小 Ice 面・魔法値表 / §4.3 データリンク
// 挙動 11 項 / §4.5 46004 の実測契約と alarm 意味論)。発注書 = TODO/040。
//
// **encoding 1.0 のクライアント(実 ECC)を受ける**。ECC はハードノードへの全プロキシに
// `-e 1.0` を焼き込む(MDaq/src/mdaq/EccBackEnd.cpp:59-65)。servant 側は Ice の仕様で
// 1.0/1.1 の両方を受けられるが、それを機械照合するテストを別に持つ(vcobo_ice_client)。
//
// ビルド: Ice **3.7 keg**(ice@3.7、unlinked)。3.7/3.8 の runtime を 1 プロセスに
// 混ぜると無言 SIGSEGV(docs §4.1)。Makefile の ICE_HOME を参照。
#include <Ice/Ice.h>

#include <chrono>
#include <csignal>
#include <fstream>
#include <sstream>
#include <string>
#include <thread>
#include <vector>

#include "get/cobo/CtrlNode.h"
#include "get/mt/AlarmService.h"
#include "mdaq/DaqControl.h"
#include "mdaq/hw/Control.h"
#include "vcobo_core.hpp"
#include "vcobo_link.hpp"

namespace {

using vcobo::log_error;
using vcobo::log_info;
using vcobo::log_warn;

// SIGINT/SIGTERM graceful 化(TODO/047-B)。既存イディオムの移植
// (tools/ecc_bridge/ecc_bridge.cpp:38-39 / fake_ecc.cpp:55-56,377)。
// ハンドラは g_stop を立てるだけ —— Ice API はここから呼ばない。
volatile std::sig_atomic_t g_stop = 0;
void on_signal(int) { g_stop = 1; }

/// レジスタモデルと、それを守る 1 本の mutex。Ice のディスパッチは複数スレッドから来る。
struct NodeState {
  std::mutex m;
  vcobo::Node node;
};

mdaq::utl::CmdException cmd_error(const std::string& reason) {
  mdaq::utl::CmdException e;
  e.reason = reason;
  return e;
}

// ---------------------------------------------------------------------------
// Device servant(identity = デバイス名。create で動的に生成される)
// ---------------------------------------------------------------------------
class DeviceI : public mdaq::hw::Device {
 public:
  DeviceI(NodeState* state, const vcobo::DeviceInfo& info) : state_(state), info_(info) {}

  ::std::string name(const Ice::Current&) override { return info_.name; }
  ::Ice::Long baseAddress(const Ice::Current&) override { return info_.base_address; }
  ::std::string registerAccess(const Ice::Current&) override { return info_.register_access; }
  ::Ice::Short registerWidth(const Ice::Current&) override {
    return ::Ice::Short(info_.register_width);
  }

  // --- ECC が実際に使う 4 op ---------------------------------------------

  ::Ice::Long readRegister(const ::std::string& reg, const Ice::Current&) override {
    std::lock_guard<std::mutex> lk(state_->m);
    return ::Ice::Long(state_->node.read_register(info_.name, reg));
  }

  ::Ice::Long readField(const ::std::string& reg, const ::std::string& field,
                        const Ice::Current&) override {
    std::lock_guard<std::mutex> lk(state_->m);
    return ::Ice::Long(state_->node.read_field(info_.name, reg, field));
  }

  void writeRegister(const ::std::string& reg, ::Ice::Long value, const Ice::Current&) override {
    std::lock_guard<std::mutex> lk(state_->m);
    state_->node.write_register(info_.name, reg, uint64_t(value));
  }

  void writeField(const ::std::string& reg, const ::std::string& field, ::Ice::Long value,
                  const Ice::Current&) override {
    std::lock_guard<std::mutex> lk(state_->m);
    state_->node.write_field(info_.name, reg, field, uint64_t(value));
  }

  // --- 記述の追加・参照 ---------------------------------------------------

  void addRegister(const mdaq::hw::RegisterConfig& cfg, const Ice::Current&) override {
    std::lock_guard<std::mutex> lk(state_->m);
    define(cfg);
  }

  void addRegisters(const mdaq::hw::RegisterConfigList& cfg, const Ice::Current&) override {
    std::lock_guard<std::mutex> lk(state_->m);
    for (const mdaq::hw::RegisterConfig& c : cfg) define(c);
  }

  void getDeviceDescription(mdaq::hw::DeviceDescription& out, const Ice::Current&) override {
    out.name = info_.name;
    out.baseAddress = info_.base_address;
    out.registerAccess = info_.register_access;
    out.registerWidth = ::Ice::Short(info_.register_width);
  }

  void getDeviceConfig(mdaq::hw::DeviceConfig& out, const Ice::Current& c) override {
    getDeviceDescription(out.descr, c);
    std::lock_guard<std::mutex> lk(state_->m);
    for (const std::string& reg : state_->node.register_names(info_.name)) {
      mdaq::hw::RegisterConfig rc;
      rc.descr.name = reg;
      rc.descr.offset = 0;
      out.registers.push_back(rc);
    }
  }

  void getListOfRegisters(mdaq::hw::RegisterList& out, const Ice::Current&) override {
    std::lock_guard<std::mutex> lk(state_->m);
    out = state_->node.register_names(info_.name);
  }

  void getListOfFields(const ::std::string& reg, mdaq::hw::FieldList& out,
                       const Ice::Current&) override {
    std::lock_guard<std::mutex> lk(state_->m);
    out = state_->node.field_names(info_.name, reg);
  }

  void getRegisterDescription(const ::std::string& reg, mdaq::hw::RegisterDescription& out,
                              const Ice::Current&) override {
    out.name = reg;
    out.offset = 0;
  }

  void getRegisterConfig(const ::std::string& reg, mdaq::hw::RegisterConfig& out,
                         const Ice::Current& c) override {
    getRegisterDescription(reg, out.descr, c);
    std::lock_guard<std::mutex> lk(state_->m);
    for (const std::string& f : state_->node.field_names(info_.name, reg)) {
      mdaq::hw::FieldDescription fd;
      const vcobo::FieldDef* def = state_->node.field_def(info_.name, reg, f);
      fd.name = f;
      fd.offset = ::Ice::Short(def != nullptr ? def->offset : 0);
      fd.width = ::Ice::Short(def != nullptr ? def->width : 0);
      fd.readOnly = false;  // サーバ側は readOnly を強制しない(実 MDaq も同じ)
      out.fields.push_back(fd);
    }
  }

  void getFieldDescription(const ::std::string& reg, const ::std::string& field,
                           mdaq::hw::FieldDescription& out, const Ice::Current&) override {
    std::lock_guard<std::mutex> lk(state_->m);
    const vcobo::FieldDef* def = state_->node.field_def(info_.name, reg, field);
    if (def == nullptr) throw cmd_error("no field '" + reg + "." + field + "'");
    out.name = field;
    out.offset = ::Ice::Short(def->offset);
    out.width = ::Ice::Short(def->width);
    out.readOnly = false;
  }

  /// batchProcessing=false 固定(describe xcfg でそう指定する)。呼ばれたら黙らず落とす。
  void execBatch(const mdaq::hw::RegCmdSeq&, mdaq::hw::RegCmdSeq&, const Ice::Current&) override {
    throw cmd_error("execBatch is not implemented: run this virtual CoBo with batchProcessing=false");
  }

  void setVerbose(bool v, const Ice::Current&) override { verbose_ = v; }
  bool isVerbose(const Ice::Current&) override { return verbose_; }

 private:
  void define(const mdaq::hw::RegisterConfig& cfg) {
    std::map<std::string, vcobo::FieldDef> fields;
    for (const mdaq::hw::FieldDescription& f : cfg.fields) {
      vcobo::FieldDef def;
      def.offset = f.offset;
      def.width = f.width;
      fields[f.name] = def;
    }
    state_->node.define_register(info_.name, cfg.descr.name, fields);
  }

  NodeState* state_;
  vcobo::DeviceInfo info_;
  bool verbose_ = false;
};

// ---------------------------------------------------------------------------
// AlarmService servant(identity `AlarmService`)—— 受理するだけ。callback は呼ばない。
// ---------------------------------------------------------------------------
class AlarmServiceI : public get::mt::AlarmService {
 public:
  void subscribe(const ::std::string& id, const get::mt::AlarmCallbackPrx&,
                 const Ice::Current&) override {
    log_info("AlarmService::subscribe(\"" + id + "\") accepted (no callback is ever invoked)");
  }
  void unsubscribe(const ::std::string& id, const Ice::Current&) override {
    log_info("AlarmService::unsubscribe(\"" + id + "\")");
  }
  void reset(const Ice::Current&) override { log_info("AlarmService::reset()"); }
  void sendAsadAlarm(const ::std::string& coboId, ::Ice::Long bits, const Ice::Current&) override {
    log_info("AlarmService::sendAsadAlarm(" + coboId + ", " + std::to_string(bits) + ")");
  }
};

// ---------------------------------------------------------------------------
// HwNode servant(identity `HwNode`)—— get::cobo::CtrlNode を継承するので
// Node / AsAdPulserMgr / AsAdAlarmMonitor / LedManager の ice_isA は自動で立つ。
// ---------------------------------------------------------------------------
class HwNodeI : public get::cobo::CtrlNode {
 public:
  HwNodeI(NodeState* state, int cobo_id) : state_(state), cobo_id_(cobo_id) {}

  // --- mdaq::hw::Node -----------------------------------------------------

  void testConnectionToHardware(const Ice::Current&) override {}

  void shutdown(const Ice::Current& c) override {
    log_info("Node::shutdown() requested by the client");
    c.adapter->getCommunicator()->shutdown();
  }

  void reboot(const Ice::Current&) override {
    log_warn("Node::reboot() ignored: this is a virtual CoBo, there is nothing to reboot");
  }

  ::std::string name(const Ice::Current& c) override {
    std::lock_guard<std::mutex> lk(state_->m);
    const std::string& n = state_->node.name();
    return n.empty() ? c.adapter->getName() : n;
  }

  void setName(const ::std::string& n, const Ice::Current&) override {
    const std::string expected = "CoBo[" + std::to_string(cobo_id_) + "]";
    if (n != expected) {
      // DataLinkSet の DataSender id と突合されるので、食い違いは必ず見せる。
      log_warn("setName(\"" + n + "\") does not match the configured cobo_id (expected \"" +
               expected + "\"); the DataLinkSet must use the name the ECC just assigned");
    }
    log_info("setName(\"" + n + "\")");
    std::lock_guard<std::mutex> lk(state_->m);
    state_->node.set_name(n);
  }

  /// describe の本体。ハード記述 xcfg 全体が 1 メッセージで来る(実機で 6594 行)。
  void create(const mdaq::hw::NodeConfig& config, const Ice::Current& c) override {
    std::lock_guard<std::mutex> lk(state_->m);
    remove_device_servants(c.adapter);
    state_->node.reset();
    state_->node.set_name(config.id);

    for (const mdaq::hw::DeviceConfig& dc : config.devices) {
      vcobo::DeviceInfo info;
      info.name = dc.descr.name;
      info.base_address = dc.descr.baseAddress;
      info.register_access = dc.descr.registerAccess;
      info.register_width = dc.descr.registerWidth;
      state_->node.create_device(info);

      for (const mdaq::hw::RegisterConfig& rc : dc.registers) {
        std::map<std::string, vcobo::FieldDef> fields;
        for (const mdaq::hw::FieldDescription& f : rc.fields) {
          vcobo::FieldDef def;
          def.offset = f.offset;
          def.width = f.width;
          fields[f.name] = def;
        }
        state_->node.define_register(info.name, rc.descr.name, fields);
      }

      const Ice::Identity id = c.adapter->getCommunicator()->stringToIdentity(info.name);
      devices_[info.name] =
          mdaq::hw::DevicePrx::uncheckedCast(c.adapter->add(new DeviceI(state_, info), id));
    }

    // 実 FPGA なら電源投入時から立っている値を焼き込む(039 実測の必須 4 種 + alarm)。
    state_->node.apply_magic_values();
    log_info("create(\"" + config.id + "\"): " + std::to_string(devices_.size()) +
             " devices, magic values seeded");
  }

  void destroy(const Ice::Current& c) override {
    std::lock_guard<std::mutex> lk(state_->m);
    remove_device_servants(c.adapter);
    state_->node.reset();
    log_info("destroy(): all devices removed");
  }

  void getNodeConfig(mdaq::hw::NodeConfig& out, const Ice::Current&) override {
    std::lock_guard<std::mutex> lk(state_->m);
    out.id = state_->node.name();
  }

  void getListOfDevices(mdaq::hw::DeviceList& out, const Ice::Current&) override {
    std::lock_guard<std::mutex> lk(state_->m);
    out = state_->node.device_names();
  }

  void getMapOfDevices(mdaq::hw::DeviceMap& out, const Ice::Current&) override {
    std::lock_guard<std::mutex> lk(state_->m);
    out = devices_;
  }

  mdaq::hw::DevicePrx findDevice(const ::std::string& n, const Ice::Current&) override {
    std::lock_guard<std::mutex> lk(state_->m);
    auto it = devices_.find(n);
    if (it == devices_.end()) throw cmd_error("Device not found: " + n);
    return it->second;
  }

  void removeDevice(const ::std::string& n, const Ice::Current& c) override {
    std::lock_guard<std::mutex> lk(state_->m);
    auto it = devices_.find(n);
    if (it == devices_.end()) throw cmd_error("Device not found: " + n);
    c.adapter->remove(c.adapter->getCommunicator()->stringToIdentity(n));
    devices_.erase(it);
  }

  void removeAllDevices(const Ice::Current& c) override {
    std::lock_guard<std::mutex> lk(state_->m);
    remove_device_servants(c.adapter);
    state_->node.reset();
  }

  void addDevice(const mdaq::hw::DeviceDescription& d, const Ice::Current& c) override {
    std::lock_guard<std::mutex> lk(state_->m);
    vcobo::DeviceInfo info;
    info.name = d.name;
    info.base_address = d.baseAddress;
    info.register_access = d.registerAccess;
    info.register_width = d.registerWidth;
    state_->node.create_device(info);
    const Ice::Identity id = c.adapter->getCommunicator()->stringToIdentity(info.name);
    devices_[info.name] =
        mdaq::hw::DevicePrx::uncheckedCast(c.adapter->add(new DeviceI(state_, info), id));
  }

  // --- get::cobo::CtrlNode ------------------------------------------------

  get::mt::AlarmServicePrx getAlarmService(const Ice::Current& c) override {
    return get::mt::AlarmServicePrx::uncheckedCast(
        c.adapter->createProxy(c.adapter->getCommunicator()->stringToIdentity("AlarmService")));
  }

  // --- AsAdAlarmMonitor ---------------------------------------------------

  void setAsAdAlarmMonitoringEnabled(bool enabled, const Ice::Current&) override {
    log_info(std::string("setAsAdAlarmMonitoringEnabled(") + (enabled ? "true" : "false") + ")");
  }

  // --- AsAdPulserMgr(受理して no-op。ECC は例外を握り潰す)---------------

  void setDefaultPulserVoltage(::Ice::Long, const Ice::Current&) override {}
  void triggerPulser(::Ice::Long, const Ice::Current&) override {}
  void configureExternalPulser(bool, ::Ice::Long, const Ice::Current&) override {}
  void configurePeriodicPulser(::Ice::Long, ::Ice::Long, const Ice::Current&) override {}
  void setRandomPulserEnabled(bool, const Ice::Current&) override {}
  void configureDoublePulserMode(::Ice::Long, const Ice::Current&) override {}
  void startPeriodicPulser(const Ice::Current&) override {}
  void stopPeriodicPulser(const Ice::Current&) override {}

  // --- LedManager(受理して no-op)---------------------------------------

  void blinkLEDs(const Ice::Current&) override {}
  void setLED(::Ice::Int, bool, const Ice::Current&) override {}
  void setLEDs(bool, const Ice::Current&) override {}
  void flipLED(::Ice::Int, const Ice::Current&) override {}
  void flipLEDs(const Ice::Current&) override {}
  void pulseLED(::Ice::Int, ::Ice::Int, const Ice::Current&) override {}
  void modifyLED(get::cobo::LedType, ::Ice::Int, get::cobo::LedState,
                 const Ice::Current&) override {}

 private:
  /// `state_->m` を保持したまま呼ぶこと。
  void remove_device_servants(const Ice::ObjectAdapterPtr& adapter) {
    for (const auto& kv : devices_) {
      try {
        adapter->remove(adapter->getCommunicator()->stringToIdentity(kv.first));
      } catch (const Ice::NotRegisteredException&) {
      }
    }
    devices_.clear();
  }

  NodeState* state_;
  int cobo_id_;
  mdaq::hw::DeviceMap devices_;
};

// ---------------------------------------------------------------------------
// DaqCtrlNode servant(identity `DaqCtrlNode`、port 46004)
// ---------------------------------------------------------------------------
class DaqCtrlNodeI : public mdaq::DaqCtrlNode {
 public:
  DaqCtrlNodeI(vcobo::DataLink* link, int asad_count) : link_(link), asad_count_(asad_count) {}

  void connect(const ::std::string& type, const ::std::string& endpoint,
               const Ice::Current&) override {
    log_info("connect(dataRouterType=\"" + type + "\", dataRouterName=\"" + endpoint + "\")");
    std::string err;
    if (!link_->connect(type, endpoint, err)) {
      log_error("connect failed: " + err);
      throw cmd_error(err);
    }
  }

  void disconnect(const Ice::Current&) override {
    log_info("disconnect()");
    link_->disconnect();
  }

  void setAlwaysFlushData(bool enable, const Ice::Current&) override {
    log_info(std::string("setAlwaysFlushData(") + (enable ? "true" : "false") + ")");
    link_->set_always_flush(enable);
  }

  void setCircularBuffersEnabled(::Ice::Int mask, const Ice::Current&) override {
    log_info("setCircularBuffersEnabled(mask=0x" + to_hex(mask) + ")");
    if (!vcobo::asad_mask_matches_config(mask, asad_count_)) {
      log_warn("AsAd mask 0x" + to_hex(mask) + " does not fit asad_count=" +
               std::to_string(asad_count_) + " (check the describe/configure xcfg)");
    }
    link_->set_asad_mask(mask);
  }

  void daqStart(const Ice::Current&) override {
    log_info("daqStart()");
    std::string err;
    if (!link_->daq_start(err)) {
      log_error("daqStart failed: " + err);
      throw cmd_error(err);
    }
  }

  void daqStop(const Ice::Current&) override {
    log_info("daqStop()");
    link_->daq_stop();
  }

  void shutdown(const Ice::Current& c) override {
    log_info("DaqCtrlNode::shutdown()");
    c.adapter->getCommunicator()->shutdown();
  }

 private:
  static std::string to_hex(int v) {
    std::ostringstream os;
    os << std::hex << v;
    return os.str();
  }

  vcobo::DataLink* link_;
  int asad_count_;
};

// ---------------------------------------------------------------------------

void usage() {
  std::fprintf(stderr,
               "usage: vcobo_daq --config FILE [--graw FILE]... [--rate-hz HZ] [--full-speed]\n"
               "                 [--ctrl-port N] [--daq-port N] [--loop]\n"
               "  --config FILE   TOML 部分集合(tools/vcobo/vcobo.example.conf を参照)\n"
               "  それ以外は設定ファイルの上書き(CI / オラクル照合スクリプト用)\n");
}

bool read_file(const std::string& path, std::string& out) {
  std::ifstream in(path.c_str());
  if (!in) return false;
  std::ostringstream os;
  os << in.rdbuf();
  out = os.str();
  return true;
}

}  // namespace

int main(int argc, char* argv[]) {
  vcobo::Config cfg;
  std::vector<std::string> graw_override;
  bool have_config = false;

  for (int i = 1; i < argc; ++i) {
    const std::string a = argv[i];
    const bool has_value = (i + 1 < argc);
    if (a == "--help" || a == "-h") {
      usage();
      return 0;
    } else if (a == "--config" && has_value) {
      std::string text;
      if (!read_file(argv[++i], text)) {
        log_error(std::string("cannot read config file '") + argv[i] + "'");
        return 2;
      }
      std::string err;
      if (!vcobo::parse_config(text, cfg, err)) {
        log_error("config: " + err);
        return 2;
      }
      have_config = true;
    } else if (a == "--graw" && has_value) {
      graw_override.push_back(argv[++i]);
    } else if (a == "--rate-hz" && has_value) {
      cfg.rate_hz = std::atof(argv[++i]);
    } else if (a == "--full-speed") {
      cfg.rate_hz = 0.0;
    } else if (a == "--ctrl-port" && has_value) {
      cfg.ctrl_port = std::atoi(argv[++i]);
    } else if (a == "--daq-port" && has_value) {
      cfg.daq_port = std::atoi(argv[++i]);
    } else if (a == "--loop") {
      cfg.loop = true;
    } else {
      log_error("unknown argument '" + a + "'");
      usage();
      return 2;
    }
  }

  std::signal(SIGINT, on_signal);
  std::signal(SIGTERM, on_signal);

  if (!graw_override.empty()) cfg.graw_files = graw_override;
  if (!have_config && graw_override.empty()) {
    log_error("no --config and no --graw: nothing to replay");
    usage();
    return 2;
  }
  if (cfg.graw_files.empty()) {
    log_error("config has no graw_files");
    return 2;
  }

  vcobo::GrawSet graw;
  std::string err;
  if (!vcobo::load_graw_set(cfg.graw_files, graw, err)) {
    log_error(err);
    return 2;
  }
  log_info("graw set: " + std::to_string(graw.frames.size()) + " frames, " +
           std::to_string(graw.total_bytes()) + " bytes, pacing " +
           (cfg.rate_hz > 0 ? std::to_string(cfg.rate_hz) + " Hz" : std::string("full speed")) +
           (cfg.loop ? ", looping" : ", single pass"));

  NodeState state;
  state.node.set_warn([](const std::string& m) { log_warn(m); });

  vcobo::DataLink link;
  link.start(&graw, cfg);

  int status = 0;
  Ice::CommunicatorPtr ic;
  try {
    Ice::InitializationData init;
    init.properties = Ice::createProperties();
    // getHwServer と同じ罠: 無指定だと IPv6-only bind になり、IPv4 で来る ECC と繋がらない。
    init.properties->setProperty("Ice.IPv6", "0");
    init.properties->setProperty("Ice.Warn.Dispatch", "1");
    // 2 つのアダプタ(46001 / 46004)が同じスレッドプールを共有する。1 本だと
    // daqStop の待ち(最大 10 s)が HwNode 側を巻き添えにするので余裕を持たせる。
    init.properties->setProperty("Ice.ThreadPool.Server.Size", "4");
    ic = Ice::initialize(init);

    const std::string ctrl_ep =
        "default -h " + cfg.listen_host + " -p " + std::to_string(cfg.ctrl_port);
    const std::string daq_ep =
        "default -h " + cfg.listen_host + " -p " + std::to_string(cfg.daq_port);

    Ice::ObjectAdapterPtr ctrl = ic->createObjectAdapterWithEndpoints("vcoboCtrl", ctrl_ep);
    ctrl->add(new HwNodeI(&state, cfg.cobo_id), ic->stringToIdentity("HwNode"));
    ctrl->add(new AlarmServiceI, ic->stringToIdentity("AlarmService"));
    ctrl->activate();

    Ice::ObjectAdapterPtr daq = ic->createObjectAdapterWithEndpoints("vcoboDaq", daq_ep);
    daq->add(new DaqCtrlNodeI(&link, cfg.asad_count), ic->stringToIdentity("DaqCtrlNode"));
    daq->activate();

    log_info("vcobo_daq ready: HwNode/AlarmService on " + ctrl_ep + ", DaqCtrlNode on " + daq_ep +
             " (CoBo[" + std::to_string(cfg.cobo_id) + "], " + std::to_string(cfg.asad_count) +
             " AsAd)");
    while (g_stop == 0) std::this_thread::sleep_for(std::chrono::milliseconds(50));
  } catch (const Ice::Exception& e) {
    std::ostringstream os;
    os << e;
    log_error("Ice exception: " + os.str());
    status = 1;
  }

  link.shutdown_worker();
  if (ic) ic->destroy();
  log_info("vcobo_daq stopped");
  return status;
}
