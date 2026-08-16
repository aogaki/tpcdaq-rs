// vcobo_core.hpp — 仮想 zCoBo の純コア。**Ice も socket も知らない**。
//
// ここに入るのは 3 つだけ:
//   1. レジスタ / ビットフィールドのメモリモデル(TODO/040 §1、docs/VIRTUAL_ZCOBO_ja.md §4.2)
//      - 魔法値の焼き込み(monitorID=0x41 / VDD=0xCD / reg5=0x201 / PLG{0..3}=1)
//      - `ctrl`/`asadStatus.alarm` ビット i = !`ctrl`/`asadEnable.powerON{i}`(実 FPGA 意味論)
//   2. GRAW(MFM)フレーム境界の切り出し(src/framer.rs の parse_frame_size の移植)
//   3. 設定ファイル(TOML 部分集合)のパース
//
// Silent failure を作らない: 未知のレジスタ / フィールドに触られたら **名前ごとに 1 回**
// 警告を出す(黙って 0 を返さない)。警告の出口は差し替え可能(テストで観測するため)。
#ifndef TPCDAQ_VCOBO_CORE_HPP
#define TPCDAQ_VCOBO_CORE_HPP

#include <cctype>
#include <cstdint>
#include <cstdio>
#include <functional>
#include <map>
#include <set>
#include <string>
#include <vector>

namespace vcobo {

// ---------------------------------------------------------------------------
// ビットフィールド
// ---------------------------------------------------------------------------

/// CoBo 板 1 枚が持つ AsAd コネクタ数(FPGA レジスタ幅そのもの: PLG/powerON/alarm は
/// いずれも 4 bit)。**ch 数ではない** ので焼き込んでよい定数。
constexpr int kAsAdSlots = 4;

/// `value` の [offset, offset+width) ビットを取り出す。範囲外の指定は 0 を返す。
inline uint64_t extract_bits(uint64_t value, int offset, int width) {
  if (width <= 0 || offset < 0 || offset >= 64) return 0;
  if (width > 64 - offset) width = 64 - offset;
  const uint64_t mask = (width == 64) ? ~uint64_t(0) : ((uint64_t(1) << width) - 1);
  return (value >> offset) & mask;
}

/// `value` の [offset, offset+width) ビットだけを `field` で置き換えた値を返す。
inline uint64_t insert_bits(uint64_t value, int offset, int width, uint64_t field) {
  if (width <= 0 || offset < 0 || offset >= 64) return value;
  if (width > 64 - offset) width = 64 - offset;
  const uint64_t mask = (width == 64) ? ~uint64_t(0) : ((uint64_t(1) << width) - 1);
  return (value & ~(mask << offset)) | ((field & mask) << offset);
}

// ---------------------------------------------------------------------------
// レジスタモデル
// ---------------------------------------------------------------------------

struct FieldDef {
  int offset = 0;
  int width = 1;
};

struct Register {
  uint64_t value = 0;
  std::map<std::string, FieldDef> fields;
};

struct DeviceInfo {
  std::string name;
  int64_t base_address = 0;
  std::string register_access;
  int register_width = 0;  ///< bytes(記述をそのまま返すだけ。値のマスクはしない)
};

struct Device {
  DeviceInfo info;
  std::map<std::string, Register> regs;
};

/// 1 台の CoBo(HwNode)のレジスタ空間。`create(NodeConfig)` で構築され `destroy` で消える。
///
/// スレッド安全ではない —— 呼び出し側(Ice servant 層)が 1 本の mutex で守る。
class Node {
 public:
  using WarnFn = std::function<void(const std::string&)>;

  Node() = default;

  void set_warn(WarnFn fn) { warn_ = std::move(fn); }

  const std::string& name() const { return name_; }
  void set_name(const std::string& n) { name_ = n; }

  /// `destroy` 相当。デバイス・レジスタ・警告済みマークを全部捨てる。
  void reset() {
    devices_.clear();
    warned_.clear();
  }

  /// `create(NodeConfig)` の 1 デバイス分。同名があれば置き換える。
  Device& create_device(const DeviceInfo& info) {
    Device& d = devices_[info.name];
    d.info = info;
    d.regs.clear();
    return d;
  }

  bool has_device(const std::string& dev) const { return devices_.count(dev) != 0; }

  std::vector<std::string> device_names() const {
    std::vector<std::string> out;
    out.reserve(devices_.size());
    for (const auto& kv : devices_) out.push_back(kv.first);
    return out;
  }

  const DeviceInfo* device_info(const std::string& dev) const {
    auto it = devices_.find(dev);
    return it == devices_.end() ? nullptr : &it->second.info;
  }

  std::vector<std::string> register_names(const std::string& dev) const {
    std::vector<std::string> out;
    auto it = devices_.find(dev);
    if (it == devices_.end()) return out;
    out.reserve(it->second.regs.size());
    for (const auto& kv : it->second.regs) out.push_back(kv.first);
    return out;
  }

  std::vector<std::string> field_names(const std::string& dev, const std::string& reg) const {
    std::vector<std::string> out;
    const Register* r = find_register(dev, reg);
    if (r == nullptr) return out;
    out.reserve(r->fields.size());
    for (const auto& kv : r->fields) out.push_back(kv.first);
    return out;
  }

  const FieldDef* field_def(const std::string& dev, const std::string& reg,
                            const std::string& field) const {
    const Register* r = find_register(dev, reg);
    if (r == nullptr) return nullptr;
    auto it = r->fields.find(field);
    return it == r->fields.end() ? nullptr : &it->second;
  }

  /// レジスタ定義を足す(create 時の RegisterConfigList の 1 要素)。
  void define_register(const std::string& dev, const std::string& reg,
                       const std::map<std::string, FieldDef>& fields) {
    Register& r = devices_[dev].regs[reg];
    r.fields = fields;
  }

  // --- Device の 4 op --------------------------------------------------

  uint64_t read_register(const std::string& dev, const std::string& reg) {
    const Register* r = find_register(dev, reg);
    if (r == nullptr) {
      warn_once(dev + "/" + reg, "read of undeclared register '" + dev + "/" + reg + "' -> 0");
      return 0;
    }
    return r->value;
  }

  void write_register(const std::string& dev, const std::string& reg, uint64_t value) {
    Register* r = find_register(dev, reg);
    if (r == nullptr) {
      warn_once(dev + "/" + reg, "write to undeclared register '" + dev + "/" + reg + "' accepted");
      r = &devices_[dev].regs[reg];
    }
    r->value = value;
    after_write(dev, reg);
  }

  uint64_t read_field(const std::string& dev, const std::string& reg, const std::string& field) {
    const Register* r = find_register(dev, reg);
    if (r == nullptr) {
      warn_once(dev + "/" + reg, "read of undeclared register '" + dev + "/" + reg + "' -> 0");
      return 0;
    }
    auto it = r->fields.find(field);
    if (it == r->fields.end()) {
      warn_once(dev + "/" + reg + "." + field,
                "read of undeclared field '" + dev + "/" + reg + "." + field + "' -> 0");
      return 0;
    }
    return extract_bits(r->value, it->second.offset, it->second.width);
  }

  void write_field(const std::string& dev, const std::string& reg, const std::string& field,
                   uint64_t value) {
    Register* r = find_register(dev, reg);
    if (r == nullptr) {
      warn_once(dev + "/" + reg, "write to undeclared register '" + dev + "/" + reg + "' accepted");
      r = &devices_[dev].regs[reg];
    }
    auto it = r->fields.find(field);
    if (it == r->fields.end()) {
      // 幅が分からないので「レジスタ全体」として受理する(捨てない、黙らない)。
      warn_once(dev + "/" + reg + "." + field,
                "write to undeclared field '" + dev + "/" + reg + "." + field +
                    "' accepted as the whole register");
      FieldDef whole;
      whole.offset = 0;
      whole.width = 64;
      it = r->fields.emplace(field, whole).first;
    }
    r->value = insert_bits(r->value, it->second.offset, it->second.width, value);
    after_write(dev, reg);
  }

  // --- 魔法値(039 実測の必須 4 種、docs/VIRTUAL_ZCOBO_ja.md §4.5-R2)-----

  /// `create` 直後に呼ぶ。実 FPGA なら電源投入時から立っている値を焼き込む。
  /// 対象レジスタが記述に無ければ **警告して飛ばす**(黙って諦めない)。
  void apply_magic_values() {
    seed_register("asad", "monitorID", 0x41);  // 'A' = AsAd 実在確認
    seed_register("asad", "VDD", 0xCD);        // 電源電圧チェックの閾値越え
    seed_register("aget", "reg5", 0x201);      // AGET チップ版数
    for (int i = 0; i < kAsAdSlots; ++i) {
      seed_field("ctrl", "asadConnection", "PLG" + std::to_string(i), 1);  // 物理接続検出
    }
    sync_alarm();
  }

  /// `ctrl`/`asadStatus.alarm` を powerON から導出する。
  /// **alarm ビット i = !powerON{i}** —— ECC は power off 後に 1、power on 後に 0 を
  /// 同じビットに要求する(CoBoNode.cpp:1059 / :1082)。静的メモリでは両立しないので
  /// 書き込みを追跡して導出する。
  void sync_alarm() {
    const Register* enable = find_register("ctrl", "asadEnable");
    Register* status = find_register("ctrl", "asadStatus");
    if (enable == nullptr || status == nullptr) return;
    auto alarm_it = status->fields.find("alarm");
    if (alarm_it == status->fields.end()) return;

    uint64_t alarm = 0;
    for (int i = 0; i < kAsAdSlots; ++i) {
      auto f = enable->fields.find("powerON" + std::to_string(i));
      if (f == enable->fields.end()) continue;
      const uint64_t on = extract_bits(enable->value, f->second.offset, f->second.width);
      if (on == 0) alarm |= (uint64_t(1) << i);
    }
    status->value =
        insert_bits(status->value, alarm_it->second.offset, alarm_it->second.width, alarm);
  }

 private:
  const Register* find_register(const std::string& dev, const std::string& reg) const {
    auto d = devices_.find(dev);
    if (d == devices_.end()) return nullptr;
    auto r = d->second.regs.find(reg);
    return r == d->second.regs.end() ? nullptr : &r->second;
  }

  Register* find_register(const std::string& dev, const std::string& reg) {
    return const_cast<Register*>(static_cast<const Node*>(this)->find_register(dev, reg));
  }

  void after_write(const std::string& dev, const std::string& reg) {
    if (dev == "ctrl" && reg == "asadEnable") sync_alarm();
  }

  void seed_register(const std::string& dev, const std::string& reg, uint64_t value) {
    Register* r = find_register(dev, reg);
    if (r == nullptr) {
      warn_once("seed:" + dev + "/" + reg,
                "magic value for '" + dev + "/" + reg + "' skipped: not in the node description");
      return;
    }
    r->value = value;
  }

  void seed_field(const std::string& dev, const std::string& reg, const std::string& field,
                  uint64_t value) {
    Register* r = find_register(dev, reg);
    if (r == nullptr) {
      warn_once("seed:" + dev + "/" + reg,
                "magic value for '" + dev + "/" + reg + "' skipped: not in the node description");
      return;
    }
    auto it = r->fields.find(field);
    if (it == r->fields.end()) {
      warn_once("seed:" + dev + "/" + reg + "." + field,
                "magic value for '" + dev + "/" + reg + "." + field +
                    "' skipped: field not in the node description");
      return;
    }
    r->value = insert_bits(r->value, it->second.offset, it->second.width, value);
  }

  void warn_once(const std::string& key, const std::string& message) {
    if (!warned_.insert(key).second) return;
    if (warn_) {
      warn_(message);
    } else {
      std::fprintf(stderr, "WARN %s\n", message.c_str());
    }
  }

  std::string name_;
  std::map<std::string, Device> devices_;
  std::set<std::string> warned_;
  WarnFn warn_;
};

// ---------------------------------------------------------------------------
// GRAW(MFM)フレーム境界 —— src/framer.rs::parse_frame_size の移植
// ---------------------------------------------------------------------------

/// MFM プライマリヘッダのバイト数。
constexpr size_t kPrimaryHeaderSize = 8;

/// プライマリヘッダ(8 バイト)からフレーム総バイト数を読む。壊れていれば 0。
/// byte0 = metaType(bit7: set=little / clear=big、bits[3:0] = log2(blkSize))、
/// bytes1-3 = 24bit frameSize_blk。総バイト数 = frameSize_blk × blkSize。
inline size_t graw_frame_size(const uint8_t* header) {
  const uint8_t meta_type = header[0];
  const bool little = (meta_type & 0x80) != 0;
  const size_t blk_size = size_t(1) << (meta_type & 0x0F);
  uint32_t frame_blk;
  if (little) {
    frame_blk = uint32_t(header[1]) | (uint32_t(header[2]) << 8) | (uint32_t(header[3]) << 16);
  } else {
    frame_blk = (uint32_t(header[1]) << 16) | (uint32_t(header[2]) << 8) | uint32_t(header[3]);
  }
  const size_t frame_bytes = size_t(frame_blk) * blk_size;
  return frame_bytes < kPrimaryHeaderSize ? 0 : frame_bytes;
}

/// フレーム 1 個の位置(バイト列そのものは持たない)。
struct FrameSpan {
  size_t offset = 0;
  size_t length = 0;
};

/// バイト列をフレームに切り分ける。**バイトは 1 つも変換しない**。オフセットは `data` 基準。
/// 末尾に端数(不完全フレーム)が残ったら `trailing` に入れて false を返す
/// (呼び出し側が警告する。黙って捨てない)。
inline bool index_graw(const uint8_t* data, size_t size, std::vector<FrameSpan>& out,
                       size_t& trailing) {
  out.clear();
  trailing = 0;
  size_t pos = 0;
  while (pos + kPrimaryHeaderSize <= size) {
    const size_t len = graw_frame_size(data + pos);
    if (len == 0 || pos + len > size) break;
    FrameSpan s;
    s.offset = pos;
    s.length = len;
    out.push_back(s);
    pos += len;
  }
  trailing = size - pos;
  return trailing == 0;
}

/// vector 版(1 GiB のファイルを一時 vector へ複製しないで済むよう、実体はポインタ版)。
inline bool index_graw(const std::vector<uint8_t>& bytes, std::vector<FrameSpan>& out,
                       size_t& trailing) {
  return index_graw(bytes.data(), bytes.size(), out, trailing);
}

// ---------------------------------------------------------------------------
// eventIdx マージ送出(TODO/056) —— src/bin/graw_replay.rs `mod merge` の移植
// ---------------------------------------------------------------------------

/// `p` から `n` バイトを符号なし整数として読む(エンディアン指定)。
inline uint64_t read_header_uint(const uint8_t* p, size_t n, bool little) {
  uint64_t v = 0;
  if (little) {
    for (size_t i = n; i-- > 0;) v = (v << 8) | uint64_t(p[i]);
  } else {
    for (size_t i = 0; i < n; ++i) v = (v << 8) | uint64_t(p[i]);
  }
  return v;
}

/// 共通 MFM ヘッダの eventIdx(offset 22..26)を覗き見る。src/decode.rs::peek_event_idx と
/// **同じゲート**: 28 バイト未満 / frameType(offset 5..7)∉ {1,2} は false =「制御フレーム」。
/// 制御フレームは eventIdx を持たないのでマージ順序に関与しない(遭遇時にそのまま流す)。
inline bool peek_event_idx(const uint8_t* frame, size_t len, uint32_t& event_idx) {
  if (len < 28) return false;
  const bool little = (frame[0] & 0x80) != 0;
  const uint64_t frame_type = read_header_uint(frame + 5, 2, little);
  if (frame_type != 1 && frame_type != 2) return false;
  event_idx = uint32_t(read_header_uint(frame + 22, 4, little));
  return true;
}

/// ファイル毎のフレーム列(`per_file[i]` = i 番目のファイルのフレーム、ファイル内順)を
/// **eventIdx 昇順に安定マージ**した送出順を返す。オフセットは `bytes` 基準の絶対座標。
///
/// 正本は `src/bin/graw_replay.rs` の `mod merge`(TODO/021)。規則は 3 つだけ:
///   ①先頭が制御フレームのソースが最優先(複数あればファイル指定順の最小)
///   ②データフレームは (eventIdx, ファイル指定順) の最小を選ぶ
///   ③ソース内の順序は常に保存する(= 全体ソートではない k-way マージ)
///
/// **バイト列には一切触らない**(並べ替えるのは FrameSpan の順序だけ)。
/// ELITPC の 4 AsAd(各 3,852 フレーム)でも比較回数は 4 × 15,408 なので線形探索で足りる。
inline std::vector<FrameSpan> merge_by_event_idx(
    const std::vector<uint8_t>& bytes, const std::vector<std::vector<FrameSpan>>& per_file) {
  size_t total = 0;
  for (const std::vector<FrameSpan>& f : per_file) total += f.size();

  std::vector<size_t> cursor(per_file.size(), 0);
  std::vector<FrameSpan> out;
  out.reserve(total);

  while (out.size() < total) {
    const size_t none = per_file.size();
    size_t pick = none;
    uint32_t best_event = 0;
    for (size_t i = 0; i < per_file.size(); ++i) {
      if (cursor[i] >= per_file[i].size()) continue;
      const FrameSpan& head = per_file[i][cursor[i]];
      uint32_t event = 0;
      if (!peek_event_idx(bytes.data() + head.offset, head.length, event)) {
        pick = i;  // ①制御フレームは最優先。以降を見る必要はない。
        break;
      }
      if (pick == none || event < best_event) {  // ②(eventIdx, ファイル順)の最小
        pick = i;
        best_event = event;
      }
    }
    if (pick == none) break;  // 全ソース枯渇(total と一致するので通常は来ない)
    out.push_back(per_file[pick][cursor[pick]]);
    ++cursor[pick];
  }
  return out;
}

// ---------------------------------------------------------------------------
// 設定ファイル(TOML 部分集合: `key = value` と 1 行の文字列配列)
// ---------------------------------------------------------------------------

struct Config {
  int cobo_id = 0;             ///< ECC が setName で名乗らせる `CoBo[<id>]` の id
  int asad_count = 1;          ///< mini = 1 / ELITPC = 4。**焼き込み禁止**なので設定から
  std::string listen_host = "0.0.0.0";
  int ctrl_port = 46001;       ///< HwNode / AlarmService
  int daq_port = 46004;        ///< DaqCtrlNode
  double rate_hz = 100.0;      ///< 0 = 全速(031 負荷ハーネスモード)
  bool loop = false;           ///< 送り切ったあとに先頭へ戻すか
  size_t flush_bytes = 8192;   ///< これだけ溜まったら送る(Nagle 前の自前チャンク化)
  int heartbeat_flush_ms = 3000;  ///< 端数 flush の周期(実機 3 s、§4.3-⑥)
  std::vector<std::string> graw_files;
};

namespace detail {

inline std::string trim(const std::string& s) {
  size_t b = 0;
  size_t e = s.size();
  while (b < e && std::isspace(static_cast<unsigned char>(s[b]))) ++b;
  while (e > b && std::isspace(static_cast<unsigned char>(s[e - 1]))) --e;
  return s.substr(b, e - b);
}

inline bool unquote(const std::string& s, std::string& out) {
  if (s.size() < 2) return false;
  const char q = s.front();
  if ((q != '"' && q != '\'') || s.back() != q) return false;
  out = s.substr(1, s.size() - 2);
  return true;
}

inline bool parse_bool(const std::string& s, bool& out) {
  if (s == "true") { out = true; return true; }
  if (s == "false") { out = false; return true; }
  return false;
}

inline bool parse_int(const std::string& s, long long& out) {
  try {
    size_t used = 0;
    out = std::stoll(s, &used, 0);
    return used == s.size();
  } catch (...) {
    return false;
  }
}

inline bool parse_double(const std::string& s, double& out) {
  try {
    size_t used = 0;
    out = std::stod(s, &used);
    return used == s.size();
  } catch (...) {
    return false;
  }
}

/// `["a", "b"]` を分解する。
inline bool parse_string_array(const std::string& s, std::vector<std::string>& out) {
  if (s.size() < 2 || s.front() != '[' || s.back() != ']') return false;
  out.clear();
  const std::string body = trim(s.substr(1, s.size() - 2));
  if (body.empty()) return true;
  size_t pos = 0;
  while (pos <= body.size()) {
    const size_t comma = body.find(',', pos);
    const std::string item =
        trim(body.substr(pos, comma == std::string::npos ? std::string::npos : comma - pos));
    std::string value;
    if (!unquote(item, value)) return false;
    out.push_back(value);
    if (comma == std::string::npos) break;
    pos = comma + 1;
  }
  return true;
}

}  // namespace detail

/// 設定テキストを読む。未知のキー・壊れた値は **エラーにする**(黙って既定値に落ちない)。
inline bool parse_config(const std::string& text, Config& cfg, std::string& err) {
  size_t line_no = 0;
  size_t pos = 0;
  while (pos <= text.size()) {
    const size_t nl = text.find('\n', pos);
    std::string line = text.substr(pos, nl == std::string::npos ? std::string::npos : nl - pos);
    pos = (nl == std::string::npos) ? text.size() + 1 : nl + 1;
    ++line_no;

    const size_t hash = line.find('#');
    if (hash != std::string::npos) line = line.substr(0, hash);
    line = detail::trim(line);
    if (line.empty()) continue;

    const size_t eq = line.find('=');
    if (eq == std::string::npos) {
      err = "line " + std::to_string(line_no) + ": expected 'key = value'";
      return false;
    }
    const std::string key = detail::trim(line.substr(0, eq));
    const std::string val = detail::trim(line.substr(eq + 1));

    long long i = 0;
    double d = 0;
    bool b = false;
    bool ok = false;
    if (key == "cobo_id" && detail::parse_int(val, i)) { cfg.cobo_id = int(i); ok = true; }
    else if (key == "asad_count" && detail::parse_int(val, i)) { cfg.asad_count = int(i); ok = true; }
    else if (key == "listen_host") { ok = detail::unquote(val, cfg.listen_host); }
    else if (key == "ctrl_port" && detail::parse_int(val, i)) { cfg.ctrl_port = int(i); ok = true; }
    else if (key == "daq_port" && detail::parse_int(val, i)) { cfg.daq_port = int(i); ok = true; }
    else if (key == "rate_hz" && detail::parse_double(val, d)) { cfg.rate_hz = d; ok = true; }
    else if (key == "loop" && detail::parse_bool(val, b)) { cfg.loop = b; ok = true; }
    else if (key == "flush_bytes" && detail::parse_int(val, i) && i > 0) {
      cfg.flush_bytes = size_t(i);
      ok = true;
    } else if (key == "heartbeat_flush_ms" && detail::parse_int(val, i) && i > 0) {
      cfg.heartbeat_flush_ms = int(i);
      ok = true;
    } else if (key == "graw_files") {
      ok = detail::parse_string_array(val, cfg.graw_files);
    }

    if (!ok) {
      err = "line " + std::to_string(line_no) + ": bad key or value: '" + line + "'";
      return false;
    }
  }

  if (cfg.asad_count < 1 || cfg.asad_count > kAsAdSlots) {
    err = "asad_count must be 1.." + std::to_string(kAsAdSlots);
    return false;
  }
  if (cfg.rate_hz < 0) {
    err = "rate_hz must be >= 0 (0 = full speed)";
    return false;
  }
  return true;
}

/// AsAd マスク(`setCircularBuffersEnabled`)が設定の AsAd 数と矛盾しないか。
/// 矛盾は **エラーにせず可視化する**(ECC 側の設定ミスを黙って飲まない)。
inline bool asad_mask_matches_config(int mask, int asad_count) {
  const int expected = (1 << asad_count) - 1;
  return (mask & ~expected) == 0;
}

}  // namespace vcobo

#endif  // TPCDAQ_VCOBO_CORE_HPP
