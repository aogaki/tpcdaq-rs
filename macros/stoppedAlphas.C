#include <TCanvas.h>
#include <TFile.h>
#include <TH1D.h>
#include <TH2D.h>
#include <TProfile.h>
#include <TString.h>
#include <TTree.h>

#include <cmath>
#include <iostream>

// Alphas that STOP inside the cage: residual energy at the entrance wall from
// the residual range (entrance wall -> stop point, 3D) with the NIST ASTAR
// CSDA range of alphas in CO2 (matno 134, https://physics.nist.gov/PhysRefData/Star/Text/ASTAR.html)
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
// (SRC_X, SRC_Y): path to the wall L = (CAGE_XLO - SRC_X) / dir_x (no z
// needed), total range = L + residual range -> ASTAR. A real source alpha
// gives a line (228Th chain: 6.05 / 6.29 / 6.78 / 8.78 MeV); an alpha born in
// the gas on the way gives a continuum. Feed it the *_src.root of
// sourceTracks.C to test whether the direction filter isolates the source.
//
// Usage:  root -l 'stoppedAlphas.C("CoBo_xxx_tracks.root")'

const double PRESSURE_MBAR = 100.0;
const double TEMP_K = 293.15;
const double PHI_MAX_DEG = 15.0;
const double CAGE_XLO = -53.0, CAGE_XHI = 53.0;  // from fastqa
const double CAGE_YLO = -58.0, CAGE_YHI = 48.0;
const double Z_LO = 0.0, Z_HI = 89.0;  // 512 cells x mm/cell (analyzeUVW)
const double EDGE_MM = 10.0, Z_EDGE_MM = 10.0;
const double MIN_LENGTH_MM = 15.0;
const double SRC_X = -470.0, SRC_Y = 0.0;  // source position (09-25 setup)

// NIST ASTAR, alpha in CO2: kinetic energy [MeV], CSDA range [g/cm2]
const double ASTAR[][2] = {
    {5.000E-01, 4.460E-04},
    {5.500E-01, 4.721E-04},
    {6.000E-01, 4.977E-04},
    {6.500E-01, 5.231E-04},
    {7.000E-01, 5.483E-04},
    {7.500E-01, 5.735E-04},
    {8.000E-01, 5.988E-04},
    {8.500E-01, 6.243E-04},
    {9.000E-01, 6.500E-04},
    {9.500E-01, 6.760E-04},
    {1.000E+00, 7.023E-04},
    {1.250E+00, 8.395E-04},
    {1.500E+00, 9.880E-04},
    {1.750E+00, 1.149E-03},
    {2.000E+00, 1.323E-03},
    {2.250E+00, 1.510E-03},
    {2.500E+00, 1.711E-03},
    {2.750E+00, 1.924E-03},
    {3.000E+00, 2.149E-03},
    {3.500E+00, 2.638E-03},
    {4.000E+00, 3.176E-03},
    {4.500E+00, 3.761E-03},
    {5.000E+00, 4.392E-03},
    {5.500E+00, 5.068E-03},
    {6.000E+00, 5.789E-03},
    {6.500E+00, 6.554E-03},
    {7.000E+00, 7.362E-03},
    {7.500E+00, 8.212E-03},
    {8.000E+00, 9.104E-03},
    {8.500E+00, 1.004E-02},
    {9.000E+00, 1.101E-02},
    {9.500E+00, 1.203E-02},
    {1.000E+01, 1.308E-02},};
const int N_ASTAR = sizeof(ASTAR) / sizeof(ASTAR[0]);

// CO2 mass density [g/cm3] of the run (ideal gas, M = 44.01 g/mol)
double gasDensity()
{
  return 44.01 * PRESSURE_MBAR * 100.0 / (8.314 * TEMP_K) * 1e-6;
}

// CSDA range [mm] of an alpha of energy E [MeV] (log-log interpolation)
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

// energy [MeV] whose CSDA range equals r [mm] in CO2 at the run density
double energyFromRange(double rMm)
{
  const double r = rMm / 10.0 * gasDensity();  // g/cm2
  if (r <= ASTAR[0][1]) return ASTAR[0][0] * std::sqrt(r / ASTAR[0][1]);
  for (int i = 1; i < N_ASTAR; ++i)
    if (r <= ASTAR[i][1]) {  // log-log interpolation between rows i-1 and i
      const double f = std::log(r / ASTAR[i - 1][1]) /
                       std::log(ASTAR[i][1] / ASTAR[i - 1][1]);
      return ASTAR[i - 1][0] * std::pow(ASTAR[i][0] / ASTAR[i - 1][0], f);
    }
  return -1;  // beyond the table (10 MeV)
}

void stoppedAlphas(const char *trackFile)
{
  auto file = TFile::Open(trackFile);
  if (!file || file->IsZombie()) {
    std::cerr << "Failed to open file: " << trackFile << std::endl;
    return;
  }
  auto tree = (TTree *)file->Get("tracks");
  if (!tree) {
    std::cerr << "No TTree 'tracks' in " << trackFile << std::endl;
    return;
  }
  Float_t xs, ys, zs, xe, ye, ze, length, charge;
  Int_t ok;
  tree->SetBranchAddress("xStart", &xs);
  tree->SetBranchAddress("yStart", &ys);
  tree->SetBranchAddress("zStart", &zs);
  tree->SetBranchAddress("xEnd", &xe);
  tree->SetBranchAddress("yEnd", &ye);
  tree->SetBranchAddress("zEnd", &ze);
  tree->SetBranchAddress("length", &length);
  tree->SetBranchAddress("charge", &charge);
  tree->SetBranchAddress("ok", &ok);

  auto hE = new TH1D("hE", "stopped inside: residual energy at the entrance wall;E [MeV];tracks", 60, 0, 3);
  auto hXend = new TH1D("hXend", "stopped inside: stop point;x_{end} [mm];tracks", 72, -54, 54);  // 1.5 mm = strip pitch
  auto hQvsE = new TH2D("hQvsE", "stopped inside: track charge vs E;E [MeV];charge (3 views) [ADC]", 60, 0, 3, 100, 0, 1.5e6);
  auto pQvsE = new TProfile("pQvsE", "", 30, 0, 3);
  auto hEgen = new TH1D(
      "hEgen",
      Form("energy at emission if born at the source (x = %.0f);E [MeV];tracks", SRC_X),
      100, 4, 10);

  Long64_t nStop = 0;
  std::cout << Form("CO2 density %.4f mg/cm3 (%.0f mbar, %.1f K); ASTAR CSDA "
                    "ranges [mm]: 212Bi 6.05 -> %.0f, 220Rn 6.29 -> %.0f, "
                    "216Po 6.78 -> %.0f, 212Po 8.78 -> %.0f",
                    gasDensity() * 1e3, PRESSURE_MBAR, TEMP_K,
                    rangeFromEnergy(6.05), rangeFromEnergy(6.29),
                    rangeFromEnergy(6.78), rangeFromEnergy(8.78))
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
    if (x0 >= CAGE_XLO + EDGE_MM) continue;             // must enter at the wall
    if (x1 >= CAGE_XHI - EDGE_MM) continue;             // exits through the far wall
    if (y1 <= CAGE_YLO + EDGE_MM || y1 >= CAGE_YHI - EDGE_MM) continue;
    if (z1 <= Z_LO + Z_EDGE_MM || z1 >= Z_HI - Z_EDGE_MM) continue;
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
    hEgen->Fill(energyFromRange(r + (CAGE_XLO - SRC_X) / dirx));
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
}
