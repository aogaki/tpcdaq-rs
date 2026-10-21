// =============================================================================
// compareUVW.C - are two *_uvw.root files identical? (TODO/169)
//
// Compares tree "uvw" (entry count; every entry, every branch, bit for bit,
// eventId order included, vectors element by element), the histograms hCharge
// and hWidth_U/V/W (every bin incl. under/overflow) and the SET of "evt_*"
// keys. Prints PASS, or the first difference and FAIL; the process exit code
// is 0 / 1 so scripts can test it.
//
// Usage:  root -l -b -q 'compareUVW.C("a_uvw.root", "b_uvw.root")'
// =============================================================================
#include <TFile.h>
#include <TH1.h>
#include <TKey.h>
#include <TList.h>
#include <TString.h>
#include <TSystem.h>
#include <TTree.h>

#include <cstring>
#include <iostream>
#include <memory>
#include <set>
#include <string>
#include <vector>

namespace {
struct Row {
  UInt_t eventId = 0;
  float qview[3] = {0, 0, 0}, width[3] = {0, 0, 0};
  int nhits[3] = {0, 0, 0};
  std::vector<int> slView;
  std::vector<float> slZ, slS, slQ;
};

template <class T>
bool sameBits(const T *a, const T *b, size_t n)
{
  return std::memcmp(a, b, n * sizeof(T)) == 0;
}
template <class T>
bool sameVec(const std::vector<T> &a, const std::vector<T> &b)
{
  return a.size() == b.size() && sameBits(a.data(), b.data(), a.size());
}

std::set<std::string> evtKeys(TFile *f)
{
  std::set<std::string> s;
  for (auto k : *f->GetListOfKeys()) {
    std::string n = ((TKey *)k)->GetName();
    if (n.rfind("evt_", 0) == 0) s.insert(n);
  }
  return s;
}

int fail(const TString &msg)
{
  std::cout << "FAIL: " << msg << std::endl;
  return 1;
}

int compare(const char *fa, const char *fb)
{
  std::unique_ptr<TFile> a(TFile::Open(fa)), b(TFile::Open(fb));
  if (!a || a->IsZombie()) return fail(TString("cannot open ") + fa);
  if (!b || b->IsZombie()) return fail(TString("cannot open ") + fb);
  auto ta = (TTree *)a->Get("uvw");
  auto tb = (TTree *)b->Get("uvw");
  if (!ta || !tb) return fail("tree 'uvw' missing");
  if (ta->GetEntries() != tb->GetEntries())
    return fail(Form("entries %lld vs %lld", ta->GetEntries(),
                     tb->GetEntries()));

  Row ra, rb;
  // vectors: SetBranchAddress needs a pointer-to-pointer that outlives the loop
  std::vector<int> *av = &ra.slView, *bv = &rb.slView;
  std::vector<float> *az = &ra.slZ, *as = &ra.slS, *aq = &ra.slQ;
  std::vector<float> *bz = &rb.slZ, *bs = &rb.slS, *bq = &rb.slQ;
  auto bind = [](TTree *t, Row &r, std::vector<int> *&v, std::vector<float> *&z,
                 std::vector<float> *&s, std::vector<float> *&q) {
    t->SetBranchAddress("eventId", &r.eventId);
    t->SetBranchAddress("qview", r.qview);
    t->SetBranchAddress("width", r.width);
    t->SetBranchAddress("nhits", r.nhits);
    t->SetBranchAddress("slView", &v);
    t->SetBranchAddress("slZ", &z);
    t->SetBranchAddress("slS", &s);
    t->SetBranchAddress("slQ", &q);
  };
  bind(ta, ra, av, az, as, aq);
  bind(tb, rb, bv, bz, bs, bq);
  const Long64_t n = ta->GetEntries();
  for (Long64_t i = 0; i < n; ++i) {
    ta->GetEntry(i);
    tb->GetEntry(i);
    const char *bad = nullptr;
    if (ra.eventId != rb.eventId) bad = "eventId";
    else if (!sameBits(ra.qview, rb.qview, 3)) bad = "qview";
    else if (!sameBits(ra.width, rb.width, 3)) bad = "width";
    else if (!sameBits(ra.nhits, rb.nhits, 3)) bad = "nhits";
    else if (!sameVec(ra.slView, rb.slView)) bad = "slView";
    else if (!sameVec(ra.slZ, rb.slZ)) bad = "slZ";
    else if (!sameVec(ra.slS, rb.slS)) bad = "slS";
    else if (!sameVec(ra.slQ, rb.slQ)) bad = "slQ";
    if (bad)
      return fail(Form("entry %lld branch %s differs (eventId %u vs %u)", i,
                       bad, ra.eventId, rb.eventId));
  }

  for (const char *hn : {"hCharge", "hWidth_U", "hWidth_V", "hWidth_W"}) {
    auto ha = (TH1 *)a->Get(hn);
    auto hb = (TH1 *)b->Get(hn);
    if (!ha || !hb) return fail(TString("histogram missing: ") + hn);
    if (ha->GetNbinsX() != hb->GetNbinsX())
      return fail(TString("nbins differ: ") + hn);
    for (int k = 0; k <= ha->GetNbinsX() + 1; ++k)
      if (ha->GetBinContent(k) != hb->GetBinContent(k))
        return fail(Form("%s bin %d: %g vs %g", hn, k, ha->GetBinContent(k),
                         hb->GetBinContent(k)));
  }

  if (evtKeys(a.get()) != evtKeys(b.get()))
    return fail("evt_* key sets differ");

  std::cout << "PASS: " << n << " entries identical (" << fa << " == " << fb
            << ")" << std::endl;
  return 0;
}
}  // namespace

void compareUVW(const char *fileA, const char *fileB)
{
  const int rc = compare(fileA, fileB);
  gSystem->Exit(rc);
}
