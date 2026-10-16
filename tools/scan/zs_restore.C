// =============================================================================
// zs_restore.C — ZS raw の抑制セルを ped で埋めた raw を書く(TODO/154)。
//
//   root -l -b -q 'tools/scan/zs_restore.C("run0133_raw.root", "ped.json", "out/run0133_raw.root")'
//   int zs_restore(const char* rawIn, const char* pedJson, const char* rawOut, int maxEvents = 0)
//
// 目的: macros/analyzeUVW.C は ped = cells 1..20 の平均(FPN なし・閾値 35)。ZS の raw は抑制セルが 0
// なので ped ≈ 0 になって壊れる。抑制セルを表の mean で埋めれば ped = 表の mean、埋めたセルは
// val = 0 ≤ 35 で無視される → macros を無改造のまま非 ZS と同じ振る舞いにできる。
//
// 契約: raw ツリー(eventId/i, eventTime/l, adc[N][68][512]/S、N は leaf の長さから)をそのまま書き出し、
//   規則 = cell == 0 かつ ch が表にある → round(mean)(FPN ch も。表の block = asad*4 + aget)。
//   表に無い ch は不変のまま数え、WARNING を 1 回出す。
// 出力: 0 = 書いた、1 = 失敗(入力 / 表が読めない、rawOut == rawIn、書けない)。events と埋めたセル数を印字。
// run_metrics.C の loadPedTable を流用する(表の読み方を二重に持たない)。
// =============================================================================
#include "run_metrics.C"

#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

namespace zsrestore {

struct Stats {
  long events = 0;
  long long filled = 0;    // 表の round(mean) で埋めたセル数
  long long unfilled = 0;  // 0 だが表に無い ch のセル数(そのまま 0)
};

bool restore(const std::string &rawIn, const std::string &pedJson, const std::string &rawOut,
             int maxEvents, Stats &st, std::string &err)
{
  st = Stats();
  using runmetrics::kCells;
  using runmetrics::kChannels;
  if (rawIn == rawOut || runmetrics::resolvedPath(rawIn) == runmetrics::resolvedPath(rawOut)) {
    err = "rawOut is the same file as rawIn (" + rawIn + ") — refusing to overwrite the input";
    return false;
  }
  TFile fin(rawIn.c_str(), "READ");
  if (fin.IsZombie()) {
    err = "cannot open " + rawIn;
    return false;
  }
  TTree *tin = fin.Get<TTree>("raw");
  TLeaf *leaf = tin ? tin->GetLeaf("adc") : nullptr;
  if (!leaf || std::string(leaf->GetTypeName()) != "Short_t" ||
      leaf->GetLen() % (kChannels * kCells) != 0) {
    err = "raw: no TTree 'raw' with adc[N][68][512]/S in " + rawIn;
    return false;
  }
  const int nBlocks = leaf->GetLen() / (kChannels * kCells);
  std::vector<double> table;
  if (!runmetrics::loadPedTable(pedJson, nBlocks, table, err)) return false;
  // 表 → 整数(NaN = 表に無い)。round は 1 回だけ
  std::vector<Short_t> fillVal(table.size(), 0);
  std::vector<char> inTable(table.size(), 0);
  for (size_t i = 0; i < table.size(); ++i)
    if (std::isfinite(table[i])) {
      inTable[i] = 1;
      fillVal[i] = static_cast<Short_t>(std::lround(table[i]));
    }

  std::vector<Short_t> adc(static_cast<size_t>(leaf->GetLen()));
  UInt_t eventId = 0;
  ULong64_t eventTime = 0;
  if (tin->SetBranchAddress("adc", adc.data()) < 0 || tin->SetBranchAddress("eventId", &eventId) < 0 ||
      tin->SetBranchAddress("eventTime", &eventTime) < 0) {
    err = "raw: cannot read branches eventId / eventTime / adc in " + rawIn;
    return false;
  }

  TFile fout(rawOut.c_str(), "RECREATE");
  if (fout.IsZombie()) {
    err = "cannot create " + rawOut;
    return false;
  }
  TTree tout("raw", tin->GetTitle());
  tout.Branch("eventId", &eventId, "eventId/i");
  tout.Branch("eventTime", &eventTime, "eventTime/l");
  tout.Branch("adc", adc.data(), ("adc[" + std::to_string(nBlocks) + "][68][512]/S").c_str());

  Long64_t n = tin->GetEntries();
  if (maxEvents > 0 && maxEvents < n) n = maxEvents;
  for (Long64_t i = 0; i < n; ++i) {
    tin->GetEntry(i);
    for (int b = 0; b < nBlocks; ++b)
      for (int c = 0; c < kChannels; ++c) {
        Short_t *w = &adc[(static_cast<size_t>(b) * kChannels + c) * kCells];
        const size_t key = static_cast<size_t>(b) * kChannels + c;
        for (int t = 0; t < kCells; ++t) {
          if (w[t] != 0) continue;
          if (inTable[key]) {
            w[t] = fillVal[key];
            ++st.filled;
          } else {
            ++st.unfilled;
          }
        }
      }
    tout.Fill();
    ++st.events;
  }
  tout.Write();
  if (fout.Write() < 0 || fout.IsZombie()) {
    err = "write failed: " + rawOut;
    return false;
  }
  return true;
}

}  // namespace zsrestore

int zs_restore(const char *rawIn, const char *pedJson, const char *rawOut, int maxEvents = 0)
{
  zsrestore::Stats st;
  std::string err;
  if (!zsrestore::restore(rawIn, pedJson, rawOut, maxEvents, st, err)) {
    std::fprintf(stderr, "zs_restore: ERROR %s\n", err.c_str());
    return 1;
  }
  if (st.unfilled > 0)
    std::fprintf(stderr,
                 "zs_restore: WARNING %lld zero cells in channels absent from the pedestal table "
                 "left as 0\n",
                 st.unfilled);
  std::printf("zs_restore: %ld events, %lld cells filled (%lld left 0) -> %s\n", st.events, st.filled,
              st.unfilled, rawOut);
  return 0;
}
