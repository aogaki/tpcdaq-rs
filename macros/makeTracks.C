// =============================================================================
// makeTracks.C - step 3: build 3D tracks from the UVW strip hits.
//
// Reads the "uvw" tree from analyzeUVW.C: slView/slZ/slS/slQ hold one entry
// per band-selected strip hit (view, drift z, strip position s, charge q).
// Every event is treated as ONE straight alpha track (mini-eTPC assumption).
// Strip coordinate of view v: s = pitch_v . (x,y) with
//   pitch_U = (-1, 0)   pitch_V = (0.5, +sqrt3/2)   pitch_W = (0.5, -sqrt3/2)
//   1. per view v: charge-weighted mean ms_v and variance
//      var_v = sum q (s - ms_v)^2 / sum q over ALL its hits. U, V and W all
//      need at least MIN_VIEW_HITS hits, else ok == 0.
//   2. xy direction WITHOUT z (a flat track keeps its in-plane angle): the
//      charge distribution in (x,y) has a 2x2 covariance S with
//      var_v = pitch_v^T S pitch_v, so the three views give it back:
//        Sxx = var_U   Sxy = (var_V - var_W) / sqrt3
//        Syy = (var_V + var_W - 0.5 var_U) / 1.5
//      d_xy = eigenvector of the largest eigenvalue, at
//      phi = 0.5 atan2(2 Sxy, Sxx - Syy). A smearing that adds the same
//      variance c to every view (strip pitch, diffusion) adds c*I to S and
//      does not move the eigenvector. lambda_max - lambda_min below
//      MIN_EIGEN_GAP_MM2 (no xy elongation: track along z) -> ok == 0.
//   3. anchor (x0,y0) = least squares of pitch_v . (x0,y0) = ms_v
//   4. xy path length of every hit, l = (s - ms_v) / (pitch_v . d_xy), from
//      the views with |pitch_v . d_xy| >= MIN_PROJ (a smaller value means the
//      view sees the track foreshortened); a charge-weighted straight-line
//      fit z = z0 + tan(theta) * l over those hits pooled gives the dip and
//      the z anchor: direction = (d_xy, tan(theta)), normalised
//   5. endpoints = 2%-98% charge quantiles of l over the same hits; the
//      outermost hits (lMin, lMax) are kept too as the "full" endpoints:
//      the quantile trims 1-4 mm off a stopping track (more charge, more
//      trim), which biases a range measurement (TODO/099, 100)
//   6. head/tail: the charge-heavier half is called the END.
//      WARNING: alphas cross the detector without stopping, so there is no
//      Bragg peak and this sign is basically a coin flip - use the geometry
//      of your source to orient tracks in analysis, or use |direction| only.
//
// Output: <input>_tracks.root with TTree "tracks" (one entry per event).
//
// Usage:  root -l -b -q 'makeTracks.C("CoBo_xxx_uvw.root")'
// =============================================================================
#include <TFile.h>
#include <TParameter.h>
#include <TString.h>
#include <TTree.h>

#include <algorithm>
#include <cmath>
#include <utility>
#include <vector>

const double PITCH[3][2] = {{-1, 0}, {0.5, 0.8660254}, {0.5, -0.8660254}};
const double QUANTILE = 0.02;          // charge fraction cut: endpoints
const int MIN_VIEW_HITS = 3;           // strip hits EACH of U, V, W needs
const double MIN_EIGEN_GAP_MM2 = 1.0;  // xy elongation needed for d_xy
const double MIN_PROJ = 0.3;           // |pitch_v . d_xy| a view needs for l

// QUANTILE .. 1-QUANTILE charge quantiles of (position, charge) pairs
// (sorts v; v must not be empty)
void quantileRange(std::vector<std::pair<double, double>> &v, double &lo,
                   double &hi)
{
  std::sort(v.begin(), v.end());
  double W = 0, cum = 0;
  for (auto &e : v) W += e.second;
  lo = v.front().first;
  hi = v.back().first;
  for (auto &e : v) {
    cum += e.second;
    if (cum / W <= QUANTILE) lo = e.first;
    if (cum / W <= 1 - QUANTILE) hi = e.first;
  }
}

void makeTracks(const char *uvwFile)
{
  TFile in(uvwFile);
  TTree *tree = (TTree *)in.Get("uvw");
  if (!tree) {
    printf(
        "no tree 'uvw' in %s - this macro takes the *_uvw.root "
        "from analyzeUVW.C\n",
        uvwFile);
    return;
  }
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
  int ok;
  float xs, ys, zs, xe, ye, ze, dirx, diry, dirz, length, charge, dEdx;
  float xsF, ysF, zsF, xeF, yeF, zeF, lengthF;  // outermost hits
  ttree.Branch("eventId", &eventId);
  ttree.Branch("ok", &ok);
  ttree.Branch("xStart", &xs);
  ttree.Branch("yStart", &ys);
  ttree.Branch("zStart", &zs);
  ttree.Branch("xEnd", &xe);
  ttree.Branch("yEnd", &ye);
  ttree.Branch("zEnd", &ze);
  ttree.Branch("dirX", &dirx);
  ttree.Branch("dirY", &diry);
  ttree.Branch("dirZ", &dirz);
  ttree.Branch("length", &length);
  ttree.Branch("charge", &charge);
  ttree.Branch("dEdx", &dEdx);    // charge / length: the gain metric
  ttree.Branch("xStartF", &xsF);  // "full" endpoints: outermost hits, same
  ttree.Branch("yStartF", &ysF);  // orientation as start/end
  ttree.Branch("zStartF", &zsF);
  ttree.Branch("xEndF", &xeF);
  ttree.Branch("yEndF", &yeF);
  ttree.Branch("zEndF", &zeF);
  ttree.Branch("lengthF", &lengthF);

  for (Long64_t i = 0; i < tree->GetEntries(); ++i) {
    tree->GetEntry(i);
    ok = 0;
    length = charge = dEdx = lengthF = 0;
    const size_t nk = slView->size();

    // 1. per view: charge-weighted mean and variance of s
    int nv[3] = {0, 0, 0};
    double qv[3] = {0, 0, 0}, ms[3] = {0, 0, 0};
    for (size_t k = 0; k < nk; ++k) {
      const int v = slView->at(k);
      ++nv[v];
      qv[v] += slQ->at(k);
      ms[v] += slQ->at(k) * slS->at(k);
    }
    if (nv[0] < MIN_VIEW_HITS || nv[1] < MIN_VIEW_HITS ||
        nv[2] < MIN_VIEW_HITS) {
      ttree.Fill();
      continue;
    }
    for (int v = 0; v < 3; ++v) ms[v] /= qv[v];
    double var[3] = {0, 0, 0};
    for (size_t k = 0; k < nk; ++k) {
      const int v = slView->at(k);
      const double ds = slS->at(k) - ms[v];
      var[v] += slQ->at(k) * ds * ds;
    }
    for (int v = 0; v < 3; ++v) var[v] /= qv[v];

    // 2. xy covariance from the three projections -> major axis = d_xy
    const double Sxx = var[0];
    const double Sxy = (var[1] - var[2]) / std::sqrt(3.0);
    const double Syy = (var[1] + var[2] - 0.5 * var[0]) / 1.5;
    const double gap = std::hypot(Sxx - Syy, 2 * Sxy);  // lam_max - lam_min
    if (gap < MIN_EIGEN_GAP_MM2) {
      ttree.Fill();
      continue;
    }
    const double phi = 0.5 * std::atan2(2 * Sxy, Sxx - Syy);
    const double ux = std::cos(phi), uy = std::sin(phi);

    // 3. anchor (x0,y0) from the view means; Q = charge of all three views.
    //    Any two distinct pitch vectors are independent, so det > 0.
    double A00 = 0, A01 = 0, A11 = 0, b0 = 0, b1 = 0, Q = 0;
    for (int v = 0; v < 3; ++v) {
      const double *p = PITCH[v];
      A00 += p[0] * p[0];
      A01 += p[0] * p[1];
      A11 += p[1] * p[1];
      b0 += p[0] * ms[v];
      b1 += p[1] * ms[v];
      Q += qv[v];
    }
    const double det = A00 * A11 - A01 * A01;
    const double m[2] = {(A11 * b0 - A01 * b1) / det,
                         (-A01 * b0 + A00 * b1) / det};

    // 4. xy path length l of every hit; charge-weighted fit z(l) -> dip
    double proj[3];
    for (int v = 0; v < 3; ++v) proj[v] = PITCH[v][0] * ux + PITCH[v][1] * uy;
    std::vector<std::pair<double, double>> lq;  // (l, q)
    double W = 0, Sl = 0, Sz = 0, Sll = 0, Slz = 0;
    for (size_t k = 0; k < nk; ++k) {
      const int v = slView->at(k);
      if (std::fabs(proj[v]) < MIN_PROJ) continue;
      const double l = (slS->at(k) - ms[v]) / proj[v], q = slQ->at(k),
                   z = slZ->at(k);
      lq.push_back({l, q});
      W += q;
      Sl += q * l;
      Sz += q * z;
      Sll += q * l * l;
      Slz += q * l * z;
    }
    // max_v |proj_v| >= cos 30 deg and a view has >= 3 distinct strips, so
    // lq is never empty and varL > 0; guarded against NaN all the same
    const double varL = W > 0 ? Sll / W - (Sl / W) * (Sl / W) : 0;
    if (varL <= 0) {
      ttree.Fill();
      continue;
    }
    const double tanTh = (Slz / W - (Sl / W) * (Sz / W)) / varL;
    const double z0 = Sz / W - tanTh * Sl / W;  // fit intercept at l = 0
    const double nrm = std::sqrt(1 + tanTh * tanTh);
    double dir[3] = {ux / nrm, uy / nrm, tanTh / nrm};

    // 5. endpoints from the pooled charge quantiles of l
    double lo, hi;
    quantileRange(lq, lo, hi);  // sorts lq
    double loF = lq.front().first, hiF = lq.back().first;

    // 6. orient so the charge-heavier half is the end (see WARNING above)
    double mid = 0.5 * (lo + hi), qlow = 0, qhigh = 0;
    for (auto &e : lq) (e.first < mid ? qlow : qhigh) += e.second;
    double lStart = lo, lEnd = hi, lStartF = loF, lEndF = hiF;
    if (qhigh < qlow) {
      std::swap(lStart, lEnd);
      std::swap(lStartF, lEndF);
      for (int j = 0; j < 3; ++j) dir[j] = -dir[j];
    }

    // point at l: (x0 + l ux, y0 + l uy, z0 + l tan(theta))
    ok = 1;
    xs = m[0] + lStart * ux;
    ys = m[1] + lStart * uy;
    zs = z0 + lStart * tanTh;
    xe = m[0] + lEnd * ux;
    ye = m[1] + lEnd * uy;
    ze = z0 + lEnd * tanTh;
    dirx = dir[0];
    diry = dir[1];
    dirz = dir[2];
    length = (hi - lo) * nrm;  // 3D length
    xsF = m[0] + lStartF * ux;
    ysF = m[1] + lStartF * uy;
    zsF = z0 + lStartF * tanTh;
    xeF = m[0] + lEndF * ux;
    yeF = m[1] + lEndF * uy;
    zeF = z0 + lEndF * tanTh;
    lengthF = (hiF - loF) * nrm;
    charge = Q;  // all three views, independent of the MIN_PROJ selection
    dEdx = length > 0 ? Q / length : 0;
    ttree.Fill();
  }
  out.cd();
  // run constants of analyzeUVW.C (TODO/178), copied on unchanged
  const char *runPars[6] = {"pressure_mbar", "sampling_MHz", "drift_hv_V",
                            "drift_v_cm_us", "mm_per_cell", "z_window_mm"};
  int nMissing = 0;
  for (const char *name : runPars) {
    auto *par = (TParameter<double> *)in.Get(name);
    if (!par) {
      ++nMissing;
      continue;
    }
    TParameter<double>(name, par->GetVal(), 'f').Write();  // 'f': hadd keeps the first
  }
  if (nMissing > 0)
    printf(
        "WARNING: %d of 6 run constants missing in %s (old uvw file): not "
        "copied; re-run analyzeUVW.C\n",
        nMissing, uvwFile);
  ttree.Write();
  out.Close();
  printf("wrote %s\n", outName.Data());
}
