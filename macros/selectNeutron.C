// =============================================================================
// selectNeutron.C - provisional neutron-event selection v1 (TODO/194).
//
// Classifies every event with ONE reason code, so the cut is never silent:
//   0 blob              ok == 0 and the hit extent is <= 10 mm in s and z in all
//                       three views (and there are hits at all)
//   1 reject            ok == 0 (not a blob), or both ends outside
//   2 contained         both ends inside
//   3 wall-alpha        one end inside; charge along the line falls from the
//                       inner end to the outer end: q_wall < q_mid < q_in
//                       (an alpha entering through a wall and stopping)
//   4/5/6 vertex-in-exit-x/-y/-z
//                       one end inside, anything else: the inner end is the
//                       vertex; exit-x/y/z = wall the outer end is outside of
//                       (x checked first). Includes steep tracks
//                       (|d_xy| < 0.3 -> q = -1).
// Cut on x-aligned direction is NOT made: alongX is only a variable.
//
// Charge along the line (one-end-inside events): S = inner end, d = unit
// vector inner -> outer, L = lengthF. For a strip hit (view v, s) of a view
// with |a_v| >= 0.3, a_v = p_v . u (u = d_xy / |d_xy|, p_v = PITCH of
// makeTracks.C): xy path l = (s - p_v . S_xy) / a_v, 3D path t = l / |d_xy|.
// q_in = sum q (t in [0,5)) / 5 / nViews, q_mid: t in [L/2-2.5, L/2+2.5),
// q_wall: t in (L-5, L].  [ADC / mm]
//
// Output: TTree "sel" (one row per event: eventId, entry, code, q_in, q_mid,
// q_wall, exitWall (0 none/1 x/2 y/3 z), alongX (|dx| > 2|dy|), lengthF) and
// TParameter<double> "hours" (raw eventTime max - min, 10 ns ticks). A table
// of count and rate per hour per code is printed.
//
// Usage: root -l -b -q 'selectNeutron.C("run_tracks.root","run_uvw.root",
//                                       "run_raw.root","run_sel.root")'
// =============================================================================
#include <TFile.h>
#include <TParameter.h>
#include <TString.h>
#include <TTree.h>

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <vector>

namespace sn {
// Inner region, same numbers as contained.C: the detector half-sizes
// x +-53, y -58..+48 (mm) shrunk by a 10 mm margin; z in (10, z_window - 10).
const double X_HALF = 53, Y_LO = -58, Y_HI = 48, MARGIN = 10;
const double PITCH[3][2] = {{-1, 0}, {0.5, 0.8660254}, {0.5, -0.8660254}};
const double MIN_PROJ = 0.3;     // |a_v| a view needs to be used (as makeTracks.C)
const double MIN_DXY = 0.3;      // below: steep track, no charge profile
const double WIN_MM = 5.0;       // width of the in / mid / wall windows
const double BLOB_EXTENT = 10.0; // mm, compact.C "short event"
enum Code { BLOB, REJECT, CONTAINED, WALL_ALPHA, VIN_X, VIN_Y, VIN_Z, NCODE };
const char *const NAME[NCODE] = {"blob", "reject", "contained", "wall-alpha",
                                 "vertex-in-exit-x", "vertex-in-exit-y", "vertex-in-exit-z"};

bool inside(double x, double y, double z, double zWin)
{
  return x > -X_HALF + MARGIN && x < X_HALF - MARGIN && y > Y_LO + MARGIN && y < Y_HI - MARGIN &&
         z > MARGIN && z < zWin - MARGIN;
}
// 1 x, 2 y, 3 z (x first); 0 if the point is inside
int exitWall(double x, double y, double z, double zWin)
{
  if (!(x > -X_HALF + MARGIN && x < X_HALF - MARGIN)) return 1;
  if (!(y > Y_LO + MARGIN && y < Y_HI - MARGIN)) return 2;
  if (!(z > MARGIN && z < zWin - MARGIN)) return 3;
  return 0;
}
}  // namespace sn

// (max - min) of eventTime in hours; < 0 if the file / tree cannot be read
double selectNeutronHours(const char *rawFile)
{
  TFile f(rawFile);
  TTree *raw = f.IsZombie() ? nullptr : (TTree *)f.Get("raw");
  if (!raw) {
    printf("selectNeutron ERROR: no tree 'raw' in %s\n", rawFile);
    return -1;
  }
  ULong64_t et = 0, lo = ~0ULL, hi = 0;
  raw->SetBranchStatus("*", false);
  raw->SetBranchStatus("eventTime", true);
  raw->SetBranchAddress("eventTime", &et);
  const Long64_t n = raw->GetEntries();
  for (Long64_t i = 0; i < n; ++i) {
    raw->GetEntry(i);
    lo = std::min(lo, et);
    hi = std::max(hi, et);
  }
  if (n < 2 || hi <= lo) {
    printf("selectNeutron ERROR: %s has no time span (%lld entries)\n", rawFile, n);
    return -1;
  }
  return static_cast<double>(hi - lo) * 10e-9 / 3600.0;
}

void selectNeutron(const char *tracksFile, const char *uvwFile, const char *rawFile,
                   const char *outFile)
{
  using namespace sn;
  TFile ft(tracksFile), fu(uvwFile);
  TTree *tt = ft.IsZombie() ? nullptr : (TTree *)ft.Get("tracks");
  TTree *tu = fu.IsZombie() ? nullptr : (TTree *)fu.Get("uvw");
  if (!tt || !tu) {
    printf("selectNeutron ERROR: need tree 'tracks' in %s and 'uvw' in %s\n", tracksFile, uvwFile);
    return;
  }
  if (tt->GetEntries() != tu->GetEntries()) {
    printf("selectNeutron ERROR: tracks has %lld entries, uvw %lld\n", tt->GetEntries(),
           tu->GetEntries());
    return;
  }
  auto *zp = (TParameter<double> *)ft.Get("z_window_mm");
  if (!zp) {
    printf("selectNeutron ERROR: no z_window_mm in %s\n", tracksFile);
    return;
  }
  const double zWin = zp->GetVal();
  const double hours = selectNeutronHours(rawFile);
  if (hours <= 0) return;

  UInt_t eventId = 0, uEventId = 0;
  Int_t ok = 0;
  Float_t xs, ys, zs, xe, ye, ze, lengthF;
  tt->SetBranchAddress("eventId", &eventId);
  tt->SetBranchAddress("ok", &ok);
  tt->SetBranchAddress("xStartF", &xs);
  tt->SetBranchAddress("yStartF", &ys);
  tt->SetBranchAddress("zStartF", &zs);
  tt->SetBranchAddress("xEndF", &xe);
  tt->SetBranchAddress("yEndF", &ye);
  tt->SetBranchAddress("zEndF", &ze);
  tt->SetBranchAddress("lengthF", &lengthF);
  std::vector<int> *slView = nullptr;
  std::vector<float> *slZ = nullptr, *slS = nullptr, *slQ = nullptr;
  tu->SetBranchAddress("eventId", &uEventId);
  tu->SetBranchAddress("slView", &slView);
  tu->SetBranchAddress("slZ", &slZ);
  tu->SetBranchAddress("slS", &slS);
  tu->SetBranchAddress("slQ", &slQ);

  TFile fo(outFile, "RECREATE");
  TTree sel("sel", "selectNeutron v1");
  Int_t entry = 0, code = 0, exitW = 0, alongX = 0;
  Float_t qIn = -1, qMid = -1, qWall = -1, lenOut = 0;
  sel.Branch("eventId", &eventId);
  sel.Branch("entry", &entry);
  sel.Branch("code", &code);
  sel.Branch("q_in", &qIn);
  sel.Branch("q_mid", &qMid);
  sel.Branch("q_wall", &qWall);
  sel.Branch("exitWall", &exitW);
  sel.Branch("alongX", &alongX);
  sel.Branch("lengthF", &lenOut);

  long count[NCODE] = {0};
  const Long64_t n = tt->GetEntries();
  for (Long64_t i = 0; i < n; ++i) {
    tt->GetEntry(i);
    tu->GetEntry(i);
    if (uEventId != eventId) {
      printf("selectNeutron ERROR: entry %lld eventId %u (tracks) != %u (uvw)\n", i, eventId, uEventId);
      return;
    }
    entry = i;
    qIn = qMid = qWall = -1;
    exitW = 0;
    lenOut = lengthF;
    alongX = std::fabs(xe - xs) > 2 * std::fabs(ye - ys) ? 1 : 0;

    const bool sIn = inside(xs, ys, zs, zWin), eIn = inside(xe, ye, ze, zWin);

    // blob: ok == 0, hits exist, extent <= 10 mm in s and z in every view
    bool blob = false;
    if (!ok && slView && !slView->empty()) {
      double smin[3], smax[3], zmin[3], zmax[3];
      for (int v = 0; v < 3; ++v) {
        smin[v] = zmin[v] = 1e30;
        smax[v] = zmax[v] = -1e30;
      }
      for (size_t k = 0; k < slView->size(); ++k) {
        const int v = (*slView)[k];
        if (v < 0 || v > 2) continue;
        smin[v] = std::min<double>(smin[v], (*slS)[k]);
        smax[v] = std::max<double>(smax[v], (*slS)[k]);
        zmin[v] = std::min<double>(zmin[v], (*slZ)[k]);
        zmax[v] = std::max<double>(zmax[v], (*slZ)[k]);
      }
      blob = true;  // a view without hits has no extent
      for (int v = 0; v < 3; ++v)
        if (smax[v] >= smin[v] && (smax[v] - smin[v] > BLOB_EXTENT || zmax[v] - zmin[v] > BLOB_EXTENT))
          blob = false;
    }

    if (blob) {
      code = BLOB;
    } else if (!ok || (!sIn && !eIn)) {
      code = REJECT;
    } else if (sIn && eIn) {
      code = CONTAINED;
    } else {
      // one end inside: S = inner end, outer end decides the wall
      const double sx = sIn ? xs : xe, sy = sIn ? ys : ye, sz = sIn ? zs : ze;
      const double ox = sIn ? xe : xs, oy = sIn ? ye : ys, oz = sIn ? ze : zs;
      exitW = exitWall(ox, oy, oz, zWin);
      const double dl = std::sqrt((ox - sx) * (ox - sx) + (oy - sy) * (oy - sy) + (oz - sz) * (oz - sz));
      const double dxy = std::sqrt((ox - sx) * (ox - sx) + (oy - sy) * (oy - sy)) / (dl > 0 ? dl : 1);
      code = VIN_X + exitW - 1;
      if (dl > 0 && dxy >= MIN_DXY) {
        const double ux = (ox - sx) / dl / dxy, uy = (oy - sy) / dl / dxy;
        const double L = lengthF;
        double a[3];
        int nUsed = 0;
        for (int v = 0; v < 3; ++v) {
          a[v] = PITCH[v][0] * ux + PITCH[v][1] * uy;
          if (std::fabs(a[v]) >= MIN_PROJ) ++nUsed;
        }
        double sumIn = 0, sumMid = 0, sumWall = 0;
        for (size_t k = 0; k < slView->size(); ++k) {
          const int v = (*slView)[k];
          if (v < 0 || v > 2 || std::fabs(a[v]) < MIN_PROJ) continue;
          const double l = ((*slS)[k] - (PITCH[v][0] * sx + PITCH[v][1] * sy)) / a[v];
          const double t = l / dxy;
          const double q = (*slQ)[k];
          if (t >= 0 && t < WIN_MM) sumIn += q;
          if (t >= L / 2 - WIN_MM / 2 && t < L / 2 + WIN_MM / 2) sumMid += q;
          if (t > L - WIN_MM && t <= L) sumWall += q;
        }
        if (nUsed > 0) {
          qIn = sumIn / WIN_MM / nUsed;
          qMid = sumMid / WIN_MM / nUsed;
          qWall = sumWall / WIN_MM / nUsed;
          if (qWall < qMid && qMid < qIn) code = WALL_ALPHA;
        }
      }
    }
    ++count[code];
    sel.Fill();
  }
  TParameter<double>("hours", hours, 'f').Write();
  sel.Write();

  printf("selectNeutron: %lld events, %.3f h  (%s)\n", n, hours, tracksFile);
  printf("  %-18s %8s %10s\n", "code", "count", "rate [/h]");
  for (int c = 0; c < NCODE; ++c) printf("  %-18s %8ld %10.2f\n", NAME[c], count[c], count[c] / hours);
}
