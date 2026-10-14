// =============================================================================
// drift_end.C — α 線源 run 1 本 → イベントごとの最初 / 最後のヒット cell の分布とドリフト端
// (CURRENT「今日の進行」A: 12.5 MHz ドリフト端 run)。v_drift = L / t_max の材料。
//
//   root -l -b -q 'drift_end.C("run0100_raw.root", "run0100_drift_end.json", "run0100_drift_end.png")'
//   root -l -b -q 'drift_end.C("run0100_raw.root", "out.json", nullptr, 1000)'  (先頭 1000 件)
//
// 終了コード 0 = JSON(と PNG)を書いた、1 = 失敗(理由は stderr)。
//
// 入力 = raw ツリー(SPEC §6.7、tools/root_sink/raw_tree.hpp): TTree `raw`、
//   `adc[{4×nAsad}][68][512]/S`(生 ADC、FPN 込み、未読 ch は全セル 0)。先頭次元(= AGET の数)は
//   焼き込まず TLeaf::GetLen() / (68·512) で求める(run_metrics.C と同じ)。
//
// 手順(イベントごと・AGET ごと。定数と FPN の扱いは run_metrics.C と同じ — それを include して使う):
//   1. 読めた FPN ch {11,22,45,56}(全セル 0 でない)のセル毎平均 fpn(t) を作る(読めた FPN が無い
//      AGET は fpn = 0 — 引かない。数は agets_without_fpn として WARNING)
//   2. 各信号 ch(FPN 除く、全セル 0 の未読 ch は除く)で v(t) = ADC(t) − fpn(t) − ped、
//      ped = (ADC − fpn) の cell 1..20 の平均
//   3. ch 条件 = run_metrics.C の「ヒット ch」(ピーク > 35 かつ ピーク ±5 cells の閾値超の和
//      q ≥ 2000。同じ定数 kAdcThreshold / kPeakHalfCells / kStripQMin、同じ計算)。満たさない ch は無視
//   4. ヒット cell = 条件を満たす ch の v > 35 の cell のうち cell [2, 509) だけ(kHitCellMin /
//      kHitCellMax — 先頭の整定ノイズと窓端 2 cell の跳ねを除く)。
//      イベントの first = 全ヒット ch のヒット cell の最小、last = 最大。ヒットの無いイベントは
//      events に数えるだけで分布に入れない。JSON に hit_cell_range [2, 509] と
//      channel_condition "run_metrics" を書く(文書化)。
//      注: cell < 509 に限るので last ≤ 508 = clip_back_frac(last ≥ 509)は構造的に 0
//
// 指標(ヒットのあるイベントだけの分布。分位点は線形補間 h = (n−1)·q — run_metrics.C の quantile):
//   first_hit_cell {median, p10, p90}、last_hit_cell {median, p10, p90, p99}
//   first_hit_hist / last_hit_hist   cell 0..511 ごとのイベント数(512 個の整数)
//   edge.cell_p99      last の分布を cell の降順に累積して 1 % に達する cell = last_hit_cell.p99
//   edge.width_cells   上端の立ち下がり幅 = (累積 1 % の cell) − (累積 10 % の cell)= p99 − p90
//                      (降順の累積 x % = 昇順の分位点 1 − x。どちらも同じ quantile で補間)
//   edge.t_max_cells   cell_p99 − first_hit_cell.median(トリガ帯からドリフト端までの cell 数)
//   clip_back_frac     last ≥ 509 のイベント数 ÷ ヒットのあるイベント数(窓で切れていて端が
//                      見えていない割合。509 = run_metrics.C の clip_back_edge と同じ境界)
// 値が無ければ JSON は null。
//
// PNG(任意): first / last のヒストを重ねた 1 枚(x = cell 0..511、対数 y)。
// =============================================================================
#include "run_metrics.C"

#include <TCanvas.h>
#include <TH1.h>
#include <TLegend.h>
#include <TROOT.h>
#include <TSystem.h>

#include <algorithm>
#include <climits>
#include <cstdio>
#include <string>
#include <vector>

namespace driftend {

using runmetrics::kAdcThreshold;
using runmetrics::kCells;
using runmetrics::kChannels;
using runmetrics::kFpn;
using runmetrics::kNull;
using runmetrics::kPeakHalfCells;
using runmetrics::kPedHi;
using runmetrics::kPedLo;
using runmetrics::kStripQMin;
using runmetrics::Quantiles;

// 窓で切れている last の下限 = run_metrics.C の clip_back_edge の境界(ヒットセルが ≥ 509)。
const int kClipBackMinCell = runmetrics::kClipEdgeBackMinCell;  // 509

// ヒットに使う cell の範囲 [kHitCellMin, kHitCellMax)。窓端の跳ね(cell 510, 511)と先頭の整定ノイズ
// (cell 0, 1)を除く。上端は run_metrics.C の kClipEdgeBackMinCell(509)に合わせる。
const int kHitCellMin = 2;
const int kHitCellMax = runmetrics::kClipEdgeBackMinCell;  // 509

struct Edge {
  double cell_p99 = kNull, width_cells = kNull, t_max_cells = kNull;
};

struct Result {
  long run = -1;  // -1 = 名前から読めない(JSON は null)
  std::string raw_file;
  long events = 0, events_with_hits = 0;
  // 信号 ch は読めたが FPN が 1 本も読めなかった (イベント, AGET) の数(run_metrics.C と同じ)
  long agets_without_fpn = 0;
  Quantiles first_hit_cell, last_hit_cell;
  double last_hit_cell_p99 = kNull;
  Edge edge;
  double clip_back_frac = kNull;
  std::vector<long> first_hit_hist = std::vector<long>(kCells, 0);
  std::vector<long> last_hit_hist = std::vector<long>(kCells, 0);
};

bool computeDriftEnd(const std::string &rawPath, int maxEvents, Result &r, std::string &err)
{
  r = Result();
  r.raw_file = runmetrics::resolvedPath(rawPath);
  r.run = runmetrics::runFromName(rawPath);

  TFile f(rawPath.c_str(), "READ");
  if (f.IsZombie()) {
    err = "cannot open " + rawPath;
    return false;
  }
  TTree *tree = f.Get<TTree>("raw");
  if (!tree) {
    err = "no TTree 'raw' in " + rawPath + " (SPEC 6.7 raw tree expected)";
    return false;
  }
  TLeaf *leaf = tree->GetLeaf("adc");
  if (!leaf || std::string(leaf->GetTypeName()) != "Short_t" ||
      leaf->GetLen() % (kChannels * kCells) != 0) {
    err = "raw: branch 'adc' is not adc[N][68][512]/S in " + rawPath;
    return false;
  }
  const int nBlocks = leaf->GetLen() / (kChannels * kCells);  // AGET の数(4 × AsAd 数)
  std::vector<Short_t> adc(static_cast<size_t>(leaf->GetLen()));
  if (tree->SetBranchAddress("adc", adc.data()) < 0) {
    err = "raw: cannot read branch 'adc' in " + rawPath;
    return false;
  }

  Long64_t n = tree->GetEntries();
  if (maxEvents > 0) n = std::min<Long64_t>(n, maxEvents);
  std::vector<double> firsts, lasts;
  std::vector<double> fpn(kCells), v(kCells);  // AGET 1 つ分の common mode、ch 1 本分の v
  long clipBack = 0;
  for (Long64_t i = 0; i < n; ++i) {
    tree->GetEntry(i);
    int first = INT_MAX, last = -1;
    for (int b = 0; b < nBlocks; ++b) {
      const Short_t *aget = &adc[static_cast<size_t>(b) * kChannels * kCells];
      // 1. common mode = 読めた FPN ch のセル毎平均(読めた FPN が無ければ 0 のまま)
      std::fill(fpn.begin(), fpn.end(), 0.0);
      int nFpn = 0;
      for (int fc : kFpn) {
        const Short_t *w = aget + fc * kCells;
        if (runmetrics::allZero(w)) continue;
        ++nFpn;
        for (int t = 0; t < kCells; ++t) fpn[t] += w[t];
      }
      if (nFpn > 0)
        for (double &x : fpn) x /= nFpn;
      bool signalRead = false;
      for (int c = 0; c < kChannels; ++c) {
        if (runmetrics::isFpn(c)) continue;
        const Short_t *w = aget + c * kCells;
        if (runmetrics::allZero(w)) continue;
        signalRead = true;
        // 2. ped
        double ped = 0;
        for (int t = kPedLo; t <= kPedHi; ++t) ped += w[t] - fpn[t];
        ped /= (kPedHi - kPedLo + 1);
        // 3. ch 条件 = run_metrics.C の「ヒット ch」: ピークが閾値超、ピーク ±5 の電荷 q ≥ STRIP_Q_MIN
        int peak = 0;
        for (int t = 0; t < kCells; ++t) {
          v[t] = w[t] - fpn[t] - ped;
          if (v[t] > v[peak]) peak = t;
        }
        if (v[peak] <= kAdcThreshold) continue;
        const int tLo = std::max(0, peak - kPeakHalfCells);
        const int tHi = std::min(kCells - 1, peak + kPeakHalfCells);
        double q = 0;
        for (int t = tLo; t <= tHi; ++t)
          if (v[t] > kAdcThreshold) q += v[t];
        if (q < kStripQMin) continue;  // halo / 単独 cell のノイズ
        // 4. ヒット cell = 範囲 [kHitCellMin, kHitCellMax) の v > 閾値
        for (int t = kHitCellMin; t < kHitCellMax; ++t) {
          if (v[t] <= kAdcThreshold) continue;
          first = std::min(first, t);
          last = std::max(last, t);
        }
      }
      if (signalRead && nFpn == 0) ++r.agets_without_fpn;
    }
    if (last < 0) continue;  // ヒット無し
    firsts.push_back(first);
    lasts.push_back(last);
    ++r.first_hit_hist[static_cast<size_t>(first)];
    ++r.last_hit_hist[static_cast<size_t>(last)];
    if (last >= kClipBackMinCell) ++clipBack;
  }

  r.events = static_cast<long>(n);
  r.events_with_hits = static_cast<long>(lasts.size());
  r.first_hit_cell = runmetrics::quantiles(firsts);
  r.last_hit_cell = runmetrics::quantiles(lasts);
  r.last_hit_cell_p99 = runmetrics::quantile(lasts, 0.99);
  r.edge.cell_p99 = r.last_hit_cell_p99;
  r.edge.width_cells = r.last_hit_cell_p99 - r.last_hit_cell.p90;      // NaN なら NaN
  r.edge.t_max_cells = r.last_hit_cell_p99 - r.first_hit_cell.median;  // NaN なら NaN
  if (r.events_with_hits > 0) r.clip_back_frac = static_cast<double>(clipBack) / r.events_with_hits;
  return true;
}

// ---- JSON ------------------------------------------------------------------------------------
std::string array(const std::vector<long> &y)
{
  std::string s = "[";
  for (size_t i = 0; i < y.size(); ++i) s += (i ? ", " : "") + std::to_string(y[i]);
  return s + "]";
}

std::string toJson(const Result &r)
{
  using runmetrics::num;
  const Quantiles &l = r.last_hit_cell;
  std::string s = "{\n";
  s += "  \"run\": " + (r.run < 0 ? std::string("null") : std::to_string(r.run)) + ",\n";
  s += "  \"raw_file\": " + runmetrics::str(r.raw_file) + ",\n";
  s += "  \"events\": " + std::to_string(r.events) + ",\n";
  s += "  \"events_with_hits\": " + std::to_string(r.events_with_hits) + ",\n";
  s += "  \"hit_cell_range\": [" + std::to_string(kHitCellMin) + ", " + std::to_string(kHitCellMax) +
       "],\n";
  s += "  \"channel_condition\": \"run_metrics\",\n";
  s += "  \"first_hit_cell\": " + runmetrics::obj(r.first_hit_cell) + ",\n";
  s += "  \"last_hit_cell\": {\"median\": " + num(l.median) + ", \"p10\": " + num(l.p10) +
       ", \"p90\": " + num(l.p90) + ", \"p99\": " + num(r.last_hit_cell_p99) + "},\n";
  s += "  \"edge\": {\"cell_p99\": " + num(r.edge.cell_p99) +
       ", \"width_cells\": " + num(r.edge.width_cells) +
       ", \"t_max_cells\": " + num(r.edge.t_max_cells) + "},\n";
  s += "  \"clip_back_frac\": " + num(r.clip_back_frac) + ",\n";
  s += "  \"first_hit_hist\": " + array(r.first_hit_hist) + ",\n";
  s += "  \"last_hit_hist\": " + array(r.last_hit_hist) + "\n";
  s += "}\n";
  return s;
}

// ---- PNG -------------------------------------------------------------------------------------
// first / last のヒストを重ねる(対数 y)。描いた物は kCanDelete でキャンバスに持たせる
// (pulser_kernel.C と同じ)。gROOT の一覧には入れない(同じ名前で 2 回呼んでも置き換え警告を出さない)。
bool drawPng(const Result &r, const std::string &path)
{
  gROOT->SetBatch(kTRUE);
  TCanvas c("drift_end", "drift end", 1000, 700);
  c.SetLogy();
  const std::string title = "first / last hit cell (run " +
                            (r.run < 0 ? std::string("?") : std::to_string(r.run)) + ", " +
                            std::to_string(r.events_with_hits) + " of " + std::to_string(r.events) +
                            " events with hits);cell;events";
  TH1D *hFirst = new TH1D("drift_end_first", title.c_str(), kCells, -0.5, kCells - 0.5);
  TH1D *hLast = new TH1D("drift_end_last", title.c_str(), kCells, -0.5, kCells - 0.5);
  long maxCount = 1;
  for (int t = 0; t < kCells; ++t) {
    hFirst->SetBinContent(t + 1, r.first_hit_hist[static_cast<size_t>(t)]);
    hLast->SetBinContent(t + 1, r.last_hit_hist[static_cast<size_t>(t)]);
    maxCount = std::max({maxCount, r.first_hit_hist[static_cast<size_t>(t)],
                         r.last_hit_hist[static_cast<size_t>(t)]});
  }
  for (TH1D *h : {hFirst, hLast}) {
    h->SetDirectory(nullptr);
    h->SetStats(kFALSE);
    h->SetLineWidth(2);
    h->SetBit(kCanDelete);
  }
  hFirst->SetLineColor(kBlue + 1);
  hLast->SetLineColor(kRed + 1);
  hFirst->SetMinimum(0.5);  // 対数 y で 0 の bin を描かない
  hFirst->SetMaximum(2.0 * maxCount);
  hFirst->Draw("HIST");
  hLast->Draw("HIST SAME");
  TLegend *legend = new TLegend(0.70, 0.78, 0.89, 0.89);
  legend->SetBit(kCanDelete);
  legend->AddEntry(hFirst, "first hit cell", "l");
  legend->AddEntry(hLast, "last hit cell", "l");
  legend->Draw();
  c.SaveAs(path.c_str());
  FileStat_t st;
  return gSystem->GetPathInfo(path.c_str(), st) == 0 && st.fSize > 0;
}

}  // namespace driftend

// 入口。0 = JSON(と PNG)を書いた、1 = 失敗(理由は stderr)。maxEvents ≤ 0 = 全イベント。
int drift_end(const char *rawFile, const char *outJson, const char *outPng = nullptr,
              int maxEvents = 0)
{
  TStopwatch sw;
  driftend::Result r;
  std::string err;
  if (!driftend::computeDriftEnd(rawFile, maxEvents, r, err)) {
    std::fprintf(stderr, "drift_end: ERROR %s\n", err.c_str());
    return 1;
  }
  const std::string json = driftend::toJson(r);
  FILE *fp = std::fopen(outJson, "w");
  const bool written = fp && std::fputs(json.c_str(), fp) >= 0;
  const bool closed = fp && std::fclose(fp) == 0;
  if (!written || !closed) {
    std::fprintf(stderr, "drift_end: ERROR cannot write %s\n", outJson);
    return 1;
  }
  if (outPng && *outPng && !driftend::drawPng(r, outPng)) {
    std::fprintf(stderr, "drift_end: ERROR cannot write %s\n", outPng);
    return 1;
  }
  // 黙らないこと(CLAUDE.md): 引けなかった common mode、ヒット無し、端が窓で切れている
  if (r.agets_without_fpn > 0)
    std::fprintf(stderr,
                 "drift_end: WARNING %ld (event, AGET) had signal channels but no FPN channel "
                 "read — common mode not subtracted there (isFPNRead=false?)\n",
                 r.agets_without_fpn);
  if (r.events_with_hits == 0)
    std::fprintf(stderr,
                 "drift_end: WARNING no event has a cell above %.0f ADC — metrics are null\n",
                 runmetrics::kAdcThreshold);
  if (r.edge.cell_p99 >= driftend::kClipBackMinCell)
    std::fprintf(
        stderr,
        "drift_end: WARNING edge.cell_p99 = %.2f is at the readout window end (clip_back_frac "
        "%.3f) — the drift end is not seen\n",
        r.edge.cell_p99, r.clip_back_frac);
  std::printf(
      "drift_end: %ld events, %ld with hits, edge.cell_p99 %.2f, t_max %.2f cells, %.1f s -> %s\n",
      r.events, r.events_with_hits, r.edge.cell_p99, r.edge.t_max_cells, sw.RealTime(), outJson);
  return 0;
}
