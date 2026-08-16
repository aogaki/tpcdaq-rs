// test_vcobo_core.cpp — vcobo_core.hpp の単体テスト(**Ice も socket も要らない**)。
//
//   make -C tools/vcobo test
//
// TODO/040 の受け入れ ① 「レジスタ/フィールド格納のユニット(ビット抽出・alarm=!powerON)」。
// 値はすべて手計算の出典コメント付き。データは非対称(AsAd0 と AsAd2 で違う値)にして、
// 添字の取り違えがテストを素通りしないようにしてある。
#include "vcobo_core.hpp"

#include <string>
#include <utility>
#include <vector>

#include "check.hpp"

using vcobo::Config;
using vcobo::FieldDef;
using vcobo::FrameSpan;
using vcobo::Node;

namespace {

// ---------------------------------------------------------------------------
// テスト用のノード記述 —— reference/_spike/run039/configs の
// hardwareDescription_fullCoBoStandAlone.xcfg から必要部分だけを写したもの:
//   ctrl/asadConnection: PLG{0..3}(offset i, width 1) / PLG(offset 0, width 4)
//   ctrl/asadEnable:     powerON{0..3}(offset i, width 1)
//   ctrl/asadStatus:     alarm(offset 0, width 4)
//   asad/monitorID, asad/VDD, aget/reg5(ビットフィールド無し)
// ---------------------------------------------------------------------------
Node make_mini_node() {
  Node n;
  vcobo::DeviceInfo ctrl;
  ctrl.name = "ctrl";
  ctrl.base_address = 0x80000000;
  ctrl.register_access = "MemBus";
  ctrl.register_width = 4;
  n.create_device(ctrl);

  std::map<std::string, FieldDef> plg;
  plg["PLG"] = FieldDef{0, 4};
  for (int i = 0; i < 4; ++i) plg["PLG" + std::to_string(i)] = FieldDef{i, 1};
  n.define_register("ctrl", "asadConnection", plg);

  std::map<std::string, FieldDef> enable;
  enable["powerON"] = FieldDef{0, 4};
  for (int i = 0; i < 4; ++i) enable["powerON" + std::to_string(i)] = FieldDef{i, 1};
  n.define_register("ctrl", "asadEnable", enable);

  std::map<std::string, FieldDef> status;
  status["alarm"] = FieldDef{0, 4};
  n.define_register("ctrl", "asadStatus", status);

  // 非対称な確認用: offset 8 / width 12 のフィールドを持つレジスタ。
  std::map<std::string, FieldDef> mutant;
  mutant["mode"] = FieldDef{8, 12};
  n.define_register("ctrl", "mutantConfig", mutant);

  vcobo::DeviceInfo asad;
  asad.name = "asad";
  asad.base_address = 0x0;
  asad.register_access = "AsAdBus";
  asad.register_width = 1;
  n.create_device(asad);
  n.define_register("asad", "monitorID", {});
  n.define_register("asad", "VDD", {});

  vcobo::DeviceInfo aget;
  aget.name = "aget";
  aget.register_access = "AGetBus";
  aget.register_width = 4;
  n.create_device(aget);
  n.define_register("aget", "reg5", {});

  return n;
}

// ---------------------------------------------------------------------------
// 1. ビット抽出・挿入
// ---------------------------------------------------------------------------
void test_bit_helpers() {
  // 0xDEADBEEF の bits[15:8] = 0xBE(手計算: 0xDEADBEEF >> 8 = 0xDEADBE、& 0xFF = 0xBE)
  CHECK_EQ(vcobo::extract_bits(0xDEADBEEFull, 8, 8), 0xBEull);
  // bits[19:8](width 12)= 0xDBE(0xDEADBEEF >> 8 = 0xDEADBE、& 0xFFF = 0xDBE)
  CHECK_EQ(vcobo::extract_bits(0xDEADBEEFull, 8, 12), 0xDBEull);
  // width 0 / 負の offset は 0
  CHECK_EQ(vcobo::extract_bits(0xFFFFull, 0, 0), 0ull);
  CHECK_EQ(vcobo::extract_bits(0xFFFFull, -1, 4), 0ull);

  // 0x1234 の bits[11:4] を 0xAB に差し替え → 0x1AB4
  //   手計算: 0x1234 & ~0x0FF0 = 0x1004、0xAB << 4 = 0xAB0、和 = 0x1AB4
  CHECK_EQ(vcobo::insert_bits(0x1234ull, 4, 8, 0xABull), 0x1AB4ull);
  // はみ出す値は幅で切られる: width 4 に 0x1F を書くと 0xF
  CHECK_EQ(vcobo::insert_bits(0x0000ull, 0, 4, 0x1Full), 0x000Full);
  // 64bit 全幅
  CHECK_EQ(vcobo::insert_bits(0ull, 0, 64, 0xFFFFFFFFFFFFFFFFull), 0xFFFFFFFFFFFFFFFFull);
}

// ---------------------------------------------------------------------------
// 2. レジスタ / フィールドの格納
// ---------------------------------------------------------------------------
void test_register_storage() {
  Node n = make_mini_node();

  n.write_register("ctrl", "mutantConfig", 0x00012300ull);
  CHECK_EQ(n.read_register("ctrl", "mutantConfig"), 0x00012300ull);
  // mode = bits[19:8] → 0x00012300 >> 8 = 0x123、& 0xFFF = 0x123
  CHECK_EQ(n.read_field("ctrl", "mutantConfig", "mode"), 0x123ull);

  // フィールド書き込みは他のビットを壊さない。
  n.write_register("ctrl", "mutantConfig", 0xFF0000FFull);
  n.write_field("ctrl", "mutantConfig", "mode", 0x456ull);
  //   0xFF0000FF & ~0x000FFF00 = 0xFF0000FF、0x456 << 8 = 0x45600 → 0xFF0456FF
  CHECK_EQ(n.read_register("ctrl", "mutantConfig"), 0xFF0456FFull);
  CHECK_EQ(n.read_field("ctrl", "mutantConfig", "mode"), 0x456ull);
}

void test_unknown_names_warn_once_and_do_not_lose_data() {
  Node n = make_mini_node();
  std::vector<std::string> warns;
  n.set_warn([&warns](const std::string& m) { warns.push_back(m); });

  // 未知レジスタの read は 0、警告は名前ごとに 1 回だけ。
  CHECK_EQ(n.read_register("ctrl", "noSuchReg"), 0ull);
  CHECK_EQ(n.read_register("ctrl", "noSuchReg"), 0ull);
  CHECK_EQ(warns.size(), 1u);

  // 未知レジスタの write は受理して格納する(黙って捨てない)。
  n.write_register("ctrl", "brandNewReg", 0x5A5Aull);
  CHECK_EQ(n.read_register("ctrl", "brandNewReg"), 0x5A5Aull);
  CHECK_EQ(warns.size(), 2u);  // brandNewReg の警告が 1 回増えただけ

  // 未知フィールドの write は「レジスタ全体」として受理し、read で戻る。
  n.write_field("asad", "monitorID", "unknownField", 0x77ull);
  CHECK_EQ(n.read_field("asad", "monitorID", "unknownField"), 0x77ull);
  CHECK_EQ(warns.size(), 3u);
}

// ---------------------------------------------------------------------------
// 3. 魔法値(039 実測の必須 4 種)
// ---------------------------------------------------------------------------
void test_magic_values() {
  Node n = make_mini_node();
  n.apply_magic_values();

  CHECK_EQ(n.read_register("asad", "monitorID"), 0x41ull);  // 'A'
  CHECK_EQ(n.read_register("asad", "VDD"), 0xCDull);
  CHECK_EQ(n.read_register("aget", "reg5"), 0x201ull);
  for (int i = 0; i < 4; ++i) {
    CHECK_EQ(n.read_field("ctrl", "asadConnection", "PLG" + std::to_string(i)), 1ull);
  }
  // PLG{0..3} を立てた結果、4bit の PLG は 0xF になる(同じレジスタの別名フィールド)。
  CHECK_EQ(n.read_field("ctrl", "asadConnection", "PLG"), 0xFull);

  // 記述に無いレジスタへの焼き込みは黙って諦めず警告する。
  Node bare;
  std::vector<std::string> warns;
  bare.set_warn([&warns](const std::string& m) { warns.push_back(m); });
  bare.apply_magic_values();
  CHECK(warns.size() >= 4u);  // monitorID / VDD / reg5 / asadConnection
}

// ---------------------------------------------------------------------------
// 4. alarm 意味論(alarm ビット i = !powerON{i})
// ---------------------------------------------------------------------------
void test_alarm_is_the_inverse_of_power_on() {
  Node n = make_mini_node();
  n.apply_magic_values();

  // 起動直後は全 AsAd が power off → alarm = 0b1111 = 0xF。
  // ECC は power off 直後に「電源が来ている = alarm ビットが 1」を要求する
  // (CoBoNode.cpp:1059 isAsAdPowerSupplied)。
  CHECK_EQ(n.read_field("ctrl", "asadStatus", "alarm"), 0xFull);

  // AsAd2 だけ power on → alarm = 0b1011 = 0xB(非対称: 0 番ではなく 2 番)。
  n.write_field("ctrl", "asadEnable", "powerON2", 1);
  CHECK_EQ(n.read_field("ctrl", "asadStatus", "alarm"), 0xBull);

  // AsAd0 も power on → alarm = 0b1010 = 0xA。
  n.write_field("ctrl", "asadEnable", "powerON0", 1);
  CHECK_EQ(n.read_field("ctrl", "asadStatus", "alarm"), 0xAull);

  // AsAd2 を power off に戻す → alarm = 0b1110 = 0xE。
  n.write_field("ctrl", "asadEnable", "powerON2", 0);
  CHECK_EQ(n.read_field("ctrl", "asadStatus", "alarm"), 0xEull);

  // レジスタ丸ごとの書き込みでも追従する(powerON = 0b0110 → alarm = 0b1001 = 0x9)。
  n.write_register("ctrl", "asadEnable", 0x6ull);
  CHECK_EQ(n.read_field("ctrl", "asadStatus", "alarm"), 0x9ull);

  // 全 AsAd power on → alarm = 0。ECC の asadAlertsInit はここで 0 を要求する。
  n.write_register("ctrl", "asadEnable", 0xFull);
  CHECK_EQ(n.read_field("ctrl", "asadStatus", "alarm"), 0x0ull);
}

void test_destroy_forgets_everything() {
  Node n = make_mini_node();
  n.apply_magic_values();
  n.set_name("CoBo[0]");
  CHECK(n.has_device("ctrl"));

  n.reset();
  CHECK(!n.has_device("ctrl"));
  CHECK_EQ(n.device_names().size(), 0u);
  CHECK_STR(n.name(), "CoBo[0]");  // name は destroy で消えない(ECC が setName で上書きする)

  // 再 create で魔法値がもう一度乗る(039: シードは describe 毎に消える)。
  n = make_mini_node();
  n.apply_magic_values();
  CHECK_EQ(n.read_register("asad", "monitorID"), 0x41ull);
}

void test_device_map_is_sorted_and_complete() {
  Node n = make_mini_node();
  const std::vector<std::string> names = n.device_names();
  CHECK_EQ(names.size(), 3u);
  CHECK_STR(names[0], "aget");
  CHECK_STR(names[1], "asad");
  CHECK_STR(names[2], "ctrl");

  const vcobo::DeviceInfo* asad = n.device_info("asad");
  CHECK(asad != nullptr);
  if (asad != nullptr) {
    CHECK_STR(asad->register_access, "AsAdBus");
    CHECK_EQ(asad->register_width, 1);
  }
  CHECK(n.device_info("nope") == nullptr);
}

// ---------------------------------------------------------------------------
// 5. GRAW フレーム境界
// ---------------------------------------------------------------------------

/// `total` バイトの MFM フレームを作る(src/framer.rs のテストヘルパと同じ作り)。
std::vector<uint8_t> make_frame(size_t total, bool little, uint8_t fill) {
  std::vector<uint8_t> f(total, fill);
  f[0] = little ? 0x80 : 0x00;  // bit7=endian、下位ニブル 0 → blkSize = 1
  const uint32_t blk = uint32_t(total);
  if (little) {
    f[1] = uint8_t(blk & 0xFF);
    f[2] = uint8_t((blk >> 8) & 0xFF);
    f[3] = uint8_t((blk >> 16) & 0xFF);
  } else {
    f[1] = uint8_t((blk >> 16) & 0xFF);
    f[2] = uint8_t((blk >> 8) & 0xFF);
    f[3] = uint8_t(blk & 0xFF);
  }
  return f;
}

void test_graw_frame_size() {
  // little-endian, blkSize=1, 40 バイト
  const std::vector<uint8_t> le = make_frame(40, true, 0xAB);
  CHECK_EQ(vcobo::graw_frame_size(le.data()), 40u);
  // big-endian, blkSize=1, 40 バイト
  const std::vector<uint8_t> be = make_frame(40, false, 0xAB);
  CHECK_EQ(vcobo::graw_frame_size(be.data()), 40u);

  // 実データの符号化: metaType 0x08 = big-endian・blkSize 256、frameSize_blk = 3
  //   → 3 × 256 = 768 バイト
  const uint8_t compact[8] = {0x08, 0x00, 0x00, 0x03, 0, 0, 0, 0};
  CHECK_EQ(vcobo::graw_frame_size(compact), 768u);

  // frameSize 0 は壊れ扱い。
  const uint8_t zero[8] = {0x00, 0x00, 0x00, 0x00, 0, 0, 0, 0};
  CHECK_EQ(vcobo::graw_frame_size(zero), 0u);
  // ヘッダ長未満(4 バイト)も壊れ扱い。
  const uint8_t tiny[8] = {0x00, 0x00, 0x00, 0x04, 0, 0, 0, 0};
  CHECK_EQ(vcobo::graw_frame_size(tiny), 0u);
}

void test_index_graw() {
  // 非対称な 3 フレーム: 24 / 512 / 40 バイト
  std::vector<uint8_t> bytes;
  for (const auto& f : {make_frame(24, false, 0x11), make_frame(512, false, 0x22),
                        make_frame(40, false, 0x33)}) {
    bytes.insert(bytes.end(), f.begin(), f.end());
  }

  std::vector<FrameSpan> spans;
  size_t trailing = 0;
  CHECK(vcobo::index_graw(bytes, spans, trailing));
  CHECK_EQ(trailing, 0u);
  CHECK_EQ(spans.size(), 3u);
  CHECK_EQ(spans[0].offset, 0u);
  CHECK_EQ(spans[0].length, 24u);
  CHECK_EQ(spans[1].offset, 24u);
  CHECK_EQ(spans[1].length, 512u);
  CHECK_EQ(spans[2].offset, 536u);  // 24 + 512
  CHECK_EQ(spans[2].length, 40u);

  // 端数が残ったら false + trailing に計上(黙って捨てない)。
  bytes.resize(bytes.size() + 7, 0x99);
  CHECK(!vcobo::index_graw(bytes, spans, trailing));
  CHECK_EQ(trailing, 7u);
  CHECK_EQ(spans.size(), 3u);
}

// ---------------------------------------------------------------------------
// 5b. eventIdx マージ送出(TODO/056)
//
// 正本 = src/bin/graw_replay.rs の `mod merge`(TODO/021 の複数ファイルマージ):
//   ①制御フレーム(eventIdx を持たないフレーム)が先頭に居るソースを最優先で流す
//     (複数あればファイル指定順の最小)
//   ②データフレームは (eventIdx, ファイル指定順) の最小を選ぶ
//   ③ソース内の順序は常に保存する(全体ソートではない k-way マージ)
// ---------------------------------------------------------------------------

/// `bytes` の `offset` から `n` バイトを big/little endian で書く(ヘッダ組み立て用)。
void write_uint(std::vector<uint8_t>& bytes, size_t offset, uint64_t value, size_t n, bool little) {
  for (size_t i = 0; i < n; ++i) {
    const size_t at = little ? offset + i : offset + (n - 1 - i);
    bytes[at] = uint8_t((value >> (8 * i)) & 0xFF);
  }
}

/// データフレーム(frameType 1/2)を 1 本作る。frameType は bytes[5..7]、
/// eventIdx は bytes[22..26](共通 MFM ヘッダ — src/decode.rs::peek_event_idx)。
std::vector<uint8_t> make_data_frame(size_t total, bool little, uint16_t frame_type,
                                     uint32_t event_idx, uint8_t fill) {
  std::vector<uint8_t> f = make_frame(total, little, fill);
  write_uint(f, 5, frame_type, 2, little);
  write_uint(f, 22, event_idx, 4, little);
  return f;
}

/// 複数「ファイル」を 1 本のバイト列に連結しつつ、ファイル毎のフレーム座標を覚えておく
/// (load_graw_set が実際にやることのミニチュア)。
struct MergeInput {
  std::vector<uint8_t> bytes;
  std::vector<std::vector<FrameSpan>> per_file;

  void add_file(const std::vector<std::vector<uint8_t>>& frames) {
    std::vector<FrameSpan> spans;
    for (const std::vector<uint8_t>& f : frames) {
      FrameSpan s;
      s.offset = bytes.size();
      s.length = f.size();
      bytes.insert(bytes.end(), f.begin(), f.end());
      spans.push_back(s);
    }
    per_file.push_back(spans);
  }

  /// 期待順序を (ファイル番号, ファイル内の索引) の列で照合する。
  void check_order(const std::vector<FrameSpan>& got,
                   const std::vector<std::pair<size_t, size_t>>& want) {
    CHECK_EQ(got.size(), want.size());
    if (got.size() != want.size()) return;
    for (size_t i = 0; i < want.size(); ++i) {
      const FrameSpan& expected = per_file[want[i].first][want[i].second];
      CHECK_EQ(got[i].offset, expected.offset);
      CHECK_EQ(got[i].length, expected.length);
    }
  }
};

void test_peek_event_idx() {
  for (bool little : {false, true}) {
    // frameType 2、eventIdx 0x01020304 → ヘッダ 22..26 からそのまま読める。
    const std::vector<uint8_t> f = make_data_frame(256, little, 2, 0x01020304u, 0x5A);
    uint32_t ev = 0;
    CHECK(vcobo::peek_event_idx(f.data(), f.size(), ev));
    CHECK_EQ(ev, 0x01020304u);
  }

  // frameType 1 も対象(2018 形式)。
  const std::vector<uint8_t> t1 = make_data_frame(64, false, 1, 3851u, 0x11);
  uint32_t ev = 0;
  CHECK(vcobo::peek_event_idx(t1.data(), t1.size(), ev));
  CHECK_EQ(ev, 3851u);

  // frameType ∉ {1,2} は制御フレーム扱い(eventIdx を持たない)。
  const std::vector<uint8_t> ctrl = make_data_frame(64, false, 7, 9u, 0x22);
  ev = 0xFFFFFFFFu;
  CHECK(!vcobo::peek_event_idx(ctrl.data(), ctrl.size(), ev));

  // 28 バイト未満も制御フレーム扱い(graw_replay と同じゲート)。
  const std::vector<uint8_t> tiny = make_frame(24, false, 0x33);
  CHECK(!vcobo::peek_event_idx(tiny.data(), tiny.size(), ev));
}

void test_merge_single_file_keeps_the_original_order() {
  // mini 回帰: 1 ファイルなら送出順は索引順のまま(バイト一致テストの土台)。
  MergeInput in;
  in.add_file({make_data_frame(256, false, 2, 0, 0x01), make_data_frame(512, false, 2, 1, 0x02),
               make_data_frame(256, false, 2, 2, 0x03)});
  const std::vector<FrameSpan> got = vcobo::merge_by_event_idx(in.bytes, in.per_file);
  in.check_order(got, {{0, 0}, {0, 1}, {0, 2}});
}

void test_merge_interleaves_files_event_by_event() {
  // 非対称: AsAd0 は 4 イベント、AsAd1 は先頭 2 イベントだけ(短いファイル)。
  //   期待: (0,ev0) (1,ev0) (0,ev1) (1,ev1) (0,ev2) (0,ev3)
  MergeInput in;
  in.add_file({make_data_frame(256, false, 2, 0, 0xA0), make_data_frame(256, false, 2, 1, 0xA1),
               make_data_frame(256, false, 2, 2, 0xA2), make_data_frame(256, false, 2, 3, 0xA3)});
  in.add_file({make_data_frame(512, false, 2, 0, 0xB0), make_data_frame(512, false, 2, 1, 0xB1)});

  const std::vector<FrameSpan> got = vcobo::merge_by_event_idx(in.bytes, in.per_file);
  in.check_order(got, {{0, 0}, {1, 0}, {0, 1}, {1, 1}, {0, 2}, {0, 3}});
}

void test_merge_breaks_ties_by_file_argument_order() {
  // 同じ eventIdx が 4 ファイルに 1 本ずつ。引数順(= AsAd 昇順で渡す)で並ぶ。
  MergeInput in;
  for (int a = 0; a < 4; ++a) {
    in.add_file({make_data_frame(256, false, 2, 7, uint8_t(0xC0 + a))});
  }
  const std::vector<FrameSpan> got = vcobo::merge_by_event_idx(in.bytes, in.per_file);
  in.check_order(got, {{0, 0}, {1, 0}, {2, 0}, {3, 0}});
}

void test_merge_gives_control_frames_priority_like_graw_replay() {
  // graw_replay: 先頭に制御フレームが居るソースが最優先(データより先)。
  //   file0 = [data ev0, data ev1]、file1 = [ctrl, data ev0]
  //   → ctrl(file1) → data ev0(file0) → data ev0(file1) → data ev1(file0)
  MergeInput in;
  in.add_file({make_data_frame(256, false, 2, 0, 0xD0), make_data_frame(256, false, 2, 1, 0xD1)});
  in.add_file({make_data_frame(256, false, 7, 0, 0xE0), make_data_frame(256, false, 2, 0, 0xE1)});

  const std::vector<FrameSpan> got = vcobo::merge_by_event_idx(in.bytes, in.per_file);
  in.check_order(got, {{1, 0}, {0, 0}, {1, 1}, {0, 1}});
}

void test_merge_preserves_source_order_when_event_idx_goes_backwards() {
  // ④eventIdx 逆転入力でも「ソース内の順序」は必ず保存する(全体ソートはしない)。
  //   file0 = [ev5, ev1]、file1 = [ev2, ev3]
  //   先頭比較: (5,2) → 2、(5,3) → 3、file1 枯渇 → 5、1。= graw_replay と同じ結果。
  MergeInput in;
  in.add_file({make_data_frame(256, false, 2, 5, 0xF0), make_data_frame(256, false, 2, 1, 0xF1)});
  in.add_file({make_data_frame(256, false, 2, 2, 0x70), make_data_frame(256, false, 2, 3, 0x71)});

  const std::vector<FrameSpan> got = vcobo::merge_by_event_idx(in.bytes, in.per_file);
  in.check_order(got, {{1, 0}, {1, 1}, {0, 0}, {0, 1}});
}

void test_merge_of_nothing_is_nothing() {
  MergeInput in;
  const std::vector<FrameSpan> got = vcobo::merge_by_event_idx(in.bytes, in.per_file);
  CHECK_EQ(got.size(), 0u);
}

// ---------------------------------------------------------------------------
// 6. 設定
// ---------------------------------------------------------------------------
void test_config_parse() {
  const std::string text =
      "# vcobo demo\n"
      "cobo_id = 3\n"
      "asad_count = 4\n"
      "listen_host = \"127.0.0.1\"\n"
      "ctrl_port = 47001\n"
      "daq_port = 47004\n"
      "rate_hz = 12.5\n"
      "loop = true\n"
      "flush_bytes = 4096\n"
      "heartbeat_flush_ms = 3000\n"
      "graw_files = [\"a.graw\", \"b.graw\"]  # 2 本\n";
  Config cfg;
  std::string err;
  CHECK(vcobo::parse_config(text, cfg, err));
  CHECK_STR(err, "");
  CHECK_EQ(cfg.cobo_id, 3);
  CHECK_EQ(cfg.asad_count, 4);
  CHECK_STR(cfg.listen_host, "127.0.0.1");
  CHECK_EQ(cfg.ctrl_port, 47001);
  CHECK_EQ(cfg.daq_port, 47004);
  CHECK(cfg.rate_hz > 12.4 && cfg.rate_hz < 12.6);
  CHECK(cfg.loop);
  CHECK_EQ(cfg.flush_bytes, 4096u);
  CHECK_EQ(cfg.heartbeat_flush_ms, 3000);
  CHECK_EQ(cfg.graw_files.size(), 2u);
  CHECK_STR(cfg.graw_files[0], "a.graw");
  CHECK_STR(cfg.graw_files[1], "b.graw");

  // 既定値(空の設定)
  Config def;
  CHECK(vcobo::parse_config("", def, err));
  CHECK_EQ(def.ctrl_port, 46001);
  CHECK_EQ(def.daq_port, 46004);
  CHECK_EQ(def.asad_count, 1);
  CHECK(!def.loop);

  // 未知キーはエラー(黙って既定値に落ちない)
  Config bad;
  CHECK(!vcobo::parse_config("nb_channels = 256\n", bad, err));
  CHECK(!err.empty());
  // 値の型違いもエラー
  CHECK(!vcobo::parse_config("ctrl_port = \"46001\"\n", bad, err));
  // 範囲違反もエラー
  CHECK(!vcobo::parse_config("asad_count = 5\n", bad, err));
  CHECK(!vcobo::parse_config("rate_hz = -1\n", bad, err));
}

void test_asad_mask_check() {
  // mini(1 AsAd)は mask=0x1 のみ整合。0x3 は AsAd1 が居ないので不整合。
  CHECK(vcobo::asad_mask_matches_config(0x1, 1));
  CHECK(!vcobo::asad_mask_matches_config(0x3, 1));
  // ELITPC(4 AsAd)は 0xF まで整合。
  CHECK(vcobo::asad_mask_matches_config(0xF, 4));
  CHECK(vcobo::asad_mask_matches_config(0x5, 4));
  CHECK(!vcobo::asad_mask_matches_config(0x1F, 4));
}

}  // namespace

int main() {
  test_bit_helpers();
  test_register_storage();
  test_unknown_names_warn_once_and_do_not_lose_data();
  test_magic_values();
  test_alarm_is_the_inverse_of_power_on();
  test_destroy_forgets_everything();
  test_device_map_is_sorted_and_complete();
  test_graw_frame_size();
  test_index_graw();
  test_peek_event_idx();
  test_merge_single_file_keeps_the_original_order();
  test_merge_interleaves_files_event_by_event();
  test_merge_breaks_ties_by_file_argument_order();
  test_merge_gives_control_frames_priority_like_graw_replay();
  test_merge_preserves_source_order_when_event_idx_goes_backwards();
  test_merge_of_nothing_is_nothing();
  test_config_parse();
  test_asad_mask_check();
  return tpccheck::report("test_vcobo_core");
}
