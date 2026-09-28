#include <TFile.h>
#include <TString.h>
#include <TTree.h>

#include <cmath>
#include <iostream>

// Event filter: keep only the tracks that plausibly came from the source.
//   - re-oriented to point +x (the source sits at -x; makeTracks' Bragg-based
//     start/end is a coin flip for crossing alphas)
//   - roughly parallel to x:            |phi| <= PHI_MAX_DEG
//   - the extended line passes the source: |y at x = SRC_X - SRC_Y| <= SRC_R_MM
// Writes <input>_src.root with the same "tracks" tree (start = -x end), so
// analyzeTracks.C / analyzeAngles.C run on it unchanged.
//
// Usage:  root -l -b -q 'sourceTracks.C("CoBo_xxx_tracks.root")'

const double SRC_X = -470.0;      // source x [mm] (09-25 setup; the xy fit gives -410, biased
                                  // closer by the Rn cloud + port aperture, see TODO/095)
const double SRC_Y = 0.0;         // source y [mm]
const double SRC_R_MM = 30.0;     // accepted miss distance at the source
const double PHI_MAX_DEG = 15.0;  // |in-plane angle| w.r.t. +x
const double MIN_LENGTH_MM = 50.0;

void sourceTracks(const char *trackFile)
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
  Float_t xs, ys, zs, xe, ye, ze, dx, dy, dz, length;
  Int_t ok;
  tree->SetBranchAddress("xStart", &xs);
  tree->SetBranchAddress("yStart", &ys);
  tree->SetBranchAddress("zStart", &zs);
  tree->SetBranchAddress("xEnd", &xe);
  tree->SetBranchAddress("yEnd", &ye);
  tree->SetBranchAddress("zEnd", &ze);
  tree->SetBranchAddress("dirX", &dx);
  tree->SetBranchAddress("dirY", &dy);
  tree->SetBranchAddress("dirZ", &dz);
  tree->SetBranchAddress("length", &length);
  tree->SetBranchAddress("ok", &ok);

  TString outName(trackFile);
  outName.ReplaceAll("_tracks.root", "_src.root");
  TFile out(outName, "RECREATE");
  auto otree = tree->CloneTree(0);  // same branches, filled selectively

  Long64_t nIn = 0, nKept = 0;
  const auto n = tree->GetEntries();
  for (Long64_t i = 0; i < n; ++i) {
    tree->GetEntry(i);
    if (!ok || length < MIN_LENGTH_MM) continue;
    ++nIn;
    if (xe < xs) {  // orient toward +x
      std::swap(xs, xe);
      std::swap(ys, ye);
      std::swap(zs, ze);
      dx = -dx;
      dy = -dy;
      dz = -dz;
    }
    const double ddx = xe - xs, ddy = ye - ys;
    if (ddx <= 0) continue;
    const double phi = std::atan2(ddy, ddx) * 180.0 / M_PI;
    if (std::fabs(phi) > PHI_MAX_DEG) continue;
    const double yAtSrc = ys + (SRC_X - xs) * (ddy / ddx);
    if (std::fabs(yAtSrc - SRC_Y) > SRC_R_MM) continue;
    ++nKept;
    otree->Fill();
  }
  otree->Write();
  std::cout << "kept " << nKept << " / " << nIn << " tracks (|phi| <= "
            << PHI_MAX_DEG << " deg, |y - " << SRC_Y << "| <= " << SRC_R_MM
            << " mm at x = " << SRC_X << ") -> " << outName << std::endl;
  out.Close();
}
