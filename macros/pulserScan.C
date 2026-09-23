// =============================================================================
// pulserScan.C - D-3 pulser amplitude scan analysis (TODO/091 SS1).
//
// Reads the two flat CSV files produced by
//   calib scan --amp <mV>=<graw> [--amp ...] --out scan.json \
//              --csv-channels scan_channels.csv --csv-waveform scan_waveform.csv
// (see src/calib/scan.rs) and makes 3 plots + a rise/peak/FWHM summary:
//   (1) per-channel response vs amplitude, all channels overlaid (FPN in red)
//   (2) per-channel residual fraction vs amplitude (signal channels only):
//       median, 16th/84th percentile band
//   (3) average waveform per amplitude (signal-channel mean, FPN excluded)
//   (4) rise time (10-90%) / peak bucket / FWHM per amplitude, from (3), as text
//       - units are ADC sample buckets: the CSVs carry no AGET clock period, so
//         this macro does NOT convert to ns (see TODO/091 result notes).
//
// channelsCsv columns: asad,aget,ch,is_fpn,amp_<v0>,...,amp_<vN-1>,
//                       fit_slope,fit_intercept,fit_max_resid_frac
// waveformCsv columns (long format): asad,aget,ch,is_fpn,amp_mv,bucket,adc_avg
//
// Usage: root -l -b -q 'pulserScan.C("scan_channels.csv","scan_waveform.csv")'
//   (on this Mac + CLT 27, ACLiC/cling needs
//    SDKROOT=$(xcrun --sdk macosx --show-sdk-path) in front - see macros/README.md)
// =============================================================================
#include <TCanvas.h>
#include <TGraph.h>
#include <TLegend.h>
#include <TMultiGraph.h>
#include <TPaveText.h>
#include <TStyle.h>

#include <algorithm>
#include <array>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <map>
#include <sstream>
#include <string>
#include <vector>

namespace {

std::vector<std::string> splitCsv(const std::string &line) {
  std::vector<std::string> out;
  std::stringstream ss(line);
  std::string tok;
  while (std::getline(ss, tok, ',')) out.push_back(tok);
  return out;
}

struct ChannelRow {
  int asad = 0, aget = 0, ch = 0;
  bool isFpn = false;
  std::vector<double> amplitude;  // aligned with ChannelsCsv::amps
  double slope = 0, intercept = 0, maxResidFrac = 0;
};

struct ChannelsCsv {
  std::vector<double> amps;
  std::vector<ChannelRow> rows;
};

// asad,aget,ch,is_fpn, amp_<v0>...amp_<vN-1>, fit_slope,fit_intercept,fit_max_resid_frac
ChannelsCsv readChannelsCsv(const std::string &path) {
  ChannelsCsv out;
  std::ifstream f(path);
  if (!f) {
    std::printf("cannot open %s\n", path.c_str());
    return out;
  }
  std::string header;
  std::getline(f, header);
  const std::vector<std::string> cols = splitCsv(header);
  const int nAmps = static_cast<int>(cols.size()) - 4 - 3;
  if (nAmps < 1) {
    std::printf("unexpected header in %s (%zu columns)\n", path.c_str(), cols.size());
    return out;
  }
  for (int i = 0; i < nAmps; ++i) {
    const std::string &tok = cols[4 + i];  // "amp_<value>"
    out.amps.push_back(std::atof(tok.c_str() + 4));
  }

  std::string line;
  while (std::getline(f, line)) {
    if (line.empty()) continue;
    const std::vector<std::string> v = splitCsv(line);
    if (static_cast<int>(v.size()) != 4 + nAmps + 3) continue;
    ChannelRow r;
    r.asad = std::atoi(v[0].c_str());
    r.aget = std::atoi(v[1].c_str());
    r.ch = std::atoi(v[2].c_str());
    r.isFpn = std::atoi(v[3].c_str()) != 0;
    for (int i = 0; i < nAmps; ++i) r.amplitude.push_back(std::atof(v[4 + i].c_str()));
    r.slope = std::atof(v[4 + nAmps].c_str());
    r.intercept = std::atof(v[5 + nAmps].c_str());
    r.maxResidFrac = std::atof(v[6 + nAmps].c_str());
    out.rows.push_back(r);
  }
  return out;
}

// amp_mv -> average waveform over signal (non-FPN) channels, 512 buckets.
// asad,aget,ch,is_fpn,amp_mv,bucket,adc_avg
std::map<double, std::array<double, 512>> readWaveformCsvSignalAverage(const std::string &path) {
  std::map<double, std::array<double, 512>> sum;
  std::map<double, int> nCh;
  std::ifstream f(path);
  if (!f) {
    std::printf("cannot open %s\n", path.c_str());
    return {};
  }
  std::string header;
  std::getline(f, header);
  std::string line;
  while (std::getline(f, line)) {
    if (line.empty()) continue;
    const std::vector<std::string> v = splitCsv(line);
    if (v.size() != 7) continue;
    const bool isFpn = std::atoi(v[3].c_str()) != 0;
    if (isFpn) continue;
    const double amp = std::atof(v[4].c_str());
    const int bucket = std::atoi(v[5].c_str());
    const double adc = std::atof(v[6].c_str());
    if (bucket < 0 || bucket >= 512) continue;
    if (sum.find(amp) == sum.end()) sum[amp].fill(0.0);
    sum[amp][bucket] += adc;
    if (bucket == 0) ++nCh[amp];
  }
  for (auto &kv : sum) {
    const int n = nCh[kv.first];
    if (n > 0)
      for (double &x : kv.second) x /= n;
  }
  return sum;
}

double percentile(std::vector<double> v, double p) {
  if (v.empty()) return 0.0;
  std::sort(v.begin(), v.end());
  const double idx = p * (v.size() - 1);
  const size_t lo = static_cast<size_t>(std::floor(idx));
  const size_t hi = static_cast<size_t>(std::ceil(idx));
  if (lo == hi) return v[lo];
  const double frac = idx - lo;
  return v[lo] * (1 - frac) + v[hi] * frac;
}

// Linear-interpolated bucket where `wave` first crosses `threshold` searching
// upward from bucket 0 to peakBucket.
double crossingUp(const std::array<double, 512> &wave, int peakBucket, double threshold) {
  for (int i = 1; i <= peakBucket; ++i) {
    if (wave[i] >= threshold) {
      const double d = wave[i] - wave[i - 1];
      if (d == 0) return static_cast<double>(i);
      return (i - 1) + (threshold - wave[i - 1]) / d;
    }
  }
  return static_cast<double>(peakBucket);
}

// Linear-interpolated bucket where `wave` last drops below `threshold` searching
// from peakBucket to the end.
double crossingDown(const std::array<double, 512> &wave, int peakBucket, double threshold) {
  for (int i = peakBucket; i < 511; ++i) {
    if (wave[i + 1] <= threshold) {
      const double d = wave[i + 1] - wave[i];
      if (d == 0) return static_cast<double>(i);
      return i + (threshold - wave[i]) / d;
    }
  }
  return static_cast<double>(peakBucket);
}

}  // namespace

void pulserScan(const char *channelsCsv, const char *waveformCsv, const char *outDir = ".") {
  const ChannelsCsv chans = readChannelsCsv(channelsCsv);
  if (chans.rows.empty()) {
    std::printf("no channel rows read from %s\n", channelsCsv);
    return;
  }
  const std::map<double, std::array<double, 512>> avgWave = readWaveformCsvSignalAverage(waveformCsv);
  std::printf("%zu channels, %zu amplitudes (channels.csv), %zu amplitudes (waveform.csv)\n",
              chans.rows.size(), chans.amps.size(), avgWave.size());

  gStyle->SetOptStat(0);
  TCanvas c("c", "pulserScan", 1600, 1200);
  c.Divide(2, 2);

  // --- panel 1: per-channel response vs amplitude (FPN red) ------------------
  c.cd(1);
  TMultiGraph mg1;
  std::vector<TGraph *> chGraphs;
  for (const ChannelRow &r : chans.rows) {
    auto amps = chans.amps;  // TGraph ctor wants non-const double*
    TGraph *g = new TGraph(static_cast<int>(amps.size()), amps.data(), r.amplitude.data());
    g->SetLineColorAlpha(r.isFpn ? kRed : kBlue, 0.25);
    mg1.Add(g, "L");
    chGraphs.push_back(g);
  }
  mg1.SetTitle("per-channel response (blue=signal, red=FPN);pulser amplitude [mV];peak - "
               "baseline [ADC]");
  mg1.Draw("A");

  // --- panel 2: residual fraction vs amplitude, signal ch only ----------------
  c.cd(2);
  std::vector<double> ampX, medY, loY, hiY;
  for (size_t i = 0; i < chans.amps.size(); ++i) {
    std::vector<double> resid;
    for (const ChannelRow &r : chans.rows) {
      if (r.isFpn) continue;
      const double pred = r.slope * chans.amps[i] + r.intercept;
      if (std::fabs(pred) < 1e-9) continue;
      resid.push_back((r.amplitude[i] - pred) / pred);
    }
    ampX.push_back(chans.amps[i]);
    medY.push_back(percentile(resid, 0.5));
    loY.push_back(percentile(resid, 0.16));
    hiY.push_back(percentile(resid, 0.84));
  }
  TGraph gMed(static_cast<int>(ampX.size()), ampX.data(), medY.data());
  gMed.SetLineColor(kBlack);
  gMed.SetMarkerStyle(20);
  TGraph gLo(static_cast<int>(ampX.size()), ampX.data(), loY.data());
  gLo.SetLineColor(kGray + 1);
  gLo.SetLineStyle(2);
  TGraph gHi(static_cast<int>(ampX.size()), ampX.data(), hiY.data());
  gHi.SetLineColor(kGray + 1);
  gHi.SetLineStyle(2);
  TMultiGraph mg2;
  mg2.Add(&gLo, "L");
  mg2.Add(&gMed, "LP");
  mg2.Add(&gHi, "L");
  mg2.SetTitle("residual fraction vs amplitude (signal ch, median +- 16-84%);pulser amplitude "
               "[mV];(actual - fit) / fit");
  mg2.Draw("A");

  // --- panel 3: average waveform per amplitude (signal ch mean) --------------
  c.cd(3);
  TMultiGraph mg3;
  std::vector<double> bucketX(512);
  for (int b = 0; b < 512; ++b) bucketX[b] = b;
  std::vector<TGraph *> waveGraphs;
  const std::vector<int> palette = {kBlack, kBlue, kGreen + 2, kOrange + 7, kMagenta + 1, kRed};
  int idx = 0;
  for (const auto &kv : avgWave) {
    auto wave = kv.second;  // TGraph ctor wants non-const double*
    TGraph *g = new TGraph(512, bucketX.data(), wave.data());
    g->SetLineColor(palette[idx % static_cast<int>(palette.size())]);
    g->SetLineWidth(2);
    g->SetTitle(Form("%.0f mV", kv.first));
    mg3.Add(g, "L");
    waveGraphs.push_back(g);
    ++idx;
  }
  mg3.SetTitle("average waveform (signal ch mean, FPN excluded);sample bucket;ADC (pedestal "
               "subtracted)");
  mg3.Draw("A");
  TLegend leg3(0.65, 0.55, 0.88, 0.88);
  for (TGraph *g : waveGraphs) leg3.AddEntry(g, g->GetTitle(), "l");
  leg3.Draw();

  // --- panel 4: rise/peak/FWHM table (printed + drawn as text) ---------------
  c.cd(4);
  std::printf("amp[mV]  peak_bucket  peak[ADC]  rise10-90[buckets]  FWHM[buckets]\n");
  TPaveText pt(0.01, 0.05, 0.99, 0.95, "NB");
  pt.SetTextAlign(12);
  pt.SetTextFont(82);
  pt.SetTextSize(0.032);
  pt.AddText("amp  peak_bkt  peak[ADC]  rise10-90  FWHM[bkt]");
  for (const auto &kv : avgWave) {
    const std::array<double, 512> &w = kv.second;
    const int peakBucket = static_cast<int>(std::max_element(w.begin(), w.end()) - w.begin());
    const double peak = w[peakBucket];
    const double b10 = crossingUp(w, peakBucket, 0.10 * peak);
    const double b90 = crossingUp(w, peakBucket, 0.90 * peak);
    const double bLo50 = crossingUp(w, peakBucket, 0.50 * peak);
    const double bHi50 = crossingDown(w, peakBucket, 0.50 * peak);
    const double rise = b90 - b10;
    const double fwhm = bHi50 - bLo50;
    std::printf("%7.0f  %11d  %9.2f  %18.2f  %13.2f\n", kv.first, peakBucket, peak, rise, fwhm);
    pt.AddText(Form("%5.0f  %6d  %9.2f  %9.2f  %8.2f", kv.first, peakBucket, peak, rise, fwhm));
  }
  pt.Draw();

  const std::string outPng = std::string(outDir) + "/pulserScan.png";
  c.SaveAs(outPng.c_str());
  std::printf("wrote %s\n", outPng.c_str());
}
