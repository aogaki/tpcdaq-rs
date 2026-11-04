// =============================================================================
// test_analyzeUVW_qmin.C - synthetic tests for analyzeUVW's stripQMin (TODO/207).
//
//   export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX27.sdk
//   cd macros && root -l -b -q test_analyzeUVW_qmin.C   -> exit code = failures (0 = green)
//
// Writes a synthetic raw tree + synthetic channel_map.csv into a temp directory and
// chdirs there (the real channel_map.csv is never read). One event, a straight track:
// aget 0, channels 0-5 = U, 6-11 = V, 12-17 = W; channel k%6 sits at s = 1.5 * (k%6) mm
// and has its pulse peak at cell 100 + 10 * (k%6) (z slope 10 cells * 0.1736 mm = 1.74 mm
// per 1.5 mm strip: a straight line, well inside the 10 mm band). Baseline 100 in every
// cell (pedestal 100 exactly), the pulse = peak cell 100 + P, neighbours +-1 cell 100 + P/2.
// All three cells are above the 35 ADC threshold, q = P + 2 * P/2 = 2 P (hand-computed):
//   strips 0-3: P = 2000 -> q = 4000    strip 4: P = 1500 -> q = 3000
//   strip 5   : P =  750 -> q = 1500    (the "faint side" of a 232 ns track)
// Hits per view that survive the halo cut:
//   stripQMin = 2000 : q >= 2000 -> strips 0-4 = 5 hits   (strip 5, q = 1500, dropped)
//   stripQMin = 1100 : strips 0-5 = 6 hits
// =============================================================================
#include "analyzeUVW.C"

#include <TSystem.h>

#include <cstdio>
#include <fstream>
#include <string>

static int g_failures = 0;
static void check(bool ok, const std::string &name)
{
  std::printf("%s  %s\n", ok ? "PASS" : "FAIL", name.c_str());
  if (!ok) ++g_failures;
}

static void writeMap()
{
  std::ofstream f("channel_map.csv");
  f << "# aget,raw_channel,view,strip,s_mm\n";
  for (int ch = 0; ch < 18; ++ch)
    f << "0," << ch << "," << ch / 6 << "," << ch % 6 << "," << 1.5 * (ch % 6) << "\n";
}

static void writeRaw(const char *path)
{
  static Short_t adc[4][68][512];
  TFile f(path, "RECREATE");
  TTree t("raw", "synthetic");
  UInt_t eventId = 7;
  t.Branch("eventId", &eventId, "eventId/i");
  t.Branch("adc", adc, "adc[4][68][512]/S");
  const int P[6] = {2000, 2000, 2000, 2000, 1500, 750};
  for (int a = 0; a < 4; ++a)
    for (int c = 0; c < 68; ++c)
      for (int s = 0; s < 512; ++s) adc[a][c][s] = 100;
  for (int ch = 0; ch < 18; ++ch) {
    const int k = ch % 6, t0 = 100 + 10 * k;
    adc[0][ch][t0] = (Short_t)(100 + P[k]);
    adc[0][ch][t0 - 1] = (Short_t)(100 + P[k] / 2);
    adc[0][ch][t0 + 1] = (Short_t)(100 + P[k] / 2);
  }
  t.Fill();
  t.Write();
  f.Close();
}

static int totalHits(const char *path)
{
  TFile f(path);
  TTree *t = (TTree *)f.Get("uvw");
  if (!t) return -1;
  int nh[3];
  t->SetBranchAddress("nhits", nh);
  t->GetEntry(0);
  return nh[0] + nh[1] + nh[2];
}
static double par(const char *path, const char *name)
{
  TFile f(path);
  auto *p = (TParameter<double> *)f.Get(name);
  return p ? p->GetVal() : -1e30;
}

void test_analyzeUVW_qmin()
{
  TString cwd = gSystem->pwd();
  TString dir = gSystem->TempDirectory();
  dir += "/test_qmin_207";
  gSystem->mkdir(dir, true);
  gSystem->ChangeDirectory(dir);
  writeMap();
  writeRaw("t_raw.root");
  gSystem->Unlink("def_uvw.root");
  gSystem->Unlink("q2000_uvw.root");
  gSystem->Unlink("q1100_uvw.root");
  gSystem->Unlink("bad_uvw.root");

  // 100 mbar / 25 MHz / 1200 V: arbitrary test values
  analyzeUVW("t_raw.root", 100, 25, 1200, 0, -1, "def_uvw.root");
  analyzeUVW("t_raw.root", 100, 25, 1200, 0, -1, "q2000_uvw.root", 2000.0);
  analyzeUVW("t_raw.root", 100, 25, 1200, 0, -1, "q1100_uvw.root", 1100.0);
  analyzeUVW("t_raw.root", 100, 25, 1200, 0, -1, "bad_uvw.root", 0.0);

  check(totalHits("q2000_uvw.root") == 15, "stripQMin=2000: faint strip (q=1500) dropped, 5 hits x 3 views");
  check(totalHits("q1100_uvw.root") == 18, "stripQMin=1100: faint strip kept, 6 hits x 3 views");
  check(totalHits("def_uvw.root") == 15, "default (no argument) == 2000 behaviour");
  check(par("q1100_uvw.root", "strip_q_min") == 1100.0, "TParameter strip_q_min = 1100 written");
  check(par("def_uvw.root", "strip_q_min") == 2000.0, "TParameter strip_q_min = 2000 by default");
  check(gSystem->AccessPathName("bad_uvw.root"), "stripQMin <= 0: error, nothing written");

  gSystem->ChangeDirectory(cwd);
  std::printf("TOTAL failures: %d\n", g_failures);
  gSystem->Exit(g_failures);
}
