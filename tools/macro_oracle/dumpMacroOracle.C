// =============================================================================
// dumpMacroOracle.C — TODO/125 改訂 2 レーン D: macros の analyzeUVW.C + makeTracks.C の
// 出力(本物のマクロを実行した結果)を、UI の track-fit.ts と照合するための JSON に書き出す。
//
//   root -l -b -q 'dumpMacroOracle.C("X_raw.root", "X_uvw.root", "X_tracks.root",
//                                    "channel_map.csv", "out.json", 200)'
//
// ふつうは run_macro_oracle.sh から呼ぶ(macros/ を一時ディレクトリにコピーして実行する手順込み)。
// **出力はリポに入れない**(実データ由来)。
//
// 出力 JSON:
//   {"channelMap": [[aget, ch, view, strip, s_mm], ...],       // channel_map.csv の行(ファイル順)
//    "events": [
//      {"eventId": N,
//       "adc": [[512 個の int], ...],    // raw の adc[aget][ch][*]、channelMap の行と同じ並び
//                                         // (未マップ ch = FPN は書かない — analyzeUVW も読まない)
//       "uvw": {"view": [...], "z": [...], "s": [...], "q": [...]},   // slView / slZ / slS / slQ
//       "track": {"ok": 0|1, "xStart": ..., ..., "lengthF": ...}}     // tracks の全ブランチ
//    ]}
// float は %.9g(float32 を往復で失わない桁数)。3 つの木は同じ順に 1 イベント 1 entry
// (analyzeUVW / makeTracks は全 entry を Fill する)— eventId が食い違えば中断する。
// =============================================================================
#include <TFile.h>
#include <TTree.h>

#include <cstdio>
#include <fstream>
#include <sstream>
#include <string>
#include <vector>

void dumpMacroOracle(const char *rawFile, const char *uvwFile, const char *tracksFile,
                     const char *channelMapFile, const char *outJson, Long64_t maxEvents = -1)
{
  // --- channel_map.csv(analyzeUVW.C と同じ読み方)--------------------------------
  struct Row {
    int aget, ch, view, strip;
    float s;
  };
  std::vector<Row> map;
  std::ifstream fm(channelMapFile);
  if (!fm) {
    printf("ERROR: %s not found\n", channelMapFile);
    return;
  }
  for (std::string line; std::getline(fm, line);) {
    if (line.empty() || line[0] == '#') continue;
    std::istringstream ss(line);
    Row r;
    char comma;
    if (ss >> r.aget >> comma >> r.ch >> comma >> r.view >> comma >> r.strip >> comma >> r.s)
      map.push_back(r);
  }

  // --- 3 つの木 ----------------------------------------------------------------
  TFile fr(rawFile), fu(uvwFile), ft(tracksFile);
  TTree *raw = (TTree *)fr.Get("raw");
  TTree *uvw = (TTree *)fu.Get("uvw");
  TTree *trk = (TTree *)ft.Get("tracks");
  if (!raw || !uvw || !trk) {
    printf("ERROR: tree missing (raw=%p uvw=%p tracks=%p)\n", (void *)raw, (void *)uvw,
           (void *)trk);
    return;
  }
  UInt_t eRaw, eUvw, eTrk;
  static Short_t adc[4][68][512];
  raw->SetBranchAddress("eventId", &eRaw);
  raw->SetBranchAddress("adc", adc);

  std::vector<int> *slView = nullptr;
  std::vector<float> *slZ = nullptr, *slS = nullptr, *slQ = nullptr;
  uvw->SetBranchAddress("eventId", &eUvw);
  uvw->SetBranchAddress("slView", &slView);
  uvw->SetBranchAddress("slZ", &slZ);
  uvw->SetBranchAddress("slS", &slS);
  uvw->SetBranchAddress("slQ", &slQ);

  const char *floatNames[] = {"xStart", "yStart", "zStart", "xEnd",   "yEnd",    "zEnd",
                              "dirX",   "dirY",   "dirZ",   "length", "charge",  "dEdx",
                              "xStartF", "yStartF", "zStartF", "xEndF", "yEndF", "zEndF",
                              "lengthF"};
  const int nFloat = sizeof(floatNames) / sizeof(floatNames[0]);
  float fv[32];
  int ok;
  trk->SetBranchAddress("eventId", &eTrk);
  trk->SetBranchAddress("ok", &ok);
  for (int k = 0; k < nFloat; ++k) trk->SetBranchAddress(floatNames[k], &fv[k]);

  Long64_t n = raw->GetEntries();
  if (uvw->GetEntries() != n || trk->GetEntries() != n) {
    printf("ERROR: entry counts differ (raw %lld, uvw %lld, tracks %lld)\n", n,
           uvw->GetEntries(), trk->GetEntries());
    return;
  }
  if (maxEvents >= 0 && maxEvents < n) n = maxEvents;

  FILE *f = fopen(outJson, "w");
  if (!f) {
    printf("ERROR: cannot write %s\n", outJson);
    return;
  }
  fprintf(f, "{\"channelMap\":[");
  for (size_t k = 0; k < map.size(); ++k)
    fprintf(f, "%s[%d,%d,%d,%d,%.9g]", k ? "," : "", map[k].aget, map[k].ch, map[k].view,
            map[k].strip, map[k].s);
  fprintf(f, "],\n\"events\":[");

  int nOk = 0;
  for (Long64_t i = 0; i < n; ++i) {
    raw->GetEntry(i);
    uvw->GetEntry(i);
    trk->GetEntry(i);
    if (eRaw != eUvw || eRaw != eTrk) {
      printf("ERROR: entry %lld eventId differs (raw %u, uvw %u, tracks %u)\n", i, eRaw, eUvw,
             eTrk);
      fclose(f);
      return;
    }
    fprintf(f, "%s\n{\"eventId\":%u,\"adc\":[", i ? "," : "", eRaw);
    for (size_t k = 0; k < map.size(); ++k) {
      fprintf(f, "%s[", k ? "," : "");
      for (int t = 0; t < 512; ++t) fprintf(f, t ? ",%d" : "%d", adc[map[k].aget][map[k].ch][t]);
      fprintf(f, "]");
    }
    fprintf(f, "],\n\"uvw\":{\"view\":[");
    for (size_t k = 0; k < slView->size(); ++k) fprintf(f, k ? ",%d" : "%d", slView->at(k));
    fprintf(f, "],\"z\":[");
    for (size_t k = 0; k < slZ->size(); ++k) fprintf(f, k ? ",%.9g" : "%.9g", slZ->at(k));
    fprintf(f, "],\"s\":[");
    for (size_t k = 0; k < slS->size(); ++k) fprintf(f, k ? ",%.9g" : "%.9g", slS->at(k));
    fprintf(f, "],\"q\":[");
    for (size_t k = 0; k < slQ->size(); ++k) fprintf(f, k ? ",%.9g" : "%.9g", slQ->at(k));
    fprintf(f, "]},\n\"track\":{\"ok\":%d", ok);
    for (int k = 0; k < nFloat; ++k) fprintf(f, ",\"%s\":%.9g", floatNames[k], fv[k]);
    fprintf(f, "}}");
    nOk += ok;
  }
  fprintf(f, "]}\n");
  fclose(f);
  printf("wrote %s: %lld events (ok %d / not ok %lld), %zu mapped channels\n", outJson, n, nOk,
         n - nOk, map.size());
}
