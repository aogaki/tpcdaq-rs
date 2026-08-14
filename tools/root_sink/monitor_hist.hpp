// monitor_hist.hpp — モニタ用 9 枚のヒスト集計(SPEC §5.2 v1.9)。TODO/022。
//
// **純ヘッダ。ROOT 非依存・ZMQ 非依存**(依存は geo.hpp と eb_core.hpp だけ)。
// ROOT 化(TH1D/TH2D)は Recorder スレッドの仕事(root_recorder.hpp)、PUB への
// 直列化は monitor_pub.hpp の仕事 —— ここは「配列に足す」だけ。
//
// 集計の持ち主が root-sink である理由は SPEC §5.1(ロスレス系で全イベントが通る唯一の
// 場所なので、`run<N>_monitor.root` が「見えた分だけの部分集計」にならない)。
//
// --- 9 枚(SPEC §5.2)-------------------------------------------------------
//   id 1–3 StripTime{U,V,W}  TH2D  Nstrip × 512   x=strip(1..N) y=bucket
//                            全サンプルを weight=生 ADC で加算(R3)
//   id 4–6 Charge{U,V,W}     TH1D  512 [0,4096)   x=波高
//                            **チャンネル(= 物理ストリップ)毎に 1 エントリ**(R4①)
//   id 7–9 ChargeMax{U,V,W}  TH1D  512 [0,4096)   イベント毎・面内の最大 1 エントリ(R4②)
//
// **ビン数を焼き込まない**(CLAUDE.md): `nx = geometry.max_strip[plane]`。
// mini(U72/V92/W92)と ELITPC(U132/V225/W226)の差はここだけで吸収される。
//
// --- 集計単位の要点(v1.9 明確化)-------------------------------------------
//   * 「波高」= そのイベントでチャンネルが持つ**生 ADC の最大値**(減算なし)。
//   * 波高を数えるのは **ジオメトリで Strip に割り付いたチャンネル**だけ
//     (FPN・AUX・未記載は除外)、かつ **そのイベントでサンプルが 1 個以上あったもの**
//     だけ。部分読み出しでサンプル 0 のチャンネルは飽和率の分母にも入れない。
//   * 同一ストリップ番号の複数セクションは StripTime では同じ行に合算され、
//     Charge では**同じビンへ別エントリ**として入る(1 チャンネル 1 エントリなので)。
//   * incomplete イベントも届いた分で fill する(捨てない)。emit 済みイベントへの
//     遅延フラグメントでは `on_event` を**呼ばない**(呼び元の責務 —— SPEC §5.2)。
//
// --- ホットパスの約束(CLAUDE.md)------------------------------------------
//   * バッファは ctor / reset() で確保する。`on_event` は heap を触らない。
//   * チャンネル役割はフラグメント毎に 1 回だけ引く(272 回)。item 毎に
//     `Geometry::lookup` を呼ぶと同じ答えを 10 万回引き直すことになる ——
//     pevent_fill.hpp が `build_channel_index` で採ったのと同じ形(答えは同一)。
//   * ログはホットパスに置かない。範囲外 item はカウンタで可視化する。

#ifndef TPCDAQ_ROOT_SINK_MONITOR_HIST_HPP
#define TPCDAQ_ROOT_SINK_MONITOR_HIST_HPP

#include <array>
#include <cstdint>
#include <cstring>
#include <vector>

#include "eb_core.hpp"
#include "geo.hpp"

namespace rsmon {

// ---------------------------------------------------------------------------
// 定数(SPEC §5.2 / §5.3)
// ---------------------------------------------------------------------------

constexpr size_t kHistCount = 9;
constexpr size_t kPlaneCount = 3;
// AGET の SCA 深さ = 2D の y 軸ビン数(bucket は 9bit なので構造上 0..511)。
constexpr uint32_t kBuckets = 512;
// Charge / ChargeMax の 1D ビン数と幅。512 × 8 = [0, 4096) —— ADC は 12bit なので
// clamp は要らない(波高 ≤ 4095 → ビン ≤ 511)。x レンジ固定 = 飽和天井が常に見える。
constexpr uint32_t kChargeBins = 512;
constexpr uint32_t kChargeBinWidth = 8;
// 飽和判定(SPEC §5.2「波高 ≥ 4095」)。
constexpr uint16_t kSaturationAdc = 4095;

// AGET の物理構造(geo.hpp の値を再定義せず参照する)。
constexpr uint32_t kAgetChips = tpcgeo::kAgetChipsPerAsad;  // 4
constexpr uint32_t kRawChPerAget = tpcgeo::kRawChPerAget;   // 68

// ---------------------------------------------------------------------------
// メタと受け皿
// ---------------------------------------------------------------------------

// 1 枚のヒストの素性。`ny == 1` なら 1D(SPEC §5.3 のワイヤ形式と同じ規約)。
struct HistMeta {
  uint8_t id = 0;              // 1..9
  const char* name = "";       // SPEC §5.2 の名前(静的文字列)
  uint32_t nx = 0;
  uint32_t ny = 0;
};

// スナップショットの受け皿 1 枚分。PUB の直列化と `run{run:04}_monitor.root` の
// 書き出しが同じものを使う(集計は 1 つ = SPEC §5.1「一致性」)。
struct HistBuffer {
  uint8_t id = 0;
  const char* name = "";
  uint32_t nx = 0;
  uint32_t ny = 0;
  std::vector<double> bins;
};

using HistSnapshot = std::array<HistBuffer, kHistCount>;

// 面毎の飽和率の材料(run 積算)。UI では saturated / counted を % 表示する。
struct Saturation {
  uint64_t saturated = 0;
  uint64_t counted = 0;
};

// 9 枚の並び(id = index + 1)。この順序が SPEC §5.2 の表の順序。
inline size_t strip_time_index(tpcgeo::Plane p) { return static_cast<size_t>(p); }
inline size_t charge_index(tpcgeo::Plane p) { return 3 + static_cast<size_t>(p); }
inline size_t charge_max_index(tpcgeo::Plane p) { return 6 + static_cast<size_t>(p); }

// ---------------------------------------------------------------------------
// HistAccumulator
// ---------------------------------------------------------------------------
//
// スレッド規約(SPEC §5.1): このオブジェクトを触ってよいのは **集計スレッド
// (on_event / reset)と Publisher(copy_into)だけ**で、両者は `hist_mutex` で
// 排他する。Recorder(TTree)スレッドとはロックを共有しない —— 保存への非干渉。
class HistAccumulator {
 public:
  explicit HistAccumulator(const tpcgeo::Geometry& geometry) : geo_(geometry) {
    for (size_t p = 0; p < kPlaneCount; ++p) {
      const tpcgeo::Plane plane = static_cast<tpcgeo::Plane>(p);
      const uint32_t nstrip = geometry.max_strip[p];
      meta_[strip_time_index(plane)] =
          HistMeta{static_cast<uint8_t>(strip_time_index(plane) + 1), kStripTimeNames[p],
                   nstrip, kBuckets};
      meta_[charge_index(plane)] = HistMeta{static_cast<uint8_t>(charge_index(plane) + 1),
                                            kChargeNames[p], kChargeBins, 1};
      meta_[charge_max_index(plane)] =
          HistMeta{static_cast<uint8_t>(charge_max_index(plane) + 1), kChargeMaxNames[p],
                   kChargeBins, 1};
    }
    for (size_t i = 0; i < kHistCount; ++i) {
      bins_[i].assign(static_cast<size_t>(meta_[i].nx) * meta_[i].ny, 0.0);
    }
  }

  HistAccumulator(const HistAccumulator&) = delete;
  HistAccumulator& operator=(const HistAccumulator&) = delete;

  // run open で呼ぶ(SPEC §5.2「すべて run start でリセット」)。確保済みの
  // バッファは手放さない —— run 境界で heap を触り直さない。
  void reset() {
    for (size_t i = 0; i < kHistCount; ++i) {
      std::memset(bins_[i].data(), 0, bins_[i].size() * sizeof(double));
    }
    for (size_t p = 0; p < kPlaneCount; ++p) saturation_[p] = Saturation{};
  }

  // 組み上がったイベント 1 個を積む。**O(items) の配列加算のみ**。
  void on_event(const rootsink::BuiltEvent& ev) {
    // 面毎の最大波高(ChargeMax 用)。-1 = その面に計数チャンネルが 1 本も無い。
    int32_t plane_peak[kPlaneCount] = {-1, -1, -1};

    for (const rootsink::OwnedFragment& f : ev.fragments) {
      load_channel_roles(f.cobo, f.asad);
      // 波高 scratch。-1 = このイベントでサンプル 0(= 数えない)。
      for (uint32_t a = 0; a < kAgetChips; ++a) {
        for (uint32_t c = 0; c < kRawChPerAget; ++c) peak_[a][c] = -1;
      }

      const size_t n = f.item_count();
      for (size_t i = 0; i < n; ++i) {
        const uint32_t w = f.item(i);
        const uint32_t aget = (w >> 30) & 0x3u;       // [31:30]
        const uint32_t chan = (w >> 23) & 0x7Fu;      // [29:23] raw 0–67(FPN 込み)
        const uint32_t bucket = (w >> 14) & 0x1FFu;   // [22:14] 0..511
        const uint32_t adc = w & 0xFFFu;              // [11:0] 生 ADC(減算なし)
        if (chan >= kRawChPerAget) {
          // 7bit の chan は 127 まで表現できるが AGET は 68 ch —— 置き場が無い。
          // 黙っては捨てない(CLAUDE.md)。
          ++items_out_of_range_;
          continue;
        }
        const ChanRole& role = roles_[aget][chan];
        if (role.plane < 0) continue;  // FPN / AUX / 未記載 = ヒストに入れない
        bins_[strip_time_index(static_cast<tpcgeo::Plane>(role.plane))]
             [role.row_offset + bucket] += static_cast<double>(adc);
        if (static_cast<int32_t>(adc) > peak_[aget][chan]) {
          peak_[aget][chan] = static_cast<int32_t>(adc);
        }
      }

      // チャンネル(= 物理ストリップ)毎の波高を 1 エントリずつ Charge へ。
      // peak_ が立っているのは Strip チャンネルだけ(上のループで弾いてある)。
      for (uint32_t a = 0; a < kAgetChips; ++a) {
        for (uint32_t c = 0; c < kRawChPerAget; ++c) {
          const int32_t peak = peak_[a][c];
          if (peak < 0) continue;  // このイベントでサンプル 0 = 分母にも入れない
          const size_t p = static_cast<size_t>(roles_[a][c].plane);
          ++saturation_[p].counted;
          if (peak >= static_cast<int32_t>(kSaturationAdc)) ++saturation_[p].saturated;
          bins_[3 + p][static_cast<uint32_t>(peak) / kChargeBinWidth] += 1.0;
          if (peak > plane_peak[p]) plane_peak[p] = peak;
        }
      }
    }

    for (size_t p = 0; p < kPlaneCount; ++p) {
      if (plane_peak[p] < 0) continue;  // 計数チャンネルが 1 本も無い面は入れない
      bins_[6 + p][static_cast<uint32_t>(plane_peak[p]) / kChargeBinWidth] += 1.0;
    }
  }

  const HistMeta& meta(size_t i) const { return meta_[i]; }
  const std::vector<double>& bins(size_t i) const { return bins_[i]; }
  Saturation saturation(tpcgeo::Plane p) const { return saturation_[static_cast<size_t>(p)]; }

  // AGET に存在しない chan を持つ item の数(累積。run をまたいで消さない)。
  uint64_t items_out_of_range() const { return items_out_of_range_; }
  // ジオメトリが返した strip がヒストのレンジ外だった回数(累積)。
  uint64_t strips_out_of_range() const { return strips_out_of_range_; }

  // --- スナップショット(Publisher / RunClose の受け皿)---
  //
  // `prepare` は**ロック外で 1 回**呼んで受け皿を確保する。`copy_into` は
  // **ロック中に呼ぶ** —— 中身は memcpy だけ(SPEC §5.1 の「ロック中に許すのは
  // 配列 fill / memcpy / 数個の u64 コピーだけ」)。
  void prepare(HistSnapshot& out) const {
    for (size_t i = 0; i < kHistCount; ++i) {
      out[i].id = meta_[i].id;
      out[i].name = meta_[i].name;
      out[i].nx = meta_[i].nx;
      out[i].ny = meta_[i].ny;
      out[i].bins.assign(bins_[i].size(), 0.0);
    }
  }

  void copy_into(HistSnapshot& out) const {
    for (size_t i = 0; i < kHistCount; ++i) {
      // prepare 済みなら常に一致する(不一致は取り違えなので直して続ける)。
      if (out[i].bins.size() != bins_[i].size()) out[i].bins.resize(bins_[i].size());
      out[i].id = meta_[i].id;
      out[i].name = meta_[i].name;
      out[i].nx = meta_[i].nx;
      out[i].ny = meta_[i].ny;
      if (!bins_[i].empty()) {
        std::memcpy(out[i].bins.data(), bins_[i].data(), bins_[i].size() * sizeof(double));
      }
    }
  }

 private:
  // (aget, raw_ch) の役割キャッシュ。`plane < 0` = ヒストに入れないチャンネル。
  struct ChanRole {
    int8_t plane = -1;
    uint32_t row_offset = 0;  // (strip - 1) * kBuckets
  };

  // フラグメント毎に 1 回だけジオメトリを引き直す(272 回)。
  void load_channel_roles(uint8_t cobo, uint8_t asad) {
    for (uint32_t a = 0; a < kAgetChips; ++a) {
      for (uint32_t c = 0; c < kRawChPerAget; ++c) {
        ChanRole& out = roles_[a][c];
        out.plane = -1;
        out.row_offset = 0;
        const tpcgeo::ChannelRole role = geo_.lookup(cobo, asad, a, c);
        if (role.kind != tpcgeo::RoleKind::Strip) continue;
        const size_t p = static_cast<size_t>(role.plane);
        if (role.strip < 1 || role.strip > geo_.max_strip[p]) {
          // ヒストのレンジ外(ジオメトリが自分の max_strip と食い違っている)。
          // 黙って踏まない —— 数えて捨てる(CLAUDE.md)。
          ++strips_out_of_range_;
          continue;
        }
        out.plane = static_cast<int8_t>(p);
        out.row_offset = (static_cast<uint32_t>(role.strip) - 1) * kBuckets;
      }
    }
  }

  static constexpr const char* kStripTimeNames[kPlaneCount] = {"StripTimeU", "StripTimeV",
                                                                "StripTimeW"};
  static constexpr const char* kChargeNames[kPlaneCount] = {"ChargeU", "ChargeV", "ChargeW"};
  static constexpr const char* kChargeMaxNames[kPlaneCount] = {"ChargeMaxU", "ChargeMaxV",
                                                                "ChargeMaxW"};

  const tpcgeo::Geometry& geo_;
  std::array<HistMeta, kHistCount> meta_{};
  std::array<std::vector<double>, kHistCount> bins_{};
  Saturation saturation_[kPlaneCount]{};
  ChanRole roles_[kAgetChips][kRawChPerAget]{};
  int32_t peak_[kAgetChips][kRawChPerAget]{};
  uint64_t items_out_of_range_ = 0;
  uint64_t strips_out_of_range_ = 0;
};

}  // namespace rsmon

#endif  // TPCDAQ_ROOT_SINK_MONITOR_HIST_HPP
