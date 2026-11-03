// =============================================================================
// test_drawUVW.C - synthetic tests for drawUVW.C (TODO/190, TODO/191).
//
//   export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX27.sdk
//   root -l -b -q macros/test_drawUVW.C      -> exit code = number of failures (0 = green)
//
// fillUVW() is called directly on hand-made adc arrays; the end-to-end tests write a
// synthetic raw tree + synthetic channel_map.csv into a temp directory and chdir there
// (the real channel_map.csv is never read). Expected numbers are hand-computed:
//   [1] strip cells 0..24 = 100 (baseline 100), cell 200 = 300      -> 0 at cell 0, 200 at cell 200
//   [2] cell 0 = 100, cells 1..24 = 90/110 alternating (12 x 90 + 12 x 110 = 2400, +100 = 2500,
//       /25 = 100 exactly), cell 25 = 1000 -> 900, cell 30 = 100 -> 0
//   [3] baseline 100, cell 300 = 40 -> -60 (negative kept)
//   [4] FPN ch (raw 11) and unmapped ch carry 5000 -> nothing in any plane
//   [5] 3 ch x 30000 = 90000 -> clamp 65535, baseline of the clamped sum (all cells equal) = 65535
//       -> value 0 in cell 0..; make cell 300 differ: 3 x 30000 at cell 300, 3 x 10 elsewhere
//       -> sum 30 elsewhere (baseline 30); at 300: clamp(90000) = 65535 -> 65535 - 30 = 65505.
//       2 ch (10000 + 20000 = 30000 < 65535) at cell 300, 2 x 10 elsewhere -> 30000 - 20 = 29980.
//   [6] map strips 1, 2, 5 only -> nStrips 5; strips 3, 4 are all 0
// TODO/191 (3D points). Synthetic planes: pitch 2 mm (NOT the real 1.5), strip k of plane v at
//   s = O[v] + 2 (k - 31), k = 1..61, O = (1.25, -3.5, 6.75), c = sum O = 4.5. A 3D point with
//   (u', v') = (2a, 2b) (integers a, b), w' = -u' - v' hits strips k = a+31, b+31, 31-a-b at once and
//   closes exactly (sum of pitch vectors = 0). With the cluster centres s_v = O[v] + (u', v', w') and
//   c/3 = 1.5 the point formula (1/1.5) sum p_v (s_v - c/3) gives, by hand:
//     x = (1/1.5)[0.375 - 1.5 u'] = 0.25 - u'
//     y = (0.8660254/1.5)(s_V - s_W) = 0.5773503 (-10.25 + u' + 2 v')
//   [11] run constants   [12] clusters   [13] c   [14] line   [15] ghosts   [16] flat segment
//   [17] ranges   [18] outputs   [19] checks leave no directory
// =============================================================================
#include "drawUVW.C"
#include "analyzeUVW.C"

#include <TSystem.h>

#include <cstdio>
#include <fstream>
#include <iostream>
#include <sstream>
#include <string>

static int g_failures = 0;
static void check(bool ok, const std::string &name)
{
  std::printf("%s  %s\n", ok ? "PASS" : "FAIL", name.c_str());
  if (!ok) ++g_failures;
}

static Short_t g_adc[4][68][512];
static int g_view[4][68], g_strip[4][68];

static void clearAll()
{
  for (int a = 0; a < 4; ++a)
    for (int c = 0; c < 68; ++c) {
      g_view[a][c] = -1;
      g_strip[a][c] = 0;
      for (int t = 0; t < 512; ++t) g_adc[a][c][t] = 0;
    }
}
static void setCh(int a, int c, int t, int v) { g_adc[a][c][t] = (Short_t)v; }
static void flat(int a, int c, int v)
{
  for (int t = 0; t < 512; ++t) g_adc[a][c][t] = (Short_t)v;
}

struct Hists {
  TH2D *h[3];
  Hists(int nU, int nV, int nW)
  {
    int n[3] = {nU, nV, nW};
    uvwdrawMakeHists(n, h);
  }
  ~Hists()
  {
    for (int v = 0; v < 3; ++v) delete h[v];
  }
};

static std::string readFile(const std::string &p)
{
  std::ifstream f(p, std::ios::binary);
  std::stringstream ss;
  ss << f.rdbuf();
  return ss.str();
}
static int countOf(const std::string &s, const std::string &pat)
{
  int n = 0;
  for (size_t p = s.find(pat); p != std::string::npos; p = s.find(pat, p + 1)) ++n;
  return n;
}
// number of pages: "/Type /Page" not followed by 's' (i.e. not "/Type /Pages")
static int pdfPages(const std::string &s)
{
  int n = 0;
  const std::string pat = "/Type /Page";
  for (size_t p = s.find(pat); p != std::string::npos; p = s.find(pat, p + 1)) {
    const size_t e = p + pat.size();
    if (e >= s.size() || s[e] != 's') ++n;
  }
  return n;
}
static bool exists(const std::string &p) { return !gSystem->AccessPathName(p.c_str()); }

// synthetic raw file: eventId firstId..; one pulse (cells 100..119, 500 + 10 e on a baseline of 100)
// on the U, V, W channels of the 3D point (0, 0) of the rich map (strip 31 of each plane)
static void writeRaw(const std::string &path, int nEvents, int firstId)
{
  TFile f(path.c_str(), "RECREATE");
  TTree t("raw", "synthetic");
  UInt_t eventId;
  static Short_t adc[4][68][512];
  t.Branch("eventId", &eventId, "eventId/i");
  t.Branch("adc", adc, "adc[4][68][512]/S");
  for (int e = 0; e < nEvents; ++e) {
    eventId = firstId + e;
    for (int a = 0; a < 4; ++a)
      for (int c = 0; c < 68; ++c)
        for (int s = 0; s < 512; ++s) adc[a][c][s] = 100;
    for (int s = 100; s < 120; ++s) {
      adc[0][30][s] = (Short_t)(500 + 10 * e);  // U strip 31 = rich-map index 30
      adc[1][27][s] = (Short_t)(500 + 10 * e);  // V strip 31 = index 61 + 30 = 91 -> aget 1, ch 27
      adc[2][24][s] = (Short_t)(500 + 10 * e);  // W strip 31 = index 122 + 30 = 152 -> aget 2, ch 24
    }
    t.Fill();
  }
  t.Write();
  f.Close();
}

static const double O[3] = {1.25, -3.5, 6.75};  // plane origins, sum = 4.5
static const double C0 = 4.5;
static const double RUN_P = 198, RUN_F = 12.5, RUN_HV = 1600;

static std::vector<double> richS(int v)
{
  std::vector<double> s;
  for (int k = 1; k <= 61; ++k) s.push_back(O[v] + 2.0 * (k - 31));
  return s;
}
// rich map file: 3 planes x 61 strips, one raw channel each (index -> aget = idx / 64, ch = idx % 64)
static void writeRichMap()
{
  std::ofstream m("channel_map.csv");
  m << "# aget,raw_channel,view(0=U 1=V 2=W),strip,s_mm\n";
  int idx = 0;
  for (int v = 0; v < 3; ++v)
    for (int k = 1; k <= 61; ++k, ++idx) m << idx / 64 << "," << idx % 64 << "," << v << "," << k << "," << O[v] + 2.0 * (k - 31) << "\n";
}
static void setV(Hists &H, int v, int strip, int cell, double val) { H.h[v]->SetBinContent(cell + 1, strip, val); }
// clusters of the 3 planes at a cell -> points
static std::vector<UvwdrawPoint> pointsAt(Hists &H, int cell, double c, double mm)
{
  std::vector<double> s[3] = {richS(0), richS(1), richS(2)};
  std::vector<UvwdrawCluster> cl[3];
  for (int v = 0; v < 3; ++v) cl[v] = uvwdrawClustersAtCell(H.h[v], s[v], 2.0, cell);
  std::vector<UvwdrawPoint> pts;
  uvwdrawMatchCell(cl, c, cell * mm, pts);
  return pts;
}

int test_drawUVW()
{
  const std::string dir = std::string(gSystem->TempDirectory()) + "/test_drawUVW_" + std::to_string(gSystem->GetPid());
  gSystem->mkdir(dir.c_str(), true);
  const std::string startDir = gSystem->WorkingDirectory();

  // ---- [1]-[5] fillUVW ------------------------------------------------------
  {  // [1] baseline = mean of cells 0..24
    clearAll();
    g_view[0][0] = 0;
    g_strip[0][0] = 1;
    flat(0, 0, 100);
    setCh(0, 0, 200, 300);
    Hists H(2, 2, 2);
    fillUVW(g_adc, g_view, g_strip, H.h);
    check(H.h[0]->GetBinContent(1, 1) == 0, "[1] cell 0 = 0 after baseline");
    check(H.h[0]->GetBinContent(201, 1) == 200, "[1] cell 200 = 300 - 100 = 200");
    check(H.h[0]->GetBinContent(1, 2) == 0, "[1] strip without channel stays 0");
  }
  {  // [2] baseline uses only the first 25 cells
    clearAll();
    g_view[1][3] = 1;
    g_strip[1][3] = 2;
    flat(1, 3, 100);
    for (int t = 1; t <= 24; ++t) setCh(1, 3, t, (t % 2) ? 90 : 110);
    setCh(1, 3, 25, 1000);
    Hists H(2, 2, 2);
    fillUVW(g_adc, g_view, g_strip, H.h);
    check(H.h[1]->GetBinContent(26, 2) == 900, "[2] cell 25 = 1000 - 100 = 900");
    check(H.h[1]->GetBinContent(31, 2) == 0, "[2] cell 30 = 100 - 100 = 0");
  }
  {  // [3] negative values kept
    clearAll();
    g_view[2][5] = 2;
    g_strip[2][5] = 1;
    flat(2, 5, 100);
    setCh(2, 5, 300, 40);
    Hists H(2, 2, 2);
    fillUVW(g_adc, g_view, g_strip, H.h);
    check(H.h[2]->GetBinContent(301, 1) == -60, "[3] cell 300 = 40 - 100 = -60");
  }
  {  // [4] FPN (raw 11) and unmapped channels do not enter any plane
    clearAll();
    g_view[0][0] = 0;
    g_strip[0][0] = 1;
    flat(0, 0, 100);
    flat(0, 11, 5000);  // FPN: view = -1
    flat(0, 40, 5000);  // not in the map
    setCh(0, 11, 300, 9000);
    Hists H(2, 2, 2);
    fillUVW(g_adc, g_view, g_strip, H.h);
    double sum = 0;
    for (int v = 0; v < 3; ++v)
      for (int x = 1; x <= 512; ++x)
        for (int y = 1; y <= 2; ++y) sum += std::fabs(H.h[v]->GetBinContent(x, y));
    check(sum == 0, "[4] FPN / unmapped channels contribute nothing (flat mapped ch -> all 0)");
  }
  {  // [5] several channels on one strip: sum, clamp, then baseline
    clearAll();
    for (int c = 0; c < 3; ++c) {
      g_view[0][c] = 0;
      g_strip[0][c] = 1;
      flat(0, c, 10);
      setCh(0, c, 300, 30000);
    }
    // strip 2: 2 channels 10000 + 20000 = 30000 (< 65535), 10 + 10 elsewhere
    g_view[0][3] = 0;
    g_strip[0][3] = 2;
    flat(0, 3, 10);
    setCh(0, 3, 300, 10000);
    g_view[0][4] = 0;
    g_strip[0][4] = 2;
    flat(0, 4, 10);
    setCh(0, 4, 300, 20000);
    Hists H(2, 2, 2);
    fillUVW(g_adc, g_view, g_strip, H.h);
    check(H.h[0]->GetBinContent(301, 1) == 65535 - 30, "[5] 3 x 30000 clamped to 65535, baseline 30 -> 65505");
    check(H.h[0]->GetBinContent(301, 2) == 30000 - 20, "[5] 2 channels, sum below clamp: 30000 - 20 = 29980");
    check(H.h[0]->GetBinContent(1, 1) == 0, "[5] baseline cell -> 0");
  }

  // ---- map reader + [6] nStrips / holes + [7] axes ----------------------------
  gSystem->ChangeDirectory(dir.c_str());
  {
    std::ofstream m("channel_map.csv");
    m << "# aget,raw_channel,view(0=U 1=V 2=W),strip,s_mm\n";
    m << "0,0,0,1,0.0\n0,1,0,2,1.5\n0,2,0,5,6.0\n";  // U: strips 1, 2, 5 (holes at 3, 4)
    m << "1,0,1,3,0.0\n";                              // V: strip 3
    m << "1,12,2,2,0.0\n";                             // W: strip 2 (raw ch 12, after FPN 11)
  }
  int view[4][68], strip[4][68], nStrips[3];
  std::vector<double> sPos[3];
  check(uvwdrawLoadMap("channel_map.csv", view, strip, nStrips, sPos), "[6] synthetic map loads");
  check(sPos[0].size() == 5 && sPos[0][4] == 6.0 && sPos[0][1] == 1.5 && std::isnan(sPos[0][2]) && sPos[2][1] == 0.0,
        "[6] s_mm per strip is kept (NaN where no channel)");
  check(nStrips[0] == 5 && nStrips[1] == 3 && nStrips[2] == 2, "[6] nStrips = max strip number (5, 3, 2), not count");
  check(view[0][2] == 0 && strip[0][2] == 5 && view[0][3] == -1, "[6] map lookup / unmapped = -1");
  {
    clearAll();
    for (int a = 0; a < 4; ++a)
      for (int c = 0; c < 68; ++c) {
        g_view[a][c] = view[a][c];
        g_strip[a][c] = strip[a][c];
      }
    flat(0, 0, 100);
    setCh(0, 0, 50, 400);  // U strip 1
    flat(0, 2, 100);
    setCh(0, 2, 50, 700);  // U strip 5
    Hists H(nStrips[0], nStrips[1], nStrips[2]);
    fillUVW(g_adc, g_view, g_strip, H.h);
    check(H.h[0]->GetNbinsY() == 5 && H.h[0]->GetYaxis()->GetXmin() == 1 && H.h[0]->GetYaxis()->GetXmax() == 6,
          "[6] y axis = 5 bins [1, 6)");
    bool holes0 = true;
    for (int x = 1; x <= 512; ++x)
      for (int y = 3; y <= 4; ++y)
        if (H.h[0]->GetBinContent(x, y) != 0) holes0 = false;
    check(holes0, "[6] strips without a channel (3, 4) are all 0");
    check(H.h[0]->GetBinContent(51, 1) == 300 && H.h[0]->GetBinContent(51, 5) == 600, "[6] strips 1 and 5 filled (300, 600)");
    // [7] axes and titles
    check(H.h[0]->GetNbinsX() == 512 && H.h[0]->GetXaxis()->GetXmin() == 0 && H.h[0]->GetXaxis()->GetXmax() == 512,
          "[7] x axis = 512 bins [0, 512)");
    const std::string t = uvwdrawTitle(77, 5, 1);
    check(t.find("event 77") != std::string::npos && t.find("entry 5") != std::string::npos && t.find("V") != std::string::npos,
          "[7] title has eventId, entry and plane name: '" + t + "'");
  }

  // ---- [11] run constants ------------------------------------------------------------
  writeRichMap();
  writeRaw("syn_raw.root", 3, 10);
  writeRaw("neg_raw.root", 1, 1);
  drawUVW("neg_raw.root", 0, RUN_F, RUN_HV, "d11a");
  drawUVW("neg_raw.root", -5, RUN_F, RUN_HV, "d11a2");
  drawUVW("neg_raw.root", RUN_P, 30, RUN_HV, "d11b");
  drawUVW("neg_raw.root", RUN_P, RUN_F, 0, "d11c");
  drawUVW("neg_raw.root", RUN_P, RUN_F, -1600, "d11d.pdf");
  drawUVW("neg_raw.root", 0, RUN_F, RUN_HV);  // default name too
  check(!exists("d11a") && !exists("d11a2") && !exists("d11b") && !exists("d11c") && !exists("d11d.pdf") && !exists("neg_uvw_events"),
        "[11] pressure <= 0 / sampling 30 / HV <= 0 -> ERROR, no directory or file");
  {
    const double mm = uvwdrawMmPerCell(198, 12.5, 1600);
    check(std::fabs(mm - driftVelocityCmUs(1600, 20, 198) / 12.5 * 10) < 1e-12, "[11] mmPerCell = analyzeUVW.C driftVelocityCmUs(1600, 20, 198) / 12.5 * 10");
    // by hand: E/p = 1600/20/198 = 0.40404, v = 0.7233 * 0.40404 = 0.292242 cm/us, / 12.5 * 10 = 0.233794 mm
    check(std::fabs(mm - 0.233794) < 1e-5, "[11] mmPerCell ~ 0.233794 mm (hand value)");
  }

  // ---- [12] clusters -------------------------------------------------------------------
  {
    std::vector<double> s;
    for (int k = 1; k <= 20; ++k) s.push_back(2.0 * (k - 1));  // strip k at 2 (k-1): strip 3 = 4, 5 = 8, 6 = 10
    check(uvwdrawPitch(s) == 2.0, "[12] pitch = smallest neighbour difference = 2 mm");
    Hists H(20, 20, 20);
    setV(H, 0, 3, 10, 100);  // one missing strip (4) between 3 and 5: gap 4 = 2 pitch -> one cluster
    setV(H, 0, 5, 10, 200);
    auto c1 = uvwdrawClustersAtCell(H.h[0], s, 2.0, 10);
    // Q = 300, centre = (100*4 + 200*8) / 300 = 2000/300
    check(c1.size() == 1 && std::fabs(c1[0].q - 300) < 1e-9 && std::fabs(c1[0].s - 2000.0 / 300) < 1e-9, "[12] one strip missing -> 1 cluster, Q 300, centre 6.6667");
    setV(H, 0, 3, 20, 200);  // strips 3 (s 4) and 6 (s 10): gap 6 > 4.1 -> two clusters
    setV(H, 0, 6, 20, 160);
    auto c2 = uvwdrawClustersAtCell(H.h[0], s, 2.0, 20);
    check(c2.size() == 2 && c2[0].s == 4.0 && c2[0].q == 200 && c2[1].s == 10.0 && c2[1].q == 160, "[12] two strips missing -> 2 clusters (4 mm / 200, 10 mm / 160)");
    setV(H, 0, 3, 30, 149);  // Q < 150 dropped
    check(uvwdrawClustersAtCell(H.h[0], s, 2.0, 30).empty(), "[12] Q = 149 < 150 is dropped");
    setV(H, 0, 3, 31, 150);
    check(uvwdrawClustersAtCell(H.h[0], s, 2.0, 31).size() == 1, "[12] Q = 150 is kept");
    setV(H, 0, 3, 40, 200);  // strip 4 = 35 is NOT a hit (v > 35), strip 5 = 36 is
    setV(H, 0, 4, 40, 35);
    setV(H, 0, 5, 40, 36);
    auto c3 = uvwdrawClustersAtCell(H.h[0], s, 2.0, 40);
    // Q = 236 (35 is not added), centre = (200*4 + 36*8) / 236 = 1088/236
    check(c3.size() == 1 && c3[0].q == 236 && std::fabs(c3[0].s - 1088.0 / 236) < 1e-9, "[12] v <= 35 does not enter (Q 236, centre 4.6102)");
  }

  // ---- [13] closure constant -----------------------------------------------------------
  {
    std::vector<UvwdrawCell> cells;
    for (int i = 0; i < 3; ++i) {  // single clusters, s = O + (2a, 2b, -2a-2b): sum = 4.5 for every point
      const double up = 2.0 * (i + 1), vp = 2.0 * (3 - i);
      UvwdrawCell c;
      c.cl[0].push_back({O[0] + up, 200});
      c.cl[1].push_back({O[1] + vp, 200});
      c.cl[2].push_back({O[2] - up - vp, 200});
      cells.push_back(c);
    }
    for (int i = 0; i < 5; ++i) {  // cells with 2 clusters in U: sum would be 100+ -> must not be used
      UvwdrawCell c;
      c.cl[0].push_back({100.0 + i, 200});
      c.cl[0].push_back({0.0, 200});
      c.cl[1].push_back({0.0, 200});
      c.cl[2].push_back({0.0, 200});
      cells.push_back(c);
    }
    int nUsed = -1;
    const double c = uvwdrawClosureC(cells, &nUsed);
    check(nUsed == 3 && std::fabs(c - 4.5) < 1e-9, "[13] c = 4.5 from the 3 single-cluster cells only (got " + std::to_string(c) + ", n " + std::to_string(nUsed) + ")");
    std::vector<UvwdrawCell> none;
    check(uvwdrawClosureC(none, &nUsed) == 0 && nUsed == 0, "[13] no usable cell -> c = 0, n = 0");
  }

  // ---- [14] one straight 3D line, cells 100..139 -------------------------------------------
  {
    Hists H(61, 61, 61);
    const double mm = 0.25;
    for (int i = 0; i < 40; ++i) {  // a = i - 20, b = 3
      const int a = i - 20, b = 3, cell = 100 + i;
      setV(H, 0, a + 31, cell, 200);
      setV(H, 1, b + 31, cell, 300);
      setV(H, 2, 31 - a - b, cell, 400);
    }
    bool ok = true;
    int total = 0;
    for (int cell = 0; cell < 512; ++cell) {
      auto pts = pointsAt(H, cell, C0, mm);
      total += (int)pts.size();
      if (cell < 100 || cell >= 140) {
        if (!pts.empty()) ok = false;
        continue;
      }
      if (pts.size() != 1) {
        ok = false;
        continue;
      }
      const int a = cell - 100 - 20;
      const double up = 2.0 * a, vp = 6.0;
      const double x = 0.25 - up, y = 0.8660254 / 1.5 * (-10.25 + up + 2 * vp), z = cell * mm;
      if (std::fabs(pts[0].x - x) > 1e-6 || std::fabs(pts[0].y - y) > 1e-6 || std::fabs(pts[0].z - z) > 1e-6 || std::fabs(pts[0].w - 300) > 1e-9) ok = false;
    }
    check(ok && total == 40, "[14] one point per cell for 40 cells, (x, y, z, w) = hand values (total " + std::to_string(total) + ")");
    // cell 100: a = -20, b = 3 -> u' = -40, v' = 6: x = 0.25 + 40 = 40.25; y = 0.5773503 (-10.25 - 40 + 12) = -22.0837
    auto p0 = pointsAt(H, 100, C0, mm);
    check(p0.size() == 1 && std::fabs(p0[0].x - 40.25) < 1e-6 && std::fabs(p0[0].y - (-22.0837)) < 1e-3 && std::fabs(p0[0].z - 25.0) < 1e-9, "[14] cell 100: (40.25, -22.0837, 25.0)");
  }

  // ---- [15] ghosts: two points in one cell, 8 triples, only the 2 real ones close ----------------
  {
    Hists H(61, 61, 61);
    // point 1: (a, b) = (0, 0); point 2: (a, b) = (3, 3). Wrong triples deviate by 2 (a1-a2) = 6, 2 (b1-b2) = 6
    // or 2 ((a1+b1)-(a2+b2)) = 12 mm (all > 2); the real clusters are 6 mm apart (> 4.1: not merged)
    setV(H, 0, 31, 50, 200);
    setV(H, 1, 31, 50, 300);
    setV(H, 2, 31, 50, 400);
    setV(H, 0, 34, 50, 210);
    setV(H, 1, 34, 50, 310);
    setV(H, 2, 25, 50, 410);  // k = 31 - 6
    auto pts = pointsAt(H, 50, C0, 0.25);
    std::sort(pts.begin(), pts.end(), [](const UvwdrawPoint &p, const UvwdrawPoint &q) { return p.x > q.x; });
    // point 1: u' = 0, v' = 0: x = 0.25, w = 300. point 2: u' = 6, v' = 6: x = -5.75, w = (210+310+410)/3 = 310
    check(pts.size() == 2 && std::fabs(pts[0].x - 0.25) < 1e-6 && std::fabs(pts[0].w - 300) < 1e-9 && std::fabs(pts[1].x - (-5.75)) < 1e-6 && std::fabs(pts[1].w - 310) < 1e-9,
          "[15] 8 triples -> exactly the 2 real points (got " + std::to_string(pts.size()) + ")");
  }

  // ---- [16] flat segment: one cell holds a whole segment -> one point at its centroid ----------------
  {
    Hists H(61, 61, 61);
    // (a, b) = (j, j), j = 0..11 with 100 each: U strips 31..42 (u' = 2j), V the same (v' = 2j), W strips
    // 31 - 2j (w' = -4j): every plane has >= 10 hit strips; centres u' = v' = 11, w' = -22 (mean of j = 5.5)
    for (int j = 0; j < 12; ++j) {
      setV(H, 0, 31 + j, 70, 100);
      setV(H, 1, 31 + j, 70, 100);
      setV(H, 2, 31 - 2 * j, 70, 100);
    }
    auto pts = pointsAt(H, 70, C0, 0.25);
    // x = 0.25 - 11 = -10.75; y = 0.5773503 (-10.25 + 11 + 22) = 0.5773503 * 22.75 = 13.1349; Q = 1200 each
    check(pts.size() == 1 && std::fabs(pts[0].x - (-10.75)) < 1e-6 && std::fabs(pts[0].y - 13.1349) < 1e-3 && std::fabs(pts[0].w - 1200) < 1e-9,
          "[16] wide flat segment (12 strips) -> 1 point (-10.75, 13.1349), weight 1200");
  }

  // ---- [17] ranges ------------------------------------------------------------------------------------
  {
    std::vector<double> s[3] = {richS(0), richS(1), richS(2)};
    double r[4];
    check(uvwdrawRanges(s, C0, r), "[17] ranges found");
    // s_U in [-58.75, 61.25] -> x = -s_U in [-61.25, 58.75], +-5 -> [-66.25, 63.75].
    // y = 0.5773503 (s_V - s_W), s_W = 4.5 - s_U - s_V must be in W's [-53.25, 66.75]:
    //   s_V - s_W = 2 s_V + s_U - 4.5; max at s_V = 56.5, s_U = 1.25 (s_W = -53.25): 109.75;
    //   min at s_V = -63.5, s_U = 1.25 (s_W = 66.75): -130.25. y in [-75.2, 63.4]-ish +-5
    const double k = 0.8660254 / 1.5;
    check(std::fabs(r[0] - (-66.25)) < 1e-9 && std::fabs(r[1] - 63.75) < 1e-9, "[17] x range [-66.25, 63.75]");
    check(std::fabs(r[2] - (k * -130.25 - 5)) < 1e-6 && std::fabs(r[3] - (k * 109.75 + 5)) < 1e-6, "[17] y range [-130.25 k - 5, 109.75 k + 5], k = 0.5773503");
    TH2D *hx[3];
    uvwdrawMakeXYZ(r, 120.0, hx);
    check(hx[1]->GetNbinsY() == 120 && hx[1]->GetYaxis()->GetXmax() == 120.0 && hx[0]->GetNbinsX() == 130, "[17] z axis [0, 120] in 1 mm bins; x: 130 bins over 130 mm");
    std::vector<UvwdrawPoint> pts = {{1.2, -3.4, 7.7, 5.0}, {1.2, -3.4, 7.7, 2.0}};
    uvwdrawFillPoints(pts, hx);
    check(hx[0]->GetBinContent(hx[0]->FindBin(1.2, -3.4)) == 7.0 && hx[1]->GetBinContent(hx[1]->FindBin(1.2, 7.7)) == 7.0 && hx[2]->GetBinContent(hx[2]->FindBin(-3.4, 7.7)) == 7.0,
          "[17] points fill XY / XZ / YZ with weights (5 + 2 = 7)");
    uvwdrawFillPoints({}, hx);
    check(hx[0]->GetEntries() == 0 && hx[1]->Integral() == 0, "[17] fill resets first");
    for (int i = 0; i < 3; ++i) delete hx[i];
    std::vector<double> bad[3] = {s[0], s[1], std::vector<double>(3, 1000.0)};  // W far away: nothing closes
    check(!uvwdrawRanges(bad, C0, r), "[17] no pair closes -> false");
  }

  // ---- [8] [18] outputs -------------------------------------------------------------------------------------
  drawUVW("syn_raw.root", RUN_P, RUN_F, RUN_HV);
  check(exists("syn_uvw_events/event0000010.png") && exists("syn_uvw_events/event0000011.png") && exists("syn_uvw_events/event0000012.png"),
        "[8][18] default out: syn_raw.root -> syn_uvw_events/event0000010..12.png");
  check(std::string(gSystem->GetFromPipe("ls syn_uvw_events | wc -l").Data()).find('3') != std::string::npos && !exists("syn_uvw_events/event0000013.png"),
        "[18] exactly 3 PNG files");
  {
    const std::string s = readFile("syn_uvw_events/event0000011.png");
    auto be = [&](int o) { return ((unsigned char)s[o] << 24) | ((unsigned char)s[o + 1] << 16) | ((unsigned char)s[o + 2] << 8) | (unsigned char)s[o + 3]; };
    check(s.size() > 24 && s.substr(1, 3) == "PNG" && be(16) == 1200 && be(20) == 600, "[18] PNG is 1200 x 600");
  }
  drawUVW("syn_raw.root", RUN_P, RUN_F, RUN_HV, "a/b/c");
  check(exists("a/b/c/event0000010.png") && exists("a/b/c/event0000012.png"), "[18] nested path a/b/c is made");
  drawUVW("syn_raw.root", RUN_P, RUN_F, RUN_HV, "a/b/c", 1, 1);  // existing directory is used, files kept
  check(exists("a/b/c/event0000010.png") && exists("a/b/c/event0000011.png"), "[18] existing directory is reused");
  {
    std::ofstream("conflict") << "x";
    drawUVW("syn_raw.root", RUN_P, RUN_F, RUN_HV, "conflict");
    check(readFile("conflict") == "x", "[18] a FILE with the directory name -> ERROR, file untouched");
    gSystem->mkdir("conflict_parent", true);
    std::ofstream("conflict_parent/f") << "y";
    drawUVW("syn_raw.root", RUN_P, RUN_F, RUN_HV, "conflict_parent/f");
    check(readFile("conflict_parent/f") == "y", "[18] same for a file in a subdirectory");
  }
  drawUVW("syn_raw.root", RUN_P, RUN_F, RUN_HV, "syn.pdf");
  {
    const std::string s = readFile("syn.pdf");
    check(pdfPages(s) == 3, "[8][18] .pdf -> 3 pages (got " + std::to_string(pdfPages(s)) + ")");
    check(s.find("event 10") != std::string::npos && s.find("event 11") != std::string::npos && s.find("event 12") != std::string::npos, "[8][18] bookmarks event 10..12");
  }
  drawUVW("syn_raw.root", RUN_P, RUN_F, RUN_HV, "one.pdf", 1, 1);
  {
    const std::string s = readFile("one.pdf");
    check(pdfPages(s) == 1, "[8] first=1 n=1 -> 1 page");
    check(s.find("event 11") != std::string::npos && s.find("event 10") == std::string::npos, "[8] only event 11");
  }

  // ---- [10] drawMinAdc / logz -------------------------------------------------------
  drawUVW("syn_raw.root", RUN_P, RUN_F, RUN_HV, "min20.pdf", 0, -1, 20);
  check(pdfPages(readFile("min20.pdf")) == 3, "[10] drawMinAdc = 20: 3 pages");
  drawUVW("syn_raw.root", RUN_P, RUN_F, RUN_HV, "logz.pdf", 0, -1, -1, true);
  check(pdfPages(readFile("logz.pdf")) == 3, "[10] logz: 3 pages");

  // ---- [9] [19] errors: nothing is created ------------------------------------------------
  drawUVW("syn_raw.root", RUN_P, RUN_F, RUN_HV, "bad1.pdf", 3, 1);
  drawUVW("syn_raw.root", RUN_P, RUN_F, RUN_HV, "bad2.pdf", -1, 1);
  drawUVW("syn_raw.root", RUN_P, RUN_F, RUN_HV, "bad3.pdf", 0, 0);
  check(!exists("bad1.pdf") && !exists("bad2.pdf") && !exists("bad3.pdf"), "[9] first >= nTotal / first < 0 / n == 0 -> no PDF");
  drawUVW("syn_raw.root", RUN_P, RUN_F, RUN_HV, "d19a", 3, 1);
  drawUVW("syn_raw.root", RUN_P, RUN_F, RUN_HV, "d19b", -1, 1);
  drawUVW("syn_raw.root", RUN_P, RUN_F, RUN_HV, "d19c", 0, 0);
  {
    TFile f("notree.root", "RECREATE");
    f.Close();
  }
  drawUVW("notree.root", RUN_P, RUN_F, RUN_HV, "bad4.pdf");
  drawUVW("notree.root", RUN_P, RUN_F, RUN_HV, "d19d");
  check(!exists("bad4.pdf") && !exists("d19a") && !exists("d19b") && !exists("d19c") && !exists("d19d"), "[9][19] range errors / no tree 'raw' -> no file, no directory");
  writeRaw("noext.root", 1, 1);  // no '_raw.root' in the name
  drawUVW("noext.root", RUN_P, RUN_F, RUN_HV);
  check(!exists("noext_uvw_events") && !exists("noext.root_uvw_events") && !exists("noext_uvw_events.pdf"), "[9][19] no '_raw.root' in the name and no out -> nothing created");
  {
    std::string before = readFile("syn_raw.root");
    drawUVW("syn_raw.root", RUN_P, RUN_F, RUN_HV, "syn_raw.root");
    check(readFile("syn_raw.root") == before, "[9] output name == input name -> input untouched");
  }
  gSystem->mkdir("nomap", true);
  gSystem->ChangeDirectory("nomap");
  drawUVW("../syn_raw.root", RUN_P, RUN_F, RUN_HV, "../bad5.pdf");
  drawUVW("../syn_raw.root", RUN_P, RUN_F, RUN_HV, "../d19e");
  gSystem->ChangeDirectory("..");
  check(!exists("bad5.pdf") && !exists("d19e"), "[9][19] no channel_map.csv in the cwd -> nothing created");

  // ---- TODO/193: mm axes, auto zoom, equal mm per pixel -------------------------------------------
  auto near = [](double a, double b) { return std::fabs(a - b) < 1e-9; };
  {  // [20] mm axes. rich plane U: s_k = 1.25 + 2 (k - 31), k = 1..61 -> slope 2, offset 1.25 - 62 = -60.75
    const double mm = 0.25;
    UvwdrawPlane pls[3];
    bool ok = true;
    for (int v = 0; v < 3; ++v) ok = uvwdrawFitPlane(richS(v), "UVW"[v], pls[v]) && ok;
    check(ok && near(pls[0].slope, 2) && near(pls[0].offset, -60.75) && pls[0].n == 61 && !pls[0].rev && near(pls[0].pitch, 2),
          "[20] plane fit: slope 2, offset -60.75, 61 strips, pitch 2");
    int ns[3] = {61, 61, 61};
    TH2D *hs[3], *hd[3];
    uvwdrawMakeHists(ns, hs);
    uvwdrawMakeDispHists(pls, mm, hd);
    // s(1) = -58.75, s(61) = 61.25 -> y axis [-58.75 - 1, 61.25 + 1] = [-59.75, 62.25]; x axis [0, 512 * 0.25 = 128]
    check(near(hd[0]->GetYaxis()->GetXmin(), -59.75) && near(hd[0]->GetYaxis()->GetXmax(), 62.25) && hd[0]->GetNbinsY() == 61,
          "[20] top y axis = [s(1) - 1, s(n) + 1] = [-59.75, 62.25], 61 bins");
    check(near(hd[0]->GetXaxis()->GetXmin(), 0) && near(hd[0]->GetXaxis()->GetXmax(), 512 * mm) && hd[0]->GetNbinsX() == 512,
          "[20] top x axis = [0, 512 * mmPerCell] = [0, 128], 512 bins");
    check(near(hd[1]->GetYaxis()->GetXmin(), -3.5 - 60 - 1) && near(hd[2]->GetYaxis()->GetXmax(), 6.75 + 60 + 1),
          "[20] V / W y axes from their own offsets (-64.5, 67.75)");
    hs[0]->SetBinContent(101, 5, 700);  // cell 100, strip 5 -> s_5 = 1.25 + 2 (5 - 31) = -50.75
    uvwdrawToDisp(hs, pls, hd);
    const int b = hd[0]->FindBin(100 * mm + 0.1, -50.75);  // z in cell 100 = [25, 25.25)
    check(hd[0]->GetBinContent(b) == 700 && hd[0]->Integral() == 700, "[20] strip 5 cell 100 lands in the bin of s_5 = -50.75 (only one non-zero bin)");
    check(hd[0]->GetBinContent(hd[0]->FindBin(100 * mm + 0.1, -48.75)) == 0, "[20] the neighbour strip (s = -48.75) stays 0");
    const std::string t = uvwdrawTitle(77, 5, 1);
    check(t.find("z [mm]") != std::string::npos && t.find("V [mm]") != std::string::npos, "[20] axis names z [mm] / V [mm]: '" + t + "'");
    for (int v = 0; v < 3; ++v) {
      delete hs[v];
      delete hd[v];
    }
  }
  {  // [21] negative slope: s = -2 strip + 100, strips 1..10 -> s = 98 ... 80; y axis [79, 99]; low s first
    std::vector<double> s;
    for (int k = 1; k <= 10; ++k) s.push_back(100.0 - 2 * k);
    UvwdrawPlane pls[3];
    bool ok = true;
    for (int v = 0; v < 3; ++v) ok = uvwdrawFitPlane(s, "UVW"[v], pls[v]) && ok;
    check(ok && near(pls[0].slope, -2) && near(pls[0].offset, 100) && pls[0].rev && near(pls[0].pitch, 2), "[21] negative slope fitted (slope -2, offset 100, reversed)");
    int ns[3] = {10, 10, 10};
    TH2D *hs[3], *hd[3];
    uvwdrawMakeHists(ns, hs);
    uvwdrawMakeDispHists(pls, 0.25, hd);
    check(near(hd[0]->GetYaxis()->GetXmin(), 79) && near(hd[0]->GetYaxis()->GetXmax(), 99), "[21] y axis [s(10) - 1, s(1) + 1] = [79, 99]");
    hs[0]->SetBinContent(8, 1, 11);   // cell 7, strip 1 (s = 98)
    hs[0]->SetBinContent(8, 10, 22);  // cell 7, strip 10 (s = 80)
    uvwdrawToDisp(hs, pls, hd);
    check(hd[0]->GetBinContent(hd[0]->FindBin(7 * 0.25 + 0.1, 98)) == 11 && hd[0]->GetBinContent(hd[0]->FindBin(7 * 0.25 + 0.1, 80)) == 22 &&
              hd[0]->GetBinContent(8, 10) == 11 && hd[0]->GetBinContent(8, 1) == 22,
          "[21] strip 1 (s 98) is in the LAST y bin, strip 10 (s 80) in the first");
    for (int v = 0; v < 3; ++v) {
      delete hs[v];
      delete hd[v];
    }
  }
  {  // [22] a map that is not on a straight line (one s shifted by 0.3 mm) -> ERROR, nothing made
    std::vector<double> s = richS(1);
    s[20] += 0.3;
    UvwdrawPlane pl;
    check(!uvwdrawFitPlane(s, 'V', pl), "[22] s_mm off the line by 0.3 mm -> fit refused");
    std::vector<double> one(5, std::nan(""));
    one[2] = 1.0;
    check(!uvwdrawFitPlane(one, 'V', pl), "[22] a single strip cannot give a pitch -> refused");
    {
      std::ofstream m("channel_map.csv");
      m << "# aget,raw_channel,view(0=U 1=V 2=W),strip,s_mm\n";
      int idx = 0;
      for (int v = 0; v < 3; ++v)
        for (int k = 1; k <= 61; ++k, ++idx)
          m << idx / 64 << "," << idx % 64 << "," << v << "," << k << "," << O[v] + 2.0 * (k - 31) + ((v == 1 && k == 21) ? 0.3 : 0.0) << "\n";
    }
    drawUVW("syn_raw.root", RUN_P, RUN_F, RUN_HV, "d22a");
    drawUVW("syn_raw.root", RUN_P, RUN_F, RUN_HV, "d22b.pdf");
    check(!exists("d22a") && !exists("d22b.pdf"), "[22] drawUVW with that map -> ERROR, no directory, no PDF");
    writeRichMap();
  }
  {  // [23] window, hand values. W x H = 300 x 200 px, m = 5, L_min = 20, k in mm / px
    UvwdrawPlane pl[3];
    const double lo[3] = {0, 0, 0}, hi[3] = {90, 100, 110};
    for (int v = 0; v < 3; ++v) {
      pl[v].sLo = lo[v];
      pl[v].sHi = hi[v];
    }
    UvwdrawSig sig[3] = {{true, 10, 70, 100, 150}, {true, 20, 50, 40, 70}, {true, 30, 40, 200, 210}};
    // dz = 70 - 10 = 60, ds = 50, 30, 10. k = max((60 + 10) / 300 = 0.2333, (50 + 10) / 200 = 0.3, 40 / 200, 20 / 200 = 0.1) = 0.3
    UvwdrawTopWindow w = uvwdrawTopWindow(sig, true, 120, pl, 300, 200);
    check(near(w.k, 0.3), "[23] k = 0.3 (the tallest plane decides)");
    // z: centre 40, +- 0.3 * 150 = 45 -> [-5, 85]; plane 0: centre 125, +- 0.3 * 100 = 30 -> [95, 155]; plane 1: [25, 85]; plane 2: [175, 235]
    check(near(w.z0, -5) && near(w.z1, 85) && near(w.s0[0], 95) && near(w.s1[0], 155) && near(w.s0[1], 25) && near(w.s1[1], 85) &&
              near(w.s0[2], 175) && near(w.s1[2], 235),
          "[23] window: z [-5, 85], s [95, 155] / [25, 85] / [175, 235]");
    bool same = true, hasM = w.z0 <= 10 - 5 + 1e-9 && w.z1 >= 70 + 5 - 1e-9;
    for (int v = 0; v < 3; ++v) {
      same = same && near((w.z1 - w.z0) / 300, w.k) && near((w.s1[v] - w.s0[v]) / 200, w.k);
      hasM = hasM && w.s0[v] <= sig[v].s0 - 5 + 1e-9 && w.s1[v] >= sig[v].s1 + 5 - 1e-9;
    }
    check(same, "[23] the same k in all 3 planes (mm / px in x and y)");
    check(hasM, "[23] the window holds the signal range + 5 mm on every side");
    // L_min: dz = 4, ds = 2 -> k = max(14 / 300, 12 / 200 = 0.06, 20 / 200 = 0.1) = 0.1; height 20 mm
    UvwdrawSig tiny[3] = {{true, 50, 54, 100, 102}, {true, 50, 54, 10, 12}, {true, 50, 54, 0, 2}};
    w = uvwdrawTopWindow(tiny, true, 120, pl, 300, 200);
    check(near(w.k, 0.1) && near(w.s1[0] - w.s0[0], 20) && near(w.s0[0], 91) && near(w.z0, 37) && near(w.z1, 67), "[23] L_min = 20 mm decides: k 0.1, s [91, 111], z [37, 67]");
    // zoom = false: k = max(120 / 300 = 0.4, 110 / 200 = 0.55) = 0.55; z centre 60 +- 82.5; plane 2 centre 55 +- 55 = its full range
    w = uvwdrawTopWindow(sig, false, 120, pl, 300, 200);
    check(near(w.k, 0.55) && near(w.z0, -22.5) && near(w.z1, 142.5) && near(w.s0[2], 0) && near(w.s1[2], 110) && near(w.s0[0], -10) && near(w.s1[0], 100),
          "[23] zoom = false: whole range, smallest k = 0.55 (no margin)");
    UvwdrawSig none[3] = {{false, 0, 0, 0, 0}, {false, 0, 0, 0, 0}, {false, 0, 0, 0, 0}};
    UvwdrawTopWindow wn = uvwdrawTopWindow(none, true, 120, pl, 300, 200);
    check(near(wn.k, 0.55) && near(wn.z0, w.z0) && near(wn.s1[1], w.s1[1]), "[23] no signal cell at all: same as zoom = false");
    // only plane 1 has signal: dz = 30, ds = 30 -> k = max(40 / 300, 40 / 200 = 0.2, 0.1) = 0.2; the others: centre of their full range
    UvwdrawSig one[3] = {{false, 0, 0, 0, 0}, {true, 20, 50, 40, 70}, {false, 0, 0, 0, 0}};
    w = uvwdrawTopWindow(one, true, 120, pl, 300, 200);
    check(near(w.k, 0.2) && near(w.z0, 35 - 30) && near(w.s0[1], 55 - 20) && near(w.s0[0], 45 - 20) && near(w.s1[0], 45 + 20),
          "[23] plane without signal: its window is centred on its full range, same k 0.2");
    // bottom: points x [0, 10], y [-10, 20], z [30, 90]; frames (dx, dy) = (10, 30), (10, 60), (30, 60)
    // k = max(20 / 300, 40 / 200 = 0.2, 70 / 200 = 0.35, 40 / 300, 0.1) = 0.35
    std::vector<UvwdrawPoint> pts = {{0, -10, 30, 1}, {10, 20, 90, 1}, {4, 3, 50, 1}};
    const double r[4] = {-60, 60, -40, 40};
    UvwdrawBotWindow bw = uvwdrawBotWindow(pts, true, r, 120, 300, 200);
    // x centre 5 +- 0.35 * 150 = 52.5; y centre 5: XY +- 0.35 * 100 = 35, YZ +- 52.5; z centre 60 +- 35 (XZ and YZ share it)
    check(near(bw.k, 0.35) && near(bw.xy[0], -47.5) && near(bw.xy[1], 57.5) && near(bw.xy[2], -30) && near(bw.xy[3], 40) && near(bw.xz[0], -47.5) &&
              near(bw.xz[2], 25) && near(bw.xz[3], 95) && near(bw.yz[0], -47.5) && near(bw.yz[1], 57.5) && near(bw.yz[2], 25) && near(bw.yz[3], 95),
          "[23] bottom window: k 0.35, XY / XZ / YZ ranges by hand, z shared");
    // no points: r = x [-60, 60], y [-40, 40], z [0, 120]: frames (120, 80), (120, 120), (80, 120) -> k = max(0.4, 0.4, 0.6, 80 / 300) = 0.6
    bw = uvwdrawBotWindow({}, true, r, 120, 300, 200);
    check(near(bw.k, 0.6) && near(bw.xz[0], -90) && near(bw.xz[1], 90) && near(bw.xz[2], 0) && near(bw.xz[3], 120) && near(bw.xy[2], -60) && near(bw.xy[3], 60),
          "[23] no 3D point: whole range, k 0.6, z [0, 120]");
    bw = uvwdrawBotWindow(pts, false, r, 120, 300, 200);
    check(near(bw.k, 0.6), "[23] zoom = false: whole range too");
  }
  {  // [24] signal cell rule: > max(35, 10 % of the plane maximum)
    UvwdrawPlane pls[3];
    for (int v = 0; v < 3; ++v) uvwdrawFitPlane(richS(v), "UVW"[v], pls[v]);
    const double mm = 0.25;
    TH2D *hd[3];
    uvwdrawMakeDispHists(pls, mm, hd);
    // plane 0: max 1000 -> threshold 100. 90 (strip 20, cell 50) is out, 150 (strip 40, cell 200) is in, 1000 (strip 31, cell 100) is in
    hd[0]->SetBinContent(101, hd[0]->GetYaxis()->FindBin(1.25), 1000);
    hd[0]->SetBinContent(51, hd[0]->GetYaxis()->FindBin(1.25 + 2 * (20 - 31)), 90);
    hd[0]->SetBinContent(201, hd[0]->GetYaxis()->FindBin(1.25 + 2 * (40 - 31)), 150);
    UvwdrawSig sig;
    check(uvwdrawSignalRange(hd[0], sig), "[24] a signal is found");
    // cells 100 .. 200 -> z [25, 201 * 0.25 = 50.25]; strips 31 (s 1.25) .. 40 (s 19.25) +- 1 -> [0.25, 20.25]. The 90 would reach z 12.5 / s -18.5
    check(near(sig.z0, 25) && near(sig.z1, 50.25) && near(sig.s0, 0.25) && near(sig.s1, 20.25), "[24] 90 (< 100) is out, 150 (> 100) is in: z [25, 50.25], s [0.25, 20.25]");
    // plane 1: only a 90 -> max 90, threshold max(35, 9) = 35 -> in
    hd[1]->SetBinContent(51, hd[1]->GetYaxis()->FindBin(-3.5), 90);
    check(uvwdrawSignalRange(hd[1], sig) && near(sig.z0, 12.5) && near(sig.z1, 12.75), "[24] a plane whose maximum is 90: threshold 35, the 90 is a signal cell");
    hd[2]->SetBinContent(51, hd[2]->GetYaxis()->FindBin(6.75), 35);
    check(!uvwdrawSignalRange(hd[2], sig), "[24] only 35 (not > 35) -> no signal");
    for (int v = 0; v < 3; ++v) delete hd[v];
  }
  {  // [27]-[30] TODO/199: connected components (8 neighbours) of the signal cells; 3D points in blobs (single link)
    UvwdrawPlane pls[3];
    for (int v = 0; v < 3; ++v) uvwdrawFitPlane(richS(v), "UVW"[v], pls[v]);
    const double mm = 0.25;
    TH2D *hd[3];
    uvwdrawMakeDispHists(pls, mm, hd);
    TH2D *h = hd[0];
    auto clear = [&]() { h->Reset(); };
    auto zlo = [&](int bx) { return h->GetXaxis()->GetBinLowEdge(bx); };
    auto zhi = [&](int bx) { return h->GetXaxis()->GetBinUpEdge(bx); };
    UvwdrawSig sig;
    {  // [27] blob 10 cells (x bins 101..110) x 25 strips (y bins 10..34) x 400 = 100,000 (max 400 -> threshold 40);
       // stray cell 200 (x 201, y 50, > 40 -> a signal cell, 0.2 % of the blob); stripe: every strip x 3 cells (x 301..303) x 50
       // = 61 * 3 * 50 = 9,150 (< 10 % of 100,000 = 10,000; 50 > 40 -> signal cells, but one component)
      clear();
      for (int x = 101; x <= 110; ++x)
        for (int y = 10; y <= 34; ++y) h->SetBinContent(x, y, 400);
      h->SetBinContent(201, 50, 200);
      for (int x = 301; x <= 303; ++x)
        for (int y = 1; y <= 61; ++y) h->SetBinContent(x, y, 50);
      const bool ok = uvwdrawSignalRange(h, sig);
      check(ok && near(sig.z0, zlo(101)) && near(sig.z1, zhi(110)) && near(sig.s0, h->GetYaxis()->GetBinLowEdge(10)) &&
                near(sig.s1, h->GetYaxis()->GetBinUpEdge(34)),
            "[27] stray cell + noise stripe (< 10 % of the largest component) do not widen the window (z [25, 27.5])");
    }
    {  // [28] two separate arms: 10 cells x 1000 = 10,000 (x 101..110, y 20) and 3 cells x 1000 = 3,000 (x 301..303, y 40): 30 % -> both
      clear();
      for (int x = 101; x <= 110; ++x) h->SetBinContent(x, 20, 1000);
      for (int x = 301; x <= 303; ++x) h->SetBinContent(x, 40, 1000);
      bool ok = uvwdrawSignalRange(h, sig);
      check(ok && near(sig.z0, zlo(101)) && near(sig.z1, zhi(303)) && near(sig.s0, h->GetYaxis()->GetBinLowEdge(20)) && near(sig.s1, h->GetYaxis()->GetBinUpEdge(40)),
            "[28] second arm 3,000 (30 % of 10,000) is kept: window holds both");
      // second arm = one cell of 500 = 5 % of 10,000 (500 > threshold 100: a signal cell, but out)
      for (int x = 301; x <= 303; ++x) h->SetBinContent(x, 40, 0);
      h->SetBinContent(301, 40, 500);
      ok = uvwdrawSignalRange(h, sig);
      check(ok && near(sig.z1, zhi(110)) && near(sig.s1, h->GetYaxis()->GetBinUpEdge(20)), "[28] second arm 500 (5 % of 10,000) is dropped");
    }
    {  // [29] 8 neighbours: blob 20 cells x 500 = 10,000 (x 101..120, y 10; max 500 -> threshold 50); chain of 5 cells x 300 = 1,500
       // (15 %): touching only diagonally (x 301 + i, y 40 + i) -> one component, kept. Each cell alone is 3 % -> would be out.
      clear();
      for (int x = 101; x <= 120; ++x) h->SetBinContent(x, 10, 500);
      for (int i = 0; i < 5; ++i) h->SetBinContent(301 + i, 40 + i, 300);
      bool ok = uvwdrawSignalRange(h, sig);
      check(ok && near(sig.z1, zhi(305)) && near(sig.s1, h->GetYaxis()->GetBinUpEdge(44)), "[29] cells touching only at the corner are one component (chain kept)");
      // the same cells two bins apart: no neighbours, 5 components of 300 = 3 % -> all dropped
      for (int i = 0; i < 5; ++i) h->SetBinContent(301 + i, 40 + i, 0);
      for (int i = 0; i < 5; ++i) h->SetBinContent(301 + 2 * i, 40 + 2 * i, 300);
      ok = uvwdrawSignalRange(h, sig);
      check(ok && near(sig.z1, zhi(120)) && near(sig.s1, h->GetYaxis()->GetBinUpEdge(10)), "[29] the same cells 2 bins apart are 5 separate components (3 % each): dropped");
    }
    for (int v = 0; v < 3; ++v) delete hd[v];
    {  // [30] 3D points. blob: 10 points on x = 0..4.5 (step 0.5), y 0, z 30, weight 10 each = 100; a point 20 mm away, weight 5 (5 %)
      std::vector<UvwdrawPoint> blob, withFar;
      for (int i = 0; i < 10; ++i) blob.push_back({0.5 * i, 0, 30, 10});
      withFar = blob;
      withFar.push_back({24.5, 0, 30, 5});  // 20 mm from x = 4.5
      const double r[4] = {-60, 60, -40, 40};
      const UvwdrawBotWindow wb = uvwdrawBotWindow(blob, true, r, 120, 300, 200);
      const UvwdrawBotWindow wf = uvwdrawBotWindow(withFar, true, r, 120, 300, 200);
      bool same = near(wb.k, wf.k);
      for (int i = 0; i < 4; ++i) same = same && near(wb.xy[i], wf.xy[i]) && near(wb.xz[i], wf.xz[i]) && near(wb.yz[i], wf.yz[i]);
      check(same, "[30] a point 20 mm away with 5 % of the blob's weight does not move the window");
      withFar.back().w = 15;  // 15 % -> kept
      const UvwdrawBotWindow w2 = uvwdrawBotWindow(withFar, true, r, 120, 300, 200);
      check(w2.xy[1] > wb.xy[1] + 1, "[30] the same far point with 15 % is kept: the window widens");
      // chain: 5 points 4 mm apart (<= 5 mm links), weight 3 each = 15 (15 % of 100) -> one blob, kept; 6 mm apart -> 5 blobs of 3 (3 %), dropped
      std::vector<UvwdrawPoint> a = blob;
      for (int i = 0; i < 5; ++i) a.push_back({50.0 + 4 * i, 0, 30, 3});
      check(uvwdrawKeepPointBlobs(a).size() == 15, "[30] points 4 mm apart chain into one blob (15 % of the largest): 15 points kept");
      std::vector<UvwdrawPoint> b = blob;
      for (int i = 0; i < 5; ++i) b.push_back({50.0 + 6 * i, 0, 30, 3});
      check(uvwdrawKeepPointBlobs(b).size() == 10, "[30] points 6 mm apart do not link (> 5 mm): 5 blobs of 3 %, dropped, 10 kept");
      check(uvwdrawKeepPointBlobs({}).empty(), "[30] no points -> no points");
    }
  }
  {  // [25] [26] drawn result: mm per pixel equal in x and y, in all 6 pads; 3D window holds the points
    const double mm = 0.25;
    UvwdrawPlane pls[3];
    for (int v = 0; v < 3; ++v) uvwdrawFitPlane(richS(v), "UVW"[v], pls[v]);
    TH2D *hd[3], *hx[3];
    uvwdrawMakeDispHists(pls, mm, hd);
    const double rng[4] = {-66.25, 63.75, -75.0, 63.0};
    uvwdrawMakeXYZ(rng, 512 * mm, hx);
    // blobs of different shapes: U 11 strips x 41 cells, V 3 x 11, W 1 x 1 (all 300)
    for (int k = 10; k <= 20; ++k)
      for (int c = 100; c <= 140; ++c) hd[0]->SetBinContent(c + 1, k, 300);
    for (int k = 30; k <= 32; ++k)
      for (int c = 110; c <= 120; ++c) hd[1]->SetBinContent(c + 1, k, 300);
    hd[2]->SetBinContent(106, 31, 300);
    std::vector<UvwdrawPoint> pts = {{10, -20, 30, 5}, {25, 5, 60, 5}, {-5, 15, 45, 5}};
    uvwdrawFillPoints(pts, hx);
    TCanvas cv("c25", "c25", 1200, 600);
    uvwdrawSetupCanvas(cv);
    for (int zoomMode = 1; zoomMode >= 0; --zoomMode) {
      uvwdrawDrawEvent(cv, 7, 3, hd, pls, hx, pts, rng, 512 * mm, -1, false, zoomMode == 1);
      cv.Update();
      double mx[6], my[6];
      bool hasPalette = true;
      for (int i = 0; i < 6; ++i) {
        TVirtualPad *p = cv.GetPad(i + 1);
        const double wpx = (p->GetUxmax() - p->GetUxmin()) / (p->GetX2() - p->GetX1()) * p->GetAbsWNDC() * p->GetWw();
        const double hpx = (p->GetUymax() - p->GetUymin()) / (p->GetY2() - p->GetY1()) * p->GetAbsHNDC() * p->GetWh();
        mx[i] = (p->GetUxmax() - p->GetUxmin()) / wpx;  // mm / px, x
        my[i] = (p->GetUymax() - p->GetUymin()) / hpx;  // mm / px, y
        if (i < 3 && !p->FindObject("palette") && !hd[i]->FindObject("palette")) hasPalette = false;
      }
      const std::string tag = zoomMode ? "zoom" : "zoom=false";
      bool sq = true, top = true, bot = true;
      for (int i = 0; i < 6; ++i) sq = sq && std::fabs(mx[i] / my[i] - 1) < 0.01;
      for (int i = 1; i < 3; ++i) top = top && std::fabs(mx[i] / mx[0] - 1) < 0.01;
      for (int i = 4; i < 6; ++i) bot = bot && std::fabs(mx[i] / mx[3] - 1) < 0.01;
      std::printf("      %s: mm/px x = %.4f %.4f %.4f | %.4f %.4f %.4f ; y = %.4f %.4f %.4f | %.4f %.4f %.4f\n", tag.c_str(), mx[0], mx[1], mx[2], mx[3], mx[4], mx[5], my[0], my[1], my[2], my[3], my[4], my[5]);
      check(sq, "[25] " + tag + ": mm / px in x equals y in all 6 pads (1 %)");
      check(top && bot, "[25] " + tag + ": the same mm / px in the 3 top pads, and in the 3 bottom pads");
      check(hasPalette, "[25] " + tag + ": the colz palette is there on the top pads");
      if (zoomMode) {
        // [26] bottom windows hold the points (+ 5 mm), XZ and YZ share the z window
        bool holds = true;
        const double xs[2] = {-5 - 5, 25 + 5}, ys[2] = {-20 - 5, 15 + 5}, zs[2] = {30 - 5, 60 + 5};
        TVirtualPad *pxy = cv.GetPad(4), *pxz = cv.GetPad(5), *pyz = cv.GetPad(6);
        auto in = [](TVirtualPad *p, const double *h, const double *v) {
          return p->GetUxmin() <= h[0] + 1e-9 && p->GetUxmax() >= h[1] - 1e-9 && p->GetUymin() <= v[0] + 1e-9 && p->GetUymax() >= v[1] - 1e-9;
        };
        holds = in(pxy, xs, ys) && in(pxz, xs, zs) && in(pyz, ys, zs);
        check(holds, "[26] zoom: XY / XZ / YZ windows hold the 3D points + 5 mm");
        check(near(pxz->GetUymin(), pyz->GetUymin()) && near(pxz->GetUymax(), pyz->GetUymax()), "[26] XZ and YZ share the z window");
        // the whole-range picture is much wider than the zoomed one
        check(pxy->GetUxmax() - pxy->GetUxmin() < 120, "[26] zoomed XY window narrower than the whole range (130 mm)");
      }
    }
    // an empty event (no signal, no point) is drawn too
    for (int v = 0; v < 3; ++v) hd[v]->Reset();
    uvwdrawFillPoints({}, hx);
    uvwdrawDrawEvent(cv, 8, 4, hd, pls, hx, {}, rng, 512 * mm, -1, false, true);
    check(true, "[25] an empty event is drawn without error");
    {  // the painter caches the colour range; the next event's plane maximum must be its own (TH1::GetMaximum() is stale)
      for (int v = 0; v < 3; ++v) hd[v]->SetBinContent(101, 31, 50);  // event A: maximum 50 -> threshold 35
      uvwdrawDrawEvent(cv, 9, 5, hd, pls, hx, {}, rng, 512 * mm, -1, false, true);
      cv.Update();
      for (int v = 0; v < 3; ++v) {
        hd[v]->Reset();
        hd[v]->SetBinContent(101, 31, 1000);  // event B: maximum 1000 -> threshold 100
      }
      hd[0]->SetBinContent(11, 50, 90);  // far away, 90 < 100: not a signal cell
      uvwdrawDrawEvent(cv, 10, 6, hd, pls, hx, {}, rng, 512 * mm, -1, false, true);
      cv.Update();
      TVirtualPad *p1 = cv.GetPad(1);
      // only the 1000 cells count: dz = 0.25, ds = 2 -> L_min decides, the window is 20 mm high (not the span to the 90)
      check(near(p1->GetUymax() - p1->GetUymin(), 20), "[25] the next event is judged by its own plane maximum, not the painter's cached one (window 20 mm high, got " +
                                                          std::to_string(p1->GetUymax() - p1->GetUymin()) + ")");
    }
    for (int v = 0; v < 3; ++v) {
      delete hd[v];
      delete hx[v];
    }
  }
  {  // zoom = false end to end
    drawUVW("syn_raw.root", RUN_P, RUN_F, RUN_HV, "dz0", 0, -1, -1, false, false);
    check(exists("dz0/event0000010.png") && exists("dz0/event0000012.png"), "[25] drawUVW(..., zoom = false) writes the 3 PNG files");
  }

  // ---- cleanup -------------------------------------------------------------------------------
  gSystem->ChangeDirectory(startDir.c_str());
  gSystem->Exec(("rm -rf '" + dir + "'").c_str());

  std::printf("%s: %d failure(s)\n", g_failures == 0 ? "GREEN" : "RED", g_failures);
  return g_failures;
}
