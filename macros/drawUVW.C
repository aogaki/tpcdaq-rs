// =============================================================================
// drawUVW.C - every event of a raw tree as an image: U / V / W like the web UI's UVW
// event display (top row) and XY / XZ / YZ of a model-free 3D point cloud (bottom row).
//
// Takes the *_raw.root (tree `raw`: eventId, adc[4][68][512]) from graw2root.C
// or the DAQ's root-sink --raw-tree. One event = one 1200 x 600 canvas divided 3 x 2
// (400 x 300 each), no stats box, title = "event <eventId> (entry <n>)  <plane>".
//
// Top row (TODO/190): the SAME algorithm as the UI:
//   src/monitor/display.rs  accumulate_uvw: (aget, raw ch) -> (plane, strip) from the
//       geometry; grid[plane][strip][bucket] += adc, RAW ADC (no pedestal), several
//       channels on one strip are summed, clamped to u16 (0..65535); FPN / unmapped
//       channels are not drawn. nStrips[plane] = largest strip NUMBER of the plane.
//   ui/src/app/monitor/root-histo.ts  buildUvw / stripBaseline: value = grid minus the
//       strip's baseline = mean of the first 25 cells (negative values are kept).
//       The values are filled into a strip-numbered TH2D (x = time bucket, y = strip) as before.
// Axes in mm (TODO/193): the strip histogram is copied into a TH2D with x = z [mm]
//   ([0, 512 mmPerCell], bin b = [b, b + 1) mmPerCell) and y = strip position s [mm]
//   (strip k = [s_k - p/2, s_k + p/2]). s = slope * strip + offset per plane, from
//   channel_map.csv; every row must be on the line within 1e-6 mm (else ERROR, nothing
//   written). A negative slope puts the strips in reverse order (low s first).
// Auto zoom (zoom = true): signal cells = value > max(35, 0.1 x the plane maximum), grouped into 8-neighbour
//   connected components; only components with charge >= 0.1 x the largest count (TODO/199: strays, noise stripes
//   stay out; bottom row: 3D points in single-link blobs <= 5 mm, same 0.1 rule). The window
//   is their z (union of the planes) / s extent + 5 mm, at least 20 mm high, and the SAME
//   mm per pixel in x and y and in all 3 planes (k = max((dz + 2m) / W, (ds + 2m) / H,
//   20 / H), W x H = pad frame in pixels). Bottom row: same rule on the 3D points (XY / XZ /
//   YZ share k, XZ and YZ share the z window). No signal / no point / zoom = false: the whole
//   range at the smallest k that holds it (the frame may extend past the histogram).
// Bottom row (TODO/191): points (x, y, z) found per cell WITHOUT assuming a track model
// (the web UI's XY/XZ/YZ put points on makeTracks' one straight line, which cannot show a
// two-track event). Per cell and plane the strips above UVWDRAW_HIT_ADC are grouped into
// clusters (gap <= 2 pitch + 0.1 mm; charge Q >= UVWDRAW_CLUSTER_Q_MIN, no width limit),
// and every (U, V, W) cluster triple with |sU + sV + sW - c| < UVWDRAW_CLOSURE_MM is a
// point. c = the closure constant, the median over the run's first 300 events of
// the sum for cells with exactly one cluster per plane. z = cell * mmPerCell.
// Zero-suppressed data (TODO/152 restoration) is not handled: this reads raw ROOT.
// The channel map is channel_map.csv in the current directory (as analyzeUVW.C); it
// was checked 256/256 against the real geometry, i.e. the same strip numbers as the UI.
//
// Usage:  root -l -b -q 'drawUVW.C("run0209_raw.root", 198, 12.5, 1600)'
//            -> run0209_raw.root -> directory run0209_uvw_events/ with event0000010.png ...
//         drawUVW(rawFile, pressureMbar, samplingMHz, driftHvVolt, out = "", first = 0,
//                 n = -1, drawMinAdc = -1, logz = false, zoom = true)
//         pressureMbar, samplingMHz (100 / 50 / 25 / 12.5), driftHvVolt: the run constants
//           (required, checked as in analyzeUVW.C); mmPerCell = v_drift / samplingMHz * 10.
//         out: empty -> directory from the input name (needs '_raw.root' in it);
//              ends with ".pdf" -> ONE PDF (1 event = 1 page, bookmark "event <eventId>");
//              otherwise -> a directory (made with its parents) of PNGs.
//         first, n: entry range [first, first+n) (n < 0: to the end)
//         drawMinAdc < 0 (default): nothing extra = same as the UI. >= 0: every top plane
//           gets SetMinimum(drawMinAdc) and the pad frame is filled with the lowest
//           palette colour, so cells below the threshold show as background.
//         logz: SetLogz() on every top pad.
//         zoom: false = always the whole range (mm axes, equal mm per pixel).
// Size (run 209, 1200 x 600): PNG ~32 kB / event zoomed (TODO/193; ~46 kB before); PDF ~570 kB /
//   page. Above 2 GiB ROOT writes a broken xref table (32-bit offsets); the macro warns, and
//   `mutool clean in.pdf out.pdf` rewrites a valid file (or split with first / n).
// =============================================================================
#include <TCanvas.h>
#include <TColor.h>
#include <TFile.h>
#include <TH2D.h>
#include <TString.h>
#include <TStyle.h>
#include <TSystem.h>
#include <TTree.h>

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <fstream>
#include <iostream>
#include <sstream>
#include <string>
#include <utility>
#include <vector>

const int UVWDRAW_NBUCKET = 512;
const int UVWDRAW_BASELINE_CELLS = 25;  // BASELINE_CELLS in root-histo.ts

// channel_map.csv -> view / strip per (aget, raw ch); view = -1: not in the map.
// nStrips[v] = largest strip number of the view. false: file missing.
// sPos[v][strip-1] = s_mm of the strip (NaN: no channel on that strip).
bool uvwdrawLoadMap(const char *path, int view[4][68], int strip[4][68], int nStrips[3], std::vector<double> sPos[3])
{
  std::vector<std::pair<std::pair<int, int>, double>> posList;  // ((view, strip), s_mm)
  for (int a = 0; a < 4; ++a)
    for (int c = 0; c < 68; ++c) {
      view[a][c] = -1;
      strip[a][c] = 0;
    }
  for (int v = 0; v < 3; ++v) nStrips[v] = 0;
  std::ifstream fm(path);
  if (!fm) return false;
  for (std::string line; std::getline(fm, line);) {
    if (line.empty() || line[0] == '#') continue;
    std::istringstream ss(line);
    int a, c, v, s;
    double pos;
    char comma;
    if (!(ss >> a >> comma >> c >> comma >> v >> comma >> s >> comma >> pos)) continue;
    if (a < 0 || a >= 4 || c < 0 || c >= 68 || v < 0 || v >= 3 || s < 1) {
      std::cout << "WARNING: channel_map.csv line skipped (out of range): " << line << std::endl;
      continue;
    }
    view[a][c] = v;
    strip[a][c] = s;
    nStrips[v] = std::max(nStrips[v], s);
    posList.push_back({{v, s}, pos});
  }
  for (int v = 0; v < 3; ++v) sPos[v].assign(nStrips[v], std::nan(""));
  for (const auto &p : posList) sPos[p.first.first][p.first.second - 1] = p.second;
  return true;
}

// the three histograms, made once and reused for every event
void uvwdrawMakeHists(const int nStrips[3], TH2D *h[3])
{
  const char *vn[3] = {"U", "V", "W"};
  for (int v = 0; v < 3; ++v) {
    h[v] = new TH2D(Form("hUVW_%s", vn[v]), "", UVWDRAW_NBUCKET, 0, UVWDRAW_NBUCKET, nStrips[v], 1,
                    nStrips[v] + 1);
    h[v]->SetDirectory(nullptr);
    h[v]->SetStats(0);
  }
}

std::string uvwdrawTitle(UInt_t eventId, Long64_t entry, int v)
{
  const char *vn[3] = {"U", "V", "W"};
  return std::string(Form("event %u (entry %lld)  %s;z [mm];%s [mm]", eventId, entry, vn[v], vn[v]));
}

// One event -> the three planes (the UI's accumulate_uvw + buildUvw). view[a][c] = -1: not in
// the map (FPN, aux, unmapped). The h[v] axes (nStrips) are made by the caller.
void fillUVW(const Short_t adc[4][68][512], const int view[4][68], const int strip[4][68], TH2D *h[3])
{
  for (int v = 0; v < 3; ++v) {
    const int ny = h[v]->GetNbinsY();
    std::vector<double> acc((size_t)ny * UVWDRAW_NBUCKET, 0.0);
    for (int a = 0; a < 4; ++a)
      for (int c = 0; c < 68; ++c) {
        if (view[a][c] != v) continue;
        const int s = strip[a][c];
        if (s < 1 || s > ny) continue;
        double *row = &acc[(size_t)(s - 1) * UVWDRAW_NBUCKET];
        for (int t = 0; t < UVWDRAW_NBUCKET; ++t) row[t] += adc[a][c][t];
      }
    h[v]->Reset();
    for (int s = 1; s <= ny; ++s) {
      double *row = &acc[(size_t)(s - 1) * UVWDRAW_NBUCKET];
      for (int t = 0; t < UVWDRAW_NBUCKET; ++t) row[t] = std::min(std::max(row[t], 0.0), 65535.0);
      double base = 0;
      for (int t = 0; t < UVWDRAW_BASELINE_CELLS; ++t) base += row[t];
      base /= UVWDRAW_BASELINE_CELLS;
      for (int t = 0; t < UVWDRAW_NBUCKET; ++t) h[v]->SetBinContent(t + 1, s, row[t] - base);
    }
  }
}


// ---- 3D points (TODO/191) ----------------------------------------------------------
const double UVWDRAW_HIT_ADC = 35.0;          // = ADC_THRESHOLD in analyzeUVW.C
const double UVWDRAW_CLUSTER_Q_MIN = 150.0;   // clusters with a smaller charge are dropped
const double UVWDRAW_CLOSURE_MM = 2.0;        // |sU + sV + sW - c| below this = one point
const double UVWDRAW_GAP_EXTRA_MM = 0.1;      // two strips join if gap <= 2 pitch + this
const int UVWDRAW_C_EVENTS = 300;             // events used to find c
const double UVWDRAW_DRIFT_LEN_CM = 20.0;     // = DRIFT_LEN_CM in analyzeUVW.C
// strip pitch vectors of U, V, W (= PITCH in makeTracks.C); sum of the 3 outer products = 1.5 I
const double UVWDRAW_PITCH[3][2] = {{-1, 0}, {0.5, 0.8660254}, {0.5, -0.8660254}};

struct UvwdrawCluster {
  double s;  // charge-weighted centre [mm]
  double q;  // sum of the strip values
};
struct UvwdrawCell {  // the clusters of one cell, per plane
  std::vector<UvwdrawCluster> cl[3];
};
struct UvwdrawPoint {
  double x, y, z, w;  // [mm], weight = mean cluster charge
};

// same function as driftVelocityCmUs in analyzeUVW.C (same warning text)
double uvwdrawDriftVelocityCmUs(double hvVolt, double lenCm, double pMbar)
{
  const double MOBILITY = 0.7233;  // (cm/us) per (V/cm/mbar)
  const double eOverP = hvVolt / lenCm / pMbar;
  if (eOverP < 0.54 || eOverP > 1.05)
    std::cout << "WARNING: E/p = " << eOverP
              << " V/(cm*mbar) is outside the 0.54-1.05 range of the "
                 "operating-point table; v_drift is an extrapolation here."
              << std::endl;
  return MOBILITY * eOverP;
}

// the run-constant checks of analyzeUVW.C; false = ERROR printed
bool uvwdrawCheckRunConstants(double pressureMbar, double samplingMHz, double driftHvVolt)
{
  const double SAMPLING_OK[4] = {100.0, 50.0, 25.0, 12.5};
  bool samplingOk = false;
  for (double s : SAMPLING_OK)
    if (samplingMHz == s) samplingOk = true;
  if (!samplingOk) {
    std::cout << "ERROR: samplingMHz = " << samplingMHz << " is not one of 100, 50, 25, 12.5; nothing written" << std::endl;
    return false;
  }
  if (!(driftHvVolt > 0)) {
    std::cout << "ERROR: driftHvVolt = " << driftHvVolt << " must be > 0; nothing written" << std::endl;
    return false;
  }
  if (!(pressureMbar > 0)) {
    std::cout << "ERROR: pressureMbar = " << pressureMbar << " must be > 0; nothing written" << std::endl;
    return false;
  }
  return true;
}

double uvwdrawMmPerCell(double pressureMbar, double samplingMHz, double driftHvVolt)
{
  return uvwdrawDriftVelocityCmUs(driftHvVolt, UVWDRAW_DRIFT_LEN_CM, pressureMbar) / samplingMHz * 10.0;
}

// smallest difference between neighbouring distinct s of a plane (NaN = no strip); 0 if < 2 distinct
double uvwdrawPitch(const std::vector<double> &sPos)
{
  std::vector<double> s;
  for (double x : sPos)
    if (!std::isnan(x)) s.push_back(x);
  std::sort(s.begin(), s.end());
  double pitch = 0;
  for (size_t i = 1; i < s.size(); ++i) {
    const double d = s[i] - s[i - 1];
    if (d > 1e-9 && (pitch == 0 || d < pitch)) pitch = d;
  }
  return pitch;
}

// clusters of one plane at one cell of the baseline-subtracted histogram (x bin = cell + 1, y bin = strip)
std::vector<UvwdrawCluster> uvwdrawClustersAtCell(const TH2D *h, const std::vector<double> &sPos, double pitch, int cell)
{
  std::vector<std::pair<double, double>> hits;  // (s, value)
  const int ny = std::min<int>(h->GetNbinsY(), (int)sPos.size());
  for (int st = 1; st <= ny; ++st) {
    const double v = h->GetBinContent(cell + 1, st);
    if (v > UVWDRAW_HIT_ADC && !std::isnan(sPos[st - 1])) hits.push_back({sPos[st - 1], v});
  }
  std::sort(hits.begin(), hits.end());
  std::vector<UvwdrawCluster> out;
  const double maxGap = 2 * pitch + UVWDRAW_GAP_EXTRA_MM;
  size_t i = 0;
  while (i < hits.size()) {
    double q = 0, qs = 0;
    size_t j = i;
    for (; j < hits.size(); ++j) {
      if (j > i && hits[j].first - hits[j - 1].first > maxGap) break;
      q += hits[j].second;
      qs += hits[j].second * hits[j].first;
    }
    if (q >= UVWDRAW_CLUSTER_Q_MIN) out.push_back({qs / q, q});
    i = j;
  }
  return out;
}

// closure constant: median of sU + sV + sW over the cells with exactly one cluster in each plane
// (0 and *nUsed = 0 if there is none)
double uvwdrawClosureC(const std::vector<UvwdrawCell> &cells, int *nUsed)
{
  std::vector<double> sums;
  for (const auto &c : cells)
    if (c.cl[0].size() == 1 && c.cl[1].size() == 1 && c.cl[2].size() == 1)
      sums.push_back(c.cl[0][0].s + c.cl[1][0].s + c.cl[2][0].s);
  if (nUsed) *nUsed = (int)sums.size();
  if (sums.empty()) return 0;
  std::sort(sums.begin(), sums.end());
  const size_t m = sums.size();
  return (m % 2) ? sums[m / 2] : 0.5 * (sums[m / 2 - 1] + sums[m / 2]);
}

// all (U, V, W) triples of one cell that close -> points at height z (appended to out)
void uvwdrawMatchCell(const std::vector<UvwdrawCluster> cl[3], double c, double z, std::vector<UvwdrawPoint> &out)
{
  for (const auto &u : cl[0])
    for (const auto &v : cl[1])
      for (const auto &w : cl[2]) {
        if (!(std::fabs(u.s + v.s + w.s - c) < UVWDRAW_CLOSURE_MM)) continue;
        const double sv[3] = {u.s - c / 3, v.s - c / 3, w.s - c / 3};
        double x = 0, y = 0;
        for (int k = 0; k < 3; ++k) {
          x += UVWDRAW_PITCH[k][0] * sv[k];
          y += UVWDRAW_PITCH[k][1] * sv[k];
        }
        out.push_back({x / 1.5, y / 1.5, z, (u.q + v.q + w.q) / 3});
      }
}

// x / y range of the XY plots from the map: r = {xlo, xhi, ylo, yhi}. false: no (U, V) pair
// closes into the W range.
bool uvwdrawRanges(const std::vector<double> sPos[3], double c, double r[4])
{
  const double MARGIN = 5.0;
  double uMin = 1e300, uMax = -1e300, wMin = 1e300, wMax = -1e300;
  for (double s : sPos[0])
    if (!std::isnan(s)) {
      uMin = std::min(uMin, s);
      uMax = std::max(uMax, s);
    }
  for (double s : sPos[2])
    if (!std::isnan(s)) {
      wMin = std::min(wMin, s);
      wMax = std::max(wMax, s);
    }
  if (uMin > uMax || wMin > wMax) return false;
  r[0] = -uMax - MARGIN;
  r[1] = -uMin + MARGIN;
  const double eps = 1e-9;
  double yMin = 1e300, yMax = -1e300;
  for (double su : sPos[0]) {
    if (std::isnan(su)) continue;
    for (double sv : sPos[1]) {
      if (std::isnan(sv)) continue;
      const double sw = c - su - sv;  // predicted by the closure
      if (sw < wMin - eps || sw > wMax + eps) continue;
      const double y = (UVWDRAW_PITCH[1][1] * (sv - c / 3) + UVWDRAW_PITCH[2][1] * (sw - c / 3)) / 1.5;
      yMin = std::min(yMin, y);
      yMax = std::max(yMax, y);
    }
  }
  if (yMin > yMax) return false;
  r[2] = yMin - MARGIN;
  r[3] = yMax + MARGIN;
  return true;
}

// XY, XZ, YZ histograms (made once, reused): ~1 mm bins over r (x, y) and [0, zMax]
void uvwdrawMakeXYZ(const double r[4], double zMax, TH2D *h[3])
{
  const int nx = std::max(1, (int)std::ceil(r[1] - r[0]));
  const int ny = std::max(1, (int)std::ceil(r[3] - r[2]));
  const int nz = std::max(1, (int)std::ceil(zMax));
  h[0] = new TH2D("hUVW_XY", "", nx, r[0], r[1], ny, r[2], r[3]);
  h[1] = new TH2D("hUVW_XZ", "", nx, r[0], r[1], nz, 0, zMax);
  h[2] = new TH2D("hUVW_YZ", "", ny, r[2], r[3], nz, 0, zMax);
  for (int k = 0; k < 3; ++k) {
    h[k]->SetDirectory(nullptr);
    h[k]->SetStats(0);
  }
}

// points -> the three histograms (reset first), weighted by the point charge
void uvwdrawFillPoints(const std::vector<UvwdrawPoint> &pts, TH2D *h[3])
{
  for (int k = 0; k < 3; ++k) h[k]->Reset();
  for (const auto &p : pts) {
    h[0]->Fill(p.x, p.y, p.w);
    h[1]->Fill(p.x, p.z, p.w);
    h[2]->Fill(p.y, p.z, p.w);
  }
}

std::string uvwdrawTitleXYZ(UInt_t eventId, Long64_t entry, int k)
{
  const char *t[3] = {"XY;x [mm];y [mm]", "XZ;x [mm];z [mm]", "YZ;y [mm];z [mm]"};
  return std::string(Form("event %u (entry %lld)  %s", eventId, entry, t[k]));
}

// the points of one event, from the filled top histograms h
void uvwdrawEventPoints(TH2D *const h[3], const std::vector<double> sPos[3], const double pitch[3], double c, double mmPerCell,
                        std::vector<UvwdrawPoint> &pts)
{
  pts.clear();
  std::vector<UvwdrawCluster> cl[3];
  for (int t = 0; t < UVWDRAW_NBUCKET; ++t) {
    for (int v = 0; v < 3; ++v) cl[v] = uvwdrawClustersAtCell(h[v], sPos[v], pitch[v], t);
    if (cl[0].empty() || cl[1].empty() || cl[2].empty()) continue;
    uvwdrawMatchCell(cl, c, t * mmPerCell, pts);
  }
}


// ---- mm axes, auto zoom, equal mm per pixel (TODO/193) --------------------------------------
const double UVWDRAW_ZOOM_FRACTION = 0.1;   // a signal cell is > max(HIT_ADC, this x the plane maximum)
const double UVWDRAW_ZOOM_KEEP = 0.1;       // (TODO/199) only components with charge >= this x the largest component set the window
const double UVWDRAW_LINK_MM = 5.0;         // (TODO/199) 3D points closer than this (single link) are one blob
const double UVWDRAW_ZOOM_MARGIN_MM = 5.0;  // m: margin around the signal on every side
const double UVWDRAW_ZOOM_LMIN_MM = 20.0;   // the window is never smaller than this (height)
const double UVWDRAW_LINE_TOL_MM = 1e-6;    // s_mm of every strip must be on s = slope * strip + offset

struct UvwdrawPlane {  // strip k (1-based) is at s = slope * k + offset [mm]
  double slope, offset, pitch;  // pitch = |slope|
  int n;                        // number of strips (largest strip number)
  bool rev;                     // slope < 0: y bins run from the LAST strip to the first
  double sLo, sHi;              // y range of the display histogram = whole plane [mm]
};
struct UvwdrawSig {  // the signal cells of one plane [mm]
  bool has;
  double z0, z1, s0, s1;
};
struct UvwdrawTopWindow {
  double k, z0, z1, s0[3], s1[3];  // k [mm / px]
};
struct UvwdrawBotWindow {
  double k;
  double xy[4], xz[4], yz[4];  // {horizontal lo, hi, vertical lo, hi}
};

// s_mm of a plane (NaN = no channel) -> the straight line; false (ERROR printed) if a strip is off it.
bool uvwdrawFitPlane(const std::vector<double> &sPos, char name, UvwdrawPlane &pl)
{
  int k0 = -1, k1 = -1;  // first / last strip with a channel (0-based index)
  for (size_t i = 0; i < sPos.size(); ++i)
    if (!std::isnan(sPos[i])) {
      if (k0 < 0) k0 = (int)i;
      k1 = (int)i;
    }
  if (k0 < 0 || k0 == k1) {
    std::cout << "ERROR: plane " << name << " has fewer than 2 strips with a position in channel_map.csv; nothing written" << std::endl;
    return false;
  }
  const double slope = (sPos[k1] - sPos[k0]) / (k1 - k0);
  if (!(std::fabs(slope) > 0)) {
    std::cout << "ERROR: plane " << name << " has slope 0 in channel_map.csv; nothing written" << std::endl;
    return false;
  }
  const double offset = sPos[k0] - slope * (k0 + 1);
  for (size_t i = 0; i < sPos.size(); ++i) {
    if (std::isnan(sPos[i])) continue;
    const double d = sPos[i] - (slope * (i + 1) + offset);
    if (std::fabs(d) > UVWDRAW_LINE_TOL_MM) {
      std::cout << "ERROR: plane " << name << " strip " << i + 1 << " is " << d << " mm off the line s = " << slope << " * strip + " << offset
                << " (tolerance " << UVWDRAW_LINE_TOL_MM << " mm) in channel_map.csv; nothing written" << std::endl;
      return false;
    }
  }
  pl.slope = slope;
  pl.offset = offset;
  pl.pitch = std::fabs(slope);
  pl.n = (int)sPos.size();
  pl.rev = slope < 0;
  const double a = slope + offset, b = slope * pl.n + offset;
  pl.sLo = std::min(a, b) - pl.pitch / 2;
  pl.sHi = std::max(a, b) + pl.pitch / 2;
  return true;
}

// the top histograms in mm: x = z [0, 512 mmPerCell] (512 bins), y = s [sLo, sHi] (n bins)
void uvwdrawMakeDispHists(const UvwdrawPlane pl[3], double mmPerCell, TH2D *h[3])
{
  const char *vn[3] = {"U", "V", "W"};
  for (int v = 0; v < 3; ++v) {
    h[v] = new TH2D(Form("hUVW_%s_mm", vn[v]), "", UVWDRAW_NBUCKET, 0, UVWDRAW_NBUCKET * mmPerCell, pl[v].n, pl[v].sLo, pl[v].sHi);
    h[v]->SetDirectory(nullptr);
    h[v]->SetStats(0);
  }
}

// strip-numbered histograms (fillUVW's) -> the mm histograms; values are copied as they are
void uvwdrawToDisp(TH2D *const hs[3], const UvwdrawPlane pl[3], TH2D *const hd[3])
{
  for (int v = 0; v < 3; ++v) {
    const int n = pl[v].n;
    for (int k = 1; k <= n; ++k) {
      const int j = pl[v].rev ? n + 1 - k : k;
      for (int t = 1; t <= UVWDRAW_NBUCKET; ++t) hd[v]->SetBinContent(t, j, hs[v]->GetBinContent(t, k));
    }
  }
}

// signal cells of a display histogram: value > max(HIT_ADC, ZOOM_FRACTION x plane maximum), of the connected
// components (8 neighbours) with charge >= ZOOM_KEEP x the largest. sig = their z / s extent [mm] (cell edges). false (sig.has = false): none.
bool uvwdrawSignalRange(const TH2D *h, UvwdrawSig &sig)
{
  sig.has = false;
  sig.z0 = sig.z1 = sig.s0 = sig.s1 = 0;
  const int nx = h->GetNbinsX(), ny = h->GetNbinsY();
  // the plane maximum by scanning: TH1::GetMaximum() returns the value the painter cached on the previous draw
  double vmax = 0;
  for (int y = 1; y <= ny; ++y)
    for (int x = 1; x <= nx; ++x) vmax = std::max(vmax, h->GetBinContent(x, y));
  const double thr = std::max(UVWDRAW_HIT_ADC, UVWDRAW_ZOOM_FRACTION * vmax);
  // signal cells -> connected components (8 neighbours); only components with charge (sum of values)
  // >= UVWDRAW_ZOOM_KEEP x the largest set the window (TODO/199): strays and noise stripes stay out
  std::vector<int> cells;  // x + nx * y (0-based) of the cells above the threshold
  for (int y = 1; y <= ny; ++y)
    for (int x = 1; x <= nx; ++x)
      if (h->GetBinContent(x, y) > thr) cells.push_back((x - 1) + nx * (y - 1));
  if (cells.empty()) return false;
  std::vector<int> comp(cells.size(), -1);  // component id per cell; neighbours found by binary search (cells is sorted)
  std::vector<double> charge;
  std::vector<int> stack;
  for (size_t i = 0; i < cells.size(); ++i) {
    if (comp[i] >= 0) continue;
    const int id = (int)charge.size();
    charge.push_back(0.0);
    comp[i] = id;
    stack.push_back((int)i);
    while (!stack.empty()) {
      const int a = stack.back();
      stack.pop_back();
      const int cx = cells[a] % nx, cy = cells[a] / nx;
      charge[id] += h->GetBinContent(cx + 1, cy + 1);
      for (int dy = -1; dy <= 1; ++dy)
        for (int dx = -1; dx <= 1; ++dx) {
          const int px = cx + dx, py = cy + dy;
          if ((dx == 0 && dy == 0) || px < 0 || px >= nx || py < 0 || py >= ny) continue;
          const auto it = std::lower_bound(cells.begin(), cells.end(), px + nx * py);
          if (it == cells.end() || *it != px + nx * py) continue;
          const int b = (int)(it - cells.begin());
          if (comp[b] >= 0) continue;
          comp[b] = id;
          stack.push_back(b);
        }
    }
  }
  double qmax = 0;
  for (double q : charge) qmax = std::max(qmax, q);
  int xlo = nx + 1, xhi = 0, ylo = ny + 1, yhi = 0;
  for (size_t i = 0; i < cells.size(); ++i) {
    if (charge[comp[i]] < UVWDRAW_ZOOM_KEEP * qmax) continue;
    const int x = cells[i] % nx + 1, y = cells[i] / nx + 1;
    xlo = std::min(xlo, x);
    xhi = std::max(xhi, x);
    ylo = std::min(ylo, y);
    yhi = std::max(yhi, y);
  }
  if (xhi == 0) return false;
  sig.has = true;
  sig.z0 = h->GetXaxis()->GetBinLowEdge(xlo);
  sig.z1 = h->GetXaxis()->GetBinUpEdge(xhi);
  sig.s0 = h->GetYaxis()->GetBinLowEdge(ylo);
  sig.s1 = h->GetYaxis()->GetBinUpEdge(yhi);
  return true;
}

// mm / px: the smallest k that holds every frame (dh, dv: extent of the content + 2 m) and is at least L_min high
double uvwdrawScaleK(const double dh[], const double dv[], int n, double W, double H, double margin, double lmin)
{
  double k = lmin / H;
  for (int i = 0; i < n; ++i) k = std::max(k, std::max((dh[i] + 2 * margin) / W, (dv[i] + 2 * margin) / H));
  return k;
}

// Top row window. zoom and some plane with signal: z = union of the signal, k from the signal + m / L_min;
// a plane without signal is centred on its whole range (extent 0). Else the whole range, smallest k (no margin).
UvwdrawTopWindow uvwdrawTopWindow(const UvwdrawSig sig[3], bool zoom, double zFullHi, const UvwdrawPlane pl[3], double W, double H)
{
  bool any = false;
  if (zoom)
    for (int v = 0; v < 3; ++v) any = any || sig[v].has;
  double zlo = 0, zhi = zFullHi, sc[3], ds[3];
  if (any) {
    zlo = 1e300;
    zhi = -1e300;
    for (int v = 0; v < 3; ++v)
      if (sig[v].has) {
        zlo = std::min(zlo, sig[v].z0);
        zhi = std::max(zhi, sig[v].z1);
      }
  }
  for (int v = 0; v < 3; ++v) {
    if (any && sig[v].has) {
      sc[v] = 0.5 * (sig[v].s0 + sig[v].s1);
      ds[v] = sig[v].s1 - sig[v].s0;
    } else {
      sc[v] = 0.5 * (pl[v].sLo + pl[v].sHi);
      ds[v] = any ? 0.0 : pl[v].sHi - pl[v].sLo;
    }
  }
  const double dz[3] = {zhi - zlo, zhi - zlo, zhi - zlo};
  UvwdrawTopWindow w;
  w.k = uvwdrawScaleK(dz, ds, 3, W, H, any ? UVWDRAW_ZOOM_MARGIN_MM : 0.0, any ? UVWDRAW_ZOOM_LMIN_MM : 0.0);
  const double zc = 0.5 * (zlo + zhi);
  w.z0 = zc - w.k * W / 2;
  w.z1 = zc + w.k * W / 2;
  for (int v = 0; v < 3; ++v) {
    w.s0[v] = sc[v] - w.k * H / 2;
    w.s1[v] = sc[v] + w.k * H / 2;
  }
  return w;
}

// 3D points -> only the points of blobs with charge >= UVWDRAW_ZOOM_KEEP x the largest blob (TODO/199).
// Blob = single link: two points closer than (<=) UVWDRAW_LINK_MM are in the same blob. Charge = sum of the weights.
// Used for the window only; the histograms keep every point. O(n^2) with union-find.
std::vector<UvwdrawPoint> uvwdrawKeepPointBlobs(const std::vector<UvwdrawPoint> &pts)
{
  const size_t n = pts.size();
  std::vector<size_t> parent(n);
  for (size_t i = 0; i < n; ++i) parent[i] = i;
  auto find = [&](size_t i) {
    while (parent[i] != i) {
      parent[i] = parent[parent[i]];
      i = parent[i];
    }
    return i;
  };
  const double l2 = UVWDRAW_LINK_MM * UVWDRAW_LINK_MM;
  for (size_t i = 0; i < n; ++i)
    for (size_t j = i + 1; j < n; ++j) {
      const double dx = pts[i].x - pts[j].x, dy = pts[i].y - pts[j].y, dz = pts[i].z - pts[j].z;
      if (dx * dx + dy * dy + dz * dz <= l2) parent[find(i)] = find(j);
    }
  std::vector<double> q(n, 0.0);
  double qmax = 0;
  for (size_t i = 0; i < n; ++i) q[find(i)] += pts[i].w;
  for (size_t i = 0; i < n; ++i) qmax = std::max(qmax, q[i]);
  std::vector<UvwdrawPoint> out;
  for (size_t i = 0; i < n; ++i)
    if (q[find(i)] >= UVWDRAW_ZOOM_KEEP * qmax) out.push_back(pts[i]);
  return out;
}

// Bottom row window (XY, XZ, YZ share k). Without zoom or points: the whole range r = {xlo, xhi, ylo, yhi}, z [0, zFullHi].
UvwdrawBotWindow uvwdrawBotWindow(const std::vector<UvwdrawPoint> &allPts, bool zoom, const double r[4], double zFullHi, double W, double H)
{
  const std::vector<UvwdrawPoint> pts = zoom ? uvwdrawKeepPointBlobs(allPts) : std::vector<UvwdrawPoint>();
  const bool any = zoom && !pts.empty();
  double lo[3] = {r[0], r[2], 0}, hi[3] = {r[1], r[3], zFullHi};
  if (any) {
    for (int a = 0; a < 3; ++a) {
      lo[a] = 1e300;
      hi[a] = -1e300;
    }
    for (const auto &p : pts) {
      const double c[3] = {p.x, p.y, p.z};
      for (int a = 0; a < 3; ++a) {
        lo[a] = std::min(lo[a], c[a]);
        hi[a] = std::max(hi[a], c[a]);
      }
    }
  }
  const double e[3] = {hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]};
  const double dh[3] = {e[0], e[0], e[1]}, dv[3] = {e[1], e[2], e[2]};  // XY, XZ, YZ
  UvwdrawBotWindow w;
  w.k = uvwdrawScaleK(dh, dv, 3, W, H, any ? UVWDRAW_ZOOM_MARGIN_MM : 0.0, any ? UVWDRAW_ZOOM_LMIN_MM : 0.0);
  const double cx = 0.5 * (lo[0] + hi[0]), cy = 0.5 * (lo[1] + hi[1]), cz = 0.5 * (lo[2] + hi[2]);
  const double hw = w.k * W / 2, hh = w.k * H / 2;
  const double xy[4] = {cx - hw, cx + hw, cy - hh, cy + hh};
  const double xz[4] = {cx - hw, cx + hw, cz - hh, cz + hh};
  const double yz[4] = {cy - hw, cy + hw, cz - hh, cz + hh};
  for (int i = 0; i < 4; ++i) {
    w.xy[i] = xy[i];
    w.xz[i] = xz[i];
    w.yz[i] = yz[i];
  }
  return w;
}

// the six pads: same size, same margins; this is the frame (inside the axes) in pixels
void uvwdrawFramePx(TVirtualPad *p, double &W, double &H)
{
  W = p->GetWw() * p->GetAbsWNDC() * (1 - p->GetLeftMargin() - p->GetRightMargin());
  H = p->GetWh() * p->GetAbsHNDC() * (1 - p->GetTopMargin() - p->GetBottomMargin());
}

// 1200 x 600 canvas, 3 x 2 pads, margins that leave room for axis titles and the colz palette
void uvwdrawSetupCanvas(TCanvas &cv)
{
  // the ctor's size is the WINDOW (batch drew 1196 x 572); SetCanvasSize makes the image exactly 1200 x 600
  cv.SetCanvasSize(1200, 600);
  cv.Divide(3, 2);
  for (int i = 1; i <= 6; ++i) {
    TVirtualPad *p = cv.cd(i);
    p->SetLeftMargin(0.14);
    p->SetRightMargin(0.16);
    p->SetBottomMargin(0.13);
    p->SetTopMargin(0.10);
  }
}

// One event onto the six pads. The frames of the previous event are deleted (pad Clear), the histograms are not.
void uvwdrawDrawEvent(TCanvas &cv, UInt_t eventId, Long64_t entry, TH2D *const hd[3], const UvwdrawPlane pl[3], TH2D *const hx[3],
                      const std::vector<UvwdrawPoint> &pts, const double rng[4], double zFullHi, double drawMinAdc, bool logz, bool zoom)
{
  double W, H;
  uvwdrawFramePx(cv.cd(1), W, H);
  UvwdrawSig sig[3];
  for (int v = 0; v < 3; ++v) {
    sig[v].has = false;
    if (zoom) uvwdrawSignalRange(hd[v], sig[v]);
  }
  const UvwdrawTopWindow tw = uvwdrawTopWindow(sig, zoom, zFullHi, pl, W, H);
  for (int v = 0; v < 3; ++v) {
    TVirtualPad *p = cv.cd(v + 1);
    p->Clear();
    p->SetLogz(logz ? 1 : 0);
    hd[v]->SetMaximum();  // forget the colour range the painter kept from the previous event (-1111 = automatic)
    hd[v]->SetMinimum();
    if (drawMinAdc >= 0) {
      hd[v]->SetMinimum(drawMinAdc);
      p->SetFrameFillColor(TColor::GetColorPalette(0));
    }
    p->DrawFrame(tw.z0, tw.s0[v], tw.z1, tw.s1[v], uvwdrawTitle(eventId, entry, v).c_str());
    hd[v]->Draw("colz same");
  }
  uvwdrawFramePx(cv.cd(4), W, H);
  const UvwdrawBotWindow bw = uvwdrawBotWindow(pts, zoom, rng, zFullHi, W, H);
  const double *box[3] = {bw.xy, bw.xz, bw.yz};
  for (int k = 0; k < 3; ++k) {
    TVirtualPad *p = cv.cd(k + 4);
    p->Clear();
    hx[k]->SetMaximum();  // as above: the painter's colour range of the previous event
    hx[k]->SetMinimum();
    p->DrawFrame(box[k][0], box[k][2], box[k][1], box[k][3], uvwdrawTitleXYZ(eventId, entry, k).c_str());
    hx[k]->Draw("colz same");
  }
}

// mkdir -p. ROOT's recursive mkdir fails on a relative path ("a/b/c" -> -1), so make it absolute first.
bool uvwdrawMakeDir(const TString &path)
{
  TString abs(path);
  if (!gSystem->IsAbsoluteFileName(path)) gSystem->PrependPathName(gSystem->WorkingDirectory(), abs);
  gSystem->mkdir(abs, kTRUE);
  FileStat_t st;
  return gSystem->GetPathInfo(abs, st) == 0 && R_ISDIR(st.fMode);
}

void drawUVW(const char *rawFile, double pressureMbar, double samplingMHz, double driftHvVolt, const char *out = "",
             Long64_t first = 0, Long64_t n = -1, double drawMinAdc = -1, bool logz = false, bool zoom = true)
{
  // --- everything is checked BEFORE anything is created -----------------------
  if (!uvwdrawCheckRunConstants(pressureMbar, samplingMHz, driftHvVolt)) return;
  int view[4][68], strip[4][68], nStrips[3];
  std::vector<double> sPos[3];
  if (!uvwdrawLoadMap("channel_map.csv", view, strip, nStrips, sPos)) {
    std::cout << "ERROR: channel_map.csv not found (run from the macros folder); nothing written" << std::endl;
    return;
  }
  for (int v = 0; v < 3; ++v)
    if (nStrips[v] < 1) {
      std::cout << "ERROR: channel_map.csv has no strip in view " << v << "; nothing written" << std::endl;
      return;
    }
  UvwdrawPlane pl[3];
  for (int v = 0; v < 3; ++v)
    if (!uvwdrawFitPlane(sPos[v], "UVW"[v], pl[v])) return;  // ERROR printed, nothing created
  TFile in(rawFile);
  TTree *tree = (TTree *)in.Get("raw");
  if (!tree) {
    std::cout << "ERROR: no tree 'raw' in " << rawFile << " - this macro takes the *_raw.root from graw2root.C; nothing written"
              << std::endl;
    return;
  }
  UInt_t eventId;
  static Short_t adc[4][68][512];
  tree->SetBranchAddress("eventId", &eventId);
  tree->SetBranchAddress("adc", adc);

  const Long64_t nTotal = tree->GetEntries();
  if (first < 0 || first >= nTotal || n == 0) {
    std::cout << "ERROR: bad entry range first=" << first << " n=" << n << " (tree has " << nTotal << " entries); nothing written"
              << std::endl;
    return;
  }
  const Long64_t last = (n < 0) ? nTotal : std::min(nTotal, first + n);

  TString outName(out);
  const bool pdf = outName.EndsWith(".pdf");
  if (outName.IsNull()) {
    outName = rawFile;
    if (!outName.Contains("_raw.root")) {
      std::cout << "ERROR: no '_raw.root' in the input name " << rawFile << " and no out given; nothing written" << std::endl;
      return;
    }
    outName.ReplaceAll("_raw.root", "_uvw_events");
  }
  if (outName == TString(rawFile)) {
    std::cout << "ERROR: output name equals the input name (" << rawFile << "); refusing to overwrite the input" << std::endl;
    return;
  }
  bool dirExists = false;
  if (!pdf) {
    FileStat_t st;
    if (gSystem->GetPathInfo(outName, st) == 0) {
      if (!R_ISDIR(st.fMode)) {
        std::cout << "ERROR: " << outName << " exists and is not a directory; nothing written" << std::endl;
        return;
      }
      dirExists = true;
    }
  }

  // --- c and the XY ranges (from the map and the first events) ------------------
  const double mmPerCell = uvwdrawMmPerCell(pressureMbar, samplingMHz, driftHvVolt);
  const double pitch[3] = {uvwdrawPitch(sPos[0]), uvwdrawPitch(sPos[1]), uvwdrawPitch(sPos[2])};
  TH2D *h[3];
  uvwdrawMakeHists(nStrips, h);
  double c = 0;
  {
    std::vector<UvwdrawCell> cells;
    const Long64_t nc = std::min<Long64_t>(UVWDRAW_C_EVENTS, nTotal);
    for (Long64_t i = 0; i < nc; ++i) {
      tree->GetEntry(i);
      fillUVW(adc, view, strip, h);
      for (int t = 0; t < UVWDRAW_NBUCKET; ++t) {
        UvwdrawCell cell;
        for (int v = 0; v < 3; ++v) cell.cl[v] = uvwdrawClustersAtCell(h[v], sPos[v], pitch[v], t);
        if (!cell.cl[0].empty() && !cell.cl[1].empty() && !cell.cl[2].empty()) cells.push_back(cell);
      }
    }
    int nUsed = 0;
    c = uvwdrawClosureC(cells, &nUsed);
    if (nUsed == 0)
      std::cout << "WARNING: no cell with exactly one cluster in each plane in the first " << nc << " events; closure constant c = 0"
                << std::endl;
    else
      std::printf("drawUVW: closure constant c = %.4f mm (median of %d cells, first %lld events)\n", c, nUsed, nc);
  }
  double rng[4];
  if (!uvwdrawRanges(sPos, c, rng)) {
    std::cout << "ERROR: no (U, V) strip pair closes into the W range of channel_map.csv; nothing written" << std::endl;
    for (int v = 0; v < 3; ++v) delete h[v];
    return;
  }

  // --- create the output (only now) ----------------------------------------------
  if (!pdf) {
    if (dirExists) {
      std::cout << "INFO: directory " << outName << " exists; PNG files of the same name are overwritten" << std::endl;
    } else if (!uvwdrawMakeDir(outName)) {
      std::cout << "ERROR: cannot create directory " << outName << "; nothing written" << std::endl;
      for (int v = 0; v < 3; ++v) delete h[v];
      return;
    }
  }

  // --- draw ----------------------------------------------------------------------
  TH2D *hx[3], *hd[3];
  const double zFullHi = UVWDRAW_NBUCKET * mmPerCell;
  uvwdrawMakeXYZ(rng, zFullHi, hx);
  uvwdrawMakeDispHists(pl, mmPerCell, hd);
  TCanvas cv("cUVW", "UVW events", 1200, 600);
  uvwdrawSetupCanvas(cv);
  gStyle->SetOptStat(0);
  if (pdf) cv.Print(outName + "[");
  std::vector<UvwdrawPoint> pts;
  Long64_t pages = 0;
  for (Long64_t i = first; i < last; ++i) {
    tree->GetEntry(i);
    fillUVW(adc, view, strip, h);
    uvwdrawEventPoints(h, sPos, pitch, c, mmPerCell, pts);
    uvwdrawFillPoints(pts, hx);
    uvwdrawToDisp(h, pl, hd);
    uvwdrawDrawEvent(cv, eventId, i, hd, pl, hx, pts, rng, zFullHi, drawMinAdc, logz, zoom);
    if (pdf)
      cv.Print(outName, Form("Title:event %u", eventId));
    else
      cv.Print(Form("%s/event%07u.png", outName.Data(), eventId));
    ++pages;
    if (pages % 500 == 0) std::printf("drawUVW: %lld events done\n", pages);
  }
  if (pdf) cv.Print(outName + "]");
  for (int v = 0; v < 3; ++v) {
    delete h[v];
    delete hd[v];
    delete hx[v];
  }
  std::printf("drawUVW: wrote %lld %s (entries [%lld, %lld) of %lld) to %s\n", pages, pdf ? "pages" : "PNG files", first, last, nTotal,
              outName.Data());
  // ROOT's TPDF keeps byte offsets in 32-bit ints: above 2 GiB the xref table is broken
  // (negative startxref; seen with run 209 = 7946 pages, 4.18 GB). MuPDF repairs it on open.
  FileStat_t st;
  if (pdf && gSystem->GetPathInfo(outName, st) == 0 && st.fSize > 2147483647LL)
    std::printf("WARNING: %s is %.2f GB > 2 GiB - ROOT wrote a broken xref table. Fix it with\n"
                "  mutool clean %s <fixed.pdf>\n"
                "or split the run with first / n.\n",
                outName.Data(), st.fSize / 1e9, outName.Data());
}
