#include <TCanvas.h>
#include <TF1.h>
#include <TFile.h>
#include <TH1D.h>
#include <TH2D.h>
#include <TLegend.h>
#include <TProfile.h>
#include <TString.h>
#include <TTree.h>

#include <cmath>
#include <iostream>

// Track angle distributions. Every track is re-oriented to point +x (the
// source sits at -x), so the Bragg-based start/end of makeTracks is ignored.
//   phi   = atan2(dy, dx)          in-plane angle, 0 = along +x   [deg]
//   theta = atan2(dz, |dxy|)       dip angle w.r.t. the pad plane  [deg]
// "through" = both ends within EDGE_MM of the cage x walls (left -> right).
// Third pad: tan(phi) vs entrance y at the left wall; for a point source
// tan(phi) = (y - ySrc) / (xWall - xSrc), so a straight-line fit gives the
// source distance D = 1/slope and ySrc = -intercept/slope. Angular noise does
// not bias the slope (unlike the back-projection waist in analyzeTracks).

const double MIN_LENGTH_MM = 50.0;
const double CAGE_XLO = -53.0, CAGE_XHI = 53.0;  // active area, from fastqa
const double EDGE_MM = 10.0;                     // "touches the wall" margin

void analyzeAngles(const char *trackFile)
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

  Float_t xStart, yStart, zStart, xEnd, yEnd, zEnd, length;
  Int_t ok;
  tree->SetBranchStatus("*", kFALSE);
  for (const char *b : {"xStart", "yStart", "zStart", "xEnd", "yEnd", "zEnd",
                        "length", "ok"})
    tree->SetBranchStatus(b, kTRUE);
  tree->SetBranchAddress("xStart", &xStart);
  tree->SetBranchAddress("yStart", &yStart);
  tree->SetBranchAddress("zStart", &zStart);
  tree->SetBranchAddress("xEnd", &xEnd);
  tree->SetBranchAddress("yEnd", &yEnd);
  tree->SetBranchAddress("zEnd", &zEnd);
  tree->SetBranchAddress("length", &length);
  tree->SetBranchAddress("ok", &ok);

  auto hPhiAll = new TH1D("hPhiAll", "In-plane angle #phi;#phi [deg];tracks",
                          180, -90, 90);
  auto hPhiThr = new TH1D("hPhiThr", "through", 180, -90, 90);
  auto hThetaAll = new TH1D(
      "hThetaAll", "Dip angle #theta;#theta [deg];tracks", 180, -90, 90);
  auto hThetaThr = new TH1D("hThetaThr", "through", 180, -90, 90);
  auto hPhiVsY = new TH2D(
      "hPhiVsY", "through: tan#phi vs entrance y;y at x = -53 [mm];tan#phi",
      60, -60, 60, 100, -0.5, 0.5);
  auto pPhiVsY = new TProfile("pPhiVsY", "", 60, -60, 60);
  auto hThetaVsPhi = new TH2D(
      "hThetaVsPhi", "all: #theta vs #phi;#phi [deg];#theta [deg]", 120, -60,
      60, 120, -60, 60);

  Long64_t nAll = 0, nThr = 0;
  const auto nEvents = tree->GetEntries();
  for (Long64_t i = 0; i < nEvents; ++i) {
    tree->GetEntry(i);
    if (!ok || length < MIN_LENGTH_MM) continue;

    double dx = xEnd - xStart, dy = yEnd - yStart, dz = zEnd - zStart;
    if (dx < 0) {  // orient toward +x
      dx = -dx;
      dy = -dy;
      dz = -dz;
    }
    const double dxy = std::hypot(dx, dy);
    if (dxy < 1.0) continue;  // along z: no in-plane angle
    const double phi = std::atan2(dy, dx) * 180.0 / M_PI;
    const double theta = std::atan2(dz, dxy) * 180.0 / M_PI;

    const double xLo = std::min(xStart, xEnd), xHi = std::max(xStart, xEnd);
    const bool through = xLo < CAGE_XLO + EDGE_MM && xHi > CAGE_XHI - EDGE_MM;

    ++nAll;
    hPhiAll->Fill(phi);
    hThetaAll->Fill(theta);
    hThetaVsPhi->Fill(phi, theta);
    if (!through) continue;
    ++nThr;
    hPhiThr->Fill(phi);
    hThetaThr->Fill(theta);
    // entrance y at the left wall, along the fitted line
    const double yEntry = (xStart < xEnd ? yStart : yEnd) +
                          (CAGE_XLO - xLo) * (dy / dx);
    const double tanPhi = dy / dx;
    hPhiVsY->Fill(yEntry, tanPhi);
    pPhiVsY->Fill(yEntry, tanPhi);
  }
  std::cout << "selected " << nAll << " tracks (length >= " << MIN_LENGTH_MM
            << " mm), through (left -> right) " << nThr << std::endl;

  auto canvas = new TCanvas("canvasAngles", "Track Angles", 1200, 900);
  canvas->Divide(2, 2);

  canvas->cd(1);
  hPhiAll->SetLineColor(kAzure + 1);
  hPhiThr->SetLineColor(kOrange + 7);
  hPhiAll->Draw();
  hPhiThr->Draw("same");
  auto leg1 = new TLegend(0.62, 0.72, 0.88, 0.86);
  leg1->AddEntry(hPhiAll, "all", "l");
  leg1->AddEntry(hPhiThr, "through", "l");
  leg1->Draw();

  canvas->cd(2);
  hThetaAll->SetLineColor(kAzure + 1);
  hThetaThr->SetLineColor(kOrange + 7);
  hThetaAll->Draw();
  hThetaThr->Draw("same");

  canvas->cd(3);
  hThetaVsPhi->SetStats(0);
  hThetaVsPhi->Draw("COLZ");

  canvas->cd(4);
  hPhiVsY->SetStats(0);
  hPhiVsY->Draw("COLZ");
  pPhiVsY->SetLineColor(kBlack);
  pPhiVsY->SetMarkerStyle(20);
  pPhiVsY->SetMarkerSize(0.6);
  pPhiVsY->Draw("same");
  auto fit = new TF1("fitPhiVsY", "[0] + [1]*x", -45, 45);
  fit->SetLineColor(kRed);
  pPhiVsY->Fit(fit, "QR");
  const double a = fit->GetParameter(0), b = fit->GetParameter(1);
  const double D = 1.0 / b, ySrc = -a / b;
  const double Derr = fit->GetParError(1) / (b * b);
  std::cout << Form("tan(phi) = %.4f + %.5f * y  ->  source at x = %.0f "
                    "(D = %.0f +- %.0f mm from the left wall), y = %.1f mm",
                    a, b, CAGE_XLO - D, D, Derr, ySrc)
            << std::endl;
  canvas->Update();
}
