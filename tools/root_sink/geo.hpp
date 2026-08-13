// geo.hpp — root-sink 側ジオメトリパーサ(TPCReco `.dat`)。ROOT/ZMQ 非依存の純ヘッダ。
//
// 出自: NEW/LEGACY 判別・ヘッダ 7 キー・FPN リオーダの着想は
// `~/test/get/tpcdaq/src/geometry/geometry.cpp`(ユーザー自身の C++ 版、コピー + 改変)。
// ただし **意味論の正は `src/geometry.rs`(TODO/002 完了、実ジオメトリ .dat オラクル一致済み)**
// —— このヘッダは Rust 実装の逐語移植であり、両者が食い違う場合は Rust に合わせてある
// (差分は TODO/018_cpp_geometry.md の「Rust と C++ 版原本の食い違い」節を参照)。
//
// SPEC §4.1(2 レイアウト受理)/ §4.2(ChannelRole)/ §4.3(FPN リオーダ)/
// §4.5(二重実装の一致テスト = このヘッダの存在理由そのもの)。
//
// 相方: `tools/root_sink/test_geo.cpp`(単体テスト + `test_geo <file.dat>` ダンプモード)、
// `tools/root_sink/run_geo_conformance.sh`(Rust `geometry_dump` の TSV とバイト一致を diff)。

#ifndef TPCDAQ_ROOT_SINK_GEO_HPP
#define TPCDAQ_ROOT_SINK_GEO_HPP

#include <array>
#include <atomic>
#include <cctype>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <limits>
#include <optional>
#include <set>
#include <sstream>
#include <stdexcept>
#include <string>
#include <tuple>
#include <utility>
#include <vector>

namespace tpcgeo {

// ---------------------------------------------------------------------
// ハードウェア定数(AGET/AsAd ASIC の物理構造。src/geometry.rs と同一値)。
// ---------------------------------------------------------------------

constexpr uint32_t kAgetChipsPerAsad = 4;
constexpr uint32_t kRawChPerAget = 68;
constexpr uint32_t kSignalChPerAget = 64;

// FPN の raw チャンネル番号(AGET 毎、昇順)。`ChannelRole` の fpn_index はこの配列内の
// 位置 + 1(1..4)。
constexpr std::array<uint32_t, 4> kFpnRawChannels = {11, 22, 45, 56};

// 信号チャンネル(0–63)→ raw チャンネル(0–67)の並び替え定数表。
// src/geometry.rs `REORDER_FROM_GEOMETRY_TO_GRAW` と同一(TPCReco `Aget_normal2raw` の
// ループ版と全 64 入力一致することは Rust 側 002 で確認済み。test_geo.cpp でも再確認する)。
constexpr std::array<uint32_t, 64> kReorderFromGeometryToGraw = {
    0,  1,  2,  3,  4,  5,  6,  7,  8,  9,  10, 12, 13, 14, 15, 16,
    17, 18, 19, 20, 21, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33,
    34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 46, 47, 48, 49, 50,
    51, 52, 53, 54, 55, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67};

// ---------------------------------------------------------------------
// 役割・平面(SPEC §4.2)
// ---------------------------------------------------------------------

enum class Plane : uint8_t { U = 0, V = 1, W = 2 };

inline bool plane_parse(const std::string& token, Plane& out) {
  if (token == "U") {
    out = Plane::U;
    return true;
  }
  if (token == "V") {
    out = Plane::V;
    return true;
  }
  if (token == "W") {
    out = Plane::W;
    return true;
  }
  return false;
}

// TSV ダンプ等で使う表示名(dump_tsv とバイト一致させるための唯一の変換点)。
inline const char* plane_as_str(Plane p) {
  switch (p) {
    case Plane::U: return "U";
    case Plane::V: return "V";
    case Plane::W: return "W";
  }
  return "?";
}

enum class RoleKind : uint8_t { Strip, Fpn, Aux, Unmapped };

// `(cobo, asad, aget, raw_ch)` を引いた結果の役割。FPN は「ハッシュミス」ではなく
// 明示的な役割として返す。Unmapped は `.dat` に記載のないチャンネル(要可視化)。
struct ChannelRole {
  RoleKind kind = RoleKind::Unmapped;
  Plane plane = Plane::U;  // kind == Strip のときだけ有効
  uint8_t section = 0;     // kind == Strip のときだけ有効
  uint16_t strip = 0;      // kind == Strip のときだけ有効
  uint8_t fpn_index = 0;   // kind == Fpn のときだけ有効(1..4)
  std::string aux_name;    // kind == Aux のときだけ有効

  bool operator==(const ChannelRole& other) const {
    if (kind != other.kind) return false;
    switch (kind) {
      case RoleKind::Strip:
        return plane == other.plane && section == other.section && strip == other.strip;
      case RoleKind::Fpn:
        return fpn_index == other.fpn_index;
      case RoleKind::Aux:
        return aux_name == other.aux_name;
      case RoleKind::Unmapped:
        return true;
    }
    return true;
  }
  bool operator!=(const ChannelRole& other) const { return !(*this == other); }
};

inline ChannelRole make_unmapped() { return ChannelRole{}; }
inline ChannelRole make_fpn(uint8_t index) {
  ChannelRole r;
  r.kind = RoleKind::Fpn;
  r.fpn_index = index;
  return r;
}
inline ChannelRole make_strip(Plane p, uint8_t section, uint16_t strip) {
  ChannelRole r;
  r.kind = RoleKind::Strip;
  r.plane = p;
  r.section = section;
  r.strip = strip;
  return r;
}
inline ChannelRole make_aux(std::string name) {
  ChannelRole r;
  r.kind = RoleKind::Aux;
  r.aux_name = std::move(name);
  return r;
}

// ヘッダスカラ 7 キー(SPEC §4.1)。mm 変換はしない(SPEC §4.4)ので、パースした値を
// そのまま保持するだけ。値が読めなかったキーは `nullopt`。
struct HeaderScalars {
  std::optional<std::array<double, 3>> angles_deg;
  std::optional<double> diamond_size_mm;
  std::optional<std::pair<double, double>> reference_point_mm;
  std::optional<double> drift_velocity_cm_per_us;
  std::optional<double> sampling_rate_mhz;
  std::optional<double> trigger_delay_us;
  std::optional<std::pair<double, double>> drift_cage_acceptance_mm;
};

// 重複 `(cobo,asad,aget,raw_ch)` の警告(先勝ちで無視した側の記録)。
struct DuplicateChannel {
  uint32_t cobo = 0;
  uint32_t asad = 0;
  uint32_t aget = 0;
  uint32_t raw_ch = 0;
  size_t line_number = 0;
};

// トークン数が NEW(10)/LEGACY(7)/AUX(5)のいずれにも合わない、または数値パースに
// 失敗した行の記録(silent にしないためのフック)。
struct MalformedLine {
  size_t line_number = 0;
};

// `(cobo,asad,aget,raw_ch)` を稠密配列の添字に変換する。lookup / dump_tsv / パース時の
// 構築の 3 か所で共有する(src/geometry.rs `compute_index` と同一ロジック)。
inline std::optional<size_t> compute_index(const std::vector<uint32_t>& asad_offset,
                                            const std::vector<uint32_t>& asad_count,
                                            uint32_t cobo, uint32_t asad, uint32_t aget,
                                            uint32_t raw_ch) {
  size_t c = cobo;
  if (c >= asad_offset.size() || asad >= asad_count[c]) return std::nullopt;
  if (aget >= kAgetChipsPerAsad || raw_ch >= kRawChPerAget) return std::nullopt;
  size_t global_asad = static_cast<size_t>(asad_offset[c]) + asad;
  return (global_asad * kAgetChipsPerAsad + aget) * kRawChPerAget + raw_ch;
}

// ---------------------------------------------------------------------
// Geometry 本体
// ---------------------------------------------------------------------
//
// パース済みジオメトリ。`(cobo, asad, aget, raw_ch)` の稠密配列(累積 AsAd オフセット
// 方式 — TPCReco `Global_raw2normal` と同じ考え方、SPEC §4.2)。
class Geometry {
 public:
  HeaderScalars header;
  // 平面ごとの最大ストリップ番号(`[U, V, W]`)。
  std::array<uint16_t, 3> max_strip{0, 0, 0};

  Geometry() = default;

  // `std::atomic` はコピー・ムーブ不可なので既定のムーブは使えない —— `build_geometry`
  // の `return g;`(NRVO が効かない場合のフォールバック含む)に必要な分だけ手で書く。
  // カウンタは「今の値を新しい atomic に積み替える」だけで良い(旧オブジェクトはこの後
  // 破棄される)。コピーは使う場面がないので delete のままにする(KISS)。
  Geometry(Geometry&& other) noexcept
      : header(other.header),
        max_strip(other.max_strip),
        asad_offset_(std::move(other.asad_offset_)),
        asad_count_(std::move(other.asad_count_)),
        channels_(std::move(other.channels_)),
        duplicates_(std::move(other.duplicates_)),
        malformed_(std::move(other.malformed_)),
        unmapped_hits_(other.unmapped_hits_.load(std::memory_order_relaxed)) {}

  Geometry& operator=(Geometry&& other) noexcept {
    if (this != &other) {
      header = other.header;
      max_strip = other.max_strip;
      asad_offset_ = std::move(other.asad_offset_);
      asad_count_ = std::move(other.asad_count_);
      channels_ = std::move(other.channels_);
      duplicates_ = std::move(other.duplicates_);
      malformed_ = std::move(other.malformed_);
      unmapped_hits_.store(other.unmapped_hits_.load(std::memory_order_relaxed),
                            std::memory_order_relaxed);
    }
    return *this;
  }

  Geometry(const Geometry&) = delete;
  Geometry& operator=(const Geometry&) = delete;

  // `(cobo, asad, aget, raw_ch)` から役割を引く。範囲外・未記載はいずれも Unmapped を
  // 返し、`unmapped_hits_` を加算する(silent にしない可視化フック)。
  ChannelRole lookup(uint32_t cobo, uint32_t asad, uint32_t aget, uint32_t raw_ch) const {
    auto idx = index_of(cobo, asad, aget, raw_ch);
    if (!idx) {
      unmapped_hits_.fetch_add(1, std::memory_order_relaxed);
      return make_unmapped();
    }
    const ChannelRole& role = channels_[*idx];
    if (role.kind == RoleKind::Unmapped) {
      unmapped_hits_.fetch_add(1, std::memory_order_relaxed);
    }
    return role;
  }

  // `lookup` が Unmapped を返した累計回数。
  uint64_t unmapped_hit_count() const {
    return unmapped_hits_.load(std::memory_order_relaxed);
  }

  const std::vector<DuplicateChannel>& duplicate_warnings() const { return duplicates_; }
  const std::vector<MalformedLine>& malformed_lines() const { return malformed_; }

  // 登場した CoBo の数(`cobo` id の最大値 + 1)。
  size_t cobo_count() const { return asad_offset_.size(); }

  std::optional<size_t> index_of(uint32_t cobo, uint32_t asad, uint32_t aget,
                                  uint32_t raw_ch) const {
    return compute_index(asad_offset_, asad_count_, cobo, asad, aget, raw_ch);
  }

  // 以下は dump_tsv とパーサ内部(build_geometry)専用。private にすると dump_tsv が
  // friend 宣言だらけになるので、rs_core.hpp の流儀(struct 的にフィールドを直接置く)
  // に合わせて public のままにしてある(KISS)。
  std::vector<uint32_t> asad_offset_;  // cobo ごとの累積 AsAd オフセット
  std::vector<uint32_t> asad_count_;   // cobo ごとの AsAd 枚数
  std::vector<ChannelRole> channels_;
  std::vector<DuplicateChannel> duplicates_;
  std::vector<MalformedLine> malformed_;
  mutable std::atomic<uint64_t> unmapped_hits_{0};
};

// ---------------------------------------------------------------------
// パース内部実装(src/geometry.rs の逐語移植)
// ---------------------------------------------------------------------

namespace detail {

inline std::string trim(const std::string& s) {
  size_t b = 0, e = s.size();
  while (b < e && std::isspace(static_cast<unsigned char>(s[b]))) ++b;
  while (e > b && std::isspace(static_cast<unsigned char>(s[e - 1]))) --e;
  return s.substr(b, e - b);
}

// Rust `str::split_whitespace()` 相当: 連続する空白は 1 つの区切りとして扱い、
// 空トークンは生成しない。
inline std::vector<std::string> split_whitespace(const std::string& s) {
  std::vector<std::string> out;
  size_t i = 0, n = s.size();
  while (i < n) {
    while (i < n && std::isspace(static_cast<unsigned char>(s[i]))) ++i;
    size_t start = i;
    while (i < n && !std::isspace(static_cast<unsigned char>(s[i]))) ++i;
    if (i > start) out.push_back(s.substr(start, i - start));
  }
  return out;
}

// Rust `str::parse::<f64>()` 相当: トークン全体が数値でなければ失敗(部分一致は不可)。
inline std::optional<double> parse_f64(const std::string& tok) {
  if (tok.empty()) return std::nullopt;
  const char* begin = tok.c_str();
  char* end = nullptr;
  double v = std::strtod(begin, &end);
  if (end != begin + tok.size()) return std::nullopt;
  return v;
}

// Rust `str::parse::<T>()`(符号なし整数)相当: 10 進数字のみ、符号・空白・小数不可、
// 型の範囲を超えたら失敗。
template <class T>
std::optional<T> parse_uint(const std::string& tok) {
  if (tok.empty()) return std::nullopt;
  for (char c : tok) {
    if (c < '0' || c > '9') return std::nullopt;
  }
  unsigned long long v = 0;
  for (char c : tok) {
    v = v * 10 + static_cast<unsigned long long>(c - '0');
    if (v > static_cast<unsigned long long>(std::numeric_limits<T>::max())) {
      return std::nullopt;
    }
  }
  return static_cast<T>(v);
}

// 空白区切りで厳密に `N` 個の `f64` が取れたときだけ値を返す(失敗したトークンは
// 数えずに捨てるので、混入した非数値トークンは長さ不一致として検出される —
// src/geometry.rs `parse_n_f64` と同一の filter_map 挙動)。
template <std::size_t N>
std::optional<std::array<double, N>> parse_n_f64(const std::string& rest) {
  std::vector<std::string> toks = split_whitespace(rest);
  std::vector<double> vals;
  vals.reserve(toks.size());
  for (const auto& t : toks) {
    auto v = parse_f64(t);
    if (v) vals.push_back(*v);
  }
  if (vals.size() != N) return std::nullopt;
  std::array<double, N> out{};
  for (size_t i = 0; i < N; ++i) out[i] = vals[i];
  return out;
}

inline bool starts_with(const std::string& s, const char* prefix) {
  size_t n = std::strlen(prefix);
  return s.size() >= n && s.compare(0, n, prefix) == 0;
}

// `KEY: 値...` 形式のヘッダ行を認識して `header` に書き込む。キーに一致すれば値の
// パース可否によらず(未パースなら該当フィールドは nullopt のまま)true を返す —
// 認識できたキーは「データ行」ではないので、以降のトークン数判定には回さない。
inline bool try_parse_header_scalar(const std::string& trimmed, HeaderScalars& header) {
  if (starts_with(trimmed, "ANGLES:")) {
    auto v = parse_n_f64<3>(trimmed.substr(std::strlen("ANGLES:")));
    if (v) header.angles_deg = *v;
    return true;
  }
  if (starts_with(trimmed, "DIAMOND SIZE:")) {
    auto v = parse_n_f64<1>(trimmed.substr(std::strlen("DIAMOND SIZE:")));
    if (v) header.diamond_size_mm = (*v)[0];
    return true;
  }
  if (starts_with(trimmed, "REFERENCE POINT:")) {
    auto v = parse_n_f64<2>(trimmed.substr(std::strlen("REFERENCE POINT:")));
    if (v) header.reference_point_mm = std::make_pair((*v)[0], (*v)[1]);
    return true;
  }
  if (starts_with(trimmed, "DRIFT VELOCITY:")) {
    auto v = parse_n_f64<1>(trimmed.substr(std::strlen("DRIFT VELOCITY:")));
    if (v) header.drift_velocity_cm_per_us = (*v)[0];
    return true;
  }
  if (starts_with(trimmed, "SAMPLING RATE:")) {
    auto v = parse_n_f64<1>(trimmed.substr(std::strlen("SAMPLING RATE:")));
    if (v) header.sampling_rate_mhz = (*v)[0];
    return true;
  }
  if (starts_with(trimmed, "TRIGGER DELAY:")) {
    auto v = parse_n_f64<1>(trimmed.substr(std::strlen("TRIGGER DELAY:")));
    if (v) header.trigger_delay_us = (*v)[0];
    return true;
  }
  if (starts_with(trimmed, "DRIFT CAGE ACCEPTANCE:")) {
    auto v = parse_n_f64<2>(trimmed.substr(std::strlen("DRIFT CAGE ACCEPTANCE:")));
    if (v) header.drift_cage_acceptance_mm = std::make_pair((*v)[0], (*v)[1]);
    return true;
  }
  return false;
}

struct ParsedStrip {
  uint32_t cobo = 0;
  uint32_t asad = 0;
  uint32_t aget = 0;
  Plane plane = Plane::U;
  uint8_t section = 0;
  uint16_t strip = 0;
  uint32_t signal_ch = 0;
};

struct ParsedAux {
  std::string name;
  uint32_t cobo = 0;
  uint32_t asad = 0;
  uint32_t aget = 0;
  uint32_t signal_ch = 0;
};

struct PendingChannel {
  uint32_t cobo = 0;
  uint32_t asad = 0;
  uint32_t aget = 0;
  uint32_t raw_ch = 0;
  ChannelRole role;
  size_t line_number = 0;
};

using AgetKey = std::tuple<uint32_t, uint32_t, uint32_t>;

// NEW(10 欄): DIR SECTION STRIP COBO ASAD AGET AGET_CH OFF_PAD OFF_STRIP LEN_PADS。
inline std::optional<ParsedStrip> parse_new_strip_line(const std::vector<std::string>& t) {
  Plane plane;
  if (!plane_parse(t[0], plane)) return std::nullopt;
  auto section = parse_uint<uint8_t>(t[1]);
  if (!section) return std::nullopt;
  auto strip = parse_uint<uint16_t>(t[2]);
  if (!strip) return std::nullopt;
  auto cobo = parse_uint<uint32_t>(t[3]);
  if (!cobo) return std::nullopt;
  auto asad = parse_uint<uint32_t>(t[4]);
  if (!asad) return std::nullopt;
  auto aget = parse_uint<uint32_t>(t[5]);
  if (!aget) return std::nullopt;
  if (*aget >= kAgetChipsPerAsad) return std::nullopt;
  auto signal_ch = parse_uint<uint32_t>(t[6]);
  if (!signal_ch) return std::nullopt;
  if (!parse_f64(t[7])) return std::nullopt;
  if (!parse_f64(t[8])) return std::nullopt;
  if (!parse_f64(t[9])) return std::nullopt;
  return ParsedStrip{*cobo, *asad, *aget, plane, *section, *strip, *signal_ch};
}

// LEGACY(7 欄): DIR STRIP AGET AGET_CH OFF_PAD OFF_STRIP LEN_PADS(section/cobo/asad = 0)。
inline std::optional<ParsedStrip> parse_legacy_strip_line(const std::vector<std::string>& t) {
  Plane plane;
  if (!plane_parse(t[0], plane)) return std::nullopt;
  auto strip = parse_uint<uint16_t>(t[1]);
  if (!strip) return std::nullopt;
  auto aget = parse_uint<uint32_t>(t[2]);
  if (!aget) return std::nullopt;
  if (*aget >= kAgetChipsPerAsad) return std::nullopt;
  auto signal_ch = parse_uint<uint32_t>(t[3]);
  if (!signal_ch) return std::nullopt;
  if (!parse_f64(t[4])) return std::nullopt;
  if (!parse_f64(t[5])) return std::nullopt;
  if (!parse_f64(t[6])) return std::nullopt;
  return ParsedStrip{0, 0, *aget, plane, 0, *strip, *signal_ch};
}

// AUX(5 欄): NAME COBO ASAD AGET AGET_CH。AGET_CH は STRIP 行と同じ「信号番号 0–63」
// 規約(src/geometry.rs `parse_aux_line` のコメント・SPEC §4.1 で確定済み)。
inline std::optional<ParsedAux> parse_aux_line(const std::vector<std::string>& t) {
  std::string name = t[0];
  auto cobo = parse_uint<uint32_t>(t[1]);
  if (!cobo) return std::nullopt;
  auto asad = parse_uint<uint32_t>(t[2]);
  if (!asad) return std::nullopt;
  auto aget = parse_uint<uint32_t>(t[3]);
  if (!aget) return std::nullopt;
  if (*aget >= kAgetChipsPerAsad) return std::nullopt;
  auto signal_ch = parse_uint<uint32_t>(t[4]);
  if (!signal_ch) return std::nullopt;
  return ParsedAux{name, *cobo, *asad, *aget, *signal_ch};
}

inline void push_strip(const ParsedStrip& parsed, size_t line_number,
                        std::vector<PendingChannel>& pending, std::set<AgetKey>& active_agets,
                        std::array<uint16_t, 3>& max_strip,
                        std::vector<MalformedLine>& malformed) {
  if (parsed.signal_ch >= kReorderFromGeometryToGraw.size()) {
    malformed.push_back(MalformedLine{line_number});
    return;
  }
  uint32_t raw_ch = kReorderFromGeometryToGraw[parsed.signal_ch];
  active_agets.insert({parsed.cobo, parsed.asad, parsed.aget});
  size_t plane_idx = static_cast<size_t>(parsed.plane);
  if (parsed.strip > max_strip[plane_idx]) max_strip[plane_idx] = parsed.strip;
  PendingChannel entry;
  entry.cobo = parsed.cobo;
  entry.asad = parsed.asad;
  entry.aget = parsed.aget;
  entry.raw_ch = raw_ch;
  entry.role = make_strip(parsed.plane, parsed.section, parsed.strip);
  entry.line_number = line_number;
  pending.push_back(std::move(entry));
}

inline void push_aux(const ParsedAux& parsed, size_t line_number,
                      std::vector<PendingChannel>& pending, std::set<AgetKey>& active_agets,
                      std::vector<MalformedLine>& malformed) {
  if (parsed.signal_ch >= kReorderFromGeometryToGraw.size()) {
    malformed.push_back(MalformedLine{line_number});
    return;
  }
  uint32_t raw_ch = kReorderFromGeometryToGraw[parsed.signal_ch];
  active_agets.insert({parsed.cobo, parsed.asad, parsed.aget});
  PendingChannel entry;
  entry.cobo = parsed.cobo;
  entry.asad = parsed.asad;
  entry.aget = parsed.aget;
  entry.raw_ch = raw_ch;
  entry.role = make_aux(parsed.name);
  entry.line_number = line_number;
  pending.push_back(std::move(entry));
}

// 稠密配列を組み立てる。手順: (1) 登場した全 (cobo,asad,aget) に FPN を先に置く
// (ハードウェア的な真実、`.dat` には書かれない)。(2) ファイル順に STRIP/AUX を適用し、
// 埋まっていたら重複警告 + 先勝ち。
inline Geometry build_geometry(HeaderScalars header, std::array<uint16_t, 3> max_strip,
                                std::vector<PendingChannel> pending,
                                const std::set<AgetKey>& active_agets,
                                std::vector<MalformedLine> malformed) {
  size_t cobo_count = 0;
  for (const auto& key : active_agets) {
    uint32_t cobo = std::get<0>(key);
    if (static_cast<size_t>(cobo) + 1 > cobo_count) cobo_count = static_cast<size_t>(cobo) + 1;
  }

  std::vector<uint32_t> asad_count(cobo_count, 0);
  for (const auto& key : active_agets) {
    uint32_t cobo = std::get<0>(key);
    uint32_t asad = std::get<1>(key);
    if (asad + 1 > asad_count[cobo]) asad_count[cobo] = asad + 1;
  }

  std::vector<uint32_t> asad_offset(cobo_count, 0);
  uint32_t running = 0;
  for (size_t c = 0; c < cobo_count; ++c) {
    asad_offset[c] = running;
    running += asad_count[c];
  }
  size_t total_asad = running;
  size_t array_len = total_asad * kAgetChipsPerAsad * kRawChPerAget;
  std::vector<ChannelRole> channels(array_len);  // 既定コンストラクタ = Unmapped

  for (const auto& key : active_agets) {
    uint32_t cobo = std::get<0>(key);
    uint32_t asad = std::get<1>(key);
    uint32_t aget = std::get<2>(key);
    for (size_t i = 0; i < kFpnRawChannels.size(); ++i) {
      auto idx = compute_index(asad_offset, asad_count, cobo, asad, aget, kFpnRawChannels[i]);
      if (idx) channels[*idx] = make_fpn(static_cast<uint8_t>(i + 1));
    }
  }

  std::vector<DuplicateChannel> duplicates;
  for (const auto& entry : pending) {
    auto idx = compute_index(asad_offset, asad_count, entry.cobo, entry.asad, entry.aget,
                              entry.raw_ch);
    if (idx) {
      if (channels[*idx].kind == RoleKind::Unmapped) {
        channels[*idx] = entry.role;
      } else {
        duplicates.push_back(
            DuplicateChannel{entry.cobo, entry.asad, entry.aget, entry.raw_ch, entry.line_number});
      }
    } else {
      malformed.push_back(MalformedLine{entry.line_number});
    }
  }

  Geometry g;
  g.header = header;
  g.max_strip = max_strip;
  g.channels_ = std::move(channels);
  g.asad_offset_ = std::move(asad_offset);
  g.asad_count_ = std::move(asad_count);
  g.duplicates_ = std::move(duplicates);
  g.malformed_ = std::move(malformed);
  return g;
}

}  // namespace detail

// `.dat` テキストをパースする。行単位で寛容(TPCReco の sscanf 方式を踏襲): 解釈できない
// 行はエラーにせず MalformedLine として記録し、後続行のパースは続行する。重複
// `(cobo,asad,aget,raw_ch)` も同様に警告 + 先勝ち。
inline Geometry parse(const std::string& text) {
  HeaderScalars header;
  std::vector<detail::PendingChannel> pending;
  std::set<detail::AgetKey> active_agets;
  std::vector<MalformedLine> malformed;
  std::array<uint16_t, 3> max_strip{0, 0, 0};

  std::istringstream iss(text);
  std::string raw_line;
  size_t line_number = 0;
  while (std::getline(iss, raw_line)) {
    ++line_number;
    std::string trimmed = detail::trim(raw_line);
    if (trimmed.empty() || trimmed[0] == '#') continue;
    if (detail::try_parse_header_scalar(trimmed, header)) continue;

    std::vector<std::string> tokens = detail::split_whitespace(trimmed);
    switch (tokens.size()) {
      case 10: {
        auto parsed = detail::parse_new_strip_line(tokens);
        if (parsed) {
          detail::push_strip(*parsed, line_number, pending, active_agets, max_strip, malformed);
        } else {
          malformed.push_back(MalformedLine{line_number});
        }
        break;
      }
      case 7: {
        auto parsed = detail::parse_legacy_strip_line(tokens);
        if (parsed) {
          detail::push_strip(*parsed, line_number, pending, active_agets, max_strip, malformed);
        } else {
          malformed.push_back(MalformedLine{line_number});
        }
        break;
      }
      case 5: {
        auto parsed = detail::parse_aux_line(tokens);
        if (parsed) {
          detail::push_aux(*parsed, line_number, pending, active_agets, malformed);
        } else {
          malformed.push_back(MalformedLine{line_number});
        }
        break;
      }
      default:
        malformed.push_back(MalformedLine{line_number});
    }
  }

  return detail::build_geometry(header, max_strip, std::move(pending), active_agets,
                                 std::move(malformed));
}

// ファイルパスから読み込む(`parse` のファイル版)。I/O 失敗は throw(ファイル内容の
// 不整合は上記の通り警告扱いであって throw しない)。
inline Geometry load(const std::string& path) {
  std::ifstream in(path, std::ios::binary);
  if (!in) {
    throw std::runtime_error("failed to read geometry file: " + path);
  }
  std::ostringstream ss;
  ss << in.rdbuf();
  return parse(ss.str());
}

// 全チャンネルを cobo/asad/aget/raw_ch 昇順で 1 行ずつ、
// `cobo asad aget raw_ch role plane section strip` をタブ区切りでダンプする(SPEC §4.5、
// Rust/C++ 一致テストの入力)。role に付随しない列は空文字。ホットパスの
// unmapped_hits_ は増やさない(全走査で意味のない値になるため — src/geometry.rs
// `dump_tsv` と同じ配慮)。
//
// **フォーマットは `src/geometry.rs::dump_tsv` が正**。1 文字たりとも違えてはいけない
// (run_geo_conformance.sh がバイト一致で検証する)。
inline std::string dump_tsv(const Geometry& g) {
  std::string out;
  for (size_t cobo = 0; cobo < g.asad_offset_.size(); ++cobo) {
    uint32_t asad_count = g.asad_count_[cobo];
    for (uint32_t asad = 0; asad < asad_count; ++asad) {
      for (uint32_t aget = 0; aget < kAgetChipsPerAsad; ++aget) {
        for (uint32_t raw_ch = 0; raw_ch < kRawChPerAget; ++raw_ch) {
          // cobo/asad/aget/raw_ch はすべてこのループの範囲内なので必ず値が返る。
          auto idx = compute_index(g.asad_offset_, g.asad_count_, static_cast<uint32_t>(cobo),
                                    asad, aget, raw_ch);
          if (!idx) continue;
          const ChannelRole& role = g.channels_[*idx];
          const char* role_name = "Unmapped";
          std::string plane_s, section_s, strip_s;
          switch (role.kind) {
            case RoleKind::Strip:
              role_name = "Strip";
              plane_s = plane_as_str(role.plane);
              section_s = std::to_string(static_cast<unsigned>(role.section));
              strip_s = std::to_string(role.strip);
              break;
            case RoleKind::Fpn:
              role_name = "Fpn";
              break;
            case RoleKind::Aux:
              role_name = "Aux";
              break;
            case RoleKind::Unmapped:
              role_name = "Unmapped";
              break;
          }
          out += std::to_string(cobo);
          out += '\t';
          out += std::to_string(asad);
          out += '\t';
          out += std::to_string(aget);
          out += '\t';
          out += std::to_string(raw_ch);
          out += '\t';
          out += role_name;
          out += '\t';
          out += plane_s;
          out += '\t';
          out += section_s;
          out += '\t';
          out += strip_s;
          out += '\n';
        }
      }
    }
  }
  return out;
}

}  // namespace tpcgeo

#endif  // TPCDAQ_ROOT_SINK_GEO_HPP
