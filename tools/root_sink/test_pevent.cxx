// test_pevent.cxx — PEventTPC 充填(pevent_fill.hpp)と PEventTPC 出力モードの
//                   Recorder(root_recorder.hpp)の単体テスト(TODO/020)。
//
// **`make test` からは外してある**(`make test-root`)—— ROOT + TPCReco クラスを
// リンクする(SPEC §6.4 v1.8 のビルド時参照)。
//
// 試験方式は既存の流儀(素の CHECK + main、check.hpp)。やっていること:
//
//   1. 合成 GDataFrame → PEventTPC の chargeMap を**手計算オラクル**と全 key 照合
//      (strip 射影 / FPN リオーダ / signal 窓 / ペデスタル数値 / `+=` 加算)。
//   2. Recorder で書いた .root を**同プロセスで開き直して**、TPCReco `EventSourceROOT`
//      が期待する形(ツリー `TPCData` / ブランチ `Event` / myChargeArray 無効)と
//      streamer(version + checksum)を機械照合する。
//   3. env `TPCDAQ_REAL_PEVENT` があれば**実機 grawToEventTPC 出力**の streamer /
//      ツリー / ブランチ / pedestalSubtracted と突き合わせる(未設定なら SKIP を印字)。
//
// **GET クラスの地雷**(third_party/get は無改変): `GDataFrame` の TClonesArray は
// static 共有(fgChannels/fgSamples)で `~GDataFrame()` がそれを **delete する**。
// 同時に 2 個生かしてはいけない —— 充填テストの GDataFrame を**畳んでから** Recorder
// (内部で GDataFrame を 1 個持つ)のテストに入る。main の scope 分けはそのため。

#include <sys/stat.h>

#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <ctime>
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

#include "GDataFrame.h"
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
// パック形式(SPEC §2.4 と root_recorder.hpp::fill が読む形)。
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

// 上の items を GDataFrame に詰める(Recorder が OwnedFragment からやることと同じ形)。
void load_frame(GET::GDataFrame& frame, uint8_t cobo, uint8_t asad, uint32_t event_idx,
                uint64_t event_time, const std::vector<Item>& items) {
  frame.Clear();
  frame.fHeader.fCoboIdx = cobo;
  frame.fHeader.fAsadIdx = asad;
  frame.fHeader.fEventIdx = event_idx;
  frame.fHeader.fEventTime = event_time;
  for (const Item& it : items) {
    frame.AddSample(static_cast<UShort_t>(it.aget), static_cast<UShort_t>(it.chan),
                    static_cast<UShort_t>(it.bucket), static_cast<UShort_t>(it.adc));
  }
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
void test_fill_without_pedestal(GET::GDataFrame& frame) {
  const tpcgeo::Geometry geo = tpcgeo::load(kFixtureMiniReduced);
  tpcpevent::Filler filler(geo, tiny_windows(/*remove_pedestal=*/false));
  load_frame(frame, 0, 0, 42, 123456789, synthetic_items());

  PEventTPC ev;
  ev.Clear();
  filler.add_frame(frame, ev);

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
void test_fill_with_pedestal(GET::GDataFrame& frame) {
  const tpcgeo::Geometry geo = tpcgeo::load(kFixtureMiniReduced);
  tpcpevent::Filler filler(geo, tiny_windows(/*remove_pedestal=*/true));
  load_frame(frame, 0, 0, 42, 123456789, synthetic_items());

  PEventTPC ev;
  ev.Clear();
  filler.add_frame(frame, ev);

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
void test_pedestal_is_reset_per_frame(GET::GDataFrame& frame) {
  const tpcgeo::Geometry geo = tpcgeo::load(kFixtureMiniReduced);
  tpcpevent::Filler filler(geo, tiny_windows(/*remove_pedestal=*/true));

  PEventTPC ev;
  ev.Clear();
  load_frame(frame, 0, 0, 42, 1, synthetic_items());
  filler.add_frame(frame, ev);
  load_frame(frame, 0, 0, 42, 1, synthetic_items());
  filler.add_frame(frame, ev);

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
void test_default_signal_window_bounds(GET::GDataFrame& frame) {
  const tpcgeo::Geometry geo = tpcgeo::load(kFixtureMiniReduced);
  tpcpevent::FillConfig cfg;  // 既定 = 5 / 25 / 5 / 506
  cfg.remove_pedestal = false;
  CHECK_EQ(cfg.min_pedestal_cell, 5);
  CHECK_EQ(cfg.max_pedestal_cell, 25);
  CHECK_EQ(cfg.min_signal_cell, 5);
  CHECK_EQ(cfg.max_signal_cell, 506);
  tpcpevent::Filler filler(geo, cfg);

  // AGET0 raw 0 = U strip 1 に 4 点だけ置く(非対称値)。
  load_frame(frame, 0, 0, 1, 1,
             {{0, 0, 4, 111}, {0, 0, 5, 222}, {0, 0, 506, 333}, {0, 0, 507, 444}});
  PEventTPC ev;
  ev.Clear();
  filler.add_frame(frame, ev);

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
void test_same_strip_accumulates(GET::GDataFrame& frame) {
  const char* dat =
      "U\t0\t1\t0\t0\t0\t0\t0.0\t0.0\t10\n"
      "U\t0\t1\t0\t0\t1\t0\t0.0\t0.0\t10\n";
  const tpcgeo::Geometry geo = tpcgeo::parse(dat);
  tpcpevent::Filler filler(geo, tiny_windows(/*remove_pedestal=*/false));

  // 同じ cell 3 に別々の値を置く: 40 + 2 = 42。
  load_frame(frame, 0, 0, 1, 1, {{0, 0, 3, 40}, {1, 0, 3, 2}});
  PEventTPC ev;
  ev.Clear();
  filler.add_frame(frame, ev);

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
void test_strip_number_beyond_array_is_dropped(GET::GDataFrame& frame) {
  const char* dat =
      "U\t0\t255\t0\t0\t0\t0\t0.0\t0.0\t10\n"   // 収まる(境界の内側)
      "U\t0\t256\t0\t0\t0\t1\t0.0\t0.0\t10\n";  // 収まらない(境界のすぐ外)
  const tpcgeo::Geometry geo = tpcgeo::parse(dat);
  tpcpevent::Filler filler(geo, tiny_windows(/*remove_pedestal=*/false));

  load_frame(frame, 0, 0, 1, 1, {{0, 0, 3, 11}, {0, 1, 3, 22}, {0, 1, 4, 33}});
  PEventTPC ev;
  ev.Clear();
  filler.add_frame(frame, ev);

  CHECK_EQ(ev.GetChargeMap().size(), 1);
  CHECK_D(charge_at(ev, 0, 0, 255, 3), 11.0);
  CHECK(!has_key(ev, 0, 0, 256, 3));
  CHECK_EQ(filler.keys_out_of_range(), 2);  // strip 256 の 2 サンプル
}

// ---------------------------------------------------------------------------
// 7. ジオメトリに無い (cobo,asad) のフレームは丸ごと捨てて数える
// ---------------------------------------------------------------------------
void test_frame_outside_geometry(GET::GDataFrame& frame) {
  const tpcgeo::Geometry geo = tpcgeo::load(kFixtureMiniReduced);  // cobo 0 / asad 0 のみ
  tpcpevent::Filler filler(geo, tiny_windows(/*remove_pedestal=*/false));

  load_frame(frame, 0, 3, 1, 1, {{0, 0, 3, 40}});  // asad=3 は .dat に無い
  PEventTPC ev;
  ev.Clear();
  filler.add_frame(frame, ev);

  CHECK_EQ(ev.GetChargeMap().size(), 0);
  CHECK_EQ(filler.frames_outside_geometry(), 1);
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
    cfg.format = rootsink::OutputFormat::PEvent;
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
  }  // ここで Recorder(と内部の GDataFrame)を畳んでから読み戻す

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

// gdataframe モード(テスト専用の旧出力)が残っていること —— §12-3 の旧オラクル回帰は
// これに乗っている。中身の全値照合は test_recorder.cxx の担当なので、ここでは
// **ツリー名が切り替わる**ことだけ見る。
void test_recorder_gdataframe_mode_still_writes_old_tree() {
  const std::string dir = scratch_dir("gdf");
  {
    rootsink::RecorderConfig cfg;
    cfg.output_root = dir;
    cfg.format = rootsink::OutputFormat::GDataFrame;
    rootsink::Recorder rec(cfg);
    rootsink::BuiltEvent ev;
    ev.run_number = 8;
    ev.event_idx = 1;
    ev.fragments.push_back(make_fragment(1, 0, 0, 5, {{0, 0, 3, 40}, {0, 1, 3, 2}}));
    rec.write(ev, 0);
    CHECK_EQ(rec.entries_written(), 1);
    rec.close_run(8, 0);
  }
  const std::string path = dir + "/run0008/run0008.root";
  CHECK(rootsink::path_exists(path));
  TFile* f = TFile::Open(path.c_str(), "READ");
  CHECK(f != nullptr && !f->IsZombie());
  if (f != nullptr && !f->IsZombie()) {
    CHECK(f->Get("tree") != nullptr);
    CHECK(f->Get("TPCData") == nullptr);
    f->Close();
  }
  delete f;
  remove_tree(dir);
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

  // **GDataFrame は同時に 1 個だけ**(ヘッダ冒頭の地雷)。充填テストはこの scope で
  // 済ませ、Recorder(内部で 1 個持つ)のテストはこの後に回す。
  {
    GET::GDataFrame frame;
    test_fill_without_pedestal(frame);
    test_fill_with_pedestal(frame);
    test_pedestal_is_reset_per_frame(frame);
    test_default_signal_window_bounds(frame);
    test_same_strip_accumulates(frame);
    test_strip_number_beyond_array_is_dropped(frame);
    test_frame_outside_geometry(frame);
  }
  test_run_id_from_tm();
  test_fill_config_validation();
  test_recorder_writes_tpcdata_tree();
  test_recorder_gdataframe_mode_still_writes_old_tree();
  test_real_pevent_structure();

  return tpccheck::report("test_pevent");
}
