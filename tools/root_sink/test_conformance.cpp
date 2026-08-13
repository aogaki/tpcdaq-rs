// test_conformance.cpp — クロス言語適合性テスト(SPEC §10.4 方式)。
//
//   ./test_conformance <golden_stream.bin>
//
// Rust 本番エンコーダ(`tpcdaq::msg::write_golden_stream` / `golden_messages`)が吐いた
// 「u32 LE 長さ + ペイロード」の連結ストリームを、C++ 側の**本番デコーダ**(tpc_wire.hpp)で
// 読み、既知値と機械照合する。生成 → 照合の自動化は run_conformance.sh。
//
// フィクスチャはコミットしない(毎回再生成 = 陳腐化が構造的に起きない)。パス未指定 or
// ファイル不在なら SKIP して exit 0 —— C++ 単体の `make test` を cargo に依存させないため。

#include <cstdint>
#include <cstdio>
#include <cstring>
#include <fstream>
#include <string>
#include <vector>

#include "check.hpp"
#include "tpc_wire.hpp"

using namespace tpcwire;

// 「u32 LE 長さ + ペイロード」の連結を分解する(Rust 側 split_length_prefixed の対)。
static std::vector<std::pair<const uint8_t*, size_t>> split_length_prefixed(
    const std::vector<uint8_t>& buf) {
  std::vector<std::pair<const uint8_t*, size_t>> frames;
  size_t pos = 0;
  while (pos < buf.size()) {
    if (buf.size() - pos < 4) throw std::runtime_error("truncated length prefix");
    uint32_t len = static_cast<uint32_t>(buf[pos]) |
                   (static_cast<uint32_t>(buf[pos + 1]) << 8) |
                   (static_cast<uint32_t>(buf[pos + 2]) << 16) |
                   (static_cast<uint32_t>(buf[pos + 3]) << 24);
    pos += 4;
    if (buf.size() - pos < len) throw std::runtime_error("truncated frame");
    frames.emplace_back(buf.data() + pos, len);
    pos += len;
  }
  return frames;
}

int main(int argc, char** argv) {
  if (argc < 2) {
    std::printf("test_conformance: SKIP (no golden stream path given)\n");
    return 0;
  }
  const std::string path = argv[1];
  std::ifstream in(path, std::ios::binary);
  if (!in) {
    std::printf("test_conformance: SKIP (%s not found — run run_conformance.sh)\n",
                path.c_str());
    return 0;
  }
  std::vector<uint8_t> buf((std::istreambuf_iterator<char>(in)),
                           std::istreambuf_iterator<char>());
  std::printf("test_conformance: %s (%zu bytes)\n", path.c_str(), buf.size());

  auto frames = split_length_prefixed(buf);
  // golden_messages() は 4 通(RawFrames バッチ / Fragments バッチ / Heartbeat / EOS)
  CHECK_EQ(frames.size(), 4);
  if (frames.size() != 4) return tpccheck::report("test_conformance");

  // --- 1. Data(Batch<RawFrames>): receiver → graw-writer リンク(SPEC §2.3)-----
  {
    Envelope env = parse_envelope(frames[0].first, frames[0].second);
    CHECK(env.kind == MsgKind::Data);
    BatchHeader h = parse_batch_header(env.payload, env.payload_size);
    CHECK_EQ(h.source_id, 0);  // receiver = cobo_id 0
    CHECK_EQ(h.run_number, 7);
    CHECK_EQ(h.sequence_number, 0);
    CHECK(h.created_ns == 1755000000123456789ULL);
    std::vector<Bin> raw;
    read_raw_frames(h.payload, h.payload_size, raw);
    CHECK_EQ(raw.size(), 2);
    const uint8_t f0[] = {0x08, 0x05, 0x00, 0x01, 0x02, 0x03};
    const uint8_t f1[] = {0xff, 0x7f};
    CHECK_EQ(raw[0].size, sizeof(f0));
    CHECK(raw[0].size == sizeof(f0) && std::memcmp(raw[0].data, f0, sizeof(f0)) == 0);
    CHECK_EQ(raw[1].size, sizeof(f1));
    CHECK(raw[1].size == sizeof(f1) && std::memcmp(raw[1].data, f1, sizeof(f1)) == 0);
  }

  // --- 2. Data(Batch<Fragments>): decoder → root-sink リンク(本番経路)---------
  {
    Envelope env = parse_envelope(frames[1].first, frames[1].second);
    CHECK(env.kind == MsgKind::Data);
    BatchHeader h = parse_batch_header(env.payload, env.payload_size);
    CHECK_EQ(h.source_id, 100);  // decoder(SPEC §3.2)
    CHECK_EQ(h.run_number, 7);
    CHECK_EQ(h.sequence_number, 1);
    CHECK(h.created_ns == 1755000000223456789ULL);

    std::vector<FragmentView> frags;
    read_fragments(h.payload, h.payload_size, frags);
    CHECK_EQ(frags.size(), 1);
    if (frags.size() == 1) {
      const FragmentView& f = frags[0];
      CHECK_EQ(f.event_idx, 42);
      CHECK(f.event_time == 0x00001234'56789abcULL);
      CHECK_EQ(f.cobo, 0);
      CHECK_EQ(f.asad, 1);
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
      // items(SPEC §2.4 の 32bit パック。値は src/msg.rs の手計算出典と同じ)
      //   pack_item(0,   0,   0,    0)    = 0x00000000
      //   pack_item(2,  45, 300, 1234)    = 0x96cb04d2
      //   pack_item(3, 127, 511, 4095)    = 0xffffcfff(予約ビット [13:12] は 0)
      CHECK_EQ(f.items_size, 12);
      CHECK_EQ(f.item_count(), 3);
      if (f.item_count() == 3) {
        CHECK(f.item(0) == 0x00000000u);
        CHECK(f.item(1) == 0x96cb04d2u);
        CHECK(f.item(2) == 0xffffcfffu);
        CHECK((f.item(2) & 0x00003000u) == 0u);
      }
    }
  }

  // --- 3. Heartbeat ------------------------------------------------------------
  {
    Envelope env = parse_envelope(frames[2].first, frames[2].second);
    CHECK(env.kind == MsgKind::Heartbeat);
    CHECK_EQ(env.source_id, 0);
    CHECK_EQ(env.run_number, 7);
    CHECK_EQ(env.counter, 3);
  }

  // --- 4. EndOfStream(バイト列そのものも既知)----------------------------------
  {
    Envelope env = parse_envelope(frames[3].first, frames[3].second);
    CHECK(env.kind == MsgKind::EndOfStream);
    CHECK_EQ(env.source_id, 0);
    CHECK_EQ(env.run_number, 7);
    // 0x81 fixmap(1) / 0xab "EndOfStream" / 0x92 fixarray(2) / 0x00 0x07
    const uint8_t expect[] = {0x81, 0xab, 'E',  'n',  'd', 'O', 'f', 'S',
                              't',  'r',  'e',  'a',  'm', 0x92, 0x00, 0x07};
    CHECK_EQ(frames[3].second, sizeof(expect));
    CHECK(frames[3].second == sizeof(expect) &&
          std::memcmp(frames[3].first, expect, sizeof(expect)) == 0);
  }

  return tpccheck::report("test_conformance");
}
