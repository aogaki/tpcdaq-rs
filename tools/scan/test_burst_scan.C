// test_burst_scan.C — burst_scan.C の合成テスト(TODO/174)。終了コード = 失敗数。
//   root -l -b -q tools/scan/test_burst_scan.C
// 合成 raw: adc[4][68][512]/S、基線 ch ごとに 300–600、ガウス雑音 σ = 8 ADC、乱数の種は固定。
//   (a) 信号だけ: 信号 ch 数本(1..20)に幅 10 cell のパルス(高さ 500–1500)。FPN は雑音だけ
//   (b) バーストだけ: 全 ch(FPN 込み)に減衰正弦 A = 150、周期 1.5 µs、τ = 3 µs、開始 s は [0, 512−W] 一様
//   (c) 信号 + バースト(位置は独立)  (d) 雑音だけ
//   (e) 信号 + FPN 全 ch に +12 ADC の段差(信号と同時刻、σ = 2)→ 偽陽性 ≤ 2 %(TODO/174 §3a)
// 期待(発注書 §1-B): 偽陽性 ≤ 2 %、検出 ≥ 95 %、位置 ±1 周期(真値 = 開始バケット s)、
//   周期 ±10 %、peak-to-peak ±25 %(真値 2A = 300)、12.5 / 25 MHz 両方で周期が µs で正しい、
//   一様開始の KS p > 0.01、固定開始(100 ± 2)の KS p < 1e-6。
#include <TFile.h>
#include <TRandom3.h>
#include <TROOT.h>
#include <TSystem.h>
#include <TTree.h>

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <fstream>
#include <sstream>
#include <string>
#include <vector>

namespace {
int fails = 0;
void check(bool ok, const std::string &what)
{
  std::printf("%s %s\n", ok ? "ok  " : "FAIL", what.c_str());
  if (!ok) ++fails;
}

const int kNa = 4, kNch = 68, kNc = 512;
const double kAmp = 150.0, kPeriodUs = 1.5, kTauUs = 3.0, kSigma = 8.0;
const int kFpnCh[4] = {11, 22, 45, 56};

enum Kind { kSignal, kBurst, kBoth, kNoise, kStep };

struct Truth {
  int start = -1;      // バースト開始バケット(無ければ -1)
  int sigBucket = -1;  // 信号パルスの中心バケット(無ければ -1)
};

// startMode: 0 = 一様 [0, 512−W]、1 = 固定 100 ± 2
std::vector<Truth> writeRaw(const std::string &path, Kind kind, int nEv, double mhz, int startMode, UInt_t seed)
{
  TRandom3 rng(seed);
  const int W = static_cast<int>(std::lround(2.0 * mhz));
  TFile f(path.c_str(), "RECREATE");
  TTree t("raw", "raw");
  UInt_t eventId = 0;
  ULong64_t eventTime = 0;
  std::vector<Short_t> adc(kNa * kNch * kNc);
  t.Branch("eventId", &eventId, "eventId/i");
  t.Branch("eventTime", &eventTime, "eventTime/l");
  t.Branch("adc", adc.data(), "adc[4][68][512]/S");
  std::vector<double> base(kNa * kNch);
  for (double &b : base) b = rng.Uniform(300, 600);
  std::vector<Truth> truth;
  std::vector<double> row(kNc);
  for (int ev = 0; ev < nEv; ++ev) {
    Truth tr;
    std::vector<double> burst(kNc, 0.0);
    const bool withBurst = (kind == kBurst || kind == kBoth);
    if (withBurst) {
      tr.start = startMode == 0 ? static_cast<int>(rng.Uniform(0, kNc - W + 1)) : 98 + static_cast<int>(rng.Uniform(0, 5));
      for (int c = tr.start; c < kNc; ++c) {
        const double us = (c - tr.start) / mhz;
        burst[c] = kAmp * std::exp(-us / kTauUs) * std::sin(2.0 * M_PI * us / kPeriodUs);
      }
    }
    std::vector<double> pulse(kNc, 0.0);
    std::vector<int> sigChs;
    const bool withSignal = (kind == kSignal || kind == kBoth || kind == kStep);
    if (withSignal) {
      tr.sigBucket = static_cast<int>(rng.Uniform(60, 400));
      const int nCh = 1 + static_cast<int>(rng.Uniform(0, 20));
      for (int i = 0; i < nCh; ++i) sigChs.push_back(static_cast<int>(rng.Uniform(0, 4 * 64)));
    }
    for (int blk = 0; blk < kNa; ++blk)
      for (int ch = 0; ch < kNch; ++ch) {
        const bool fpn = ch == kFpnCh[0] || ch == kFpnCh[1] || ch == kFpnCh[2] || ch == kFpnCh[3];
        double height = 0;
        if (withSignal && !fpn) {
          // 信号 ch 番号 = blk*64 + (ch を FPN 抜きで詰めた番号 ≈ ch 0..63 で近似)。ここでは単に (blk,ch) の一意化
          const int id = blk * 64 + (ch % 64);
          if (std::find(sigChs.begin(), sigChs.end(), id) != sigChs.end()) height = 500 + (id * 37) % 1001;
        }
        const double sigma = kind == kStep ? 2.0 : kSigma;  // (e) は雑音 σ = 2
        for (int c = 0; c < kNc; ++c) {
          double v = base[blk * kNch + ch] + rng.Gaus(0, sigma) + burst[c];
          // (e) 信号の立ち上がりと同時刻に FPN 全 ch へ +12 ADC の段差(100 cell)
          if (kind == kStep && fpn && c >= tr.sigBucket && c < tr.sigBucket + 100) v += 12.0;
          if (height > 0 && std::abs(c - tr.sigBucket) < 5) v += height;
          adc[(blk * kNch + ch) * kNc + c] = static_cast<Short_t>(std::lround(v));
        }
      }
    eventId = ev;
    eventTime = static_cast<ULong64_t>(ev) * 10000000ULL;  // 10 ms 間隔
    t.Fill();
    truth.push_back(tr);
  }
  t.Write();
  return truth;
}

struct Run {
  std::vector<int> has, bucket;
  std::vector<double> amp, per;
  std::vector<int> sigPeak;
  double ksP = -1;
  long overOnly = -1;
};

Run runScan(const std::string &dir, const std::string &name, double mhz)
{
  const std::string raw = dir + "/" + name + "_raw.root", out = dir + "/" + name;
  const int rc = static_cast<int>(reinterpret_cast<long>(gROOT->ProcessLine(
      ("burst_scan(\"" + raw + "\", \"" + out + "\", " + std::to_string(mhz) + ")").c_str())));
  check(rc == 0, name + ": burst_scan returns 0");
  Run r;
  TFile f((out + "_burst.root").c_str());
  TTree *t = f.Get<TTree>("burst");
  if (!t) { check(false, name + ": burst tree"); return r; }
  Int_t has, bucket, sp;
  Double_t amp, per;
  t->SetBranchAddress("hasBurst", &has);
  t->SetBranchAddress("burstBucket", &bucket);
  t->SetBranchAddress("burstAmpPP", &amp);
  t->SetBranchAddress("burstPeriodUs", &per);
  t->SetBranchAddress("sigPeak", &sp);
  for (Long64_t i = 0; i < t->GetEntries(); ++i) {
    t->GetEntry(i);
    r.has.push_back(has);
    r.bucket.push_back(bucket);
    r.amp.push_back(amp);
    r.per.push_back(per);
    r.sigPeak.push_back(sp);
  }
  std::ifstream in(out + "_burst.json");
  std::stringstream ss;
  ss << in.rdbuf();
  const std::string s = ss.str();
  const size_t k = s.find("\"ks_p\":");
  if (k != std::string::npos) r.ksP = std::atof(s.c_str() + k + 7);  // null → atof = 0 になるので下で別扱い
  if (k != std::string::npos && s.compare(k + 7, 4, "null") == 0) r.ksP = -1;
  const size_t ko = s.find("\"events_over_kthr_only\":");
  if (ko != std::string::npos) r.overOnly = std::atol(s.c_str() + ko + 24);
  check(gSystem->AccessPathName((out + "_burst.png").c_str()) == false, name + ": png written");
  return r;
}

double rate(const std::vector<int> &v)
{
  double n = 0;
  for (int x : v) n += x;
  return v.empty() ? 0 : n / v.size();
}

// バースト事象の真値との比較(位置・周期・振幅の最悪誤差と中央値)
void checkBurst(const Run &r, const std::vector<Truth> &tr, double mhz, const std::string &label)
{
  const int W = static_cast<int>(std::lround(2.0 * mhz));
  const double periodB = kPeriodUs * mhz;
  int nDet = 0, nPosOk = 0, nPerOk = 0, nAmpOk = 0;
  std::vector<double> pers;
  double worstPos = 0, worstPer = 0, worstAmp = 0;
  for (size_t i = 0; i < tr.size(); ++i) {
    if (!r.has[i]) continue;
    ++nDet;
    const double dpos = std::abs(r.bucket[i] - tr[i].start);
    const double dper = std::abs(r.per[i] / kPeriodUs - 1.0);
    const double damp = std::abs(r.amp[i] / (2 * kAmp) - 1.0);
    worstPos = std::max(worstPos, dpos);
    worstPer = std::max(worstPer, dper);
    worstAmp = std::max(worstAmp, damp);
    if (dpos <= periodB) ++nPosOk;
    if (dper <= 0.10) ++nPerOk;
    if (damp <= 0.25) ++nAmpOk;
    pers.push_back(r.per[i]);
  }
  std::sort(pers.begin(), pers.end());
  const double n = tr.size();
  std::printf("      [%s] %d/%d detected (%.1f %%), pos within 1 period %d, period within 10 %% %d, amp within 25 %% %d; "
              "worst |dpos| = %.1f buckets (period %.1f), worst dperiod = %.3f, worst damp = %.3f, median period = %.3f us\n",
              label.c_str(), nDet, static_cast<int>(n), 100.0 * nDet / n, nPosOk, nPerOk, nAmpOk, worstPos, periodB, worstPer,
              worstAmp, pers.empty() ? -1.0 : pers[pers.size() / 2]);
  check(nDet >= 0.95 * n, label + ": detection >= 95 %");
  check(nPosOk == nDet, label + ": position within 1 period of truth (all detected)");
  check(nPerOk == nDet, label + ": period within 10 % (all detected)");
  check(nAmpOk == nDet, label + ": peak-to-peak within 25 % of 2A (all detected)");
}
}  // namespace

int test_burst_scan()
{
  gROOT->ProcessLine(".L tools/scan/burst_scan.C+");
  const std::string dir = "/tmp/test_burst_scan";
  gSystem->mkdir(dir.c_str(), true);

  for (double mhz : {12.5, 25.0}) {
    char tag[32];
    std::snprintf(tag, sizeof tag, "run%02d", static_cast<int>(mhz));
    const std::string p = tag;
    const std::string lab = std::to_string(static_cast<int>(mhz)) + " MHz";
    const int n = 300;
    const std::vector<Truth> tb = writeRaw(dir + "/" + p + "b_raw.root", kBurst, n, mhz, 0, 11);
    const Run rb = runScan(dir, p + "b", mhz);
    checkBurst(rb, tb, mhz, "(b) burst only @" + lab);
    const std::vector<Truth> tc = writeRaw(dir + "/" + p + "c_raw.root", kBoth, n, mhz, 0, 12);
    const Run rc = runScan(dir, p + "c", mhz);
    checkBurst(rc, tc, mhz, "(c) signal + burst @" + lab);
    writeRaw(dir + "/" + p + "a_raw.root", kSignal, n, mhz, 0, 13);
    const Run ra = runScan(dir, p + "a", mhz);
    std::printf("      [(a) @%s] false positive %.2f %%\n", lab.c_str(), 100 * rate(ra.has));
    check(rate(ra.has) <= 0.02, "(a) signal only @" + lab + ": false positives <= 2 %");
    writeRaw(dir + "/" + p + "d_raw.root", kNoise, n, mhz, 0, 14);
    const Run rd = runScan(dir, p + "d", mhz);
    std::printf("      [(d) @%s] false positive %.2f %%\n", lab.c_str(), 100 * rate(rd.has));
    check(rate(rd.has) <= 0.02, "(d) noise only @" + lab + ": false positives <= 2 %");
    std::printf("      [KS @%s] uniform start p = %.4g\n", lab.c_str(), rb.ksP);
    check(rb.ksP > 0.01, "KS p > 0.01 for uniform burst start @" + lab);
    writeRaw(dir + "/" + p + "e_raw.root", kStep, n, mhz, 0, 16);
    const Run re = runScan(dir, p + "e", mhz);
    std::printf("      [(e) @%s] false positive %.2f %%, events_over_kthr_only = %ld\n", lab.c_str(), 100 * rate(re.has),
                re.overOnly);
    check(rate(re.has) <= 0.02, "(e) signal + FPN step 12 ADC @" + lab + ": false positives <= 2 %");
    check(re.overOnly > 0.5 * n, "(e) @" + lab + ": events_over_kthr_only recorded (> 50 % of events)");
    if (mhz == 12.5) {
      // 固定開始(100 ± 2)
      writeRaw(dir + "/fixb_raw.root", kBurst, 200, mhz, 1, 15);
      const Run rf = runScan(dir, "fixb", mhz);
      std::printf("      [KS fixed start] p = %.4g\n", rf.ksP);
      check(rf.ksP >= 0 && rf.ksP < 1e-6, "KS p < 1e-6 for fixed burst start (100 +- 2)");
    }
  }
  std::printf("test_burst_scan: %d failure(s)\n", fails);
  return fails;
}
