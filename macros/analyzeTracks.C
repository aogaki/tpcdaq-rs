#include <TBox.h>
#include <TCanvas.h>
#include <TFile.h>
#include <TH2D.h>
#include <TH3F.h>
#include <TPolyLine3D.h>
#include <TString.h>
#include <TTree.h>

#include <algorithm>
#include <array>
#include <cmath>
#include <iostream>
#include <vector>

// same semantics as minitpc fastqa (back-projection source imaging)
const double MIN_LENGTH_MM =
    50.0;  // quality cut: direction is meaningless below
const double EXTEND_MM =
    500.0;  // extend both ends: the source sits outside the cage
const int MAX_3D_TRACKS = 1000;  // cap the 3D overlay (drawing cost)
// Am entry region (fastqa): an endpoint here tags the track as a source alpha
const double ENTRY_X0 = -5.0, ENTRY_DX = 15.0, ENTRY_YMIN = 42.0;

TH2D *histTrackEnds = nullptr;
TH2D *histSource = nullptr;

// clip p(t) = a + t (b - a), t in [0,1], to an axis-aligned box;
// returns false when the segment misses the box entirely
bool ClipToBox(const double a[3], const double b[3], const double lo[3],
               const double hi[3], double out0[3], double out1[3])
{
  double t0 = 0.0, t1 = 1.0;
  for (int i = 0; i < 3; ++i) {
    const double d = b[i] - a[i];
    if (std::fabs(d) < 1e-12) {
      if (a[i] < lo[i] || a[i] > hi[i]) return false;
      continue;
    }
    double ta = (lo[i] - a[i]) / d, tb = (hi[i] - a[i]) / d;
    if (ta > tb) std::swap(ta, tb);
    t0 = std::max(t0, ta);
    t1 = std::min(t1, tb);
    if (t0 > t1) return false;
  }
  for (int i = 0; i < 3; ++i) {
    out0[i] = a[i] + t0 * (b[i] - a[i]);
    out1[i] = a[i] + t1 * (b[i] - a[i]);
  }
  return true;
}

void InitHists()
{
  if (!histTrackEnds) {
    histTrackEnds =
        new TH2D("histTrackEnds", "Track Ends", 200, -100, 100, 200, -100, 100);
    histTrackEnds->GetXaxis()->SetTitle("X [mm]");
    histTrackEnds->GetYaxis()->SetTitle("Y [mm]");
  }
  if (!histSource) {
    histSource =
        new TH2D("histSource", "Source", 160, -160, 160, 160, -160, 160);
    histSource->GetXaxis()->SetTitle("X [mm]");
    histSource->GetYaxis()->SetTitle("Y [mm]");
  }
}

void analyzeTracks(const char *trackFile)
{
  InitHists();

  // Implementation of the analyzeTracks function goes here.
  auto file = TFile::Open(trackFile);
  if (!file || file->IsZombie()) {
    std::cerr << "Failed to open file: " << trackFile << std::endl;
    return;
  }

  auto tree = (TTree *)file->Get("tracks");
  if (!tree) {
    std::cerr << "Failed to get TTree 'tracks' from file: " << trackFile
              << std::endl;
    return;
  }

  tree->SetBranchStatus("*", kFALSE);

  Float_t xStart, yStart, zStart;
  tree->SetBranchStatus("xStart", kTRUE);
  tree->SetBranchStatus("yStart", kTRUE);
  tree->SetBranchStatus("zStart", kTRUE);
  tree->SetBranchAddress("xStart", &xStart);
  tree->SetBranchAddress("yStart", &yStart);
  tree->SetBranchAddress("zStart", &zStart);

  Float_t xEnd, yEnd, zEnd;
  tree->SetBranchStatus("xEnd", kTRUE);
  tree->SetBranchStatus("yEnd", kTRUE);
  tree->SetBranchStatus("zEnd", kTRUE);
  tree->SetBranchAddress("xEnd", &xEnd);
  tree->SetBranchAddress("yEnd", &yEnd);
  tree->SetBranchAddress("zEnd", &zEnd);

  Int_t ok;
  Float_t length;
  tree->SetBranchStatus("ok", kTRUE);
  tree->SetBranchStatus("length", kTRUE);
  tree->SetBranchAddress("ok", &ok);
  tree->SetBranchAddress("length", &length);

  // 3D overlay buffer: (xs, ys, zs, xe, ye, ze, isSource)
  std::vector<std::array<float, 7>> segments;

  const auto nEvents = tree->GetEntries();
  for (auto iEvent = 0; iEvent < nEvents; ++iEvent) {
    tree->GetEntry(iEvent);
    if (!ok || length < MIN_LENGTH_MM) continue;

    const bool fromSource =
        (std::fabs(xStart - ENTRY_X0) < ENTRY_DX && yStart > ENTRY_YMIN) ||
        (std::fabs(xEnd - ENTRY_X0) < ENTRY_DX && yEnd > ENTRY_YMIN);
    if ((int)segments.size() < MAX_3D_TRACKS)
      segments.push_back(
          {xStart, yStart, zStart, xEnd, yEnd, zEnd, fromSource ? 1.f : 0.f});

    histTrackEnds->Fill(xStart, yStart);
    histTrackEnds->Fill(xEnd, yEnd);

    // arc-length parametrisation in xy (safe for vertical tracks)
    double ux = xEnd - xStart, uy = yEnd - yStart;
    const double n2d = std::hypot(ux, uy);
    if (n2d < 1.0) continue;  // track points along z: no xy back-projection
    ux /= n2d;
    uy /= n2d;

    // walk the extended segment in ~1 mm steps; deposit 1/npts per step so
    // every track contributes a total weight of 1 whatever its length/angle
    const double x0 = xStart - ux * EXTEND_MM, y0 = yStart - uy * EXTEND_MM;
    const double x1 = xEnd + ux * EXTEND_MM, y1 = yEnd + uy * EXTEND_MM;
    const int npts = (int)std::hypot(x1 - x0, y1 - y0);
    const double w = 1.0 / npts;
    for (int k = 0; k < npts; ++k) {
      const double lam = double(k) / (npts - 1);
      histSource->Fill(x0 + lam * (x1 - x0), y0 + lam * (y1 - y0), w);
    }
  }

  auto canvas = new TCanvas("canvas", "Track Analysis", 1200, 600);
  canvas->Divide(2, 1);
  canvas->cd(1);
  histTrackEnds->Draw("COLZ");
  canvas->cd(2);
  histSource->Draw("COLZ");
  auto cage = new TBox(-53, -58, 53, 48);  // active area, from fastqa
  cage->SetFillStyle(0);
  cage->SetLineColor(kRed);
  cage->Draw("same");
  canvas->Update();

  // 3D overlay: every selected track as a straight segment; rotate with the
  // mouse in an interactive session (source-alpha candidates in orange)
  auto canvas3d = new TCanvas("canvas3d", "3D Tracks", 800, 700);
  auto frame3d =
      new TH3F("frame3d",
               Form("3D tracks, first %d selected;X [mm];Y [mm];Z drift [mm]",
                    (int)segments.size()),
               1, -150, 150, 1, -150, 150, 1, 0, 150);
  frame3d->SetStats(0);
  frame3d->Draw();
  const double boxLo[3] = {-150, -150, 0}, boxHi[3] = {150, 150, 150};
  for (const auto &t : segments) {
    // extension along the full 3D direction, clipped to the frame box
    // (dashed and fainter, drawn under the measured segment)
    double dx = t[3] - t[0], dy = t[4] - t[1], dz = t[5] - t[2];
    const double n3d = std::sqrt(dx * dx + dy * dy + dz * dz);
    if (n3d > 1e-6) {
      dx /= n3d;
      dy /= n3d;
      dz /= n3d;
      const double a[3] = {t[0] - dx * EXTEND_MM, t[1] - dy * EXTEND_MM,
                           t[2] - dz * EXTEND_MM};
      const double b[3] = {t[3] + dx * EXTEND_MM, t[4] + dy * EXTEND_MM,
                           t[5] + dz * EXTEND_MM};
      double c0[3], c1[3];
      if (ClipToBox(a, b, boxLo, boxHi, c0, c1)) {
        auto ext = new TPolyLine3D(2);
        ext->SetPoint(0, c0[0], c0[1], c0[2]);
        ext->SetPoint(1, c1[0], c1[1], c1[2]);
        ext->SetLineStyle(2);
        ext->SetLineColorAlpha(t[6] > 0 ? kOrange + 7 : kAzure + 1,
                               t[6] > 0 ? 0.30 : 0.12);
        ext->Draw("same");
      }
    }

    auto line = new TPolyLine3D(2);
    line->SetPoint(0, t[0], t[1], t[2]);
    line->SetPoint(1, t[3], t[4], t[5]);
    if (t[6] > 0) {
      line->SetLineColorAlpha(kOrange + 7, 0.60);
      line->SetLineWidth(2);
    } else {
      line->SetLineColorAlpha(kAzure + 1, 0.25);
    }
    line->Draw("same");
  }
  canvas3d->Update();
}
