// test_tpc_wire.cpp — tpc_wire.hpp の単体テスト(ZMQ 不要・ROOT 不要)。
//
//   g++ -std=c++17 -O2 -Wall -Wextra test_tpc_wire.cpp -o test_tpc_wire && ./test_tpc_wire
//
// バイト列はすべて**手で組む**(Rust 側エンコーダを一切呼ばない)。だからこそ
// 「Rust の実出力と突き合わせる」適合性テスト(test_conformance.cpp)と独立した
// オラクルになる。各配列には msgpack マーカーの出典コメントを付ける。
// SPEC §2.2(エンベロープ)/ §2.3(リンク別ペイロード)/ §2.4(Fragment)。

#include <cstdint>
#include <string>
#include <vector>

#include "check.hpp"
#include "tpc_wire.hpp"

using namespace tpcwire;

// 例外を投げるか(malformed 検出のテスト用)。
template <class F>
static bool throws(F f) {
  try {
    f();
  } catch (const std::exception&) {
    return true;
  }
  return false;
}

using Bytes = std::vector<uint8_t>;

// ---------------------------------------------------------------------------
// 既知バイト列(手計算の出典 = 各行のコメント)
// ---------------------------------------------------------------------------

// Message::EndOfStream { source_id: 1, run_number: 7 }
//   0x81                fixmap(1)
//   0xab "EndOfStream"  fixstr(11) = 0xa0|11
//   0x92                fixarray(2)  ← struct variant も positional
//   0x01 0x07           source_id=1, run_number=7(positive fixint)
// (src/msg.rs の end_of_stream_wire_bytes_are_exact と同一の 16 バイト)
static Bytes eos_1_7() {
  return {0x81, 0xab, 'E', 'n', 'd', 'O', 'f', 'S', 't', 'r',
          'e',  'a',  'm', 0x92, 0x01, 0x07};
}

// Message::Heartbeat { source_id: 0, run_number: 7, counter: 3 }
//   0x81 0xa9 "Heartbeat" 0x93 0x00 0x07 0x03
static Bytes heartbeat_0_7_3() {
  return {0x81, 0xa9, 'H', 'e', 'a', 'r', 't', 'b', 'e', 'a', 't', 0x93, 0x00, 0x07, 0x03};
}

// Message::Data(Batch<Fragments>) — 1 フラグメント。
//   0x81 0xa4 "Data"            fixmap(1) + fixstr(4)
//   0x95                        Batch = fixarray(5)
//     0x64                        source_id = 100(decoder、SPEC §3.2)
//     0x07                        run_number = 7
//     0xcd 0x01 0x2c              sequence_number = 300(uint16 BE: 0x012c)
//     0xcf 01 23 45 67 89 ab cd ef created_ns = 0x0123456789abcdef
//     0x91                        payload = fixarray(1) の Fragment
//       0x9c                        Fragment = fixarray(12)
//         0x2a                        event_idx = 42
//         0xcf 00 00 12 34 56 78 9a bc event_time = 0x123456789abc(48bit)
//         0x01                        cobo = 1
//         0x03                        asad = 3
//         0x02                        frame_type = 2(2025 compact)
//         0x05                        revision = 5
//         0xcc 0x88                   read_offset = 136(uint8)
//         0x00                        status = 0
//         0x94 0x44 0x00 0x11 0x03    mult = [68, 0, 17, 3]
//         0xcd 0x02 0x00              window_out = 512
//         0x94 0x07 0x09 0x0b 0x0d    last_cell = [7, 9, 11, 13]
//         0xc4 0x08 d2 04 cb 96 01 00 00 00
//                                     items = bin(8) = u32 LE ×2
//                                     = [0x96cb04d2, 0x00000001]
static Bytes data_fragments() {
  return {0x81, 0xa4, 'D',  'a',  't',  'a',
          0x95,
          0x64,
          0x07,
          0xcd, 0x01, 0x2c,
          0xcf, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
          0x91,
          0x9c,
          0x2a,
          0xcf, 0x00, 0x00, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc,
          0x01,
          0x03,
          0x02,
          0x05,
          0xcc, 0x88,
          0x00,
          0x94, 0x44, 0x00, 0x11, 0x03,
          0xcd, 0x02, 0x00,
          0x94, 0x07, 0x09, 0x0b, 0x0d,
          0xc4, 0x08, 0xd2, 0x04, 0xcb, 0x96, 0x01, 0x00, 0x00, 0x00};
}

// Message::Data(Batch<RawFrames>) — receiver → graw-writer リンク(SPEC §2.3)。
//   0x81 0xa4 "Data" 0x95
//     0x00        source_id = 0(cobo_id 0)
//     0x07        run_number = 7
//     0x00        sequence_number = 0
//     0x05        created_ns = 5
//     0x92        payload = fixarray(2)
//       0xc4 0x06 08 05 00 01 02 03   bin(6)
//       0xc4 0x02 ff 7f               bin(2)  ← 長さ非対称
static Bytes data_raw_frames() {
  return {0x81, 0xa4, 'D', 'a', 't', 'a', 0x95, 0x00, 0x07, 0x00, 0x05, 0x92,
          0xc4, 0x06, 0x08, 0x05, 0x00, 0x01, 0x02, 0x03,
          0xc4, 0x02, 0xff, 0x7f};
}

// ---------------------------------------------------------------------------
// 1. エンベロープ(SPEC §2.2)
// ---------------------------------------------------------------------------

static void test_end_of_stream_known_bytes() {
  Bytes b = eos_1_7();
  Envelope env = parse_envelope(b.data(), b.size());
  CHECK(env.kind == MsgKind::EndOfStream);
  CHECK(env.variant == "EndOfStream");
  CHECK_EQ(env.source_id, 1);
  CHECK_EQ(env.run_number, 7);
}

static void test_heartbeat_known_bytes() {
  Bytes b = heartbeat_0_7_3();
  Envelope env = parse_envelope(b.data(), b.size());
  CHECK(env.kind == MsgKind::Heartbeat);
  CHECK_EQ(env.source_id, 0);
  CHECK_EQ(env.run_number, 7);
  CHECK_EQ(env.counter, 3);
}

static void test_data_envelope_points_at_the_batch() {
  Bytes b = data_fragments();
  Envelope env = parse_envelope(b.data(), b.size());
  CHECK(env.kind == MsgKind::Data);
  CHECK(env.variant == "Data");
  // "Data" キーの直後 = Batch の fixarray(5) マーカー。0x81 0xa4 + 4 文字 = 6 バイト目。
  CHECK(env.payload == b.data() + 6);
  CHECK_EQ(env.payload_size, b.size() - 6);
  CHECK_EQ(*env.payload, 0x95);
}

// 未知バリアントは「壊れている」ではなく「知らない」— 名前を持って返し、
// 呼び手が可視化できるようにする(silent failure を作らない)。
static void test_unknown_variant_is_reported_by_name() {
  Bytes b = {0x81, 0xa3, 'F', 'o', 'o', 0x90};  // fixmap(1) "Foo" -> fixarray(0)
  Envelope env = parse_envelope(b.data(), b.size());
  CHECK(env.kind == MsgKind::Unknown);
  CHECK(env.variant == "Foo");
}

static void test_malformed_envelopes_throw() {
  // トップレベルが map ではない(array)
  Bytes arr = {0x92, 0x01, 0x02};
  CHECK(throws([&] { parse_envelope(arr.data(), arr.size()); }));
  // fixmap(2) — Message は必ず 1 要素
  Bytes map2 = {0x82, 0xa1, 'a', 0x01, 0xa1, 'b', 0x02};
  CHECK(throws([&] { parse_envelope(map2.data(), map2.size()); }));
  // 途中で切れている
  Bytes trunc = eos_1_7();
  trunc.resize(trunc.size() - 1);
  CHECK(throws([&] { parse_envelope(trunc.data(), trunc.size()); }));
  // 空
  CHECK(throws([&] { parse_envelope(nullptr, 0); }));
}

// ---------------------------------------------------------------------------
// 2. Batch ヘッダ(SPEC §2.2 positional array(5))
// ---------------------------------------------------------------------------

static void test_batch_header_fields() {
  Bytes b = data_fragments();
  Envelope env = parse_envelope(b.data(), b.size());
  BatchHeader h = parse_batch_header(env.payload, env.payload_size);
  CHECK_EQ(h.source_id, 100);
  CHECK_EQ(h.run_number, 7);
  CHECK_EQ(h.sequence_number, 300);
  CHECK(h.created_ns == 0x0123456789abcdefULL);
  CHECK_EQ(*h.payload, 0x91);  // payload = fixarray(1)
}

// 前方互換(delila 方式): 先頭 5 フィールドを読み、増えた末尾は無視する。
static void test_batch_with_extra_trailing_field_still_parses() {
  Bytes b = data_fragments();
  b[6] = 0x96;         // fixarray(5) -> fixarray(6)
  b.push_back(0xc0);   // 6 番目のフィールド = nil(将来の追加フィールドを模す)
  Envelope env = parse_envelope(b.data(), b.size());
  BatchHeader h = parse_batch_header(env.payload, env.payload_size);
  CHECK_EQ(h.source_id, 100);
  CHECK_EQ(h.sequence_number, 300);
  std::vector<FragmentView> frags;
  read_fragments(h.payload, h.payload_size, frags);
  CHECK_EQ(frags.size(), 1);
}

static void test_short_batch_throws() {
  // fixarray(4) — 必須 5 フィールドに足りない = プロトコル不一致
  Bytes b = {0x94, 0x64, 0x07, 0x00, 0x05};
  CHECK(throws([&] { parse_batch_header(b.data(), b.size()); }));
}

// ---------------------------------------------------------------------------
// 3. Fragment(SPEC §2.4 positional array(12))
// ---------------------------------------------------------------------------

static void test_fragment_fields_and_items() {
  Bytes b = data_fragments();
  Envelope env = parse_envelope(b.data(), b.size());
  BatchHeader h = parse_batch_header(env.payload, env.payload_size);
  std::vector<FragmentView> frags;
  size_t n = read_fragments(h.payload, h.payload_size, frags);
  CHECK_EQ(n, 1);
  CHECK_EQ(frags.size(), 1);
  const FragmentView& f = frags[0];
  CHECK_EQ(f.event_idx, 42);
  CHECK(f.event_time == 0x123456789abcULL);
  CHECK_EQ(f.cobo, 1);
  CHECK_EQ(f.asad, 3);
  CHECK_EQ(f.frame_type, 2);
  CHECK_EQ(f.revision, 5);
  CHECK_EQ(f.read_offset, 136);
  CHECK_EQ(f.status, 0);
  CHECK_EQ(f.mult[0], 68);
  CHECK_EQ(f.mult[1], 0);
  CHECK_EQ(f.mult[2], 17);
  CHECK_EQ(f.mult[3], 3);
  CHECK_EQ(f.window_out, 512);
  CHECK_EQ(f.last_cell[0], 7);
  CHECK_EQ(f.last_cell[1], 9);
  CHECK_EQ(f.last_cell[2], 11);
  CHECK_EQ(f.last_cell[3], 13);
  // items = bin(8) = u32 LE ×2
  CHECK_EQ(f.items_size, 8);
  CHECK_EQ(f.item_count(), 2);
  CHECK(f.item(0) == 0x96cb04d2u);  // d2 04 cb 96(LE)
  CHECK(f.item(1) == 0x00000001u);
}

static void test_fragment_with_extra_trailing_field_still_parses() {
  Bytes b = data_fragments();
  b[22] = 0x9d;       // Fragment fixarray(12) -> fixarray(13)(index 22 = 0x9c)
  b.push_back(0x2a);  // 13 番目 = 42(将来の追加フィールド)
  Envelope env = parse_envelope(b.data(), b.size());
  BatchHeader h = parse_batch_header(env.payload, env.payload_size);
  std::vector<FragmentView> frags;
  read_fragments(h.payload, h.payload_size, frags);
  CHECK_EQ(frags.size(), 1);
  CHECK_EQ(frags[0].event_idx, 42);
  CHECK_EQ(frags[0].item_count(), 2);
}

static void test_short_fragment_throws() {
  Bytes b = data_fragments();
  b[22] = 0x9b;  // fixarray(11) — 必須 12 フィールドに足りない(index 22 = 0x9c)
  Envelope env = parse_envelope(b.data(), b.size());
  BatchHeader h = parse_batch_header(env.payload, env.payload_size);
  std::vector<FragmentView> frags;
  CHECK(throws([&] { read_fragments(h.payload, h.payload_size, frags); }));
}

// items は u32 LE の連結。4 の倍数でない長さは境界ずれ = malformed。
static void test_misaligned_items_throw() {
  Bytes b = data_fragments();
  b[b.size() - 8 - 1] = 0x06;  // bin(8) -> bin(6)
  b.resize(b.size() - 2);
  Envelope env = parse_envelope(b.data(), b.size());
  BatchHeader h = parse_batch_header(env.payload, env.payload_size);
  std::vector<FragmentView> frags;
  CHECK(throws([&] { read_fragments(h.payload, h.payload_size, frags); }));
}

// items が bin ではなく array で来たら(Rust 側が serde_bytes を外した等)必ず落ちる。
static void test_items_as_array_throws() {
  Bytes b = data_fragments();
  size_t items_marker = b.size() - 10;  // 0xc4 の位置
  CHECK_EQ(b[items_marker], 0xc4);
  b[items_marker] = 0x98;  // bin8 -> fixarray(8)
  b.erase(b.begin() + static_cast<long>(items_marker) + 1);  // 長さバイトを外す
  Envelope env = parse_envelope(b.data(), b.size());
  BatchHeader h = parse_batch_header(env.payload, env.payload_size);
  std::vector<FragmentView> frags;
  CHECK(throws([&] { read_fragments(h.payload, h.payload_size, frags); }));
}

// ---------------------------------------------------------------------------
// 4. RawFrames(SPEC §2.3。root-sink 本体は使わないが、golden fixture の照合に要る)
// ---------------------------------------------------------------------------

static void test_raw_frames_are_bins() {
  Bytes b = data_raw_frames();
  Envelope env = parse_envelope(b.data(), b.size());
  BatchHeader h = parse_batch_header(env.payload, env.payload_size);
  CHECK_EQ(h.source_id, 0);
  CHECK_EQ(h.sequence_number, 0);
  std::vector<Bin> frames;
  read_raw_frames(h.payload, h.payload_size, frames);
  CHECK_EQ(frames.size(), 2);
  CHECK_EQ(frames[0].size, 6);
  CHECK_EQ(frames[0].data[0], 0x08);
  CHECK_EQ(frames[0].data[5], 0x03);
  CHECK_EQ(frames[1].size, 2);
  CHECK_EQ(frames[1].data[0], 0xff);
  CHECK_EQ(frames[1].data[1], 0x7f);
}

// Fragments を期待している所へ RawFrames が来たら黙って通さない(bin ≠ array(12))。
static void test_raw_frames_read_as_fragments_throws() {
  Bytes b = data_raw_frames();
  Envelope env = parse_envelope(b.data(), b.size());
  BatchHeader h = parse_batch_header(env.payload, env.payload_size);
  std::vector<FragmentView> frags;
  CHECK(throws([&] { read_fragments(h.payload, h.payload_size, frags); }));
}

int main() {
  test_end_of_stream_known_bytes();
  test_heartbeat_known_bytes();
  test_data_envelope_points_at_the_batch();
  test_unknown_variant_is_reported_by_name();
  test_malformed_envelopes_throw();
  test_batch_header_fields();
  test_batch_with_extra_trailing_field_still_parses();
  test_short_batch_throws();
  test_fragment_fields_and_items();
  test_fragment_with_extra_trailing_field_still_parses();
  test_short_fragment_throws();
  test_misaligned_items_throw();
  test_items_as_array_throws();
  test_raw_frames_are_bins();
  test_raw_frames_read_as_fragments_throws();
  return tpccheck::report("test_tpc_wire");
}
