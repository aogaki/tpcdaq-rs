// =============================================================================
// test_run_metrics.C — run_metrics.C の合成テスト(TODO/136 §2)。
//
//   root -l -b -q tools/scan/test_run_metrics.C       → exit code = 失敗数(0 = green)
//
// 合成 `run0042_raw.root`(10 イベント、mini = adc[4][68][512])と合成 `run0042_tracks.root` を
// 一時ディレクトリに書き、run_metrics.C の関数で数えた値が手計算どおりになることを見る。
// 期待値の出典は各 check の上のコメント(手計算)。分位点は線形補間 h = (n−1)·q
// (run_metrics.C の quantile と同じ定義)。
// =============================================================================
#include "run_metrics.C"

#include <TSystem.h>

#include <cmath>
#include <cstdio>
#include <cstring>
#include <string>

namespace test_rm {

int g_failures = 0;

void check(bool ok, const std::string &what)
{
  if (ok) {
    std::printf("  ok   %s\n", what.c_str());
  } else {
    std::printf("  FAIL %s\n", what.c_str());
    ++g_failures;
  }
}

void checkNear(double got, double want, const std::string &what, double tol = 1e-9)
{
  const bool ok = std::isfinite(got) && std::fabs(got - want) <= tol;
  char buf[256];
  std::snprintf(buf, sizeof(buf), "%s: got %.10g want %.10g", what.c_str(), got, want);
  check(ok, buf);
}

void checkNull(double got, const std::string &what)
{
  char buf[256];
  std::snprintf(buf, sizeof(buf), "%s: got %.10g want null(NaN)", what.c_str(), got);
  check(std::isnan(got), buf);
}

bool contains(const std::string &hay, const std::string &needle)
{
  return hay.find(needle) != std::string::npos;
}

// ---- 合成 raw ---------------------------------------------------------------------------------
// 全 ch(FPN 含む)に基線 b(a,c) = 200 + 3c + 50a。ペデスタル窓 cell 1..20 は ±2 の交互
// (奇数 cell = b−2、偶数 cell = b+2 → 平均ちょうど b)、それ以外の cell は b。
// パルスは b + H の矩形 → ped 減算後ちょうど H(閾値 35 超)。
Short_t g_adc[4][68][512];

int baseline(int a, int c) { return 200 + 3 * c + 50 * a; }

void fillBaseline()
{
  for (int a = 0; a < 4; ++a)
    for (int c = 0; c < 68; ++c)
      for (int t = 0; t < 512; ++t) {
        int v = baseline(a, c);
        if (t >= 1 && t <= 20) v += (t % 2 == 1) ? -2 : +2;
        g_adc[a][c][t] = static_cast<Short_t>(v);
      }
}

void pulse(int a, int c, int tLo, int tHi, int height)
{
  for (int t = tLo; t <= tHi; ++t) g_adc[a][c][t] = static_cast<Short_t>(baseline(a, c) + height);
}

void setRaw(int a, int c, int tLo, int tHi, int value)
{
  for (int t = tLo; t <= tHi; ++t) g_adc[a][c][t] = static_cast<Short_t>(value);
}

// FPN ch(11/22/45/56)にだけ載せる派手な信号。無視されなければ first = 0 / last = 511 /
// 飽和 / ヒット ch 数 +3 になって下の期待値を全部壊す。
void fpnDecoys()
{
  setRaw(0, 11, 50, 60, 4095);                     // FPN 11: 飽和
  pulse(2, 45, 495, 511, 1000);                    // FPN 45: 後ろの窓端
  pulse(3, 56, 0, 0, 500);                         // FPN 56: cell 0(前の窓端)
}

// ヒット ch の候補(FPN でない、AGET をまたいで非対称)と矩形の高さ H。
struct PoolCh {
  int a, c, h;
};
const PoolCh kPool[4] = {{0, 3, 100}, {1, 40, 150}, {3, 67, 250}, {2, 12, 400}};

// イベント k のヒット ch 数 n_k = 1 + (k mod 4) = [1,2,3,4,1,2,3,4,1,2](kPool の先頭 n_k 個)。
// 通常は cell 100..130(31 cells)。特別なイベント:
//   k=2: kPool[0] を cell 0 だけ(前の窓端クリップ、幅 1)
//   k=5: kPool[1] の cell 100..130 を 4095(飽和)
//   k=7: kPool[3] を cell 490..511(後ろの窓端クリップ、幅 22)
// eventTime = 2^40 + k·5,000,000(10 ns 刻み → 50 ms 間隔、48 bit 域で 32 bit に収まらない値)
const ULong64_t kT0 = 1099511627776ULL;  // 2^40
const ULong64_t kDt = 5000000ULL;

bool writeSyntheticRaw(const std::string &path)
{
  TFile f(path.c_str(), "RECREATE");
  if (f.IsZombie()) return false;
  TTree tree("raw", "synthetic raw (test_run_metrics.C)");
  UInt_t eventId = 0;
  ULong64_t eventTime = 0;
  tree.Branch("eventId", &eventId);
  tree.Branch("eventTime", &eventTime);
  tree.Branch("adc", g_adc, "adc[4][68][512]/S");
  for (int k = 0; k < 10; ++k) {
    fillBaseline();
    fpnDecoys();
    const int n = 1 + (k % 4);
    for (int p = 0; p < n; ++p) {
      int lo = 100, hi = 130;
      if (k == 2 && p == 0) lo = hi = 0;               // 前の窓端
      if (k == 7 && p == 3) lo = 490, hi = 511;        // 後ろの窓端
      pulse(kPool[p].a, kPool[p].c, lo, hi, kPool[p].h);
    }
    if (k == 5) setRaw(kPool[1].a, kPool[1].c, 100, 130, 4095);  // 飽和
    eventId = static_cast<UInt_t>(k);
    eventTime = kT0 + static_cast<ULong64_t>(k) * kDt;
    tree.Fill();
  }
  tree.Write();
  return true;
}

// FPN ch にしか信号の無い 3 イベント → ヒット 0。
bool writeFpnOnlyRaw(const std::string &path)
{
  TFile f(path.c_str(), "RECREATE");
  if (f.IsZombie()) return false;
  TTree tree("raw", "synthetic raw, FPN only");
  UInt_t eventId = 0;
  ULong64_t eventTime = 0;
  tree.Branch("eventId", &eventId);
  tree.Branch("eventTime", &eventTime);
  tree.Branch("adc", g_adc, "adc[4][68][512]/S");
  for (int k = 0; k < 3; ++k) {
    fillBaseline();
    fpnDecoys();
    pulse(1, 22, 100, 130, 300);  // FPN 22 にも
    eventId = static_cast<UInt_t>(k);
    eventTime = 1000ULL + static_cast<ULong64_t>(k) * 100000000ULL;  // 1 s 間隔
    tree.Fill();
  }
  tree.Write();
  return true;
}

// ---- 合成 tracks(makeTracks.C の "tracks" と同じ名前・型: ok = int、他は float)--------------
// e0: ok, dir (1,0,0)            → dip 0、φ 0
// e1: ok, dir (0, 1/√2, 1/√2)    → dip 45、φ 90
// e2: ok=0(ゴミ値 — 統計から外れること)
// e3: ok, dir (1/√2, −1/√2, 0)   → dip 0、φ −45
// e4: ok, dir (0.5, 0, −√3/2)    → dip −60、φ 0
bool writeSyntheticTracks(const std::string &path)
{
  TFile f(path.c_str(), "RECREATE");
  if (f.IsZombie()) return false;
  TTree tree("tracks", "synthetic tracks (test_run_metrics.C)");
  int ok = 0;
  float length = 0, charge = 0, dEdx = 0, dirX = 0, dirY = 0, dirZ = 0;
  tree.Branch("ok", &ok);
  tree.Branch("length", &length);
  tree.Branch("charge", &charge);
  tree.Branch("dEdx", &dEdx);
  tree.Branch("dirX", &dirX);
  tree.Branch("dirY", &dirY);
  tree.Branch("dirZ", &dirZ);
  const float r2 = static_cast<float>(1.0 / std::sqrt(2.0));
  const float r3 = static_cast<float>(std::sqrt(3.0) / 2.0);
  struct Row {
    int ok;
    float length, charge, dEdx, dx, dy, dz;
  };
  const Row rows[5] = {{1, 40, 1000, 25, 1, 0, 0},
                       {1, 50, 3000, 60, 0, r2, r2},
                       {0, 999, 99999, 999, 0, 0, 1},
                       {1, 60, 2000, 40, r2, -r2, 0},
                       {1, 80, 6000, 75, 0.5f, 0, -r3}};
  for (const Row &r : rows) {
    ok = r.ok;
    length = r.length;
    charge = r.charge;
    dEdx = r.dEdx;
    dirX = r.dx;
    dirY = r.dy;
    dirZ = r.dz;
    tree.Fill();
  }
  tree.Write();
  return true;
}

std::string readFile(const std::string &path)
{
  std::string s;
  FILE *fp = std::fopen(path.c_str(), "r");
  if (!fp) return s;
  char buf[4096];
  size_t n;
  while ((n = std::fread(buf, 1, sizeof(buf), fp)) > 0) s.append(buf, n);
  std::fclose(fp);
  return s;
}

}  // namespace test_rm

int test_run_metrics()
{
  using namespace test_rm;
  const std::string dir = std::string(gSystem->TempDirectory()) + "/test_run_metrics_" +
                          std::to_string(gSystem->GetPid());
  gSystem->mkdir(dir.c_str(), kTRUE);
  const std::string raw = dir + "/run0042_raw.root";
  const std::string tracks = dir + "/run0042_tracks.root";
  const std::string fpnRaw = dir + "/run0043_raw.root";
  const std::string json = dir + "/run0042_metrics.json";
  gSystem->Unlink(tracks.c_str());
  check(writeSyntheticRaw(raw), "write synthetic raw");
  check(writeFpnOnlyRaw(fpnRaw), "write FPN-only raw");

  // ---- tracks 無し ------------------------------------------------------------------------------
  std::printf("[synthetic raw, no tracks]\n");
  runmetrics::RunMetrics m;
  std::string err;
  check(runmetrics::computeRunMetrics(raw, m, err), "computeRunMetrics ok (" + err + ")");

  // 件数・run 番号・レート: 10 イベント、名前 run0042 → 42、
  // (10−1) / (9 × 5,000,000 × 10 ns) = 9 / 0.45 s = 20 Hz
  check(m.events == 10, "events == 10");
  check(m.run == 42, "run == 42 (from run0042_raw.root)");
  const std::string tail = "/run0042_raw.root";
  check(m.raw_file.size() > tail.size() &&
            m.raw_file.compare(m.raw_file.size() - tail.size(), tail.size(), tail) == 0 &&
            m.raw_file[0] == '/',
        "raw_file is an absolute path ending in run0042_raw.root (" + m.raw_file + ")");
  checkNear(m.rate_hz, 20.0, "rate_hz");

  // 帯の位置: first = [100,100,0,100×7] → 昇順 [0,100×9]
  //   median h=4.5 → 100、p10 h=0.9 → 0 + 0.9·100 = 90、p90 h=8.1 → 100
  checkNear(m.first_hit_cell.median, 100, "first_hit_cell.median");
  checkNear(m.first_hit_cell.p10, 90, "first_hit_cell.p10");
  checkNear(m.first_hit_cell.p90, 100, "first_hit_cell.p90");
  // last = [130×7, 511(k=7), 130×2] → 昇順 [130×9, 511]
  //   median 130、p10 130、p90 h=8.1 → 130 + 0.1·381 = 168.1
  checkNear(m.last_hit_cell.median, 130, "last_hit_cell.median");
  checkNear(m.last_hit_cell.p10, 130, "last_hit_cell.p10");
  checkNear(m.last_hit_cell.p90, 168.1, "last_hit_cell.p90");

  // 窓端クリップ: 前 = k=2 だけ(first 0 ≤ 2)、後ろ = k=7 だけ(last 511 ≥ 509) → 1/10
  checkNear(m.clip_front, 0.1, "clip_front");
  checkNear(m.clip_back, 0.1, "clip_back");

  // 飽和: hit セル数 = Σ_k(31·n_k)を k=2(P0 が 1 cell)と k=7(P3 が 22 cells)で補正
  //   = 31+62+63+124+31+62+93+115+31+62 = 674、うち 4095 は k=5 の P1 の 31 cells
  checkNear(m.saturated_sample_frac, 31.0 / 674.0, "saturated_sample_frac", 1e-12);
  checkNear(m.saturated_event_frac, 0.1, "saturated_event_frac");

  // パルス幅: ヒット ch 23 本(Σn_k)の幅 = [1(k=2 P0), 22(k=7 P3), 31×21] → 中央値 31
  checkNear(m.pulse_width_cells_median, 31, "pulse_width_cells_median");

  // ヒット ch 数: [1,2,3,4,1,2,3,4,1,2] → 昇順 [1,1,1,2,2,2,3,3,4,4] → 中央値 2
  checkNear(m.hit_channels_median, 2, "hit_channels_median");

  // 総電荷(ped 減算後の閾値超の和)。H·31 の和:
  //   n=1: 3100、n=2: 31·250 = 7750、n=3: 31·500 = 15500、n=4: 31·900 = 27900
  //   k=2: 100(P0 の 1 cell)+ 31·150 + 31·250 = 12500
  //   k=5: 31·100 + 31·(4095 − b(1,40) = 4095 − 370 = 3725) = 3100 + 115475 = 118575
  //   k=7: 31·(100+150+250) + 22·400 = 15500 + 8800 = 24300
  //   [3100,7750,12500,27900,3100,118575,15500,24300,3100,7750]
  //   → 昇順 [3100,3100,3100,7750,7750,12500,15500,24300,27900,118575] → (7750+12500)/2 = 10125
  checkNear(m.charge_median, 10125, "charge_median");

  // tracks ファイルが無い → tracks は null
  check(!m.tracks.present, "tracks absent -> not present");
  const std::string js = runmetrics::toJson(m);
  check(contains(js, "\"tracks\": null"), "JSON has \"tracks\": null");
  check(contains(js, "\"run\": 42,"), "JSON has \"run\": 42");
  check(contains(js, "\"clip_front\": 0.1,"), "JSON has \"clip_front\": 0.1");
  check(contains(js, "\"last_hit_cell\": {\"median\": 130, \"p10\": 130, \"p90\": 168.1}"),
        "JSON last_hit_cell object");
  for (const char *key :
       {"\"run\"", "\"raw_file\"", "\"events\"", "\"rate_hz\"", "\"first_hit_cell\"",
        "\"last_hit_cell\"", "\"clip_front\"", "\"clip_back\"", "\"saturated_sample_frac\"",
        "\"saturated_event_frac\"", "\"pulse_width_cells_median\"", "\"hit_channels_median\"",
        "\"charge_median\"", "\"tracks\""})
    check(contains(js, key), std::string("JSON key ") + key);

  // エントリ関数がファイルを書く(終了コード 0)
  check(run_metrics(raw.c_str(), json.c_str(), 100.0) == 0, "run_metrics() returns 0");
  check(readFile(json) == js, "run_metrics() wrote the same JSON");

  // ---- FPN だけに信号 → 何もヒットしない --------------------------------------------------------
  std::printf("[synthetic raw, FPN channels only]\n");
  runmetrics::RunMetrics f;
  check(runmetrics::computeRunMetrics(fpnRaw, f, err), "computeRunMetrics ok (FPN only)");
  check(f.events == 3, "events == 3");
  checkNear(f.hit_channels_median, 0, "hit_channels_median (FPN ignored)");
  checkNear(f.charge_median, 0, "charge_median (FPN ignored)");
  checkNull(f.first_hit_cell.median, "first_hit_cell.median (no hit)");
  checkNull(f.last_hit_cell.p90, "last_hit_cell.p90 (no hit)");
  checkNear(f.clip_front, 0, "clip_front (FPN ignored)");
  checkNear(f.clip_back, 0, "clip_back (FPN ignored)");
  checkNull(f.saturated_sample_frac, "saturated_sample_frac (no hit cells)");
  checkNear(f.saturated_event_frac, 0, "saturated_event_frac (FPN ignored)");
  checkNull(f.pulse_width_cells_median, "pulse_width_cells_median (no hit)");
  // 3 イベント、1 s 間隔 → (3−1) / 2 s = 1 Hz
  checkNear(f.rate_hz, 1.0, "rate_hz (FPN only)");
  const std::string fjs = runmetrics::toJson(f);
  check(contains(fjs, "\"first_hit_cell\": {\"median\": null, \"p10\": null, \"p90\": null}"),
        "JSON null quantiles");

  // ---- tracks あり ------------------------------------------------------------------------------
  std::printf("[synthetic raw + tracks]\n");
  check(writeSyntheticTracks(tracks), "write synthetic tracks");
  runmetrics::RunMetrics t;
  check(runmetrics::computeRunMetrics(raw, t, err), "computeRunMetrics ok (with tracks)");
  check(t.tracks.present, "tracks present");
  check(t.tracks.n == 5, "tracks.n == 5");
  // ok = [1,1,0,1,1] → 4/5
  checkNear(t.tracks.ok_frac, 0.8, "tracks.ok_frac");
  // ok の 4 本だけ。length [40,50,60,80]: median h=1.5 → 55、q25 h=0.75 → 47.5、
  //   q75 h=2.25 → 60 + 0.25·20 = 65 → IQR 17.5
  checkNear(t.tracks.length.median, 55, "tracks.length.median");
  checkNear(t.tracks.length.iqr, 17.5, "tracks.length.iqr");
  // charge 昇順 [1000,2000,3000,6000]: median 2500、q25 1750、q75 3750 → IQR 2000
  checkNear(t.tracks.charge.median, 2500, "tracks.charge.median");
  checkNear(t.tracks.charge.iqr, 2000, "tracks.charge.iqr");
  // dEdx 昇順 [25,40,60,75]: median 50、q25 25 + 0.75·15 = 36.25、q75 60 + 0.25·15 = 63.75 → 27.5
  checkNear(t.tracks.dEdx.median, 50, "tracks.dEdx.median");
  checkNear(t.tracks.dEdx.iqr, 27.5, "tracks.dEdx.iqr");
  // dip [0,45,0,−60] → √((0+2025+0+3600)/4) = √1406.25 = 37.5(float の dir なので 1e-4 deg)
  checkNear(t.tracks.dip_rms_deg, 37.5, "tracks.dip_rms_deg", 1e-4);
  // φ [0,90,−45,0] → √((0+8100+2025+0)/4) = √2531.25
  checkNear(t.tracks.phi_rms_deg, std::sqrt(2531.25), "tracks.phi_rms_deg", 1e-4);
  const std::string tjs = runmetrics::toJson(t);
  check(contains(tjs, "\"tracks\": {\"n\": 5, \"ok_frac\": 0.8, "
                      "\"length\": {\"median\": 55, \"iqr\": 17.5}, "
                      "\"charge\": {\"median\": 2500, \"iqr\": 2000}, "
                      "\"dEdx\": {\"median\": 50, \"iqr\": 27.5}, "),
        "JSON tracks object");

  // 後片付け
  for (const std::string &p : {raw, tracks, fpnRaw, json}) gSystem->Unlink(p.c_str());
  gSystem->Unlink(dir.c_str());

  std::printf("%s: %d failure(s)\n", g_failures == 0 ? "GREEN" : "RED", g_failures);
  return g_failures;
}
