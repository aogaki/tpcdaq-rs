// =============================================================================
// chargeVsTime.C - is a rate change a *counting* effect or a *signal* effect?
//
// Joins the per-event charge from analyzeUVW.C (<run>_uvw.root: eventId,
// qview[3], nhits[3]) with the event time from graw2root.C (<run>_raw.root:
// eventId, eventTime = 48-bit CoBo timestamp, 10 ns ticks) and draws
//   * hQvsT   : per-time-bin median (and 16/84 % band) of the event charge
//   * hNhitsT : per-time-bin median number of hits
//   * hQearly / hQlate : charge spectra of the first and last quarter of the run
//   * hRate   : rate [Hz] (same binning) for reference
// Electron attachment (humidity / O2) moves the whole charge spectrum down
// with time and the rate drops because events fall under the trigger
// threshold; a decaying radioactive population leaves the spectrum unchanged
// and only the count drops.
//
// Usage: root -l -b -q 'chargeVsTime.C("CoBo_xxx_uvw.root", "CoBo_xxx_raw.root", 3600)'
// =============================================================================
#include <TCanvas.h>
#include <TFile.h>
#include <TGraphAsymmErrors.h>
#include <TH1D.h>
#include <TLegend.h>
#include <TStyle.h>
#include <TTree.h>

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <map>
#include <string>
#include <vector>

namespace {
const double TICK_S = 1e-8;

double quantile(std::vector<double> v, double q) {
  if (v.empty()) return 0;
  std::sort(v.begin(), v.end());
  const size_t i = std::min(v.size() - 1, static_cast<size_t>(q * v.size()));
  return v[i];
}
}  // namespace

void chargeVsTime(const char *uvwFile, const char *rawFile, double binSeconds = 3600.0) {
  // --- event time from the raw tree (eventId -> stitched seconds) --------------
  TFile fr(rawFile);
  TTree *raw = static_cast<TTree *>(fr.Get("raw"));
  if (!raw) {
    std::printf("no 'raw' tree in %s\n", rawFile);
    return;
  }
  UInt_t eventId = 0;
  ULong64_t eventTime = 0;
  raw->SetBranchStatus("*", false);
  raw->SetBranchStatus("eventId", true);
  raw->SetBranchStatus("eventTime", true);
  raw->SetBranchAddress("eventId", &eventId);
  raw->SetBranchAddress("eventTime", &eventTime);
  std::map<UInt_t, double> tOf;  // eventId -> seconds since first event
  double offset = 0, firstRaw = -1, prevRaw = -1;
  std::vector<double> tAll;
  for (Long64_t i = 0; i < raw->GetEntries(); ++i) {
    raw->GetEntry(i);
    const double t = static_cast<double>(eventTime) * TICK_S;
    if (firstRaw < 0) firstRaw = t;
    if (prevRaw >= 0 && t < prevRaw) {  // DAQ restart: counter reset
      offset += prevRaw - firstRaw;
      firstRaw = t;
    }
    prevRaw = t;
    const double ts = offset + (t - firstRaw);
    tOf[eventId] = ts;  // eventId repeats after a restart: keep the latest
    tAll.push_back(ts);
  }
  if (tAll.empty()) return;
  const double duration = tAll.back();

  // --- per-event charge from the uvw tree ------------------------------------
  TFile fu(uvwFile);
  TTree *uvw = static_cast<TTree *>(fu.Get("uvw"));
  if (!uvw) {
    std::printf("no 'uvw' tree in %s\n", uvwFile);
    return;
  }
  UInt_t uId = 0;
  Float_t qview[3] = {0, 0, 0};
  Int_t nhits[3] = {0, 0, 0};
  uvw->SetBranchStatus("*", false);
  uvw->SetBranchStatus("eventId", true);
  uvw->SetBranchStatus("qview", true);
  uvw->SetBranchStatus("nhits", true);
  uvw->SetBranchAddress("eventId", &uId);
  uvw->SetBranchAddress("qview", qview);
  uvw->SetBranchAddress("nhits", nhits);

  const int nBins = static_cast<int>(std::ceil(duration / binSeconds));
  std::vector<std::vector<double>> qBin(nBins), hBin(nBins);
  std::vector<double> qEarly, qLate, qAllv;
  long matched = 0, unmatched = 0;
  // uvw entries are in raw-tree order, so walk both trees in step (eventId may repeat after a restart)
  const Long64_t n = std::min(uvw->GetEntries(), raw->GetEntries());
  for (Long64_t i = 0; i < n; ++i) {
    uvw->GetEntry(i);
    const double t = tAll[i];
    ++matched;
    const double q = qview[0] + qview[1] + qview[2];
    const double h = nhits[0] + nhits[1] + nhits[2];
    const int b = std::min(nBins - 1, static_cast<int>(t / binSeconds));
    qBin[b].push_back(q);
    hBin[b].push_back(h);
    qAllv.push_back(q);
    if (t < 0.25 * duration) qEarly.push_back(q);
    if (t > 0.75 * duration) qLate.push_back(q);
  }
  unmatched = raw->GetEntries() - n;
  std::printf("%ld events joined (%ld raw entries without uvw entry), %.1f h, %d bins of %.0f s\n",
              matched, unmatched, duration / 3600, nBins, binSeconds);

  // --- histograms -----------------------------------------------------------
  gStyle->SetOptStat(0);
  const double tMaxH = nBins * binSeconds / 3600.0;
  TH1D hRate("hRate", "event rate;time since first event [h];rate [Hz]", nBins, 0, tMaxH);
  TH1D hQmed("hQmed", "event charge (median, 16-84% band);time since first event [h];#Sigma qview [ADC]",
             nBins, 0, tMaxH);
  TH1D hHmed("hHmed", "hits per event (median);time since first event [h];hits", nBins, 0, tMaxH);
  TGraphAsymmErrors gQ(nBins);
  std::printf("bin  events  rate[Hz]  Qmedian  Q16  Q84  hits_median\n");
  for (int b = 0; b < nBins; ++b) {
    const double w = (b == nBins - 1) ? duration - b * binSeconds : binSeconds;
    hRate.SetBinContent(b + 1, w > 0 ? qBin[b].size() / w : 0);
    hRate.SetBinError(b + 1, w > 0 ? std::sqrt(static_cast<double>(qBin[b].size())) / w : 0);
    const double m = quantile(qBin[b], 0.5), lo = quantile(qBin[b], 0.16), hi = quantile(qBin[b], 0.84);
    hQmed.SetBinContent(b + 1, m);
    gQ.SetPoint(b, (b + 0.5) * binSeconds / 3600.0, m);
    gQ.SetPointError(b, 0, 0, m - lo, hi - m);
    hHmed.SetBinContent(b + 1, quantile(hBin[b], 0.5));
    std::printf("%3d  %6zu  %.4f  %8.0f  %8.0f  %8.0f  %6.0f\n", b, qBin[b].size(),
                hRate.GetBinContent(b + 1), m, lo, hi, hHmed.GetBinContent(b + 1));
  }
  const double qMax = quantile(qAllv, 0.995) * 1.1;
  TH1D hQearly("hQearly", "event charge, first quarter vs last quarter of the run;#Sigma qview [ADC];events (norm.)",
               60, 0, qMax);
  TH1D hQlate("hQlate", "", 60, 0, qMax);
  for (double q : qEarly) hQearly.Fill(q);
  for (double q : qLate) hQlate.Fill(q);
  if (hQearly.Integral() > 0) hQearly.Scale(1.0 / hQearly.Integral());
  if (hQlate.Integral() > 0) hQlate.Scale(1.0 / hQlate.Integral());
  std::printf("charge median first quarter %.0f, last quarter %.0f (ratio %.3f)\n", quantile(qEarly, 0.5),
              quantile(qLate, 0.5), quantile(qEarly, 0.5) > 0 ? quantile(qLate, 0.5) / quantile(qEarly, 0.5) : 0);

  std::string outName = uvwFile;
  outName.replace(outName.size() - 9, 9, "_charge_vs_time");
  TCanvas c("c", "charge vs time", 1400, 900);
  c.Divide(2, 2);
  c.cd(1);
  hRate.SetMinimum(0);
  hRate.Draw("E");
  c.cd(2);
  hQmed.SetMinimum(0);
  hQmed.SetMaximum(qMax);
  hQmed.Draw("AXIS");
  gQ.SetMarkerStyle(20);
  gQ.Draw("P");
  c.cd(3);
  hHmed.SetMinimum(0);
  hHmed.Draw("HIST");
  c.cd(4);
  hQearly.SetLineColor(kBlue);
  hQlate.SetLineColor(kRed);
  hQearly.Draw("HIST");
  hQlate.Draw("HIST SAME");
  TLegend leg(0.55, 0.7, 0.88, 0.88);
  leg.AddEntry(&hQearly, "first quarter", "l");
  leg.AddEntry(&hQlate, "last quarter", "l");
  leg.Draw();
  c.SaveAs((outName + ".png").c_str());
  TFile out((outName + ".root").c_str(), "RECREATE");
  hRate.Write();
  hQmed.Write();
  hHmed.Write();
  hQearly.Write();
  hQlate.Write();
  gQ.Write("gQmedian");
  out.Close();
  std::printf("wrote %s.root / .png\n", outName.c_str());
}
