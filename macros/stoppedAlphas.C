#include <TCanvas.h>
#include <TF1.h>
#include <TFile.h>
#include <TH1D.h>
#include <TH2D.h>
#include <TParameter.h>
#include <TProfile.h>
#include <TString.h>
#include <TTree.h>

#include <cmath>
#include <iostream>

// Alphas that STOP inside the cage: residual energy at the entrance wall from
// the residual range (entrance wall -> stop point, 3D) with the NIST ASTAR
// projected range of alphas in CO2 (matno 134, https://physics.nist.gov/PhysRefData/Star/Text/ASTAR.html)
// scaled to the gas density of the run.
//
// Selection (tracks tree of makeTracks.C, re-oriented to +x):
//   - source-like: |phi| <= PHI_MAX_DEG
//   - starts at the left wall:  xStart < CAGE_XLO + EDGE_MM
//   - stops inside: xEnd < CAGE_XHI - EDGE_MM, yEnd inside the y walls by
//     EDGE_MM, zEnd inside the drift window by Z_EDGE_MM (self-trigger: the
//     window clips tracks in z, and a clipped end is NOT a stop)
//   - length >= MIN_LENGTH_MM (short, so a stopping 216Po is not thrown away)
// No Bragg-peak test yet: a track leaving through a strip gap or a dead region
// would be counted as stopped.
//
// Fourth pad: energy AT EMISSION assuming the alpha started at the source
// x = srcX (2nd argument): path to the wall L = (CAGE_XLO - srcX) / dir_x (no z
// needed), total range = L + residual range -> ASTAR. A real source alpha
// gives a line (228Th chain: 6.05 / 6.29 / 6.78 / 8.78 MeV); an alpha born in
// the gas on the way gives a continuum. Feed it the *_src.root of
// sourceTracks.C to test whether the direction filter isolates the source.
//
// Stop point = the outermost hit of the track (branches xEndF etc. of
// makeTracks.C), not the 2%-98% charge quantile: the quantile trims 1-4 mm
// (more for a longer track), which biases a range measurement. Direction
// and the wall crossing still come from the fitted line.
// After the histograms, every 228Th line predicted inside the cage is fitted
// in the stop-point histogram and, with two or more lines, the gas density
// and the source distance are solved from  x_i = CAGE_XLO + M_i / rho - d
// (M_i = ASTAR mass range): the line spacing gives rho (a thermometer when
// the pressure is known), the absolute position gives d.
//
// Usage:  root -l 'stoppedAlphas.C("CoBo_xxx_tracks.root", -370, 100)'
//         ALL arguments are required (no defaults: a silent default pressure
//         gave a wrong analysis once, TODO/168).
//         srcX: source x [mm] (current layout about -370; -470 was the
//         2026-09-25 run, about -386 2026-09-29)
//         pressureMbar: run pressure [mbar]; it sets the gas density.
//         Use the same value as for analyzeUVW.C (a difference
//         of 0.5 mbar or more from the tracks file's pressure_mbar warns).
//         The z window comes from the tracks file (z_window_mm, TODO/178).

double PRESSURE_MBAR = 100.0;  // run pressure, set from the 3rd argument
const double TEMP_K = 293.15;
const double PHI_MAX_DEG = 15.0;
const double CAGE_XLO = -53.0, CAGE_XHI = 53.0;  // from fastqa
const double CAGE_YLO = -58.0, CAGE_YHI = 48.0;
const double Z_LO = 0.0;  // the upper end comes from the tracks file (z_window_mm)
const double EDGE_MM = 10.0, Z_EDGE_MM = 10.0;
// a track whose outermost hit is within this of the exit wall is taken as
// leaving the cage (the last strip of a crossing track sits at x = 51-52)
const double EXIT_EDGE_MM = 5.0;
const double MIN_LENGTH_MM = 15.0;
// a line depositing less than this at the wall is cut into by the trigger
// threshold (~0.33 MeV, TODO/100): its peak is biased deeper, so it is
// shown but not used in the density fit
const double E_WALL_MIN_FIT = 0.6;

// NIST ASTAR, alpha in CO2: kinetic energy [MeV], PROJECTED range [g/cm2]
// (what a stop point along the track measures; 0.7-1 % below the CSDA range
// above 3 MeV, the detour factor)
const double ASTAR[][2] = {
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
const int N_ASTAR = sizeof(ASTAR) / sizeof(ASTAR[0]);

// CO2 mass density [g/cm3] of the run (ideal gas, M = 44.01 g/mol)
double gasDensity()
{
  return 44.01 * PRESSURE_MBAR * 100.0 / (8.314 * TEMP_K) * 1e-6;
}

// projected range [mm] of an alpha of energy E [MeV] (log-log interpolation)
double rangeFromEnergy(double E)
{
  for (int i = 1; i < N_ASTAR; ++i)
    if (E <= ASTAR[i][0]) {
      const double f = std::log(E / ASTAR[i - 1][0]) /
                       std::log(ASTAR[i][0] / ASTAR[i - 1][0]);
      return ASTAR[i - 1][1] * std::pow(ASTAR[i][1] / ASTAR[i - 1][1], f) /
             gasDensity() * 10.0;
    }
  return -1;
}

// energy [MeV] whose projected range equals r [mm] in CO2 at the run density
double energyFromRange(double rMm)
{
  const double r = rMm / 10.0 * gasDensity();  // g/cm2
  if (r <= ASTAR[0][1])                        // below 0.1 MeV: E ~ sqrt(R)
    return ASTAR[0][0] * std::sqrt(r / ASTAR[0][1]);
  for (int i = 1; i < N_ASTAR; ++i)
    if (r <= ASTAR[i][1]) {  // log-log interpolation between rows i-1 and i
      const double f = std::log(r / ASTAR[i - 1][1]) /
                       std::log(ASTAR[i][1] / ASTAR[i - 1][1]);
      return ASTAR[i - 1][0] * std::pow(ASTAR[i][0] / ASTAR[i - 1][0], f);
    }
  return -1;  // beyond the table (10 MeV)
}

void stoppedAlphas(const char *trackFile, double srcX, double pressureMbar)
{
  PRESSURE_MBAR = pressureMbar;
  auto file = TFile::Open(trackFile);
  if (!file || file->IsZombie()) {
    std::cerr << "Failed to open file: " << trackFile << std::endl;
    return;
  }
  // drift window = 512 cells x mm/cell, recorded by analyzeUVW.C (TODO/178)
  auto zPar = (TParameter<double> *)file->Get("z_window_mm");
  if (!zPar) {
    std::cerr << "ERROR: no z_window_mm in " << trackFile
              << " - old tracks file. Re-run analyzeUVW -> makeTracks."
              << std::endl;
    return;
  }
  const double zHi = zPar->GetVal();
  auto pPar = (TParameter<double> *)file->Get("pressure_mbar");
  if (pPar && std::fabs(pPar->GetVal() - pressureMbar) >= 0.5)
    std::cout << "WARNING: pressure argument " << pressureMbar
              << " mbar differs from the tracks file's " << pPar->GetVal()
              << " mbar; the gas density uses the argument" << std::endl;
  auto tree = (TTree *)file->Get("tracks");
  if (!tree) {
    std::cerr << "No TTree 'tracks' in " << trackFile << std::endl;
    return;
  }
  Float_t xs, ys, zs, xe, ye, ze, length, charge;
  Int_t ok;
  if (!tree->GetBranch("xEndF")) {
    std::cerr << "No xEndF branch: re-run makeTracks.C (TODO/100)" << std::endl;
    return;
  }
  tree->SetBranchAddress("xStartF", &xs);
  tree->SetBranchAddress("yStartF", &ys);
  tree->SetBranchAddress("zStartF", &zs);
  tree->SetBranchAddress("xEndF", &xe);
  tree->SetBranchAddress("yEndF", &ye);
  tree->SetBranchAddress("zEndF", &ze);
  tree->SetBranchAddress("length", &length);
  tree->SetBranchAddress("charge", &charge);
  tree->SetBranchAddress("ok", &ok);

  auto hE = new TH1D(
      "hE",
      "stopped inside: residual energy at the entrance wall;E [MeV];tracks", 80,
      0, 4);
  auto hXend =
      new TH1D("hXend", "stopped inside: stop point;x_{end} [mm];tracks", 72,
               -54, 54);  // 1.5 mm = strip pitch
  auto hQvsE = new TH2D(
      "hQvsE",
      "stopped inside: track charge vs E;E [MeV];charge (3 views) [ADC]", 80, 0,
      4, 100, 0, 1.5e6);
  auto pQvsE = new TProfile("pQvsE", "", 40, 0, 4);
  auto hEgen = new TH1D(
      "hEgen",
      Form("energy at emission if born at the source (x = %.0f);E [MeV];tracks",
           srcX),
      600, 4, 10);

  Long64_t nStop = 0;
  std::cout
      << Form(
             "CO2 density %.4f mg/cm3 (%.0f mbar, %.1f K); ASTAR projected "
             "ranges [mm]: 212Bi 6.05 -> %.0f, 220Rn 6.29 -> %.0f, "
             "216Po 6.78 -> %.0f, 212Po 8.78 -> %.0f",
             gasDensity() * 1e3, PRESSURE_MBAR, TEMP_K, rangeFromEnergy(6.05),
             rangeFromEnergy(6.29), rangeFromEnergy(6.78),
             rangeFromEnergy(8.78))
      << std::endl;
  const auto n = tree->GetEntries();
  for (Long64_t i = 0; i < n; ++i) {
    tree->GetEntry(i);
    if (!ok || length < MIN_LENGTH_MM) continue;
    double x0 = xs, y0 = ys, z0 = zs, x1 = xe, y1 = ye, z1 = ze;
    if (x1 < x0) {
      std::swap(x0, x1);
      std::swap(y0, y1);
      std::swap(z0, z1);
    }
    const double dx = x1 - x0, dy = y1 - y0, dz = z1 - z0;
    if (dx <= 0) continue;
    if (std::fabs(std::atan2(dy, dx)) * 180.0 / M_PI > PHI_MAX_DEG) continue;
    if (x0 >= CAGE_XLO + EDGE_MM) continue;       // must enter at the wall
    if (x1 >= CAGE_XHI - EXIT_EDGE_MM) continue;  // exits through the far wall
    if (y1 <= CAGE_YLO + EDGE_MM || y1 >= CAGE_YHI - EDGE_MM) continue;
    if (z1 <= Z_LO + Z_EDGE_MM || z1 >= zHi - Z_EDGE_MM) continue;
    // residual range: wall crossing point (on the line) -> stop point
    const double t = (CAGE_XLO - x0) / dx;
    const double wx = CAGE_XLO, wy = y0 + t * dy, wz = z0 + t * dz;
    const double r = std::sqrt((x1 - wx) * (x1 - wx) + (y1 - wy) * (y1 - wy) +
                               (z1 - wz) * (z1 - wz));
    const double E = energyFromRange(r);
    if (E < 0) continue;
    ++nStop;
    hE->Fill(E);
    hXend->Fill(x1);
    hQvsE->Fill(E, charge);
    pQvsE->Fill(E, charge);
    const double dirx = dx / std::sqrt(dx * dx + dy * dy + dz * dz);
    hEgen->Fill(energyFromRange(r + (CAGE_XLO - srcX) / dirx));
  }
  std::cout << "stopped inside (source-like, |phi| <= " << PHI_MAX_DEG
            << " deg): " << nStop << " tracks" << std::endl;

  auto canvas = new TCanvas("canvasStopped", "Stopped Alphas", 1200, 900);
  canvas->Divide(2, 2);
  canvas->cd(1);
  hE->Draw();
  canvas->cd(2);
  hXend->Draw();
  canvas->cd(3);
  hQvsE->SetStats(0);
  hQvsE->Draw("COLZ");
  pQvsE->SetMarkerStyle(20);
  pQvsE->SetMarkerSize(0.6);
  pQvsE->Draw("same");
  canvas->cd(4);
  hEgen->Draw();
  canvas->Update();

  // --- lines inside the cage: fit, then solve density and source distance
  struct Line {
    const char *name;
    double E;     // MeV (212Bi: intensity-weighted doublet 6.051 / 6.090)
    double mass;  // ASTAR mass range [g/cm2]
    double xPred, xFit, xErr, sigma;
    int n;
    bool found;
    TF1 *fn;
  };
  Line lines[] = {{"212Bi", 6.062, 0, 0, 0, 0, 0, 0, false, nullptr},
                  {"220Rn", 6.288, 0, 0, 0, 0, 0, 0, false, nullptr},
                  {"216Po", 6.778, 0, 0, 0, 0, 0, 0, false, nullptr},
                  {"212Po", 8.785, 0, 0, 0, 0, 0, 0, false, nullptr}};
  const double dNominal = CAGE_XLO - srcX;
  int nFound = 0;
  for (auto &L : lines) {
    L.mass = rangeFromEnergy(L.E) / 10.0 * gasDensity();
    L.xPred = CAGE_XLO + rangeFromEnergy(L.E) - dNominal;
    if (L.xPred < CAGE_XLO + EDGE_MM + 12 ||
        L.xPred > CAGE_XHI - EXIT_EDGE_MM - 4)
      continue;
    const double w = 9.0;  // fit window: +-9 mm around the prediction,
    const double hiW =
        std::min(L.xPred + w, CAGE_XHI - EXIT_EDGE_MM);  // clipped
    auto g = new TF1(Form("g_%s", L.name), "gaus", L.xPred - w, hiW);
    g->SetParameters(hXend->GetBinContent(hXend->FindBin(L.xPred)), L.xPred,
                     3.5);
    hXend->Fit(g, "QR0+");  // "0": do not redraw on the current pad
    L.fn = g;
    L.xFit = g->GetParameter(1);
    L.xErr = g->GetParError(1);
    L.sigma = g->GetParameter(2);
    L.n = (int)hXend->Integral(hXend->FindBin(L.xFit - 2.5 * L.sigma),
                               hXend->FindBin(L.xFit + 2.5 * L.sigma));
    const double eWall = energyFromRange(L.xPred - CAGE_XLO);
    L.found = L.n > 20 && std::fabs(L.xFit - L.xPred) < w && L.sigma < 8 &&
              eWall >= E_WALL_MIN_FIT;
    std::cout << Form(
                     "%s %.3f MeV: predicted x = %+6.1f (%.2f MeV at the "
                     "wall), fitted %+6.1f +- %.1f (sigma %.1f, n = %d)%s",
                     L.name, L.E, L.xPred, eWall, L.xFit, L.xErr, L.sigma, L.n,
                     L.found ? ""
                             : (eWall < E_WALL_MIN_FIT
                                    ? "  [trigger-truncated, not used]"
                                    : "  [not used]"))
              << std::endl;
    if (L.found) ++nFound;
  }
  canvas->cd(2);
  for (auto &L : lines)
    if (L.fn) {
      L.fn->SetLineColor(L.found ? kRed : kGray + 2);
      L.fn->Draw("same");
    }
  canvas->Update();
  if (nFound < 2) {
    std::cout << "fewer than two lines inside: no density fit" << std::endl;
    return;
  }
  // least squares for (1/rho, d):  x_i - CAGE_XLO = M_i * (1/rho) - d
  double Saa = 0, Sa = 0, S1 = 0, Say = 0, Sy = 0;
  for (auto &L : lines) {
    if (!L.found) continue;
    const double wgt = 1.0 / (L.xErr * L.xErr + 1.0);       // +1 mm2 systematic
    const double a = L.mass * 10.0, y = L.xFit - CAGE_XLO;  // mm g/cm3 units
    Saa += wgt * a * a;
    Sa += wgt * a;
    S1 += wgt;
    Say += wgt * a * y;
    Sy += wgt * y;
  }
  const double det = Saa * S1 - Sa * Sa;
  const double invRho = (Say * S1 - Sa * Sy) / det;  // cm3/g
  const double d = -(Saa * Sy - Sa * Say) / det;
  const double rho = 1.0 / invRho;  // g/cm3
  const double rhoN = gasDensity();
  const double Tfit = TEMP_K * rhoN / rho;  // if the pressure is right
  const double pFit =
      PRESSURE_MBAR * rho / rhoN;  // if the temperature is right
  std::cout << Form(
                   "density fit (%d lines): rho = %.4f mg/cm3 = nominal x %.3f "
                   "-> T = %.1f K at %.0f mbar, or p = %.1f mbar at %.1f K; "
                   "source distance d = %.0f mm (x_src = %.0f, nominal %.0f)",
                   nFound, rho * 1e3, rho / rhoN, Tfit, PRESSURE_MBAR, pFit,
                   TEMP_K, d, CAGE_XLO - d, srcX)
            << std::endl;
  for (auto &L : lines)
    if (L.found || L.n > 0)
      std::cout << Form("  %s residual: %+.1f mm", L.name,
                        L.xFit - (CAGE_XLO + L.mass * 10.0 * invRho - d))
                << std::endl;
}
