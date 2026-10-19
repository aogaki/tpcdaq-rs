// test_gas_trend.C — gas_trend.C の合成テスト(TODO/161)。終了コード = 失敗数。
//   root -l -b -q tools/scan/test_gas_trend.C
// 合成 raw: 2 つの時間帯(0..19 s と 1000..1019 s、各 20 events)に 1 ch だけパルス(高さ H、
// cells 100..109、ピーク 105 = H + 10)を置く。ビン 600 s → bin 0 / bin 1。期待: charge_med = 10 H + 10、
// hit_med = 1、first = last = 105、width = 10、rate = 1 Hz、sat_frac = 0(bin 0)/ 0.25(bin 1 は 1/4 を 4095 に)。
#include <TFile.h>
#include <TROOT.h>
#include <TSystem.h>
#include <TTree.h>

#include <cstdio>
#include <fstream>
#include <sstream>
#include <string>
#include <vector>

namespace {
int fails = 0;
void check(bool ok, const char *what) {
  std::printf("%s %s\n", ok ? "ok  " : "FAIL", what);
  if (!ok) ++fails;
}
bool has(const std::string &s, const std::string &needle) { return s.find(needle) != std::string::npos; }

void writeRaw(const char *path) {
  const int kA = 4, kCh = 68, kCells = 512;
  TFile f(path, "RECREATE");
  TTree t("raw", "raw");
  UInt_t eventId = 0;
  ULong64_t eventTime = 0;
  std::vector<Short_t> adc(kA * kCh * kCells);
  t.Branch("eventId", &eventId, "eventId/i");
  t.Branch("eventTime", &eventTime, "eventTime/l");
  t.Branch("adc", adc.data(), "adc[4][68][512]/S");
  for (int ev = 0; ev < 40; ++ev) {
    const bool second = ev >= 20;
    const double tSec = second ? 1000.0 + (ev - 20) : ev;
    eventId = ev;
    eventTime = static_cast<ULong64_t>(tSec * 1e8);  // 10 ns tick
    std::fill(adc.begin(), adc.end(), 450);          // ped 450、FPN も 450(共通モード 0)
    const int H = second ? 200 : 300;
    Short_t *w = &adc[(0 * kCh + 5) * kCells];       // AGET 0, ch 5
    for (int c = 100; c < 110; ++c) w[c] = static_cast<Short_t>(450 + H);
    w[105] = static_cast<Short_t>(450 + H + 10);  // ピークを 105 に(±5 の窓に 10 cell 全部が入る)
    if (second && (ev % 4 == 0)) w[105] = 4095;      // bin 1 の 1/4 に飽和サンプル(中央値は動かない)
    t.Fill();
  }
  t.Write();
}
}  // namespace

int test_gas_trend() {
  gROOT->ProcessLine(".L tools/scan/gas_trend.C+");
  const std::string dir = "/tmp/test_gas_trend";
  gSystem->mkdir(dir.c_str(), true);
  const std::string raw = dir + "/run0001_raw.root", json = dir + "/run0001_trend.json";
  writeRaw(raw.c_str());

  const int rc = static_cast<int>(reinterpret_cast<long>(gROOT->ProcessLine(
      ("gas_trend(\"" + raw + "\", \"" + json + "\", 600)").c_str())));
  check(rc == 0, "gas_trend returns 0");
  std::ifstream in(json);
  std::stringstream ss;
  ss << in.rdbuf();
  const std::string s = ss.str();
  check(has(s, "\"events\":40"), "events = 40");
  check(has(s, "\"t_h\":0.083"), "bin 0 centre 0.083 h");
  check(has(s, "\"t_h\":0.250"), "bin 1 centre 0.250 h (1000 s / 600 s)");
  check(has(s, "\"charge_med\":3010"), "bin 0 charge = 10 x 300 + 10");
  check(has(s, "\"charge_med\":2010"), "bin 1 charge = 10 x 200 + 10 (median unaffected by the 5 saturated events)");
  check(has(s, "\"hit_med\":1.0"), "hit channels = 1");
  check(has(s, "\"first_med\":105.0,\"last_med\":105.0,\"width_med\":10.0"), "first/last/width");
  check(has(s, "\"rate_hz\":1.0000"), "rate = 1 Hz from the bin's own span");
  check(has(s, "\"sat_frac\":0.00000"), "bin 0 no saturation");
  check(has(s, "\"sat_frac\":0.25000"), "bin 1: 5 of 20 events saturated");
  check(gSystem->AccessPathName((dir + "/run0001_trend.png").c_str()) == false, "png written");

  const int rc2 = static_cast<int>(reinterpret_cast<long>(gROOT->ProcessLine(
      ("gas_trend(\"" + dir + "/none.root\", \"" + json + "\", 600)").c_str())));
  check(rc2 == 1, "missing input returns 1");

  std::printf("test_gas_trend: %d failure(s)\n", fails);
  return fails;
}
