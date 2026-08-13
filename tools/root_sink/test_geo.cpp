// test_geo.cpp — geo.hpp の単体テスト(ROOT 不要・ZMQ 不要)。TODO/018。
//
//   g++ -std=c++17 -O2 -Wall -Wextra test_geo.cpp -o test_geo && ./test_geo
//
// 代表ケースは `src/geometry.rs` のテスト(インライン単体テスト + tests/geometry_*.rs)
// から移植した。フィクスチャは Rust 側と**同一の 3 ファイル**
// (tests/fixtures/geometry_{mini_reduced,2cobo_fake,legacy}.dat)を共有する ——
// 別のフィクスチャを新規に作ると「Rust と C++ が同じものを読んでいる」保証が薄れる。
// 重複/malformed/unmapped-hit-count のケースだけは(Rust 側もインライン文字列なので)
// このファイルの中で手組みの `.dat` テキストを使う。各値の出典は各テストのコメント。
//
// **ダンプモード**: `./test_geo <file.dat>` は CHECK 群を実行せず、指定ファイルを
// パースして `dump_tsv` を stdout に出すだけ(run_geo_conformance.sh が Rust
// `geometry_dump` の出力とバイト一致を diff するための相方)。

#include <cstdio>
#include <string>
#include <vector>

#include "check.hpp"
#include "geo.hpp"

using namespace tpcgeo;

// フィクスチャは cwd = tools/root_sink 前提(Makefile の `test:` はこのディレクトリで
// 実行する — repo 直下からは `cd tools/root_sink && make test`)。
static const char* kFixtureMiniReduced = "../../tests/fixtures/geometry_mini_reduced.dat";
static const char* kFixture2CoboFake = "../../tests/fixtures/geometry_2cobo_fake.dat";
static const char* kFixtureLegacy = "../../tests/fixtures/geometry_legacy.dat";

static bool double_eq(double a, double b) { return a == b; }

template <class F>
static bool throws(F f) {
  try {
    f();
  } catch (const std::exception&) {
    return true;
  }
  return false;
}

// ---------------------------------------------------------------------------
// 1. FPN リオーダ表(TPCReco `Aget_normal2raw` ループ版との一致 — src/geometry.rs
//    `aget_normal2raw_loop` の移植)。
// ---------------------------------------------------------------------------

static uint32_t aget_normal2raw_loop(uint32_t channel_idx) {
  uint32_t raw = channel_idx;
  for (uint32_t fpn : kFpnRawChannels) {
    if (fpn < raw) raw += 1;
    if (fpn == raw) raw += 1;
  }
  return raw;
}

static void test_reorder_loop_matches_constant_table_for_all_64_inputs() {
  for (uint32_t signal_ch = 0; signal_ch < kSignalChPerAget; ++signal_ch) {
    CHECK_EQ(aget_normal2raw_loop(signal_ch), kReorderFromGeometryToGraw[signal_ch]);
  }
}

static void test_plane_parse_accepts_only_uppercase_uvw() {
  Plane p;
  CHECK(plane_parse("U", p) && p == Plane::U);
  CHECK(plane_parse("V", p) && p == Plane::V);
  CHECK(plane_parse("W", p) && p == Plane::W);
  CHECK(!plane_parse("u", p));
  CHECK(!plane_parse("FPN", p));
  CHECK(!plane_parse("", p));
}

// ---------------------------------------------------------------------------
// 2. NEW(10 欄)フォーマット — tests/fixtures/geometry_mini_reduced.dat
//    (tests/geometry_new_format.rs の移植。オラクル値はそちらと同一)。
// ---------------------------------------------------------------------------

static void test_header_scalars_are_parsed_verbatim() {
  Geometry g = load(kFixtureMiniReduced);
  // フィクスチャファイルの値をそのまま手で書き写しただけ(mm 計算はしない — SPEC §4.4)。
  CHECK(g.header.angles_deg.has_value());
  CHECK(double_eq((*g.header.angles_deg)[0], 11.0));
  CHECK(double_eq((*g.header.angles_deg)[1], 22.0));
  CHECK(double_eq((*g.header.angles_deg)[2], 33.0));
  CHECK(g.header.diamond_size_mm.has_value() && double_eq(*g.header.diamond_size_mm, 2.5));
  CHECK(g.header.reference_point_mm.has_value());
  CHECK(double_eq(g.header.reference_point_mm->first, -5.0));
  CHECK(double_eq(g.header.reference_point_mm->second, 6.0));
  CHECK(g.header.drift_velocity_cm_per_us.has_value() &&
        double_eq(*g.header.drift_velocity_cm_per_us, 0.6));
  CHECK(g.header.sampling_rate_mhz.has_value() && double_eq(*g.header.sampling_rate_mhz, 12.5));
  CHECK(g.header.trigger_delay_us.has_value() && double_eq(*g.header.trigger_delay_us, 4.5));
  CHECK(g.header.drift_cage_acceptance_mm.has_value());
  CHECK(double_eq(g.header.drift_cage_acceptance_mm->first, -40.0));
  CHECK(double_eq(g.header.drift_cage_acceptance_mm->second, 40.0));
}

static void test_strip_channels_resolve_to_expected_plane_section_strip() {
  Geometry g = load(kFixtureMiniReduced);
  // signal_ch 0..4(< 11)は REORDER 表で恒等写像(raw_ch == signal_ch)なので、
  // フィクスチャの AGET_CH をそのまま raw_ch として使える。
  CHECK(g.lookup(0, 0, 0, 0) == make_strip(Plane::U, 0, 1));
  CHECK(g.lookup(0, 0, 0, 3) == make_strip(Plane::U, 0, 4));
  CHECK(g.lookup(0, 0, 1, 0) == make_strip(Plane::V, 0, 1));
  CHECK(g.lookup(0, 0, 1, 3) == make_strip(Plane::W, 0, 1));
  CHECK(g.lookup(0, 0, 1, 6) == make_strip(Plane::W, 0, 4));
}

static void test_aux_channels_resolve_with_their_name() {
  Geometry g = load(kFixtureMiniReduced);
  CHECK(g.lookup(0, 0, 0, 4) == make_aux("TestAux0"));
  CHECK(g.lookup(0, 0, 1, 7) == make_aux("TestAux1"));
}

static void test_fpn_channels_resolve_for_every_active_aget_only() {
  Geometry g = load(kFixtureMiniReduced);
  // aget=0 と aget=1 はどちらもストリップ/AUX を持つ「active」な AGET なので
  // raw {11,22,45,56} が FPN(index 1..4)になる。
  for (uint32_t aget = 0; aget < 2; ++aget) {
    CHECK(g.lookup(0, 0, aget, 11) == make_fpn(1));
    CHECK(g.lookup(0, 0, aget, 22) == make_fpn(2));
    CHECK(g.lookup(0, 0, aget, 45) == make_fpn(3));
    CHECK(g.lookup(0, 0, aget, 56) == make_fpn(4));
  }
  // aget=2/3 はフィクスチャに一切登場しないので、FPN スロットも含めて Unmapped。
  CHECK(g.lookup(0, 0, 2, 11) == make_unmapped());
  CHECK(g.lookup(0, 0, 3, 56) == make_unmapped());
}

static void test_unmapped_channels_are_unmapped_within_active_aget_too() {
  Geometry g = load(kFixtureMiniReduced);
  // aget=0 は active だが signal_ch=63(raw63)はどのストリップにも割り当てられていない。
  CHECK(g.lookup(0, 0, 0, 63) == make_unmapped());
}

static void test_max_strip_matches_fixture_hand_count() {
  Geometry g = load(kFixtureMiniReduced);
  // 手計算: U は strip 1..4(4 本)、V は 1..3(3 本)、W は 1..4(4 本)。
  CHECK_EQ(g.max_strip[0], 4);
  CHECK_EQ(g.max_strip[1], 3);
  CHECK_EQ(g.max_strip[2], 4);
}

static void test_cobo_count_is_one_for_single_cobo_fixture() {
  Geometry g = load(kFixtureMiniReduced);
  CHECK_EQ(g.cobo_count(), 1);
}

static void test_clean_fixture_has_no_duplicate_or_malformed_warnings() {
  Geometry g = load(kFixtureMiniReduced);
  CHECK(g.duplicate_warnings().empty());
  CHECK(g.malformed_lines().empty());
}

static void test_load_missing_file_throws() {
  CHECK(throws([] { load("../../tests/fixtures/does-not-exist.dat"); }));
}

// ---------------------------------------------------------------------------
// 3. 複数 CoBo(架空 2-CoBo フィクスチャ) — tests/fixtures/geometry_2cobo_fake.dat
//    (tests/geometry_multi_cobo.rs の移植)。rust_reference の cobo 欠落キーの欠陥
//    (SPEC §4.2)を再発させないことを確認する: 同じ (asad,aget,raw_ch) でも cobo が
//    違えば別チャンネルとして解決されること。
// ---------------------------------------------------------------------------

static void test_cobo_count_reflects_both_cobos() {
  Geometry g = load(kFixture2CoboFake);
  CHECK_EQ(g.cobo_count(), 2);
}

static void test_identical_asad_aget_raw_ch_resolve_independently_per_cobo() {
  Geometry g = load(kFixture2CoboFake);
  // cobo=0, asad=0, aget=0, raw_ch=0 -> strip 1(U)。
  CHECK(g.lookup(0, 0, 0, 0) == make_strip(Plane::U, 0, 1));
  // 同じ (asad=0,aget=0,raw_ch=0) でも cobo=1 なら別チャンネル(strip 101)。
  // (asad,aget,chan) 3 タプルキーだとここが cobo=0 の strip1 と衝突してしまう
  // (SPEC §4.2 が明示的に直した欠陥)。
  CHECK(g.lookup(1, 0, 0, 0) == make_strip(Plane::U, 0, 101));
  CHECK(g.lookup(1, 0, 0, 1) == make_strip(Plane::U, 0, 102));
}

static void test_asad_count_can_differ_per_cobo() {
  Geometry g = load(kFixture2CoboFake);
  // cobo=1, asad=1 は本フィクスチャで使われている(V strip 201)。
  CHECK(g.lookup(1, 1, 0, 0) == make_strip(Plane::V, 0, 201));
  // cobo=0 は asad=0 しか使っていないので、asad=1 は範囲外 = Unmapped。
  CHECK(g.lookup(0, 1, 0, 0) == make_unmapped());
}

static void test_fpn_is_resolved_independently_for_each_cobo_asad_pair() {
  Geometry g = load(kFixture2CoboFake);
  CHECK(g.lookup(0, 0, 0, 11) == make_fpn(1));
  CHECK(g.lookup(1, 0, 0, 11) == make_fpn(1));
  CHECK(g.lookup(1, 1, 0, 11) == make_fpn(1));
}

static void test_2cobo_fixture_has_no_warnings() {
  Geometry g = load(kFixture2CoboFake);
  CHECK(g.duplicate_warnings().empty());
  CHECK(g.malformed_lines().empty());
}

// ---------------------------------------------------------------------------
// 4. LEGACY(7 欄)フォーマット — tests/fixtures/geometry_legacy.dat
//    (tests/geometry_legacy_format.rs の移植)。
// ---------------------------------------------------------------------------

static void test_legacy_lines_default_cobo_asad_section_to_zero() {
  Geometry g = load(kFixtureLegacy);
  // LEGACY: DIR STRIP AGET AGET_CH OFF_PAD OFF_STRIP LEN_PADS(section/cobo/asad は 0)。
  // U 1 0 0 ... -> aget=0, signal_ch=0 -> raw_ch=0(< 11 なので恒等写像)。
  CHECK(g.lookup(0, 0, 0, 0) == make_strip(Plane::U, 0, 1));
  // U 2 0 1 ... -> aget=0, signal_ch=1 -> raw_ch=1。
  CHECK(g.lookup(0, 0, 0, 1) == make_strip(Plane::U, 0, 2));
  // V 1 1 0 ... -> aget=1, signal_ch=0 -> raw_ch=0。
  CHECK(g.lookup(0, 0, 1, 0) == make_strip(Plane::V, 0, 1));
  // W 1 1 1 ... -> aget=1, signal_ch=1 -> raw_ch=1。
  CHECK(g.lookup(0, 0, 1, 1) == make_strip(Plane::W, 0, 1));
}

static void test_legacy_fpn_is_still_resolved_for_active_agets() {
  Geometry g = load(kFixtureLegacy);
  CHECK(g.lookup(0, 0, 0, 11) == make_fpn(1));
  CHECK(g.lookup(0, 0, 1, 56) == make_fpn(4));
}

static void test_legacy_max_strip_matches_hand_count() {
  Geometry g = load(kFixtureLegacy);
  // 手計算: U は strip 1,2(最大 2)。V は strip 1(最大 1)。W は strip 1(最大 1)。
  CHECK_EQ(g.max_strip[0], 2);
  CHECK_EQ(g.max_strip[1], 1);
  CHECK_EQ(g.max_strip[2], 1);
}

// ---------------------------------------------------------------------------
// 5. dump_tsv(SPEC §4.5)— tests/fixtures/geometry_mini_reduced.dat
//    (tests/geometry_tsv_dump.rs の移植。フォーマットは src/geometry.rs が正)。
// ---------------------------------------------------------------------------

static size_t count_lines(const std::string& s) {
  if (s.empty()) return 0;
  size_t n = 0;
  for (char c : s) {
    if (c == '\n') ++n;
  }
  return n;
}

static std::vector<std::string> split_lines(const std::string& s) {
  std::vector<std::string> out;
  size_t start = 0;
  for (size_t i = 0; i < s.size(); ++i) {
    if (s[i] == '\n') {
      out.push_back(s.substr(start, i - start));
      start = i + 1;
    }
  }
  return out;
}

static void test_dump_covers_every_slot_of_the_dense_array() {
  Geometry g = load(kFixtureMiniReduced);
  std::string dump = dump_tsv(g);
  // 手計算: cobo_count=1 * asad_count[0]=1 * AGET_CHIPS_PER_ASAD=4 * RAW_CH_PER_AGET=68
  // = 272。
  CHECK_EQ(count_lines(dump), 272);
}

static void test_dump_rows_are_tab_separated_with_eight_columns() {
  Geometry g = load(kFixtureMiniReduced);
  std::vector<std::string> lines = split_lines(dump_tsv(g));
  for (const auto& line : lines) {
    size_t tabs = 0;
    for (char c : line) {
      if (c == '\t') ++tabs;
    }
    CHECK_EQ(tabs, 7);  // 8 列 = 7 個のタブ区切り
  }
}

static void test_dump_first_row_is_the_first_strip_channel() {
  Geometry g = load(kFixtureMiniReduced);
  std::vector<std::string> lines = split_lines(dump_tsv(g));
  CHECK(!lines.empty());
  // cobo=0 asad=0 aget=0 raw_ch=0 -> Strip U section=0 strip=1(フィクスチャの先頭行)。
  CHECK(lines[0] == "0\t0\t0\t0\tStrip\tU\t0\t1");
}

static bool dump_contains(const std::vector<std::string>& lines, const std::string& target) {
  for (const auto& line : lines) {
    if (line == target) return true;
  }
  return false;
}

static void test_dump_fpn_row_has_empty_plane_section_strip_columns() {
  Geometry g = load(kFixtureMiniReduced);
  std::vector<std::string> lines = split_lines(dump_tsv(g));
  // aget=0, raw_ch=11 は FPN(index=1)だが、SPEC §4.5 のダンプ列は
  // (role, plane, section, strip) のみ — FPN の index 列は無い。
  CHECK(dump_contains(lines, "0\t0\t0\t11\tFpn\t\t\t"));
}

static void test_dump_aux_row_has_empty_plane_section_strip_columns() {
  Geometry g = load(kFixtureMiniReduced);
  std::vector<std::string> lines = split_lines(dump_tsv(g));
  // aget=0, raw_ch=4 は TestAux0(SPEC §4.5 のダンプ列に AUX の name は無い)。
  CHECK(dump_contains(lines, "0\t0\t0\t4\tAux\t\t\t"));
}

static void test_dump_unmapped_row_has_empty_plane_section_strip_columns() {
  Geometry g = load(kFixtureMiniReduced);
  std::vector<std::string> lines = split_lines(dump_tsv(g));
  // aget=2 はフィクスチャで一切使われていない(inactive)ので raw_ch=0 は Unmapped。
  CHECK(dump_contains(lines, "0\t0\t2\t0\tUnmapped\t\t\t"));
}

static void test_dump_tsv_does_not_increment_unmapped_hit_count() {
  Geometry g = load(kFixtureMiniReduced);
  std::string dump = dump_tsv(g);
  CHECK(!dump.empty());
  CHECK_EQ(g.unmapped_hit_count(), 0);
}

// ---------------------------------------------------------------------------
// 6. 重複・malformed・unmapped-hit-count(手組みの .dat テキスト —
//    src/geometry.rs の同名インラインテストの移植。手計算はコメント参照)。
// ---------------------------------------------------------------------------

static void test_duplicate_channel_key_warns_and_keeps_first() {
  // 1 行目: cobo0 asad0 aget0 signal_ch0 -> raw_ch=REORDER[0]=0。U section0 strip1。
  // 2 行目: 同じ (cobo0,asad0,aget0,signal_ch0) -> raw_ch=0(同じセルを狙う)。
  //         V section0 strip9(先勝ちで無視される側)。
  std::string text =
      "U\t0\t1\t0\t0\t0\t0\t0.0\t0.0\t10\n"
      "V\t0\t9\t0\t0\t0\t0\t9.0\t9.0\t10\n";
  Geometry g = parse(text);
  CHECK(g.lookup(0, 0, 0, 0) == make_strip(Plane::U, 0, 1));
  CHECK_EQ(g.duplicate_warnings().size(), 1);
  const DuplicateChannel& dup = g.duplicate_warnings()[0];
  CHECK_EQ(dup.cobo, 0);
  CHECK_EQ(dup.asad, 0);
  CHECK_EQ(dup.aget, 0);
  CHECK_EQ(dup.raw_ch, 0);
  CHECK_EQ(dup.line_number, 2);  // 1 行目 U、2 行目 V(ヘッダなし)。
}

static void test_malformed_lines_are_recorded_not_fatal() {
  // 1 行目: 6 トークン(10/7/5 いずれにも合わない)。
  // 2 行目: DIR="Z" は Plane として不正(NEW 10 欄の形はしているが弾かれる)。
  // 3 行目: AGET_CH=99 は 0..64 の範囲外(REORDER 表引きで弾かれる)。
  // 4 行目: 正しい行 -> cobo0 asad0 aget0 raw_ch0、U section0 strip2。
  std::string text =
      "foo bar baz qux quux corge\n"
      "Z\t0\t1\t0\t0\t0\t0\t0.0\t0.0\t10\n"
      "U\t0\t1\t0\t0\t0\t99\t0.0\t0.0\t10\n"
      "U\t0\t2\t0\t0\t0\t0\t0.0\t0.0\t10\n";
  Geometry g = parse(text);
  CHECK_EQ(g.malformed_lines().size(), 3);
  CHECK(g.lookup(0, 0, 0, 0) == make_strip(Plane::U, 0, 2));
}

static void test_aget_index_out_of_hardware_range_is_malformed_not_corrupting() {
  // aget=4 は AsAd 1 枚あたりの物理チップ数(4 個, 0..3)を超える。境界チェックを忘れると
  // 次の AsAd ブロックへ書き込みが漏れてデータ破壊になるので、malformed 扱いで
  // 弾かれることを確認する。
  std::string text = "U\t0\t1\t0\t0\t4\t0\t0.0\t0.0\t10\n";
  Geometry g = parse(text);
  CHECK_EQ(g.malformed_lines().size(), 1);
  CHECK_EQ(g.cobo_count(), 0);
}

static void test_unmapped_hit_count_increments_on_every_unmapped_lookup() {
  std::string text = "U\t0\t1\t0\t0\t0\t0\t0.0\t0.0\t10\n";
  Geometry g = parse(text);
  CHECK_EQ(g.unmapped_hit_count(), 0);

  // in-bounds だが未記載(同じ active aget の未使用信号 ch)。
  CHECK(g.lookup(0, 0, 0, 63) == make_unmapped());
  CHECK_EQ(g.unmapped_hit_count(), 1);

  // 完全に範囲外(cobo が存在しない)。
  CHECK(g.lookup(9, 0, 0, 0) == make_unmapped());
  CHECK_EQ(g.unmapped_hit_count(), 2);

  // Strip の再ヒットはカウントしない。
  (void)g.lookup(0, 0, 0, 0);
  CHECK_EQ(g.unmapped_hit_count(), 2);
}

// ---------------------------------------------------------------------------
// main — 引数ありならダンプモード(run_geo_conformance.sh の相方)、
//        引数なしなら CHECK 群を走らせる。
// ---------------------------------------------------------------------------

int main(int argc, char** argv) {
  if (argc >= 2) {
    try {
      Geometry g = load(argv[1]);
      std::string dump = dump_tsv(g);
      std::fwrite(dump.data(), 1, dump.size(), stdout);
    } catch (const std::exception& e) {
      std::fprintf(stderr, "test_geo: failed to load %s: %s\n", argv[1], e.what());
      return 1;
    }
    return 0;
  }

  test_reorder_loop_matches_constant_table_for_all_64_inputs();
  test_plane_parse_accepts_only_uppercase_uvw();

  test_header_scalars_are_parsed_verbatim();
  test_strip_channels_resolve_to_expected_plane_section_strip();
  test_aux_channels_resolve_with_their_name();
  test_fpn_channels_resolve_for_every_active_aget_only();
  test_unmapped_channels_are_unmapped_within_active_aget_too();
  test_max_strip_matches_fixture_hand_count();
  test_cobo_count_is_one_for_single_cobo_fixture();
  test_clean_fixture_has_no_duplicate_or_malformed_warnings();
  test_load_missing_file_throws();

  test_cobo_count_reflects_both_cobos();
  test_identical_asad_aget_raw_ch_resolve_independently_per_cobo();
  test_asad_count_can_differ_per_cobo();
  test_fpn_is_resolved_independently_for_each_cobo_asad_pair();
  test_2cobo_fixture_has_no_warnings();

  test_legacy_lines_default_cobo_asad_section_to_zero();
  test_legacy_fpn_is_still_resolved_for_active_agets();
  test_legacy_max_strip_matches_hand_count();

  test_dump_covers_every_slot_of_the_dense_array();
  test_dump_rows_are_tab_separated_with_eight_columns();
  test_dump_first_row_is_the_first_strip_channel();
  test_dump_fpn_row_has_empty_plane_section_strip_columns();
  test_dump_aux_row_has_empty_plane_section_strip_columns();
  test_dump_unmapped_row_has_empty_plane_section_strip_columns();
  test_dump_tsv_does_not_increment_unmapped_hit_count();

  test_duplicate_channel_key_warns_and_keeps_first();
  test_malformed_lines_are_recorded_not_fatal();
  test_aget_index_out_of_hardware_range_is_malformed_not_corrupting();
  test_unmapped_hit_count_increments_on_every_unmapped_lookup();

  return tpccheck::report("test_geo");
}
