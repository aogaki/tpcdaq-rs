// =============================================================================
// test_fitProngs.C - synthetic tests for fitProngs.C (TODO/197).
//
//   export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX27.sdk
//   cd macros
//   root -l -b -q -e 'gSystem->SetBuildDir("/tmp/fp_build", true)' 'test_fitProngs.C+'
//        -> exit code = number of failures (0 = green). ACLiC (+) because [6] runs 9 fits
//           (~1 min compiled; interpreted it is much slower). The build dir keeps .so / .pcm
//           out of the repository.
//   Optional: FITPRONGS_REAL_KERNEL=<path to run0107_kernel.json> adds [3b] on the measured
//   kernel (local data, not in the repository); without it [3b] prints SKIP.
//
// The pure functions are called directly; [9] writes a synthetic raw tree, tracks tree,
// channel_map.csv and kernel JSON into a temp directory and chdirs there (the real
// channel_map.csv and macros/data are never read). Hand-computed values:
//   [1] CO2 at 198 mbar, 293.15 K: rho = 44.01 * 19800 / (8.314 * 293.15) * 1e-6
//         = 3.575334e-4 g/cm3; ASTAR 5.0 MeV = 4.353e-3 g/cm2 -> R = 121.75086 mm
//       (= stoppedAlphas.C rangeFromEnergy(5.0) at PRESSURE_MBAR = 198).
//       dE/dR at R(5.0 MeV) from below: log-log segment 4.5 -> 5.0 MeV,
//         b = ln(5/4.5) / ln(4.353/3.722) = 0.6727826, dE/dR = b * 5.0 / 121.75086 = 0.0276295 MeV/mm
//       sum over the 0.1 mm points of dE/dx * h = E within 0.5 % (5.0 and 2.0 MeV); a point's charge
//       is the integral of E(R) over its step (exact sum); dE/dx has no step at 0.1 MeV (R 4.06 mm);
//       the charge maximum of a 60 mm track is at R 13.68-15.08 mm (stopping power maximum).
//   [2] band: Gaussian centre on a strip boundary, sigma 1 mm, pitch 1.5 -> both neighbours
//         0.5 - Phi(-1.5) = 0.4331928; sigma 0.05 -> 0.5 / 0.5. Sum over strips = 1 inside,
//         0.5 when the centre sits on the outer edge of the last strip (the rest is lost).
//   [3] kernel 25 -> 12.5 MHz: synthetic AGET shape (tau 502 ns) with a 0.008 floor: after the
//       conversion the floor is gone (first values 0 within 1e-12), peak = 1 at index 20
//       (25 MHz peak at 40: the pair (40, 41)), FWHM = FWHM_25 / 2 within 1 cell.
//       JSON reader: hand-made file (30 floor points 0.01, then 0.51 1.01 0.51) at 25 MHz
//       -> (x - 0.01) / 1.0 = 0.5 1 0.5 at indices 31 32 33, i0 = 32.
//   [4] charge conservation: kernel {0.5, 1, 0.5} (area / peak = 2), L = R(2.0 MeV) = 35.9966 mm,
//       A = 1000 -> each view sums to A * E * 2 = 4000 (+-0.5 %: the 0.1 mm Riemann sum;
//       A * sum(q) * 2 up to the 5 sigma cuts of the Gaussians, < 1e-5).
//   [5] saturation: ceiling = half the view's largest pixel for every strip -> max = ceiling, pixels
//       below it unchanged.
//   [6] closure: 9 alphas (20 / 40 / 60 mm x dip 0 / 30 / 60 deg) drawn with the model
//       (A 5000, sigma_T 0.8, sigma_L 1.3) on a 250 ADC baseline + Gaussian noise 7 ADC
//       (TRandom3 seed 197), fitted from endpoints moved by ~1.5 mm, sigma = 1 mm:
//       endpoints within 1 mm (3D distance) and the stopping end chosen correctly
//       (odd cases give the track reversed = expected dir 1).
//   [7] chi2 on a 2-strip x 3-cell grid: U (diff 1 0 2 / 0 -3 0, sigma 2) 14/4 = 3.5,
//       V only strip 1 in the mask (1 1 1, sigma 1) = 3, W strip 2 excluded (0.5 0 -0.5,
//       sigma 0.5) = 2 -> chi2 = 8.5 over 12 bins. ROI of a point at (0,0,10), pitch 1.5,
//       41 strips, 0.5 mm cells: 21 strips x 51 cells = 1071 per view; a segment: same set
//       as a brute-force scan of t (10001 steps).
//   [8] data grid (fillUVW rule): baseline 100 (98/102 alternating), cell 200 = 300 -> 200;
//       RMS 2 / 4 / 6 -> plane noise = median 4; ceiling 4095 - 100 = 3995;
//       a raw sample 4090 -> saturated, 4089 -> not.
//   [9] end to end: fitProngs() on 2 synthetic events (entry 0 ok = 1, entry 1 ok = 0).
//  TODO/201 (hardening):
//   [10] sensitive box: 60 mm alpha (40,0,30) -> (100,0,30), A 1000, kernel {0.5,1,0.5} (area 2):
//        the box edge x = 53 cuts at residual range 47 mm -> each view sums to
//        A * (E(60 mm) - E(47 mm)) * 2 (+-0.5 %); a 0.1 mm step centred at x = 52.99 is drawn,
//        53.01 is not (y: 47.99 / 48.01 and -57.99 / -58.01 likewise).
//   [11] end-point box: the [6] closure (start 1.5 mm off) ends with atLimit = 0; a start 12 mm
//        off (both ends moved by 12 mm in x) ends with atLimit != 0 and every end point inside
//        start +- 10 mm.
//   [12] common mode: 41 / 17 / 29 strips x 512 cells, offset c_t = 30 sin(2 pi t / 20) on every
//        strip, track in the ROI strips 1..10 -> after the subtraction the pixels outside the ROI
//        are 0 (1e-9) and the ROI pixels the track alone; view 1 has 7 strips outside the ROI
//        (< 8): untouched, cmSkipped bit 1 set; view 2: 10 saturated strips (+1000) outside the
//        ROI out-number the 9 usable ones - they must not enter the median.
//   [13] error model, one U pixel data 100 / model 90 / noise 2 / f 0.1: variance 4 + 10^2 = 104 ->
//        chi2 = 10^2 / 104; a pixel with data -50 / model -48: max(data, 0) = 0 -> 4 / 4 = 1.
//   [14] fixed A: adcPerMeV 2000 / area 2 = A 1000 -> [4]-type render sums to 4000; a [6] event
//        fitted with adcPerMeV = 5000 * area: A stays 5000, ndf = bins - 8, end points within 1 mm.
//   [15] tree: sigmaXY = sqrt(sigmaT^2 + 0.146), atLimit, cmSkipped, noise[3] (the [9] e2e).
//   [16] e2e png >= 10 kB.
//   [17] fpClipSegment (initial segment cut to x +-53, y -58..48, z 0..512 cells): y = 66 end cut at
//        y = 48 with the direction unchanged (1e-9), a segment inside unchanged (flags 0), both ends,
//        z, and completely outside (flags 4 = skipped).
// =============================================================================
#include "fitProngs.C"
#include "stoppedAlphas.C"
#include "analyzeUVW.C"

#include <TFile.h>
#include <TParameter.h>
#include <TRandom3.h>
#include <TStopwatch.h>
#include <TSystem.h>
#include <TTree.h>

#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <string>
#include <vector>

static int g_failures = 0;
static void check(bool ok, const std::string &name)
{
  std::printf("%s  %s\n", ok ? "PASS" : "FAIL", name.c_str());
  if (!ok) ++g_failures;
}
static void checkNear(double got, double want, double tol, const std::string &name)
{
  char buf[512];
  std::snprintf(buf, sizeof(buf), "%s: got %.9g want %.9g (tol %g)", name.c_str(), got, want, tol);
  check(std::isfinite(got) && std::fabs(got - want) <= tol, buf);
}

// ---- synthetic geometry: pitch 1.5 mm, strip k of plane v at s = 1.5 (k - (n+1)/2) ---------
static FpGeom makeGeom(int nU, int nV, int nW, double mmPerCell, int ncell = FP_NCELL)
{
  FpGeom g;
  const int n[3] = {nU, nV, nW};
  for (int v = 0; v < 3; ++v) {
    g.pl[v].n = n[v];
    g.pl[v].slope = 1.5;
    g.pl[v].pitch = 1.5;
    g.pl[v].offset = -1.5 * (n[v] + 1) / 2.0;
  }
  g.mmPerCell = mmPerCell;
  g.ncell = ncell;
  return g;
}

// (aget, ch) of the synthetic map: signal channels (not 11/22/45/56) in order: U 1..nU, V, W
static void makeChannels(const FpGeom &g, int view[4][68], int strip[4][68])
{
  for (int a = 0; a < 4; ++a)
    for (int c = 0; c < 68; ++c) {
      view[a][c] = -1;
      strip[a][c] = 0;
    }
  int v = 0, k = 1;
  for (int a = 0; a < 4 && v < 3; ++a)
    for (int c = 0; c < 68 && v < 3; ++c) {
      if (c == 11 || c == 22 || c == 45 || c == 56) continue;
      view[a][c] = v;
      strip[a][c] = k;
      if (++k > g.pl[v].n) {
        ++v;
        k = 1;
      }
    }
}

// AGET shaper shape (TPCReco StripResponseCalculator.cpp:886-889): x^3 exp(-3x) sin(x), x = t/tau
static double agetShape(double tNs, double tauNs)
{
  const double x = tNs / tauNs;
  if (x <= 0 || x >= M_PI) return 0;
  return std::exp(-3 * x) * x * x * x * std::sin(x);
}

// 341 samples at 40 ns, the peak at index 40 (as mean_norm of pulser_kernel.C), peak 1 after
// adding a constant floor
static std::vector<double> agetSamples25(double tauNs, double floorFrac)
{
  double tPk = 0, fPk = 0;
  for (double t = 0; t < 4 * tauNs; t += 0.01) {
    const double f = agetShape(t, tauNs);
    if (f > fPk) {
      fPk = f;
      tPk = t;
    }
  }
  std::vector<double> h(341);
  for (int i = 0; i < 341; ++i)
    h[i] = (agetShape(tPk + (i - 40) * 40.0, tauNs) / fPk + floorFrac) / (1 + floorFrac);
  return h;
}

// FWHM [samples] by linear interpolation around the maximum
static double fwhm(const std::vector<double> &h)
{
  int ip = 0;
  for (size_t i = 0; i < h.size(); ++i)
    if (h[i] > h[ip]) ip = (int)i;
  const double half = 0.5 * h[ip];
  int l = ip, r = ip;
  while (l > 0 && h[l] > half) --l;
  while (r < (int)h.size() - 1 && h[r] > half) ++r;
  const double xl = l + (half - h[l]) / (h[l + 1] - h[l]);
  const double xr = r - 1 + (h[r - 1] - half) / (h[r - 1] - h[r]);
  return xr - xl;
}

static int argmax(const std::vector<double> &h)
{
  int ip = 0;
  for (size_t i = 0; i < h.size(); ++i)
    if (h[i] > h[ip]) ip = (int)i;
  return ip;
}

static double imageSum(const FpImage &im, int v)
{
  double s = 0;
  for (double x : im.v[v]) s += x;
  return s;
}

static double imageMax(const FpImage &im, int v)
{
  double m = -1e300;
  for (double x : im.v[v]) m = std::max(m, x);
  return m;
}

static double dist3(const double *a, const double *b)
{
  return std::sqrt((a[0] - b[0]) * (a[0] - b[0]) + (a[1] - b[1]) * (a[1] - b[1]) + (a[2] - b[2]) * (a[2] - b[2]));
}

// synthetic adc: baseline + model (+ Gaussian noise if rng), rounded and clamped to 0..4095
static Short_t g_adc[4][68][512];
static void makeAdc(const FpImage &model, const int view[4][68], const int strip[4][68], double base, TRandom3 *rng,
                    double noise, double *maxAdc)
{
  *maxAdc = 0;
  for (int a = 0; a < 4; ++a)
    for (int c = 0; c < 68; ++c)
      for (int t = 0; t < 512; ++t) {
        double x = base;
        if (view[a][c] >= 0 && !model.v[view[a][c]].empty()) x += model.get(view[a][c], strip[a][c], t);
        if (rng) x += rng->Gaus(0, noise);
        x = std::min(std::max(std::round(x), 0.0), 4095.0);
        g_adc[a][c][t] = (Short_t)x;
        if (view[a][c] >= 0) *maxAdc = std::max(*maxAdc, x);
      }
}

static bool writeText(const std::string &path, const std::string &text)
{
  std::ofstream f(path);
  f << text;
  return (bool)f;
}

int test_fitProngs()
{
  const std::string startDir = gSystem->WorkingDirectory();
  const double rho198 = 3.575334175e-4;  // g/cm3, hand value above

  // ---- [1] Bragg curve -------------------------------------------------------------------
  std::printf("[1] ASTAR range, density, dE/dx and its integral\n");
  checkNear(fpGasDensity(198), rho198, 1e-12, "fpGasDensity(198 mbar) = 3.575334e-4 g/cm3");
  checkNear(fpRangeMm(5.0, fpGasDensity(198)), 121.75085704, 1e-6, "R(5.0 MeV) at 198 mbar = 121.75086 mm");
  PRESSURE_MBAR = 198;  // stoppedAlphas.C
  checkNear(fpRangeMm(5.0, fpGasDensity(198)), rangeFromEnergy(5.0), 1e-9, "R(5.0 MeV) = stoppedAlphas.C rangeFromEnergy");
  checkNear(fpRangeMm(3.3, fpGasDensity(198)), rangeFromEnergy(3.3), 1e-9, "R(3.3 MeV) = stoppedAlphas.C rangeFromEnergy");
  checkNear(fpEnergyFromRangeMm(rangeFromEnergy(3.3), fpGasDensity(198)), 3.3, 1e-9, "E(R(3.3 MeV)) = 3.3");
  checkNear(fpEnergyFromRangeMm(55.0, fpGasDensity(198)), energyFromRange(55.0), 1e-9, "E(55 mm) = stoppedAlphas.C energyFromRange");
  checkNear(fpDedx(121.75085704 - 1e-9, fpGasDensity(198)), 0.0276294809, 1e-8, "dE/dR at R(5 MeV) (segment 4.5-5.0) = 0.0276295 MeV/mm");
  for (double E : {5.0, 2.0}) {
    FpTrack t;
    t.S[0] = 0;
    t.E[0] = fpRangeMm(E, rho198);  // along x, stops at E
    t.A = 1.0;
    const int n = fpNumPoints(t.E[0]);
    double sum = 0, xyz[3];
    for (int i = 0; i < n; ++i) sum += fpPoint(t, rho198, n, i, xyz);
    checkNear(sum, E, 0.005 * E, Form("sum dE/dx * h over the 0.1 mm points = %.1f MeV (+-0.5 %%)", E));
  }
  {
    FpTrack t;
    t.E[0] = 10.0;
    t.A = 2.0;
    double xyz[3];
    checkNear(fpNumPoints(10.0), 100, 0, "fpNumPoints(10 mm) = 100 (0.1 mm steps)");
    const double q0 = fpPoint(t, rho198, 100, 0, xyz);
    checkNear(xyz[0], 0.05, 1e-12, "point 0 at the midpoint 0.05 mm of the first step");
    checkNear(q0, 2.0 * (fpEnergyFromRangeMm(10.0, rho198) - fpEnergyFromRangeMm(9.9, rho198)), 1e-12,
              "q = A * (E(10.0 mm) - E(9.9 mm)) = A * integral of dE/dx over the step");
    checkNear(q0, 2.0 * fpDedx(9.95, rho198) * 0.1, 1e-3 * q0, "q = A * dE/dx(residual range 9.95 mm) * 0.1 within 0.1 %");
    const double q99 = fpPoint(t, rho198, 100, 99, xyz);
    checkNear(q99, 2.0 * fpEnergyFromRangeMm(0.1, rho198), 1e-15, "last point: A * E(0.1 mm) (stopping end = E)");
    // R(0.1 MeV) = 1.450e-4 / rho * 10 = 4.0556 mm: no step in dE/dx there (first row extended)
    const double r01 = 1.450e-3 / rho198;
    checkNear(fpDedx(r01 - 1e-7, rho198), fpDedx(r01 + 1e-7, rho198), 1e-8, "dE/dx continuous at 0.1 MeV (R 4.06 mm)");
    // 60 mm: the charge is largest at residual range 13.68-15.08 mm (ASTAR stopping power maximum
    // 0.0712 MeV/mm at 0.65-0.75 MeV), the first point (R ~ 60 mm) is lower
    FpTrack t6;
    t6.E[0] = 60.0;
    t6.A = 1.0;
    int iMax = 0;
    double qMax = 0, qFirst = fpPoint(t6, rho198, 600, 0, xyz);
    for (int i = 0; i < 600; ++i) {
      const double q = fpPoint(t6, rho198, 600, i, xyz);
      if (q > qMax) qMax = q, iMax = i;
    }
    const double rMax = 60.0 - (iMax + 0.5) * 0.1;
    check(rMax >= 13.68 && rMax <= 15.08 && qFirst < 0.7 * qMax,
          Form("60 mm: Bragg maximum at residual range %.2f mm (13.68-15.08), first point %.3f of it", rMax, qFirst / qMax));
  }

  // ---- [2] band integral ------------------------------------------------------------------
  std::printf("[2] strip band integral\n");
  {
    FpGeom g = makeGeom(72, 92, 92, 0.25);
    int k[64];
    double w[64];
    int m = fpStripWeights(g.pl[0], 0.3, 0.9, k, w, 64);
    double sum = 0;
    for (int i = 0; i < m; ++i) sum += w[i];
    checkNear(sum, 1.0, 1e-6, "sum over strips = 1 (centre inside, sigma 0.9; the 5 sigma cut loses < 1e-6)");
    // strip 36 at s = -0.75, strip 37 at +0.75: their boundary is s = 0
    m = fpStripWeights(g.pl[0], 0.0, 1.0, k, w, 64);
    double w36 = -1, w37 = -1;
    for (int i = 0; i < m; ++i) {
      if (k[i] == 36) w36 = w[i];
      if (k[i] == 37) w37 = w[i];
    }
    checkNear(w36, 0.4331927987, 1e-9, "centre on the 36|37 boundary, sigma 1: strip 36 = 0.5 - Phi(-1.5)");
    checkNear(w37, 0.4331927987, 1e-9, "centre on the 36|37 boundary, sigma 1: strip 37 = 0.5 - Phi(-1.5)");
    m = fpStripWeights(g.pl[0], 0.0, 0.05, k, w, 64);
    w36 = w37 = -1;
    for (int i = 0; i < m; ++i) {
      if (k[i] == 36) w36 = w[i];
      if (k[i] == 37) w37 = w[i];
    }
    checkNear(w36, 0.5, 1e-9, "centre on the boundary, sigma 0.05: 0.5");
    checkNear(w37, 0.5, 1e-9, "centre on the boundary, sigma 0.05: 0.5");
    m = fpStripWeights(g.pl[0], 54.0, 1.0, k, w, 64);  // outer edge of strip 72 (s = 53.25 + 0.75)
    sum = 0;
    for (int i = 0; i < m; ++i) sum += w[i];
    checkNear(sum, 0.5, 1e-6, "centre on the outer edge of the last strip: half the charge is lost");
    m = fpStripWeights(g.pl[0], 80.0, 1.0, k, w, 64);
    check(m == 0, "centre 26 mm outside the plane: no strip");
    FpPlane rev = g.pl[0];  // reversed order: strip k at s = -1.5 k + offset
    rev.slope = -1.5;
    rev.offset = 54.75;
    m = fpStripWeights(rev, 0.0, 0.05, k, w, 64);
    w36 = w37 = -1;
    for (int i = 0; i < m; ++i) {
      if (k[i] == 36) w36 = w[i];
      if (k[i] == 37) w37 = w[i];
    }
    checkNear(w36 + w37, 1.0, 1e-9, "negative slope: strips 36 (s = +0.75) and 37 (s = -0.75) share the charge");
  }

  // ---- [3] kernel 25 -> 12.5 MHz ----------------------------------------------------------
  std::printf("[3] kernel conversion and JSON reader\n");
  {
    const std::vector<double> h25 = agetSamples25(502, 0.008);
    FpKernel k;
    std::string err;
    check(fpKernelFromSamples(h25, 2, k, err), "fpKernelFromSamples(factor 2) ok (" + err + ")");
    checkNear(k.i0, 20, 0, "12.5 MHz peak index 20 (25 MHz pair 40, 41)");
    checkNear(k.h.empty() ? -1 : k.h[k.i0], 1.0, 1e-12, "peak normalised to 1");
    checkNear(argmax(k.h), k.i0, 0, "i0 = argmax");
    checkNear(k.h.empty() ? -1 : k.h[0], 0.0, 1e-12, "floor removed (mean of the first 25 samples)");
    std::vector<double> h25n(h25.size());
    const double fl = 0.008 / 1.008;
    for (size_t i = 0; i < h25.size(); ++i) h25n[i] = (h25[i] - fl) / (1 - fl);
    const double f25 = fwhm(h25n), f12 = k.h.empty() ? -1 : fwhm(k.h);
    std::printf("     FWHM 25 MHz %.3f cells (%.0f ns), 12.5 MHz %.3f cells (%.0f ns)\n", f25, f25 * 40, f12, f12 * 80);
    checkNear(f12, f25 / 2, 1.0, "FWHM at 12.5 MHz = FWHM at 25 MHz / 2 within 1 cell");
    checkNear(k.h.size(), 170, 0, "341 samples -> 170 pairs");
    FpKernel k1;
    check(fpKernelFromSamples(h25, 1, k1, err) && k1.i0 == 40 && k1.h.size() == 341, "factor 1 (25 MHz): unchanged length, i0 = 40");

    const std::string dir = std::string(gSystem->TempDirectory()) + "/test_fitProngs_k_" + std::to_string(gSystem->GetPid());
    gSystem->mkdir(dir.c_str(), kTRUE);
    std::string js = "{\"run\": 1, \"window_cells\": [-31, 3], \"all\": {\"fwhm_cells\": 2.0, \"mean_norm\": [";
    for (int i = 0; i < 30; ++i) js += "0.01, ";
    js += "0.01, 0.51, 1.01, 0.51, 0.01]}, \"fpn\": {\"mean_norm\": [9, 9, 9]}}\n";
    const std::string jpath = dir + "/k.json";
    writeText(jpath, js);
    FpKernel kj;
    check(fpLoadKernel(jpath.c_str(), 25.0, kj, err), "fpLoadKernel(25 MHz) ok (" + err + ")");
    check(kj.h.size() == 35 && kj.i0 == 32, "JSON: 35 samples of all.mean_norm (not fpn), peak index 32");
    if (kj.h.size() == 35) {
      checkNear(kj.h[31], 0.5, 1e-12, "JSON: (0.51 - 0.01) / 1.0 = 0.5");
      checkNear(kj.h[32], 1.0, 1e-12, "JSON: peak 1");
      checkNear(kj.h[0], 0.0, 1e-12, "JSON: floor 0");
    }
    check(!fpLoadKernel(jpath.c_str(), 50.0, kj, err), "50 MHz: no kernel (25 / 50 is not an integer) -> false");
    check(!fpLoadKernel((dir + "/nope.json").c_str(), 25.0, kj, err), "missing JSON -> false");
    gSystem->Unlink(jpath.c_str());
    gSystem->Unlink(dir.c_str());

    const char *real = std::getenv("FITPRONGS_REAL_KERNEL");
    if (real && *real) {
      FpKernel kr;
      err.clear();
      check(fpLoadKernel(real, 12.5, kr, err), std::string("[3b] real kernel ") + real + " (" + err + ")");
      const double fr = kr.h.empty() ? -1 : fwhm(kr.h);
      std::printf("     real kernel at 12.5 MHz: %zu samples, i0 %d, FWHM %.3f cells (%.0f ns)\n", kr.h.size(), kr.i0, fr, fr * 80);
      checkNear(fr, 19.696111 / 2, 1.0, "[3b] FWHM = 19.7 cells @ 40 ns / 2 within 1 cell @ 80 ns");
      checkNear(kr.i0 * 80.0, 40 * 40.0, 80.0, "[3b] peak time = 25 MHz peak (40 x 40 ns) within 1 cell");
    } else {
      std::printf("SKIP  [3b] real kernel (set FITPRONGS_REAL_KERNEL)\n");
    }
  }

  // ---- [4] charge conservation, [5] saturation -------------------------------------------
  std::printf("[4] charge conservation  [5] saturation\n");
  {
    FpGeom g = makeGeom(120, 120, 120, 0.25);
    FpKernel k;
    k.h = {0.5, 1.0, 0.5};
    k.i0 = 1;
    FpTrack t;
    const double L = fpRangeMm(2.0, rho198);  // 35.9966 mm
    const double d[3] = {0.6, 0.48, 0.64};    // unit vector (0.36 + 0.2304 + 0.4096 = 1)
    t.S[0] = -10;
    t.S[1] = 4;
    t.S[2] = 30;
    for (int j = 0; j < 3; ++j) t.E[j] = t.S[j] + L * d[j];
    t.A = 1000;
    t.sigT = 1.0;
    t.sigL = 1.2;
    const int n = fpNumPoints(L);
    double sumq = 0, xyz[3];
    for (int i = 0; i < n; ++i) sumq += fpPoint(t, rho198, n, i, xyz);
    FpImage model, work;
    fpRender(t, rho198, n, g, k, nullptr, model, work);
    for (int v = 0; v < 3; ++v) {
      const double s = model.v[v].empty() ? -1 : imageSum(model, v);
      checkNear(s, 4000.0, 20.0, Form("[4] view %d: sum = A * E * (area/peak) = 1000 * 2.0 * 2 = 4000 (+-0.5 %%)", v));
      checkNear(s, sumq * 2.0, 1e-5 * 4000, Form("[4] view %d: sum = 2 * sum(q) (exact up to the 5 sigma cuts of the two Gaussians, < 1e-5)", v));
    }
    // ceiling = half the largest model pixel of the view (the [4] track is faint: max ~10 ADC)
    std::vector<double> ceil[3];
    double cv[3];
    for (int v = 0; v < 3; ++v) {
      cv[v] = 0.5 * imageMax(model, v);
      ceil[v].assign(g.pl[v].n, cv[v]);
    }
    FpImage clipped;
    fpRender(t, rho198, n, g, k, ceil, clipped, work);
    for (int v = 0; v < 3; ++v) {
      bool same = !clipped.v[v].empty() && clipped.v[v].size() == model.v[v].size();
      int nClip = 0;
      for (size_t i = 0; same && i < model.v[v].size(); ++i) {
        const double want = std::min(model.v[v][i], cv[v]);
        if (std::fabs(clipped.v[v][i] - want) > 1e-12) same = false;
        if (model.v[v][i] > cv[v]) ++nClip;
      }
      check(same && nClip > 0 && imageMax(clipped, v) == cv[v],
            Form("[5] view %d: pixels above the ceiling (%.2f ADC) cut to it (%d), the rest unchanged", v, cv[v], nClip));
    }
  }

  // ---- [7] chi2 and ROI -------------------------------------------------------------------
  std::printf("[7] chi2, ndf and ROI\n");
  {
    FpGeom g = makeGeom(2, 2, 2, 0.5, 3);
    FpImage data, model;
    data.init(g);
    model.init(g);
    const double dU[6] = {1, 2, 3, 4, 5, 6}, mU[6] = {0, 2, 1, 4, 8, 6};
    const double dV[6] = {1, 1, 1, 9, 9, 9}, mV[6] = {0, 0, 0, 0, 0, 0};
    const double dW[6] = {2, 2, 2, 100, 100, 100}, mW[6] = {1.5, 2, 2.5, 0, 0, 0};
    bool sized = data.v[0].size() == 6 && data.v[1].size() == 6 && data.v[2].size() == 6;
    check(sized, "FpImage::init: 2 strips x 3 cells per view");
    if (sized) {
      for (int i = 0; i < 6; ++i) {
        data.v[0][i] = dU[i];
        model.v[0][i] = mU[i];
        data.v[1][i] = dV[i];
        model.v[1][i] = mV[i];
        data.v[2][i] = dW[i];
        model.v[2][i] = mW[i];
      }
      std::vector<char> mask[3], excl[3];
      mask[0].assign(6, 1);
      mask[1] = {1, 1, 1, 0, 0, 0};
      mask[2].assign(6, 1);
      excl[0].assign(2, 0);
      excl[1].assign(2, 0);
      excl[2] = {0, 1};
      const double noise[3] = {2, 1, 0.5};
      int nb = -1;
      const double c2 = fpChi2(data, model, mask, excl, noise, nb, 0.0);
      checkNear(c2, 8.5, 1e-12, "chi2 = 3.5 (U) + 3 (V, mask) + 2 (W, strip 2 excluded) = 8.5");
      checkNear(nb, 12, 0, "bins = 6 + 3 + 3 = 12 -> ndf = 12 - 9 = 3");
    }
    // ROI
    FpGeom gr = makeGeom(41, 41, 41, 0.5, 100);
    std::vector<char> mask[3];
    const double P[3] = {0, 0, 10};
    fpRoi(P, P, gr, mask);
    for (int v = 0; v < 3; ++v) {
      long cnt = 0;
      for (char c : mask[v]) cnt += c;
      checkNear(cnt, 1071, 0, Form("ROI of a point, view %d: 21 strips x 51 cells = 1071", v));
    }
    const double S[3] = {-10, 3, 10}, E[3] = {12, -7, 34};
    fpRoi(S, E, gr, mask);
    for (int v = 0; v < 3; ++v) {
      long diff = 0, cnt = 0;
      for (int kk = 1; kk <= 41; ++kk)
        for (int tt = 0; tt < 100; ++tt) {
          const double sk = gr.pl[v].slope * kk + gr.pl[v].offset, zn = tt * 0.5;
          bool in = false;
          for (int it = 0; it <= 10000 && !in; ++it) {
            const double u = it / 10000.0;
            const double x = S[0] + u * (E[0] - S[0]), y = S[1] + u * (E[1] - S[1]), z = S[2] + u * (E[2] - S[2]);
            const double s = FP_PITCH[v][0] * x + FP_PITCH[v][1] * y;
            in = std::fabs(s - sk) <= 15 && std::fabs(z - zn) <= 15;
          }
          const char got = mask[v].empty() ? 0 : mask[v][(size_t)(kk - 1) * 100 + tt];
          cnt += got;
          if (got != (char)in) ++diff;
        }
      check(diff == 0 && cnt > 0, Form("ROI of a segment, view %d: %ld pixels, %ld differ from the brute-force scan", v, cnt, diff));
    }
  }

  // ---- [8] data grid ----------------------------------------------------------------------
  std::printf("[8] data grid, noise, ceiling, saturation\n");
  {
    FpGeom g = makeGeom(3, 1, 1, 0.25);
    int view[4][68], strip[4][68];
    for (int a = 0; a < 4; ++a)
      for (int c = 0; c < 68; ++c) {
        view[a][c] = -1;
        strip[a][c] = 0;
      }
    view[0][0] = 0, strip[0][0] = 1;
    view[0][1] = 0, strip[0][1] = 2;
    view[0][2] = 0, strip[0][2] = 3;
    view[0][3] = 1, strip[0][3] = 1;
    view[0][4] = 2, strip[0][4] = 1;
    for (int a = 0; a < 4; ++a)
      for (int c = 0; c < 68; ++c)
        for (int t = 0; t < 512; ++t) g_adc[a][c][t] = 3000;  // unmapped channels: ignored
    const int amp[5] = {2, 4, 6, 1, 1}, base[5] = {100, 100, 100, 250, 300};
    for (int c = 0; c < 5; ++c)
      for (int t = 0; t < 512; ++t)
        g_adc[0][c][t] = (Short_t)(t < 25 ? base[c] + (t % 2 ? amp[c] : -amp[c]) : base[c]);
    // 25 cells alternate -a / +a starting with -a: 13 x (-a) + 12 x (+a) -> mean = base - a / 25
    g_adc[0][0][200] = 300;
    g_adc[0][1][300] = 4090;
    g_adc[0][2][300] = 4089;
    FpData d;
    fpFillData(g_adc, view, strip, g, d);
    bool sized = d.img.v[0].size() == 3 * 512 && d.sat[0].size() == 3 && d.ceil[0].size() == 3;
    check(sized, "fpFillData: U 3 strips x 512 cells, per-strip ceiling / saturation flags");
    if (sized) {
      const double mean0 = 100 - 2.0 / 25;
      checkNear(d.img.get(0, 1, 200), 300 - mean0, 1e-9, "U1 cell 200: 300 - baseline (mean of cells 0..24)");
      checkNear(d.img.get(0, 1, 300), 100 - mean0, 1e-9, "U1 cell 300: 100 - baseline");
      // RMS around the mean: a^2 - (a/25)^2 (13 x -a, 12 x +a)
      const double rms[3] = {std::sqrt(4 - 4.0 / 625), std::sqrt(16 - 16.0 / 625), std::sqrt(36 - 36.0 / 625)};
      checkNear(d.noise[0], rms[1], 1e-9, "U noise = median of the strip RMS (2, 4, 6) = 4 (exactly sqrt(16 - 16/625))");
      checkNear(d.noise[1], std::sqrt(1 - 1.0 / 625), 1e-9, "V noise (1 strip)");
      checkNear(d.ceil[0][0], 4095 - mean0, 1e-9, "U1 ceiling = 4095 - baseline");
      checkNear(d.ceil[1][0], 4095 - (250 - 1.0 / 25), 1e-9, "V1 ceiling = 4095 - baseline");
      check(d.sat[0][0] == 0 && d.sat[0][1] == 1 && d.sat[0][2] == 0, "saturated: U2 (raw 4090) yes, U1 / U3 (4089) no");
    }
  }

  // ---- [6] closure ------------------------------------------------------------------------
  std::printf("[6] closure: model -> noise -> fit\n");
  const double mmPerCell = driftVelocityCmUs(1600, 20, 198) / 12.5 * 10;
  checkNear(fpMmPerCell(198, 12.5, 1600), mmPerCell, 1e-12, "fpMmPerCell = analyzeUVW.C driftVelocityCmUs / 12.5 * 10");
  {
    FpGeom g = makeGeom(72, 92, 92, mmPerCell);
    int view[4][68], strip[4][68];
    makeChannels(g, view, strip);
    FpKernel k;
    std::string err;
    fpKernelFromSamples(agetSamples25(502, 0.008), 2, k, err);
    TRandom3 rng(197);
    const double Ls[3] = {20, 40, 60}, dips[3] = {0, 30, 60};
    int ic = 0;
    for (double L : Ls)
      for (double dip : dips) {
        const double phi = (20 + 40 * ic) * M_PI / 180, th = dip * M_PI / 180;
        const double d[3] = {std::cos(th) * std::cos(phi), std::cos(th) * std::sin(phi), std::sin(th)};
        const double M[3] = {0, -5, 60};
        FpTrack t;
        for (int j = 0; j < 3; ++j) {
          t.S[j] = M[j] - 0.5 * L * d[j];
          t.E[j] = M[j] + 0.5 * L * d[j];
        }
        t.A = 5000;
        t.sigT = 0.8;
        t.sigL = 1.3;
        FpImage model, work;
        fpRender(t, rho198, fpNumPoints(L), g, k, nullptr, model, work);
        double maxAdc = 0;
        makeAdc(model, view, strip, 250, &rng, 7.0, &maxAdc);
        FpData data;
        fpFillData(g_adc, view, strip, g, data);
        // initial endpoints moved by ~1.5 mm; odd cases give the track reversed
        const double dS[3] = {1.2, -0.8, 0.9}, dE[3] = {-1.0, 1.1, -0.7};
        double a0[3], b0[3];
        for (int j = 0; j < 3; ++j) {
          a0[j] = t.S[j] + dS[j];
          b0[j] = t.E[j] + dE[j];
        }
        const bool rev = ic % 2 == 1;
        TStopwatch sw;
        const FpEventFit f = rev ? fpFitEvent(data, g, k, rho198, b0, a0, true, 0.0, 0.0) : fpFitEvent(data, g, k, rho198, a0, b0, true, 0.0, 0.0);
        const int want = rev ? 1 : 0;
        const FpFit &b = f.fit[f.dir < 0 ? 0 : f.dir];
        const double dSf = dist3(b.par, t.S), dEf = dist3(b.par + 3, t.E);
        std::printf("     L %2.0f dip %2.0f phi %3.0f: dir %d (want %d) chi2 %.0f / %.0f ndf %d  |dS| %.3f |dE| %.3f mm  "
                    "A %.0f sT %.3f sL %.3f  status %d/%d calls %d/%d  max adc %.0f  %.2f s\n",
                    L, dip, phi * 180 / M_PI, f.dir, want, f.fit[0].chi2, f.fit[1].chi2, f.ndf, dSf, dEf, b.par[6], b.par[7],
                    b.par[8], f.fit[0].status, f.fit[1].status, f.fit[0].ncalls, f.fit[1].ncalls, maxAdc, sw.RealTime());
        check(maxAdc < 4090, Form("[6] fixture L %.0f dip %.0f: no saturated sample (max %.0f)", L, dip, maxAdc));
        check(f.dir == want, Form("[6] L %.0f dip %.0f: stopping end chosen correctly (dir %d)", L, dip, f.dir));
        check(dSf <= 1.0 && dEf <= 1.0, Form("[6] L %.0f dip %.0f: endpoints within 1 mm (|dS| %.3f, |dE| %.3f)", L, dip, dSf, dEf));
        check(b.atLimit == 0, Form("[11] L %.0f dip %.0f: no end point at its +-10 mm limit (atLimit %d)", L, dip, b.atLimit));
        ++ic;
      }
  }

  // ---- [9] end to end ---------------------------------------------------------------------
  std::printf("[9] fitProngs() end to end on synthetic files\n");
  {
    const std::string dir = std::string(gSystem->TempDirectory()) + "/test_fitProngs_" + std::to_string(gSystem->GetPid());
    gSystem->mkdir(dir.c_str(), kTRUE);
    gSystem->ChangeDirectory(dir.c_str());
    FpGeom g = makeGeom(72, 92, 92, mmPerCell);
    int view[4][68], strip[4][68];
    makeChannels(g, view, strip);
    std::string csv = "# aget,raw_channel,view(0=U 1=V 2=W),strip,s_mm  (synthetic, test_fitProngs.C)\n";
    for (int a = 0; a < 4; ++a)
      for (int c = 0; c < 68; ++c)
        if (view[a][c] >= 0) {
          const FpPlane &p = g.pl[view[a][c]];
          csv += Form("%d,%d,%d,%d,%.3f\n", a, c, view[a][c], strip[a][c], p.slope * strip[a][c] + p.offset);
        }
    writeText("channel_map.csv", csv);
    const std::vector<double> h25 = agetSamples25(502, 0.008);
    std::string js = "{\"run\": 1, \"window_cells\": [-40, 300], \"all\": {\"mean_norm\": [";
    for (size_t i = 0; i < h25.size(); ++i) js += Form("%s%.10g", i ? ", " : "", h25[i]);
    js += "]}}\n";
    writeText("kernel.json", js);
    FpKernel k;
    std::string err;
    fpKernelFromSamples(h25, 2, k, err);
    // event 77 (entry 0): 30 mm alpha stopping at E; event 78 (entry 1): ok = 0
    FpTrack t;
    const double S[3] = {-12, 4, 50}, E[3] = {12, 16, 62};  // |E - S| = sqrt(576 + 144 + 144) = 29.39 mm
    for (int j = 0; j < 3; ++j) {
      t.S[j] = S[j];
      t.E[j] = E[j];
    }
    t.A = 5000;
    t.sigT = 0.9;
    t.sigL = 1.1;
    FpImage model, work;
    fpRender(t, rho198, fpNumPoints(29.39), g, k, nullptr, model, work);
    {
      TFile f("run0001_raw.root", "RECREATE");
      TTree tr("raw", "synthetic");
      UInt_t id;
      tr.Branch("eventId", &id);
      tr.Branch("adc", g_adc, "adc[4][68][512]/S");
      TRandom3 rng(1971);
      double mx;
      makeAdc(model, view, strip, 250, &rng, 7.0, &mx);
      id = 77;
      tr.Fill();
      id = 78;
      tr.Fill();
      tr.Write();
    }
    {
      TFile f("run0001_tracks.root", "RECREATE");
      TTree tr("tracks", "synthetic");
      UInt_t id;
      Int_t ok;
      Float_t xs, ys, zs, xe, ye, ze, lf;
      tr.Branch("eventId", &id);
      tr.Branch("ok", &ok);
      tr.Branch("xStartF", &xs);
      tr.Branch("yStartF", &ys);
      tr.Branch("zStartF", &zs);
      tr.Branch("xEndF", &xe);
      tr.Branch("yEndF", &ye);
      tr.Branch("zEndF", &ze);
      tr.Branch("lengthF", &lf);
      // tracks file order: start = the STOPPING end (as makeTracks may give it) -> expected dir 1
      id = 77, ok = 1, xs = E[0] - 1, ys = E[1] + 1, zs = E[2] - 0.5, xe = S[0] + 1, ye = S[1] - 1, ze = S[2] + 0.5, lf = 27;
      tr.Fill();
      id = 78, ok = 0, xs = ys = zs = xe = ye = ze = lf = 0;
      tr.Fill();
      tr.Write();
      TParameter<double>("mm_per_cell", mmPerCell, 'f').Write();
    }
    fitProngs("run0001_raw.root", "run0001_tracks.root", 198, 12.5, 1600, "kernel.json", "out_fit.root", "", "png");
    TFile *fo = TFile::Open("out_fit.root");
    TTree *ft = fo ? (TTree *)fo->Get("fit") : nullptr;
    check(ft != nullptr, "[9] out_fit.root has tree 'fit'");
    if (ft) {
      check(ft->GetEntries() == 1, Form("[9] 1 entry (the ok = 0 event is skipped): %lld", ft->GetEntries()));
      UInt_t id = 0;
      Long64_t entry = -1;
      Int_t dirB = -1, ndf = 0, nSat = -1, status[2] = {-9, -9};
      Double_t chi2[2], xS, yS, zS, xE, yE, zE, A, sT, sL, len, tsec, energy;
      ft->SetBranchAddress("eventId", &id);
      ft->SetBranchAddress("entry", &entry);
      ft->SetBranchAddress("dir", &dirB);
      ft->SetBranchAddress("chi2", chi2);
      ft->SetBranchAddress("ndf", &ndf);
      ft->SetBranchAddress("xS", &xS);
      ft->SetBranchAddress("yS", &yS);
      ft->SetBranchAddress("zS", &zS);
      ft->SetBranchAddress("xE", &xE);
      ft->SetBranchAddress("yE", &yE);
      ft->SetBranchAddress("zE", &zE);
      ft->SetBranchAddress("A", &A);
      ft->SetBranchAddress("sigmaT", &sT);
      ft->SetBranchAddress("sigmaL", &sL);
      ft->SetBranchAddress("length", &len);
      ft->SetBranchAddress("energy", &energy);
      ft->SetBranchAddress("status", status);
      ft->SetBranchAddress("time_s", &tsec);
      ft->SetBranchAddress("nSatStrips", &nSat);
      Double_t sXY = -1, noiseB[3] = {-1, -1, -1};
      Int_t atLimit = -1, cmSkipped = -1;
      const bool haveNew = ft->GetBranch("sigmaXY") && ft->GetBranch("atLimit") && ft->GetBranch("cmSkipped") && ft->GetBranch("noise");
      check(haveNew, "[15] tree has sigmaXY, atLimit, cmSkipped, noise");
      if (haveNew) {
        ft->SetBranchAddress("sigmaXY", &sXY);
        ft->SetBranchAddress("atLimit", &atLimit);
        ft->SetBranchAddress("cmSkipped", &cmSkipped);
        ft->SetBranchAddress("noise", noiseB);
      }
      ft->GetEntry(0);
      const double fS[3] = {xS, yS, zS}, fE[3] = {xE, yE, zE};
      std::printf("     eventId %u entry %lld dir %d chi2 %.1f / %.1f ndf %d S (%.2f %.2f %.2f) E (%.2f %.2f %.2f) A %.0f "
                  "sT %.3f sL %.3f len %.2f E %.3f MeV status %d/%d %.2f s nSat %d\n",
                  id, entry, dirB, chi2[0], chi2[1], ndf, xS, yS, zS, xE, yE, zE, A, sT, sL, len, energy, status[0],
                  status[1], tsec, nSat);
      check(id == 77 && entry == 0, "[9] eventId 77, entry 0");
      check(dirB == 1, "[9] dir 1: the stop is at the tracks file's START end");
      check(dist3(fS, S) <= 1.0 && dist3(fE, E) <= 1.0, "[9] S = entering end, E = stopping end, both within 1 mm");
      checkNear(len, dist3(fS, fE), 1e-9, "[9] length = |E - S|");
      checkNear(energy, fpEnergyFromRangeMm(len, rho198), 1e-9, "[9] energy = ASTAR energy of the length");
      check(ndf > 0 && nSat == 0 && tsec > 0, "[9] ndf > 0, nSatStrips 0, time_s > 0");
      check(gSystem->AccessPathName("png/event0000077.png") == kFALSE, "[9] png/event0000077.png written");
      FileStat_t st;
      const bool gotStat = gSystem->GetPathInfo("png/event0000077.png", st) == 0;
      check(gotStat && st.fSize >= 10000, Form("[16] png >= 10 kB (%lld bytes)", gotStat ? (Long64_t)st.fSize : -1LL));
      if (haveNew) {
        checkNear(sXY, std::sqrt(sT * sT + 0.146), 1e-9, "[15] sigmaXY = sqrt(sigmaT^2 + 0.146)");
        check(atLimit == 0 && cmSkipped == 0, Form("[15] atLimit %d, cmSkipped %d (both 0 on this event)", atLimit, cmSkipped));
        check(ft->GetBranch("initClipped") != nullptr, "[17] tree has initClipped");
        check(noiseB[0] > 0 && noiseB[1] > 0 && noiseB[2] > 0, Form("[15] noise %.2f %.2f %.2f (> 0)", noiseB[0], noiseB[1], noiseB[2]));
        std::printf("     sigmaT %.4f sigmaXY %.4f noise %.3f %.3f %.3f\n", sT, sXY, noiseB[0], noiseB[1], noiseB[2]);
      }
      TParameter<double> *apm = (TParameter<double> *)fo->Get("adc_per_mev");
      check(apm && apm->GetVal() == 0, "[14] TParameter adc_per_mev = 0 (free) stored");
      fo->Close();
    }
    fitProngs("run0001_raw.root", "run0001_tracks.root", 198, 12.5, 1600, "kernel.json", "out_fit1.root", "1", "");
    fo = TFile::Open("out_fit1.root");
    ft = fo ? (TTree *)fo->Get("fit") : nullptr;
    check(ft && ft->GetEntries() == 0, "[9] entryList \"1\" (ok = 0): no fit, empty tree");
    if (fo) fo->Close();
    fitProngs("run0001_raw.root", "run0001_tracks.root", 198, 12.5, 1600, "nope.json", "out_fit2.root", "", "");
    check(gSystem->AccessPathName("out_fit2.root") == kTRUE, "[9] missing kernel JSON: nothing written");
    for (const char *f : {"channel_map.csv", "kernel.json", "run0001_raw.root", "run0001_tracks.root", "out_fit.root",
                          "out_fit1.root", "png/event0000077.png"})
      gSystem->Unlink(f);
    gSystem->Unlink("png");
    gSystem->ChangeDirectory(startDir.c_str());
    gSystem->Unlink(dir.c_str());
  }


  // ---- [10] sensitive box -----------------------------------------------------------------
  std::printf("[10] sensitive box (cage x +-53, y -58..48)\n");
  {
    FpGeom g = makeGeom(120, 120, 120, 0.25);
    FpKernel k;
    k.h = {0.5, 1.0, 0.5};
    k.i0 = 1;
    FpTrack t;
    t.S[0] = 40, t.S[1] = 0, t.S[2] = 30;
    t.E[0] = 100, t.E[1] = 0, t.E[2] = 30;
    t.A = 1000;
    t.sigT = 1.0;
    t.sigL = 1.2;
    FpImage model, work;
    fpRender(t, rho198, fpNumPoints(60.0), g, k, nullptr, model, work);
    // x <= 53 = the first 13 mm: residual range from 60 to 47 mm
    const double want = 1000.0 * (fpEnergyModel(60.0, rho198) - fpEnergyModel(47.0, rho198)) * 2.0;
    for (int v = 0; v < 3; ++v)
      checkNear(imageSum(model, v), want, 0.005 * want, Form("[10] view %d: sum = A (E(60) - E(47)) * 2 = %.2f (+-0.5 %%)", v, want));
    // single-step tracks (0.1 mm: one point at the midpoint)
    auto drawn = [&](double xs, double ys, double xe, double ye) {
      FpTrack s;
      s.S[0] = xs, s.S[1] = ys, s.S[2] = 30;
      s.E[0] = xe, s.E[1] = ye, s.E[2] = 30;
      s.A = 1000;
      s.sigT = 1.0;
      s.sigL = 1.2;
      FpImage m, w;
      fpRender(s, rho198, 1, g, k, nullptr, m, w);
      return imageSum(m, 0) + imageSum(m, 1) + imageSum(m, 2);
    };
    check(drawn(52.94, 0, 53.04, 0) > 0, "[10] point at x = 52.99: drawn");
    check(drawn(52.96, 0, 53.06, 0) == 0, "[10] point at x = 53.01: charge 0");
    check(drawn(-53.04, 0, -52.94, 0) > 0, "[10] point at x = -52.99: drawn");
    check(drawn(-53.06, 0, -52.96, 0) == 0, "[10] point at x = -53.01: charge 0");
    check(drawn(0, 47.94, 0, 48.04) > 0, "[10] point at y = 47.99: drawn");
    check(drawn(0, 47.96, 0, 48.06) == 0, "[10] point at y = 48.01: charge 0");
    check(drawn(0, -58.04, 0, -57.94) > 0, "[10] point at y = -57.99: drawn");
    check(drawn(0, -58.06, 0, -57.96) == 0, "[10] point at y = -58.01: charge 0");
  }

  // ---- [11] end-point box -----------------------------------------------------------------
  std::printf("[11] end points limited to start +- 10 mm\n");
  {
    FpGeom g = makeGeom(72, 92, 92, mmPerCell);
    int view[4][68], strip[4][68];
    makeChannels(g, view, strip);
    FpKernel k;
    std::string err;
    fpKernelFromSamples(agetSamples25(502, 0.008), 2, k, err);
    TRandom3 rng(2011);
    const double L = 40, phi = 60 * M_PI / 180, th = 30 * M_PI / 180;
    const double d[3] = {std::cos(th) * std::cos(phi), std::cos(th) * std::sin(phi), std::sin(th)};
    const double M[3] = {0, -5, 60};
    FpTrack t;
    for (int j = 0; j < 3; ++j) {
      t.S[j] = M[j] - 0.5 * L * d[j];
      t.E[j] = M[j] + 0.5 * L * d[j];
    }
    t.A = 5000;
    t.sigT = 0.8;
    t.sigL = 1.3;
    FpImage model, work;
    fpRender(t, rho198, fpNumPoints(L), g, k, nullptr, model, work);
    double maxAdc = 0;
    makeAdc(model, view, strip, 250, &rng, 7.0, &maxAdc);
    FpData data;
    fpFillData(g_adc, view, strip, g, data);
    double a0[3], b0[3];
    for (int j = 0; j < 3; ++j) {
      a0[j] = t.S[j] + (j == 0 ? 12.0 : 0.0);
      b0[j] = t.E[j] + (j == 0 ? 12.0 : 0.0);
    }
    const FpEventFit f = fpFitEvent(data, g, k, rho198, a0, b0, true, 0.0, 0.0);
    const FpFit &b = f.fit[f.dir < 0 ? 0 : f.dir];
    const double *i0 = f.dir == 1 ? b0 : a0, *i1 = f.dir == 1 ? a0 : b0;  // initial entering / stopping end
    double worst = 0;
    for (int j = 0; j < 3; ++j) worst = std::max({worst, std::fabs(b.par[j] - i0[j]), std::fabs(b.par[3 + j] - i1[j])});
    std::printf("     12 mm off: dir %d atLimit %d max |end - initial| %.4f mm chi2 %.0f ndf %d status %d\n", f.dir, b.atLimit, worst,
                b.chi2, f.ndf, b.status);
    check(b.atLimit != 0, Form("[11] start 12 mm off: atLimit = %d != 0", b.atLimit));
    check(worst <= 10.0 + 1e-6, Form("[11] every end point within initial +- 10 mm (max %.6f)", worst));
    check(worst >= 9.99, Form("[11] ... and the limit is reached (max %.4f mm)", worst));
  }

  // ---- [12] common mode -------------------------------------------------------------------
  std::printf("[12] common-mode subtraction\n");
  {
    FpGeom g = makeGeom(41, 17, 29, 0.25);
    FpData d;
    d.img.init(g);
    std::vector<char> mask[3];
    auto cmOf = [](int t) { return 30.0 * std::sin(2 * M_PI * t / 20.0); };
    auto trackOf = [](int k, int t) { return (k >= 1 && k <= 10 && t >= 100 && t <= 300) ? 50.0 * k + 0.01 * t : 0.0; };
    for (int v = 0; v < 3; ++v) {
      const int n = g.pl[v].n;
      d.noise[v] = 99;
      d.ceil[v].assign(n, 4000);
      d.sat[v].assign(n, 0);
      d.hasCh[v].assign(n, 1);
      mask[v].assign((size_t)n * g.ncell, 0);
      for (int k = 1; k <= n; ++k)
        for (int t = 0; t < g.ncell; ++t) {
          d.img.at(v, k, t) = cmOf(t) + trackOf(k, t);
          if (k <= 10 && t >= 100 && t <= 300) mask[v][(size_t)(k - 1) * g.ncell + t] = 1;
        }
    }
    for (int t = 0; t < g.ncell; ++t) d.img.at(0, 15, t) = 4095;  // one saturated strip outside the ROI
    d.sat[0][14] = 1;
    for (int k = 11; k <= 20; ++k) {  // view 2: 10 saturated strips (+1000), 9 usable strips (21..29)
      d.sat[2][k - 1] = 1;
      for (int t = 0; t < g.ncell; ++t) d.img.at(2, k, t) = cmOf(t) + 1000;
    }
    const int skipped = fpSubtractCommonMode(d, mask);
    check(skipped == 2, Form("[12] cmSkipped = %d: only view 1 (7 strips outside the ROI < 8) -> bit 1 = 2", skipped));
    double e0 = 0, eIn = 0, e1 = 0, e2 = 0, eSat = 0;
    for (int k = 1; k <= 41; ++k)
      for (int t = 0; t < g.ncell; ++t) {
        if (k == 15) {
          eSat = std::max(eSat, std::fabs(d.img.get(0, k, t) - (4095 - cmOf(t))));
          continue;
        }
        e0 = std::max(e0, std::fabs(d.img.get(0, k, t) - trackOf(k, t)));
      }
    for (int k = 1; k <= 17; ++k)
      for (int t = 0; t < g.ncell; ++t) e1 = std::max(e1, std::fabs(d.img.get(1, k, t) - (cmOf(t) + trackOf(k, t))));
    for (int k = 1; k <= 29; ++k)
      for (int t = 0; t < g.ncell; ++t) {
        const double want = (k >= 11 && k <= 20) ? 1000.0 : trackOf(k, t);
        e2 = std::max(e2, std::fabs(d.img.get(2, k, t) - want));
        if (k <= 10 && t >= 100 && t <= 300) eIn = std::max(eIn, std::fabs(d.img.get(2, k, t) - trackOf(k, t)));
      }
    checkNear(e0, 0, 1e-9, "[12] view 0: outside the ROI 0, ROI pixels = the track alone");
    checkNear(eSat, 0, 1e-9, "[12] view 0: the saturated strip 15 gets the same subtraction (4095 - c_t)");
    checkNear(e1, 0, 1e-9, "[12] view 1 (7 strips outside the ROI): not touched");
    checkNear(e2, 0, 1e-9, "[12] view 2: 10 saturated strips out-number 9 usable ones, the median ignores them");
    checkNear(eIn, 0, 1e-9, "[12] view 2: ROI pixels = the track alone");
    checkNear(d.noise[0], 0, 1e-9, "[12] view 0: noise re-taken after the subtraction (median of the first-25-cell RMS = 0)");
    checkNear(d.noise[1], 99, 0, "[12] view 1: noise kept (not subtracted)");
    checkNear(d.noise[2], 0, 1e-9, "[12] view 2: noise re-taken");
  }

  // ---- [13] error model -------------------------------------------------------------------
  std::printf("[13] chi2 with sigma^2 = noise^2 + (f max(data, 0))^2\n");
  {
    FpGeom g = makeGeom(2, 2, 2, 0.5, 3);
    FpImage data, model;
    data.init(g);
    model.init(g);
    data.v[0][0] = 100, model.v[0][0] = 90;
    data.v[0][1] = -50, model.v[0][1] = -48;
    data.v[0][2] = 7, model.v[0][2] = 0;  // not in the mask
    std::vector<char> mask[3], excl[3];
    mask[0] = {1, 1, 0, 0, 0, 0};
    mask[1].assign(6, 0);
    mask[2].assign(6, 0);
    const double noise[3] = {2, 2, 2};
    int nb = -1;
    const double c2 = fpChi2(data, model, mask, excl, noise, nb, 0.1);
    checkNear(c2, 100.0 / 104.0 + 1.0, 1e-12, "[13] f = 0.1: 10^2 / (4 + 10^2) + (-2)^2 / 4 = 1.961538");
    checkNear(nb, 2, 0, "[13] 2 bins");
    const double c0 = fpChi2(data, model, mask, excl, noise, nb, 0.0);
    checkNear(c0, 100.0 / 4.0 + 1.0, 1e-12, "[13] f = 0: 100 / 4 + 1 = 26");
  }

  // ---- [14] fixed A -----------------------------------------------------------------------
  std::printf("[14] A fixed by adcPerMeV\n");
  {
    FpKernel k2;
    k2.h = {0.5, 1.0, 0.5};
    k2.i0 = 1;
    checkNear(fpFixedA(2000, k2), 1000, 1e-12, "[14] adcPerMeV 2000 / area 2 = A 1000");
    checkNear(fpFixedA(0, k2), 0, 0, "[14] adcPerMeV 0 = free (0)");
    FpGeom g4 = makeGeom(120, 120, 120, 0.25);
    FpTrack t4;
    const double L4 = fpRangeMm(2.0, rho198);
    t4.S[0] = -10, t4.S[1] = 4, t4.S[2] = 30;
    t4.E[0] = -10 + L4 * 0.6, t4.E[1] = 4 + L4 * 0.48, t4.E[2] = 30 + L4 * 0.64;
    t4.A = fpFixedA(2000, k2);
    t4.sigT = 1.0;
    t4.sigL = 1.2;
    FpImage m4, w4;
    fpRender(t4, rho198, fpNumPoints(L4), g4, k2, nullptr, m4, w4);
    for (int v = 0; v < 3; ++v)
      checkNear(imageSum(m4, v), 4000.0, 20.0, Form("[14] view %d: sum = adcPerMeV * E = 2000 * 2.0 = 4000 (+-0.5 %%)", v));

    FpGeom g = makeGeom(72, 92, 92, mmPerCell);
    int view[4][68], strip[4][68];
    makeChannels(g, view, strip);
    FpKernel k;
    std::string err;
    fpKernelFromSamples(agetSamples25(502, 0.008), 2, k, err);
    TRandom3 rng(2014);
    const double L = 40, phi = 120 * M_PI / 180, th = 30 * M_PI / 180;
    const double d[3] = {std::cos(th) * std::cos(phi), std::cos(th) * std::sin(phi), std::sin(th)};
    const double M[3] = {0, -5, 60};
    FpTrack t;
    for (int j = 0; j < 3; ++j) {
      t.S[j] = M[j] - 0.5 * L * d[j];
      t.E[j] = M[j] + 0.5 * L * d[j];
    }
    t.A = 5000;
    t.sigT = 0.8;
    t.sigL = 1.3;
    FpImage model, work;
    fpRender(t, rho198, fpNumPoints(L), g, k, nullptr, model, work);
    double maxAdc = 0;
    makeAdc(model, view, strip, 250, &rng, 7.0, &maxAdc);
    FpData data;
    fpFillData(g_adc, view, strip, g, data);
    const double dS[3] = {1.2, -0.8, 0.9}, dE[3] = {-1.0, 1.1, -0.7};
    double a0[3], b0[3];
    for (int j = 0; j < 3; ++j) {
      a0[j] = t.S[j] + dS[j];
      b0[j] = t.E[j] + dE[j];
    }
    const FpEventFit f = fpFitEvent(data, g, k, rho198, a0, b0, true, 0.0, 5000.0 * fpKernelArea(k));
    const FpFit &b = f.fit[f.dir < 0 ? 0 : f.dir];
    const double dSf = dist3(b.par, t.S), dEf = dist3(b.par + 3, t.E);
    std::printf("     fixed A: dir %d chi2 %.0f ndf %d (bins %d) A %.6f |dS| %.3f |dE| %.3f sT %.3f sL %.3f status %d\n", f.dir, b.chi2,
                f.ndf, b.nBins, b.par[6], dSf, dEf, b.par[7], b.par[8], b.status);
    checkNear(b.par[6], 5000.0, 1e-9, "[14] A stays at adcPerMeV / area = 5000");
    checkNear(f.ndf, b.nBins - 8, 0, "[14] ndf = bins - 8");
    check(f.dir == 0 && dSf <= 1.0 && dEf <= 1.0, Form("[14] end points within 1 mm (|dS| %.3f, |dE| %.3f), dir %d", dSf, dEf, f.dir));
  }


  // ---- [17] initial segment clipped to the sensitive volume ---------------------------------
  std::printf("[17] fpClipSegment\n");
  {
    FpGeom g = makeGeom(72, 92, 92, 0.25);  // z 0 .. 128 mm
    double a2[3], b2[3];
    int fl = -1;
    {  // out through y = 66 (the entry-73 type): cut exactly at y = 48, direction unchanged
      const double a[3] = {-4.0, -44.0, 30.0}, b[3] = {-50.0, 66.0, 56.0};
      const bool in = fpClipSegment(a, b, g, a2, b2, fl);
      const double u = (48.0 - a[1]) / (b[1] - a[1]);  // 92 / 110
      checkNear(b2[1], 48.0, 1e-9, "[17] y = 66 end cut at y = 48");
      checkNear(b2[0], a[0] + u * (b[0] - a[0]), 1e-9, "[17] x of the cut = start + u (b - a)");
      checkNear(b2[2], a[2] + u * (b[2] - a[2]), 1e-9, "[17] z of the cut");
      checkNear(a2[0], a[0], 1e-9, "[17] the start is not moved");
      const double d1[3] = {b2[0] - a2[0], b2[1] - a2[1], b2[2] - a2[2]}, d0[3] = {b[0] - a[0], b[1] - a[1], b[2] - a[2]};
      const double cx = d1[1] * d0[2] - d1[2] * d0[1], cy = d1[2] * d0[0] - d1[0] * d0[2], cz = d1[0] * d0[1] - d1[1] * d0[0];
      checkNear(std::fabs(cx) + std::fabs(cy) + std::fabs(cz), 0, 1e-9, "[17] direction unchanged (cross product 0)");
      check(in && fl == 2, Form("[17] inside, flags = %d (bit 1: the end moved)", fl));
    }
    {  // inside: unchanged
      const double a[3] = {-10, 5, 30}, b[3] = {20, -30, 60};
      const bool in = fpClipSegment(a, b, g, a2, b2, fl);
      double dd = 0;
      for (int j = 0; j < 3; ++j) dd += std::fabs(a2[j] - a[j]) + std::fabs(b2[j] - b[j]);
      check(in && fl == 0 && dd == 0, "[17] a segment inside the box: unchanged, flags 0");
    }
    {  // both ends out, the middle crosses
      const double a[3] = {-70, 0, 10}, b[3] = {70, 0, 10};
      const bool in = fpClipSegment(a, b, g, a2, b2, fl);
      check(in && fl == 3 && std::fabs(a2[0] + 53) < 1e-9 && std::fabs(b2[0] - 53) < 1e-9,
            Form("[17] both ends moved to x = -+53, flags %d", fl));
    }
    {  // z: below 0 / above 128 (= 512 x 0.25)
      const double a[3] = {0, 0, -20}, b[3] = {0, 0, 20};
      const bool in = fpClipSegment(a, b, g, a2, b2, fl);
      check(in && fl == 1 && std::fabs(a2[2]) < 1e-9, Form("[17] z < 0 start cut at z = 0, flags %d", fl));
      const double c[3] = {0, 0, 100}, e[3] = {0, 0, 150};
      const bool in2 = fpClipSegment(c, e, g, a2, b2, fl);
      check(in2 && fl == 2 && std::fabs(b2[2] - 128) < 1e-9, Form("[17] z > 128 end cut at z = 128, flags %d", fl));
    }
    {  // completely outside: x, y, z, and a corner miss
      const double a[3] = {60, 0, 30}, b[3] = {80, 10, 40};
      check(!fpClipSegment(a, b, g, a2, b2, fl) && fl == 4, "[17] x > 53 all along: skipped, flags 4 (bit 2)");
      const double c[3] = {0, 50, 30}, e[3] = {5, 70, 40};
      check(!fpClipSegment(c, e, g, a2, b2, fl) && fl == 4, "[17] y > 48 all along: skipped");
      const double h[3] = {0, 0, -30}, i[3] = {5, 5, -2};
      check(!fpClipSegment(h, i, g, a2, b2, fl) && fl == 4, "[17] z < 0 all along: skipped");
      const double j[3] = {-60, 60, 30}, kk[3] = {60, 100, 30};
      check(!fpClipSegment(j, kk, g, a2, b2, fl) && fl == 4, "[17] a segment missing the box: skipped");
    }
  }

  std::printf("%s: %d failure(s)\n", g_failures == 0 ? "GREEN" : "RED", g_failures);
  return g_failures;
}
