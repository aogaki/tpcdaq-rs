// pevent_fill.hpp — GET::GDataFrame → PEventTPC 充填(SPEC §6.4 v1.8、TODO/020)。
//
// **意味論の正**(2026-08-13 実ソース確認、`reference/TPCReco/TPCReco-HIGS2026_online/`):
//   * `EventSources/src/EventSourceGRAW.cpp::fillEventFromFrame`(L284-325)= 充填本体
//   * `GrawToROOT/src/PedestalCalculatorGRAW.cpp` + `DataFormats/src/PedestalCalculator.cpp`
//     = ペデスタル算法
// この 2 つを**純算術**で移植したもの。TPCReco の `GeometryTPC` / `TProfile` には
// 依存しない —— ジオメトリは我々の `geo.hpp`(018、Rust `src/geometry.rs` の逐語移植で
// 実 .dat オラクル一致済み)、TProfile は「256 ビン・整数中心・重み 1」なので
// **平均 = 合計 / 件数** で同値(空ビンの `GetBinContent` = 0.0 も含めて再現)。
//
// 入力を GDataFrame に揃えてあるのは SPEC §6.4 の指示(「fillEventFromFrame と同じ
// 入力形に揃えることで意味論の等価性を構造的に担保」)。Fragment → GDataFrame の
// 充填は 011 で実データ全値一致を実証済み。
//
// **ROOT に触るのは PEventTPC / GET クラス経由だけ**。このヘッダを include してよいのは
// root_recorder.hpp と test_pevent.cxx。
//
// --- TPCReco 本家との意図的な差(2 点だけ)-----------------------------------
//  1. `GDataFrame::SearchChannel` を使わない。あれは TIter の線形走査で、
//     1 フレームあたり (64+4)×4 回 × 272 チャンネル ≒ 7 万回の比較になる。
//     **フレーム毎に 1 回だけ** (aget, raw_ch) の索引を作って O(1) で引く
//     (CLAUDE.md「ホットパスで per-frame の heap 確保をしない」—— 索引は固定長配列)。
//     同じ (aget,chan) が 2 度現れたときは SearchChannel と同じく**先勝ち**。
//  2. `PEventTPC::AddValByStrip` は chargeMap と **`myChargeArray[3][3][256][512]`** の
//     両方に書く。strip 番号が 256 以上(ELITPC は 1–1024)だとこの固定長配列を
//     はみ出して UB になるので、**範囲外は捨てて数える**(`keys_out_of_range()`)。
//     本家は境界検査をしていない —— 黙って踏むより落として数える(CLAUDE.md)。

#ifndef TPCDAQ_ROOT_SINK_PEVENT_FILL_HPP
#define TPCDAQ_ROOT_SINK_PEVENT_FILL_HPP

#include <cstdint>
#include <cstdio>
#include <cstring>
#include <ctime>
#include <string>
#include <tuple>

#include <TClonesArray.h>
#include <TRefArray.h>

#include "GDataChannel.h"
#include "GDataFrame.h"
#include "GDataSample.h"
#include "TPCReco/PEventTPC.h"
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
// Filler — 1 フラグメント(= 1 GDataFrame = 1 (cobo,asad))を PEventTPC に足し込む
// ---------------------------------------------------------------------------
//
// 状態(FPN 平均・チャンネルオフセット)は **フレーム毎にリセット**される
// (TPCReco も per (cobo,asad) フレーム毎に `ResetTables` する = run 状態を持たない)。
// 作業配列はメンバに置いて使い回す —— ホットパスで heap を触らない(CLAUDE.md)。
class Filler {
 public:
  Filler(const tpcgeo::Geometry& geometry, FillConfig cfg)
      : geo_(geometry), cfg_(cfg) {}

  Filler(const Filler&) = delete;
  Filler& operator=(const Filler&) = delete;

  // `frame` の normal チャンネルを `out` の chargeMap に足し込む。
  // **`out` は呼び手が Clear 済みであること**(1 イベント = 複数フラグメント)。
  void add_frame(GET::GDataFrame& frame, PEventTPC& out) {
    const uint32_t cobo = frame.fHeader.fCoboIdx;
    const uint32_t asad = frame.fHeader.fAsadIdx;

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
      return;
    }

    build_channel_index(frame);
    if (cfg_.remove_pedestal) compute_pedestals();

    for (int aget = 0; aget < kAgetChips; ++aget) {
      for (int chan = 0; chan < kSignalChPerAget; ++chan) {
        // normal(0–63)→ raw(0–67)。FPN リオーダの正は geo.hpp の定数表
        // (TPCReco `Aget_normal2raw` と全 64 入力一致を 002/018 で確認済み)。
        const uint32_t raw_ch = tpcgeo::kReorderFromGeometryToGraw[chan];
        GET::GDataChannel* channel = index_[aget][raw_ch];
        if (channel == nullptr) continue;

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

        for (int i = 0; i < channel->fNsamples; ++i) {
          const GET::GDataSample* sample =
              static_cast<GET::GDataSample*>(channel->fSamples.At(i));
          if (sample == nullptr) continue;
          const int cell = sample->fBuckIdx;
          if (cell < cfg_.min_signal_cell || cell > cfg_.max_signal_cell) continue;
          double value = sample->fValue;
          if (cfg_.remove_pedestal) {
            value -= offset_[aget][chan] + fpn_ave_signal_[aget][cell];
          }
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
  }

  // --- カウンタ(すべて累積。「黙って落とさない」ための可視化)---
  uint64_t values_added() const { return values_added_; }
  uint64_t channels_without_strip() const { return channels_without_strip_; }
  uint64_t keys_out_of_range() const { return keys_out_of_range_; }
  uint64_t frames_outside_geometry() const { return frames_outside_geometry_; }

  const FillConfig& config() const { return cfg_; }

 private:
  // (aget, raw_ch) → チャンネル。フレーム毎に 1 回だけ作る(冒頭の注記 1)。
  void build_channel_index(GET::GDataFrame& frame) {
    std::memset(index_, 0, sizeof(index_));
    TClonesArray* channels = frame.GetChannels();
    if (channels == nullptr) return;
    const Int_t n = frame.GetNchannels();
    for (Int_t i = 0; i < n; ++i) {
      GET::GDataChannel* ch = static_cast<GET::GDataChannel*>(channels->At(i));
      if (ch == nullptr) continue;
      const int aget = ch->fAgetIdx;
      const int raw = ch->fChanIdx;
      if (aget < 0 || aget >= kAgetChips || raw < 0 || raw >= kRawChPerAget) continue;
      if (index_[aget][raw] == nullptr) index_[aget][raw] = ch;  // SearchChannel = 先勝ち
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
      for (size_t f = 0; f < tpcgeo::kFpnRawChannels.size(); ++f) {
        GET::GDataChannel* channel = index_[aget][tpcgeo::kFpnRawChannels[f]];
        if (channel == nullptr) continue;
        for (int i = 0; i < channel->fNsamples; ++i) {
          const GET::GDataSample* sample =
              static_cast<GET::GDataSample*>(channel->fSamples.At(i));
          if (sample == nullptr) continue;
          const int cell = sample->fBuckIdx;
          if (cell < 0 || cell >= kTimeCells) continue;
          if (cell >= cfg_.min_pedestal_cell && cell <= cfg_.max_pedestal_cell) {
            fpn_ave_pedestal_[aget][cell] += sample->fValue;
            ++fpn_entries_pedestal_[aget][cell];
          }
          if (cell >= cfg_.min_signal_cell && cell <= cfg_.max_signal_cell) {
            fpn_ave_signal_[aget][cell] += sample->fValue;
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
        GET::GDataChannel* channel = index_[aget][tpcgeo::kReorderFromGeometryToGraw[chan]];
        if (channel == nullptr) continue;
        for (int i = 0; i < channel->fNsamples; ++i) {
          const GET::GDataSample* sample =
              static_cast<GET::GDataSample*>(channel->fSamples.At(i));
          if (sample == nullptr) continue;
          const int cell = sample->fBuckIdx;
          if (cell < cfg_.min_pedestal_cell || cell > cfg_.max_pedestal_cell) continue;
          offset_sum_[aget][chan] +=
              static_cast<double>(sample->fValue) - fpn_ave_pedestal_[aget][cell];
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

  GET::GDataChannel* index_[kAgetChips][kRawChPerAget] = {};

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
};

}  // namespace tpcpevent

#endif  // TPCDAQ_ROOT_SINK_PEVENT_FILL_HPP
