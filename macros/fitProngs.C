// =============================================================================
// fitProngs.C - forward-model fit of ONE alpha per event (TODO/197 v0, hardened by TODO/201 = v0.1).
//
// The model (a 3D segment with a Bragg curve) is drawn on the strip x cell image of each
// plane U / V / W and compared with the raw data by chi2 (TODO/192, Fable verdict). No 3D
// points are made: the 3 one-dimensional projections are compared as they are.
//
// Data grid (the rule of fillUVW in drawUVW.C, copied - drawUVW.C is not included): per strip
// the raw ADC of its channels is summed (clamped to 0..65535), the mean of the first 25 cells
// is subtracted. Per strip also: ceiling = 4095 - baseline (saturation of the model), noise =
// RMS of the first 25 cells; the noise sigma_v of a plane = median over its strips.
// A strip with a raw sample >= 4090 is "saturated" and left out of the chi2 (TODO/195: such a
// strip drags a ~+100 ADC tail to the end of the window, which the clipped model does not have).
//
// TODO/201 (v0.1): (1) a model point outside the sensitive box x in [-53, 53], y in [-58, 48] mm (the cage
//   of contained.C / stoppedAlphas.C) carries no charge; (2) every end point is a limited Minuit variable,
//   initial +- 10 mm, and a fit ending within 0.01 mm of a limit sets bit j in `atLimit`; (3) A can be fixed
//   per run by `adcPerMeV` (= A x kernel area, integrated ADC per MeV per plane; 0 = free; fixed A: 8
//   parameters, ndf = bins - 8); (4) common mode: per plane and cell the median over the strips OUTSIDE the
//   ROI (channel, not saturated; needs >= 8, else the plane is left alone = bit in `cmSkipped`) is
//   subtracted from the whole plane, then sigma_v is taken again (RMS of the first 25 cells, median);
//   (5) pixel variance = sigma_v^2 + (f max(data, 0))^2, f = 0.10 (`fracErr`), made from the data so it is
//   constant during a fit; (6) tree: sigmaXY = sqrt(sigmaT^2 + 0.146), atLimit, cmSkipped, noise[3];
//   (6b) the initial segment (tracks F end points) is first clipped along itself to x +-53, y -58..48,
//   z 0..512 mmPerCell (fpClipSegment); the clipped ends are the initial values (and the ROI). Tree
//   `initClipped`: bit 0 = start moved, bit 1 = end moved, bit 2 = nothing inside: not fitted (a row with
//   chi2 = -1, status = -1 marks it);
//   (7) images in mm (x = z, y = s) with the fitted segment (thin line, start = open circle, stop = filled
//   circle) on the model and residual rows only.
//
// Model, parameters S (x, y, z), E (x, y, z), A, sigma_T, sigma_L (9). E = the STOPPING end.
//   - nPts = ceil(|E0 - S0| / 0.1 mm) points at the midpoints of equal steps h = L / nPts
//     (nPts is fixed during a fit, so the chi2 is smooth in L; h ~ 0.1 mm), charge
//     q = A * dE/dx(R) * h, R = residual range to E, dE/dx * h = the integral of E(R) over the
//     step (see fpPoint). E(R) = NIST ASTAR projected range of alphas in CO2 (table copied from
//     stoppedAlphas.C, log-log interpolation, gas density from the pressure at 293.15 K as in
//     stoppedAlphas.C; below 0.1 MeV see fpEnergyAndSlope).
//   - across: plane v sees s0 = p_v . (x, y) (p_v = PITCH of makeTracks.C); strip k gets the
//     integral of a Gaussian (sigma_T) over its band [s_k - pitch/2, s_k + pitch/2], s_k from
//     channel_map.csv. Charge outside the strips is lost (a track may leave the sensitive area).
//     sigma_T is the width of THIS band model: TPCReco's diamond pads (1.0 mm across a strip,
//     triangular) are narrower than the 1.5 mm band, and TPCReco's sigma_xy matches our
//     sigma_T = sqrt(sigma_xy^2 - 0.146 mm^2) (TODO/197 cross-check: 1.0 -> 0.92, 0.8 -> 0.70).
//   - along z: cell j is [j - 1/2, j + 1/2) x mmPerCell (z = cell x mmPerCell as in analyzeUVW.C);
//     a Gaussian (sigma_L) is integrated over the cells, then convolved with the measured
//     response kernel (TODO/138) whose PEAK is put on the arrival cell (the z origin is
//     relative anyway). Charge outside the 512 cells is lost.
//   - the model is cut at the strip ceilings (saturation).
//   Gaussians are cut at 5 sigma (< 1e-6 of the charge).
// Kernel: all.mean_norm of pulser_kernel.C JSON (25 MHz, peak 1). The mean of its first 25 samples
//   (a constant floor, ~0.8 % of the peak in run 107) is subtracted, then (12.5 MHz) pairs of
//   neighbouring samples are averaged, the pair phase starting at the peak sample (25 MHz peak
//   40 -> pair (40, 41) -> cell 20), and the result is normalised to peak 1 again.
// chi2 = sum over planes and ROI pixels of (data - model)^2 / (sigma_v^2 + (f max(data, 0))^2). ROI = the (strip, cell)
//   whose (s_k, z_n) is within 15 mm in s AND within 15 mm in z of one point of the projection
//   of the initial segment (fixed during the fit), minus saturated strips. ndf = bins - 9 (A free) or - 8 (A fixed).
// Fit: initial = xStartF..zEndF of makeTracks.C, A from the ROI charge, sigma_T = sigma_L = 1 mm
//   (both free in [0.3, 5] mm). Minuit2 / Migrad once with the stop at the tracks file's End
//   (dir 0) and once with the stop at its Start (dir 1); the smaller chi2 wins. A Migrad that
//   does not converge keeps its chi2 (status != 0 is written; cut on it in the analysis).
//   The fit cannot measure v_drift (TODO/066: a straight line stays straight in (strip, cell)).
//
// Usage (from the macros folder, it reads channel_map.csv there like analyzeUVW.C):
//   root -l -b -q 'fitProngs.C+("data/run0209_raw.root", "data/run0209_tracks.root", 198, 12.5, 1600,
//                               "data/kernel/run0107_kernel.json", "run0209_fit.root", "0,5,27", "png", 81406)'
//   fitProngs(rawFile, tracksFile, pressureMbar, samplingMHz, driftHvVolt, kernelJson, outFile,
//             entryList = "", pngDir = "", adcPerMeV = 0, fracErr = 0.10, commonMode = true)
//   adcPerMeV: > 0 fixes A = adcPerMeV / kernel area (stored as TParameter adc_per_mev); 0 = A free.
//   fracErr / commonMode: the error model's f and the common-mode switch (tests and the TODO/201 2 x 2 table).
//   pressureMbar / samplingMHz / driftHvVolt: run constants, checked as in analyzeUVW.C;
//     mmPerCell = driftVelocityCmUs(driftHvVolt, 20, pressureMbar) / samplingMHz * 10 (a
//     WARNING if the tracks file's mm_per_cell differs). The kernel is 25 MHz: samplingMHz must
//     be 25 or 12.5.
//   entryList: entries of the raw tree (= tracks tree, same order), separated by commas or
//     spaces; "" = every entry with ok = 1. Entries with ok = 0 are skipped (a message).
//   pngDir: "" = no images; else event<%07u eventId>.png = 3 planes x (data / model / data -
//     model), strip x cell axes, the bounding box of the ROI.
// Output: TTree "fit", one entry per fitted event: eventId, entry, dir (0 = stop at the tracks
//   End, 1 = at its Start), chi2[2] (index = dir), ndf, xS yS zS (entering end) xE yE zE
//   (stopping end), A [ADC at the kernel peak per MeV in one cell], sigmaT, sigmaL [mm], length
//   [mm], energy [MeV] (ASTAR energy of the length, -1 beyond 10 MeV), status[2] (Minuit2 status
//   per dir, 0 = converged), ncalls[2], nSatStrips (saturated strips taken out of the ROI),
//   time_s (both fits), sigmaXY [mm], atLimit (bit j: end parameter xS yS zS xE yE zE at its limit),
//   cmSkipped (bit v: plane v not common-mode subtracted), noise[3] (sigma_v used). Per-view integrated ADC per MeV = A x kernel area (printed).
// Test: root -l -b -q 'test_fitProngs.C+' (exit code = failures; see its header).
// =============================================================================
#include <Math/Factory.h>
#include <Math/Functor.h>
#include <Math/Minimizer.h>
#include <Math/MinimizerOptions.h>
#include <TCanvas.h>
#include <TFile.h>
#include <TH2D.h>
#include <TLine.h>
#include <TMarker.h>
#include <TParameter.h>
#include <TStopwatch.h>
#include <TString.h>
#include <TVirtualPad.h>
#include <TSystem.h>
#include <TTree.h>

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <iostream>
#include <memory>
#include <sstream>
#include <string>
#include <utility>
#include <vector>

const int FP_NCELL = 512;
const int FP_BASELINE_CELLS = 25;     // = UVWDRAW_BASELINE_CELLS (drawUVW.C) = root-histo.ts
const double FP_STEP_MM = 0.1;        // charge points along the segment
const double FP_ROI_MM = 15.0;        // ROI half-width in s and in z
const double FP_SAT_ADC = 4090.0;     // a raw sample >= this: saturated strip (TODO/195)
const double FP_ADC_MAX = 4095.0;     // 12-bit ADC
const double FP_TEMP_K = 293.15;      // = TEMP_K of stoppedAlphas.C
const double FP_DRIFT_LEN_CM = 20.0;  // = DRIFT_LEN_CM of analyzeUVW.C
const double FP_NSIG = 5.0;           // Gaussians cut at this many sigma
const double FP_SIGMA_MIN = 0.3, FP_SIGMA_MAX = 5.0, FP_SIGMA_INIT = 1.0;  // [mm]
const int FP_MAX_CALLS = 4000;        // Migrad call limit per orientation
const double FP_TOLERANCE = 0.1;      // Minuit2: EDM < 0.002 x tolerance (chi2 units)
const int FP_MAX_BAND = 512;          // strips / cells one Gaussian can touch
// sensitive box [mm]: the cage of contained.C / stoppedAlphas.C (CAGE_XLO.. from fastqa); a point outside
// carries no charge (TODO/201: the strips are longer than the box, 73 had V / W bands outside it)
const double FP_CAGE_X = 53.0, FP_CAGE_Y_LO = -58.0, FP_CAGE_Y_HI = 48.0;
const double FP_END_BOX_MM = 10.0;    // an end point may move this far from its initial value (TODO/201)
const double FP_AT_LIMIT_MM = 0.01;   // within this of a limit = atLimit
const double FP_FRAC_ERR = 0.10;      // pixel sigma^2 = noise^2 + (FRAC * max(data, 0))^2 (TODO/197: residual ~ 10 % of the peak)
const int FP_CM_MIN_STRIPS = 8;       // common mode needs at least this many strips outside the ROI
const double FP_SIGMA_XY_PAD2 = 0.146;  // sigmaXY^2 = sigmaT^2 + this [mm^2] (diamond pad, TODO/197)
// strip pitch vectors of U, V, W: s = pitch_v . (x, y) (= PITCH of makeTracks.C)
const double FP_PITCH[3][2] = {{-1, 0}, {0.5, 0.8660254}, {0.5, -0.8660254}};

// NIST ASTAR, alpha in CO2: kinetic energy [MeV], PROJECTED range [g/cm2] - copied from
// stoppedAlphas.C (https://physics.nist.gov/PhysRefData/Star/Text/ASTAR.html, matno 134)
const double FP_ASTAR[][2] = {
    {1.000E-01, 1.450E-04}, {1.250E-01, 1.692E-04}, {1.500E-01, 1.913E-04},
    {1.750E-01, 2.118E-04}, {2.000E-01, 2.311E-04}, {2.250E-01, 2.493E-04},
    {2.500E-01, 2.666E-04}, {2.750E-01, 2.832E-04}, {3.000E-01, 2.992E-04},
    {3.500E-01, 3.295E-04}, {4.000E-01, 3.583E-04}, {4.500E-01, 3.858E-04},
    {5.000E-01, 4.124E-04}, {5.500E-01, 4.383E-04}, {6.000E-01, 4.638E-04},
    {6.500E-01, 4.891E-04}, {7.000E-01, 5.142E-04}, {7.500E-01, 5.393E-04},
    {8.000E-01, 5.645E-04}, {8.500E-01, 5.899E-04}, {9.000E-01, 6.155E-04},
    {9.500E-01, 6.414E-04}, {1.000E+00, 6.677E-04}, {1.250E+00, 8.045E-04},
    {1.500E+00, 9.528E-04}, {1.750E+00, 1.114E-03}, {2.000E+00, 1.287E-03},
    {2.250E+00, 1.474E-03}, {2.500E+00, 1.674E-03}, {2.750E+00, 1.887E-03},
    {3.000E+00, 2.112E-03}, {3.500E+00, 2.601E-03}, {4.000E+00, 3.138E-03},
    {4.500E+00, 3.722E-03}, {5.000E+00, 4.353E-03}, {5.500E+00, 5.029E-03},
    {6.000E+00, 5.749E-03}, {6.500E+00, 6.513E-03}, {7.000E+00, 7.320E-03},
    {7.500E+00, 8.170E-03}, {8.000E+00, 9.061E-03}, {8.500E+00, 9.994E-03},
    {9.000E+00, 1.097E-02}, {9.500E+00, 1.198E-02}, {1.000E+01, 1.304E-02},
};
const int FP_N_ASTAR = sizeof(FP_ASTAR) / sizeof(FP_ASTAR[0]);

// ---- types -----------------------------------------------------------------------------
struct FpPlane {  // strip k (1..n) at s = slope * k + offset [mm]; pitch = |slope|
  int n = 0;
  double slope = 0, offset = 0, pitch = 0;
};
struct FpGeom {
  FpPlane pl[3];
  double mmPerCell = 0;
  int ncell = FP_NCELL;
};
struct FpKernel {  // out[n] += q[j] * h[n - j + i0]; h[i0] = 1 = the peak
  std::vector<double> h;
  int i0 = 0;
};
struct FpImage {  // per view: strip k (1-based) x cell t, row-major
  int n[3] = {0, 0, 0};
  int ncell = FP_NCELL;
  std::vector<double> v[3];
  void init(const FpGeom &g)
  {
    ncell = g.ncell;
    for (int i = 0; i < 3; ++i) {
      n[i] = g.pl[i].n;
      v[i].assign((size_t)n[i] * ncell, 0.0);
    }
  }
  double &at(int view, int k, int t) { return v[view][(size_t)(k - 1) * ncell + t]; }
  double get(int view, int k, int t) const { return v[view][(size_t)(k - 1) * ncell + t]; }
};
struct FpData {  // one event on the grid
  FpImage img;
  double noise[3] = {0, 0, 0};
  std::vector<double> ceil[3];  // per strip: 4095 - baseline
  std::vector<char> sat[3];     // per strip: a raw sample >= FP_SAT_ADC
  std::vector<char> hasCh[3];   // per strip: a channel in the map
};
struct FpTrack {  // E = the stopping end
  double S[3] = {0, 0, 0}, E[3] = {0, 0, 0};
  double A = 0, sigT = 0, sigL = 0;
};
struct FpFit {  // par = xS yS zS xE yE zE A sigmaT sigmaL (E = stopping end)
  double par[9] = {0, 0, 0, 0, 0, 0, 0, 0, 0};
  double chi2 = 0;
  int nBins = 0, status = -1, ncalls = 0;
  int atLimit = 0;  // bit j: end point parameter j (xS yS zS xE yE zE) within FP_AT_LIMIT_MM of its +-FP_END_BOX_MM limit
};
struct FpEventFit {
  FpFit fit[2];  // [0] stop at the tracks End, [1] at its Start
  int dir = -1, nSat = 0, ndf = 0;
};

// ---- ASTAR -----------------------------------------------------------------------------
// CO2 mass density [g/cm3] (ideal gas, M = 44.01 g/mol) - gasDensity() of stoppedAlphas.C
double fpGasDensity(double pMbar) { return 44.01 * pMbar * 100.0 / (8.314 * FP_TEMP_K) * 1e-6; }

// projected range [mm] of an alpha of energy E [MeV] - rangeFromEnergy() of stoppedAlphas.C
double fpRangeMm(double E, double rho)
{
  for (int i = 1; i < FP_N_ASTAR; ++i)
    if (E <= FP_ASTAR[i][0]) {
      const double f = std::log(E / FP_ASTAR[i - 1][0]) / std::log(FP_ASTAR[i][0] / FP_ASTAR[i - 1][0]);
      return FP_ASTAR[i - 1][1] * std::pow(FP_ASTAR[i][1] / FP_ASTAR[i - 1][1], f) / rho * 10.0;
    }
  return -1;
}

// E [MeV] and the local log-log slope b = dlnE / dlnR for a mass range r [g/cm2], log-log
// interpolation of the table. Outside the table the first / last row's power law is extended
// (beyond = true above 10 MeV). NOTE: energyFromRange of stoppedAlphas.C uses E ~ sqrt(r) below
// 0.1 MeV (R < 4.1 mm at 198 mbar); that puts a x2.9 step into dE/dx at 0.1 MeV (b 1.45 -> 0.5)
// and a dE/dx ~ 1/sqrt(R) spike at the very end of the track, so the model extends the first row
// instead (dE/dx -> 0 at R -> 0, continuous at 0.1 MeV).
double fpEnergyAndSlope(double r, double &b, bool &beyond)
{
  int i = 1;
  while (i < FP_N_ASTAR - 1 && r > FP_ASTAR[i][1]) ++i;
  beyond = r > FP_ASTAR[FP_N_ASTAR - 1][1];
  b = std::log(FP_ASTAR[i][0] / FP_ASTAR[i - 1][0]) / std::log(FP_ASTAR[i][1] / FP_ASTAR[i - 1][1]);
  if (!(r > 0)) return 0;
  return FP_ASTAR[i - 1][0] * std::pow(r / FP_ASTAR[i - 1][1], b);
}

// energy [MeV] of residual range R [mm] - energyFromRange() of stoppedAlphas.C inside the table
// (-1 beyond 10 MeV; below 0.1 MeV see fpEnergyAndSlope)
double fpEnergyFromRangeMm(double R, double rho)
{
  double b;
  bool beyond;
  const double E = fpEnergyAndSlope(R / 10.0 * rho, b, beyond);
  return beyond ? -1 : E;
}

// energy [MeV] of residual range R [mm], extended beyond 10 MeV (the model's E(R))
double fpEnergyModel(double R, double rho)
{
  double b;
  bool beyond;
  return R > 0 ? fpEnergyAndSlope(R / 10.0 * rho, b, beyond) : 0;
}

// dE/dx [MeV/mm] at residual range R [mm] = dE/dR = b E / R
double fpDedx(double R, double rho)
{
  if (!(R > 0)) return 0;
  double b;
  bool beyond;
  const double E = fpEnergyAndSlope(R / 10.0 * rho, b, beyond);
  return b * E / R;
}

// ---- model pieces ----------------------------------------------------------------------
int fpNumPoints(double L) { return std::max(1, (int)std::ceil(L / FP_STEP_MM - 1e-9)); }

// point i of nPts: position (midpoint of step i from S to E) -> xyz, returns its charge
// A * dE/dx * h, with dE/dx * h taken as the integral of the ASTAR curve over the step,
// E(R + h/2) - E(R - h/2) (R = residual range to E at the point): the log-log table has steps in
// dE/dR at its nodes, and a point crossing a node made the chi2 jump (Migrad stalled); the
// integral is continuous in L and the charges add up to A E(L) exactly.
double fpPoint(const FpTrack &t, double rho, int nPts, int i, double xyz[3])
{
  const double u = (i + 0.5) / nPts;
  double L2 = 0;
  for (int j = 0; j < 3; ++j) {
    const double d = t.E[j] - t.S[j];
    xyz[j] = t.S[j] + u * d;
    L2 += d * d;
  }
  const double L = std::sqrt(L2);
  return t.A * (fpEnergyModel((1 - (double)i / nPts) * L, rho) - fpEnergyModel((1 - (i + 1.0) / nPts) * L, rho));
}

// Gaussian (centre x0, sigma sig) integrated over the bands [c_k - w/2, c_k + w/2],
// c_k = slope * k + offset, k in [kMin, kMax], w = |slope|; only bands within FP_NSIG sigma.
// Returns the number of bands written to k[] / wt[] (at most cap).
int fpBandWeights(double x0, double sig, double slope, double offset, int kMin, int kMax, int *k, double *wt, int cap)
{
  if (kMax < kMin || slope == 0 || !(sig > 0)) return 0;
  const double pitch = std::fabs(slope);
  const double u = (x0 - offset) / slope, du = (FP_NSIG * sig + 0.5 * pitch) / pitch;
  const int lo = std::max(kMin, (int)std::floor(u - du)), hi = std::min(kMax, (int)std::ceil(u + du));
  const double inv = 1.0 / (sig * std::sqrt(2.0)), sgn = slope > 0 ? 0.5 : -0.5;
  // neighbouring bands share an edge (|slope| = width): edge k|k+1 at c_k + slope / 2
  double ePrev = std::erf((slope * lo + offset - 0.5 * slope - x0) * inv);
  int m = 0;
  for (int kk = lo; kk <= hi && m < cap; ++kk) {
    const double e = std::erf((slope * kk + offset + 0.5 * slope - x0) * inv);
    const double w = sgn * (e - ePrev);
    ePrev = e;
    if (w <= 0) continue;
    k[m] = kk;
    wt[m] = w;
    ++m;
  }
  return m;
}

// charge fraction of the strips of a plane for a Gaussian at s0 (sigma sig)
int fpStripWeights(const FpPlane &pl, double s0, double sig, int *k, double *w, int cap)
{
  return fpBandWeights(s0, sig, pl.slope, pl.offset, 1, pl.n, k, w, cap);
}

// 25 MHz samples (peak ~1) -> kernel at 25 / factor MHz: minus the mean of the first 25 samples,
// mean of groups of `factor` samples (phase: a group starts at the peak sample), peak = 1.
bool fpKernelFromSamples(const std::vector<double> &h25, int factor, FpKernel &k, std::string &err)
{
  if (factor < 1) {
    err = "factor < 1";
    return false;
  }
  if ((int)h25.size() <= FP_BASELINE_CELLS + factor) {
    err = "kernel has " + std::to_string(h25.size()) + " samples, need > " + std::to_string(FP_BASELINE_CELLS + factor);
    return false;
  }
  double floorV = 0;
  for (int i = 0; i < FP_BASELINE_CELLS; ++i) floorV += h25[i];
  floorV /= FP_BASELINE_CELLS;
  int ipk = 0;
  for (size_t i = 0; i < h25.size(); ++i)
    if (h25[i] > h25[ipk]) ipk = (int)i;
  const int start = ipk % factor, ng = ((int)h25.size() - start) / factor;
  k.h.assign(ng, 0.0);
  for (int m = 0; m < ng; ++m) {
    double s = 0;
    for (int j = 0; j < factor; ++j) s += h25[start + m * factor + j] - floorV;
    k.h[m] = s / factor;
  }
  k.i0 = 0;
  for (int m = 0; m < ng; ++m)
    if (k.h[m] > k.h[k.i0]) k.i0 = m;
  const double pk = k.h[k.i0];
  if (!(pk > 0)) {
    err = "kernel peak <= 0 after the floor subtraction";
    return false;
  }
  for (double &x : k.h) x /= pk;
  return true;
}

// all.mean_norm of a pulser_kernel.C JSON (25 MHz) -> kernel at samplingMHz (25 or 12.5)
bool fpLoadKernel(const char *path, double samplingMHz, FpKernel &k, std::string &err)
{
  const double ratio = 25.0 / samplingMHz;
  const int factor = (int)std::lround(ratio);
  if (factor < 1 || std::fabs(ratio - factor) > 1e-9) {
    err = Form("the kernel is 25 MHz: samplingMHz = %g needs 25 / samplingMHz to be an integer >= 1", samplingMHz);
    return false;
  }
  std::ifstream f(path);
  if (!f) {
    err = std::string("cannot read kernel JSON ") + path;
    return false;
  }
  std::stringstream ss;
  ss << f.rdbuf();
  const std::string js = ss.str();
  size_t p = js.find("\"all\"");
  if (p != std::string::npos) p = js.find("\"mean_norm\"", p);
  if (p != std::string::npos) p = js.find('[', p);
  if (p == std::string::npos) {
    err = std::string("no all.mean_norm array in ") + path;
    return false;
  }
  std::vector<double> h;
  const char *c = js.c_str() + p + 1;
  while (*c && *c != ']') {
    char *end = nullptr;
    const double x = std::strtod(c, &end);
    if (end == c) {
      ++c;  // ',' or white space
      continue;
    }
    h.push_back(x);
    c = end;
  }
  if (*c != ']') {
    err = std::string("unterminated all.mean_norm in ") + path;
    return false;
  }
  return fpKernelFromSamples(h, factor, k, err);
}

// The model image of one track (out); work = scratch charge grid (same size). ceil: per view and
// strip ceiling, or nullptr (no saturation).
void fpRender(const FpTrack &t, double rho, int nPts, const FpGeom &g, const FpKernel &kern, const std::vector<double> *ceil,
              FpImage &out, FpImage &work)
{
  const int nc = g.ncell;
  for (FpImage *im : {&out, &work}) {
    bool same = im->ncell == nc;
    for (int v = 0; v < 3; ++v) same = same && im->n[v] == g.pl[v].n && im->v[v].size() == (size_t)g.pl[v].n * nc;
    if (same)
      for (int v = 0; v < 3; ++v) std::fill(im->v[v].begin(), im->v[v].end(), 0.0);
    else
      im->init(g);
  }
  std::vector<int> lo[3], hi[3];  // cells with charge per strip
  for (int v = 0; v < 3; ++v) {
    lo[v].assign(g.pl[v].n + 1, nc);
    hi[v].assign(g.pl[v].n + 1, -1);
  }
  int kz[FP_MAX_BAND], ks[FP_MAX_BAND];
  double wz[FP_MAX_BAND], ws[FP_MAX_BAND];
  double xyz[3];
  for (int i = 0; i < nPts; ++i) {
    const double q = fpPoint(t, rho, nPts, i, xyz);
    if (!(q > 0)) continue;
    if (xyz[0] < -FP_CAGE_X || xyz[0] > FP_CAGE_X || xyz[1] < FP_CAGE_Y_LO || xyz[1] > FP_CAGE_Y_HI) continue;  // outside the box
    const int nz = fpBandWeights(xyz[2], t.sigL, g.mmPerCell, 0.0, 0, nc - 1, kz, wz, FP_MAX_BAND);
    if (nz == 0) continue;
    for (int v = 0; v < 3; ++v) {
      const double s0 = FP_PITCH[v][0] * xyz[0] + FP_PITCH[v][1] * xyz[1];
      const int ns = fpStripWeights(g.pl[v], s0, t.sigT, ks, ws, FP_MAX_BAND);
      for (int a = 0; a < ns; ++a) {
        double *row = &work.v[v][(size_t)(ks[a] - 1) * nc];
        const double qs = q * ws[a];
        for (int b = 0; b < nz; ++b) row[kz[b]] += qs * wz[b];
        lo[v][ks[a]] = std::min(lo[v][ks[a]], kz[0]);
        hi[v][ks[a]] = std::max(hi[v][ks[a]], kz[nz - 1]);
      }
    }
  }
  const int nk = (int)kern.h.size(), i0 = kern.i0;
  for (int v = 0; v < 3; ++v)
    for (int k = 1; k <= g.pl[v].n; ++k) {
      if (hi[v][k] < lo[v][k]) continue;
      const double *qrow = &work.v[v][(size_t)(k - 1) * nc];
      double *orow = &out.v[v][(size_t)(k - 1) * nc];
      for (int j = lo[v][k]; j <= hi[v][k]; ++j) {
        const double qj = qrow[j];
        if (qj == 0) continue;
        const int mLo = std::max(0, i0 - j), mHi = std::min(nk, nc - j + i0);  // n = j + m - i0 in [0, nc)
        for (int m = mLo; m < mHi; ++m) orow[j + m - i0] += qj * kern.h[m];
      }
      if (ceil) {
        const double c = ceil[v][k - 1];
        const int nLo = std::max(0, lo[v][k] - i0), nHi = std::min(nc, hi[v][k] + nk - i0);
        for (int n = nLo; n < nHi; ++n) orow[n] = std::min(orow[n], c);
      }
    }
}

// t-range [lo, hi] where |a + t (b - a) - c| <= half; false = never
bool fpInterval(double a, double b, double c, double half, double &lo, double &hi)
{
  const double d = b - a;
  if (std::fabs(d) < 1e-12) {
    lo = -1e300;
    hi = 1e300;
    return std::fabs(a - c) <= half;
  }
  const double t1 = (c - half - a) / d, t2 = (c + half - a) / d;
  lo = std::min(t1, t2);
  hi = std::max(t1, t2);
  return true;
}

// Initial segment a -> b clipped along itself to the sensitive volume x +-FP_CAGE_X, y FP_CAGE_Y_LO..HI,
// z 0..ncell x mmPerCell (Liang-Barsky on the parameter t in [0, 1]); the direction is unchanged.
// flags: bit 0 = a moved, bit 1 = b moved, bit 2 = no part of the segment is inside (a2 / b2 = a / b).
// Returns false for bit 2.
bool fpClipSegment(const double a[3], const double b[3], const FpGeom &g, double a2[3], double b2[3], int &flags)
{
  const double lo[3] = {-FP_CAGE_X, FP_CAGE_Y_LO, 0.0}, hi[3] = {FP_CAGE_X, FP_CAGE_Y_HI, g.ncell * g.mmPerCell};
  double t0 = 0, t1 = 1;
  bool inside = true;
  for (int j = 0; j < 3 && inside; ++j) {
    const double d = b[j] - a[j];
    if (d == 0) {
      inside = a[j] >= lo[j] && a[j] <= hi[j];
      continue;
    }
    double u0 = (lo[j] - a[j]) / d, u1 = (hi[j] - a[j]) / d;
    if (u0 > u1) std::swap(u0, u1);
    t0 = std::max(t0, u0);
    t1 = std::min(t1, u1);
    if (t0 > t1) inside = false;
  }
  flags = 0;
  for (int j = 0; j < 3; ++j) {
    a2[j] = a[j];
    b2[j] = b[j];
  }
  if (!inside) {
    flags = 4;
    return false;
  }
  for (int j = 0; j < 3; ++j) {
    const double d = b[j] - a[j];
    a2[j] = a[j] + t0 * d;
    b2[j] = a[j] + t1 * d;
  }
  if (t0 > 0) flags |= 1;
  if (t1 < 1) flags |= 2;
  return true;
}

// ROI per view (strip x cell): a point of the projected segment S0 -> E0 within FP_ROI_MM in s
// and in z of (s_k, cell * mmPerCell)
void fpRoi(const double S0[3], const double E0[3], const FpGeom &g, std::vector<char> mask[3])
{
  const int nc = g.ncell;
  for (int v = 0; v < 3; ++v) {
    mask[v].assign((size_t)g.pl[v].n * nc, 0);
    const double sa = FP_PITCH[v][0] * S0[0] + FP_PITCH[v][1] * S0[1];
    const double sb = FP_PITCH[v][0] * E0[0] + FP_PITCH[v][1] * E0[1];
    for (int k = 1; k <= g.pl[v].n; ++k) {
      double l1, h1;
      if (!fpInterval(sa, sb, g.pl[v].slope * k + g.pl[v].offset, FP_ROI_MM, l1, h1)) continue;
      l1 = std::max(l1, 0.0);
      h1 = std::min(h1, 1.0);
      if (l1 > h1) continue;
      for (int n = 0; n < nc; ++n) {
        double l2, h2;
        if (!fpInterval(S0[2], E0[2], n * g.mmPerCell, FP_ROI_MM, l2, h2)) continue;
        if (std::max(l1, l2) <= std::min(h1, h2)) mask[v][(size_t)(k - 1) * nc + n] = 1;
      }
    }
  }
}

// chi2 over the mask; excl[v][k-1] = 1 leaves strip k out (empty excl = none). nBins = summed pixels.
// Pixel variance = noise_v^2 + (frac * max(data, 0))^2: made from the DATA (not the model), so it is a
// constant during a fit (no bias in the estimate, ndf unchanged). frac = 0: noise only.
double fpChi2(const FpImage &data, const FpImage &model, const std::vector<char> mask[3], const std::vector<char> excl[3],
              const double noise[3], int &nBins, double frac)
{
  double chi2 = 0;
  nBins = 0;
  for (int v = 0; v < 3; ++v) {
    const double n2 = noise[v] * noise[v];
    const int nc = data.ncell;
    for (int k = 1; k <= data.n[v]; ++k) {
      if (!excl[v].empty() && excl[v][k - 1]) continue;
      const size_t r = (size_t)(k - 1) * nc;
      for (int t = 0; t < nc; ++t) {
        if (!mask[v][r + t]) continue;
        const double dv = data.v[v][r + t], d = dv - model.v[v][r + t];
        const double e = frac * std::max(dv, 0.0);
        chi2 += d * d / (n2 + e * e);
        ++nBins;
      }
    }
  }
  return chi2;
}

// ---- data ------------------------------------------------------------------------------
// channel_map.csv -> view / strip per (aget, raw ch) (-1 = not in the map) and the 3 planes
// (s_mm must be on s = slope * strip + offset within 1e-6 mm, as drawUVW.C requires).
bool fpLoadMap(const char *path, int view[4][68], int strip[4][68], FpGeom &g, std::string &err)
{
  for (int a = 0; a < 4; ++a)
    for (int c = 0; c < 68; ++c) {
      view[a][c] = -1;
      strip[a][c] = 0;
    }
  std::ifstream fm(path);
  if (!fm) {
    err = std::string(path) + " not found (run from the macros folder)";
    return false;
  }
  std::vector<std::pair<int, double>> pos[3];  // (strip, s_mm)
  for (std::string line; std::getline(fm, line);) {
    if (line.empty() || line[0] == '#') continue;
    std::istringstream ss(line);
    int a, c, v, s;
    double p;
    char comma;
    if (!(ss >> a >> comma >> c >> comma >> v >> comma >> s >> comma >> p)) continue;
    if (a < 0 || a >= 4 || c < 0 || c >= 68 || v < 0 || v >= 3 || s < 1) {
      std::cout << "WARNING: " << path << " line skipped (out of range): " << line << std::endl;
      continue;
    }
    view[a][c] = v;
    strip[a][c] = s;
    pos[v].push_back({s, p});
  }
  const char *vn = "UVW";
  for (int v = 0; v < 3; ++v) {
    if (pos[v].size() < 2) {
      err = Form("plane %c has fewer than 2 strips in %s", vn[v], path);
      return false;
    }
    std::sort(pos[v].begin(), pos[v].end());
    const auto &f = pos[v].front(), &b = pos[v].back();
    if (b.first == f.first) {
      err = Form("plane %c: one strip number only in %s", vn[v], path);
      return false;
    }
    FpPlane &pl = g.pl[v];
    pl.n = b.first;
    pl.slope = (b.second - f.second) / (b.first - f.first);
    pl.offset = f.second - pl.slope * f.first;
    pl.pitch = std::fabs(pl.slope);
    for (const auto &e : pos[v])
      if (std::fabs(pl.slope * e.first + pl.offset - e.second) > 1e-6) {
        err = Form("plane %c strip %d: s = %.4f is off the line s = %.6f * strip + %.6f in %s", vn[v], e.first, e.second,
                   pl.slope, pl.offset, path);
        return false;
      }
  }
  return true;
}

// one event -> grid (fillUVW rule), noise per plane, per-strip ceiling / saturation flags
void fpFillData(const Short_t adc[4][68][512], const int view[4][68], const int strip[4][68], const FpGeom &g, FpData &d)
{
  d.img.init(g);
  const int nc = std::min(g.ncell, 512);
  for (int v = 0; v < 3; ++v) {
    d.ceil[v].assign(g.pl[v].n, 0.0);
    d.sat[v].assign(g.pl[v].n, 0);
    d.hasCh[v].assign(g.pl[v].n, 0);
  }
  for (int a = 0; a < 4; ++a)
    for (int c = 0; c < 68; ++c) {
      const int v = view[a][c], k = strip[a][c];
      if (v < 0 || k < 1 || k > g.pl[v].n) continue;
      double *row = &d.img.v[v][(size_t)(k - 1) * g.ncell];
      d.hasCh[v][k - 1] = 1;
      for (int t = 0; t < nc; ++t) {
        row[t] += adc[a][c][t];
        if (adc[a][c][t] >= FP_SAT_ADC) d.sat[v][k - 1] = 1;
      }
    }
  for (int v = 0; v < 3; ++v) {
    std::vector<double> rms;
    for (int k = 1; k <= g.pl[v].n; ++k) {
      if (!d.hasCh[v][k - 1]) continue;
      double *row = &d.img.v[v][(size_t)(k - 1) * g.ncell];
      for (int t = 0; t < nc; ++t) row[t] = std::min(std::max(row[t], 0.0), 65535.0);
      double base = 0;
      for (int t = 0; t < FP_BASELINE_CELLS; ++t) base += row[t];
      base /= FP_BASELINE_CELLS;
      for (int t = 0; t < nc; ++t) row[t] -= base;
      double s2 = 0;
      for (int t = 0; t < FP_BASELINE_CELLS; ++t) s2 += row[t] * row[t];
      rms.push_back(std::sqrt(s2 / FP_BASELINE_CELLS));
      d.ceil[v][k - 1] = FP_ADC_MAX - base;
    }
    std::sort(rms.begin(), rms.end());
    const size_t m = rms.size();
    d.noise[v] = m == 0 ? 0 : ((m % 2) ? rms[m / 2] : 0.5 * (rms[m / 2 - 1] + rms[m / 2]));
  }
}

// sigma_v of a plane = median over its strips (with a channel) of the RMS of the first 25 cells
void fpTakeNoise(FpData &d, int v)
{
  std::vector<double> rms;
  for (int k = 1; k <= d.img.n[v]; ++k) {
    if (!d.hasCh[v][k - 1]) continue;
    double s2 = 0;
    for (int t = 0; t < FP_BASELINE_CELLS; ++t) s2 += d.img.get(v, k, t) * d.img.get(v, k, t);
    rms.push_back(std::sqrt(s2 / FP_BASELINE_CELLS));
  }
  std::sort(rms.begin(), rms.end());
  const size_t m = rms.size();
  d.noise[v] = m == 0 ? 0 : ((m % 2) ? rms[m / 2] : 0.5 * (rms[m / 2 - 1] + rms[m / 2]));
}

// Common mode (TODO/201): per plane and cell t, c_t = the median over the strips OUTSIDE the ROI
// (a channel, not saturated; "in the ROI" = a pixel of the strip is in mask), subtracted from every
// strip of the plane; then sigma_v is taken again. A plane with fewer than FP_CM_MIN_STRIPS usable
// strips is left alone. Returns the bit mask of the skipped planes (bit v) = cmSkipped.
int fpSubtractCommonMode(FpData &d, const std::vector<char> mask[3])
{
  int skipped = 0;
  const int nc = d.img.ncell;
  for (int v = 0; v < 3; ++v) {
    std::vector<int> use;
    for (int k = 1; k <= d.img.n[v]; ++k) {
      if (!d.hasCh[v][k - 1] || d.sat[v][k - 1]) continue;
      bool inRoi = false;
      for (int t = 0; t < nc && !inRoi; ++t) inRoi = mask[v][(size_t)(k - 1) * nc + t];
      if (!inRoi) use.push_back(k);
    }
    if ((int)use.size() < FP_CM_MIN_STRIPS) {
      skipped |= 1 << v;
      continue;
    }
    std::vector<double> col(use.size()), c(nc);
    for (int t = 0; t < nc; ++t) {
      for (size_t i = 0; i < use.size(); ++i) col[i] = d.img.get(v, use[i], t);
      std::sort(col.begin(), col.end());
      const size_t m = col.size();
      c[t] = (m % 2) ? col[m / 2] : 0.5 * (col[m / 2 - 1] + col[m / 2]);
    }
    for (int k = 1; k <= d.img.n[v]; ++k) {
      if (!d.hasCh[v][k - 1]) continue;
      for (int t = 0; t < nc; ++t) d.img.at(v, k, t) -= c[t];
    }
    fpTakeNoise(d, v);
  }
  return skipped;
}

double fpKernelArea(const FpKernel &k)
{
  double s = 0;
  for (double x : k.h) s += x;
  return s;
}

// A fixed by adcPerMeV (integrated ADC per MeV per plane = A x kernel area); 0 = free
double fpFixedA(double adcPerMeV, const FpKernel &k)
{
  const double area = fpKernelArea(k);
  return (adcPerMeV > 0 && area > 0) ? adcPerMeV / area : 0.0;
}

// mm per cell from the run constants - driftVelocityCmUs of analyzeUVW.C (same warning text)
double fpDriftVelocityCmUs(double hvVolt, double lenCm, double pMbar)
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
double fpMmPerCell(double pressureMbar, double samplingMHz, double driftHvVolt)
{
  return fpDriftVelocityCmUs(driftHvVolt, FP_DRIFT_LEN_CM, pressureMbar) / samplingMHz * 10.0;
}

// ---- fit -------------------------------------------------------------------------------

// A from the ROI charge: sum of the data in the ROI of 3 planes = 3 A E(L0) area(kernel)
double fpInitialA(const FpData &d, const std::vector<char> mask[3], double L0, double rho, const FpKernel &k)
{
  double sum = 0;
  for (int v = 0; v < 3; ++v)
    for (size_t i = 0; i < mask[v].size(); ++i)
      if (mask[v][i]) sum += d.img.v[v][i];
  double b;
  bool beyond;
  const double E0 = fpEnergyAndSlope(L0 / 10.0 * rho, b, beyond);
  const double A0 = sum / (3.0 * E0 * fpKernelArea(k));
  return (std::isfinite(A0) && A0 > 0) ? A0 : 1.0;
}

// one Migrad from start0 (entering end) / stop0 (stopping end)
FpFit fpFitOne(const FpData &d, const FpGeom &g, const FpKernel &k, double rho, const std::vector<char> mask[3],
               const std::vector<char> excl[3], const double start0[3], const double stop0[3], double A0, double frac,
               double fixedA)
{
  FpFit r;
  double L0 = 0;
  for (int j = 0; j < 3; ++j) L0 += (stop0[j] - start0[j]) * (stop0[j] - start0[j]);
  const int nPts = fpNumPoints(std::sqrt(L0));
  FpImage model, work;
  int ncalls = 0;
  const bool freeA = !(fixedA > 0);
  const int o = freeA ? 1 : 0;  // Minuit index shift: with A fixed it is not a variable
  auto chi2Of = [&](const double *p) {
    ++ncalls;
    FpTrack t;
    for (int j = 0; j < 3; ++j) {
      t.S[j] = p[j];
      t.E[j] = p[3 + j];
    }
    t.A = freeA ? p[6] : fixedA;
    t.sigT = p[6 + o];
    t.sigL = p[7 + o];
    fpRender(t, rho, nPts, g, k, d.ceil, model, work);
    int nb;
    return fpChi2(d.img, model, mask, excl, d.noise, nb, frac);
  };
  std::unique_ptr<ROOT::Math::Minimizer> min(ROOT::Math::Factory::CreateMinimizer("Minuit2", "Migrad"));
  if (!min) {
    std::cout << "ERROR: cannot create the Minuit2 minimizer" << std::endl;
    r.status = -2;
    return r;
  }
  ROOT::Math::Functor fcn(chi2Of, 8 + o);
  min->SetFunction(fcn);
  min->SetMaxFunctionCalls(FP_MAX_CALLS);
  min->SetTolerance(FP_TOLERANCE);
  min->SetErrorDef(1.0);
  min->SetPrintLevel(ROOT::Math::MinimizerOptions::DefaultPrintLevel());  // 0 unless set (debug)
  const char *nm[6] = {"xS", "yS", "zS", "xE", "yE", "zE"};
  double init[6];
  for (int j = 0; j < 6; ++j) {  // Minuit2 wants the variables in index order; end points in initial +- FP_END_BOX_MM
    init[j] = j < 3 ? start0[j] : stop0[j - 3];
    min->SetLimitedVariable(j, nm[j], init[j], 0.5, init[j] - FP_END_BOX_MM, init[j] + FP_END_BOX_MM);
  }
  if (freeA) min->SetLowerLimitedVariable(6, "A", A0, 0.1 * A0, 0.0);
  min->SetLimitedVariable(6 + o, "sigmaT", FP_SIGMA_INIT, 0.1, FP_SIGMA_MIN, FP_SIGMA_MAX);
  min->SetLimitedVariable(7 + o, "sigmaL", FP_SIGMA_INIT, 0.1, FP_SIGMA_MIN, FP_SIGMA_MAX);
  min->Minimize();
  r.status = min->Status();
  const double *x = min->X();
  for (int j = 0; j < 6; ++j) r.par[j] = x[j];
  r.par[6] = freeA ? x[6] : fixedA;
  r.par[7] = x[6 + o];
  r.par[8] = x[7 + o];
  for (int j = 0; j < 6; ++j)
    if (std::fabs(x[j] - (init[j] - FP_END_BOX_MM)) < FP_AT_LIMIT_MM || std::fabs(x[j] - (init[j] + FP_END_BOX_MM)) < FP_AT_LIMIT_MM)
      r.atLimit |= 1 << j;
  r.chi2 = chi2Of(x);
  fpChi2(d.img, model, mask, excl, d.noise, r.nBins, frac);
  r.ncalls = ncalls;
  return r;
}

// both orientations; S0 / E0 = the tracks file's Start / End. excludeSat: leave saturated strips out.
FpEventFit fpFitEvent(const FpData &d, const FpGeom &g, const FpKernel &k, double rho, const double S0[3], const double E0[3],
                      bool excludeSat, double frac, double adcPerMeV)
{
  FpEventFit r;
  std::vector<char> mask[3], excl[3];
  fpRoi(S0, E0, g, mask);
  for (int v = 0; v < 3; ++v) {
    excl[v].assign(g.pl[v].n, 0);
    for (int kk = 1; kk <= g.pl[v].n; ++kk) {
      bool inRoi = false;
      for (int t = 0; t < g.ncell && !inRoi; ++t) inRoi = mask[v][(size_t)(kk - 1) * g.ncell + t];
      if (!d.hasCh[v].empty() && !d.hasCh[v][kk - 1]) excl[v][kk - 1] = 1;
      if (excludeSat && !d.sat[v].empty() && d.sat[v][kk - 1]) {
        excl[v][kk - 1] = 1;
        if (inRoi) ++r.nSat;
      }
    }
  }
  double L0 = 0;
  for (int j = 0; j < 3; ++j) L0 += (E0[j] - S0[j]) * (E0[j] - S0[j]);
  const double A0 = fpInitialA(d, mask, std::sqrt(L0), rho, k);
  const double fixedA = fpFixedA(adcPerMeV, k);
  r.fit[0] = fpFitOne(d, g, k, rho, mask, excl, S0, E0, A0, frac, fixedA);
  r.fit[1] = fpFitOne(d, g, k, rho, mask, excl, E0, S0, A0, frac, fixedA);
  if (r.fit[0].status >= -1 || r.fit[1].status >= -1) r.dir = r.fit[1].chi2 < r.fit[0].chi2 ? 1 : 0;
  r.ndf = r.fit[0].nBins - (fixedA > 0 ? 8 : 9);
  return r;
}

// ---- image -----------------------------------------------------------------------------
// mm axes as drawUVW.C: x = z [mm] (cell j = [j - 1/2, j + 1/2) x mmPerCell), y = s [mm] (strip k = s_k +- pitch/2).
// The fitted segment (thin line, start = entering end open circle, stop = stopping end filled circle) is drawn on
// the model and residual rows only, never on the data row (TODO/192 3-2).
void fpDrawEvent(const FpData &d, const FpImage &model, const std::vector<char> mask[3], const FpGeom &g, UInt_t eventId,
                 Long64_t entry, const FpEventFit &f, const std::string &path)
{
  const char *vn[3] = {"U", "V", "W"}, *rowName[3] = {"data", "model", "data - model"};
  TCanvas c("fpCanvas", "", 1200, 900);
  c.Divide(3, 3);
  std::vector<TH2D *> hs;
  std::vector<TObject *> marks;
  const FpFit &b = f.fit[f.dir < 0 ? 0 : f.dir];
  for (int v = 0; v < 3; ++v) {
    int k0 = g.pl[v].n + 1, k1 = 0, t0 = g.ncell, t1 = -1;
    for (int k = 1; k <= g.pl[v].n; ++k)
      for (int t = 0; t < g.ncell; ++t)
        if (mask[v][(size_t)(k - 1) * g.ncell + t]) {
          k0 = std::min(k0, k);
          k1 = std::max(k1, k);
          t0 = std::min(t0, t);
          t1 = std::max(t1, t);
        }
    if (k1 < k0) continue;
    const double slope = g.pl[v].slope, half = 0.5 * g.pl[v].pitch;
    const double sA = slope * k0 + g.pl[v].offset, sB = slope * k1 + g.pl[v].offset;
    const double sLo = std::min(sA, sB) - half, sHi = std::max(sA, sB) + half;
    double rmax = 0;
    for (int row = 0; row < 3; ++row) {
      auto *h = new TH2D(Form("fp_%d_%d", v, row),
                         Form("event %u (entry %lld) %s %s  chi2/ndf %.0f/%d = %.2f;z [mm];s [mm]", eventId, entry, vn[v],
                              rowName[row], b.chi2, f.ndf, f.ndf > 0 ? b.chi2 / f.ndf : 0.0),
                         t1 - t0 + 1, (t0 - 0.5) * g.mmPerCell, (t1 + 0.5) * g.mmPerCell, k1 - k0 + 1, sLo, sHi);
      h->SetDirectory(nullptr);
      h->SetStats(0);
      for (int k = k0; k <= k1; ++k) {
        const int by = slope > 0 ? k - k0 + 1 : k1 - k + 1;
        for (int t = t0; t <= t1; ++t) {
          const double dv = d.img.get(v, k, t), mv = model.get(v, k, t);
          const double val = row == 0 ? dv : (row == 1 ? mv : dv - mv);
          h->SetBinContent(t - t0 + 1, by, val);
          if (row == 2) rmax = std::max(rmax, std::fabs(val));
        }
      }
      if (row == 2) {
        h->SetMinimum(-rmax);
        h->SetMaximum(rmax);
      }
      c.cd(1 + v + 3 * row);
      gPad->SetRightMargin(0.14);
      h->Draw("colz");
      hs.push_back(h);
      if (row > 0) {
        const double sS = FP_PITCH[v][0] * b.par[0] + FP_PITCH[v][1] * b.par[1];
        const double sE = FP_PITCH[v][0] * b.par[3] + FP_PITCH[v][1] * b.par[4];
        auto *ln = new TLine(b.par[2], sS, b.par[5], sE);
        ln->SetLineWidth(1);
        ln->Draw();
        auto *m0 = new TMarker(b.par[2], sS, 24);
        auto *m1 = new TMarker(b.par[5], sE, 20);
        for (TMarker *m : {m0, m1}) {
          m->SetMarkerSize(0.8);
          m->Draw();
        }
        marks.push_back(ln);
        marks.push_back(m0);
        marks.push_back(m1);
      }
    }
  }
  c.SaveAs(path.c_str());
  for (auto *h : hs) delete h;
  for (auto *m : marks) delete m;
}

// ---- entry point -----------------------------------------------------------------------
void fitProngs(const char *rawFile, const char *tracksFile, double pressureMbar, double samplingMHz, double driftHvVolt,
               const char *kernelJson, const char *outFile, const char *entryList = "", const char *pngDir = "",
               double adcPerMeV = 0, double fracErr = FP_FRAC_ERR, bool commonMode = true)
{
  // run constants (as analyzeUVW.C), checked before anything is read or written
  const double SAMPLING_OK[4] = {100.0, 50.0, 25.0, 12.5};
  bool samplingOk = false;
  for (double s : SAMPLING_OK)
    if (samplingMHz == s) samplingOk = true;
  if (!samplingOk) {
    std::cout << "ERROR: samplingMHz = " << samplingMHz << " is not one of 100, 50, 25, 12.5; nothing written" << std::endl;
    return;
  }
  if (!(driftHvVolt > 0) || !(pressureMbar > 0)) {
    std::cout << "ERROR: driftHvVolt and pressureMbar must be > 0; nothing written" << std::endl;
    return;
  }
  std::string err;
  FpKernel kern;
  if (!fpLoadKernel(kernelJson, samplingMHz, kern, err)) {
    std::cout << "ERROR: " << err << "; nothing written" << std::endl;
    return;
  }
  FpGeom geom;
  int view[4][68], strip[4][68];
  if (!fpLoadMap("channel_map.csv", view, strip, geom, err)) {
    std::cout << "ERROR: " << err << "; nothing written" << std::endl;
    return;
  }
  geom.mmPerCell = fpMmPerCell(pressureMbar, samplingMHz, driftHvVolt);
  geom.ncell = FP_NCELL;
  const double rho = fpGasDensity(pressureMbar);
  std::unique_ptr<TFile> fr(TFile::Open(rawFile));
  std::unique_ptr<TFile> ft(TFile::Open(tracksFile));
  TTree *raw = fr && !fr->IsZombie() ? (TTree *)fr->Get("raw") : nullptr;
  TTree *trk = ft && !ft->IsZombie() ? (TTree *)ft->Get("tracks") : nullptr;
  if (!raw || !trk) {
    std::cout << "ERROR: no tree 'raw' in " << rawFile << " or 'tracks' in " << tracksFile << "; nothing written" << std::endl;
    return;
  }
  if (raw->GetEntries() != trk->GetEntries()) {
    std::cout << "ERROR: " << raw->GetEntries() << " raw entries but " << trk->GetEntries() << " tracks; nothing written"
              << std::endl;
    return;
  }
  auto *mpc = (TParameter<double> *)ft->Get("mm_per_cell");
  if (mpc && std::fabs(mpc->GetVal() - geom.mmPerCell) > 1e-6 * geom.mmPerCell)
    std::cout << "WARNING: mm_per_cell " << geom.mmPerCell << " from the arguments differs from the tracks file's "
              << mpc->GetVal() << " (the arguments are used)" << std::endl;
  static Short_t adc[4][68][512];
  UInt_t eventId = 0, tEventId = 0;
  Int_t ok = 0;
  Float_t xs, ys, zs, xe, ye, ze;
  raw->SetBranchAddress("eventId", &eventId);
  raw->SetBranchAddress("adc", adc);
  trk->SetBranchAddress("eventId", &tEventId);
  trk->SetBranchAddress("ok", &ok);
  trk->SetBranchAddress("xStartF", &xs);
  trk->SetBranchAddress("yStartF", &ys);
  trk->SetBranchAddress("zStartF", &zs);
  trk->SetBranchAddress("xEndF", &xe);
  trk->SetBranchAddress("yEndF", &ye);
  trk->SetBranchAddress("zEndF", &ze);

  std::vector<Long64_t> entries;
  const std::string list(entryList ? entryList : "");
  if (list.find_first_not_of(" ,\t") == std::string::npos) {
    trk->SetBranchStatus("*", 0);
    trk->SetBranchStatus("ok", 1);
    for (Long64_t i = 0; i < trk->GetEntries(); ++i) {
      trk->GetEntry(i);
      if (ok == 1) entries.push_back(i);
    }
    trk->SetBranchStatus("*", 1);
  } else {
    std::string s = list;
    std::replace(s.begin(), s.end(), ',', ' ');
    std::istringstream ss(s);
    for (std::string tok; ss >> tok;) {
      char *end = nullptr;
      const long long e = std::strtoll(tok.c_str(), &end, 10);
      if (*end != 0 || e < 0 || e >= raw->GetEntries()) {
        std::cout << "ERROR: entryList item '" << tok << "' is not an entry of " << rawFile << "; nothing written" << std::endl;
        return;
      }
      entries.push_back(e);
    }
  }
  const std::string png(pngDir ? pngDir : "");
  if (!png.empty()) gSystem->mkdir(png.c_str(), kTRUE);

  TFile out(outFile, "RECREATE");
  if (out.IsZombie()) {
    std::cout << "ERROR: cannot write " << outFile << std::endl;
    return;
  }
  TTree tree("fit", "forward-model fit, one alpha (TODO/197)");
  Long64_t entry = 0;
  Int_t dir = 0, ndf = 0, status[2] = {0, 0}, ncalls[2] = {0, 0}, nSat = 0, atLimit = 0, cmSkipped = 0, initClipped = 0;
  Double_t sigmaXY = 0, noiseOut[3] = {0, 0, 0};
  Double_t chi2[2] = {0, 0}, xS, yS, zS, xE, yE, zE, A, sigmaT, sigmaL, length, energy, timeS;
  tree.Branch("eventId", &eventId);
  tree.Branch("entry", &entry);
  tree.Branch("dir", &dir);
  tree.Branch("chi2", chi2, "chi2[2]/D");
  tree.Branch("ndf", &ndf);
  tree.Branch("xS", &xS);
  tree.Branch("yS", &yS);
  tree.Branch("zS", &zS);
  tree.Branch("xE", &xE);
  tree.Branch("yE", &yE);
  tree.Branch("zE", &zE);
  tree.Branch("A", &A);
  tree.Branch("sigmaT", &sigmaT);
  tree.Branch("sigmaL", &sigmaL);
  tree.Branch("length", &length);
  tree.Branch("energy", &energy);
  tree.Branch("status", status, "status[2]/I");
  tree.Branch("ncalls", ncalls, "ncalls[2]/I");
  tree.Branch("nSatStrips", &nSat);
  tree.Branch("time_s", &timeS);
  tree.Branch("sigmaXY", &sigmaXY);
  tree.Branch("atLimit", &atLimit);
  tree.Branch("cmSkipped", &cmSkipped);
  tree.Branch("initClipped", &initClipped);
  tree.Branch("noise", noiseOut, "noise[3]/D");

  const double kArea = fpKernelArea(kern);
  std::cout << Form("fitProngs: %zu entries, mm/cell %.6f, rho %.4e g/cm3, kernel %zu cells (i0 %d, area/peak %.3f)",
                    entries.size(), geom.mmPerCell, rho, kern.h.size(), kern.i0, kArea)
            << std::endl;
  if (adcPerMeV < 0) {
    std::cout << "ERROR: adcPerMeV < 0; nothing written" << std::endl;
    return;
  }
  const double fixedA = fpFixedA(adcPerMeV, kern);
  std::cout << Form("fitProngs: adcPerMeV %g (A %s %g), fracErr %g, common mode %s", adcPerMeV, fixedA > 0 ? "fixed =" : "free, start", fixedA,
                    fracErr, commonMode ? "on" : "off")
            << std::endl;
  FpData data;
  int nFit = 0;
  for (Long64_t e : entries) {
    raw->GetEntry(e);
    trk->GetEntry(e);
    if (tEventId != eventId) {
      std::cout << "WARNING: entry " << e << ": eventId " << eventId << " (raw) != " << tEventId << " (tracks), skipped" << std::endl;
      continue;
    }
    if (ok != 1) {
      std::cout << "entry " << e << " (event " << eventId << "): ok = " << ok << " in the tracks file, not fitted" << std::endl;
      continue;
    }
    fpFillData(adc, view, strip, geom, data);
    const double Sraw[3] = {xs, ys, zs}, Eraw[3] = {xe, ye, ze};
    double S0[3], E0[3];
    entry = e;
    initClipped = 0;
    if (!fpClipSegment(Sraw, Eraw, geom, S0, E0, initClipped)) {
      // no part of the initial segment is in the sensitive volume: not fitted; a row with initClipped = 4 (bit 2) is the mark
      std::cout << "entry " << e << " (event " << eventId << "): the initial segment is outside the sensitive box, not fitted" << std::endl;
      dir = -1, ndf = 0, nSat = 0, atLimit = 0, cmSkipped = 0, timeS = 0;
      for (int j = 0; j < 2; ++j) chi2[j] = -1, status[j] = -1, ncalls[j] = 0;
      xS = yS = zS = xE = yE = zE = A = sigmaT = sigmaL = sigmaXY = length = 0;
      energy = -1;
      for (int v = 0; v < 3; ++v) noiseOut[v] = 0;
      tree.Fill();
      continue;
    }
    cmSkipped = 0;
    if (commonMode) {
      std::vector<char> roi[3];
      fpRoi(S0, E0, geom, roi);
      cmSkipped = fpSubtractCommonMode(data, roi);
      if (cmSkipped)
        std::cout << "INFO: entry " << e << ": common mode not subtracted in planes (bit mask " << cmSkipped
                  << "), < 8 strips outside the ROI" << std::endl;
    }
    if (!(data.noise[0] > 0 && data.noise[1] > 0 && data.noise[2] > 0)) {
      std::cout << "WARNING: entry " << e << ": a plane has noise 0 (empty?), skipped" << std::endl;
      continue;
    }
    TStopwatch sw;
    const FpEventFit f = fpFitEvent(data, geom, kern, rho, S0, E0, true, fracErr, adcPerMeV);
    timeS = sw.RealTime();
    if (f.dir < 0) {
      std::cout << "WARNING: entry " << e << ": no fit (minimizer failed), skipped" << std::endl;
      continue;
    }
    const FpFit &b = f.fit[f.dir];
    entry = e;
    dir = f.dir;
    ndf = f.ndf;
    nSat = f.nSat;
    for (int j = 0; j < 2; ++j) {
      chi2[j] = f.fit[j].chi2;
      status[j] = f.fit[j].status;
      ncalls[j] = f.fit[j].ncalls;
    }
    xS = b.par[0], yS = b.par[1], zS = b.par[2];
    xE = b.par[3], yE = b.par[4], zE = b.par[5];
    A = b.par[6], sigmaT = b.par[7], sigmaL = b.par[8];
    sigmaXY = std::sqrt(sigmaT * sigmaT + FP_SIGMA_XY_PAD2);
    atLimit = b.atLimit;
    for (int v = 0; v < 3; ++v) noiseOut[v] = data.noise[v];
    length = std::sqrt((xE - xS) * (xE - xS) + (yE - yS) * (yE - yS) + (zE - zS) * (zE - zS));
    energy = fpEnergyFromRangeMm(length, rho);
    tree.Fill();
    ++nFit;
    std::cout << Form("entry %lld event %u: dir %d chi2 %.0f / %.0f ndf %d (%.2f) L %.1f mm E %.2f MeV A %.0f (%.0f ADC/MeV per "
                      "view) sT %.2f sL %.2f status %d/%d sat %d atLimit %d cm %d clip %d %.2f s",
                      e, eventId, dir, chi2[0], chi2[1], ndf, ndf > 0 ? b.chi2 / ndf : 0.0, length, energy, A, A * kArea,
                      sigmaT, sigmaL, status[0], status[1], nSat, atLimit, cmSkipped, initClipped, timeS)
              << std::endl;
    if (!png.empty()) {
      FpTrack t;
      for (int j = 0; j < 3; ++j) {
        t.S[j] = b.par[j];
        t.E[j] = b.par[3 + j];
      }
      t.A = A, t.sigT = sigmaT, t.sigL = sigmaL;
      double L0 = 0;
      for (int j = 0; j < 3; ++j) L0 += (E0[j] - S0[j]) * (E0[j] - S0[j]);
      FpImage model, work;
      fpRender(t, rho, fpNumPoints(std::sqrt(L0)), geom, kern, data.ceil, model, work);
      std::vector<char> mask[3];
      fpRoi(S0, E0, geom, mask);
      out.cd();
      fpDrawEvent(data, model, mask, geom, eventId, e, f, Form("%s/event%07u.png", png.c_str(), eventId));
    }
  }
  out.cd();
  tree.Write();
  TParameter<double>("pressure_mbar", pressureMbar, 'f').Write();
  TParameter<double>("sampling_MHz", samplingMHz, 'f').Write();
  TParameter<double>("drift_hv_V", driftHvVolt, 'f').Write();
  TParameter<double>("mm_per_cell", geom.mmPerCell, 'f').Write();
  TParameter<double>("kernel_area", kArea, 'f').Write();
  TParameter<double>("adc_per_mev", adcPerMeV, 'f').Write();
  TParameter<double>("frac_err", fracErr, 'f').Write();
  TParameter<double>("common_mode", commonMode ? 1.0 : 0.0, 'f').Write();
  out.Close();
  std::cout << "wrote " << outFile << " (" << nFit << " fits)" << std::endl;
}
