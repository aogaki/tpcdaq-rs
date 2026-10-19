// =============================================================================
// gas_trend.C — run 1 本の指標を時間ビンごとに出す(TODO/161、ガス流量 100 → 50 ml/s の比較)。
//
// raw ツリー(SPEC §6.7: eventTime/l = 48-bit CoBo 10 ns tick、adc[N][68][512]/S)を 1 回走査し、
// run_metrics.C と同じ定義(ped = cells 1..20 − FPN 共通モード、閾値 35 ADC、ピーク ±5 の
// q ≥ 2000 = ヒット ch、電荷 = ヒット ch の閾値超サンプルの和)でイベントごとの量を取り、
// binSeconds ごとに中央値・分位をまとめる。macros 不要。
//
//   root -l -b -q 'gas_trend.C+("run0175_raw.root", "out/run0175_trend.json", 3600)'
//
// 出力: JSON(bins[] = {t_h, events, rate_hz, charge_med, charge_q16, charge_q84, hit_med,
//        sat_frac, first_med, last_med, width_med, tcent_med})と同名 .png(4 段)。
// =============================================================================
#include <TCanvas.h>
#include <TFile.h>
#include <TGraphAsymmErrors.h>
#include <TGraph.h>
#include <TLeaf.h>
#include <TStyle.h>
#include <TTree.h>

#include <algorithm>
#include <climits>
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

namespace gastrend {
const double kAdcThreshold = 35.0;
const int kPedLo = 1, kPedHi = 20;
const int kPeakHalfCells = 5;
const double kStripQMin = 2000.0;
const int kFpn[4] = {11, 22, 45, 56};
const int kAdcMax = 4095;
const int kChannels = 68, kCells = 512;
const double kTickSeconds = 1e-8;

bool isFpn(int c) { return c == 11 || c == 22 || c == 45 || c == 56; }
bool allZero(const Short_t *w) {
  for (int t = 0; t < kCells; ++t)
    if (w[t] != 0) return false;
  return true;
}
double quantile(std::vector<double> v, double q) {
  if (v.empty()) return std::nan("");
  std::sort(v.begin(), v.end());
  const size_t i = std::min(v.size() - 1, static_cast<size_t>(q * v.size()));
  return v[i];
}

struct Bin {
  std::vector<double> charge, hit, first, last, width, tcent;
  long events = 0, saturated = 0;
  double tMin = 1e300, tMax = -1e300;  // ビン内の実スパン(端の半端なビンのレート用)
};
}  // namespace gastrend

int gas_trend(const char *rawFile, const char *outJson, double binSeconds = 3600.0,
              Long64_t maxEvents = 0) {
  using namespace gastrend;
  TFile f(rawFile, "READ");
  if (f.IsZombie()) { std::printf("cannot open %s\n", rawFile); return 1; }
  TTree *tree = f.Get<TTree>("raw");
  if (!tree) { std::printf("no 'raw' tree in %s\n", rawFile); return 1; }
  TLeaf *leaf = tree->GetLeaf("adc");
  if (!leaf || leaf->GetLen() % (kChannels * kCells) != 0) {
    std::printf("adc leaf is not [N][68][512]\n");
    return 1;
  }
  const int nBlocks = leaf->GetLen() / (kChannels * kCells);
  std::vector<Short_t> adc(static_cast<size_t>(leaf->GetLen()));
  ULong64_t eventTime = 0;
  tree->SetBranchAddress("adc", adc.data());
  tree->SetBranchAddress("eventTime", &eventTime);

  Long64_t n = tree->GetEntries();
  if (maxEvents > 0 && maxEvents < n) n = maxEvents;
  std::vector<Bin> bins;
  std::vector<double> fpn(kCells), v(kCells);
  double offset = 0, firstT = -1, prevT = -1;
  for (Long64_t i = 0; i < n; ++i) {
    tree->GetEntry(i);
    // 48-bit tick → 秒(戻ったら前の値を足して継ぐ: run 中のリセット対策)
    double raw = static_cast<double>(eventTime);
    if (prevT >= 0 && raw + 1e9 < prevT) offset += prevT;
    prevT = raw;
    const double tSec = (raw + offset) * kTickSeconds;
    if (firstT < 0) firstT = tSec;
    const size_t b = static_cast<size_t>((tSec - firstT) / binSeconds);
    if (b >= bins.size()) bins.resize(b + 1);
    Bin &bin = bins[b];
    ++bin.events;
    bin.tMin = std::min(bin.tMin, tSec);
    bin.tMax = std::max(bin.tMax, tSec);

    int first = INT_MAX, last = -1, nHit = 0;
    double charge = 0, wt = 0;
    bool saturated = false;
    std::vector<int> widths;
    for (int blk = 0; blk < nBlocks; ++blk) {
      const Short_t *aget = &adc[static_cast<size_t>(blk) * kChannels * kCells];
      std::fill(fpn.begin(), fpn.end(), 0.0);
      int nFpn = 0;
      for (int fc : kFpn) {
        const Short_t *w = aget + fc * kCells;
        if (allZero(w)) continue;
        ++nFpn;
        for (int t = 0; t < kCells; ++t) fpn[t] += w[t];
      }
      if (nFpn > 0)
        for (double &x : fpn) x /= nFpn;
      for (int c = 0; c < kChannels; ++c) {
        if (isFpn(c)) continue;
        const Short_t *w = aget + c * kCells;
        if (allZero(w)) continue;
        double ped = 0;
        for (int t = kPedLo; t <= kPedHi; ++t) ped += w[t] - fpn[t];
        ped /= (kPedHi - kPedLo + 1);
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
        if (q < kStripQMin) continue;
        ++nHit;
        first = std::min(first, peak);
        last = std::max(last, peak);
        int runLen = 0, longest = 0;
        for (int t = 0; t < kCells; ++t) {
          if (v[t] <= kAdcThreshold) { runLen = 0; continue; }
          longest = std::max(longest, ++runLen);
          charge += v[t];
          wt += v[t] * t;
          if (w[t] == kAdcMax) saturated = true;
        }
        widths.push_back(longest);
      }
    }
    bin.charge.push_back(charge);
    bin.hit.push_back(nHit);
    if (saturated) ++bin.saturated;
    if (nHit > 0) {
      bin.first.push_back(first);
      bin.last.push_back(last);
      bin.width.push_back(quantile(std::vector<double>(widths.begin(), widths.end()), 0.5));
      if (charge > 0) bin.tcent.push_back(wt / charge);
    }
    if (i % 100000 == 0) std::printf("  %lld / %lld\n", i, n);
  }

  // ---- JSON --------------------------------------------------------------------
  FILE *out = std::fopen(outJson, "w");
  if (!out) { std::printf("cannot write %s\n", outJson); return 1; }
  std::fprintf(out, "{\"raw_file\":\"%s\",\"events\":%lld,\"bin_s\":%g,\"bins\":[\n", rawFile, n,
               binSeconds);
  std::vector<double> gx, gq, gqlo, gqhi, gh, gs, gr;
  for (size_t b = 0; b < bins.size(); ++b) {
    const Bin &bin = bins[b];
    const double th = (b + 0.5) * binSeconds / 3600.0;
    const double qm = quantile(bin.charge, 0.5), q16 = quantile(bin.charge, 0.16),
                 q84 = quantile(bin.charge, 0.84), hm = quantile(bin.hit, 0.5);
    const double sat = bin.events > 0 ? static_cast<double>(bin.saturated) / bin.events : 0;
    const double span = bin.tMax - bin.tMin;
    const double rate = (bin.events > 1 && span > 0) ? (bin.events - 1) / span : 0;
    std::fprintf(out,
                 "  {\"t_h\":%.3f,\"events\":%ld,\"rate_hz\":%.4f,\"charge_med\":%.0f,"
                 "\"charge_q16\":%.0f,\"charge_q84\":%.0f,\"hit_med\":%.1f,\"sat_frac\":%.5f,"
                 "\"first_med\":%.1f,\"last_med\":%.1f,\"width_med\":%.1f,\"tcent_med\":%.1f}%s\n",
                 th, bin.events, rate, qm, q16, q84, hm, sat, quantile(bin.first, 0.5),
                 quantile(bin.last, 0.5), quantile(bin.width, 0.5), quantile(bin.tcent, 0.5),
                 b + 1 < bins.size() ? "," : "");
    if (bin.events < 10) continue;  // 端の半端なビンは描かない
    gx.push_back(th); gq.push_back(qm); gqlo.push_back(qm - q16); gqhi.push_back(q84 - qm);
    gh.push_back(hm); gs.push_back(sat * 100); gr.push_back(rate);
  }
  std::fprintf(out, "]}\n");
  std::fclose(out);

  // ---- PNG ---------------------------------------------------------------------
  gStyle->SetOptStat(0);
  std::string png(outJson);
  const size_t dot = png.rfind('.');
  png = (dot == std::string::npos ? png : png.substr(0, dot)) + ".png";
  TCanvas c("c", "gas trend", 1000, 1100);
  c.Divide(1, 4);
  TGraphAsymmErrors gQ(gx.size(), gx.data(), gq.data(), nullptr, nullptr, gqlo.data(), gqhi.data());
  TGraph gH(gx.size(), gx.data(), gh.data());
  TGraph gS(gx.size(), gx.data(), gs.data());
  TGraph gR(gx.size(), gx.data(), gr.data());
  gQ.SetTitle("event charge: median and 16-84 % band;hours since run start;charge [ADC]");
  gH.SetTitle("hit channels (median);hours since run start;channels");
  gS.SetTitle("events with a saturated sample;hours since run start;%");
  gR.SetTitle("trigger rate;hours since run start;Hz");
  gQ.SetMarkerStyle(20); gH.SetMarkerStyle(20); gS.SetMarkerStyle(20); gR.SetMarkerStyle(20);
  c.cd(1); gQ.Draw("AP");
  c.cd(2); gH.Draw("AP");
  c.cd(3); gS.Draw("AP");
  c.cd(4); gR.Draw("AP");
  c.SaveAs(png.c_str());
  std::printf("wrote %s and %s (%zu bins, %lld events)\n", outJson, png.c_str(), bins.size(), n);
  return 0;
}
