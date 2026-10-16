// ped_table.hpp — ZS(ゼロサプレッション)データ用の pedestal 表(TODO/151)。
// ROOT 非依存・ZMQ 非依存の純ヘッダ(単体テスト = test_ped_table.cpp、`make test`)。
//
// **表 = `calib pedestal` の JSON**(src/calib/pedestal.rs、serde_json の pretty 出力):
//   {"meta": {"inputs": [...], "events": N, "generated_at": "...", "tool_version": "..."},
//    "channels": [{"asad": A, "aget": G, "ch": C, "mean": M, "rms": R, "runs": n,
//                  "is_fpn": b}, ...]}
// キーは (asad, aget, ch)、ch は **raw 0–67**(FPN 込み)、mean は生 ADC の平均。
// CoBo のキーは無い(1 論理 CoBo — ELITPC は 2 枚の zCoBo を 1 CoBo × 4 AsAd として扱う)。
//
// root_sink には JSON パーサが無い(依存を足さない)ので、`tools/scan/run_metrics.C`
// `loadPedTable` と同じ**平坦オブジェクト走査**で読む: `"channels"` の後ろを '{'…'}' で
// 切り、各要素から "asad" / "aget" / "ch" / "mean" の数値を拾う。channels の要素は
// 入れ子を持たない平らなオブジェクトなのでこれで足りる。

#ifndef TPCDAQ_ROOT_SINK_PED_TABLE_HPP
#define TPCDAQ_ROOT_SINK_PED_TABLE_HPP

#include <cmath>
#include <cstddef>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <string>

#include "geo.hpp"

namespace tpcpevent {

// 表の asad 軸の長さ = CoBo の AsAd スロット数(ハードウェア定数。ELITPC = 1 論理 CoBo ×
// 4 AsAd、mini = 1)。これを外れる要素は読まずに数える(`entries_skipped`)。
constexpr uint32_t kPedTableAsads = 4;

struct PedTable {
  std::string path;          // 読んだファイル(起動ログ用)
  std::string generated_at;  // meta.generated_at(無ければ空)
  std::size_t channels = 0;  // マスに入った channels[] 要素の数
  // 使えなかった要素の数(mean が null / キーが欠けている / (asad, aget, ch) が枠外)。
  // 黙って捨てない —— 呼び手が起動ログに出す(CLAUDE.md)。
  std::size_t entries_skipped = 0;
  // mean[asad][aget][raw_ch]。**表に無いマスは NaN**(呼び手は「表に無い ch」として扱う)。
  double mean[kPedTableAsads][tpcgeo::kAgetChipsPerAsad][tpcgeo::kRawChPerAget];

  PedTable() { clear_means(); }

  void clear_means() {
    for (auto& per_asad : mean) {
      for (auto& per_aget : per_asad) {
        for (double& m : per_aget) m = std::nan("");
      }
    }
  }

  // 表の mean。枠外・表に無い ch は NaN(配列の外は読まない)。
  double mean_of(uint32_t asad, uint32_t aget, uint32_t raw_ch) const {
    if (asad >= kPedTableAsads || aget >= tpcgeo::kAgetChipsPerAsad ||
        raw_ch >= tpcgeo::kRawChPerAget) {
      return std::nan("");
    }
    return mean[asad][aget][raw_ch];
  }
};

namespace pedtable_detail {

// obj の中の `"key": <number>` を読む。無い・数値でない(null 等)なら false。
inline bool number_after_key(const std::string& obj, const char* key, double* out) {
  const std::string quoted = std::string("\"") + key + "\"";
  const std::size_t p = obj.find(quoted);
  if (p == std::string::npos) return false;
  const std::size_t colon = obj.find(':', p + quoted.size());
  if (colon == std::string::npos) return false;
  const char* start = obj.c_str() + colon + 1;
  char* end = nullptr;
  const double v = std::strtod(start, &end);
  if (end == start || !std::isfinite(v)) return false;
  *out = v;
  return true;
}

// text の中の `"key": "<string>"` を読む。無ければ空。
inline std::string string_after_key(const std::string& text, const char* key) {
  const std::string quoted = std::string("\"") + key + "\"";
  const std::size_t p = text.find(quoted);
  if (p == std::string::npos) return "";
  const std::size_t colon = text.find(':', p + quoted.size());
  if (colon == std::string::npos) return "";
  const std::size_t open = text.find('"', colon + 1);
  if (open == std::string::npos) return "";
  const std::size_t close = text.find('"', open + 1);
  if (close == std::string::npos) return "";
  return text.substr(open + 1, close - open - 1);
}

}  // namespace pedtable_detail

// `path` の表を `out` に読む。失敗(開けない / "channels" が無い / 使える要素が 1 つも無い)
// は false + `*err`。呼び手(root_sink)は起動エラーにする(SPEC §3.2 の流儀)。
inline bool load_ped_table(const std::string& path, PedTable* out, std::string* err) {
  FILE* fp = std::fopen(path.c_str(), "rb");
  if (fp == nullptr) {
    *err = "cannot open pedestal table " + path;
    return false;
  }
  std::string text;
  char buf[65536];
  std::size_t got = 0;
  while ((got = std::fread(buf, 1, sizeof(buf), fp)) > 0) text.append(buf, got);
  std::fclose(fp);

  std::size_t pos = text.find("\"channels\"");
  if (pos == std::string::npos) {
    *err = "pedestal table " + path + " has no \"channels\" array";
    return false;
  }

  out->path = path;
  out->generated_at = pedtable_detail::string_after_key(text, "generated_at");
  out->channels = 0;
  out->entries_skipped = 0;
  out->clear_means();
  while ((pos = text.find('{', pos)) != std::string::npos) {
    const std::size_t close = text.find('}', pos);
    if (close == std::string::npos) break;
    const std::string obj = text.substr(pos, close - pos + 1);
    pos = close + 1;
    double asad = 0;
    double aget = 0;
    double ch = 0;
    double mu = 0;
    if (!pedtable_detail::number_after_key(obj, "asad", &asad) ||
        !pedtable_detail::number_after_key(obj, "aget", &aget) ||
        !pedtable_detail::number_after_key(obj, "ch", &ch) ||
        !pedtable_detail::number_after_key(obj, "mean", &mu)) {
      ++out->entries_skipped;
      continue;
    }
    if (asad < 0 || asad >= kPedTableAsads || aget < 0 || aget >= tpcgeo::kAgetChipsPerAsad ||
        ch < 0 || ch >= tpcgeo::kRawChPerAget) {
      ++out->entries_skipped;
      continue;
    }
    out->mean[static_cast<uint32_t>(asad)][static_cast<uint32_t>(aget)]
             [static_cast<uint32_t>(ch)] = mu;
    ++out->channels;
  }
  if (out->channels == 0) {
    *err = "pedestal table " + path + ": no usable channels[] entries (asad/aget/ch/mean)";
    return false;
  }
  return true;
}

}  // namespace tpcpevent

#endif  // TPCDAQ_ROOT_SINK_PED_TABLE_HPP
