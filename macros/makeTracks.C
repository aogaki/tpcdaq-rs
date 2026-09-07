// =============================================================================
// makeTracks.C - step 3: build 3D tracks from the UVW slice centroids.
//
// Reads the "uvw" tree from analyzeUVW.C. Every event is treated as ONE
// straight alpha track (mini-eTPC assumption):
//   1. for each drift slice, combine the U/V/W charge centroids into an (x,y)
//      point:  s_view = pitch_view . (x,y)   with
//        pitch_U = (-1, 0)   pitch_V = (0.5, +sqrt3/2)   pitch_W = (0.5, -sqrt3/2)
//   2. charge-weighted 3D PCA of the slice points -> line direction
//   3. endpoints = 2%-98% charge quantiles along the line
//   4. head/tail: the charge-heavier half is called the END.
//      WARNING: alphas cross the detector without stopping, so there is no
//      Bragg peak and this sign is basically a coin flip - use the geometry
//      of your source to orient tracks in analysis, or use |direction| only.
//
// Output: <input>_tracks.root with TTree "tracks" (one entry per event).
//
// Usage:  root -l -b -q 'makeTracks.C("CoBo_xxx_uvw.root")'
// =============================================================================
#include <vector>
#include <map>
#include <cmath>
#include <algorithm>
#include <TFile.h>
#include <TTree.h>
#include <TString.h>
#include <TMatrixDSym.h>
#include <TMatrixDSymEigen.h>
#include <TVectorD.h>

const double PITCH[3][2] = {{-1, 0}, {0.5, 0.8660254}, {0.5, -0.8660254}};
const double QUANTILE = 0.02;   // charge fraction cut defining the endpoints
const int    MIN_SLICES = 3;    // reject events with fewer 3D points

void makeTracks(const char *uvwFile) {
  TFile in(uvwFile);
  TTree *tree = (TTree*)in.Get("uvw");
  UInt_t eventId;
  std::vector<int> *slView = nullptr;
  std::vector<float> *slZ = nullptr, *slS = nullptr, *slQ = nullptr;
  tree->SetBranchAddress("eventId", &eventId);
  tree->SetBranchAddress("slView", &slView);
  tree->SetBranchAddress("slZ", &slZ);
  tree->SetBranchAddress("slS", &slS);
  tree->SetBranchAddress("slQ", &slQ);

  TString outName(uvwFile);
  outName.ReplaceAll("_uvw.root", "_tracks.root");
  TFile out(outName, "RECREATE");
  TTree ttree("tracks", "3D straight-line tracks");
  int   ok;
  float xs, ys, zs, xe, ye, ze, dirx, diry, dirz, length, charge, dEdx;
  ttree.Branch("eventId", &eventId);
  ttree.Branch("ok", &ok);
  ttree.Branch("xStart", &xs);  ttree.Branch("yStart", &ys);  ttree.Branch("zStart", &zs);
  ttree.Branch("xEnd", &xe);    ttree.Branch("yEnd", &ye);    ttree.Branch("zEnd", &ze);
  ttree.Branch("dirX", &dirx);  ttree.Branch("dirY", &diry);  ttree.Branch("dirZ", &dirz);
  ttree.Branch("length", &length);
  ttree.Branch("charge", &charge);
  ttree.Branch("dEdx", &dEdx);  // charge / length: the gain metric

  for (Long64_t i = 0; i < tree->GetEntries(); ++i) {
    tree->GetEntry(i);
    ok = 0; length = charge = dEdx = 0;

    // 1. group the slice centroids by drift slice (same global grid as
    //    analyzeUVW.C: bin center = (bin + 0.5) * 3 mm), triangulate (x,y)
    std::map<int, std::vector<int>> byZ;   // slice bin -> indices
    for (size_t k = 0; k < slView->size(); ++k)
      byZ[(int)std::floor(slZ->at(k)/3.0)].push_back(k);

    std::vector<double> px, py, pz, pw;
    for (auto &kv : byZ) {
      if (kv.second.size() < 2) continue;  // need >= 2 views for (x,y)
      // least squares for s = pitch . (x,y): accumulate normal equations
      double A00=0, A01=0, A11=0, b0=0, b1=0, wsum=0, zsum=0;
      for (int k : kv.second) {
        const double *p = PITCH[slView->at(k)];
        A00 += p[0]*p[0]; A01 += p[0]*p[1]; A11 += p[1]*p[1];
        b0  += p[0]*slS->at(k); b1 += p[1]*slS->at(k);
        wsum += slQ->at(k); zsum += slQ->at(k)*slZ->at(k);
      }
      double det = A00*A11 - A01*A01;
      if (std::fabs(det) < 1e-9) continue;
      px.push_back(( A11*b0 - A01*b1)/det);
      py.push_back((-A01*b0 + A00*b1)/det);
      pz.push_back(zsum/wsum);
      pw.push_back(wsum);
    }
    if ((int)px.size() < MIN_SLICES) { ttree.Fill(); continue; }

    // 2. charge-weighted 3D PCA
    double W = 0, m[3] = {0,0,0};
    for (size_t k = 0; k < px.size(); ++k) {
      W += pw[k]; m[0] += pw[k]*px[k]; m[1] += pw[k]*py[k]; m[2] += pw[k]*pz[k];
    }
    for (int j = 0; j < 3; ++j) m[j] /= W;
    TMatrixDSym cov(3);
    for (size_t k = 0; k < px.size(); ++k) {
      double d[3] = {px[k]-m[0], py[k]-m[1], pz[k]-m[2]};
      for (int r = 0; r < 3; ++r)
        for (int c = 0; c < 3; ++c) cov(r,c) += pw[k]*d[r]*d[c];
    }
    TMatrixDSymEigen eig(cov);           // eigenvalues sorted descending:
    const TMatrixD &evec = eig.GetEigenVectors();
    double dir[3] = {evec(0,0), evec(1,0), evec(2,0)};  // column 0 = track axis

    // 3. endpoints from charge quantiles along the line
    std::vector<std::pair<double,double>> lamq;   // (position along line, charge)
    for (size_t k = 0; k < px.size(); ++k)
      lamq.push_back({(px[k]-m[0])*dir[0] + (py[k]-m[1])*dir[1] + (pz[k]-m[2])*dir[2], pw[k]});
    std::sort(lamq.begin(), lamq.end());
    double cum = 0, lo = lamq.front().first, hi = lamq.back().first;
    for (auto &e : lamq) { cum += e.second; if (cum/W <= QUANTILE)   lo = e.first; }
    cum = 0;
    for (auto &e : lamq) { cum += e.second; if (cum/W <= 1-QUANTILE) hi = e.first; }

    // 4. orient so the charge-heavier half is the end (see WARNING above)
    double mid = 0.5*(lo+hi), qlow = 0, qhigh = 0;
    for (auto &e : lamq) (e.first < mid ? qlow : qhigh) += e.second;
    if (qhigh < qlow) {
      for (int j = 0; j < 3; ++j) dir[j] = -dir[j];
      double tmp = lo; lo = -hi; hi = -tmp;
    }

    ok = 1;
    xs = m[0] + lo*dir[0]; ys = m[1] + lo*dir[1]; zs = m[2] + lo*dir[2];
    xe = m[0] + hi*dir[0]; ye = m[1] + hi*dir[1]; ze = m[2] + hi*dir[2];
    dirx = dir[0]; diry = dir[1]; dirz = dir[2];
    length = hi - lo;
    charge = W;
    dEdx = length > 0 ? W/length : 0;
    ttree.Fill();
  }
  out.cd();
  ttree.Write();
  out.Close();
  printf("wrote %s\n", outName.Data());
}
