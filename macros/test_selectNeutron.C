// =============================================================================
// test_selectNeutron.C - synthetic test of selectNeutron.C (TODO/194).
//
//   export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX27.sdk
//   root -l -b -q macros/test_selectNeutron.C        (run from the repo root)
//   -> exit code = number of failures (0 = green)
//
// Synthetic tracks / uvw / raw files are written to a temp directory.
// Geometry: z_window_mm = 200, so the inner region is
//   x in (-43, 43), y in (-48, 38), z in (10, 190)  (strict).
// A hit is put on a straight line by addHit(): for the line S + d*t the strip
// coordinate of view v is s = p_v . S_xy + a_v * t with a_v = p_v . d_xy and
// the PITCH p_U = (-1, 0), p_V = (0.5, 0.866), p_W = (0.5, -0.866):
//   d = (1, 0, 0)  : a = (-1, 0.5, 0.5)   -> all three views used
//   d = (0,-1, 0)  : a = (0, -0.866, 0.866) -> U unused (|a| = 0 < 0.3)
// Expected charges are hand calculations (see each event): q = sum of the
// charges in the window / 5 mm / number of used views.
// =============================================================================
#include <TFile.h>
#include <TParameter.h>
#include <TROOT.h>
#include <TSystem.h>
#include <TTree.h>

#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

namespace {
int fails = 0;
void check(bool ok, const std::string &what)
{
  std::printf("%s %s\n", ok ? "ok  " : "FAIL", what.c_str());
  if (!ok) ++fails;
}
bool near(double a, double b, double tol = 1e-3) { return std::fabs(a - b) <= tol; }

const double kP[3][2] = {{-1, 0}, {0.5, 0.8660254}, {0.5, -0.8660254}};

// code numbers of selectNeutron.C
enum { T_BLOB = 0, T_REJECT = 1, T_CONTAINED = 2, T_WALL_ALPHA = 3, T_VIN_X = 4, T_VIN_Y = 5, T_VIN_Z = 6 };

struct Ev {
  int ok = 1;
  float s[3] = {0, 0, 0}, e[3] = {0, 0, 0};  // F endpoints
  float len = 0;
  std::vector<int> v;
  std::vector<float> z, s_, q;
  // expectation
  int code = -2;
  double qin = -9, qmid = -9, qwall = -9;
  int wall = 0;
  int alongX = 0;
};

void setEnds(Ev &ev, double sx, double sy, double sz, double ex, double ey, double ez)
{
  ev.s[0] = sx; ev.s[1] = sy; ev.s[2] = sz;
  ev.e[0] = ex; ev.e[1] = ey; ev.e[2] = ez;
  ev.len = std::sqrt((ex - sx) * (ex - sx) + (ey - sy) * (ey - sy) + (ez - sz) * (ez - sz));
}

// hit of view v at 3D path t along (S -> E) of ev
void addHit(Ev &ev, int v, double t, double q)
{
  const double L = ev.len;
  double d[3];
  for (int k = 0; k < 3; ++k) d[k] = (ev.e[k] - ev.s[k]) / L;
  const double sv = kP[v][0] * (ev.s[0] + d[0] * t) + kP[v][1] * (ev.s[1] + d[1] * t);
  ev.v.push_back(v);
  ev.z.push_back(ev.s[2] + d[2] * t);
  ev.s_.push_back(sv);
  ev.q.push_back(q);
}

void writeFiles(const std::vector<Ev> &evs, const std::string &trk, const std::string &uvw,
                const std::string &raw, ULong64_t t0, ULong64_t t1)
{
  {
    TFile f(trk.c_str(), "RECREATE");
    UInt_t id;
    Int_t ok;
    Float_t xs, ys, zs, xe, ye, ze, len;
    TTree t("tracks", "tracks");
    t.Branch("eventId", &id);
    t.Branch("ok", &ok);
    t.Branch("xStartF", &xs); t.Branch("yStartF", &ys); t.Branch("zStartF", &zs);
    t.Branch("xEndF", &xe); t.Branch("yEndF", &ye); t.Branch("zEndF", &ze);
    t.Branch("lengthF", &len);
    for (size_t i = 0; i < evs.size(); ++i) {
      id = 1000 + i; ok = evs[i].ok;
      xs = evs[i].s[0]; ys = evs[i].s[1]; zs = evs[i].s[2];
      xe = evs[i].e[0]; ye = evs[i].e[1]; ze = evs[i].e[2];
      len = evs[i].len;
      t.Fill();
    }
    TParameter<double>("z_window_mm", 200., 'f').Write();
    t.Write();
  }
  {
    TFile f(uvw.c_str(), "RECREATE");
    UInt_t id;
    std::vector<int> v;
    std::vector<float> z, s, q;
    TTree t("uvw", "uvw");
    t.Branch("eventId", &id);
    t.Branch("slView", &v); t.Branch("slZ", &z); t.Branch("slS", &s); t.Branch("slQ", &q);
    for (size_t i = 0; i < evs.size(); ++i) {
      id = 1000 + i; v = evs[i].v; z = evs[i].z; s = evs[i].s_; q = evs[i].q;
      t.Fill();
    }
    t.Write();
  }
  {
    TFile f(raw.c_str(), "RECREATE");
    UInt_t id;
    ULong64_t et;
    TTree t("raw", "raw");
    t.Branch("eventId", &id);
    t.Branch("eventTime", &et);
    id = 1; et = t1; t.Fill();  // not time ordered on purpose: max - min
    id = 2; et = t0; t.Fill();
    t.Write();
  }
}
}  // namespace

int test_selectNeutron()
{
  gROOT->ProcessLine(".L macros/selectNeutron.C+");
  const std::string dir = std::string(gSystem->TempDirectory()) + "/test_selectNeutron_" +
                          std::to_string(gSystem->GetPid());
  gSystem->mkdir(dir.c_str(), kTRUE);

  std::vector<Ev> evs;

  // 0 blob: ok = 0, hits within 4 mm in s and 3 mm in z in every view
  {
    Ev e; e.ok = 0; e.code = T_BLOB;
    for (int v = 0; v < 3; ++v) {
      e.v.push_back(v); e.z.push_back(50); e.s_.push_back(10 + v); e.q.push_back(100);
      e.v.push_back(v); e.z.push_back(53); e.s_.push_back(14 + v); e.q.push_back(100);
    }
    evs.push_back(e);
  }
  // 1 reject: both ends outside (x = -60 and -50)
  { Ev e; setEnds(e, -60, 0, 100, -50, 10, 100); e.code = T_REJECT; evs.push_back(e); }
  // 2 reject: ok = 0 and wide (U spread 30 mm)
  {
    Ev e; e.ok = 0; e.code = T_REJECT;
    e.v = {0, 0, 1, 2}; e.z = {50, 50, 50, 50}; e.s_ = {0, 30, 5, 5}; e.q = {10, 10, 10, 10};
    evs.push_back(e);
  }
  // 3 contained: both ends inside
  { Ev e; setEnds(e, 0, 0, 100, 20, 10, 100); e.code = T_CONTAINED; evs.push_back(e); }
  // 4 wall-alpha: S = (0,0,100) -> (60,0,100), L = 60, d = (1,0,0), all 3 views.
  //   in  [0,5): t = 1 (q 100) + t = 2.5 (q 50)   per view -> 150 * 3 views / 5 / 3 = 30
  //   mid [27.5,32.5): t = 30 (q 60)               per view -> 60 * 3 / 15 = 12
  //   wall (55,60]: t = 58 (q 20)                  per view -> 20 * 3 / 15 = 4
  //   decoys outside every window: t = -1, 5.5, 63 (q 1000)
  {
    Ev e; setEnds(e, 0, 0, 100, 60, 0, 100); e.code = T_WALL_ALPHA; e.wall = 1; e.alongX = 1;
    e.qin = 30; e.qmid = 12; e.qwall = 4;
    for (int v = 0; v < 3; ++v) {
      addHit(e, v, 1, 100); addHit(e, v, 2.5, 50); addHit(e, v, 30, 60); addHit(e, v, 58, 20);
      addHit(e, v, -1, 1000); addHit(e, v, 5.5, 1000); addHit(e, v, 63, 1000);
    }
    evs.push_back(e);
  }
  // 5 vertex-in-exit-x: same line, bright inner end, low middle, medium wall end
  //   in: t = 1 (q 200) -> 200*3/15 = 40; mid: t = 30 (q 10) -> 2; wall: t = 58 (q 50) -> 10
  {
    Ev e; setEnds(e, 0, 0, 100, 60, 0, 100); e.code = T_VIN_X; e.wall = 1; e.alongX = 1;
    e.qin = 40; e.qmid = 2; e.qwall = 10;
    for (int v = 0; v < 3; ++v) { addHit(e, v, 1, 200); addHit(e, v, 30, 10); addHit(e, v, 58, 50); }
    evs.push_back(e);
  }
  // 6 vertex-in-exit-y: S = (0,-10,100) -> (0,-60,100), L = 50, d = (0,-1,0).
  //   a = (0, -0.866, 0.866): U unused (a = 0), 2 views. U decoys q = 5000 at s = 0 must be ignored.
  //   in: t = 2 (q 150): 150 * 2 / 5 / 2 = 30; mid [22.5,27.5): t = 25 (q 5) -> 1;
  //   wall (45,50]: t = 48 (q 25) -> 5
  {
    Ev e; setEnds(e, 0, -10, 100, 0, -60, 100); e.code = T_VIN_Y; e.wall = 2; e.alongX = 0;
    e.qin = 30; e.qmid = 1; e.qwall = 5;
    for (int v = 1; v < 3; ++v) { addHit(e, v, 2, 150); addHit(e, v, 25, 5); addHit(e, v, 48, 25); }
    for (double t : {2., 25., 48.}) addHit(e, 0, t, 5000);
    evs.push_back(e);
  }
  // 7 steep: d = (0.2, 0, 0.9798), L = 200, |d_xy| = 0.2 < 0.3 -> vertex-in-exit-z, q = -1
  {
    Ev e; setEnds(e, 0, 0, 20, 40, 0, 20 + 195.959); e.code = T_VIN_Z; e.wall = 3; e.alongX = 1;    e.qin = e.qmid = e.qwall = -1;
    for (int v = 0; v < 3; ++v) addHit(e, v, 1, 100);
    evs.push_back(e);
  }
  // 8.. boundaries: start (0,0,100) inside; the end sits at the boundary value.
  struct B { double x, y, z; int code, wall; const char *name; };
  const B bs[] = {
      {43.0, 0, 100, T_VIN_X, 1, "x = 43.0 is outside"},
      {42.9, 0, 100, T_CONTAINED, 0, "x = 42.9 is inside"},
      {-43.0, 0, 100, T_VIN_X, 1, "x = -43.0 is outside"},
      {0, 38.0, 100, T_VIN_Y, 2, "y = 38.0 is outside"},
      {0, 37.9, 100, T_CONTAINED, 0, "y = 37.9 is inside"},
      {0, -48.0, 100, T_VIN_Y, 2, "y = -48.0 is outside"},
      {0, -47.9, 100, T_CONTAINED, 0, "y = -47.9 is inside"},
      {0, 0, 10.0, T_VIN_Z, 3, "z = 10.0 is outside"},
      {0, 0, 10.1, T_CONTAINED, 0, "z = 10.1 is inside"},
      {0, 0, 190.0, T_VIN_Z, 3, "z = 190.0 (z_window - 10) is outside"},
      {0, 0, 189.9, T_CONTAINED, 0, "z = 189.9 is inside"},
  };
  const size_t nBase = evs.size();
  for (const B &b : bs) {
    Ev e; setEnds(e, 0, 0, 100, b.x, b.y, b.z); e.code = b.code; e.wall = b.wall;
    e.alongX = (std::fabs(b.x) > 2 * std::fabs(b.y)) ? 1 : 0;
    evs.push_back(e);
  }

  const std::string trk = dir + "/t_tracks.root", uvw = dir + "/t_uvw.root",
                    raw = dir + "/t_raw.root", out = dir + "/t_sel.root";
  const ULong64_t t0 = 5000000000ULL, t1 = t0 + 360000000000ULL;  // 3.6e11 ticks * 10 ns = 3600 s
  writeFiles(evs, trk, uvw, raw, t0, t1);

  gROOT->ProcessLine(Form("selectNeutron(\"%s\",\"%s\",\"%s\",\"%s\")", trk.c_str(), uvw.c_str(),
                          raw.c_str(), out.c_str()));

  // ---- read back ----
  TFile f(out.c_str());
  TTree *t = f.IsZombie() ? nullptr : (TTree *)f.Get("sel");
  check(t != nullptr, "output file has tree 'sel'");
  if (!t) return fails + 1;
  check(t->GetEntries() == (Long64_t)evs.size(), "one row per event");
  UInt_t eventId = 0;
  Int_t entry, code, exitWall, alongX;
  Float_t qin, qmid, qwall, lengthF;
  t->SetBranchAddress("eventId", &eventId);
  t->SetBranchAddress("entry", &entry);
  t->SetBranchAddress("code", &code);
  t->SetBranchAddress("q_in", &qin);
  t->SetBranchAddress("q_mid", &qmid);
  t->SetBranchAddress("q_wall", &qwall);
  t->SetBranchAddress("exitWall", &exitWall);
  t->SetBranchAddress("alongX", &alongX);
  t->SetBranchAddress("lengthF", &lengthF);
  int nCode[7] = {0, 0, 0, 0, 0, 0, 0};
  const char *names[] = {"blob", "reject(both out)", "reject(ok=0 wide)", "contained", "wall-alpha",
                         "vertex-in-exit-x", "vertex-in-exit-y", "steep -> vertex-in (q=-1)"};
  for (size_t i = 0; i < evs.size(); ++i) {
    t->GetEntry(i);
    const Ev &e = evs[i];
    std::string n = i < nBase ? names[i] : bs[i - nBase].name;
    check(entry == (int)i && eventId == 1000 + i, "[" + n + "] entry / eventId");
    check(code == e.code, "[" + n + "] code " + std::to_string(code) + " == " + std::to_string(e.code));
    check(exitWall == e.wall, "[" + n + "] exitWall " + std::to_string(exitWall));
    if (i >= 3) check(alongX == e.alongX, "[" + n + "] alongX");
    if (e.qin > -9) {
      check(near(qin, e.qin) && near(qmid, e.qmid) && near(qwall, e.qwall),
            "[" + n + "] q_in/q_mid/q_wall = " + std::to_string(qin) + "/" + std::to_string(qmid) + "/" +
                std::to_string(qwall));
    }
    if (i < nBase && std::fabs(lengthF - e.len) > 1e-3) check(false, "[" + n + "] lengthF");
    if (code >= 0 && code < 7) ++nCode[code];
  }

  // ---- rate: eventTime max - min = 3600 s = 1 h -> rate / h = count ----
  {
    TParameter<double> *h = (TParameter<double> *)f.Get("hours");
    check(h && near(h->GetVal(), 1.0, 1e-9), "hours = 1.0 from raw eventTime (3600 s)");
    int n = 0;
    for (size_t i = 0; i < evs.size(); ++i) if (evs[i].code == T_CONTAINED) ++n;
    check(nCode[T_CONTAINED] == n && n == 1 + 5, "contained count = 6 (-> 6 / h)");
    check(nCode[T_BLOB] == 1 && nCode[T_REJECT] == 2 && nCode[T_WALL_ALPHA] == 1, "blob 1, reject 2, wall-alpha 1");
    // vertex-in-x: event 5 + 2 boundary (43, -43); y: event 6 + 2; z: event 7 + 2
    check(nCode[T_VIN_X] == 3 && nCode[T_VIN_Y] == 3 && nCode[T_VIN_Z] == 3, "vertex-in x/y/z = 3/3/3");
  }

  gSystem->Exec(("rm -rf " + dir).c_str());
  std::printf("test_selectNeutron: %d failure(s)\n", fails);
  return fails;
}
