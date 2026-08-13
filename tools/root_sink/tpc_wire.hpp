// tpc_wire.hpp — tpcdaq のワイヤ表現(MessagePack)デコーダ。ROOT 非依存・ZMQ 非依存。
//
// 出自: MessagePack リーダ部は **delila-rs `tools/delila2root/TDelila.hpp` の
// `tdelila::mp::Reader` からの移植**(BSD-3-Clause、同じ作者のプロジェクト)。
// 移植にあたっての変更点:
//   * bin(0xc4–0xc6)を「読み飛ばす」のをやめ、**スライス(Bin)として取り出す**。
//     tpcdaq では Fragment.items と RawFrames が bin なので本質的に必要(SPEC §2.3/§2.4)。
//   * Value(DOM)を落として型付きアクセサだけにした(このユニットに DOM の用途がない — KISS)。
//   * スカラは符号なし前提(スキーマが全部 uN)。負値は黙って通さず throw する。
//
// ワイヤ表現の正本は Rust 側 `src/msg.rs` の `mod schema` 定数表(SPEC §2.5):
//   Message  = fixmap(1) { "Data" | "EndOfStream" | "Heartbeat" : ペイロード }
//   Batch    = positional array(5) [source_id u32, run_number u32,
//                                   sequence_number u64, created_ns u64, payload]
//   Fragment = positional array(12) [event_idx u32, event_time u64, cobo u8, asad u8,
//              frame_type u8, revision u8, read_offset u16, status u8, mult [u16;4],
//              window_out u32, last_cell [u16;4], items bin]
//   items    = u32 LE の連結(1 item = 1 サンプル)
//
// **前方互換(delila 方式)**: 配列は「先頭 N フィールドを読み、残りは skip」。Rust 側が
// 末尾にフィールドを足しても C++ は落ちない。逆に足りない場合は throw(プロトコル不一致を
// 黙って通さない — CLAUDE.md「silent failure を作らない」)。
//
// エラー方針: malformed はすべて std::runtime_error。ロスレス契約では「warn して捨てる」を
// しない(SPEC §6.2-6)ので、呼び手(root_sink.cxx)は catch して Error 停止する。

#ifndef TPCDAQ_ROOT_SINK_TPC_WIRE_HPP
#define TPCDAQ_ROOT_SINK_TPC_WIRE_HPP

#include <cstdint>
#include <cstdio>
#include <stdexcept>
#include <string>
#include <vector>

namespace tpcwire {

namespace mp {

// バイト列への参照(呼び手のバッファが生きている間だけ有効)。
struct Bin {
  const uint8_t* data = nullptr;
  size_t size = 0;
};

// メモリ上のバッファを進むカーソル。malformed / 途中切れは std::runtime_error。
class Reader {
 public:
  Reader(const uint8_t* data, size_t size)
      : p_(data), end_(data + (data == nullptr ? 0 : size)) {}

  const uint8_t* ptr() const { return p_; }
  const uint8_t* end() const { return end_; }
  size_t remaining() const { return static_cast<size_t>(end_ - p_); }
  bool at_end() const { return p_ >= end_; }

  // 配列ヘッダを 1 個読み、要素数を返す。
  uint32_t read_array_len() {
    uint8_t b = u8();
    if ((b & 0xf0) == 0x90) return b & 0x0f;  // fixarray
    if (b == 0xdc) return be16();             // array16
    if (b == 0xdd) return be32();             // array32
    throw std::runtime_error("expected MessagePack array, got 0x" + hex(b));
  }

  uint32_t read_map_len() {
    uint8_t b = u8();
    if ((b & 0xf0) == 0x80) return b & 0x0f;  // fixmap
    if (b == 0xde) return be16();             // map16
    if (b == 0xdf) return be32();             // map32
    throw std::runtime_error("expected MessagePack map, got 0x" + hex(b));
  }

  std::string read_str() {
    uint8_t b = u8();
    uint32_t n;
    if ((b & 0xe0) == 0xa0) n = b & 0x1f;  // fixstr
    else if (b == 0xd9) n = u8();
    else if (b == 0xda) n = be16();
    else if (b == 0xdb) n = be32();
    else throw std::runtime_error("expected MessagePack string, got 0x" + hex(b));
    need(n);
    std::string s(reinterpret_cast<const char*>(p_), n);
    p_ += n;
    return s;
  }

  // 符号なし整数(スキーマは全フィールド uN)。負値は throw。
  uint64_t read_uint() {
    uint8_t b = u8();
    if (b <= 0x7f) return b;  // positive fixint
    switch (b) {
      case 0xcc: return u8();
      case 0xcd: return be16();
      case 0xce: return be32();
      case 0xcf: return be64();
      case 0xd0: { int8_t v = static_cast<int8_t>(u8()); return non_negative(v); }
      case 0xd1: { int16_t v = static_cast<int16_t>(be16()); return non_negative(v); }
      case 0xd2: { int32_t v = static_cast<int32_t>(be32()); return non_negative(v); }
      case 0xd3: { int64_t v = static_cast<int64_t>(be64()); return non_negative(v); }
      default:
        throw std::runtime_error("expected MessagePack unsigned int, got 0x" + hex(b));
    }
  }

  // bin をスライスとして取り出す(コピーしない)。array にフォールバックしていたら throw。
  Bin read_bin() {
    uint8_t b = u8();
    uint32_t n;
    if (b == 0xc4) n = u8();
    else if (b == 0xc5) n = be16();
    else if (b == 0xc6) n = be32();
    else throw std::runtime_error("expected MessagePack bin, got 0x" + hex(b));
    need(n);
    Bin bin{p_, n};
    p_ += n;
    return bin;
  }

  // 次の値を型に関わらず読み飛ばす(前方互換の「残りは skip」)。
  void skip_value() {
    uint8_t b = peek();
    if (b <= 0x7f || b >= 0xe0) { p_++; return; }  // fixint(正・負)
    if ((b & 0xf0) == 0x90 || b == 0xdc || b == 0xdd) {
      uint32_t n = read_array_len();
      for (uint32_t k = 0; k < n; ++k) skip_value();
      return;
    }
    if ((b & 0xe0) == 0xa0 || b == 0xd9 || b == 0xda || b == 0xdb) { read_str(); return; }
    if ((b & 0xf0) == 0x80 || b == 0xde || b == 0xdf) {
      uint32_t n = read_map_len();
      for (uint32_t k = 0; k < n; ++k) { skip_value(); skip_value(); }
      return;
    }
    p_++;
    switch (b) {
      case 0xc0: case 0xc2: case 0xc3: return;                       // nil / bool
      case 0xcc: case 0xd0: skip_bytes(1); return;
      case 0xcd: case 0xd1: skip_bytes(2); return;
      case 0xce: case 0xd2: case 0xca: skip_bytes(4); return;
      case 0xcf: case 0xd3: case 0xcb: skip_bytes(8); return;
      case 0xc4: { uint32_t n = u8();   skip_bytes(n); return; }
      case 0xc5: { uint32_t n = be16(); skip_bytes(n); return; }
      case 0xc6: { uint32_t n = be32(); skip_bytes(n); return; }
      default: throw std::runtime_error("skip: unsupported MessagePack byte 0x" + hex(b));
    }
  }

 private:
  uint8_t peek() const {
    if (p_ >= end_) throw std::runtime_error("truncated MessagePack");
    return *p_;
  }
  uint8_t u8() {
    if (p_ >= end_) throw std::runtime_error("truncated MessagePack");
    return *p_++;
  }
  uint16_t be16() { uint16_t v = static_cast<uint16_t>(u8() << 8); return static_cast<uint16_t>(v | u8()); }
  uint32_t be32() { uint32_t v = 0; for (int k = 0; k < 4; ++k) v = (v << 8) | u8(); return v; }
  uint64_t be64() { uint64_t v = 0; for (int k = 0; k < 8; ++k) v = (v << 8) | u8(); return v; }
  void need(uint32_t n) {
    if (remaining() < n) throw std::runtime_error("MessagePack value overruns buffer");
  }
  void skip_bytes(uint32_t n) { need(n); p_ += n; }
  static uint64_t non_negative(int64_t v) {
    if (v < 0) throw std::runtime_error("negative value where unsigned expected");
    return static_cast<uint64_t>(v);
  }
  static std::string hex(uint8_t b) {
    char buf[3];
    std::snprintf(buf, sizeof(buf), "%02x", b);
    return buf;
  }

  const uint8_t* p_;
  const uint8_t* end_;
};

}  // namespace mp

using mp::Bin;
using mp::Reader;

// ---------------------------------------------------------------------------
// スキーマ定数(正本は src/msg.rs `mod schema` — SPEC §2.5)
// ---------------------------------------------------------------------------

// Batch の必須フィールド数(positional array(5))。
constexpr uint32_t kBatchFields = 5;
// Fragment の必須フィールド数(positional array(12))。
constexpr uint32_t kFragmentFields = 12;
// EndOfStream { source_id, run_number }。
constexpr uint32_t kEndOfStreamFields = 2;
// Heartbeat { source_id, run_number, counter }。
constexpr uint32_t kHeartbeatFields = 3;

// ---------------------------------------------------------------------------
// 1. エンベロープ(SPEC §2.2)
// ---------------------------------------------------------------------------

enum class MsgKind { Data, EndOfStream, Heartbeat, Unknown };

// 1 ZMQ メッセージを分解した結果。Data では payload が Batch の配列ヘッダを指す
// (呼び手のバッファのスライス —— そのバッファが生きている間だけ有効)。
struct Envelope {
  MsgKind kind = MsgKind::Unknown;
  std::string variant;               // ワイヤ上のバリアント名(Unknown の可視化用)
  const uint8_t* payload = nullptr;  // Data
  size_t payload_size = 0;           // Data
  uint32_t source_id = 0;            // EndOfStream / Heartbeat
  uint32_t run_number = 0;           // EndOfStream / Heartbeat
  uint64_t counter = 0;              // Heartbeat
};

// fixmap(1) のエンベロープを剥がす。構造が壊れていれば throw。
// 知らないバリアント名は「壊れている」とは違うので kind=Unknown + variant 名で返す
// (呼び手が名前付きで可視化できる)。
inline Envelope parse_envelope(const uint8_t* data, size_t size) {
  Envelope env;
  Reader r(data, size);
  uint32_t m = r.read_map_len();
  if (m != 1) throw std::runtime_error("Message must be a msgpack map of 1 entry");
  env.variant = r.read_str();
  if (env.variant == "Data") {
    env.kind = MsgKind::Data;
    env.payload = r.ptr();
    env.payload_size = r.remaining();
    if (env.payload_size == 0) throw std::runtime_error("Data message has no Batch payload");
  } else if (env.variant == "EndOfStream") {
    env.kind = MsgKind::EndOfStream;
    uint32_t n = r.read_array_len();
    if (n < kEndOfStreamFields) throw std::runtime_error("EndOfStream: too few fields");
    env.source_id = static_cast<uint32_t>(r.read_uint());
    env.run_number = static_cast<uint32_t>(r.read_uint());
    for (uint32_t k = kEndOfStreamFields; k < n; ++k) r.skip_value();
  } else if (env.variant == "Heartbeat") {
    env.kind = MsgKind::Heartbeat;
    uint32_t n = r.read_array_len();
    if (n < kHeartbeatFields) throw std::runtime_error("Heartbeat: too few fields");
    env.source_id = static_cast<uint32_t>(r.read_uint());
    env.run_number = static_cast<uint32_t>(r.read_uint());
    env.counter = r.read_uint();
    for (uint32_t k = kHeartbeatFields; k < n; ++k) r.skip_value();
  } else {
    env.kind = MsgKind::Unknown;
  }
  return env;
}

// ---------------------------------------------------------------------------
// 2. Batch(SPEC §2.2)
// ---------------------------------------------------------------------------

struct BatchHeader {
  uint32_t source_id = 0;
  uint32_t run_number = 0;
  uint64_t sequence_number = 0;
  uint64_t created_ns = 0;
  const uint8_t* payload = nullptr;  // 第 5 フィールド(リンク別 — SPEC §2.3)
  size_t payload_size = 0;
};

inline BatchHeader parse_batch_header(const uint8_t* data, size_t size) {
  Reader r(data, size);
  uint32_t n = r.read_array_len();
  if (n < kBatchFields) throw std::runtime_error("Batch: too few fields");
  BatchHeader h;
  h.source_id = static_cast<uint32_t>(r.read_uint());
  h.run_number = static_cast<uint32_t>(r.read_uint());
  h.sequence_number = r.read_uint();
  h.created_ns = r.read_uint();
  h.payload = r.ptr();
  h.payload_size = r.remaining();
  if (h.payload_size == 0) throw std::runtime_error("Batch: missing payload");
  return h;
}

// ---------------------------------------------------------------------------
// 3. Fragment(SPEC §2.4)
// ---------------------------------------------------------------------------

// デコード済みフレーム 1 個。items は元バッファへのスライス(コピーしない)。
struct FragmentView {
  uint32_t event_idx = 0;
  uint64_t event_time = 0;  // 48bit 有効
  uint8_t cobo = 0;
  uint8_t asad = 0;
  uint8_t frame_type = 0;
  uint8_t revision = 0;
  uint16_t read_offset = 0;
  uint8_t status = 0;
  uint16_t mult[4] = {0, 0, 0, 0};
  uint32_t window_out = 0;
  uint16_t last_cell[4] = {0, 0, 0, 0};
  const uint8_t* items = nullptr;
  size_t items_size = 0;

  size_t item_count() const { return items_size / 4; }

  // i 番目の item(u32 LE)。範囲外は呼ばないこと(呼び手が item_count で回す)。
  uint32_t item(size_t i) const {
    const uint8_t* p = items + i * 4;
    return static_cast<uint32_t>(p[0]) | (static_cast<uint32_t>(p[1]) << 8) |
           (static_cast<uint32_t>(p[2]) << 16) | (static_cast<uint32_t>(p[3]) << 24);
  }
};

namespace detail {

inline void read_u16_array4(Reader& r, uint16_t (&out)[4], const char* what) {
  uint32_t n = r.read_array_len();
  if (n < 4) throw std::runtime_error(std::string(what) + ": expected 4 elements");
  for (uint32_t k = 0; k < 4; ++k) out[k] = static_cast<uint16_t>(r.read_uint());
  for (uint32_t k = 4; k < n; ++k) r.skip_value();
}

inline FragmentView read_fragment(Reader& r) {
  uint32_t n = r.read_array_len();
  if (n < kFragmentFields) throw std::runtime_error("Fragment: too few fields");
  FragmentView f;
  f.event_idx = static_cast<uint32_t>(r.read_uint());
  f.event_time = r.read_uint();
  f.cobo = static_cast<uint8_t>(r.read_uint());
  f.asad = static_cast<uint8_t>(r.read_uint());
  f.frame_type = static_cast<uint8_t>(r.read_uint());
  f.revision = static_cast<uint8_t>(r.read_uint());
  f.read_offset = static_cast<uint16_t>(r.read_uint());
  f.status = static_cast<uint8_t>(r.read_uint());
  detail::read_u16_array4(r, f.mult, "Fragment.mult");
  f.window_out = static_cast<uint32_t>(r.read_uint());
  detail::read_u16_array4(r, f.last_cell, "Fragment.last_cell");
  Bin items = r.read_bin();
  if (items.size % 4 != 0) {
    throw std::runtime_error("Fragment.items length is not a multiple of 4");
  }
  f.items = items.data;
  f.items_size = items.size;
  for (uint32_t k = kFragmentFields; k < n; ++k) r.skip_value();
  return f;
}

}  // namespace detail

// Fragments ペイロード(Vec<Fragment>)を読む。`out` は上書きされる(容量は再利用される)。
// 戻り値 = フラグメント数。
inline size_t read_fragments(const uint8_t* data, size_t size, std::vector<FragmentView>& out) {
  Reader r(data, size);
  uint32_t n = r.read_array_len();
  out.clear();
  out.reserve(n);
  for (uint32_t k = 0; k < n; ++k) out.push_back(detail::read_fragment(r));
  return out.size();
}

// RawFrames ペイロード(Vec<Bin> = MFM フレームのバイト列そのもの、SPEC §2.3)を読む。
// root-sink 本体の経路ではないが、golden fixture の照合に要る。
inline size_t read_raw_frames(const uint8_t* data, size_t size, std::vector<Bin>& out) {
  Reader r(data, size);
  uint32_t n = r.read_array_len();
  out.clear();
  out.reserve(n);
  for (uint32_t k = 0; k < n; ++k) out.push_back(r.read_bin());
  return out.size();
}

}  // namespace tpcwire

#endif  // TPCDAQ_ROOT_SINK_TPC_WIRE_HPP
