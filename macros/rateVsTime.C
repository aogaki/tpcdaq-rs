// =============================================================================
// rateVsTime.C - event rate vs time from GRAW frame headers only (no waveforms).
//
// Reads the 256-byte header of every data frame (frameType 1 or 2), takes the
// 48-bit CoBo timestamp (10 ns ticks - checked against wall clock on the
// 2026-09-01 and 2026-09-15 runs: 9.4e7 ticks/s) and the eventIdx, and fills
//   * hRate  : events per time bin -> rate [Hz] with sqrt(N) errors
//   * hDt    : time between consecutive events [s] (Poisson -> exponential)
//   * hDtLog : the same with log10(dt) bins (dead time / bursts are visible)
// DAQ restarts (timestamp counter resets) are stitched together with zero gap
// and marked with a red line - the real gap is not in the data.
//
// Input: the first chunk (follow-up chunks _0001, _0002, ... are merged, same
// rule as graw2root.C) or a list file with one path per line.
//
// Output: <run>_rate.root (histograms) and <run>_rate.png, plus a per-hour
// table on stdout.
//
// Usage:  root -l -b -q 'rateVsTime.C("CoBo_xxx_0000.graw", 600)'
//         (second argument = bin width in seconds, default 600)
// =============================================================================
#include <TCanvas.h>
#include <TFile.h>
#include <TH1D.h>
#include <TLine.h>
#include <TStyle.h>

#include <cmath>
#include <cstdio>
#include <fstream>
#include <string>
#include <vector>

namespace {

const double TICK_S = 1e-8;  // CoBo timestamp tick = 10 ns

bool endsWith(const std::string &s, const std::string &suf) {
  return s.size() >= suf.size() && s.compare(s.size() - suf.size(), suf.size(), suf) == 0;
}

// Same chunk discovery as graw2root.C: xxx_0000.graw -> xxx_0001.graw ... until a gap.
std::vector<std::string> inputFiles(const std::string &first) {
  std::vector<std::string> files;
  if (endsWith(first, ".graw")) {
    files.push_back(first);
    if (first.size() > 10 && first.substr(first.size() - 10, 5) == "_0000") {
      for (int k = 1;; ++k) {
        char suf[16];
        std::snprintf(suf, sizeof suf, "_%04d.graw", k);
        const std::string next = first.substr(0, first.size() - 10) + suf;
        std::ifstream t(next, std::ios::binary);
        if (!t) break;
        files.push_back(next);
      }
    }
  } else {
    std::ifstream list(first);
    std::string line;
    while (std::getline(list, line)) {
      const size_t a = line.find_first_not_of(" \t\r"), b = line.find_last_not_of(" \t\r");
      if (a == std::string::npos) continue;
      const std::string path = line.substr(a, b - a + 1);
      if (path[0] != '#') files.push_back(path);
    }
  }
  return files;
}

}  // namespace

void rateVsTime(const char *firstFile, double binSeconds = 600.0) {
  const std::vector<std::string> files = inputFiles(firstFile);
  if (files.empty()) {
    std::printf("no input files\n");
    return;
  }

  // --- header-only scan -------------------------------------------------------
  std::vector<double> tSec;      // stitched time since first event [s]
  std::vector<double> restarts;  // stitched time of every timestamp reset [s]
  unsigned char hdr[256];
  double offset = 0, prevRaw = -1, firstRaw = -1;
  long nFrames = 0, nSkipped = 0;
  for (const std::string &path : files) {
    std::ifstream f(path, std::ios::binary);
    if (!f) {
      std::printf("cannot open %s\n", path.c_str());
      return;
    }
    while (f.read(reinterpret_cast<char *>(hdr), 12)) {
      const long blob = 1L << (hdr[0] & 0x0F);
      const long nbytes = ((hdr[1] << 16) | (hdr[2] << 8) | hdr[3]) * blob;
      const int ftype = (hdr[5] << 8) | hdr[6];
      if (nbytes < 12) break;
      if (ftype != 1 && ftype != 2) {  // topology / other frames
        ++nSkipped;
        f.seekg(nbytes - 12, std::ios::cur);
        continue;
      }
      if (!f.read(reinterpret_cast<char *>(hdr) + 12, 256 - 12)) break;
      unsigned long long ticks = 0;
      for (int b = 16; b < 22; ++b) ticks = (ticks << 8) | hdr[b];
      const double raw = static_cast<double>(ticks) * TICK_S;
      if (firstRaw < 0) firstRaw = raw;
      if (prevRaw >= 0 && raw < prevRaw) {  // counter reset = DAQ restart
        offset += prevRaw - firstRaw;
        firstRaw = raw;
        restarts.push_back(offset);
      }
      prevRaw = raw;
      tSec.push_back(offset + (raw - firstRaw));
      ++nFrames;
      f.seekg(nbytes - 256, std::ios::cur);
    }
  }
  if (tSec.empty()) {
    std::printf("no data frames\n");
    return;
  }
  const double duration = tSec.back();
  std::printf("%ld events in %zu file(s), %ld non-data frames skipped, %zu DAQ restart(s)\n",
              nFrames, files.size(), nSkipped, restarts.size());
  std::printf("duration %.1f s = %.2f h, mean rate %.4f Hz\n", duration, duration / 3600,
              nFrames / duration);

  // --- histograms -----------------------------------------------------------
  const int nBins = static_cast<int>(std::ceil(duration / binSeconds));
  TH1D hRate("hRate", "event rate;time since first event [h];rate [Hz]", nBins, 0,
             nBins * binSeconds / 3600.0);
  hRate.Sumw2();
  TH1D hDt("hDt", "time between events;#Deltat [s];events", 200, 0, 200);
  TH1D hDtLog("hDtLog", "time between events;log_{10}(#Deltat / s);events", 80, -4, 4);
  for (size_t i = 0; i < tSec.size(); ++i) {
    hRate.Fill(tSec[i] / 3600.0);
    if (i > 0) {
      const double dt = tSec[i] - tSec[i - 1];
      hDt.Fill(dt);
      if (dt > 0) hDtLog.Fill(std::log10(dt));
    }
  }
  hRate.Scale(1.0 / binSeconds);  // counts -> Hz (errors scale with it)

  // per-hour table (stdout)
  std::printf("hour  events  rate[Hz]\n");
  const int nHours = static_cast<int>(std::ceil(duration / 3600.0));
  std::vector<long> perHour(nHours, 0);
  for (double t : tSec) ++perHour[std::min(nHours - 1, static_cast<int>(t / 3600.0))];
  for (int h = 0; h < nHours; ++h) {
    const double w = (h == nHours - 1) ? duration - h * 3600.0 : 3600.0;
    std::printf("%4d  %6ld  %.4f\n", h, perHour[h], w > 0 ? perHour[h] / w : 0.0);
  }

  // --- output ---------------------------------------------------------------
  std::string outName = files.front();
  for (auto &ch : outName)
    if (ch == ':') ch = '-';
  if (files.size() > 1)
    outName.replace(outName.size() - 10, 10, "_rate");
  else
    outName.replace(outName.size() - 5, 5, "_rate");
  gStyle->SetOptStat(0);
  TCanvas c("c", "rate", 1400, 900);
  c.Divide(2, 2);
  c.cd(1);
  hRate.SetMinimum(0);
  hRate.Draw("E");
  for (double r : restarts) {
    TLine *l = new TLine(r / 3600.0, 0, r / 3600.0, hRate.GetMaximum() * 1.05);
    l->SetLineColor(kRed);
    l->SetLineStyle(2);
    l->Draw();
  }
  c.cd(2);
  gPad->SetLogy();
  hDt.Draw();
  c.cd(3);
  hDtLog.Draw();
  c.cd(4);
  TH1D *hRateLog = static_cast<TH1D *>(hRate.Clone("hRateLog"));
  hRateLog->SetTitle("event rate (log);time since first event [h];rate [Hz]");
  hRateLog->SetMinimum(hRate.GetMaximum() * 1e-3);
  gPad->SetLogy();
  hRateLog->Draw("E");
  c.SaveAs((outName + ".png").c_str());
  TFile out((outName + ".root").c_str(), "RECREATE");
  hRate.Write();
  hDt.Write();
  hDtLog.Write();
  out.Close();
  std::printf("wrote %s.root / .png\n", outName.c_str());
}
