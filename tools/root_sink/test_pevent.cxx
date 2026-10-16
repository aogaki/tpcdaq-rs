// test_pevent.cxx — PEventTPC 充填(pevent_fill.hpp)と Recorder(root_recorder.hpp)の
//                   単体テスト(TODO/020 → 054)。
//
// **`make test` からは外してある**(`make test-root`)—— ROOT + TPCReco クラスを
// リンクする(SPEC §6.4 v1.8 のビルド時参照)。
//
// 試験方式は既存の流儀(素の CHECK + main、check.hpp)。やっていること:
//
//   1. 合成 Fragment → PEventTPC の chargeMap を**手計算オラクル**と全 key 照合
//      (strip 射影 / FPN リオーダ / signal 窓 / ペデスタル数値 / `+=` 加算)。
//      **v1.17(054)で Filler は OwnedFragment を直読する** —— GDataFrame 中間表現は撤去。
//   2. Recorder で書いた .root を**同プロセスで開き直して**、TPCReco `EventSourceROOT`
//      が期待する形(ツリー `TPCData` / ブランチ `Event` / myChargeArray 無効)と
//      streamer(version + checksum)を機械照合する。
//   3. env `TPCDAQ_REAL_PEVENT` があれば**実機 grawToEventTPC 出力**の streamer /
//      ツリー / ブランチ / pedestalSubtracted と突き合わせる(未設定なら SKIP を印字)。

#include <sys/resource.h>
#include <sys/stat.h>

#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <ctime>
#include <memory>
#include <string>
#include <vector>

#include <TBranch.h>
#include <TClass.h>
#include <TFile.h>
#include <TKey.h>
#include <TList.h>
#include <TROOT.h>
#include <TStreamerInfo.h>
#include <TTree.h>

#include "TPCReco/PEventTPC.h"
#include "check.hpp"
#include "eb_core.hpp"
#include "pevent_fill.hpp"
#include "root_recorder.hpp"

namespace {

// テストが使う合成ジオメトリ(実 .dat はリポに入れない —— CLAUDE.md)。
const char* kFixtureMiniReduced = "../../tests/fixtures/geometry_mini_reduced.dat";

// 実機オラクル(ローカルのみ)。未設定なら構造一致テストは SKIP。
const char* kRealPEventEnv = "TPCDAQ_REAL_PEVENT";

// TODO/020 / SPEC §6.4 が固定した streamer checksum。**コピー元は HIGS2026_online 固定**
// (myChargeArray が出入りした他スナップショットと混ぜると割れる)。
constexpr unsigned kChecksumPEventTPC = 0xf71c32cfu;
constexpr unsigned kChecksumEventInfo = 0xfea093e4u;
constexpr unsigned kChecksumGlobalProperties = 0x49e6428cu;
constexpr int kStreamerVersion = 1;

// double の厳密比較(許容差 0)。落ちたときに実値が出る。
#define CHECK_D(actual, expected)                                                     \
  do {                                                                                \
    const double a_ = (actual);                                                       \
    const double e_ = (expected);                                                     \
    if (a_ == e_) {                                                                   \
      ++tpccheck::g_pass;                                                             \
    } else {                                                                          \
      ++tpccheck::g_fail;                                                             \
      std::printf("FAIL %s:%d  %s == %s (got %.17g, want %.17g)\n", __FILE__,         \
                  __LINE__, #actual, #expected, a_, e_);                              \
    }                                                                                 \
  } while (0)

// ---------------------------------------------------------------------------
// 小道具
// ---------------------------------------------------------------------------

// chargeMap の 1 key。無ければ NaN(呼び手の CHECK_D が必ず落ちる)。
double charge_at(const PEventTPC& e, int dir, int section, int number, int cell) {
  const auto it = e.GetChargeMap().find(std::make_tuple(dir, section, number, cell));
  if (it == e.GetChargeMap().end()) return std::nan("");
  return it->second;
}

bool has_key(const PEventTPC& e, int dir, int section, int number, int cell) {
  return e.GetChargeMap().count(std::make_tuple(dir, section, number, cell)) != 0;
}

// `rm -rf`(test_recorder.cxx と同じもの)。
void remove_tree(const std::string& dir) {
  for (const std::string& name : rootsink::list_directory(dir)) {
    const std::string path = dir + "/" + name;
    struct stat st;
    if (::stat(path.c_str(), &st) == 0 && S_ISDIR(st.st_mode)) {
      remove_tree(path);
    } else {
      std::remove(path.c_str());
    }
  }
  ::rmdir(dir.c_str());
}

// 使い捨てディレクトリ(固定パスを書かない = 並列実行・再実行に耐える)。
// test_recorder.cxx と同じ流儀。
std::string scratch_dir(const char* tag) {
  const char* tmp = std::getenv("TMPDIR");
  std::string base = (tmp != nullptr && tmp[0] != '\0') ? std::string(tmp) : std::string("/tmp");
  if (!base.empty() && base.back() == '/') base.pop_back();
  char suffix[64];
  std::snprintf(suffix, sizeof(suffix), "/tpcdaq_test_pevent_%d_%s",
                static_cast<int>(::getpid()), tag);
  const std::string dir = base + suffix;
  remove_tree(dir);  // 前回の残骸を掃除(finalize 先が既にあると rename しない)
  rootsink::mkdir_p(dir);
  return dir;
}

// 1 フラグメント分の OwnedFragment を作る。items は (aget, chan, bucket, adc) の
// パック形式(SPEC §2.4 と pevent_fill.hpp::Filler が読む形)。
struct Item {
  uint32_t aget;
  uint32_t chan;  // **raw** チャンネル 0–67
  uint32_t bucket;
  uint32_t adc;
};

rootsink::OwnedFragment make_fragment(uint32_t event_idx, uint8_t cobo, uint8_t asad,
                                      uint64_t event_time, const std::vector<Item>& items) {
  rootsink::OwnedFragment f;
  f.run_number = 7;
  f.event_idx = event_idx;
  f.event_time = event_time;
  f.cobo = cobo;
  f.asad = asad;
  f.items.reserve(items.size() * 4);
  for (const Item& it : items) {
    const uint32_t w = ((it.aget & 0x3u) << 30) | ((it.chan & 0x7Fu) << 23) |
                       ((it.bucket & 0x1FFu) << 14) | (it.adc & 0xFFFu);
    f.items.push_back(static_cast<uint8_t>(w & 0xFFu));
    f.items.push_back(static_cast<uint8_t>((w >> 8) & 0xFFu));
    f.items.push_back(static_cast<uint8_t>((w >> 16) & 0xFFu));
    f.items.push_back(static_cast<uint8_t>((w >> 24) & 0xFFu));
  }
  return f;
}

// ---------------------------------------------------------------------------
// 1. 合成フレーム(全テストが共有する「非対称データ」)
// ---------------------------------------------------------------------------
//
// tests/fixtures/geometry_mini_reduced.dat(cobo=0, asad=0)の割り当て:
//   AGET 0: 信号 ch 0–3 → U section 0 strip 1–4 / 信号 ch 4 → AUX(TestAux0)
//   AGET 1: 信号 ch 0–2 → V section 0 strip 1–3 / 信号 ch 3–6 → W section 0 strip 1–4
//           信号 ch 7 → AUX(TestAux1)
// 信号 ch 0–10 の raw ch は同値(FPN リオーダ表 kReorderFromGeometryToGraw は
// ch 11 の手前まで恒等)—— つまり raw 0 = 信号 0、raw 1 = 信号 1、raw 3 = 信号 3、
// raw 4 = 信号 4。**FPN は raw 11/22/45/56**(信号番号ではない)。
//
// 中身(**非対称**。同じ値を並べない):
//   AGET 0 FPN raw 11/22/45/56、cell 2,3,4,5:
//       raw11 = 10,20,30,40 / raw22 = 12,22,31,42 / raw45 = 14,24,32,44 / raw56 = 20,26,35,46
//     → cell 毎 4ch 平均 = (10+12+14+20)/4 = 14.0 / (20+22+24+26)/4 = 23.0 /
//                          (30+31+32+35)/4 = 32.0 / (40+42+44+46)/4 = 43.0
//   AGET 0 raw 0(U strip 1): cell 2,3,4,5 = 100,110,120,130
//   AGET 0 raw 1(U strip 2): cell   3,4,5 =     200,210,220   ← cell 2 が無い(非対称)
//   AGET 0 raw 4(AUX)      : cell 2,3,4,5 = 900,901,902,903   ← strip でないので捨てる
//   AGET 1 FPN raw 11/22/45/56、cell 4,5 のみ:
//       raw11 = 4,10 / raw22 = 5,10 / raw45 = 6,10 / raw56 = 9,14
//     → cell4 平均 = (4+5+6+9)/4 = 6.0 / cell5 平均 = (10+10+10+14)/4 = 11.0
//       cell 2,3 は **エントリ 0 = 平均 0.0**(FPN 欠損の扱いを試す)
//   AGET 1 raw 0(V strip 1): cell 4,5 = 500,505   ← ペデスタル窓に 1 点も無い
//   AGET 1 raw 1(V strip 2): cell 2,4 = 300,310
//   AGET 1 raw 3(W strip 1): cell 6   = 777       ← signal 窓 [2,5] の外 = 捨てる
std::vector<Item> synthetic_items() {
  return {
      // AGET 0 FPN
      {0, 11, 2, 10},  {0, 11, 3, 20},  {0, 11, 4, 30},  {0, 11, 5, 40},
      {0, 22, 2, 12},  {0, 22, 3, 22},  {0, 22, 4, 31},  {0, 22, 5, 42},
      {0, 45, 2, 14},  {0, 45, 3, 24},  {0, 45, 4, 32},  {0, 45, 5, 44},
      {0, 56, 2, 20},  {0, 56, 3, 26},  {0, 56, 4, 35},  {0, 56, 5, 46},
      // AGET 0 normal
      {0, 0, 2, 100},  {0, 0, 3, 110},  {0, 0, 4, 120},  {0, 0, 5, 130},
      {0, 1, 3, 200},  {0, 1, 4, 210},  {0, 1, 5, 220},
      {0, 4, 2, 900},  {0, 4, 3, 901},  {0, 4, 4, 902},  {0, 4, 5, 903},
      // AGET 1 FPN
      {1, 11, 4, 4},   {1, 11, 5, 10},
      {1, 22, 4, 5},   {1, 22, 5, 10},
      {1, 45, 4, 6},   {1, 45, 5, 10},
      {1, 56, 4, 9},   {1, 56, 5, 14},
      // AGET 1 normal
      {1, 0, 4, 500},  {1, 0, 5, 505},
      {1, 1, 2, 300},  {1, 1, 4, 310},
      {1, 3, 6, 777},
  };
}

// テスト用の窓(手計算できる幅にする)。既定(5/25/5/506)は別テストで見る。
tpcpevent::FillConfig tiny_windows(bool remove_pedestal) {
  tpcpevent::FillConfig cfg;
  cfg.remove_pedestal = remove_pedestal;
  cfg.min_pedestal_cell = 2;
  cfg.max_pedestal_cell = 3;
  cfg.min_signal_cell = 2;
  cfg.max_signal_cell = 5;
  return cfg;
}

// ---------------------------------------------------------------------------
// 2. 充填テスト(ペデスタル OFF)
// ---------------------------------------------------------------------------
//
// 期待値 = 生 ADC そのまま(signal 窓 [2,5] だけ効く)。
//   U1 cell2–5 = 100,110,120,130 / U2 cell3–5 = 200,210,220
//   V1 cell4,5 = 500,505         / V2 cell2 = 300, cell4 = 310
//   AUX(raw 4)・FPN・W1(cell 6 = 窓外)は 1 つも入らない → 全 11 key。
void test_fill_without_pedestal() {
  const tpcgeo::Geometry geo = tpcgeo::load(kFixtureMiniReduced);
  tpcpevent::Filler filler(geo, tiny_windows(/*remove_pedestal=*/false));

  PEventTPC ev;
  ev.Clear();
  filler.add_fragment(make_fragment(42, 0, 0, 123456789, synthetic_items()), ev);

  CHECK_EQ(ev.GetChargeMap().size(), 11);
  CHECK_D(charge_at(ev, 0, 0, 1, 2), 100.0);
  CHECK_D(charge_at(ev, 0, 0, 1, 3), 110.0);
  CHECK_D(charge_at(ev, 0, 0, 1, 4), 120.0);
  CHECK_D(charge_at(ev, 0, 0, 1, 5), 130.0);
  CHECK_D(charge_at(ev, 0, 0, 2, 3), 200.0);
  CHECK_D(charge_at(ev, 0, 0, 2, 4), 210.0);
  CHECK_D(charge_at(ev, 0, 0, 2, 5), 220.0);
  CHECK_D(charge_at(ev, 1, 0, 1, 4), 500.0);
  CHECK_D(charge_at(ev, 1, 0, 1, 5), 505.0);
  CHECK_D(charge_at(ev, 1, 0, 2, 2), 300.0);
  CHECK_D(charge_at(ev, 1, 0, 2, 4), 310.0);
  // U strip 2 の cell 2 はサンプルが無い(**key を作らない**)。
  CHECK(!has_key(ev, 0, 0, 2, 2));
  // W strip 1 は cell 6 = signal 窓の外 → 1 key も無い。
  CHECK(!has_key(ev, 2, 0, 1, 6));
  // AUX(raw 4)は strip ではないので捨てる —— 数える(silent にしない)。
  // AGET0: raw 4(AUX)/ AGET1: raw 3(W1、窓外だが strip なので数えない)。
  // strip 以外で「サンプルを持っていた」チャンネルは AUX の 1 本だけ。
  CHECK_EQ(filler.channels_without_strip(), 1);
  CHECK_EQ(filler.keys_out_of_range(), 0);
}

// ---------------------------------------------------------------------------
// 3. 充填テスト(ペデスタル ON)—— TODO/020 の核
// ---------------------------------------------------------------------------
//
// 算法(PedestalCalculatorGRAW.cpp = TPCReco 実運用):
//   ① FPN 4ch を cell 毎に平均(ペデスタル窓・signal 窓それぞれ)
//   ② normal ch のペデスタル窓で `raw − FPN平均(ped)` をチャンネル毎に平均 = オフセット
//   ③ 補正 = オフセット + FPN平均(signal)[cell]、格納値 = raw − 補正
//
// **手計算(窓: ped = [2,3] / signal = [2,5])**
//   AGET0 FPN 平均: cell2 = 14.0, cell3 = 23.0, cell4 = 32.0, cell5 = 43.0
//   AGET1 FPN 平均: cell2 = 0.0(エントリ無し), cell3 = 0.0, cell4 = 6.0, cell5 = 11.0
//
//   U strip 1(AGET0 raw 0、cell 2–5 = 100,110,120,130):
//     オフセット = ((100−14) + (110−23)) / 2 = (86 + 87)/2 = **86.5**
//     cell2: 100 − (86.5 + 14) = **−0.5**
//     cell3: 110 − (86.5 + 23) = **+0.5**
//     cell4: 120 − (86.5 + 32) = **+1.5**
//     cell5: 130 − (86.5 + 43) = **+0.5**
//   U strip 2(AGET0 raw 1、cell 3–5 = 200,210,220): ペデスタル窓は cell3 の 1 点だけ
//     オフセット = (200 − 23) / 1 = **177.0**
//     cell3: 200 − (177 + 23) = **0.0**(key は作られる)
//     cell4: 210 − (177 + 32) = **+1.0**
//     cell5: 220 − (177 + 43) = **0.0**
//   V strip 1(AGET1 raw 0、cell 4,5 = 500,505): ペデスタル窓に 1 点も無い
//     → TProfile の空ビン = **オフセット 0.0**(GetBinContent の意味論)
//     cell4: 500 − (0 + 6) = **494.0**
//     cell5: 505 − (0 + 11) = **494.0**
//   V strip 2(AGET1 raw 1、cell 2 = 300, cell 4 = 310): FPN が cell2 に無い
//     オフセット = (300 − 0) / 1 = **300.0**
//     cell2: 300 − (300 + 0) = **0.0**
//     cell4: 310 − (300 + 6) = **+10.0 − 6.0 = 4.0**
void test_fill_with_pedestal() {
  const tpcgeo::Geometry geo = tpcgeo::load(kFixtureMiniReduced);
  tpcpevent::Filler filler(geo, tiny_windows(/*remove_pedestal=*/true));

  PEventTPC ev;
  ev.Clear();
  filler.add_fragment(make_fragment(42, 0, 0, 123456789, synthetic_items()), ev);

  CHECK_EQ(ev.GetChargeMap().size(), 11);
  CHECK_D(charge_at(ev, 0, 0, 1, 2), -0.5);
  CHECK_D(charge_at(ev, 0, 0, 1, 3), 0.5);
  CHECK_D(charge_at(ev, 0, 0, 1, 4), 1.5);
  CHECK_D(charge_at(ev, 0, 0, 1, 5), 0.5);
  CHECK_D(charge_at(ev, 0, 0, 2, 3), 0.0);
  CHECK_D(charge_at(ev, 0, 0, 2, 4), 1.0);
  CHECK_D(charge_at(ev, 0, 0, 2, 5), 0.0);
  CHECK_D(charge_at(ev, 1, 0, 1, 4), 494.0);
  CHECK_D(charge_at(ev, 1, 0, 1, 5), 494.0);
  CHECK_D(charge_at(ev, 1, 0, 2, 2), 0.0);
  CHECK_D(charge_at(ev, 1, 0, 2, 4), 4.0);
}

// ペデスタルは **(cobo,asad) フレーム毎にリセット**(イベント内で完結・run 状態なし)。
// 同じフレームを 2 回食わせたら、値は「2 倍」になる(`+=` 加算)だけで、
// ペデスタルの計算そのものは 1 回目と同じでなければならない。
void test_pedestal_is_reset_per_frame() {
  const tpcgeo::Geometry geo = tpcgeo::load(kFixtureMiniReduced);
  tpcpevent::Filler filler(geo, tiny_windows(/*remove_pedestal=*/true));

  PEventTPC ev;
  ev.Clear();
  filler.add_fragment(make_fragment(42, 0, 0, 1, synthetic_items()), ev);
  filler.add_fragment(make_fragment(42, 0, 0, 1, synthetic_items()), ev);

  // key 集合は変わらず、値だけ 2 倍(AddValByStrip は **+=**)。
  CHECK_EQ(ev.GetChargeMap().size(), 11);
  CHECK_D(charge_at(ev, 0, 0, 1, 2), -1.0);   // -0.5 × 2
  CHECK_D(charge_at(ev, 0, 0, 1, 4), 3.0);    // +1.5 × 2
  CHECK_D(charge_at(ev, 1, 0, 1, 4), 988.0);  // 494 × 2
}

// ---------------------------------------------------------------------------
// 4. signal 窓の既定(5..506)の境界
// ---------------------------------------------------------------------------
//
// cell 4 = 落ちる / 5 = 入る / 506 = 入る / 507 = 落ちる(SPEC §6.4 の既定値)。
void test_default_signal_window_bounds() {
  const tpcgeo::Geometry geo = tpcgeo::load(kFixtureMiniReduced);
  tpcpevent::FillConfig cfg;  // 既定 = 5 / 25 / 5 / 506
  cfg.remove_pedestal = false;
  CHECK_EQ(cfg.min_pedestal_cell, 5);
  CHECK_EQ(cfg.max_pedestal_cell, 25);
  CHECK_EQ(cfg.min_signal_cell, 5);
  CHECK_EQ(cfg.max_signal_cell, 506);
  tpcpevent::Filler filler(geo, cfg);

  // AGET0 raw 0 = U strip 1 に 4 点だけ置く(非対称値)。
  PEventTPC ev;
  ev.Clear();
  filler.add_fragment(
      make_fragment(1, 0, 0, 1,
                    {{0, 0, 4, 111}, {0, 0, 5, 222}, {0, 0, 506, 333}, {0, 0, 507, 444}}),
      ev);

  CHECK_EQ(ev.GetChargeMap().size(), 2);
  CHECK(!has_key(ev, 0, 0, 1, 4));
  CHECK_D(charge_at(ev, 0, 0, 1, 5), 222.0);
  CHECK_D(charge_at(ev, 0, 0, 1, 506), 333.0);
  CHECK(!has_key(ev, 0, 0, 1, 507));
}

// ---------------------------------------------------------------------------
// 5. 同じ strip に載る 2 チャンネルは **加算**される(AddValByStrip の `+=`)
// ---------------------------------------------------------------------------
//
// 手組みの .dat: AGET0 信号 ch0 と AGET1 信号 ch0 を **同じ** U section 0 strip 1 に
// 割り当てる(実機では 1 本の strip が複数チャンネルに跨がる配線がありうる)。
void test_same_strip_accumulates() {
  const char* dat =
      "U\t0\t1\t0\t0\t0\t0\t0.0\t0.0\t10\n"
      "U\t0\t1\t0\t0\t1\t0\t0.0\t0.0\t10\n";
  const tpcgeo::Geometry geo = tpcgeo::parse(dat);
  tpcpevent::Filler filler(geo, tiny_windows(/*remove_pedestal=*/false));

  // 同じ cell 3 に別々の値を置く: 40 + 2 = 42。
  PEventTPC ev;
  ev.Clear();
  filler.add_fragment(make_fragment(1, 0, 0, 1, {{0, 0, 3, 40}, {1, 0, 3, 2}}), ev);

  CHECK_EQ(ev.GetChargeMap().size(), 1);
  CHECK_D(charge_at(ev, 0, 0, 1, 3), 42.0);
}

// ---------------------------------------------------------------------------
// 6. PEventTPC の固定長配列に収まらない strip 番号は**捨てて数える**
// ---------------------------------------------------------------------------
//
// `PEventTPC::myChargeArray[3][3][256][512]` は strip 番号 0–255 しか持てない
// (`AddValByStrip` は chargeMap と配列の**両方**に書く)。ELITPC の 1–1024 は
// この配列をはみ出す —— 黙って UB を踏まず、落として数える(CLAUDE.md)。
void test_strip_number_beyond_array_is_dropped() {
  const char* dat =
      "U\t0\t255\t0\t0\t0\t0\t0.0\t0.0\t10\n"   // 収まる(境界の内側)
      "U\t0\t256\t0\t0\t0\t1\t0.0\t0.0\t10\n";  // 収まらない(境界のすぐ外)
  const tpcgeo::Geometry geo = tpcgeo::parse(dat);
  tpcpevent::Filler filler(geo, tiny_windows(/*remove_pedestal=*/false));

  PEventTPC ev;
  ev.Clear();
  filler.add_fragment(make_fragment(1, 0, 0, 1, {{0, 0, 3, 11}, {0, 1, 3, 22}, {0, 1, 4, 33}}),
                      ev);

  CHECK_EQ(ev.GetChargeMap().size(), 1);
  CHECK_D(charge_at(ev, 0, 0, 255, 3), 11.0);
  CHECK(!has_key(ev, 0, 0, 256, 3));
  CHECK_EQ(filler.keys_out_of_range(), 2);  // strip 256 の 2 サンプル
}

// ---------------------------------------------------------------------------
// 7. ジオメトリに無い (cobo,asad) のフレームは丸ごと捨てて数える
// ---------------------------------------------------------------------------
void test_frame_outside_geometry() {
  const tpcgeo::Geometry geo = tpcgeo::load(kFixtureMiniReduced);  // cobo 0 / asad 0 のみ
  tpcpevent::Filler filler(geo, tiny_windows(/*remove_pedestal=*/false));

  PEventTPC ev;
  ev.Clear();
  // asad=3 は .dat に無い
  filler.add_fragment(make_fragment(1, 0, 3, 1, {{0, 0, 3, 40}}), ev);

  CHECK_EQ(ev.GetChargeMap().size(), 0);
  CHECK_EQ(filler.frames_outside_geometry(), 1);
}

// ---------------------------------------------------------------------------
// 7b. ZS(ゼロサプレッション)フレームと pedestal 表(TODO/151)
// ---------------------------------------------------------------------------
//
// zCoBo の ZS は閾値未満のサンプルを**送らない**(FPN ch も抑制)。そのまま従来算法に
// 掛けると、ペデスタル窓のサンプルがほぼ無いので offset ≈ 0・FPN 項 0 になり、
// chargeMap が生 ADC のまま残る(archive/149 run 133)。ZS らしいフレームに限って
// `calib pedestal` の表の mean を引く(表モード)。
//
// 窓(手計算できる幅): ペデスタル [2,4](= 3 cell)/ signal [2,9]。
tpcpevent::FillConfig zs_windows() {
  tpcpevent::FillConfig cfg;
  cfg.remove_pedestal = true;
  cfg.min_pedestal_cell = 2;
  cfg.max_pedestal_cell = 4;
  cfg.min_signal_cell = 2;
  cfg.max_signal_cell = 9;
  return cfg;
}

// pedestal 表(asad 0)。値は非対称、すべて 2 進で厳密に表せる 1/4 刻み。
//   AGET0 raw0 (U1) = 100.25 / raw1 (U2) = 200.5 / raw2 (U3) = 150.0
//   AGET1 raw0 (V1) = 300.75
//   FPN も表にある(AGET0 raw11 = 55.5)—— 表モードは FPN 項を**使わない**ことを試す。
//   AUX(AGET0 raw4)は表に**無い** —— strip でない ch は表を引く前に捨てる(計数しない)。
tpcpevent::PedTable zs_table() {
  tpcpevent::PedTable t;
  t.mean[0][0][0] = 100.25;
  t.mean[0][0][1] = 200.5;
  t.mean[0][0][2] = 150.0;
  t.mean[0][0][11] = 55.5;
  t.mean[0][1][0] = 300.75;
  t.channels = 5;
  return t;
}

// ZS の模擬: 各 ch の全波形(cell 0–11)から「表の mean + 10 未満」のセルを除いたもの。
// FPN は全部無い(ZS で FPN ch も抑制される)。
//   U1 (AGET0 raw0, mean 100.25 → 閾値 110.25)
//      全波形 101 99 102 100 98 140 180 150 115 105 101 100
//      残る = cell5 140 / cell6 180 / cell7 150 / cell8 115
//   U2 (AGET0 raw1, mean 200.5 → 閾値 210.5)
//      全波形 201 199 202 215 200 205 260 230 209 201 212 198
//      残る = cell3 215(ペデスタル窓のノイズ山)/ cell6 260 / cell7 230 / cell10 212
//   U3 (AGET0 raw2, mean 150.0 → 閾値 160)
//      全波形 150 151 149 150 152 148 150 151 149 150 150 151 → 全部抑制 = ch ごと無い
//   V1 (AGET1 raw0, mean 300.75 → 閾値 310.75)
//      全波形 301 300 302 299 320 350 330 305 301 300 299 301
//      残る = cell4 320 / cell5 350 / cell6 330
//   AUX (AGET0 raw4)                     残る = cell6 999
//
// 述語(サンプルのある ch のうちペデスタル窓 [2,4] のサンプル数 < 3 が過半か):
//   サンプルのある ch = U1, U2, V1, AUX の 4 本。窓内の件数 = 0 / 1(cell3)/ 1(cell4)/ 0
//   → 4 本とも < 3 → 4 × 2 > 4 → **ZS らしい**。
std::vector<Item> zs_items() {
  return {
      {0, 0, 5, 140}, {0, 0, 6, 180}, {0, 0, 7, 150}, {0, 0, 8, 115},
      {0, 1, 3, 215}, {0, 1, 6, 260}, {0, 1, 7, 230}, {0, 1, 10, 212},
      {0, 4, 6, 999},
      {1, 0, 4, 320}, {1, 0, 5, 350}, {1, 0, 6, 330},
  };
}

// 全 68 ch × cell 0–11 が揃ったフレーム(フル読み出し = 非 ZS)。値は非対称な式。
std::vector<Item> full_items(const std::vector<uint32_t>& agets) {
  std::vector<Item> items;
  for (uint32_t aget : agets) {
    for (uint32_t chan = 0; chan < 68; ++chan) {
      for (uint32_t cell = 0; cell < 12; ++cell) {
        items.push_back(Item{aget, chan, cell, 200 + 7 * aget + 3 * chan + (cell * cell) % 13});
      }
    }
  }
  return items;
}

// chargeMap の完全一致(key 集合と値のビット一致)。
bool same_charge_map(const PEventTPC& a, const PEventTPC& b) {
  return a.GetChargeMap() == b.GetChargeMap();
}

// (a) ZS フラグメント + 表 → 存在するサンプルだけ `adc − mean`(FPN 項なし)。
//
// **手計算**(signal 窓 [2,9]):
//   U1 (0,0,1): cell5 140 − 100.25 = 39.75 / cell6 180 − 100.25 = 79.75
//               cell7 150 − 100.25 = 49.75 / cell8 115 − 100.25 = 14.75
//   U2 (0,0,2): cell3 215 − 200.5 = 14.5 / cell6 260 − 200.5 = 59.5 / cell7 230 − 200.5 = 29.5
//               cell10 = signal 窓の外 → key なし
//   V1 (1,0,1): cell4 320 − 300.75 = 19.25 / cell5 350 − 300.75 = 49.25
//               cell6 330 − 300.75 = 29.25
//   → 4 + 3 + 3 = 10 key。抑制されたセル(例 U1 cell4)と U3 には key を作らない。
void test_zs_frame_with_table_subtracts_the_table_mean() {
  const tpcgeo::Geometry geo = tpcgeo::load(kFixtureMiniReduced);
  const tpcpevent::PedTable table = zs_table();
  tpcpevent::Filler filler(geo, zs_windows(), &table);

  PEventTPC ev;
  ev.Clear();
  filler.add_fragment(make_fragment(5, 0, 0, 1, zs_items()), ev);

  CHECK_EQ(ev.GetChargeMap().size(), 10);
  CHECK_D(charge_at(ev, 0, 0, 1, 5), 39.75);
  CHECK_D(charge_at(ev, 0, 0, 1, 6), 79.75);
  CHECK_D(charge_at(ev, 0, 0, 1, 7), 49.75);
  CHECK_D(charge_at(ev, 0, 0, 1, 8), 14.75);
  CHECK_D(charge_at(ev, 0, 0, 2, 3), 14.5);
  CHECK_D(charge_at(ev, 0, 0, 2, 6), 59.5);
  CHECK_D(charge_at(ev, 0, 0, 2, 7), 29.5);
  CHECK_D(charge_at(ev, 1, 0, 1, 4), 19.25);
  CHECK_D(charge_at(ev, 1, 0, 1, 5), 49.25);
  CHECK_D(charge_at(ev, 1, 0, 1, 6), 29.25);
  CHECK(!has_key(ev, 0, 0, 1, 4));   // 抑制されたセルは埋めない
  CHECK(!has_key(ev, 0, 0, 2, 10));  // signal 窓の外
  CHECK(!has_key(ev, 0, 0, 3, 6));   // U3 は ch ごと抑制
  CHECK_EQ(filler.zs_frames_table(), 1);
  CHECK_EQ(filler.zs_frames_no_table(), 0);
  CHECK_EQ(filler.table_missing_channels(), 0);
  // AUX は strip でない —— 表に無くても「表に無い ch」とは数えない(先に役割で捨てる)。
  CHECK_EQ(filler.channels_without_strip(), 1);
}

// (b) 同じフラグメント、表なし → 従来算法のまま + no_table を数える(保存は止めない)。
//
// **手計算**(FPN が 1 本も無いので FPN 平均は全セル 0.0):
//   U1: ペデスタル窓 [2,4] にサンプル無し → オフセット 0.0 → 生 ADC のまま
//       cell5 140 / cell6 180 / cell7 150 / cell8 115
//   U2: 窓内は cell3 215 の 1 点 → オフセット 215.0
//       cell3 0.0 / cell6 260 − 215 = 45.0 / cell7 230 − 215 = 15.0
//   V1: 窓内は cell4 320 の 1 点 → オフセット 320.0
//       cell4 0.0 / cell5 30.0 / cell6 10.0
void test_zs_frame_without_table_keeps_the_old_algorithm_and_counts() {
  const tpcgeo::Geometry geo = tpcgeo::load(kFixtureMiniReduced);
  tpcpevent::Filler filler(geo, zs_windows());

  PEventTPC ev;
  ev.Clear();
  filler.add_fragment(make_fragment(5, 0, 0, 1, zs_items()), ev);

  CHECK_EQ(ev.GetChargeMap().size(), 10);
  CHECK_D(charge_at(ev, 0, 0, 1, 5), 140.0);
  CHECK_D(charge_at(ev, 0, 0, 1, 6), 180.0);
  CHECK_D(charge_at(ev, 0, 0, 1, 7), 150.0);
  CHECK_D(charge_at(ev, 0, 0, 1, 8), 115.0);
  CHECK_D(charge_at(ev, 0, 0, 2, 3), 0.0);
  CHECK_D(charge_at(ev, 0, 0, 2, 6), 45.0);
  CHECK_D(charge_at(ev, 0, 0, 2, 7), 15.0);
  CHECK_D(charge_at(ev, 1, 0, 1, 4), 0.0);
  CHECK_D(charge_at(ev, 1, 0, 1, 5), 30.0);
  CHECK_D(charge_at(ev, 1, 0, 1, 6), 10.0);
  CHECK_EQ(filler.zs_frames_no_table(), 1);
  CHECK_EQ(filler.zs_frames_table(), 0);

  // 2 フレーム目も数える(WARN は run 毎 1 回 —— 計数は毎フレーム)。
  ev.Clear();
  filler.add_fragment(make_fragment(6, 0, 0, 2, zs_items()), ev);
  CHECK_EQ(filler.zs_frames_no_table(), 2);
}

// (c) フル読み出し(非 ZS)+ 表 → 述語 false → **表なしと chargeMap がビット一致**
// (§12-3 の grawToEventTPC 一致オラクルが表の有無で変わらないことの単体版)。
void test_full_frame_with_table_is_bit_identical_to_without() {
  const tpcgeo::Geometry geo = tpcgeo::load(kFixtureMiniReduced);
  const tpcpevent::PedTable table = zs_table();
  tpcpevent::Filler with_table(geo, zs_windows(), &table);
  tpcpevent::Filler without_table(geo, zs_windows());

  const std::vector<Item> items = full_items({0, 1});
  // PEventTPC は myChargeArray(4.7 MB)を抱えるので 2 個はヒープに置く(スタック 8 MB)。
  std::unique_ptr<PEventTPC> a(new PEventTPC());
  std::unique_ptr<PEventTPC> b(new PEventTPC());
  a->Clear();
  b->Clear();
  with_table.add_fragment(make_fragment(9, 0, 0, 1, items), *a);
  without_table.add_fragment(make_fragment(9, 0, 0, 1, items), *b);

  // reduced ジオメトリの strip 11 本(U4 + V3 + W4)× signal 窓 [2,9] の 8 cell。
  CHECK_EQ(a->GetChargeMap().size(), 88);
  CHECK(same_charge_map(*a, *b));
  CHECK_EQ(with_table.zs_frames_table(), 0);
  CHECK_EQ(with_table.zs_frames_no_table(), 0);
  CHECK_EQ(without_table.zs_frames_no_table(), 0);
}

// (d) partial 読み出し(非 ZS)—— 読まれた ch は全セル揃い、読まれない ch は丸ごと無い。
// 述語は「サンプルのある ch」だけを見るので false → 従来算法(表ありでもビット一致)。
//   読まれた ch = AGET0 の FPN 4 本 + raw0(U1)、AGET1 raw3(W1)の計 6 本、各 cell 0–11。
void test_partial_non_zs_frame_is_not_zero_suppressed() {
  const tpcgeo::Geometry geo = tpcgeo::load(kFixtureMiniReduced);
  const tpcpevent::PedTable table = zs_table();
  tpcpevent::Filler with_table(geo, zs_windows(), &table);
  tpcpevent::Filler without_table(geo, zs_windows());

  std::vector<Item> items;
  for (const Item& it : full_items({0, 1})) {
    const bool read = (it.aget == 0 && (it.chan == 0 || it.chan == 11 || it.chan == 22 ||
                                        it.chan == 45 || it.chan == 56)) ||
                      (it.aget == 1 && it.chan == 3);
    if (read) items.push_back(it);
  }
  std::unique_ptr<PEventTPC> a(new PEventTPC());
  std::unique_ptr<PEventTPC> b(new PEventTPC());
  a->Clear();
  b->Clear();
  with_table.add_fragment(make_fragment(9, 0, 0, 1, items), *a);
  without_table.add_fragment(make_fragment(9, 0, 0, 1, items), *b);

  CHECK_EQ(a->GetChargeMap().size(), 16);  // U1 + W1 × 8 cell
  CHECK(same_charge_map(*a, *b));
  CHECK_EQ(with_table.zs_frames_table(), 0);
  CHECK_EQ(with_table.zs_frames_no_table(), 0);
}

// 述語の境界: 「過半」は**半数ちょうどを含まない**。
//   ① U1 全セル(窓内 3 件 = 揃い)+ V1 cell6 だけ(窓内 0 件)→ 1/2 = 半数 → 非 ZS
//   ② ①に U2 cell6 だけ(窓内 0 件)を足す → 2/3 = 過半 → ZS らしい
void test_zs_predicate_needs_a_strict_majority() {
  const tpcgeo::Geometry geo = tpcgeo::load(kFixtureMiniReduced);
  const tpcpevent::PedTable table = zs_table();
  tpcpevent::Filler filler(geo, zs_windows(), &table);

  std::vector<Item> half;
  for (uint32_t cell = 0; cell < 12; ++cell) half.push_back(Item{0, 0, cell, 101 + cell});
  half.push_back(Item{1, 0, 6, 330});
  PEventTPC ev;
  ev.Clear();
  filler.add_fragment(make_fragment(1, 0, 0, 1, half), ev);
  CHECK_EQ(filler.zs_frames_table(), 0);

  std::vector<Item> majority = half;
  majority.push_back(Item{0, 1, 6, 260});
  ev.Clear();
  filler.add_fragment(make_fragment(2, 0, 0, 1, majority), ev);
  CHECK_EQ(filler.zs_frames_table(), 1);
}

// (e) 表に無い strip ch → その ch は書かずに数える(他の ch は表モードのまま)。
//   (a) の表から V1(AGET1 raw0)を抜く → V1 の 3 key が消え、U1/U2 の 7 key は (a) と同値。
void test_zs_channel_missing_from_table_is_skipped_and_counted() {
  const tpcgeo::Geometry geo = tpcgeo::load(kFixtureMiniReduced);
  tpcpevent::PedTable table = zs_table();
  table.mean[0][1][0] = std::nan("");
  tpcpevent::Filler filler(geo, zs_windows(), &table);

  PEventTPC ev;
  ev.Clear();
  filler.add_fragment(make_fragment(5, 0, 0, 1, zs_items()), ev);

  CHECK_EQ(ev.GetChargeMap().size(), 7);
  CHECK_D(charge_at(ev, 0, 0, 1, 5), 39.75);
  CHECK_D(charge_at(ev, 0, 0, 2, 3), 14.5);
  CHECK(!has_key(ev, 1, 0, 1, 4));
  CHECK(!has_key(ev, 1, 0, 1, 5));
  CHECK_EQ(filler.table_missing_channels(), 1);
  CHECK_EQ(filler.zs_frames_table(), 1);
}

// --pedestal-remove 0(生 ADC)では ZS でも何も引かない —— 述語も表も使わない。
void test_zs_frame_without_pedestal_removal_stays_raw() {
  const tpcgeo::Geometry geo = tpcgeo::load(kFixtureMiniReduced);
  const tpcpevent::PedTable table = zs_table();
  tpcpevent::FillConfig cfg = zs_windows();
  cfg.remove_pedestal = false;
  tpcpevent::Filler filler(geo, cfg, &table);

  PEventTPC ev;
  ev.Clear();
  filler.add_fragment(make_fragment(5, 0, 0, 1, zs_items()), ev);

  CHECK_EQ(ev.GetChargeMap().size(), 10);
  CHECK_D(charge_at(ev, 0, 0, 1, 5), 140.0);
  CHECK_D(charge_at(ev, 1, 0, 1, 4), 320.0);
  CHECK_EQ(filler.zs_frames_table(), 0);
  CHECK_EQ(filler.zs_frames_no_table(), 0);
}

// ---------------------------------------------------------------------------
// 8. runId = run 開始時刻の %Y%m%d%H%M%S(TPCReco RunIdParser と同じ導出)
// ---------------------------------------------------------------------------
//
// 実機オラクル `PEventTPC_2026-08-11T07-47-37.051_0000.root` の runId = 20260811074737。
void test_run_id_from_tm() {
  std::tm tm{};
  tm.tm_year = 2026 - 1900;
  tm.tm_mon = 8 - 1;
  tm.tm_mday = 11;
  tm.tm_hour = 7;
  tm.tm_min = 47;
  tm.tm_sec = 37;
  CHECK_EQ(tpcpevent::run_id_from_tm(tm), 20260811074737LL);

  std::tm tm2{};  // 0 埋めの確認(1 桁の月日時分秒)
  tm2.tm_year = 2001 - 1900;
  tm2.tm_mon = 2 - 1;
  tm2.tm_mday = 3;
  tm2.tm_hour = 4;
  tm2.tm_min = 5;
  tm2.tm_sec = 6;
  CHECK_EQ(tpcpevent::run_id_from_tm(tm2), 20010203040506LL);
}

// 窓の妥当性(TPCReco PedestalCalculator::VerifyTimeCellIndices と同じ条件)。
void test_fill_config_validation() {
  std::string why;
  tpcpevent::FillConfig ok;
  CHECK(ok.validate(&why));

  tpcpevent::FillConfig bad_order;
  bad_order.min_signal_cell = 100;
  bad_order.max_signal_cell = 100;  // min >= max は不可
  CHECK(!bad_order.validate(&why));

  tpcpevent::FillConfig bad_range;
  bad_range.max_signal_cell = 512;  // 時間セルは 0–511
  CHECK(!bad_range.validate(&why));

  tpcpevent::FillConfig bad_ped;
  bad_ped.min_pedestal_cell = -1;
  CHECK(!bad_ped.validate(&why));
}

// ---------------------------------------------------------------------------
// 9. Recorder(PEventTPC モード)—— 書いて読み戻す
// ---------------------------------------------------------------------------

struct ReadEvent {
  long run_id = 0;
  unsigned event_id = 0;
  unsigned long timestamp = 0;
  bool pedestal_subtracted = false;
  int max_charge = -1;
  int integrated_charge = -1;
  int n_hits = -1;
  PEventTPC::chargeMapType charge_map;
};

// `path` の TPCData を読み切る。開けなければ空(呼び手が CHECK で落ちる)。
std::vector<ReadEvent> read_pevent_file(const std::string& path) {
  std::vector<ReadEvent> out;
  TFile* in = TFile::Open(path.c_str(), "READ");
  if (in == nullptr || in->IsZombie()) {
    std::printf("read_pevent_file: cannot open %s\n", path.c_str());
    delete in;
    return out;
  }
  TTree* tree = dynamic_cast<TTree*>(in->Get(rootsink::kPEventTreeName));
  if (tree == nullptr) {
    std::printf("read_pevent_file: no TTree named \"%s\" in %s\n", rootsink::kPEventTreeName,
                path.c_str());
    in->Close();
    delete in;
    return out;
  }
  PEventTPC* ev = nullptr;
  tree->SetBranchAddress(rootsink::kPEventBranchName, &ev);
  for (Long64_t i = 0; i < tree->GetEntries(); ++i) {
    tree->GetEntry(i);
    ReadEvent re;
    re.run_id = ev->GetEventInfo().GetRunId();
    re.event_id = ev->GetEventInfo().GetEventId();
    re.timestamp = ev->GetEventInfo().GetEventTimestamp();
    re.pedestal_subtracted = ev->GetEventInfo().GetPedestalSubtracted();
    re.max_charge = ev->GetEventInfo().GetProperties().max_charge;
    re.integrated_charge = ev->GetEventInfo().GetProperties().integrated_charge;
    re.n_hits = ev->GetEventInfo().GetProperties().n_hits;
    re.charge_map = ev->GetChargeMap();
    out.push_back(std::move(re));
  }
  in->Close();
  delete in;
  return out;
}

// TFile の StreamerInfo から (version, checksum) を引く。無ければ (-1, 0)。
std::pair<int, unsigned> streamer_of(TFile* f, const char* class_name) {
  TList* list = f->GetStreamerInfoList();
  if (list == nullptr) return {-1, 0};
  std::pair<int, unsigned> found{-1, 0};
  TIter next(list);
  for (TObject* o = next(); o != nullptr; o = next()) {
    TStreamerInfo* si = dynamic_cast<TStreamerInfo*>(o);
    if (si == nullptr) continue;
    if (std::strcmp(si->GetName(), class_name) == 0) {
      found = {si->GetClassVersion(), static_cast<unsigned>(si->GetCheckSum())};
      break;
    }
  }
  delete list;  // GetStreamerInfoList は呼び手所有のリストを返す(ROOT 6.32+)
  return found;
}

// 2 イベント + 同じ eventId の再送 1 件を書き、TPCReco が読む形になっているか見る。
void test_recorder_writes_tpcdata_tree() {
  const std::string dir = scratch_dir("recorder");
  const tpcgeo::Geometry geo = tpcgeo::load(kFixtureMiniReduced);

  {
    rootsink::RecorderConfig cfg;
    cfg.output_root = dir;
    cfg.geometry = &geo;
    cfg.fill = tiny_windows(/*remove_pedestal=*/true);
    rootsink::Recorder rec(cfg);

    rootsink::BuiltEvent ev0;
    ev0.run_number = 7;
    ev0.event_idx = 42;
    ev0.fragments.push_back(make_fragment(42, 0, 0, 1000, synthetic_items()));
    rec.write(ev0, 0);

    rootsink::BuiltEvent ev1;
    ev1.run_number = 7;
    ev1.event_idx = 43;
    ev1.fragments.push_back(make_fragment(43, 0, 0, 2000, synthetic_items()));
    rec.write(ev1, 0);

    // 同じ eventId をもう一度(遅延フラグメント相当)—— **書かずに数える**
    // (SPEC §6.3 v1.8 / grawToEventTPC の eventId 重複排除と同じ意味論)。
    rootsink::BuiltEvent late;
    late.run_number = 7;
    late.event_idx = 42;
    late.fragments.push_back(make_fragment(42, 0, 0, 1000, synthetic_items()));
    rec.write(late, 0);

    CHECK_EQ(rec.entries_written(), 2);
    CHECK_EQ(rec.duplicate_event_ids(), 1);
    CHECK(rec.fatal_reason() == nullptr);
    rec.close_run(7, 0);
  }  // ここで Recorder を畳んでから読み戻す

  const std::string path = dir + "/run0007/run0007.root";
  CHECK(rootsink::path_exists(path));

  TFile* f = TFile::Open(path.c_str(), "READ");
  CHECK(f != nullptr && !f->IsZombie());
  if (f == nullptr || f->IsZombie()) {
    delete f;
    return;
  }
  // 圧縮の既定 = 101(ZLIB-1)。実機オラクル(旧 ROOT 書き)は 1 = 「既定アルゴリズム
  // + level 1」で、**level が一致する**ことが互換の実質(TODO/014)。
  CHECK_EQ(f->GetCompressionLevel(), 1);

  // --- TPCReco EventSourceROOT がハード期待する形(SPEC §6.4)---
  TTree* tree = dynamic_cast<TTree*>(f->Get("TPCData"));
  CHECK(tree != nullptr);
  if (tree != nullptr) {
    CHECK_EQ(tree->GetEntries(), 2);
    CHECK(tree->GetBranch("Event") != nullptr);
    CHECK(std::strcmp(tree->GetTitle(), "") == 0);  // ConvertGrawFile.cpp:41 と同じ
    // splitlevel 2 = EventInfo が葉に割れる(ConvertGrawFile.cpp と同じ形)。
    CHECK(tree->GetBranch("myEventInfo.eventId") != nullptr);
    CHECK(tree->GetBranch("myChargeMap") != nullptr);
    // **myChargeArray は無効**: ブランチは streamer 由来で存在するが 1 件も書かない
    // (実運用 disabledBranches と同一。4.7 MB/イベントの節約)。
    TBranch* arr = tree->GetBranch("myChargeArray[3][3][256][512]");
    CHECK(arr != nullptr);
    if (arr != nullptr) CHECK_EQ(arr->GetEntries(), 0);
  }

  // --- streamer(version + checksum)---
  const auto pe = streamer_of(f, "PEventTPC");
  const auto ei = streamer_of(f, "eventraw::EventInfo");
  const auto gp = streamer_of(f, "eventraw::EventInfo::global_properties");
  CHECK_EQ(pe.first, kStreamerVersion);
  CHECK_EQ(pe.second, kChecksumPEventTPC);
  CHECK_EQ(ei.first, kStreamerVersion);
  CHECK_EQ(ei.second, kChecksumEventInfo);
  CHECK_EQ(gp.first, kStreamerVersion);
  CHECK_EQ(gp.second, kChecksumGlobalProperties);
  // 自分の辞書(TClass)と書いたファイルが同じ checksum であること。
  CHECK_EQ(TClass::GetClass("PEventTPC")->GetCheckSum(), kChecksumPEventTPC);
  CHECK_EQ(TClass::GetClass("eventraw::EventInfo")->GetCheckSum(), kChecksumEventInfo);
  CHECK_EQ(TClass::GetClass("eventraw::EventInfo::global_properties")->GetCheckSum(),
           kChecksumGlobalProperties);
  f->Close();
  delete f;

  // --- 中身(EventInfo と chargeMap)---
  const std::vector<ReadEvent> events = read_pevent_file(path);
  CHECK_EQ(events.size(), 2);
  if (events.size() == 2) {
    CHECK_EQ(events[0].event_id, 42);
    CHECK_EQ(events[1].event_id, 43);
    CHECK_EQ(events[0].timestamp, 1000);
    CHECK_EQ(events[1].timestamp, 2000);
    CHECK(events[0].pedestal_subtracted);
    // runId = run 開始時刻の %Y%m%d%H%M%S。実時刻なので値そのものは固定できないが、
    // **桁と下限**は固定できる(2026 年以降 = 14 桁)。
    CHECK(events[0].run_id >= 20260101000000LL);
    CHECK_EQ(events[0].run_id, events[1].run_id);  // run 中は一定
    // global_properties は 0 のまま(実運用の変換と同一 —— 埋めるのは解析側)。
    CHECK_EQ(events[0].max_charge, 0);
    CHECK_EQ(events[0].integrated_charge, 0);
    CHECK_EQ(events[0].n_hits, 0);
    // chargeMap は充填テストと同じ手計算オラクル。
    CHECK_EQ(events[0].charge_map.size(), 11);
    CHECK_D(events[0].charge_map.at(std::make_tuple(0, 0, 1, 2)), -0.5);
    CHECK_D(events[0].charge_map.at(std::make_tuple(1, 0, 1, 4)), 494.0);
    CHECK_EQ(events[1].charge_map.size(), 11);
  }
  remove_tree(dir);
}

// --run-id(TODO/021): 指定時は EventInfo.runId がその値になる(実データ照合と
// P4 controller 経路の受け口)。未指定(0)は壁時計由来 — こちらは
// test_recorder_writes_tpcdata_tree が「今日の日付の桁」であることまでは縛らず、
// run_id_from_tm の単体テストが式を固定している。
void test_recorder_run_id_override() {
  const std::string dir = scratch_dir("runid");
  const tpcgeo::Geometry geo = tpcgeo::load(kFixtureMiniReduced);
  const long kOverride = 20260811074737L;  // 実機オラクルと同じ形の値

  {
    rootsink::RecorderConfig cfg;
    cfg.output_root = dir;
    cfg.geometry = &geo;
    cfg.fill = tiny_windows(/*remove_pedestal=*/false);
    cfg.run_id_override = kOverride;
    rootsink::Recorder rec(cfg);

    rootsink::BuiltEvent ev;
    ev.run_number = 9;
    ev.event_idx = 0;
    ev.fragments.push_back(make_fragment(0, 0, 0, 500, synthetic_items()));
    rec.write(ev, 0);
    CHECK(rec.fatal_reason() == nullptr);
    rec.close_run(9, 0);
  }

  const std::vector<ReadEvent> events = read_pevent_file(dir + "/run0009/run0009.root");
  CHECK_EQ(events.size(), 1);
  if (events.size() == 1) {
    CHECK_EQ(events[0].run_id, kOverride);
  }
}

// TODO/151 §7: ZS カウンタは Recorder が **全 worker 合算**で出す(root_sink の終了 JSON
// `pevent_zs_frames_table / pevent_zs_frames_no_table / pevent_table_missing_channels` の材料)。
// workers = 2(round-robin)に ZS イベント 2 件 + フルイベント 1 件:
//   表あり → zs_frames_table = 2(両 worker に 1 件ずつ)/ no_table 0 / missing 0
//   書いたファイルの ZS イベントは表モードの値で、pedestalSubtracted = true。
void test_recorder_sums_zs_counters_over_workers() {
  const std::string dir = scratch_dir("zs");
  const tpcgeo::Geometry geo = tpcgeo::load(kFixtureMiniReduced);
  const tpcpevent::PedTable table = zs_table();

  {
    rootsink::RecorderConfig cfg;
    cfg.output_root = dir;
    cfg.geometry = &geo;
    cfg.fill = zs_windows();
    cfg.ped_table = &table;
    cfg.workers = 2;
    rootsink::Recorder rec(cfg);

    const std::vector<std::vector<Item>> frames = {zs_items(), full_items({0, 1}), zs_items()};
    for (uint32_t i = 0; i < frames.size(); ++i) {
      rootsink::BuiltEvent ev;
      ev.run_number = 15;
      ev.event_idx = i;
      ev.fragments.push_back(make_fragment(i, 0, 0, 100 + i, frames[i]));
      rec.write(ev, 0);
    }
    rec.close_run(15, 0);  // worker を流し切ってからカウンタを読む
    CHECK(rec.fatal_reason() == nullptr);
    CHECK_EQ(rec.zs_frames_table(), 2);
    CHECK_EQ(rec.zs_frames_no_table(), 0);
    CHECK_EQ(rec.table_missing_channels(), 0);
  }

  // worker 0 = event 0, 2(ZS)/ worker 1 = event 1(フル)。
  const std::vector<ReadEvent> w0 = read_pevent_file(dir + "/run0015/run0015_w0.root");
  CHECK_EQ(w0.size(), 2);
  if (w0.size() == 2) {
    CHECK(w0[0].pedestal_subtracted);
    CHECK_EQ(w0[0].charge_map.size(), 10);
    CHECK_D(w0[0].charge_map.at(std::make_tuple(0, 0, 1, 6)), 79.75);
    CHECK_D(w0[1].charge_map.at(std::make_tuple(1, 0, 1, 6)), 29.25);
  }
  remove_tree(dir);
}

// TODO/053: **1 イベント書く毎にメモリが増え続けない**。
//
// 元の実欠陥(031 soak が捕獲、053-A で計測特定)は GDataFrame 中間表現の側にあった:
// `GDataFrame::Clear()` が呼ぶ `TClonesArray::Clear()` はデストラクタを呼ばず、次フレームの
// `AddChannel()` が同じスロットへ placement new するので、`GDataChannel` が所有する
// `TRefArray fSamples` の内部配列が毎フレーム迷子になっていた(実機 mini で 0.55 MB/event)。
// **054 で GDataFrame 中間表現ごと撤去した**ので原因クラス自体が消えたが、この試験は
// 「Filler の作業マス + ROOT のバスケットが per-event に伸びない」ことの回帰として残す。
//
// 判定は `getrusage` の**ピーク RSS**(高々増える一方の値)。実機と同じ 139,264 items の
// フレームを流し、ウォームアップ後の増分を見る:
//   * 旧欠陥あり = 200 イベント × 0.55 MB = **110 MiB 増**(実測 +109〜115 MiB)
//   * 撤去後     = ROOT のバスケット等だけ = **数 MiB**(実測 +0〜4 MiB)
// しきい値 32 MiB はこの 2 群のちょうど中間より低い側に置いてある(片側 3 倍以上の余裕)。
//
// ジオメトリは reduced フィクスチャ(strip が数本だけ)—— 出力を小さく保ちつつ
// Filler は実機と同じ全チャンネル分を回す。
long peak_rss_kib() {
  struct rusage ru;
  if (::getrusage(RUSAGE_SELF, &ru) != 0) return 0;
#if defined(__APPLE__)
  return static_cast<long>(ru.ru_maxrss / 1024);  // macOS は byte
#else
  return static_cast<long>(ru.ru_maxrss);  // Linux は KiB
#endif
}

void test_frame_buffers_do_not_grow_per_event() {
  const std::string dir = scratch_dir("nogrow");
  const tpcgeo::Geometry geo = tpcgeo::load(kFixtureMiniReduced);

  // 実機 mini の 1 フレーム = 4 AGET × 68 ch × 512 bucket = 139,264 items。
  std::vector<Item> items;
  items.reserve(4u * 68u * 512u);
  for (uint32_t aget = 0; aget < 4; ++aget) {
    for (uint32_t chan = 0; chan < 68; ++chan) {
      for (uint32_t cell = 0; cell < 512; ++cell) {
        items.push_back(Item{aget, chan, cell, (aget * 137 + chan * 13 + cell) & 0xFFFu});
      }
    }
  }

  constexpr int kWarmupEvents = 40;   // ROOT のバスケット等が定常になるまで
  constexpr int kMeasuredEvents = 200;
  constexpr long kMaxGrowthKib = 32 * 1024;  // 32 MiB(欠陥時は 110 MiB 超)

  rootsink::RecorderConfig cfg;
  cfg.output_root = dir;
  cfg.geometry = &geo;
  cfg.fill = tiny_windows(/*remove_pedestal=*/true);
  rootsink::Recorder rec(cfg);

  uint32_t event_idx = 0;
  for (int i = 0; i < kWarmupEvents; ++i, ++event_idx) {
    rootsink::BuiltEvent ev;
    ev.run_number = 53;
    ev.event_idx = event_idx;
    ev.fragments.push_back(make_fragment(event_idx, 0, 0, 1000 + event_idx, items));
    rec.write(ev, 0);
  }
  const long warm_kib = peak_rss_kib();

  for (int i = 0; i < kMeasuredEvents; ++i, ++event_idx) {
    rootsink::BuiltEvent ev;
    ev.run_number = 53;
    ev.event_idx = event_idx;
    ev.fragments.push_back(make_fragment(event_idx, 0, 0, 1000 + event_idx, items));
    rec.write(ev, 0);
  }
  const long growth_kib = peak_rss_kib() - warm_kib;

  CHECK_EQ(rec.entries_written(), kWarmupEvents + kMeasuredEvents);
  CHECK(rec.fatal_reason() == nullptr);
  if (growth_kib > kMaxGrowthKib) {
    std::printf("  peak RSS grew %ld KiB over %d events (%.2f MB/event)\n", growth_kib,
                kMeasuredEvents,
                static_cast<double>(growth_kib) / 1024.0 / kMeasuredEvents);
  }
  CHECK(growth_kib <= kMaxGrowthKib);
  rec.close_run(53, 0);
}

// ---------------------------------------------------------------------------
// 10. 構造一致(env `TPCDAQ_REAL_PEVENT` = 実機 grawToEventTPC 出力)
// ---------------------------------------------------------------------------
void test_real_pevent_structure() {
  const char* path = std::getenv(kRealPEventEnv);
  if (path == nullptr || path[0] == '\0') {
    std::printf("SKIP: %s が未設定(実機 PEventTPC ファイルはリポに入れない)\n",
                kRealPEventEnv);
    return;
  }
  TFile* f = TFile::Open(path, "READ");
  CHECK(f != nullptr && !f->IsZombie());
  if (f == nullptr || f->IsZombie()) {
    delete f;
    return;
  }
  // streamer: 実機ファイル == 我々の辞書(コピー元スナップショットが同じことの証明)。
  const auto pe = streamer_of(f, "PEventTPC");
  const auto ei = streamer_of(f, "eventraw::EventInfo");
  const auto gp = streamer_of(f, "eventraw::EventInfo::global_properties");
  CHECK_EQ(pe.first, kStreamerVersion);
  CHECK_EQ(pe.second, kChecksumPEventTPC);
  CHECK_EQ(ei.first, kStreamerVersion);
  CHECK_EQ(ei.second, kChecksumEventInfo);
  CHECK_EQ(gp.first, kStreamerVersion);
  CHECK_EQ(gp.second, kChecksumGlobalProperties);
  CHECK_EQ(TClass::GetClass("PEventTPC")->GetCheckSum(), pe.second);
  CHECK_EQ(TClass::GetClass("eventraw::EventInfo")->GetCheckSum(), ei.second);
  CHECK_EQ(TClass::GetClass("eventraw::EventInfo::global_properties")->GetCheckSum(),
           gp.second);
  // 圧縮 level(実機は ZLIB-1。我々の既定 101 も level 1)。
  CHECK_EQ(f->GetCompressionLevel(), 1);

  // ツリー・ブランチ名。
  TTree* tree = dynamic_cast<TTree*>(f->Get("TPCData"));
  CHECK(tree != nullptr);
  if (tree != nullptr) {
    CHECK(tree->GetBranch("Event") != nullptr);
    CHECK(tree->GetBranch("myEventInfo.pedestalSubtracted") != nullptr);
    CHECK(tree->GetBranch("myChargeMap") != nullptr);
    // 実運用も myChargeArray を書いていない(= 我々の既定と同じ)。
    TBranch* arr = tree->GetBranch("myChargeArray[3][3][256][512]");
    CHECK(arr != nullptr);
    if (arr != nullptr) CHECK_EQ(arr->GetEntries(), 0);

    // 実ファイルの EventInfo から pedestalSubtracted を読む(実運用既定の裏取り)。
    PEventTPC* ev = nullptr;
    // chargeMap は 1 エントリ ≈ 17 MB(実機ファイルは 11 GB)—— EventInfo だけ読む。
    // **上位ブランチ "Event" は有効のまま**にしないと GetEntry がオブジェクトを
    // 作らない(ここを切ると ev が nullptr のままになる)。
    tree->SetBranchStatus("myChargeMap*", false);
    tree->SetBranchAddress("Event", &ev);
    CHECK(tree->GetEntries() > 0);
    if (tree->GetEntries() > 0) {
      tree->GetEntry(0);
      CHECK(ev != nullptr);
      if (ev != nullptr) {
        CHECK(ev->GetEventInfo().GetPedestalSubtracted());  // 実運用既定 = 減算 ON
        // runId = %Y%m%d%H%M%S(14 桁)。ファイル名 2026-08-11T07-47-37 と一致。
        CHECK_EQ(ev->GetEventInfo().GetRunId(), 20260811074737LL);
      }
    }
  }
  f->Close();
  delete f;
  std::printf("real PEventTPC structure check done: %s\n", path);
}

}  // namespace

int main() {
  gROOT->SetBatch(kTRUE);

  test_fill_without_pedestal();
  test_fill_with_pedestal();
  test_pedestal_is_reset_per_frame();
  test_default_signal_window_bounds();
  test_same_strip_accumulates();
  test_strip_number_beyond_array_is_dropped();
  test_frame_outside_geometry();
  test_zs_frame_with_table_subtracts_the_table_mean();
  test_zs_frame_without_table_keeps_the_old_algorithm_and_counts();
  test_full_frame_with_table_is_bit_identical_to_without();
  test_partial_non_zs_frame_is_not_zero_suppressed();
  test_zs_predicate_needs_a_strict_majority();
  test_zs_channel_missing_from_table_is_skipped_and_counted();
  test_zs_frame_without_pedestal_removal_stays_raw();
  test_run_id_from_tm();
  test_fill_config_validation();
  test_recorder_writes_tpcdata_tree();
  test_recorder_run_id_override();
  test_recorder_sums_zs_counters_over_workers();
  test_frame_buffers_do_not_grow_per_event();
  test_real_pevent_structure();

  return tpccheck::report("test_pevent");
}
