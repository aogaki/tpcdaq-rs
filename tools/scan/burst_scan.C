// =============================================================================
// burst_scan.C — 共通モードのオシレーション(バースト)が信号と独立かを調べる(TODO/174)。
//
//   root -l -b -q 'burst_scan.C+("run0180_raw.root", "out/run0180", 12.5)'
//
// 引数: rawFile、outPrefix、samplingMHz(必須 — µs への換算。既定値なし、169 の方針)、
//       maxEvents(-1 = 全部)、kThr(既定 5.0)、minAmpPP(既定 50 ADC)。戻り値 0 = 書いた、1 = 失敗(理由は stderr)。
//
// 入力: raw ツリー(SPEC §6.7)の eventId / eventTime(10 ns 刻み、そのまま記録)/
//       adc[N][68][512]/S(N はリーフの長さから求める。全セル 0 の ch = 未読は除く)。
// FPN = 各 AGET の生 ch 11, 22, 45, 56(src/geometry.rs FPN_RAW_CHANNELS)、信号 ch = それ以外。
//
// 事象ごと:
//   1. ch ごとの基線 = 512 セルの中央値。以後は基線を引いた値
//   2. cmF[b] = 読めた FPN ch のセル b の中央値、cmS[b] = 信号 ch のセル b の中央値
//   3. 検出は cmF(中央値を引いたもの)で: 窓幅 W = round(2.0 µs × samplingMHz) の移動 RMS
//      (窓の開始 st = 0..512−W、中心 = st + W/2)の最大値。σ_ref = 1.4826 × MAD(cmF 全セル)。
//      最大の移動 RMS > kThr × σ_ref、かつ(TODO/174 §3a で追加、AND)
//        ・その窓の cmF の peak-to-peak ≥ minAmpPP(信号連動の小さな段差を除く)
//        ・cmF(最大窓を前後 W/2 広げた区間の平均を引いた後)のゼロ交差が 3 回以上(振動であり周期が測れる)
//      ならバーストあり。5σ だけ超えて上の 2 条件で落ちた事象は events_over_kthr_only に数える
//   4. 位置 = 最大の窓の中心バケット、振幅 = その窓の cmF の peak-to-peak、長さ = 移動 RMS が
//      閾値を超えた窓の数、周期 = その窓の cmF のゼロ交差(隣のセルとの線形補間)の
//      平均間隔 × 2 [µs](交差 3 個未満はバーストとしない)、cmSigAmpPP = 同じ窓の cmS の peak-to-peak
//   5. sigPeak = 信号 ch の(基線を引いた)値の ch 和の最大バケット、sigCharge = その総和
//      (共通モードは引かない — 発注書どおり。全 ch に乗るバーストは ch 和に入る)
// 出力: <outPrefix>_burst.root(tree burst)/ _burst.json / _burst.png(2x2)。
// 分位点は線形補間 h = (n−1)·q(run_metrics.C と同じ)。値が無ければ JSON は null。
// =============================================================================
#include <TCanvas.h>
#include <TFile.h>
#include <TGraph.h>
#include <TH1D.h>
#include <TLeaf.h>
#include <TMath.h>
#include <TStyle.h>
#include <TTree.h>

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <limits>
#include <string>
#include <vector>

namespace burstscan {

const int kChannels = 68;  // SPEC §6.7: adc[..][68][512]
const int kCells = 512;
const int kFpn[4] = {11, 22, 45, 56};  // src/geometry.rs FPN_RAW_CHANNELS
const double kWindowUs = 2.0;          // 発注書 §1-A-3
const double kTickSeconds = 1e-8;      // eventTime の刻み(10 ns)
const double kMadToSigma = 1.4826;
const double kNaN = std::numeric_limits<double>::quiet_NaN();

bool isFpn(int ch) { return ch == kFpn[0] || ch == kFpn[1] || ch == kFpn[2] || ch == kFpn[3]; }

double quantile(std::vector<double> v, double q)
{
  if (v.empty()) return kNaN;
  std::sort(v.begin(), v.end());
  const double h = (v.size() - 1) * q;
  const size_t lo = static_cast<size_t>(std::floor(h));
  const size_t hi = std::min(lo + 1, v.size() - 1);
  return v[lo] + (h - lo) * (v[hi] - v[lo]);
}

// 破壊的(並べ替える)中央値。空なら 0。
double medianInPlace(std::vector<double> &v)
{
  if (v.empty()) return 0.0;
  return quantile(v, 0.5);
}

// "…/run0077_raw.root" → 77。形が違えば -1。
long runFromName(const std::string &path)
{
  const size_t slash = path.find_last_of('/');
  const std::string base = (slash == std::string::npos) ? path : path.substr(slash + 1);
  const std::string prefix = "run", suffix = "_raw.root";
  if (base.size() <= prefix.size() + suffix.size()) return -1;
  if (base.compare(0, prefix.size(), prefix) != 0) return -1;
  if (base.compare(base.size() - suffix.size(), suffix.size(), suffix) != 0) return -1;
  const std::string digits = base.substr(prefix.size(), base.size() - prefix.size() - suffix.size());
  for (char ch : digits)
    if (ch < '0' || ch > '9') return -1;
  return std::atol(digits.c_str());
}

struct EventResult {
  bool hasBurst = false;
  int burstBucket = -1;
  double ampPP = kNaN, periodUs = kNaN, cmSigAmpPP = kNaN, sigmaRef = kNaN;
  int len = 0;
  int sigPeak = -1;
  double sigCharge = 0.0;
  bool noFpn = false;
  bool overKthrOnly = false;  // 5σ は超えたが振幅下限かゼロ交差数で落ちた
};

const double kMinCrossings = 3;  // TODO/174 §3a

double peakToPeak(const std::vector<double> &x, int st, int w)
{
  double lo = x[st], hi = x[st];
  for (int i = st; i < st + w; ++i) {
    lo = std::min(lo, x[i]);
    hi = std::max(hi, x[i]);
  }
  return hi - lo;
}

// 区間 [st, st+w) の x(demean なら区間内の平均を引いた後、そうでなければ 0 基準)のゼロ交差位置(隣のセルとの線形補間)。
std::vector<double> crossings(const std::vector<double> &x, int st, int w, bool demean)
{
  double mean = 0;
  if (demean) {
    for (int i = st; i < st + w; ++i) mean += x[i];
    mean /= w;
  }
  std::vector<double> cross;
  for (int i = st; i + 1 < st + w; ++i) {
    const double a = x[i] - mean, b = x[i + 1] - mean;
    if ((a < 0 && b >= 0) || (a >= 0 && b < 0)) cross.push_back(i + a / (a - b));
  }
  return cross;
}

// 1 事象。adc は [nBlocks][68][512]。work は呼び出し側の使い回しバッファ。
EventResult analyzeEvent(const Short_t *adc, int nBlocks, double samplingMHz, double kThr, double minAmpPP)
{
  EventResult r;
  const int W = static_cast<int>(std::lround(kWindowUs * samplingMHz));
  std::vector<std::vector<double>> fpnRows, sigRows;  // 基線を引いた ch ごとの波形
  std::vector<double> tmp(kCells);
  for (int blk = 0; blk < nBlocks; ++blk) {
    for (int ch = 0; ch < kChannels; ++ch) {
      const Short_t *w = adc + (static_cast<size_t>(blk) * kChannels + ch) * kCells;
      bool allZero = true;
      for (int c = 0; c < kCells; ++c)
        if (w[c] != 0) { allZero = false; break; }
      if (allZero) continue;  // 部分読み出しの未読 ch
      for (int c = 0; c < kCells; ++c) tmp[c] = w[c];
      const double base = medianInPlace(tmp);
      std::vector<double> row(kCells);
      for (int c = 0; c < kCells; ++c) row[c] = w[c] - base;
      (isFpn(ch) ? fpnRows : sigRows).push_back(std::move(row));
    }
  }
  // 信号: ch 和
  if (!sigRows.empty()) {
    double best = -std::numeric_limits<double>::infinity();
    for (int c = 0; c < kCells; ++c) {
      double s = 0;
      for (const auto &row : sigRows) s += row[c];
      r.sigCharge += s;
      if (s > best) { best = s; r.sigPeak = c; }
    }
  }
  if (fpnRows.empty() || W < 2 || W > kCells) {
    r.noFpn = fpnRows.empty();
    return r;
  }
  auto cellMedians = [&](const std::vector<std::vector<double>> &rows) {
    std::vector<double> cm(kCells), col(rows.size());
    for (int c = 0; c < kCells; ++c) {
      for (size_t i = 0; i < rows.size(); ++i) col[i] = rows[i][c];
      cm[c] = medianInPlace(col);
    }
    return cm;
  };
  std::vector<double> cmF = cellMedians(fpnRows);
  std::vector<double> cmS = sigRows.empty() ? std::vector<double>(kCells, 0.0) : cellMedians(sigRows);

  // σ_ref = 1.4826 × MAD、以後 cmF は中央値を引いたもので扱う
  std::vector<double> t = cmF;
  const double med = medianInPlace(t);
  for (double &x : cmF) x -= med;
  std::vector<double> dev(kCells);
  for (int c = 0; c < kCells; ++c) dev[c] = std::fabs(cmF[c]);
  r.sigmaRef = kMadToSigma * medianInPlace(dev);

  // 移動 RMS(累積和)
  std::vector<double> cum(kCells + 1, 0.0);
  for (int c = 0; c < kCells; ++c) cum[c + 1] = cum[c] + cmF[c] * cmF[c];
  const int nWin = kCells - W + 1;
  std::vector<double> mrms(nWin);
  int bestSt = 0;
  for (int st = 0; st < nWin; ++st) {
    mrms[st] = std::sqrt(std::max(0.0, (cum[st + W] - cum[st]) / W));
    if (mrms[st] > mrms[bestSt]) bestSt = st;
  }
  if (!(r.sigmaRef > 0.0)) return r;  // 基準が 0 = 判定不能(呼び出し側が数えて WARNING)
  const double thr = kThr * r.sigmaRef;
  if (mrms[bestSt] <= thr) return r;
  const double ampPP = peakToPeak(cmF, bestSt, W);
  // ゼロ交差は最大窓を前後 W/2 広げた区間(最大 2W)で数える。発注書は「窓内」だが、W = 2 µs で周期
  // 1.5 µs の振動は窓内に交差が 2〜3 個しか入らず、≥ 3 では本物のバーストを落とす(報告事項)
  const int cst = std::max(0, bestSt - W / 2);
  const int cen = std::min(kCells, bestSt + W + W / 2);
  const std::vector<double> cross = crossings(cmF, cst, cen - cst, true);
  if (ampPP < minAmpPP || static_cast<double>(cross.size()) < kMinCrossings) {
    r.overKthrOnly = true;
    return r;
  }
  r.hasBurst = true;
  r.burstBucket = bestSt + W / 2;
  r.ampPP = ampPP;
  r.cmSigAmpPP = peakToPeak(cmS, bestSt, W);
  for (int st = 0; st < nWin; ++st)
    if (mrms[st] > thr) ++r.len;
  // 周期 = 最大窓(W)内の cmF(中央値基準のまま、初版と同じ)のゼロ交差の平均間隔 × 2 [bucket](交差 2 個
  // 未満なら NaN)。広げた区間の末尾は減衰して雑音が支配し、窓内平均を引くと交差が偏るので周期には使わない
  const std::vector<double> crossW = crossings(cmF, bestSt, W, false);
  if (crossW.size() >= 2) {
    const double pb = 2.0 * (crossW.back() - crossW.front()) / (crossW.size() - 1);
    r.periodUs = pb / samplingMHz;
  }
  return r;
}

// 一様分布 [lo, hi] との KS 検定の p 値(漸近式、Stephens の補正)。n < 5 なら NaN。
double ksUniformP(std::vector<double> x, double lo, double hi)
{
  const size_t n = x.size();
  if (n < 5 || !(hi > lo)) return kNaN;
  std::sort(x.begin(), x.end());
  double d = 0;
  for (size_t i = 0; i < n; ++i) {
    const double F = std::min(1.0, std::max(0.0, (x[i] - lo) / (hi - lo)));
    d = std::max(d, std::max(F - static_cast<double>(i) / n, static_cast<double>(i + 1) / n - F));
  }
  const double sn = std::sqrt(static_cast<double>(n));
  return TMath::KolmogorovProb(d * (sn + 0.12 + 0.11 / sn));
}

std::string num(double v, const char *fmt = "%.6g")
{
  if (!std::isfinite(v)) return "null";
  char b[64];
  std::snprintf(b, sizeof b, fmt, v);
  return b;
}

std::string quantTriple(const std::vector<double> &v)
{
  return "{\"median\":" + num(quantile(v, 0.5)) + ",\"p16\":" + num(quantile(v, 0.16)) + ",\"p84\":" +
         num(quantile(v, 0.84)) + ",\"n\":" + std::to_string(v.size()) + "}";
}

}  // namespace burstscan

int burst_scan(const char *rawFile, const char *outPrefix, double samplingMHz, Long64_t maxEvents = -1,
               double kThr = 5.0, double minAmpPP = 50.0)
{
  using namespace burstscan;
  if (!(samplingMHz > 0)) {
    std::fprintf(stderr, "burst_scan: samplingMHz must be > 0\n");
    return 1;
  }
  TFile f(rawFile, "READ");
  if (f.IsZombie()) { std::fprintf(stderr, "burst_scan: cannot open %s\n", rawFile); return 1; }
  TTree *tree = f.Get<TTree>("raw");
  if (!tree) { std::fprintf(stderr, "burst_scan: no 'raw' tree in %s\n", rawFile); return 1; }
  TLeaf *leaf = tree->GetLeaf("adc");
  if (!leaf || leaf->GetLen() % (kChannels * kCells) != 0 || leaf->GetLen() == 0) {
    std::fprintf(stderr, "burst_scan: adc leaf is not [N][68][512]\n");
    return 1;
  }
  const int nBlocks = leaf->GetLen() / (kChannels * kCells);
  std::vector<Short_t> adc(static_cast<size_t>(leaf->GetLen()));
  UInt_t eventId = 0;
  ULong64_t eventTime = 0;
  if (tree->SetBranchAddress("adc", adc.data()) < 0 || tree->SetBranchAddress("eventId", &eventId) < 0 ||
      tree->SetBranchAddress("eventTime", &eventTime) < 0) {
    std::fprintf(stderr, "burst_scan: raw tree lacks adc / eventId / eventTime\n");
    return 1;
  }
  Long64_t n = tree->GetEntries();
  if (maxEvents > 0 && maxEvents < n) n = maxEvents;
  const int W = static_cast<int>(std::lround(kWindowUs * samplingMHz));
  if (W < 2 || W > kCells) {
    std::fprintf(stderr, "burst_scan: window W = %d buckets is out of range\n", W);
    return 1;
  }
  const std::string prefix(outPrefix);
  const long run = runFromName(rawFile);

  TFile fo((prefix + "_burst.root").c_str(), "RECREATE");
  if (fo.IsZombie()) { std::fprintf(stderr, "burst_scan: cannot write %s_burst.root\n", outPrefix); return 1; }
  TTree bt("burst", "burst");
  Int_t b_run = static_cast<Int_t>(run), b_has = 0, b_bucket = -1, b_len = 0, b_sigPeak = -1;
  UInt_t b_eid = 0;
  ULong64_t b_time = 0;
  Double_t b_amp = 0, b_per = 0, b_cmS = 0, b_sigCharge = 0, b_sigma = 0;
  bt.Branch("run", &b_run, "run/I");
  bt.Branch("eventId", &b_eid, "eventId/i");
  bt.Branch("eventTime", &b_time, "eventTime/l");
  bt.Branch("hasBurst", &b_has, "hasBurst/I");
  bt.Branch("burstBucket", &b_bucket, "burstBucket/I");
  bt.Branch("burstAmpPP", &b_amp, "burstAmpPP/D");
  bt.Branch("burstLen", &b_len, "burstLen/I");
  bt.Branch("burstPeriodUs", &b_per, "burstPeriodUs/D");
  bt.Branch("cmSigAmpPP", &b_cmS, "cmSigAmpPP/D");
  bt.Branch("sigPeak", &b_sigPeak, "sigPeak/I");
  bt.Branch("sigCharge", &b_sigCharge, "sigCharge/D");
  bt.Branch("sigmaRef", &b_sigma, "sigmaRef/D");

  std::vector<double> buckets, periods, amps, ratios, dsig, burstT, allT;
  std::vector<char> hasV;
  long nBurst = 0, nNoFpn = 0, nSigmaZero = 0, nOverOnly = 0;
  ULong64_t t0 = 0;
  for (Long64_t i = 0; i < n; ++i) {
    tree->GetEntry(i);
    if (i == 0) t0 = eventTime;
    const EventResult r = analyzeEvent(adc.data(), nBlocks, samplingMHz, kThr, minAmpPP);
    if (r.noFpn) ++nNoFpn;
    if (r.overKthrOnly) ++nOverOnly;
    if (!r.noFpn && r.sigmaRef == 0.0) ++nSigmaZero;
    b_eid = eventId;
    b_time = eventTime;
    b_has = r.hasBurst;
    b_bucket = r.burstBucket;
    b_amp = r.ampPP;
    b_len = r.len;
    b_per = r.periodUs;
    b_cmS = r.cmSigAmpPP;
    b_sigPeak = r.sigPeak;
    b_sigCharge = r.sigCharge;
    b_sigma = r.sigmaRef;
    bt.Fill();
    const double ts = (static_cast<double>(eventTime) - static_cast<double>(t0)) * kTickSeconds;
    allT.push_back(ts);
    hasV.push_back(r.hasBurst);
    if (r.hasBurst) {
      ++nBurst;
      buckets.push_back(r.burstBucket);
      if (std::isfinite(r.periodUs)) periods.push_back(r.periodUs);
      amps.push_back(r.ampPP);
      if (r.cmSigAmpPP > 0) ratios.push_back(r.ampPP / r.cmSigAmpPP);  // FPN / 信号
      if (r.sigPeak >= 0) dsig.push_back(r.burstBucket - r.sigPeak);
      burstT.push_back(ts);
    }
  }
  bt.Write();
  fo.Close();
  if (nNoFpn > 0)
    std::fprintf(stderr, "burst_scan: WARNING %ld events have no readable FPN ch — no burst judgement\n", nNoFpn);
  if (nSigmaZero > 0)
    std::fprintf(stderr, "burst_scan: WARNING %ld events have sigma_ref = 0 (cmF flat) — no burst judgement\n",
                 nSigmaZero);

  const double lo = 0.5 * W, hi = kCells - 0.5 * W;  // 発注書 §1-A: 一様分布の区間 [W/2, 512 − W/2]
  const double ksP = ksUniformP(buckets, lo, hi);
  const double frac = n > 0 ? static_cast<double>(nBurst) / n : kNaN;
  // burstBucket ヒスト(32 区間 × 16 bucket)
  std::string histJson = "[";
  for (int k = 0; k < 32; ++k) {
    long c = 0;
    for (double b : buckets)
      if (b >= 16 * k && b < 16 * (k + 1)) ++c;
    histJson += (k ? "," : "") + std::to_string(c);
  }
  histJson += "]";
  std::string js = "{\"raw_file\":\"" + std::string(rawFile) + "\",\"run\":" + (run < 0 ? "null" : std::to_string(run)) +
                   ",\"sampling_mhz\":" + num(samplingMHz) + ",\"window_buckets\":" + std::to_string(W) +
                   ",\"k_thr\":" + num(kThr) + ",\"min_amp_pp\":" + num(minAmpPP) +
                   ",\"events\":" + std::to_string(n) + ",\"events_over_kthr_only\":" + std::to_string(nOverOnly) +
                   ",\"events_with_burst\":" + std::to_string(nBurst) + ",\"burst_fraction\":" + num(frac) +
                   ",\"events_without_fpn\":" + std::to_string(nNoFpn) + ",\"events_sigma_ref_zero\":" +
                   std::to_string(nSigmaZero) + ",\"burst_bucket_hist_16\":" + histJson +
                   ",\"burst_bucket_range\":[" + num(lo) + "," + num(hi) + "],\"ks_p\":" + num(ksP) +
                   ",\"period_us\":" + quantTriple(periods) + ",\"amp_pp_adc\":" + quantTriple(amps) +
                   ",\"fpn_over_signal_amp\":" + quantTriple(ratios) + ",\"burst_minus_sigpeak\":" +
                   quantTriple(dsig) + "}\n";
  FILE *fj = std::fopen((prefix + "_burst.json").c_str(), "w");
  if (!fj) { std::fprintf(stderr, "burst_scan: cannot write %s_burst.json\n", outPrefix); return 1; }
  std::fputs(js.c_str(), fj);
  std::fclose(fj);

  // ---- PNG(2x2)----
  gStyle->SetOptStat(0);
  TH1D h1("h1", "burst position;burst bucket;events", 64, 0, kCells);
  TH1D h2("h2", "burst bucket - signal peak bucket;buckets;events", 64, -kCells, kCells);
  TH1D h3("h3", "burst period;period [us];events", 40, 0, 4);
  for (double b : buckets) h1.Fill(b);
  for (double d : dsig) h2.Fill(d);
  for (double p : periods) h3.Fill(p);
  const int nbt = 20;
  double tMax = 0;
  for (double t : allT) tMax = std::max(tMax, t);
  std::vector<double> gx, gy;
  if (tMax > 0) {
    std::vector<long> tot(nbt, 0), bur(nbt, 0);
    for (size_t i = 0; i < allT.size(); ++i) {
      const int k = std::min(nbt - 1, static_cast<int>(allT[i] / tMax * nbt));
      ++tot[k];
      if (hasV[i]) ++bur[k];
    }
    for (int k = 0; k < nbt; ++k)
      if (tot[k] > 0) {
        gx.push_back((k + 0.5) * tMax / nbt / 3600.0);
        gy.push_back(100.0 * bur[k] / tot[k]);
      }
  }
  TCanvas c("c", "burst scan", 1100, 900);
  c.Divide(2, 2);
  c.cd(1); h1.Draw();
  c.cd(2); h2.Draw();
  c.cd(3); h3.Draw();
  c.cd(4);
  TGraph g(static_cast<int>(gx.size()), gx.data(), gy.data());
  g.SetTitle("events with burst vs time;hours since first event;%");
  g.SetMarkerStyle(20);
  if (!gx.empty()) g.Draw("APL");
  c.SaveAs((prefix + "_burst.png").c_str());

  std::printf("burst_scan: %lld events, %.1f %% with burst, KS p = %s, period median %s us\n", n,
              std::isfinite(frac) ? 100.0 * frac : 0.0, num(ksP, "%.3g").c_str(),
              num(quantile(periods, 0.5), "%.3f").c_str());
  return 0;
}
