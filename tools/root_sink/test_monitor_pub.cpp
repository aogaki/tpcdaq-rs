// test_monitor_pub.cpp — monitor_pub.hpp(モニタ PUB のワイヤ直列化)の単体テスト。
// TODO/022 / SPEC §5.3(v1.9 確定)+ §2.2(エンベロープ)+ §2.4(Fragment)。
//
//   g++ -std=c++17 -O2 -Wall -Wextra test_monitor_pub.cpp -o test_monitor_pub
//   ./test_monitor_pub
//
// ROOT 不要・ZMQ 不要(monitor_pub.hpp は bytes を組むだけ)。
//
// **オラクルは手組みの MessagePack バイト列**。期待値ビルダ(`Expect`)は monitor_pub.hpp の
// エンコーダを一切使わない別実装で、型タグ・キー順・整数幅をこのファイルに明示する ——
// つまりこのファイルが SPEC §5.3 のワイヤ形式の実行可能な写しになっている。
// Fragment の再直列化だけは **本番パーサ(tpc_wire.hpp)に読み戻して**照合する
// (リポ内適合性テスト = 008 の流儀)。

#include <cstdint>
#include <cstdio>
#include <cstring>
#include <string>
#include <vector>

#include "check.hpp"
#include "monitor_pub.hpp"
#include "tpc_wire.hpp"

namespace {

// ---------------------------------------------------------------------------
// 手組み MessagePack ビルダ(期待値専用。monitor_pub.hpp とは無関係の別実装)
// ---------------------------------------------------------------------------

struct Expect {
  std::vector<uint8_t> b;

  void raw(uint8_t v) { b.push_back(v); }
  void raw(std::initializer_list<uint8_t> vs) {
    for (uint8_t v : vs) b.push_back(v);
  }
  // fixmap / fixarray / fixstr は 15 / 15 / 31 まで。テストの期待値はその範囲で組む。
  void fixmap(uint8_t n) { b.push_back(static_cast<uint8_t>(0x80u | n)); }
  void fixarray(uint8_t n) { b.push_back(static_cast<uint8_t>(0x90u | n)); }
  void str(const char* s) {
    const size_t n = std::strlen(s);
    b.push_back(static_cast<uint8_t>(0xa0u | n));  // fixstr(< 32)
    for (size_t i = 0; i < n; ++i) b.push_back(static_cast<uint8_t>(s[i]));
  }
  void fixint(uint8_t v) { b.push_back(v); }         // 0..127
  void u8(uint8_t v) { raw({0xcc, v}); }             // 128..255
  void u16(uint16_t v) { raw({0xcd, static_cast<uint8_t>(v >> 8), static_cast<uint8_t>(v)}); }
  void boolean(bool v) { b.push_back(v ? 0xc3 : 0xc2); }
  void bin8(const std::vector<uint8_t>& payload) {
    raw({0xc4, static_cast<uint8_t>(payload.size())});
    for (uint8_t v : payload) b.push_back(v);
  }
};

std::string hex(const std::vector<uint8_t>& v, size_t from, size_t n) {
  std::string out;
  char buf[4];
  for (size_t i = from; i < v.size() && i < from + n; ++i) {
    std::snprintf(buf, sizeof(buf), "%02x ", v[i]);
    out += buf;
  }
  return out;
}

// 差分位置を出す照合(落ちたときにどのバイトから違うかが分かる)。
void check_bytes(const std::vector<uint8_t>& got, const std::vector<uint8_t>& want,
                 const char* what) {
  if (got == want) {
    CHECK(true);
    return;
  }
  size_t i = 0;
  while (i < got.size() && i < want.size() && got[i] == want[i]) ++i;
  std::printf("FAIL %s: first difference at byte %zu (len got=%zu want=%zu)\n  got  %s\n  want %s\n",
              what, i, got.size(), want.size(), hex(got, i, 16).c_str(),
              hex(want, i, 16).c_str());
  CHECK(false);
}

// ---------------------------------------------------------------------------
// 1. status(SPEC §5.3)— 全バイト照合
// ---------------------------------------------------------------------------
//
// 手計算の出典(値はすべて取り違えが見えるようにずらしてある):
//   envelope  = fixmap(1){"Data": array(5)[101, 7, 0, 42, payload]}
//               source_id 101 = SPEC §3.2 v1.9(root-sink のモニタ PUB)
//   payload   = fixmap(10){ kind, run, state, events_built, events_incomplete,
//                           late_fragments, frames_per_cobo, bytes_written,
//                           saturation, publish_drops }
//   frames_per_cobo は **値が 0 でない CoBo だけ**を 10 進文字列キーで(cobo 昇順)。
//   bytes_written = 200 は fixint(≤127)に入らないので uint8(0xcc)。
void test_status_bytes() {
  rsmon::Status s;
  s.run = 7;
  s.running = true;
  s.events_built = 5;
  s.events_incomplete = 1;
  s.late_fragments = 2;
  s.frames_per_cobo[0] = 3;
  s.frames_per_cobo[2] = 4;
  s.bytes_written = 200;
  s.saturation[0] = rsmon::Saturation{1, 10};
  s.saturation[1] = rsmon::Saturation{0, 20};
  s.saturation[2] = rsmon::Saturation{3, 30};
  s.publish_drops = 6;

  rsmon::Encoder enc;
  std::vector<uint8_t> out;
  enc.status(s, /*created_ns=*/42, out);

  Expect e;
  e.fixmap(1);
  e.str("Data");
  e.fixarray(5);
  e.fixint(101);  // source_id
  e.fixint(7);    // run_number
  e.fixint(0);    // sequence_number(最初の 1 通)
  e.fixint(42);   // created_ns
  e.fixmap(10);
  e.str("kind");              e.str("status");
  e.str("run");               e.fixint(7);
  e.str("state");             e.str("running");
  e.str("events_built");      e.fixint(5);
  e.str("events_incomplete"); e.fixint(1);
  e.str("late_fragments");    e.fixint(2);
  e.str("frames_per_cobo");
  e.fixmap(2);
  e.str("0");                 e.fixint(3);
  e.str("2");                 e.fixint(4);
  e.str("bytes_written");     e.u8(200);
  e.str("saturation");
  e.fixmap(3);
  e.str("U"); e.fixmap(2); e.str("saturated"); e.fixint(1); e.str("counted"); e.fixint(10);
  e.str("V"); e.fixmap(2); e.str("saturated"); e.fixint(0); e.str("counted"); e.fixint(20);
  e.str("W"); e.fixmap(2); e.str("saturated"); e.fixint(3); e.str("counted"); e.fixint(30);
  e.str("publish_drops");     e.fixint(6);

  check_bytes(out, e.b, "status");
}

// idle(run 外)の status も**常に**流れる。state は "idle"、run は直近クローズした run。
void test_status_idle_state_string() {
  rsmon::Status s;
  s.run = 0;
  s.running = false;
  rsmon::Encoder enc;
  std::vector<uint8_t> out;
  enc.status(s, /*created_ns=*/1, out);

  // 本番パーサでエンベロープを剥がして、payload の "state" を読む。
  tpcwire::Envelope env = tpcwire::parse_envelope(out.data(), out.size());
  CHECK(env.kind == tpcwire::MsgKind::Data);
  tpcwire::BatchHeader h = tpcwire::parse_batch_header(env.payload, env.payload_size);
  CHECK_EQ(h.source_id, 101);
  CHECK_EQ(h.run_number, 0);
  CHECK_EQ(h.created_ns, 1);

  tpcwire::Reader r(h.payload, h.payload_size);
  const uint32_t n = r.read_map_len();
  CHECK_EQ(n, 10);
  bool saw_idle = false;
  for (uint32_t i = 0; i < n; ++i) {
    const std::string key = r.read_str();
    if (key == "state") {
      saw_idle = (r.read_str() == "idle");
    } else {
      r.skip_value();
    }
  }
  CHECK(saw_idle);
  // frames_per_cobo が空(全 CoBo 0 件)でも map は出る
  CHECK(out.size() > 0);
}

// ---------------------------------------------------------------------------
// 2. hist_snapshot(SPEC §5.3)— 全バイト照合 + f64 LE の添字順
// ---------------------------------------------------------------------------
//
// bins は **f64 リトルエンディアンの生バイト列**(msgpack の float64 連打だと 9 B/ビンに
// なる ——SPEC がそう定めた理由)。手計算の出典(IEEE754 binary64):
//   0.0 → 00 00 00 00 00 00 00 00
//   1.0 = 0x3FF0000000000000 → LE 00 00 00 00 00 00 f0 3f
//   2.5 = 0x4004000000000000 → LE 00 00 00 00 00 00 04 40
//   3.0 = 0x4008000000000000 → LE 00 00 00 00 00 00 08 40
//
// 添字順は SPEC §5.3: 2D は (strip-1)*512 + bucket(strip が遅い軸)、1D は 0..511。
// このテストでは 9 枚を **nx/ny を極小にした受け皿**で組み、バイト列を手で並べる。
rsmon::HistSnapshot tiny_snapshot() {
  const char* kNames[9] = {"StripTimeU", "StripTimeV", "StripTimeW",
                           "ChargeU",    "ChargeV",    "ChargeW",
                           "ChargeMaxU", "ChargeMaxV", "ChargeMaxW"};
  rsmon::HistSnapshot snap;
  for (size_t i = 0; i < rsmon::kHistCount; ++i) {
    snap[i].id = static_cast<uint8_t>(i + 1);
    snap[i].name = kNames[i];
    if (i < 3) {
      snap[i].nx = 1;
      snap[i].ny = 2;
      snap[i].bins = {1.0, 2.5};
    } else {
      snap[i].nx = 2;
      snap[i].ny = 1;
      snap[i].bins = {3.0, 0.0};
    }
  }
  return snap;
}

void test_hist_snapshot_bytes() {
  rsmon::Encoder enc;
  std::vector<uint8_t> out;
  enc.hist_snapshot(/*run=*/9, tiny_snapshot(), /*created_ns=*/3, out);

  const std::vector<uint8_t> k2d = {0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xf0, 0x3f,
                                    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x40};
  const std::vector<uint8_t> k1d = {0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x40,
                                    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00};
  const char* kNames[9] = {"StripTimeU", "StripTimeV", "StripTimeW",
                           "ChargeU",    "ChargeV",    "ChargeW",
                           "ChargeMaxU", "ChargeMaxV", "ChargeMaxW"};

  Expect e;
  e.fixmap(1);
  e.str("Data");
  e.fixarray(5);
  e.fixint(101);
  e.fixint(9);  // run_number
  e.fixint(0);  // sequence_number
  e.fixint(3);  // created_ns
  e.fixmap(3);
  e.str("kind"); e.str("hist_snapshot");
  e.str("run");  e.fixint(9);
  e.str("hists");
  e.fixarray(9);
  for (int i = 0; i < 9; ++i) {
    e.fixmap(5);
    e.str("id");   e.fixint(static_cast<uint8_t>(i + 1));
    e.str("name"); e.str(kNames[i]);
    e.str("nx");   e.fixint(i < 3 ? 1 : 2);
    e.str("ny");   e.fixint(i < 3 ? 2 : 1);
    e.str("bins"); e.bin8(i < 3 ? k2d : k1d);
  }
  check_bytes(out, e.b, "hist_snapshot");
}

// bins の並びが (strip-1)*512 + bucket であること = 「ビン列を 8 バイト毎に切ると
// 集計器の bins() と同じ順序で出てくる」ことを、大きめの 2D で機械照合する。
void test_hist_snapshot_bin_order_is_row_major_by_strip() {
  rsmon::HistSnapshot snap = tiny_snapshot();
  snap[0].nx = 3;
  snap[0].ny = 4;
  snap[0].bins.assign(12, 0.0);
  for (size_t i = 0; i < 12; ++i) snap[0].bins[i] = static_cast<double>(i) + 0.5;

  rsmon::Encoder enc;
  std::vector<uint8_t> out;
  enc.hist_snapshot(/*run=*/1, snap, /*created_ns=*/0, out);

  // 本番パーサでエンベロープ → payload map → hists[0].bins を取り出す。
  tpcwire::Envelope env = tpcwire::parse_envelope(out.data(), out.size());
  tpcwire::BatchHeader h = tpcwire::parse_batch_header(env.payload, env.payload_size);
  tpcwire::Reader r(h.payload, h.payload_size);
  const uint32_t nkeys = r.read_map_len();
  tpcwire::Bin bins{};
  bool found = false;
  for (uint32_t i = 0; i < nkeys; ++i) {
    if (r.read_str() != "hists") {
      r.skip_value();
      continue;
    }
    CHECK_EQ(r.read_array_len(), 9);
    const uint32_t nfields = r.read_map_len();
    CHECK_EQ(nfields, 5);
    for (uint32_t k = 0; k < nfields; ++k) {
      if (r.read_str() == "bins") {
        bins = r.read_bin();
        found = true;
      } else {
        r.skip_value();
      }
    }
    break;
  }
  CHECK(found);
  CHECK_EQ(bins.size, 12u * 8u);
  for (size_t i = 0; i < 12 && found; ++i) {
    double v = 0.0;
    std::memcpy(&v, bins.data + i * 8, 8);  // ホストは LE(x86/ARM64)
    CHECK(v == static_cast<double>(i) + 0.5);
  }
}

// ---------------------------------------------------------------------------
// 3. built_event(SPEC §5.3 + §2.4)— 全バイト照合
// ---------------------------------------------------------------------------
//
// fragments は §2.4 の positional array(12) の**再直列化**:
//   [event_idx, event_time, cobo, asad, frame_type, revision, read_offset, status,
//    mult[4], window_out, last_cell[4], items(bin)]
// items は u32 LE の連結。ここでは 1 サンプル(aget1, chan5, bucket3, adc100):
//   pack = (1<<30)|(5<<23)|(3<<14)|100 = 0x4280C064 → LE 64 c0 80 42
rootsink::OwnedFragment sample_fragment() {
  rootsink::OwnedFragment f;
  f.run_number = 7;
  f.event_idx = 41;
  f.event_time = 300;
  f.cobo = 1;
  f.asad = 2;
  f.frame_type = 2;
  f.revision = 5;
  f.read_offset = 136;
  f.status = 3;
  f.mult[0] = 68;
  f.mult[1] = 0;
  f.mult[2] = 17;
  f.mult[3] = 9;
  f.window_out = 512;
  f.last_cell[0] = 7;
  f.last_cell[1] = 9;
  f.last_cell[2] = 11;
  f.last_cell[3] = 13;
  const uint32_t word = (1u << 30) | (5u << 23) | (3u << 14) | 100u;
  f.items = {static_cast<uint8_t>(word & 0xFF), static_cast<uint8_t>((word >> 8) & 0xFF),
             static_cast<uint8_t>((word >> 16) & 0xFF),
             static_cast<uint8_t>((word >> 24) & 0xFF)};
  return f;
}

void test_built_event_bytes() {
  rootsink::BuiltEvent ev;
  ev.run_number = 7;
  ev.event_idx = 41;
  ev.incomplete = false;  // ワイヤは complete = true(反転して載せる)
  ev.fragments.push_back(sample_fragment());

  rsmon::Encoder enc;
  std::vector<uint8_t> out;
  enc.built_event(ev, /*created_ns=*/4, out);

  Expect e;
  e.fixmap(1);
  e.str("Data");
  e.fixarray(5);
  e.fixint(101);
  e.fixint(7);  // run_number
  e.fixint(0);  // sequence_number
  e.fixint(4);  // created_ns
  e.fixmap(5);
  e.str("kind");      e.str("built_event");
  e.str("run");       e.fixint(7);
  e.str("event_idx"); e.fixint(41);
  e.str("complete");  e.boolean(true);
  e.str("fragments");
  e.fixarray(1);
  e.raw(0x9c);        // fixarray(12) = Fragment(SPEC §2.4)
  e.fixint(41);       // event_idx
  e.u16(300);         // event_time(> 255 なので uint16)
  e.fixint(1);        // cobo
  e.fixint(2);        // asad
  e.fixint(2);        // frame_type
  e.fixint(5);        // revision
  e.u8(136);          // read_offset
  e.fixint(3);        // status
  e.fixarray(4); e.fixint(68); e.fixint(0); e.fixint(17); e.fixint(9);  // mult
  e.u16(512);         // window_out
  e.fixarray(4); e.fixint(7); e.fixint(9); e.fixint(11); e.fixint(13);  // last_cell
  e.bin8({0x64, 0xc0, 0x80, 0x42});                                     // items(u32 LE)

  check_bytes(out, e.b, "built_event");
}

// incomplete = true のイベントは complete = **false** で載る(反転を取り違えない)。
void test_built_event_complete_flag_is_inverted() {
  rootsink::BuiltEvent ev;
  ev.run_number = 7;
  ev.event_idx = 1;
  ev.incomplete = true;
  ev.fragments.push_back(sample_fragment());

  rsmon::Encoder enc;
  std::vector<uint8_t> out;
  enc.built_event(ev, 0, out);

  tpcwire::Envelope env = tpcwire::parse_envelope(out.data(), out.size());
  tpcwire::BatchHeader h = tpcwire::parse_batch_header(env.payload, env.payload_size);
  tpcwire::Reader r(h.payload, h.payload_size);
  const uint32_t nkeys = r.read_map_len();
  bool checked = false;
  for (uint32_t i = 0; i < nkeys; ++i) {
    if (r.read_str() == "complete") {
      CHECK_EQ(*r.ptr(), 0xc2);  // msgpack false
      checked = true;
      r.skip_value();
    } else {
      r.skip_value();
    }
  }
  CHECK(checked);
}

// ---------------------------------------------------------------------------
// 4. Fragment 再直列化の round-trip(本番パーサ tpc_wire.hpp で読み戻す)
// ---------------------------------------------------------------------------

// payload map の `key` の値の先頭に位置づけた Reader を返す。
tpcwire::Reader seek_value(const uint8_t* data, size_t size, const char* key) {
  tpcwire::Reader r(data, size);
  const uint32_t n = r.read_map_len();
  for (uint32_t i = 0; i < n; ++i) {
    if (r.read_str() == key) return r;
    r.skip_value();
  }
  throw std::runtime_error(std::string("key not found: ") + key);
}

void test_fragments_round_trip_through_the_production_parser() {
  rootsink::BuiltEvent ev;
  ev.run_number = 11;
  ev.event_idx = 4242;  // > 255(整数幅の切り替わりを跨ぐ)
  ev.incomplete = false;

  // 2 フラグメント。ヘッダは全部ずらして、取り違えが値で見えるようにする。
  rootsink::OwnedFragment a = sample_fragment();
  a.event_idx = 4242;
  a.event_time = 0x0000'1234'5678'9abcULL;  // 48bit 有効
  a.cobo = 0;
  a.asad = 3;
  a.read_offset = 65535;
  a.window_out = 100000;  // > 65535(uint32 幅)
  rootsink::OwnedFragment b = sample_fragment();
  b.event_idx = 4242;
  b.event_time = 1;
  b.cobo = 1;
  b.asad = 0;
  b.mult[0] = 1;
  b.mult[3] = 511;
  b.last_cell[2] = 500;
  b.items.clear();
  for (uint32_t k = 0; k < 300; ++k) {  // bin8 では収まらない長さ(> 255 B)
    const uint32_t word = (k % 4u) << 30 | ((k * 7u) % 68u) << 23 | ((k * 13u) % 512u) << 14 |
                          ((k * 37u) % 4096u);
    b.items.push_back(static_cast<uint8_t>(word & 0xFF));
    b.items.push_back(static_cast<uint8_t>((word >> 8) & 0xFF));
    b.items.push_back(static_cast<uint8_t>((word >> 16) & 0xFF));
    b.items.push_back(static_cast<uint8_t>((word >> 24) & 0xFF));
  }
  ev.fragments.push_back(a);
  ev.fragments.push_back(b);

  rsmon::Encoder enc;
  std::vector<uint8_t> out;
  enc.built_event(ev, 1'755'000'000'000'000'000ULL, out);

  tpcwire::Envelope env = tpcwire::parse_envelope(out.data(), out.size());
  CHECK(env.kind == tpcwire::MsgKind::Data);
  tpcwire::BatchHeader h = tpcwire::parse_batch_header(env.payload, env.payload_size);
  CHECK_EQ(h.run_number, 11);
  CHECK_EQ(h.created_ns, 1'755'000'000'000'000'000ULL);

  tpcwire::Reader r = seek_value(h.payload, h.payload_size, "fragments");
  std::vector<tpcwire::FragmentView> got;
  CHECK_EQ(tpcwire::read_fragments(r.ptr(), r.remaining(), got), 2);
  if (got.size() != 2) return;

  for (size_t i = 0; i < 2; ++i) {
    const rootsink::OwnedFragment& want = ev.fragments[i];
    const tpcwire::FragmentView& g = got[i];
    CHECK_EQ(g.event_idx, want.event_idx);
    CHECK_EQ(g.event_time, want.event_time);
    CHECK_EQ(g.cobo, want.cobo);
    CHECK_EQ(g.asad, want.asad);
    CHECK_EQ(g.frame_type, want.frame_type);
    CHECK_EQ(g.revision, want.revision);
    CHECK_EQ(g.read_offset, want.read_offset);
    CHECK_EQ(g.status, want.status);
    for (int k = 0; k < 4; ++k) {
      CHECK_EQ(g.mult[k], want.mult[k]);
      CHECK_EQ(g.last_cell[k], want.last_cell[k]);
    }
    CHECK_EQ(g.window_out, want.window_out);
    CHECK_EQ(g.items_size, want.items.size());
    CHECK(std::memcmp(g.items, want.items.data(), want.items.size()) == 0);
  }
}

// ---------------------------------------------------------------------------
// 5. sequence_number(SPEC §5.3 v1.9 ③: 全 kind 単一系列・run リセット無し)
// ---------------------------------------------------------------------------

uint64_t read_sequence(const std::vector<uint8_t>& msg) {
  tpcwire::Envelope env = tpcwire::parse_envelope(msg.data(), msg.size());
  return tpcwire::parse_batch_header(env.payload, env.payload_size).sequence_number;
}

void test_sequence_is_one_series_across_kinds_and_never_resets() {
  rsmon::Encoder enc;
  std::vector<uint8_t> out;
  rsmon::Status s;
  s.run = 1;
  s.running = true;
  rootsink::BuiltEvent ev;
  ev.run_number = 1;
  ev.fragments.push_back(sample_fragment());

  enc.status(s, 0, out);
  CHECK_EQ(read_sequence(out), 0);
  enc.hist_snapshot(1, tiny_snapshot(), 0, out);
  CHECK_EQ(read_sequence(out), 1);
  enc.built_event(ev, 0, out);
  CHECK_EQ(read_sequence(out), 2);
  enc.status(s, 0, out);
  CHECK_EQ(read_sequence(out), 3);

  // **run が変わっても 0 に戻らない**(§2.2 の 0 リセット規則の例外。モニタリンクの
  // sequence_number の用途は「ギャップ = ドロップ数」の可視化だけ)。
  s.run = 2;
  ev.run_number = 2;
  enc.status(s, 0, out);
  CHECK_EQ(read_sequence(out), 4);
  enc.built_event(ev, 0, out);
  CHECK_EQ(read_sequence(out), 5);
  CHECK_EQ(enc.sequence(), 6);
}

// 出力バッファは使い回される(前の中身が残らない = ホットパスで確保し直さない)。
void test_output_buffer_is_reused_without_leftovers() {
  rsmon::Encoder enc;
  std::vector<uint8_t> out;
  rootsink::BuiltEvent big;
  big.run_number = 1;
  rootsink::OwnedFragment fat = sample_fragment();
  fat.items.assign(4000, 0x5a);  // ≒ 4 KB —— status(数百 B)より確実に大きい
  big.fragments.push_back(fat);
  big.fragments.push_back(fat);
  enc.built_event(big, 0, out);
  const size_t big_size = out.size();
  const size_t big_capacity = out.capacity();

  rsmon::Status s;
  enc.status(s, 0, out);
  CHECK(out.size() < big_size);
  CHECK_EQ(out.capacity(), big_capacity);  // 確保し直していない
  // 残骸が無いこと = 本番パーサで最後まで読み切れること
  tpcwire::Envelope env = tpcwire::parse_envelope(out.data(), out.size());
  tpcwire::BatchHeader h = tpcwire::parse_batch_header(env.payload, env.payload_size);
  tpcwire::Reader r(h.payload, h.payload_size);
  const uint32_t n = r.read_map_len();
  for (uint32_t i = 0; i < n; ++i) {
    r.read_str();
    r.skip_value();
  }
  CHECK(r.at_end());
}

// ---------------------------------------------------------------------------
// 6. 整数幅の切り替わり(大きい値でも本番パーサが同じ値を返す)
// ---------------------------------------------------------------------------

void test_large_scalars_use_wider_msgpack_integers() {
  rsmon::Status s;
  s.run = 4'000'000'000u;         // > 2^31(u32 の上端側)
  s.events_built = 1ULL << 40;    // uint64
  s.bytes_written = 70000;        // uint32
  s.frames_per_cobo[255] = 65535; // uint16 + キーは "255"
  s.running = true;

  rsmon::Encoder enc;
  std::vector<uint8_t> out;
  enc.status(s, 18'000'000'000'000'000'000ULL, out);

  tpcwire::Envelope env = tpcwire::parse_envelope(out.data(), out.size());
  tpcwire::BatchHeader h = tpcwire::parse_batch_header(env.payload, env.payload_size);
  CHECK_EQ(h.run_number, 4'000'000'000u);
  CHECK(h.created_ns == 18'000'000'000'000'000'000ULL);

  tpcwire::Reader r(h.payload, h.payload_size);
  const uint32_t n = r.read_map_len();
  uint64_t events_built = 0, bytes_written = 0, cobo255 = 0;
  for (uint32_t i = 0; i < n; ++i) {
    const std::string key = r.read_str();
    if (key == "events_built") {
      events_built = r.read_uint();
    } else if (key == "bytes_written") {
      bytes_written = r.read_uint();
    } else if (key == "frames_per_cobo") {
      const uint32_t m = r.read_map_len();
      CHECK_EQ(m, 1);
      CHECK(r.read_str() == "255");
      cobo255 = r.read_uint();
    } else {
      r.skip_value();
    }
  }
  CHECK(events_built == (1ULL << 40));
  CHECK_EQ(bytes_written, 70000);
  CHECK_EQ(cobo255, 65535);
}

}  // namespace

int main() {
  test_status_bytes();
  test_status_idle_state_string();
  test_hist_snapshot_bytes();
  test_hist_snapshot_bin_order_is_row_major_by_strip();
  test_built_event_bytes();
  test_built_event_complete_flag_is_inverted();
  test_fragments_round_trip_through_the_production_parser();
  test_sequence_is_one_series_across_kinds_and_never_resets();
  test_output_buffer_is_reused_without_leftovers();
  test_large_scalars_use_wider_msgpack_integers();
  return tpccheck::report("test_monitor_pub");
}
