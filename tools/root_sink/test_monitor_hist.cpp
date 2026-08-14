// test_monitor_hist.cpp — monitor_hist.hpp(9 枚のヒスト集計)の単体テスト。TODO/022。
//
//   g++ -std=c++17 -O2 -Wall -Wextra test_monitor_hist.cpp -o test_monitor_hist
//   ./test_monitor_hist
//
// ROOT 不要・ZMQ 不要(monitor_hist.hpp は純ヘッダ)。試験方式は既存 5 本と同じ
// 「素の CHECK + main」(check.hpp)。
//
// **このファイルが SPEC §5.2(v1.9)の実行可能な仕様書**。合成ジオメトリはテスト内の
// 文字列で組み(実 .dat はリポに入れない —— CLAUDE.md)、期待ビン値はすべて手計算で、
// 出典をコメントに残す。

#include <cstdint>
#include <cstdio>
#include <string>
#include <vector>

#include "check.hpp"
#include "monitor_hist.hpp"

namespace {

// ---------------------------------------------------------------------------
// 合成ジオメトリ(NEW 10 欄: DIR SECTION STRIP COBO ASAD AGET AGET_CH の 7 列 + 3 実数)
// ---------------------------------------------------------------------------
//
// **非対称に作る**(取り違えが値で見えるように):
//   * 面ごとに最大ストリップが違う   → max_strip = U2 / V3 / W4
//   * 同じ strip 番号 2 が section 0 と 1 に居る(別 AGET)→ 合流ビンの検証
//   * AsAd を 2 枚使う(asad 0/1)   → イベント = 2 フラグメントの合算
//   * AUX を 1 本、FPN は .dat に書かれないがハードウェア的に raw {11,22,45,56}
//
// AGET_CH は**信号番号 0–63**(geo.hpp が raw へリオーダする)。0–10 は恒等写像なので
// ここでは raw_ch = AGET_CH として読める(kReorderFromGeometryToGraw[0..10] == 0..10)。
const char* kGeometryDat = R"(# synthetic geometry for test_monitor_hist (TODO/022)
ANGLES: 11.0 22.0 33.0
SAMPLING RATE: 12.5
#
# DIR SECTION STRIP COBO ASAD AGET AGET_CH OFF_PAD OFF_STRIP LEN_PADS
U	0	1	0	0	0	0	0.0	0.0	10
U	0	2	0	0	0	1	1.0	0.1	10
U	1	2	0	0	1	2	2.0	0.2	10
V	0	1	0	0	0	3	3.0	0.3	10
W	0	2	0	0	0	5	5.0	0.5	10
V	0	3	0	1	0	0	0.0	0.0	10
W	0	4	0	1	0	1	1.0	0.1	10
#
# AUX(5 欄): NAME COBO ASAD AGET AGET_CH
TestAux	0	0	0	6
)";

// SPEC §2.4 の item パック。[31:30] aget / [29:23] chan(raw)/ [22:14] bucket / [11:0] ADC。
uint32_t pack_item(uint32_t aget, uint32_t chan, uint32_t bucket, uint32_t adc) {
  return (aget << 30) | (chan << 23) | (bucket << 14) | adc;
}

void push_item(std::vector<uint8_t>& items, uint32_t word) {
  items.push_back(static_cast<uint8_t>(word & 0xFF));
  items.push_back(static_cast<uint8_t>((word >> 8) & 0xFF));
  items.push_back(static_cast<uint8_t>((word >> 16) & 0xFF));
  items.push_back(static_cast<uint8_t>((word >> 24) & 0xFF));
}

struct Sample {
  uint32_t aget, chan, bucket, adc;
};

rootsink::OwnedFragment make_fragment(uint8_t cobo, uint8_t asad,
                                      const std::vector<Sample>& samples) {
  rootsink::OwnedFragment f;
  f.cobo = cobo;
  f.asad = asad;
  f.items.reserve(samples.size() * 4);
  for (const Sample& s : samples) push_item(f.items, pack_item(s.aget, s.chan, s.bucket, s.adc));
  return f;
}

// --- ヒストの読み出し補助(添字は SPEC §5.3: 2D = (strip-1)*512 + bucket)---

double bin2d(const rsmon::HistAccumulator& h, size_t hist, uint32_t strip, uint32_t bucket) {
  return h.bins(hist)[(strip - 1) * rsmon::kBuckets + bucket];
}

double sum(const rsmon::HistAccumulator& h, size_t hist) {
  double total = 0.0;
  for (double v : h.bins(hist)) total += v;
  return total;
}

// ---------------------------------------------------------------------------
// イベント 1(2 フラグメント)—— 手計算の出典
// ---------------------------------------------------------------------------
//
// fragment (cobo0, asad0):
//   (aget0, raw0,  bucket3,  adc 100)  U sec0 strip1
//   (aget0, raw0,  bucket4,  adc 250)  U sec0 strip1   ← 同一 ch の 2 サンプル目
//   (aget0, raw1,  bucket0,  adc 40)   U sec0 strip2
//   (aget1, raw2,  bucket7,  adc 4095) U sec1 strip2   ← 飽和 + 合流ストリップ
//   (aget0, raw3,  bucket1,  adc 8)    V sec0 strip1
//   (aget0, raw5,  bucket2,  adc 17)   W sec0 strip2
//   (aget0, raw6,  bucket2,  adc 900)  AUX     → ヒストに入れない
//   (aget0, raw11, bucket2,  adc 800)  FPN     → ヒストに入れない
//   (aget0, raw60, bucket2,  adc 700)  未記載  → ヒストに入れない
//   (aget2, raw0,  bucket5,  adc 600)  未記載の AGET → ヒストに入れない
// fragment (cobo0, asad1):
//   (aget0, raw0,  bucket9,  adc 30)   V sec0 strip3
//   (aget0, raw1,  bucket9,  adc 4095) W sec0 strip4   ← 飽和
//   (aget0, raw1,  bucket10, adc 4000) W sec0 strip4   ← 波高は 4095 のまま
//
// StripTimeU: [strip1,b3]=100 [strip1,b4]=250 [strip2,b0]=40 [strip2,b7]=4095 → 総和 4485
// StripTimeV: [strip1,b1]=8   [strip3,b9]=30                                  → 総和 38
// StripTimeW: [strip2,b2]=17  [strip4,b9]=4095 [strip4,b10]=4000              → 総和 8112
//
// 波高(チャンネル毎の生 ADC 最大。サンプル ≥1 のチャンネルのみ):
//   U: (aget0,raw0)=250 → bin 250/8=31 / (aget0,raw1)=40 → bin 5 / (aget1,raw2)=4095 → bin 511
//   V: (aget0,raw3)=8 → bin 1 / asad1 (aget0,raw0)=30 → bin 3
//   W: (aget0,raw5)=17 → bin 2 / asad1 (aget0,raw1)=4095 → bin 511
//   counted = U3 / V2 / W2、saturated = U1 / V0 / W1
// ChargeMax(イベント毎・面内の最大 1 エントリ): U 4095→511 / V 30→3 / W 4095→511
rootsink::BuiltEvent event_one() {
  rootsink::BuiltEvent ev;
  ev.run_number = 7;
  ev.event_idx = 1;
  ev.fragments.push_back(make_fragment(0, 0,
                                       {{0, 0, 3, 100},
                                        {0, 0, 4, 250},
                                        {0, 1, 0, 40},
                                        {1, 2, 7, 4095},
                                        {0, 3, 1, 8},
                                        {0, 5, 2, 17},
                                        {0, 6, 2, 900},
                                        {0, 11, 2, 800},
                                        {0, 60, 2, 700},
                                        {2, 0, 5, 600}}));
  ev.fragments.push_back(
      make_fragment(0, 1, {{0, 0, 9, 30}, {0, 1, 9, 4095}, {0, 1, 10, 4000}}));
  return ev;
}

// イベント 2(1 フラグメント)—— 合流ビンと「サンプル 0 チャンネルは数えない」の検証用。
//   (aget0, raw0, bucket3, adc 50)  U sec0 strip1 → 波高 50 → bin 50/8 = 6
//   (aget1, raw2, bucket3, adc 52)  U sec1 strip2 → 波高 52 → bin 52/8 = 6  ← **同じビンへ別エントリ**
// V/W にはサンプルが 1 個も無いので counted も ChargeMax も増えない。
rootsink::BuiltEvent event_two() {
  rootsink::BuiltEvent ev;
  ev.run_number = 7;
  ev.event_idx = 2;
  ev.fragments.push_back(make_fragment(0, 0, {{0, 0, 3, 50}, {1, 2, 3, 52}}));
  return ev;
}

// ---------------------------------------------------------------------------
// 1. メタデータ(ビン数はジオメトリから — 焼き込み禁止、SPEC §5.2)
// ---------------------------------------------------------------------------

void test_hist_metadata_comes_from_the_geometry() {
  tpcgeo::Geometry geo = tpcgeo::parse(kGeometryDat);
  CHECK_EQ(geo.max_strip[0], 2);  // U
  CHECK_EQ(geo.max_strip[1], 3);  // V
  CHECK_EQ(geo.max_strip[2], 4);  // W

  rsmon::HistAccumulator h(geo);
  CHECK_EQ(rsmon::kHistCount, 9);

  // id は 1..9、名前は SPEC §5.2 の表そのもの。
  const char* kNames[9] = {"StripTimeU", "StripTimeV", "StripTimeW",
                           "ChargeU",    "ChargeV",    "ChargeW",
                           "ChargeMaxU", "ChargeMaxV", "ChargeMaxW"};
  for (size_t i = 0; i < rsmon::kHistCount; ++i) {
    CHECK_EQ(h.meta(i).id, static_cast<int>(i) + 1);
    CHECK(std::string(h.meta(i).name) == kNames[i]);
    CHECK_EQ(h.bins(i).size(), h.meta(i).nx * h.meta(i).ny);
  }
  // 2D = Nstrip × 512
  CHECK_EQ(h.meta(0).nx, 2);
  CHECK_EQ(h.meta(0).ny, 512);
  CHECK_EQ(h.meta(1).nx, 3);
  CHECK_EQ(h.meta(2).nx, 4);
  CHECK_EQ(h.bins(2).size(), 4u * 512u);
  // 1D = 512 ビン(ny = 1)
  for (size_t i = 3; i < 9; ++i) {
    CHECK_EQ(h.meta(i).nx, 512);
    CHECK_EQ(h.meta(i).ny, 1);
  }
  // 作った直後は全ゼロ
  for (size_t i = 0; i < rsmon::kHistCount; ++i) CHECK_EQ(sum(h, i), 0);
}

// ---------------------------------------------------------------------------
// 2. StripTime — 添字((strip-1)*512 + bucket)と weight=ADC の加算
// ---------------------------------------------------------------------------

void test_strip_time_indices_and_adc_weights() {
  tpcgeo::Geometry geo = tpcgeo::parse(kGeometryDat);
  rsmon::HistAccumulator h(geo);
  h.on_event(event_one());

  // U(上の出典表のまま)
  CHECK_EQ(bin2d(h, 0, 1, 3), 100);
  CHECK_EQ(bin2d(h, 0, 1, 4), 250);
  CHECK_EQ(bin2d(h, 0, 2, 0), 40);
  CHECK_EQ(bin2d(h, 0, 2, 7), 4095);
  CHECK_EQ(sum(h, 0), 4485);  // 100+250+40+4095
  // V
  CHECK_EQ(bin2d(h, 1, 1, 1), 8);
  CHECK_EQ(bin2d(h, 1, 3, 9), 30);
  CHECK_EQ(sum(h, 1), 38);
  // W
  CHECK_EQ(bin2d(h, 2, 2, 2), 17);
  CHECK_EQ(bin2d(h, 2, 4, 9), 4095);
  CHECK_EQ(bin2d(h, 2, 4, 10), 4000);
  CHECK_EQ(sum(h, 2), 8112);  // 17+4095+4000

  // 2 回目のイベントは**加算**される(run 積算 = R3)。
  h.on_event(event_two());
  CHECK_EQ(bin2d(h, 0, 1, 3), 150);  // 100 + 50
  CHECK_EQ(bin2d(h, 0, 2, 3), 52);   // 合流ストリップの sec1 側
  CHECK_EQ(sum(h, 0), 4587);         // 4485 + 50 + 52
  CHECK_EQ(sum(h, 1), 38);           // V は増えない
  CHECK_EQ(sum(h, 2), 8112);         // W も増えない
}

// ---------------------------------------------------------------------------
// 3. FPN / AUX / 未記載チャンネルはヒストに入らない(SPEC §5.2)
// ---------------------------------------------------------------------------
//
// event_one には AUX(900)・FPN(800)・未記載 raw60(700)・未記載 aget2(600)を
// 混ぜてある。どれか 1 つでも入れば総和が合わなくなる(上の 4485/38/8112 が動く)。
void test_fpn_aux_and_unmapped_channels_are_excluded() {
  tpcgeo::Geometry geo = tpcgeo::parse(kGeometryDat);
  rsmon::HistAccumulator h(geo);
  h.on_event(event_one());
  CHECK_EQ(sum(h, 0) + sum(h, 1) + sum(h, 2), 4485 + 38 + 8112);
  // Charge のエントリ総数 = 計数したストリップチャンネル数(= 3 + 2 + 2 = 7)。
  // AUX/FPN/未記載が混ざれば増える。
  CHECK_EQ(sum(h, 3) + sum(h, 4) + sum(h, 5), 7);
  CHECK_EQ(h.saturation(tpcgeo::Plane::U).counted, 3);
  CHECK_EQ(h.saturation(tpcgeo::Plane::V).counted, 2);
  CHECK_EQ(h.saturation(tpcgeo::Plane::W).counted, 2);
}

// ---------------------------------------------------------------------------
// 4. Charge — ビン = 波高/8、1 チャンネル 1 エントリ、合流ビンには別エントリ
// ---------------------------------------------------------------------------

void test_charge_bin_is_peak_over_eight_one_entry_per_channel() {
  tpcgeo::Geometry geo = tpcgeo::parse(kGeometryDat);
  rsmon::HistAccumulator h(geo);
  h.on_event(event_one());

  // U: 250→31、40→5、4095→511(3 チャンネル = 3 エントリ)
  CHECK_EQ(h.bins(3)[31], 1);
  CHECK_EQ(h.bins(3)[5], 1);
  CHECK_EQ(h.bins(3)[511], 1);
  CHECK_EQ(sum(h, 3), 3);
  // V: 8→1、30→3
  CHECK_EQ(h.bins(4)[1], 1);
  CHECK_EQ(h.bins(4)[3], 1);
  CHECK_EQ(sum(h, 4), 2);
  // W: 17→2、4095→511
  CHECK_EQ(h.bins(5)[2], 1);
  CHECK_EQ(h.bins(5)[511], 1);
  CHECK_EQ(sum(h, 5), 2);

  // イベント 2: 波高 50(U sec0 strip1)と 52(U sec1 strip2)は **どちらも bin 6**。
  // 「同一ストリップ番号の複数セクション = 同一ビンへ**別エントリ**」(SPEC §5.2 v1.9)。
  h.on_event(event_two());
  CHECK_EQ(h.bins(3)[6], 2);
  CHECK_EQ(sum(h, 3), 5);
}

// ---------------------------------------------------------------------------
// 5. ChargeMax — イベント毎・面内の最大 1 エントリ。空の面には入れない
// ---------------------------------------------------------------------------

void test_charge_max_is_one_entry_per_plane_per_event() {
  tpcgeo::Geometry geo = tpcgeo::parse(kGeometryDat);
  rsmon::HistAccumulator h(geo);
  h.on_event(event_one());
  CHECK_EQ(h.bins(6)[511], 1);  // U 最大 4095
  CHECK_EQ(sum(h, 6), 1);
  CHECK_EQ(h.bins(7)[3], 1);  // V 最大 30
  CHECK_EQ(sum(h, 7), 1);
  CHECK_EQ(h.bins(8)[511], 1);  // W 最大 4095
  CHECK_EQ(sum(h, 8), 1);

  // イベント 2 は U だけ(最大 52 → bin 6)。V/W は計数チャンネル 0 本なので**入れない**。
  h.on_event(event_two());
  CHECK_EQ(h.bins(6)[6], 1);
  CHECK_EQ(sum(h, 6), 2);
  CHECK_EQ(sum(h, 7), 1);  // 増えない
  CHECK_EQ(sum(h, 8), 1);  // 増えない
}

// ---------------------------------------------------------------------------
// 6. サンプル 0 のチャンネルは計数しない(飽和率の分母にも入らない)
// ---------------------------------------------------------------------------

void test_channels_without_samples_are_not_counted() {
  tpcgeo::Geometry geo = tpcgeo::parse(kGeometryDat);
  rsmon::HistAccumulator h(geo);
  h.on_event(event_one());
  h.on_event(event_two());
  // event_two で動いたのは U の 2 チャンネルだけ。V/W の分母は event_one のまま。
  CHECK_EQ(h.saturation(tpcgeo::Plane::U).counted, 5);  // 3 + 2
  CHECK_EQ(h.saturation(tpcgeo::Plane::V).counted, 2);
  CHECK_EQ(h.saturation(tpcgeo::Plane::W).counted, 2);
}

// ---------------------------------------------------------------------------
// 7. 飽和(波高 ≥ 4095)の分子
// ---------------------------------------------------------------------------

void test_saturated_channels_are_counted_at_4095() {
  tpcgeo::Geometry geo = tpcgeo::parse(kGeometryDat);
  rsmon::HistAccumulator h(geo);
  h.on_event(event_one());
  CHECK_EQ(h.saturation(tpcgeo::Plane::U).saturated, 1);  // (aget1,raw2) = 4095
  CHECK_EQ(h.saturation(tpcgeo::Plane::V).saturated, 0);
  CHECK_EQ(h.saturation(tpcgeo::Plane::W).saturated, 1);  // asad1 (aget0,raw1) = 4095

  // 4094 は飽和ではない(境界は「≥ 4095」)。
  rootsink::BuiltEvent ev;
  ev.fragments.push_back(make_fragment(0, 0, {{0, 3, 0, 4094}}));
  h.on_event(ev);
  CHECK_EQ(h.saturation(tpcgeo::Plane::V).saturated, 0);
  CHECK_EQ(h.saturation(tpcgeo::Plane::V).counted, 3);
}

// ---------------------------------------------------------------------------
// 8. incomplete イベントも届いた分で fill する(SPEC §5.2 v1.9「捨てない」)
// ---------------------------------------------------------------------------

void test_incomplete_events_are_filled_with_what_arrived() {
  tpcgeo::Geometry geo = tpcgeo::parse(kGeometryDat);
  rsmon::HistAccumulator complete(geo);
  rsmon::HistAccumulator partial(geo);

  rootsink::BuiltEvent ev = event_one();
  complete.on_event(ev);

  // asad1 のフラグメントが来なかった = incomplete。届いた asad0 の分は入る。
  rootsink::BuiltEvent half;
  half.run_number = ev.run_number;
  half.event_idx = ev.event_idx;
  half.incomplete = true;
  half.fragments.push_back(ev.fragments[0]);
  partial.on_event(half);

  CHECK_EQ(sum(partial, 0), 4485);  // U は asad0 だけで完結している
  CHECK_EQ(sum(partial, 1), 8);     // V は strip1 の 8 だけ(strip3 の 30 は来ていない)
  CHECK_EQ(sum(partial, 2), 17);    // W も strip2 の 17 だけ
  CHECK_EQ(partial.saturation(tpcgeo::Plane::W).counted, 1);
  CHECK_EQ(partial.saturation(tpcgeo::Plane::W).saturated, 0);
  // U 面は asad0 だけで完結しているので complete と同じ中身になる
  // (incomplete フラグ自体は fill 規則を変えない)。V/W は届いた 1 本ずつ。
  CHECK(partial.bins(0) == complete.bins(0));
  CHECK_EQ(sum(partial, 3), 3);  // ChargeU: 250/40/4095 の 3 エントリ
  CHECK_EQ(sum(partial, 4), 1);  // ChargeV: 8 の 1 エントリ
  CHECK_EQ(sum(partial, 5), 1);  // ChargeW: 17 の 1 エントリ
}

// ---------------------------------------------------------------------------
// 9. reset() で全ゼロ(run open で呼ぶ)
// ---------------------------------------------------------------------------

void test_reset_clears_every_bin_and_counter() {
  tpcgeo::Geometry geo = tpcgeo::parse(kGeometryDat);
  rsmon::HistAccumulator h(geo);
  h.on_event(event_one());
  h.on_event(event_two());
  CHECK(sum(h, 0) > 0);

  h.reset();
  for (size_t i = 0; i < rsmon::kHistCount; ++i) {
    CHECK_EQ(sum(h, i), 0);
    // ビン数は変わらない(ジオメトリは同じ)
    CHECK_EQ(h.bins(i).size(), h.meta(i).nx * h.meta(i).ny);
  }
  for (int p = 0; p < 3; ++p) {
    CHECK_EQ(h.saturation(static_cast<tpcgeo::Plane>(p)).counted, 0);
    CHECK_EQ(h.saturation(static_cast<tpcgeo::Plane>(p)).saturated, 0);
  }
  // reset 後も同じように積める
  h.on_event(event_one());
  CHECK_EQ(sum(h, 0), 4485);
}

// ---------------------------------------------------------------------------
// 10. スナップショットのコピー(Publisher / RunClose が使う受け皿)
// ---------------------------------------------------------------------------

void test_snapshot_copy_matches_the_live_bins() {
  tpcgeo::Geometry geo = tpcgeo::parse(kGeometryDat);
  rsmon::HistAccumulator h(geo);
  h.on_event(event_one());

  rsmon::HistSnapshot snap;
  h.prepare(snap);
  h.copy_into(snap);
  for (size_t i = 0; i < rsmon::kHistCount; ++i) {
    CHECK_EQ(snap[i].id, h.meta(i).id);
    CHECK(std::string(snap[i].name) == h.meta(i).name);
    CHECK_EQ(snap[i].nx, h.meta(i).nx);
    CHECK_EQ(snap[i].ny, h.meta(i).ny);
    CHECK_EQ(snap[i].bins.size(), h.bins(i).size());
    CHECK(snap[i].bins == h.bins(i));
  }
  // 以降の fill はコピーに触らない(二重バッファ)
  h.on_event(event_two());
  CHECK_EQ(snap[0].bins[(1 - 1) * rsmon::kBuckets + 3], 100);
  CHECK_EQ(bin2d(h, 0, 1, 3), 150);
}

// ---------------------------------------------------------------------------
// 11. 範囲外の item は黙って捨てない(chan は 7bit = 127 まで表現できる)
// ---------------------------------------------------------------------------

void test_out_of_range_channel_is_counted() {
  tpcgeo::Geometry geo = tpcgeo::parse(kGeometryDat);
  rsmon::HistAccumulator h(geo);
  rootsink::BuiltEvent ev;
  ev.fragments.push_back(make_fragment(0, 0, {{0, 100, 3, 77}, {0, 0, 3, 12}}));
  h.on_event(ev);
  CHECK_EQ(h.items_out_of_range(), 1);
  CHECK_EQ(sum(h, 0), 12);  // 正常な 1 個だけが入る
}

}  // namespace

int main() {
  test_hist_metadata_comes_from_the_geometry();
  test_strip_time_indices_and_adc_weights();
  test_fpn_aux_and_unmapped_channels_are_excluded();
  test_charge_bin_is_peak_over_eight_one_entry_per_channel();
  test_charge_max_is_one_entry_per_plane_per_event();
  test_channels_without_samples_are_not_counted();
  test_saturated_channels_are_counted_at_4095();
  test_incomplete_events_are_filled_with_what_arrived();
  test_reset_clears_every_bin_and_counter();
  test_snapshot_copy_matches_the_live_bins();
  test_out_of_range_channel_is_counted();
  return tpccheck::report("test_monitor_hist");
}
