// pevent_fill.hpp — Fragment → PEventTPC 充填(SPEC §6.4 v1.17、TODO/020 → 054)。
//
// **意味論の正**(2026-08-15 に `reference/TPCReco/latest/` の実ソースで再確認):
//   * `EventSources/src/EventSourceGRAW.cpp:301-323`(`fillEventFromFrame`)= 充填本体
//     —— ループ順(aget 外・normal chan 内)/ `Aget_normal2raw` リオーダ / signal 窓 /
//     減算 / strip 射影 / chargeMap への `+=` 加算。
//   * `GrawToROOT/src/PedestalCalculatorGRAW.cpp:127-205` +
//     `DataFormats/src/PedestalCalculator.cpp:255-262` = ペデスタル算法
//     —— (cobo,asad) フレーム毎リセット / FPN 平均 2 本(窓別)/ チャンネルオフセット /
//     `correction = offset + FPN_ave_signal[cell]`。
// この 2 つを**純算術**で移植したもの。TPCReco の `GeometryTPC` / `TProfile` には
// 依存しない —— ジオメトリは我々の `geo.hpp`(018、Rust `src/geometry.rs` の逐語移植で
// 実 .dat オラクル一致済み)、TProfile は「256 ビン・整数中心・重み 1」なので
// **平均 = 合計 / 件数** で同値(空ビンの `GetBinContent` = 0.0 も含めて再現)。
//
// **v1.17(TODO/054)で GDataFrame 中間表現を撤去した**。以前は Fragment を
// `GET::GDataFrame`(TClonesArray + TRefArray)へ一度展開してから読んでいたが、
// GDataFrame は GET CoBoFrameViewer 由来のオフライン永続化モデルであって
// 我々のチェーンには不要(ユーザー裁定 2026-08-15、SPEC v1.17)。等価性の担保は
// 中間表現の共有ではなく **§12-3 の内容一致オラクル**(021: 3852 events / 0 differences)
// が担う。性能上も中間表現は 21 ms/event の 15% を占めていた(053 計測)。
//
// **ROOT に触るのは PEventTPC 経由だけ**。このヘッダを include してよいのは
// root_recorder.hpp と test_pevent.cxx。
//
// --- TPCReco 本家との意図的な差(3 点)-----------------------------------------
//  1. `GDataFrame::SearchChannel`(TIter の線形走査)に相当するものを作らない。
//     items を **(aget, raw_ch) の固定長マス**に配ってから読む。同じ (aget,chan) の
//     item は同じマスに集まるので、SearchChannel の「先勝ち」で 2 個目のチャンネルが
//     無視されるという状況自体が起きない(GDataFrame 経由でも build_frame が
//     マス単位で 1 チャンネルしか作らなかったので、以前から先勝ちは不発だった)。
//  2. `PEventTPC::AddValByStrip` は chargeMap と **`myChargeArray[3][3][256][512]`** の
//     両方に書く。strip 番号が 256 以上(ELITPC は 1–1024)だとこの固定長配列を
//     はみ出して UB になるので、**範囲外は捨てて数える**(`keys_out_of_range()`)。
//     本家は境界検査をしていない —— 黙って踏むより落として数える(CLAUDE.md)。
//  3. item の chan フィールドは 7bit(0–127)だが AGET は 68 ch。置き場が無い item は
//     **落として数える**(`items_out_of_range()`)。我々独自の防御で、本家に対応物は無い。

#ifndef TPCDAQ_ROOT_SINK_PEVENT_FILL_HPP
#define TPCDAQ_ROOT_SINK_PEVENT_FILL_HPP

#include <algorithm>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <ctime>
#include <string>
#include <tuple>
#include <vector>

#include "TPCReco/PEventTPC.h"
#include "eb_core.hpp"
#include "geo.hpp"

namespace tpcpevent {

// SPEC §6.4 の既定(TPCReco `allowedOptions.json` の既定と同値)。
constexpr int kDefaultMinPedestalCell = 5;
constexpr int kDefaultMaxPedestalCell = 25;
constexpr int kDefaultMinSignalCell = 5;
constexpr int kDefaultMaxSignalCell = 506;

// AGET の物理構造(geo.hpp と同じ値。ここで再定義せず参照する)。
constexpr int kAgetChips = static_cast<int>(tpcgeo::kAgetChipsPerAsad);   // 4
constexpr int kRawChPerAget = static_cast<int>(tpcgeo::kRawChPerAget);    // 68
constexpr int kSignalChPerAget = static_cast<int>(tpcgeo::kSignalChPerAget);  // 64
constexpr int kTimeCells = 512;  // AGET の SCA 深さ(bucket は 9 bit)

// ---------------------------------------------------------------------------
// 設定
// ---------------------------------------------------------------------------

struct FillConfig {
  bool remove_pedestal = true;  // --pedestal-remove(既定 ON)
  int min_pedestal_cell = kDefaultMinPedestalCell;
  int max_pedestal_cell = kDefaultMaxPedestalCell;
  int min_signal_cell = kDefaultMinSignalCell;
  int max_signal_cell = kDefaultMaxSignalCell;

  // TPCReco `PedestalCalculator::VerifyTimeCellIndices` と同じ条件(あちらは throw、
  // こちらは呼び手が起動エラーにする —— SPEC §3.2「設定パースエラーは起動失敗」)。
  bool validate(std::string* why) const {
    if (min_signal_cell < 0 || max_signal_cell < 0 || min_signal_cell >= max_signal_cell ||
        min_signal_cell >= kTimeCells || max_signal_cell >= kTimeCells) {
      if (why != nullptr) {
        *why = "signal cell window [" + std::to_string(min_signal_cell) + ", " +
               std::to_string(max_signal_cell) + "] is not 0 <= min < max <= 511";
      }
      return false;
    }
    if (min_pedestal_cell < 0 || max_pedestal_cell < 0 ||
        min_pedestal_cell >= max_pedestal_cell || min_pedestal_cell >= kTimeCells ||
        max_pedestal_cell >= kTimeCells) {
      if (why != nullptr) {
        *why = "pedestal cell window [" + std::to_string(min_pedestal_cell) + ", " +
               std::to_string(max_pedestal_cell) + "] is not 0 <= min < max <= 511";
      }
      return false;
    }
    return true;
  }
};

// ---------------------------------------------------------------------------
// runId(TPCReco `RunIdParser` と同じ導出 = run 開始時刻の %Y%m%d%H%M%S)
// ---------------------------------------------------------------------------
//
// 実機オラクル `PEventTPC_2026-08-11T07-47-37.051_0000.root` の runId = 20260811074737。
// 生 graw のファイル名に入る TS(DataRouter が付ける run 開始時刻)と同じ暦時刻を
// 使う —— したがって**ローカル時刻**(`CoBoClock::now()` も `local_time()` を使う)。
inline long run_id_from_tm(const std::tm& tm) {
  return (static_cast<long>(tm.tm_year) + 1900L) * 10000000000L +
         (static_cast<long>(tm.tm_mon) + 1L) * 100000000L +
         static_cast<long>(tm.tm_mday) * 1000000L + static_cast<long>(tm.tm_hour) * 10000L +
         static_cast<long>(tm.tm_min) * 100L + static_cast<long>(tm.tm_sec);
}

// 現在時刻(ローカル)から runId を作る。run を開いた瞬間に 1 回だけ呼ぶこと。
inline long run_id_now() {
  const std::time_t now = std::time(nullptr);
  std::tm tm{};
#if defined(_WIN32)
  localtime_s(&tm, &now);
#else
  localtime_r(&now, &tm);
#endif
  return run_id_from_tm(tm);
}

// ---------------------------------------------------------------------------
// Filler — 1 フラグメント(= 1 (cobo,asad) のフレーム)を PEventTPC に足し込む
// ---------------------------------------------------------------------------
//
// 状態(FPN 平均・チャンネルオフセット)は **フレーム毎にリセット**される
// (TPCReco も per (cobo,asad) フレーム毎に `ResetTables` する = run 状態を持たない)。
// 作業配列はメンバに置いて使い回す —— ホットパスで heap を触らない(CLAUDE.md)。
class Filler {
 public:
  Filler(const tpcgeo::Geometry& geometry, FillConfig cfg) : geo_(geometry), cfg_(cfg) {
    for (int a = 0; a < kAgetChips; ++a) {
      // 実機 mini/ELITPC はフルウィンドウ 512 サンプル/ch。最初のフレームで伸びきって
      // 以後は容量が残る(ホットパスで heap 確保をしない — CLAUDE.md)。
      for (int c = 0; c < kRawChPerAget; ++c) grid_[a][c].reserve(kTimeCells);
    }
  }

  Filler(const Filler&) = delete;
  Filler& operator=(const Filler&) = delete;

  // `f` の normal チャンネルを `out` の chargeMap に足し込む。
  // **`out` は呼び手が Clear 済みであること**(1 イベント = 複数フラグメント)。
  void add_fragment(const rootsink::OwnedFragment& f, PEventTPC& out) {
    const uint32_t cobo = f.cobo;
    const uint32_t asad = f.asad;

    scatter_items(f);

    // ジオメトリに無い (cobo,asad) は丸ごと捨てる(本家 fillEventFromFrame の
    // 「ASAD_idx >= GetAsadNboards() → Frame skipped」と同じ)。黙っては捨てない。
    if (!geo_.index_of(cobo, asad, 0, 0).has_value()) {
      ++frames_outside_geometry_;
      if (frames_outside_geometry_ == 1) {
        std::fprintf(stderr,
                     "root_sink: frame (cobo=%u asad=%u) is not in the geometry — "
                     "skipped, counted\n",
                     cobo, asad);
      }
      clear_grid();
      return;
    }

    sort_unsorted_cells();
    if (cfg_.remove_pedestal) compute_pedestals();

    // 走査順は **(aget 外・normal chan 内)昇順** = TPCReco `fillEventFromFrame` の
    // ループ順そのもの。
    //
    // **TODO/054-A の実測(2026-08-16。「挿入列の整列性を活かす」を試して棄却)**:
    // chargeMap の key 昇順(= strip 昇順)に並べ替えると、macOS/libc++ では
    // 隔離プローブの Filler が 6.25 → 15.67 ms/event に**悪化**した。切り分けの
    // micro ベンチ(128,512 insert)も同傾向 —— strip 散在 8.26 ms / key 昇順 15.12 ms。
    // 理由: libc++ の `map::operator[]` は各段で `k < node` と `node < k` を**両方**
    // 評価する。key 昇順だと降下経路(右スパイン)のノードは dir/section/number が
    // 一致するので tuple 比較が毎回 4 要素すべてを舐めるが、散在順なら 3 要素目で
    // 早期に決着する。**この走査順が既に最善**なので変えない。
    for (int aget = 0; aget < kAgetChips; ++aget) {
      for (int chan = 0; chan < kSignalChPerAget; ++chan) {
        // normal(0–63)→ raw(0–67)。FPN リオーダの正は geo.hpp の定数表
        // (TPCReco `Aget_normal2raw` と全 64 入力一致を 002/018 で確認済み)。
        const uint32_t raw_ch = tpcgeo::kReorderFromGeometryToGraw[chan];
        const std::vector<Sample>& cell_samples = grid_[aget][raw_ch];
        if (cell_samples.empty()) continue;

        const tpcgeo::ChannelRole role = geo_.lookup(cobo, asad, aget, raw_ch);
        if (role.kind != tpcgeo::RoleKind::Strip) {
          // FPN・AUX・.dat 未記載 —— strip でないものは chargeMap に置き場が無い。
          // ロスレスは生 graw が担う(SPEC §6.4「ここは変換出力」)。
          ++channels_without_strip_;
          continue;
        }
        const int dir = static_cast<int>(role.plane);
        const int section = static_cast<int>(role.section);
        const int number = static_cast<int>(role.strip);
        const bool key_ok = dir >= 0 && dir < static_cast<int>(PEventTPC::max_strip_dirs) &&
                            section >= 0 &&
                            section < static_cast<int>(PEventTPC::max_strip_sections) &&
                            number >= 0 &&
                            number < static_cast<int>(PEventTPC::max_strip_numbers);

        const double offset = offset_[aget][chan];
        const double* fpn_signal = fpn_ave_signal_[aget];
        for (const Sample& s : cell_samples) {
          const int cell = s.bucket;
          if (cell < cfg_.min_signal_cell || cell > cfg_.max_signal_cell) continue;
          double value = s.adc;
          if (cfg_.remove_pedestal) value -= offset + fpn_signal[cell];
          // `myChargeArray[3][3][256][512]` をはみ出す key は書かない(冒頭の注記 2)。
          if (!key_ok || cell < 0 || cell >= static_cast<int>(PEventTPC::max_strip_time_cells)) {
            ++keys_out_of_range_;
            if (keys_out_of_range_ == 1) {
              std::fprintf(stderr,
                           "root_sink: charge key {dir=%d section=%d number=%d cell=%d} is "
                           "outside PEventTPC's fixed array — dropped, counted\n",
                           dir, section, number, cell);
            }
            continue;
          }
          out.AddValByStrip(std::make_tuple(dir, section, number, cell), value);
          ++values_added_;
        }
      }
    }
    clear_grid();
  }

  // --- カウンタ(すべて累積。「黙って落とさない」ための可視化)---
  uint64_t values_added() const { return values_added_; }
  uint64_t channels_without_strip() const { return channels_without_strip_; }
  uint64_t keys_out_of_range() const { return keys_out_of_range_; }
  uint64_t frames_outside_geometry() const { return frames_outside_geometry_; }
  // item の chan が AGET の 68 ch を外れていて置き場が無かった件数(冒頭の注記 3)。
  uint64_t items_out_of_range() const { return items_out_of_range_; }

  const FillConfig& config() const { return cfg_; }

 private:
  struct Sample {
    uint16_t bucket;
    uint16_t adc;
  };
  static bool by_bucket(const Sample& a, const Sample& b) { return a.bucket < b.bucket; }

  // items(u32 LE の連結)を (aget, raw_ch) のマスに配る。grid_ は使い回し
  // (clear は容量を残すのでホットパスで heap 確保が起きない —— CLAUDE.md)。
  void scatter_items(const rootsink::OwnedFragment& f) {
    const size_t n = f.item_count();
    for (size_t i = 0; i < n; ++i) {
      const uint32_t w = f.item(i);
      const uint32_t aget = (w >> 30) & 0x3u;      // [31:30]
      const uint32_t chan = (w >> 23) & 0x7Fu;     // [29:23] raw 0–67(FPN 込み)
      const uint32_t bucket = (w >> 14) & 0x1FFu;  // [22:14]
      const uint32_t adc = w & 0xFFFu;             // [11:0] 生 ADC(減算なし)
      if (chan >= static_cast<uint32_t>(kRawChPerAget)) {
        // 7bit の chan は 127 まで表現できるが AGET は 68 ch。置き場が無いので
        // 落とすしかない —— **黙っては落とさない**(CLAUDE.md)。
        ++items_out_of_range_;
        if (items_out_of_range_ == 1) {
          std::fprintf(stderr,
                       "root_sink: item chan=%u is outside the AGET's %d channels — "
                       "dropped, counted\n",
                       chan, kRawChPerAget);
        }
        continue;
      }
      std::vector<Sample>& cell = grid_[aget][chan];
      // チャンネル内は bucket 昇順であること(標準リーダの前提、SPEC §6.4)。
      // 実データは既に昇順なので、判定だけして普段は並べ替えない。
      if (!cell.empty() && bucket < cell.back().bucket) unsorted_[aget][chan] = true;
      cell.push_back(Sample{static_cast<uint16_t>(bucket), static_cast<uint16_t>(adc)});
    }
  }

  void sort_unsorted_cells() {
    for (int a = 0; a < kAgetChips; ++a) {
      for (int c = 0; c < kRawChPerAget; ++c) {
        if (!unsorted_[a][c]) continue;
        std::stable_sort(grid_[a][c].begin(), grid_[a][c].end(), by_bucket);
        unsorted_[a][c] = false;
      }
    }
  }

  void clear_grid() {
    for (int a = 0; a < kAgetChips; ++a) {
      for (int c = 0; c < kRawChPerAget; ++c) {
        grid_[a][c].clear();  // 容量は残る
        unsorted_[a][c] = false;
      }
    }
  }

  // ①FPN 4ch の cell 毎平均(ペデスタル窓・signal 窓それぞれ)
  // ②normal ch のペデスタル窓で `raw − FPN平均(ped)` のチャンネル毎平均 = オフセット
  void compute_pedestals() {
    std::memset(fpn_ave_pedestal_, 0, sizeof(fpn_ave_pedestal_));
    std::memset(fpn_ave_signal_, 0, sizeof(fpn_ave_signal_));
    std::memset(fpn_entries_pedestal_, 0, sizeof(fpn_entries_pedestal_));
    std::memset(fpn_entries_signal_, 0, sizeof(fpn_entries_signal_));
    std::memset(offset_, 0, sizeof(offset_));
    std::memset(offset_sum_, 0, sizeof(offset_sum_));
    std::memset(offset_entries_, 0, sizeof(offset_entries_));

    for (int aget = 0; aget < kAgetChips; ++aget) {
      // --- ① FPN(raw 11/22/45/56 = TPCReco `Aget_fpn2raw`)---
      for (size_t fi = 0; fi < tpcgeo::kFpnRawChannels.size(); ++fi) {
        for (const Sample& s : grid_[aget][tpcgeo::kFpnRawChannels[fi]]) {
          const int cell = s.bucket;
          if (cell < 0 || cell >= kTimeCells) continue;
          if (cell >= cfg_.min_pedestal_cell && cell <= cfg_.max_pedestal_cell) {
            fpn_ave_pedestal_[aget][cell] += s.adc;
            ++fpn_entries_pedestal_[aget][cell];
          }
          if (cell >= cfg_.min_signal_cell && cell <= cfg_.max_signal_cell) {
            fpn_ave_signal_[aget][cell] += s.adc;
            ++fpn_entries_signal_[aget][cell];
          }
        }
      }
      // 件数 0 の cell は **0.0 のまま**(本家も割らずに 0 を残す)。
      for (int cell = cfg_.min_pedestal_cell; cell <= cfg_.max_pedestal_cell; ++cell) {
        if (fpn_entries_pedestal_[aget][cell] > 0) {
          fpn_ave_pedestal_[aget][cell] /=
              static_cast<double>(fpn_entries_pedestal_[aget][cell]);
        }
      }
      for (int cell = cfg_.min_signal_cell; cell <= cfg_.max_signal_cell; ++cell) {
        if (fpn_entries_signal_[aget][cell] > 0) {
          fpn_ave_signal_[aget][cell] /= static_cast<double>(fpn_entries_signal_[aget][cell]);
        }
      }

      // --- ② normal ch のオフセット(TProfile 256 ビン整数中心 = 平均) ---
      for (int chan = 0; chan < kSignalChPerAget; ++chan) {
        for (const Sample& s : grid_[aget][tpcgeo::kReorderFromGeometryToGraw[chan]]) {
          const int cell = s.bucket;
          if (cell < cfg_.min_pedestal_cell || cell > cfg_.max_pedestal_cell) continue;
          offset_sum_[aget][chan] +=
              static_cast<double>(s.adc) - fpn_ave_pedestal_[aget][cell];
          ++offset_entries_[aget][chan];
        }
        // 空ビンの TProfile::GetBinContent は 0.0 —— そのまま(補正 = FPN 平均のみ)。
        if (offset_entries_[aget][chan] > 0) {
          offset_[aget][chan] =
              offset_sum_[aget][chan] / static_cast<double>(offset_entries_[aget][chan]);
        }
      }
    }
  }

  const tpcgeo::Geometry& geo_;
  FillConfig cfg_;

  // (aget, raw_ch) の作業マス。1 フレーム分だけ使って毎回 clear する。
  std::vector<Sample> grid_[kAgetChips][kRawChPerAget];
  bool unsorted_[kAgetChips][kRawChPerAget] = {};

  double fpn_ave_pedestal_[kAgetChips][kTimeCells] = {};
  double fpn_ave_signal_[kAgetChips][kTimeCells] = {};
  uint32_t fpn_entries_pedestal_[kAgetChips][kTimeCells] = {};
  uint32_t fpn_entries_signal_[kAgetChips][kTimeCells] = {};
  double offset_[kAgetChips][kSignalChPerAget] = {};
  double offset_sum_[kAgetChips][kSignalChPerAget] = {};
  uint32_t offset_entries_[kAgetChips][kSignalChPerAget] = {};

  uint64_t values_added_ = 0;
  uint64_t channels_without_strip_ = 0;
  uint64_t keys_out_of_range_ = 0;
  uint64_t frames_outside_geometry_ = 0;
  uint64_t items_out_of_range_ = 0;
};

}  // namespace tpcpevent

#endif  // TPCDAQ_ROOT_SINK_PEVENT_FILL_HPP
