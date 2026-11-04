// =============================================================================
// fitProngs.C - forward-model fit of ONE alpha per event (TODO/197 v0, hardened by TODO/201 = v0.1), and of
// one alpha + one arm from its vertex (TODO/198, H1 = {alpha} vs H2 = {alpha + arm}).
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
// TODO/198 (H2 = alpha + arm): the arm is a uniform line of charge A x Eeq from the alpha's entering end S (= the vertex)
//   to E2 (xA2, yA2, zA2), 0.1 mm points (fpArmPoint), sharing sigma_T / sigma_L and the renderer with the alpha
//   (fpRenderProngs: one charge grid, then the kernel and the ceiling). 12 parameters with A fixed (Eeq >= 0, E2 within
//   +-10 mm of its seed). Per event (fpFitEventAll): (1) H1 on ROI_alpha (= the TODO/201 fit, the original branches);
//   (2) the arm seed from its residual (fpSeedArm: per view the 8-neighbour component of residual > 5 sigma_v with the
//   largest charge -> centroid and farthest pixel; xy by least squares of the s-bars, z = mean z-bar; no component =
//   seedFallback 1: vertex - 3 mm along the alpha, 0.05 MeV); ROI_event = ROI_alpha + per view vertex -> farthest pixel,
//   common mode again outside ROI_event; (3) H1 again on ROI_event (both orientations, warm); (4) H2 with the vertex at
//   either end (warm from (3)). dchi2 = chi2H1 - chi2H2 on the same ROI_event (nBinsEvent); hyp = 1 when dchi2 >
//   dchi2Threshold (> 0; 0 = no threshold, hyp always 0). armSat (bit v: a saturated strip within 5 mm of the arm in
//   plane v - armE is then a lower bound), armResid[2] (fpArmThirds). PNG: the shown hypothesis; H2 = 2 segments,
//   vertex = magenta open circle, alpha stop = filled circle, arm end = open square.
//
// TODO/198 v2 (Fable verdict 2026-11-04): (A) H2's minimised chi2 adds FP_ARM_PEN_W (100) x nBins x max(0, cosEff - 0.5)^2,
//   cosEff = fpArmCos(V, E, E2) = (E2 - V) . u_alpha / max(|E2 - V|, 1 mm) (an arm along the alpha is the degenerate "H1 +
//   amplitude" direction; recoils do not go along the alpha); chi2H2 and dchi2 include it; tree armCos, armPen, armAlong
//   (cosEff > 0.55). A box: with adcPerMeV > 0 and aBox > 0 (default 0.15) A is limited to A_ref x [1 - aBox, 1 + aBox] in
//   every fit (ROI_alpha H1 starts at A_ref, the later fits warm from the previous A), ndf = bins - 9 / - 13, atLimit /
//   atLimitH2 bit 10 = A at a box edge, tree aRelH1 / aRelH2 = A / A_ref (0 with A free); aBox 0 = A fixed (TODO/201).
//   (D) fpSeedArm leaves out a component whose charge centroid is within 2.5 mm of the alpha's (s, z) projection (tree
//   seedAlong, bit v); with no candidate left the fallback Eeq is the left-out charge (I1-a), and an H2 orientation for which the
//   shared seed lies along its alpha (cos > 0.5) starts from the fallback position instead (fpOrientSeed, tree orientFallback
//   bit d; I2-a - both from the main session's ruling on TODO/198 v2). (B) h1Broken = the ROI_event H1 has sigma_T or sigma_L within 0.01 mm of 5 mm -> hyp = -1 with a
//   threshold > 0 (flag only in calibration mode). (C) h2Worse = chi2H2 > chi2H1 -> dchi2 = 0 (flagged, not silent).
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
//             entryList = "", pngDir = "", adcPerMeV = 0, fracErr = 0.10, commonMode = true, dchi2Threshold = 0, aBox = 0.15)
//   adcPerMeV: > 0 gives A_ref = adcPerMeV / kernel area (stored as TParameter adc_per_mev); 0 = A free.
//   aBox: with adcPerMeV > 0, A in A_ref x [1 - aBox, 1 + aBox] (TParameter a_box; 0 = A fixed at A_ref, TODO/201; [0, 1)).
//   dchi2Threshold: hyp = 1 when dchi2 > it, -1 when h1Broken (TParameter dchi2_threshold); 0 = calibration (hyp = 0, dchi2 written).
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
//   TODO/198: chi2H1, chi2H2, dchi2, nBinsEvent, ndfH1 (bins - 8; - 9 with A free), ndfH2 (bins - 12; - 13), hyp, xV yV zV (vertex), xA2 yA2 zA2
//   (arm end), armLen [mm], armE [MeV eq.], armQ (= A x armE x kernel area, ADC), armDirX/Y/Z, armSat, seedFallback,
//   statusH2[2], atLimitH2 (bit 9: arm > 60 mm), dirH2, xEH2 yEH2 zEH2 / lengthH2 / sigmaTH2 / sigmaLH2 (H2's alpha),
//   cmSkippedEvent, armResid[2], timeTotal_s. TODO/198 v2: armCos, armPen, armAlong, aRelH1, aRelH2, seedAlong, h1Broken, h2Worse,
//   orientFallback
//   (atLimit / atLimitH2 bit 10: A at its box edge).
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
// TODO/198 (N = 2: alpha + a uniform arm from the vertex)
const double FP_ARM_MAX_MM = 60.0;          // an arm longer than this sets atLimit bit 9
const double FP_SEED_NSIG = 5.0;            // seed pixels: H1 residual > this x sigma_v
const double FP_SEED_FALLBACK_MM = 3.0;     // no seed component: arm end = vertex - this x (alpha direction) ...
const double FP_SEED_FALLBACK_MEV = 0.05;   // ... and Eeq = this
const double FP_ARM_SAT_MM = 5.0;           // a saturated strip within this (in s) of the arm: armSat
const double FP_WARM_MARGIN = 0.01;         // a warm start is moved this fraction of its range inside a limit
// TODO/198 v2 (Fable verdict 2026-11-04: A, C, D)
const double FP_ARM_COS_MAX = 0.5;          // H2 penalty FP_ARM_PEN_W x nBins x max(0, cosEff - this)^2 (an arm along the alpha)
const double FP_ARM_PEN_W = 100.0;
const double FP_ARM_COS_MIN_MM = 1.0;       // cosEff = (E2 - V) . u_alpha / max(|E2 - V|, this): -> 0 for a point arm
const double FP_ARM_ALONG_COS = 0.55;       // armAlong: the final cosEff above this
const double FP_SEED_ALONG_MM = 2.5;        // a seed component whose centroid is this close to the alpha's (s, z) projection: not a candidate
const double FP_A_BOX = 0.15;               // default aBox: A in A_ref x [1 - aBox, 1 + aBox] when adcPerMeV > 0
const double FP_A_AT_LIMIT_REL = 1e-3;      // A within this x A_ref of a box edge: atLimit bit 10
const double FP_H1_BROKEN_MM = 0.01;        // h1Broken: sigma_T or sigma_L of the H1 fit within this of FP_SIGMA_MAX
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
  double arm[4] = {0, 0, 0, 0};  // TODO/198, H2 only: arm end xA2 yA2 zA2 (the arm starts at S = the vertex), Eeq [MeV equivalent]
  bool hasArm = false;
  int nPts[2] = {0, 0};          // charge points of the alpha / the arm (fixed during the fit)
  double chi2 = 0;
  int nBins = 0, status = -1, ncalls = 0;
  int atLimit = 0;  // bit j: end point parameter j (xS yS zS xE yE zE, 6-8 = xA2 yA2 zA2) within FP_AT_LIMIT_MM of its
                    // +-FP_END_BOX_MM limit; bit 9: arm longer than FP_ARM_MAX_MM; bit 10: A at an edge of its box (TODO/198 v2)
  double cosEff = 0, pen = 0;  // TODO/198 v2, H2 only: fpArmCos of the solution and its penalty (included in chi2)
};
struct FpProng {  // TODO/198: one prong drawn into the event image
  bool arm = false;  // false = alpha (Bragg curve, stops at E); true = arm (uniform charge from S = the vertex to E)
  double S[3] = {0, 0, 0}, E[3] = {0, 0, 0};
  double Eeq = 0;    // arm: charge in MeV equivalent (total A x Eeq); the alpha's charge is A E(L)
  int nPts = 1;      // charge points (fixed during a fit)
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
double fpAlphaPoint(const double S[3], const double E[3], double A, double rho, int nPts, int i, double xyz[3])
{
  const double u = (i + 0.5) / nPts;
  double L2 = 0;
  for (int j = 0; j < 3; ++j) {
    const double d = E[j] - S[j];
    xyz[j] = S[j] + u * d;
    L2 += d * d;
  }
  const double L = std::sqrt(L2);
  return A * (fpEnergyModel((1 - (double)i / nPts) * L, rho) - fpEnergyModel((1 - (i + 1.0) / nPts) * L, rho));
}
double fpPoint(const FpTrack &t, double rho, int nPts, int i, double xyz[3]) { return fpAlphaPoint(t.S, t.E, t.A, rho, nPts, i, xyz); }

// TODO/198: point i of nPts of the arm V -> E2 (midpoint of step i), charge Q / nPts (uniform; Q = A x Eeq). nPts = 1 puts
// all of it at the midpoint (the 0-length arm = a point at the vertex).
double fpArmPoint(const double V[3], const double E2[3], double Q, int nPts, int i, double xyz[3])
{
  const double u = (i + 0.5) / nPts;
  for (int j = 0; j < 3; ++j) xyz[j] = V[j] + u * (E2[j] - V[j]);
  return Q / nPts;
}

// TODO/198 v2: the arm's direction relative to the alpha V -> E (V = the vertex = the alpha's entering end):
// (E2 - V) . u_alpha / max(|E2 - V|, FP_ARM_COS_MIN_MM), u_alpha = (E - V) / |E - V|; 0 for a 0-length alpha. For an arm shorter
// than 1 mm it is |E2 - V| x cos / 1 mm (-> 0 for a point arm, continuous).
double fpArmCos(const double V[3], const double E[3], const double E2[3])
{
  double la2 = 0, lb2 = 0, dot = 0;
  for (int j = 0; j < 3; ++j) {
    const double a = E[j] - V[j], b = E2[j] - V[j];
    la2 += a * a;
    lb2 += b * b;
    dot += a * b;
  }
  const double la = std::sqrt(la2);
  if (!(la > 0)) return 0;
  return dot / la / std::max(std::sqrt(lb2), FP_ARM_COS_MIN_MM);
}

// the H2 penalty of an arm along the alpha: FP_ARM_PEN_W x nBins x max(0, cosEff - FP_ARM_COS_MAX)^2 (added to H2's chi2)
double fpArmPenalty(double cosEff, int nBins)
{
  const double x = std::max(0.0, cosEff - FP_ARM_COS_MAX);
  return FP_ARM_PEN_W * nBins * x * x;
}

// distance in a plane from the point (px, pz) to the segment (ax, az) -> (bx, bz)
double fpDistToSegment2(double px, double pz, double ax, double az, double bx, double bz)
{
  const double dx = bx - ax, dz = bz - az, l2 = dx * dx + dz * dz;
  const double t = l2 > 0 ? std::min(1.0, std::max(0.0, ((px - ax) * dx + (pz - az) * dz) / l2)) : 0.0;
  return std::hypot(px - (ax + t * dx), pz - (az + t * dz));
}

// TODO/198 v2 flags: h1Broken = sigma_T or sigma_L of the H1 fit within FP_H1_BROKEN_MM of FP_SIGMA_MAX (the blob has no alpha
// hypothesis, Fable verdict B); dchi2 = chi2H1 - chi2H2, but 0 with h2Worse = 1 when chi2H2 > chi2H1 (Migrad stopped above H1,
// verdict C); hyp = -1 (h1Broken), 1 (dchi2 > threshold) or 0 with a threshold > 0, always 0 with threshold 0 (calibration).
int fpH1Broken(double sigT, double sigL) { return (FP_SIGMA_MAX - sigT <= FP_H1_BROKEN_MM || FP_SIGMA_MAX - sigL <= FP_H1_BROKEN_MM) ? 1 : 0; }
double fpDchi2(double chi2H1, double chi2H2, int &h2Worse)
{
  h2Worse = chi2H2 > chi2H1 ? 1 : 0;
  return h2Worse ? 0.0 : chi2H1 - chi2H2;
}
int fpHyp(double dchi2, double threshold, int h1Broken)
{
  if (!(threshold > 0)) return 0;
  if (h1Broken) return -1;
  return dchi2 > threshold ? 1 : 0;
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

// The model image of the prongs p[0..n) of one event (out); work = scratch charge grid (same size). ceil: per view and
// strip ceiling, or nullptr (no saturation). All prongs share A, sigma_T, sigma_L (one gas); their charge points go on ONE
// charge grid, then the kernel and the ceiling are applied once (saturation acts on the sum, TODO/198).
void fpRenderProngs(const FpProng *p, int n, double A, double sigT, double sigL, double rho, const FpGeom &g, const FpKernel &kern,
                    const std::vector<double> *ceil, FpImage &out, FpImage &work)
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
  for (int ip = 0; ip < n; ++ip) {
    const FpProng &pr = p[ip];
    for (int i = 0; i < pr.nPts; ++i) {
      const double q = pr.arm ? fpArmPoint(pr.S, pr.E, A * pr.Eeq, pr.nPts, i, xyz) : fpAlphaPoint(pr.S, pr.E, A, rho, pr.nPts, i, xyz);
      if (!(q > 0)) continue;
      if (xyz[0] < -FP_CAGE_X || xyz[0] > FP_CAGE_X || xyz[1] < FP_CAGE_Y_LO || xyz[1] > FP_CAGE_Y_HI) continue;  // outside the box
      const int nz = fpBandWeights(xyz[2], sigL, g.mmPerCell, 0.0, 0, nc - 1, kz, wz, FP_MAX_BAND);
      if (nz == 0) continue;
      for (int v = 0; v < 3; ++v) {
        const double s0 = FP_PITCH[v][0] * xyz[0] + FP_PITCH[v][1] * xyz[1];
        const int ns = fpStripWeights(g.pl[v], s0, sigT, ks, ws, FP_MAX_BAND);
        for (int a = 0; a < ns; ++a) {
          double *row = &work.v[v][(size_t)(ks[a] - 1) * nc];
          const double qs = q * ws[a];
          for (int b = 0; b < nz; ++b) row[kz[b]] += qs * wz[b];
          lo[v][ks[a]] = std::min(lo[v][ks[a]], kz[0]);
          hi[v][ks[a]] = std::max(hi[v][ks[a]], kz[nz - 1]);
        }
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

// The model image of one alpha (TODO/197)
void fpRender(const FpTrack &t, double rho, int nPts, const FpGeom &g, const FpKernel &kern, const std::vector<double> *ceil,
              FpImage &out, FpImage &work)
{
  FpProng p;
  for (int j = 0; j < 3; ++j) {
    p.S[j] = t.S[j];
    p.E[j] = t.E[j];
  }
  p.nPts = nPts;
  fpRenderProngs(&p, 1, t.A, t.sigT, t.sigL, rho, g, kern, ceil, out, work);
}

// The model image of a fit: the alpha, plus the arm S -> (xA2, yA2, zA2) for an H2 fit, with the fit's point counts
void fpRenderFit(const FpFit &f, double rho, const FpGeom &g, const FpKernel &kern, const std::vector<double> *ceil, FpImage &out,
                 FpImage &work)
{
  FpProng p[2];
  for (int j = 0; j < 3; ++j) {
    p[0].S[j] = p[1].S[j] = f.par[j];
    p[0].E[j] = f.par[3 + j];
    p[1].E[j] = f.arm[j];
  }
  p[0].nPts = std::max(1, f.nPts[0]);
  p[1].arm = true;
  p[1].Eeq = f.arm[3];
  p[1].nPts = std::max(1, f.nPts[1]);
  fpRenderProngs(p, f.hasArm ? 2 : 1, f.par[6], f.par[7], f.par[8], rho, g, kern, ceil, out, work);
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

// Adds to the ROI of view v (strip x cell) the pixels (s_k, cell * mmPerCell) within FP_ROI_MM in s and in z of one point
// of the (s, z) segment (sa, za) -> (sb, zb). maskV must have the size of the view (strips x cells).
void fpRoiAddView(int v, double sa, double za, double sb, double zb, const FpGeom &g, std::vector<char> &maskV)
{
  const int nc = g.ncell;
  for (int k = 1; k <= g.pl[v].n; ++k) {
    double l1, h1;
    if (!fpInterval(sa, sb, g.pl[v].slope * k + g.pl[v].offset, FP_ROI_MM, l1, h1)) continue;
    l1 = std::max(l1, 0.0);
    h1 = std::min(h1, 1.0);
    if (l1 > h1) continue;
    for (int n = 0; n < nc; ++n) {
      double l2, h2;
      if (!fpInterval(za, zb, n * g.mmPerCell, FP_ROI_MM, l2, h2)) continue;
      if (std::max(l1, l2) <= std::min(h1, h2)) maskV[(size_t)(k - 1) * nc + n] = 1;
    }
  }
}

// Adds the ROI of the 3D segment a -> b (projected on each view) to mask (sized here if it is not yet)
void fpRoiAdd(const double a[3], const double b[3], const FpGeom &g, std::vector<char> mask[3])
{
  for (int v = 0; v < 3; ++v) {
    if (mask[v].size() != (size_t)g.pl[v].n * g.ncell) mask[v].assign((size_t)g.pl[v].n * g.ncell, 0);
    const double sa = FP_PITCH[v][0] * a[0] + FP_PITCH[v][1] * a[1];
    const double sb = FP_PITCH[v][0] * b[0] + FP_PITCH[v][1] * b[1];
    fpRoiAddView(v, sa, a[2], sb, b[2], g, mask[v]);
  }
}

// ROI per view (strip x cell): a point of the projected segment S0 -> E0 within FP_ROI_MM in s
// and in z of (s_k, cell * mmPerCell)
void fpRoi(const double S0[3], const double E0[3], const FpGeom &g, std::vector<char> mask[3])
{
  for (int v = 0; v < 3; ++v) mask[v].assign((size_t)g.pl[v].n * g.ncell, 0);
  fpRoiAdd(S0, E0, g, mask);
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

// A_ref from adcPerMeV (integrated ADC per MeV per plane = A x kernel area): A fixed at it (aBox 0, TODO/201) or the centre of
// the A box (TODO/198 v2); 0 = A free
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

// TODO/198: one Migrad = its initial values and the centres of the +-FP_END_BOX_MM end-point limits. The limits of the
// alpha ends stay at the initial segment of the event (TODO/201) in every fit of the event (H1 on ROI_alpha, H1 again on
// ROI_event, H2), so H2 with Eeq = 0 is exactly an H1 model; the arm end is limited around its seed.
struct FpStart {
  double S[3] = {0, 0, 0}, E[3] = {0, 0, 0};        // alpha: entering / stopping end, initial values
  double boxS[3] = {0, 0, 0}, boxE[3] = {0, 0, 0};  // ... centres of their limits (also set the point count, fpNumPoints)
  double A0 = 1, sigT = FP_SIGMA_INIT, sigL = FP_SIGMA_INIT;
  bool arm = false;                                 // H2: an arm from S (the vertex) to E2
  double E2[3] = {0, 0, 0}, boxE2[3] = {0, 0, 0}, Eeq = 0;
};

// a warm start inside [lo, hi], at least FP_WARM_MARGIN x (hi - lo) from a limit (Minuit2's sin transform is flat at a limit:
// a start on the limit would stay there)
double fpInside(double x, double lo, double hi)
{
  const double m = FP_WARM_MARGIN * (hi - lo);
  return std::min(std::max(x, lo + m), hi - m);
}

// parameters of a fit: 8 with A fixed (aRef > 0, aBox 0), 9 with A free (aRef 0) or in its box (aRef > 0, aBox > 0); + 4 for the arm
int fpNumPar(double aRef, double aBox, bool arm) { return (aRef > 0 && !(aBox > 0) ? 8 : 9) + (arm ? 4 : 0); }

// One Migrad of an alpha (8 parameters with A fixed, 9 with A free or in its box) or of an alpha + arm (+ xA2 yA2 zA2 Eeq = 12 / 13).
// Minuit indices: xS yS zS xE yE zE, [A], sigmaT, sigmaL, [xA2 yA2 zA2 Eeq]. Point counts: alpha = fpNumPoints(|boxE - boxS|),
// arm = fpNumPoints(|boxE2 - boxS|), fixed during the fit. A: aRef > 0 and aBox > 0 = limited to aRef x [1 - aBox, 1 + aBox]
// (TODO/198 v2; start st.A0 moved inside, a fit ending within FP_A_AT_LIMIT_REL x aRef of an edge sets atLimit bit 10), aRef > 0
// and aBox 0 = fixed at aRef (TODO/201), aRef 0 = free (>= 0, start st.A0). H2: the minimised chi2 includes the arm direction
// penalty fpArmPenalty(fpArmCos(S, E, E2), nBins) (TODO/198 v2); r.chi2 = with it, r.pen / r.cosEff = its value / the cos.
FpFit fpFitProngs(const FpData &d, const FpGeom &g, const FpKernel &k, double rho, const std::vector<char> mask[3],
                  const std::vector<char> excl[3], const FpStart &st, double frac, double aRef, double aBox)
{
  FpFit r;
  r.hasArm = st.arm;
  double LA = 0, LB = 0;
  for (int j = 0; j < 3; ++j) {
    LA += (st.boxE[j] - st.boxS[j]) * (st.boxE[j] - st.boxS[j]);
    LB += (st.boxE2[j] - st.boxS[j]) * (st.boxE2[j] - st.boxS[j]);
  }
  r.nPts[0] = fpNumPoints(std::sqrt(LA));
  r.nPts[1] = st.arm ? fpNumPoints(std::sqrt(LB)) : 0;
  const bool fixA = aRef > 0 && !(aBox > 0), boxA = aRef > 0 && aBox > 0;
  const int o = fixA ? 0 : 1;  // Minuit index shift: with A fixed it is not a variable
  const int iT = 6 + o, iL = 7 + o, iB = 8 + o, nPar = fpNumPar(aRef, aBox, st.arm);
  FpImage model, work;
  int ncalls = 0;
  double cosLast = 0, penLast = 0;  // H2: the arm's cosEff and penalty of the last call
  FpProng pr[2];
  pr[0].nPts = r.nPts[0];
  pr[1].arm = true;
  pr[1].nPts = std::max(1, r.nPts[1]);
  auto chi2Of = [&](const double *p) {
    ++ncalls;
    for (int j = 0; j < 3; ++j) {
      pr[0].S[j] = pr[1].S[j] = p[j];
      pr[0].E[j] = p[3 + j];
      if (st.arm) pr[1].E[j] = p[iB + j];
    }
    if (st.arm) pr[1].Eeq = p[iB + 3];
    fpRenderProngs(pr, st.arm ? 2 : 1, fixA ? aRef : p[6], p[iT], p[iL], rho, g, k, d.ceil, model, work);
    int nb;
    const double c2 = fpChi2(d.img, model, mask, excl, d.noise, nb, frac);
    if (!st.arm) return c2;
    cosLast = fpArmCos(p, p + 3, p + iB);
    penLast = fpArmPenalty(cosLast, nb);
    return c2 + penLast;
  };
  std::unique_ptr<ROOT::Math::Minimizer> min(ROOT::Math::Factory::CreateMinimizer("Minuit2", "Migrad"));
  if (!min) {
    std::cout << "ERROR: cannot create the Minuit2 minimizer" << std::endl;
    r.status = -2;
    return r;
  }
  ROOT::Math::Functor fcn(chi2Of, nPar);
  min->SetFunction(fcn);
  min->SetMaxFunctionCalls(FP_MAX_CALLS);
  min->SetTolerance(FP_TOLERANCE);
  min->SetErrorDef(1.0);
  min->SetPrintLevel(ROOT::Math::MinimizerOptions::DefaultPrintLevel());  // 0 unless set (debug)
  const char *nm[9] = {"xS", "yS", "zS", "xE", "yE", "zE", "xA2", "yA2", "zA2"};
  double lo[9], hi[9];
  for (int j = 0; j < 6; ++j) {  // Minuit2 wants the variables in index order
    const double c = j < 3 ? st.boxS[j] : st.boxE[j - 3], x0 = j < 3 ? st.S[j] : st.E[j - 3];
    lo[j] = c - FP_END_BOX_MM;
    hi[j] = c + FP_END_BOX_MM;
    min->SetLimitedVariable(j, nm[j], fpInside(x0, lo[j], hi[j]), 0.5, lo[j], hi[j]);
  }
  const double aLo = aRef * (1 - aBox), aHi = aRef * (1 + aBox);
  if (boxA)
    min->SetLimitedVariable(6, "A", fpInside(st.A0, aLo, aHi), 0.05 * aBox * aRef, aLo, aHi);  // step: as 0.5 mm in the +-10 mm box
  else if (!fixA)
    min->SetLowerLimitedVariable(6, "A", st.A0, 0.1 * st.A0, 0.0);
  min->SetLimitedVariable(iT, "sigmaT", fpInside(st.sigT, FP_SIGMA_MIN, FP_SIGMA_MAX), 0.1, FP_SIGMA_MIN, FP_SIGMA_MAX);
  min->SetLimitedVariable(iL, "sigmaL", fpInside(st.sigL, FP_SIGMA_MIN, FP_SIGMA_MAX), 0.1, FP_SIGMA_MIN, FP_SIGMA_MAX);
  if (st.arm) {
    for (int j = 0; j < 3; ++j) {
      lo[6 + j] = st.boxE2[j] - FP_END_BOX_MM;
      hi[6 + j] = st.boxE2[j] + FP_END_BOX_MM;
      min->SetLimitedVariable(iB + j, nm[6 + j], fpInside(st.E2[j], lo[6 + j], hi[6 + j]), 0.5, lo[6 + j], hi[6 + j]);
    }
    const double e0 = std::max(st.Eeq, 1e-3);  // a lower-limited variable started at its limit (0) would stay there
    min->SetLowerLimitedVariable(iB + 3, "Eeq", e0, std::max(0.1 * e0, 0.01), 0.0);
  }
  min->Minimize();
  r.status = min->Status();
  const double *x = min->X();
  for (int j = 0; j < 6; ++j) r.par[j] = x[j];
  r.par[6] = fixA ? aRef : x[6];
  if (boxA && (std::fabs(x[6] - aLo) < FP_A_AT_LIMIT_REL * aRef || std::fabs(x[6] - aHi) < FP_A_AT_LIMIT_REL * aRef)) r.atLimit |= 1 << 10;
  r.par[7] = x[iT];
  r.par[8] = x[iL];
  if (st.arm)
    for (int j = 0; j < 4; ++j) r.arm[j] = x[iB + j];
  for (int j = 0; j < (st.arm ? 9 : 6); ++j) {
    const double xj = j < 6 ? x[j] : x[iB + j - 6];
    if (std::fabs(xj - lo[j]) < FP_AT_LIMIT_MM || std::fabs(xj - hi[j]) < FP_AT_LIMIT_MM) r.atLimit |= 1 << j;
  }
  if (st.arm) {
    double L2 = 0;
    for (int j = 0; j < 3; ++j) L2 += (r.arm[j] - r.par[j]) * (r.arm[j] - r.par[j]);
    if (std::sqrt(L2) > FP_ARM_MAX_MM) r.atLimit |= 1 << 9;
  }
  r.chi2 = chi2Of(x);  // H2: with the penalty
  r.cosEff = st.arm ? cosLast : 0;
  r.pen = st.arm ? penLast : 0;
  fpChi2(d.img, model, mask, excl, d.noise, r.nBins, frac);
  r.ncalls = ncalls;
  return r;
}

// one Migrad of an alpha from start0 (entering end) / stop0 (stopping end); the limits are centred on them (TODO/201)
FpFit fpFitOne(const FpData &d, const FpGeom &g, const FpKernel &k, double rho, const std::vector<char> mask[3],
               const std::vector<char> excl[3], const double start0[3], const double stop0[3], double A0, double frac,
               double aRef, double aBox)
{
  FpStart st;
  for (int j = 0; j < 3; ++j) {
    st.S[j] = st.boxS[j] = start0[j];
    st.E[j] = st.boxE[j] = stop0[j];
  }
  st.A0 = A0;
  return fpFitProngs(d, g, k, rho, mask, excl, st, frac, aRef, aBox);
}

// strips left out of the chi2 (excl[v][k-1] = 1): no channel, and (excludeSat) saturated. Returns the saturated strips with
// a pixel in mask (nSatStrips).
int fpExclude(const FpData &d, const FpGeom &g, const std::vector<char> mask[3], bool excludeSat, std::vector<char> excl[3])
{
  int nSat = 0;
  for (int v = 0; v < 3; ++v) {
    excl[v].assign(g.pl[v].n, 0);
    for (int kk = 1; kk <= g.pl[v].n; ++kk) {
      bool inRoi = false;
      for (int t = 0; t < g.ncell && !inRoi; ++t) inRoi = mask[v][(size_t)(kk - 1) * g.ncell + t];
      if (!d.hasCh[v].empty() && !d.hasCh[v][kk - 1]) excl[v][kk - 1] = 1;
      if (excludeSat && !d.sat[v].empty() && d.sat[v][kk - 1]) {
        excl[v][kk - 1] = 1;
        if (inRoi) ++nSat;
      }
    }
  }
  return nSat;
}

// both orientations; S0 / E0 = the tracks file's Start / End. excludeSat: leave saturated strips out. adcPerMeV > 0: A_ref,
// A fixed at it (aBox 0) or in A_ref x [1 - aBox, 1 + aBox] started at A_ref (TODO/198 v2); 0 = A free (start from the ROI charge).
FpEventFit fpFitEvent(const FpData &d, const FpGeom &g, const FpKernel &k, double rho, const double S0[3], const double E0[3],
                      bool excludeSat, double frac, double adcPerMeV, double aBox)
{
  FpEventFit r;
  std::vector<char> mask[3], excl[3];
  fpRoi(S0, E0, g, mask);
  r.nSat = fpExclude(d, g, mask, excludeSat, excl);
  double L0 = 0;
  for (int j = 0; j < 3; ++j) L0 += (E0[j] - S0[j]) * (E0[j] - S0[j]);
  const double aRef = fpFixedA(adcPerMeV, k);
  const double A0 = aRef > 0 ? aRef : fpInitialA(d, mask, std::sqrt(L0), rho, k);
  r.fit[0] = fpFitOne(d, g, k, rho, mask, excl, S0, E0, A0, frac, aRef, aBox);
  r.fit[1] = fpFitOne(d, g, k, rho, mask, excl, E0, S0, A0, frac, aRef, aBox);
  if (r.fit[0].status >= -1 || r.fit[1].status >= -1) r.dir = r.fit[1].chi2 < r.fit[0].chi2 ? 1 : 0;
  r.ndf = r.fit[0].nBins - fpNumPar(aRef, aBox, false);
  return r;
}

// ---- TODO/198: the arm seed, the event ROI, H1 vs H2 ----------------------------------------
// least-squares (x, y) from the strip coordinates s_v = FP_PITCH[v] . (x, y) of the views with has[v]; false with < 2 views
bool fpSolveXY(const double s[3], const int has[3], double xy[2])
{
  double a11 = 0, a12 = 0, a22 = 0, b1 = 0, b2 = 0;
  int n = 0;
  for (int v = 0; v < 3; ++v) {
    if (!has[v]) continue;
    const double px = FP_PITCH[v][0], py = FP_PITCH[v][1];
    a11 += px * px;
    a12 += px * py;
    a22 += py * py;
    b1 += px * s[v];
    b2 += py * s[v];
    ++n;
  }
  const double det = a11 * a22 - a12 * a12;
  if (n < 2 || std::fabs(det) < 1e-12) return false;
  xy[0] = (a22 * b1 - a12 * b2) / det;
  xy[1] = (a11 * b2 - a12 * b1) / det;
  return true;
}

struct FpArmSeed {
  int fallback = 0;   // 0 = from components in >= 2 views; 1 = no component (E2 = V - FP_SEED_FALLBACK_MM x the alpha
                      // direction, Eeq = FP_SEED_FALLBACK_MEV); 2 = one view only (xy = V moved along that view's pitch to s-bar)
  double E2[3] = {0, 0, 0};
  double Eeq = 0;     // [MeV equivalent] = mean component charge over the views with one / (A x kernel area)
  int has[3] = {0, 0, 0};
  int along = 0;      // TODO/198 v2: bit v = a component of view v was left out (its centroid within FP_SEED_ALONG_MM of the alpha)
  double q[3] = {0, 0, 0}, sBar[3] = {0, 0, 0}, zBar[3] = {0, 0, 0};  // per view: charge [ADC], centroid [mm]
  double sFar[3] = {0, 0, 0}, zFar[3] = {0, 0, 0}, dFar[3] = {0, 0, 0}; // ... the pixel farthest from the vertex in (s, z)
};

// The arm seed from the H1 residual data - model (pure). Per view, the ROI pixels (mask, strips not in excl) with residual
// > FP_SEED_NSIG x noise[v] are grouped into 8-neighbour connected components (the rule of uvwdrawSignalRange in drawUVW.C,
// TODO/199 - copied, not included); a component whose charge centroid is within FP_SEED_ALONG_MM of the alpha's projection
// (the (s, z) segment V -> Estop) is not a candidate (TODO/198 v2, Fable verdict D: the arm does not go along the alpha; bit v of
// `along`), a view with no candidate left has none; the candidate with the largest charge (sum of the residual) gives the charge centroid
// (s-bar, z-bar) and the pixel farthest from the vertex V in (s, z). >= 2 views: xy = least squares of the s-bars (fpSolveXY),
// z = mean of the z-bars. Pixel position: s = s_k, z = cell x mmPerCell (as fpRoi). Estop = the alpha's stopping end (the
// fallback direction). adcPerMeVEff = A x kernel area (the H1 fit's). No candidate in any view = fallback 1 with Eeq =
// FP_SEED_FALLBACK_MEV, or (TODO/198 v2, I1-a: e.g. a blob on the vertex, whose centroid sits on the projection's end) the mean
// over the views with a left-out component of the largest left-out charge / adcPerMeVEff.
FpArmSeed fpSeedArm(const FpImage &data, const FpImage &model, const std::vector<char> mask[3], const std::vector<char> excl[3],
                    const double noise[3], const FpGeom &g, const double V[3], const double Estop[3], double adcPerMeVEff)
{
  FpArmSeed s;
  const int nc = g.ncell;
  double exclQ[3] = {0, 0, 0};  // per view: the largest charge of a component left out (along the alpha)
  for (int v = 0; v < 3; ++v) {
    const double thr = FP_SEED_NSIG * noise[v];
    const double sV = FP_PITCH[v][0] * V[0] + FP_PITCH[v][1] * V[1];
    std::vector<int> cells;  // (k - 1) * nc + t of the seed pixels, ascending
    for (int k = 1; k <= data.n[v]; ++k) {
      if (!excl[v].empty() && excl[v][k - 1]) continue;
      for (int t = 0; t < nc; ++t) {
        const size_t i = (size_t)(k - 1) * nc + t;
        if (mask[v][i] && data.v[v][i] - model.v[v][i] > thr) cells.push_back((int)i);
      }
    }
    if (cells.empty()) continue;
    std::vector<int> comp(cells.size(), -1), stack;  // component id per pixel; neighbours by binary search
    std::vector<double> charge, chS, chZ;            // per component: charge, charge x s, charge x z
    for (size_t i = 0; i < cells.size(); ++i) {
      if (comp[i] >= 0) continue;
      const int id = (int)charge.size();
      charge.push_back(0.0);
      chS.push_back(0.0);
      chZ.push_back(0.0);
      comp[i] = id;
      stack.push_back((int)i);
      while (!stack.empty()) {
        const int a = stack.back();
        stack.pop_back();
        const int ka = cells[a] / nc, ta = cells[a] % nc;  // ka = k - 1
        const double ra = data.v[v][cells[a]] - model.v[v][cells[a]];
        charge[id] += ra;
        chS[id] += ra * (g.pl[v].slope * (ka + 1) + g.pl[v].offset);
        chZ[id] += ra * ta * g.mmPerCell;
        for (int dk = -1; dk <= 1; ++dk)
          for (int dt = -1; dt <= 1; ++dt) {
            const int kb = ka + dk, tb = ta + dt;
            if ((dk == 0 && dt == 0) || kb < 0 || kb >= data.n[v] || tb < 0 || tb >= nc) continue;
            const auto it = std::lower_bound(cells.begin(), cells.end(), kb * nc + tb);
            if (it == cells.end() || *it != kb * nc + tb) continue;
            const int b = (int)(it - cells.begin());
            if (comp[b] >= 0) continue;
            comp[b] = id;
            stack.push_back(b);
          }
      }
    }
    const double sE = FP_PITCH[v][0] * Estop[0] + FP_PITCH[v][1] * Estop[1];
    int best = -1;
    for (size_t c = 0; c < charge.size(); ++c) {
      if (fpDistToSegment2(chS[c] / charge[c], chZ[c] / charge[c], sV, V[2], sE, Estop[2]) <= FP_SEED_ALONG_MM) {
        s.along |= 1 << v;  // along the alpha: not a candidate
        exclQ[v] = std::max(exclQ[v], charge[c]);
        continue;
      }
      if (best < 0 || charge[c] > charge[best]) best = (int)c;
    }
    if (best < 0) continue;  // only components along the alpha: has[v] = 0
    double q = 0, qs = 0, qz = 0, dFar = -1;
    for (size_t i = 0; i < cells.size(); ++i) {
      if (comp[i] != best) continue;
      const int k = cells[i] / nc + 1, t = cells[i] % nc;
      const double r = data.v[v][cells[i]] - model.v[v][cells[i]];
      const double sk = g.pl[v].slope * k + g.pl[v].offset, z = t * g.mmPerCell;
      q += r;
      qs += r * sk;
      qz += r * z;
      const double dd = std::hypot(sk - sV, z - V[2]);
      if (dd > dFar) {
        dFar = dd;
        s.sFar[v] = sk;
        s.zFar[v] = z;
      }
    }
    s.has[v] = 1;
    s.q[v] = q;
    s.sBar[v] = qs / q;
    s.zBar[v] = qz / q;
    s.dFar[v] = dFar;
  }
  const int nHas = s.has[0] + s.has[1] + s.has[2];
  if (nHas == 0) {
    double u[3], L2 = 0;
    for (int j = 0; j < 3; ++j) L2 += (Estop[j] - V[j]) * (Estop[j] - V[j]);
    const double L = std::sqrt(L2);
    for (int j = 0; j < 3; ++j) {
      u[j] = L > 0 ? (Estop[j] - V[j]) / L : 0.0;
      s.E2[j] = V[j] - FP_SEED_FALLBACK_MM * u[j];
    }
    s.Eeq = FP_SEED_FALLBACK_MEV;
    if (s.along && adcPerMeVEff > 0) {  // TODO/198 v2 (I1-a): components were left out -> their charge scale
      double qm = 0;
      int n = 0;
      for (int v = 0; v < 3; ++v)
        if (exclQ[v] > 0) qm += exclQ[v], ++n;
      s.Eeq = qm / n / adcPerMeVEff;
    }
    s.fallback = 1;
    return s;
  }
  double qm = 0, zm = 0;
  for (int v = 0; v < 3; ++v)
    if (s.has[v]) {
      qm += s.q[v] / nHas;
      zm += s.zBar[v] / nHas;
    }
  s.Eeq = adcPerMeVEff > 0 ? qm / adcPerMeVEff : FP_SEED_FALLBACK_MEV;
  s.E2[2] = zm;
  double xy[2];
  if (fpSolveXY(s.sBar, s.has, xy)) {
    s.E2[0] = xy[0];
    s.E2[1] = xy[1];
  } else {  // one view: the vertex moved along its pitch vector (|p| = 1) to s-bar (the least-squares point nearest to V)
    const int v = s.has[0] ? 0 : (s.has[1] ? 1 : 2);
    const double ds = s.sBar[v] - (FP_PITCH[v][0] * V[0] + FP_PITCH[v][1] * V[1]);
    s.E2[0] = V[0] + ds * FP_PITCH[v][0];
    s.E2[1] = V[1] + ds * FP_PITCH[v][1];
    s.fallback = 2;
  }
  return s;
}

// TODO/198 v2 (I2-a): the arm seed of the H2 of one orientation (vertex V, alpha stop E): the shared seed E2, or, when the arm
// V -> E2 would go along that orientation's alpha (fpArmCos > FP_ARM_COS_MAX: the penalty, and the +-10 mm box cannot turn it
// below), the fallback position V - FP_SEED_FALLBACK_MM x u_alpha (cos -1). out = the seed; returns 1 when the fallback is taken.
int fpOrientSeed(const double V[3], const double E[3], const double E2[3], double out[3])
{
  for (int j = 0; j < 3; ++j) out[j] = E2[j];
  if (!(fpArmCos(V, E, E2) > FP_ARM_COS_MAX)) return 0;
  double L2 = 0;
  for (int j = 0; j < 3; ++j) L2 += (E[j] - V[j]) * (E[j] - V[j]);
  const double L = std::sqrt(L2);
  for (int j = 0; j < 3; ++j) out[j] = V[j] - FP_SEED_FALLBACK_MM * (E[j] - V[j]) / L;  // L > 0: fpArmCos is 0 for L = 0
  return 1;
}

// bit v = a saturated strip of plane v within FP_ARM_SAT_MM in s of the arm's projection V -> E2 (its charge is then a lower
// bound: saturated strips are out of the chi2)
int fpArmSat(const FpData &d, const FpGeom &g, const double V[3], const double E2[3])
{
  int bits = 0;
  for (int v = 0; v < 3; ++v) {
    const double sa = FP_PITCH[v][0] * V[0] + FP_PITCH[v][1] * V[1], sb = FP_PITCH[v][0] * E2[0] + FP_PITCH[v][1] * E2[1];
    const double lo = std::min(sa, sb), hi = std::max(sa, sb);
    for (int k = 1; k <= g.pl[v].n && !(bits & (1 << v)); ++k) {
      if (d.sat[v].empty() || !d.sat[v][k - 1]) continue;
      const double sk = g.pl[v].slope * k + g.pl[v].offset;
      const double dist = sk < lo ? lo - sk : (sk > hi ? sk - hi : 0.0);
      if (dist <= FP_ARM_SAT_MM) bits |= 1 << v;
    }
  }
  return bits;
}

// Residual along the arm of an H2 fit (TODO/198 acceptance 7): the arm is cut into thirds, third j drawn alone (Eeq / 3, no
// ceiling) = M_j, and each ROI pixel's residual is shared by M_j / model: R_j = sum (M_j / model)(data - model) / sum M_j
// over the ROI pixels (strips not excluded) with model > 0. out[0] = the vertex-side third, out[1] = the arm-end third.
void fpArmThirds(const FpData &d, const FpImage &model, const std::vector<char> mask[3], const std::vector<char> excl[3],
                 const FpGeom &g, const FpKernel &k, double rho, const FpFit &h, double out[2])
{
  out[0] = out[1] = 0;
  double L2 = 0;
  for (int j = 0; j < 3; ++j) L2 += (h.arm[j] - h.par[j]) * (h.arm[j] - h.par[j]);
  if (!h.hasArm || !(h.arm[3] > 0)) return;
  for (int third = 0; third < 2; ++third) {
    const double u0 = third == 0 ? 0.0 : 2.0 / 3, u1 = u0 + 1.0 / 3;
    FpProng p;
    p.arm = true;
    for (int j = 0; j < 3; ++j) {
      p.S[j] = h.par[j] + u0 * (h.arm[j] - h.par[j]);
      p.E[j] = h.par[j] + u1 * (h.arm[j] - h.par[j]);
    }
    p.Eeq = h.arm[3] / 3;
    p.nPts = fpNumPoints(std::sqrt(L2) / 3);
    FpImage m, w;
    fpRenderProngs(&p, 1, h.par[6], h.par[7], h.par[8], rho, g, k, nullptr, m, w);
    double num = 0, den = 0;
    for (int v = 0; v < 3; ++v)
      for (int kk = 1; kk <= g.pl[v].n; ++kk) {
        if (!excl[v].empty() && excl[v][kk - 1]) continue;
        for (int t = 0; t < g.ncell; ++t) {
          const size_t i = (size_t)(kk - 1) * g.ncell + t;
          if (!mask[v][i] || !(model.v[v][i] > 0) || m.v[v][i] == 0) continue;
          num += m.v[v][i] / model.v[v][i] * (d.img.v[v][i] - model.v[v][i]);
          den += m.v[v][i];
        }
      }
    out[third] = den > 0 ? num / den : 0;
  }
}

struct FpEventResult {  // one event: H1 on ROI_alpha (TODO/201) and H1 / H2 on ROI_event (TODO/198)
  FpEventFit h1a;       // H1 on ROI_alpha = the TODO/201 fit (the tree's original branches)
  FpEventFit h1;        // H1 on ROI_event, both orientations, warm from h1a
  FpFit h2[2];          // H2 on ROI_event, [d] = vertex at the entering end of orientation d, warm from h1.fit[d]
  int dirH2 = -1;
  FpArmSeed seed;
  int cmSkipped = 0, cmSkippedEvent = 0;  // bit v: plane v not common-mode subtracted (ROI_alpha / ROI_event)
  int nBinsEvent = 0, ndfH1 = 0, ndfH2 = 0, armSat = 0;
  double chi2H1 = 0, chi2H2 = 0, dchi2 = 0;  // chi2H1 / H2 = the better orientation (H2 with its penalty), dchi2 = chi2H1 - chi2H2
                                             // (0 when h2Worse)
  int h1Broken = 0, h2Worse = 0, armAlong = 0;  // TODO/198 v2: fpH1Broken of the H1 on ROI_event; chi2H2 > chi2H1; armCos > 0.55
  int orientFallback = 0;                       // bit d: H2 of orientation d started from the fallback seed (fpOrientSeed, I2-a)
  double armCos = 0, armPen = 0;                // the better H2's cosEff and penalty
  double aRelH1 = 0, aRelH2 = 0;                // A / A_ref of the H1 / H2 on ROI_event (0 when A is free: no A_ref)
  double armResid[2] = {0, 0};               // fpArmThirds of the better H2
  double timeH1a = 0;                        // [s] the two ROI_alpha fits (the time_s branch)
  double noiseA[3] = {0, 0, 0};              // sigma_v after the ROI_alpha common mode (the noise branch)
  FpData data;                               // the grid with the ROI_event common mode (H1 / H2 were fitted on it)
  std::vector<char> roi[3], excl[3];         // ROI_event and its left-out strips
};

// TODO/198 for one event; raw = the grid before any common mode, S0 / E0 = the initial segment (tracks Start / End, clipped).
//  (1) ROI_alpha = fpRoi(S0, E0), common mode on it, H1 both orientations (TODO/197 / 201) -> h1a;
//  (2) the arm seed from the residual of the better orientation (vertex = its entering end);
//      ROI_event = ROI_alpha + per view the (s, z) segment vertex -> the seed component's farthest pixel (a view without a
//      component: vertex -> the seed arm end); the common mode again from raw, now from the strips outside ROI_event;
//  (3) H1 on ROI_event, both orientations, warm from (1);
//  (4) H2 with the vertex at either end (= the entering end of orientation d, warm from (3)), the same arm seed - except
//      (TODO/198 v2, I2-a) an orientation for which it lies along the alpha (fpOrientSeed): there V_d - 3 mm x u_d.
// Both hypotheses: same data, noise, ROI, strips (nBins equal). false (why) = not fitted. A (TODO/198 v2): adcPerMeV > 0 and
// aBox > 0 = A in A_ref x [1 - aBox, 1 + aBox] in every fit ((1) starts at A_ref, (3) / (4) warm from the previous A); aBox 0 =
// A fixed at A_ref; adcPerMeV 0 = A free.
bool fpFitEventAll(const FpData &raw, const FpGeom &g, const FpKernel &k, double rho, const double S0[3], const double E0[3],
                   bool commonMode, double frac, double adcPerMeV, double aBox, FpEventResult &r, std::string &why)
{
  std::vector<char> roiA[3];
  fpRoi(S0, E0, g, roiA);
  FpData d1 = raw;
  r.cmSkipped = commonMode ? fpSubtractCommonMode(d1, roiA) : 0;
  if (!(d1.noise[0] > 0 && d1.noise[1] > 0 && d1.noise[2] > 0)) {
    why = "a plane has noise 0 (empty?)";
    return false;
  }
  TStopwatch sw;
  r.h1a = fpFitEvent(d1, g, k, rho, S0, E0, true, frac, adcPerMeV, aBox);
  r.timeH1a = sw.RealTime();
  for (int v = 0; v < 3; ++v) r.noiseA[v] = d1.noise[v];
  if (r.h1a.dir < 0) {
    why = "no fit (minimizer failed)";
    return false;
  }
  const double aRef = fpFixedA(adcPerMeV, k), area = fpKernelArea(k);
  const FpFit &b1 = r.h1a.fit[r.h1a.dir];
  {
    FpImage m, w;
    fpRenderFit(b1, rho, g, k, d1.ceil, m, w);
    std::vector<char> exA[3];
    fpExclude(d1, g, roiA, true, exA);
    r.seed = fpSeedArm(d1.img, m, roiA, exA, d1.noise, g, b1.par, b1.par + 3, b1.par[6] * area);
  }
  for (int v = 0; v < 3; ++v) {
    r.roi[v] = roiA[v];
    const double sV = FP_PITCH[v][0] * b1.par[0] + FP_PITCH[v][1] * b1.par[1];
    const double sB = r.seed.has[v] ? r.seed.sFar[v] : FP_PITCH[v][0] * r.seed.E2[0] + FP_PITCH[v][1] * r.seed.E2[1];
    const double zB = r.seed.has[v] ? r.seed.zFar[v] : r.seed.E2[2];
    fpRoiAddView(v, sV, b1.par[2], sB, zB, g, r.roi[v]);
  }
  r.data = raw;
  r.cmSkippedEvent = commonMode ? fpSubtractCommonMode(r.data, r.roi) : 0;
  if (!(r.data.noise[0] > 0 && r.data.noise[1] > 0 && r.data.noise[2] > 0)) {
    why = "a plane has noise 0 after the ROI_event common mode";
    return false;
  }
  r.h1.nSat = fpExclude(r.data, g, r.roi, true, r.excl);
  const double *box[2][2] = {{S0, E0}, {E0, S0}};  // orientation d: entering / stopping end of the initial segment
  for (int pass = 0; pass < 2; ++pass)              // 0 = H1 again (warm from h1a), 1 = H2 (warm from that)
    for (int d = 0; d < 2; ++d) {
      const FpFit &w = pass == 0 ? r.h1a.fit[d] : r.h1.fit[d];
      FpStart st;
      for (int j = 0; j < 3; ++j) {
        st.S[j] = w.par[j];
        st.E[j] = w.par[3 + j];
        st.boxS[j] = box[d][0][j];
        st.boxE[j] = box[d][1][j];
        st.E2[j] = st.boxE2[j] = r.seed.E2[j];
      }
      if (pass == 1 && fpOrientSeed(st.S, st.E, r.seed.E2, st.E2)) {  // TODO/198 v2 (I2-a); Eeq stays the seed's (its charge)
        for (int j = 0; j < 3; ++j) st.boxE2[j] = st.E2[j];
        r.orientFallback |= 1 << d;
      }
      st.A0 = w.par[6] > 0 ? w.par[6] : 1.0;
      st.sigT = w.par[7];
      st.sigL = w.par[8];
      st.arm = pass == 1;
      st.Eeq = r.seed.Eeq;
      FpFit f = fpFitProngs(r.data, g, k, rho, r.roi, r.excl, st, frac, aRef, aBox);
      if (pass == 0)
        r.h1.fit[d] = f;
      else
        r.h2[d] = f;
    }
  if (r.h1.fit[0].status < -1 && r.h1.fit[1].status < -1) {
    why = "no H1 fit on ROI_event (minimizer failed)";
    return false;
  }
  if (r.h2[0].status < -1 && r.h2[1].status < -1) {
    why = "no H2 fit (minimizer failed)";
    return false;
  }
  r.h1.dir = r.h1.fit[1].chi2 < r.h1.fit[0].chi2 ? 1 : 0;
  r.dirH2 = r.h2[1].chi2 < r.h2[0].chi2 ? 1 : 0;
  r.nBinsEvent = r.h1.fit[0].nBins;
  r.h1.ndf = r.ndfH1 = r.nBinsEvent - fpNumPar(aRef, aBox, false);
  r.ndfH2 = r.nBinsEvent - fpNumPar(aRef, aBox, true);
  r.chi2H1 = r.h1.fit[r.h1.dir].chi2;
  r.chi2H2 = r.h2[r.dirH2].chi2;
  r.dchi2 = fpDchi2(r.chi2H1, r.chi2H2, r.h2Worse);
  const FpFit &h = r.h2[r.dirH2], &b = r.h1.fit[r.h1.dir];
  r.h1Broken = fpH1Broken(b.par[7], b.par[8]);
  r.armCos = h.cosEff;
  r.armPen = h.pen;
  r.armAlong = h.cosEff > FP_ARM_ALONG_COS ? 1 : 0;
  r.aRelH1 = aRef > 0 ? b.par[6] / aRef : 0;
  r.aRelH2 = aRef > 0 ? h.par[6] / aRef : 0;
  r.armSat = fpArmSat(r.data, g, h.par, h.arm);
  FpImage m, w;
  fpRenderFit(h, rho, g, k, r.data.ceil, m, w);
  fpArmThirds(r.data, m, r.roi, r.excl, g, k, rho, h, r.armResid);
  return true;
}

// ---- image -----------------------------------------------------------------------------
// mm axes as drawUVW.C: x = z [mm] (cell j = [j - 1/2, j + 1/2) x mmPerCell), y = s [mm] (strip k = s_k +- pitch/2).
// The fitted segments are drawn (thin lines) on the model and residual rows only, never on the data row (TODO/192 3-2):
// H1 (h2 = false): the alpha, entering end = open circle, stopping end = filled circle (TODO/201);
// H2 (h2 = true, TODO/198): the alpha and the arm, vertex = magenta open circle, alpha stop = filled circle, arm end = open
// square. header = the start of every pad title (event, chi2 H1 / H2, dchi2, T). Returns the segments drawn per pad (1 / 2).
int fpDrawEvent(const FpData &d, const FpImage &model, const std::vector<char> mask[3], const FpGeom &g, const std::string &header,
                const FpFit &b, bool h2, const std::string &path)
{
  const char *vn[3] = {"U", "V", "W"}, *rowName[3] = {"data", "model", "data - model"};
  const int nSeg = h2 ? 2 : 1;
  TCanvas c("fpCanvas", "", 1200, 900);
  c.Divide(3, 3);
  std::vector<TH2D *> hs;
  std::vector<TObject *> marks;
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
      auto *h = new TH2D(Form("fp_%d_%d", v, row), Form("%s  %s %s;z [mm];s [mm]", header.c_str(), vn[v], rowName[row]), t1 - t0 + 1,
                         (t0 - 0.5) * g.mmPerCell, (t1 + 0.5) * g.mmPerCell, k1 - k0 + 1, sLo, sHi);
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
      if (row == 0) continue;
      const double sS = FP_PITCH[v][0] * b.par[0] + FP_PITCH[v][1] * b.par[1];
      const double sE = FP_PITCH[v][0] * b.par[3] + FP_PITCH[v][1] * b.par[4];
      const double sA2 = FP_PITCH[v][0] * b.arm[0] + FP_PITCH[v][1] * b.arm[1];
      std::vector<TObject *> objs;
      auto *ln = new TLine(b.par[2], sS, b.par[5], sE);
      ln->SetLineWidth(1);
      objs.push_back(ln);
      auto *m0 = new TMarker(b.par[2], sS, 24);
      auto *m1 = new TMarker(b.par[5], sE, 20);
      objs.push_back(m1);
      if (h2) {
        auto *la = new TLine(b.par[2], sS, b.arm[2], sA2);
        la->SetLineWidth(1);
        la->SetLineColor(kMagenta);
        objs.push_back(la);
        auto *m2 = new TMarker(b.arm[2], sA2, 25);
        m2->SetMarkerSize(0.8);
        objs.push_back(m2);
        m0->SetMarkerColor(kMagenta);
      }
      objs.push_back(m0);
      for (TObject *o : objs) {
        if (auto *m = dynamic_cast<TMarker *>(o)) m->SetMarkerSize(0.8);
        o->Draw();
        marks.push_back(o);
      }
    }
  }
  c.SaveAs(path.c_str());
  for (auto *h : hs) delete h;
  for (auto *m : marks) delete m;
  return nSeg;
}

// ---- entry point -----------------------------------------------------------------------
void fitProngs(const char *rawFile, const char *tracksFile, double pressureMbar, double samplingMHz, double driftHvVolt,
               const char *kernelJson, const char *outFile, const char *entryList = "", const char *pngDir = "",
               double adcPerMeV = 0, double fracErr = FP_FRAC_ERR, bool commonMode = true, double dchi2Threshold = 0,
               double aBox = FP_A_BOX)
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
  if (!(aBox >= 0 && aBox < 1)) {
    std::cout << "ERROR: aBox = " << aBox << " must be in [0, 1); nothing written" << std::endl;
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
  TTree tree("fit", "forward-model fit, H1 = one alpha (TODO/197, 201) vs H2 = alpha + arm (TODO/198)");
  Long64_t entry = 0;
  Int_t dir = 0, ndf = 0, status[2] = {0, 0}, ncalls[2] = {0, 0}, nSat = 0, atLimit = 0, cmSkipped = 0, initClipped = 0;
  Double_t sigmaXY = 0, noiseOut[3] = {0, 0, 0};
  Double_t chi2[2] = {0, 0}, xS, yS, zS, xE, yE, zE, A, sigmaT, sigmaL, length, energy, timeS;
  // TODO/198
  Int_t nBinsEvent = 0, ndfH1 = 0, ndfH2 = 0, hyp = 0, armSat = 0, seedFallback = 0, statusH2[2] = {0, 0}, atLimitH2 = 0, dirH2 = 0,
        cmSkippedEvent = 0;
  Double_t chi2H1 = 0, chi2H2 = 0, dchi2 = 0, xV = 0, yV = 0, zV = 0, xA2 = 0, yA2 = 0, zA2 = 0, armLen = 0, armE = 0, armQ = 0;
  Double_t armDirX = 0, armDirY = 0, armDirZ = 0, xEH2 = 0, yEH2 = 0, zEH2 = 0, lengthH2 = 0, sigmaTH2 = 0, sigmaLH2 = 0;
  Double_t armResid[2] = {0, 0}, timeTotal = 0;
  // TODO/198 v2
  Int_t armAlong = 0, seedAlong = 0, h1Broken = 0, h2Worse = 0, orientFallback = 0;
  Double_t armCos = 0, armPen = 0, aRelH1 = 0, aRelH2 = 0;
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
  tree.Branch("chi2H1", &chi2H1);
  tree.Branch("chi2H2", &chi2H2);
  tree.Branch("dchi2", &dchi2);
  tree.Branch("nBinsEvent", &nBinsEvent);
  tree.Branch("ndfH1", &ndfH1);
  tree.Branch("ndfH2", &ndfH2);
  tree.Branch("hyp", &hyp);
  tree.Branch("xV", &xV);
  tree.Branch("yV", &yV);
  tree.Branch("zV", &zV);
  tree.Branch("xA2", &xA2);
  tree.Branch("yA2", &yA2);
  tree.Branch("zA2", &zA2);
  tree.Branch("armLen", &armLen);
  tree.Branch("armE", &armE);
  tree.Branch("armQ", &armQ);
  tree.Branch("armDirX", &armDirX);
  tree.Branch("armDirY", &armDirY);
  tree.Branch("armDirZ", &armDirZ);
  tree.Branch("armSat", &armSat);
  tree.Branch("seedFallback", &seedFallback);
  tree.Branch("statusH2", statusH2, "statusH2[2]/I");
  tree.Branch("atLimitH2", &atLimitH2);
  tree.Branch("dirH2", &dirH2);
  tree.Branch("xEH2", &xEH2);
  tree.Branch("yEH2", &yEH2);
  tree.Branch("zEH2", &zEH2);
  tree.Branch("lengthH2", &lengthH2);
  tree.Branch("sigmaTH2", &sigmaTH2);
  tree.Branch("sigmaLH2", &sigmaLH2);
  tree.Branch("cmSkippedEvent", &cmSkippedEvent);
  tree.Branch("armResid", armResid, "armResid[2]/D");
  tree.Branch("timeTotal_s", &timeTotal);
  tree.Branch("armCos", &armCos);
  tree.Branch("armPen", &armPen);
  tree.Branch("armAlong", &armAlong);
  tree.Branch("aRelH1", &aRelH1);
  tree.Branch("aRelH2", &aRelH2);
  tree.Branch("seedAlong", &seedAlong);
  tree.Branch("h1Broken", &h1Broken);
  tree.Branch("h2Worse", &h2Worse);
  tree.Branch("orientFallback", &orientFallback);

  const double kArea = fpKernelArea(kern);
  std::cout << Form("fitProngs: %zu entries, mm/cell %.6f, rho %.4e g/cm3, kernel %zu cells (i0 %d, area/peak %.3f)",
                    entries.size(), geom.mmPerCell, rho, kern.h.size(), kern.i0, kArea)
            << std::endl;
  if (adcPerMeV < 0) {
    std::cout << "ERROR: adcPerMeV < 0; nothing written" << std::endl;
    return;
  }
  const double aRef = fpFixedA(adcPerMeV, kern);
  const std::string aText = !(aRef > 0) ? "free" : (aBox > 0 ? Form("in A_ref x [%g, %g], A_ref = %g", 1 - aBox, 1 + aBox, aRef) : Form("fixed = %g", aRef));
  std::cout << Form("fitProngs: adcPerMeV %g (A %s), aBox %g, fracErr %g, common mode %s, dchi2Threshold %g%s", adcPerMeV, aText.c_str(), aBox,
                    fracErr, commonMode ? "on" : "off", dchi2Threshold, dchi2Threshold > 0 ? "" : " (none: hyp = 0, dchi2 only)")
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
      for (int j = 0; j < 2; ++j) chi2[j] = -1, status[j] = -1, ncalls[j] = 0, statusH2[j] = -1;
      xS = yS = zS = xE = yE = zE = A = sigmaT = sigmaL = sigmaXY = length = 0;
      energy = -1;
      for (int v = 0; v < 3; ++v) noiseOut[v] = 0;
      chi2H1 = chi2H2 = -1;
      dchi2 = 0;
      nBinsEvent = ndfH1 = ndfH2 = hyp = armSat = seedFallback = atLimitH2 = cmSkippedEvent = 0;
      dirH2 = -1;
      xV = yV = zV = xA2 = yA2 = zA2 = armLen = armE = armQ = armDirX = armDirY = armDirZ = 0;
      xEH2 = yEH2 = zEH2 = lengthH2 = sigmaTH2 = sigmaLH2 = armResid[0] = armResid[1] = timeTotal = 0;
      armAlong = seedAlong = h1Broken = h2Worse = orientFallback = 0;
      armCos = armPen = aRelH1 = aRelH2 = 0;
      tree.Fill();
      continue;
    }
    TStopwatch swAll;
    FpEventResult r;
    std::string why;
    const bool fitted = fpFitEventAll(data, geom, kern, rho, S0, E0, commonMode, fracErr, adcPerMeV, aBox, r, why);
    timeTotal = swAll.RealTime();
    if (r.cmSkipped)
      std::cout << "INFO: entry " << e << ": common mode not subtracted in planes (bit mask " << r.cmSkipped
                << "), < 8 strips outside the ROI" << std::endl;
    if (!fitted) {
      std::cout << "WARNING: entry " << e << ": " << why << ", skipped" << std::endl;
      continue;
    }
    if (r.cmSkippedEvent)
      std::cout << "INFO: entry " << e << ": ROI_event common mode not subtracted in planes (bit mask " << r.cmSkippedEvent
                << "), < 8 strips outside ROI_event" << std::endl;
    const FpEventFit &f = r.h1a;
    const FpFit &b = f.fit[f.dir];
    timeS = r.timeH1a;
    cmSkipped = r.cmSkipped;
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
    for (int v = 0; v < 3; ++v) noiseOut[v] = r.noiseA[v];  // sigma_v of the ROI_alpha fit (TODO/201)
    length = std::sqrt((xE - xS) * (xE - xS) + (yE - yS) * (yE - yS) + (zE - zS) * (zE - zS));
    energy = fpEnergyFromRangeMm(length, rho);
    // TODO/198
    const FpFit &h = r.h2[r.dirH2];
    chi2H1 = r.chi2H1, chi2H2 = r.chi2H2, dchi2 = r.dchi2;
    nBinsEvent = r.nBinsEvent, ndfH1 = r.ndfH1, ndfH2 = r.ndfH2;
    h1Broken = r.h1Broken, h2Worse = r.h2Worse;
    hyp = fpHyp(dchi2, dchi2Threshold, h1Broken);
    xV = h.par[0], yV = h.par[1], zV = h.par[2];
    xEH2 = h.par[3], yEH2 = h.par[4], zEH2 = h.par[5];
    xA2 = h.arm[0], yA2 = h.arm[1], zA2 = h.arm[2];
    sigmaTH2 = h.par[7], sigmaLH2 = h.par[8];
    lengthH2 = std::sqrt((xEH2 - xV) * (xEH2 - xV) + (yEH2 - yV) * (yEH2 - yV) + (zEH2 - zV) * (zEH2 - zV));
    armLen = std::sqrt((xA2 - xV) * (xA2 - xV) + (yA2 - yV) * (yA2 - yV) + (zA2 - zV) * (zA2 - zV));
    armDirX = armLen > 0 ? (xA2 - xV) / armLen : 0;
    armDirY = armLen > 0 ? (yA2 - yV) / armLen : 0;
    armDirZ = armLen > 0 ? (zA2 - zV) / armLen : 0;
    armE = h.arm[3];
    armQ = h.par[6] * armE * kArea;
    armSat = r.armSat;
    seedFallback = r.seed.fallback;
    statusH2[0] = r.h2[0].status, statusH2[1] = r.h2[1].status;
    atLimitH2 = h.atLimit;
    dirH2 = r.dirH2;
    cmSkippedEvent = r.cmSkippedEvent;
    armResid[0] = r.armResid[0], armResid[1] = r.armResid[1];
    armCos = r.armCos, armPen = r.armPen, armAlong = r.armAlong, seedAlong = r.seed.along;
    aRelH1 = r.aRelH1, aRelH2 = r.aRelH2, orientFallback = r.orientFallback;
    tree.Fill();
    ++nFit;
    std::cout << Form("entry %lld event %u: dir %d chi2 %.0f / %.0f ndf %d (%.2f) L %.1f mm E %.2f MeV A %.0f (%.0f ADC/MeV per "
                      "view) sT %.2f sL %.2f status %d/%d sat %d atLimit %d cm %d clip %d %.2f s",
                      e, eventId, dir, chi2[0], chi2[1], ndf, ndf > 0 ? b.chi2 / ndf : 0.0, length, energy, A, A * kArea,
                      sigmaT, sigmaL, status[0], status[1], nSat, atLimit, cmSkipped, initClipped, timeS)
              << std::endl;
    std::cout << Form("   H1 %.0f H2 %.0f dchi2 %.1f (bins %d) hyp %d broken %d worse %d | H2 dir %d L %.1f arm %.1f mm E %.3f MeV Q %.0f "
                      "dir (%.2f %.2f %.2f) cos %.3f pen %.1f sat %d fb %d along %d ofb %d status %d/%d atLimit %d aRel %.3f/%.3f cmE %d "
                      "resid %.3f/%.3f | %.2f s",
                      chi2H1, chi2H2, dchi2, nBinsEvent, hyp, h1Broken, h2Worse, dirH2, lengthH2, armLen, armE, armQ, armDirX, armDirY,
                      armDirZ, armCos, armPen, armSat, seedFallback, seedAlong, orientFallback, statusH2[0], statusH2[1], atLimitH2, aRelH1, aRelH2,
                      cmSkippedEvent, armResid[0], armResid[1], timeTotal)
              << std::endl;
    if (!png.empty()) {
      const FpFit &shown = hyp == 1 ? h : r.h1.fit[r.h1.dir];
      FpImage model, work;
      fpRenderFit(shown, rho, geom, kern, r.data.ceil, model, work);
      const std::string header =
          Form("ev %u (%lld) %s: #chi^{2} H1 %.0f H2 %.0f #Delta %.0f T %.0f", eventId, e, hyp == 1 ? "H2" : (hyp < 0 ? "H1 broken" : "H1"), chi2H1,
               chi2H2, dchi2, dchi2Threshold);
      out.cd();
      fpDrawEvent(r.data, model, r.roi, geom, header, shown, hyp == 1, Form("%s/event%07u.png", png.c_str(), eventId));
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
  TParameter<double>("dchi2_threshold", dchi2Threshold, 'f').Write();
  TParameter<double>("a_box", aBox, 'f').Write();
  out.Close();
  std::cout << "wrote " << outFile << " (" << nFit << " fits)" << std::endl;
}
