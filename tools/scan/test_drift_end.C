// =============================================================================
// test_drift_end.C — drift_end.C の合成テスト(CURRENT「今日の進行」A: 12.5 MHz ドリフト端 run)。
//
//   root -l -b -q tools/scan/test_drift_end.C     → exit code = 失敗数(0 = green)
//
// 合成 raw(mini = adc[4][68][512])を一時ディレクトリに書き、drift_end.C の関数で求めた
// first / last のヒット cell の分布とドリフト端の指標が手計算どおりに戻ることを見る。
// 期待値の出典は各 check の上のコメント(手計算)。分位点は線形補間 h = (n−1)·q
// (run_metrics.C の quantile と同じ定義)。
//
// 全 ch(FPN 含む)の基線 = b(a,c) + cm(t)(test_pulser_kernel.C と同じ):
//   b(a,c) = 200 + 3c + 50a、cm(t) = t / 13(整数除算。cell 0 で 0、cell 511 で 39 — run0076 の
//   common mode の模型)。信号 ch だけ ped 窓 cell 1..20 に ±2 の交互(奇数 cell −2、偶数 +2 → 平均 0)。
//   FPN を引けば v(t) = 足した矩形の高さちょうど(cell 21 以降)。引かなければ
//   v = cm(t) − 0.4(cm の cell 1..20 の平均 = 8/20)が残り、cell ≥ 468(cm ≥ 36)で全信号 ch が
//   35 を越える → 全イベントの last = 511、ヒット無しイベントもヒットありになる。
//
// ヒット定義(drift_end.C): ヒットに使う cell は [2, 509) だけ、かつ ch が run_metrics.C の「ヒット ch」
// (ピーク > 35 かつ ピーク ±5 cells の閾値超の和 q ≥ 2000。ピークは窓全体の最大の最初の cell)を
// 満たすときだけ。矩形パルスは peak = 先頭 cell なので ±5 の窓に入るのは先頭から 6 cell
// → q = 6 × 高さ。高さ 400 なら q = 2400 ≥ 2000(幅 ≥ 11 cell のパルスは従来どおり数える)。
//
// 全イベント共通の「ヒットに数えてはいけないもの」(信号 ch のみ、FPN ch には入れない):
//   窓端の跳ね    全 AGET・全信号 ch の cell 510, 511 に +60、先頭の整定 cell 0, 1 に +60
//                 (v = 60 > 35 だが q ≤ 120 < 2000 で ch 条件に落ちる。cell 1 は ped 窓 1..20 に
//                  入るので ped が +60/20 = +3 ずれ、パルス以外の v は −3 になる = 閾値 35 のおとりは
//                  v = 32 に下がるだけで結論は同じ)
//   単独 cell     AGET 2 ch 20 の cell 25 と cell 505 に +40(v = 37 > 35 だが 1 cell、q < 2000)。
//                 数えると first = 25 / last = 505 になる(L < 505 のイベントで last が変わる)
//
// イベント e = 0..103(104 イベント):
//   e = 0..100(101 イベント)= ヒットあり。F = 40 + e mod 21、L は下の表。
//     AGET 1 ch 7   cell F .. F+20     高さ 400(q = 6×400 = 2400 → ヒット ch)→ first = F
//     AGET 0 ch 3   cell F+5 .. L−5    高さ 300(中の ch — first にも last にもならない)
//     AGET 2 ch 40  cell L−30 .. L     高さ 400(q = 2400)→ last = min(L, 508)
//     L = 100 + e(e = 0..95)、508(e = 96)、509(e = 97)、510(e = 98, 99)、511(e = 100)。
//         cell ≥ 509 は数えないので e = 96..100 の last はすべて 508
//   おとり(e = 0..100 にも全部入れる):
//     閾値ちょうど: AGET 2 ch 10 cell 30..34 と AGET 0 ch 60 cell 490..500 に高さ 35(v = 35 は越えない。
//                  数えると first = 30 / last = 500 になる)
//     FPN ch:      AGET 3 ch 45(FPN)cell 25..30 と 480..495 に高さ 400(FPN の平均に 1/4 入って
//                  AGET 3 の信号 ch は v = −100。FPN を信号として使うと ch 45 の v = 300 で
//                  first = 25 / last = 495 になる)
//   e = 101 = 基線だけ、e = 102 = 閾値ちょうどのおとりだけ、e = 103 = FPN のおとりだけ(3 つともヒット無し)
//
// 手計算(e = 0..100、ヒットあり 101 個。n = 101 なので h = 100·q は整数 = 昇順の index そのもの):
//   F: r = e mod 21。101 = 4·21 + 17 → r = 0..16 が 5 個、17..20 が 4 個
//      昇順 index: r ≤ 16 は [5r, 5r+4]、r = 17 → 85..88、18 → 89..92、19 → 93..96、20 → 97..100
//      p10 = idx 10 → r 2 → 42、median = idx 50 → r 10 → 50、p90 = idx 90 → r 18 → 58
//   last: 昇順 idx 0..95 = 100..195、96..100 = 508(窓端 cell ≥ 509 は数えない)
//      p10 = idx 10 → 110、median = idx 50 → 150、p90 = idx 90 → 190、p99 = idx 99 → 508
//   edge: cell_p99 = 508(= p99)、width_cells = 508 − 190(上から累積 10 % = p90)= 318、
//         t_max_cells = 508 − 50(first の median)= 458
//   clip_back_frac = last ≥ 509 は無い(cell を [2, 509) に限ったので構造的に 0)= 0
//
// maxEvents = 50(e = 0..49)は分位点が補間になる(n = 50、h = 49·q):
//   F: r = 0..7 が 3 個(idx [3r, 3r+2])、8..20 が 2 個(idx [24 + 2(r−8), 25 + 2(r−8)])
//      p10 h = 4.9 → idx 4, 5 = r 1 → 41、median h = 24.5 → idx 24, 25 = r 8 → 48、
//      p90 h = 44.1 → idx 44, 45 = r 18 → 58
//   L = 100..149: p10 = 100 + 4.9 = 104.9、median = 124.5、p90 = 144.1、p99 = 100 + 48.51 = 148.51
//   edge: cell_p99 = 148.51、width = 148.51 − 144.1 = 4.41、t_max = 148.51 − 48 = 100.51
//   (ヒストを上から数えて整数 cell で切る定義なら 149 / 4 / 101 — 補間の p99 と同じ値であることを縛る)
// =============================================================================
#include "drift_end.C"

#include <TSystem.h>

#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <string>
#include <vector>

namespace test_de {

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

void checkCount(long got, long want, const std::string &what)
{
  check(got == want, what + ": got " + std::to_string(got) + " want " + std::to_string(want));
}

bool contains(const std::string &hay, const std::string &needle)
{
  return hay.find(needle) != std::string::npos;
}

// ---- 合成 raw ---------------------------------------------------------------------------------
Short_t g_adc[4][68][512];

int baseline(int a, int c) { return 200 + 3 * c + 50 * a; }
int commonMode(int t) { return t / 13; }

void fillBaseline()
{
  for (int a = 0; a < 4; ++a)
    for (int c = 0; c < 68; ++c)
      for (int t = 0; t < 512; ++t) {
        int v = baseline(a, c) + commonMode(t);
        if (!runmetrics::isFpn(c) && t >= 1 && t <= 20) v += (t % 2 == 1) ? -2 : +2;
        g_adc[a][c][t] = static_cast<Short_t>(v);
      }
}

// cell lo..hi に +h の矩形。
void addBox(int a, int c, int lo, int hi, int h)
{
  for (int t = lo; t <= hi; ++t) g_adc[a][c][t] = static_cast<Short_t>(g_adc[a][c][t] + h);
}

int firstCell(int e) { return 40 + e % 21; }

int lastCell(int e)
{
  if (e <= 95) return 100 + e;
  if (e == 96) return 508;
  if (e == 97) return 509;
  if (e <= 99) return 510;
  return 511;
}

// 窓端の跳ねと先頭の整定(全信号 ch)、単独 cell のノイズ(冒頭)。全イベントに入れる。
void edgeArtifactsAndLoneNoise()
{
  for (int a = 0; a < 4; ++a)
    for (int c = 0; c < 68; ++c) {
      if (runmetrics::isFpn(c)) continue;
      addBox(a, c, 0, 1, 60);
      addBox(a, c, 510, 511, 60);
    }
  addBox(2, 20, 25, 25, 40);
  addBox(2, 20, 505, 505, 40);
}

void decoyAtThreshold()
{
  addBox(2, 10, 30, 34, 35);
  addBox(0, 60, 490, 500, 35);
}

void decoyOnFpn()
{
  addBox(3, 45, 25, 30, 400);
  addBox(3, 45, 480, 495, 400);
}

// 冒頭の表。
void fillEvent(int e)
{
  fillBaseline();
  edgeArtifactsAndLoneNoise();
  if (e <= 100) {
    const int F = firstCell(e), L = lastCell(e);
    addBox(1, 7, F, F + 20, 400);
    addBox(0, 3, F + 5, L - 5, 300);
    addBox(2, 40, L - 30, L, 400);
    decoyAtThreshold();
    decoyOnFpn();
  } else if (e == 102) {
    decoyAtThreshold();
  } else if (e == 103) {
    decoyOnFpn();
  }
}

// ヒット無しの 3 イベントだけの run(e = 101..103)。
void fillNoHit(int k) { fillEvent(101 + k); }

bool writeRaw(const std::string &path, int nEvents, void (*fill)(int))
{
  TFile f(path.c_str(), "RECREATE");
  if (f.IsZombie()) return false;
  TTree tree("raw", "synthetic raw (test_drift_end.C)");
  UInt_t eventId = 0;
  ULong64_t eventTime = 0;
  tree.Branch("eventId", &eventId);
  tree.Branch("eventTime", &eventTime);
  tree.Branch("adc", g_adc, "adc[4][68][512]/S");
  for (int k = 0; k < nEvents; ++k) {
    fill(k);
    eventId = static_cast<UInt_t>(k);
    eventTime = 1000ULL + static_cast<ULong64_t>(k) * 5000000ULL;  // 50 ms 間隔(10 ns 刻み)
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

// JSON の `"key": [a, b, …]` の整数の配列。キーが無ければ空。
std::vector<long> jsonIntArray(const std::string &js, const std::string &key)
{
  std::vector<long> out;
  const std::string head = "\"" + key + "\": [";
  size_t p = js.find(head);
  if (p == std::string::npos) return out;
  p += head.size();
  const size_t end = js.find(']', p);
  if (end == std::string::npos) return out;
  const std::string body = js.substr(p, end - p);
  size_t s = 0;
  while (s < body.size()) {
    size_t comma = body.find(',', s);
    if (comma == std::string::npos) comma = body.size();
    out.push_back(std::atol(body.substr(s, comma - s).c_str()));
    s = comma + 1;
  }
  return out;
}

long sum(const std::vector<long> &v)
{
  long s = 0;
  for (long x : v) s += x;
  return s;
}

// hist の bin(範囲外なら -1 — 配列の長さの check が別に落ちる)。
long bin(const std::vector<long> &h, int cell)
{
  return (cell >= 0 && static_cast<size_t>(cell) < h.size()) ? h[static_cast<size_t>(cell)] : -1;
}

}  // namespace test_de

int test_drift_end()
{
  using namespace test_de;
  const std::string dir = std::string(gSystem->TempDirectory()) + "/test_drift_end_" +
                          std::to_string(gSystem->GetPid());
  gSystem->mkdir(dir.c_str(), kTRUE);
  const std::string raw = dir + "/run0105_raw.root";
  const std::string noHitRaw = dir + "/run0106_raw.root";
  const std::string json = dir + "/run0105_drift_end.json";
  const std::string png = dir + "/run0105_drift_end.png";
  check(writeRaw(raw, 104, fillEvent), "write synthetic raw (104 events)");
  check(writeRaw(noHitRaw, 3, fillNoHit), "write no-hit raw (3 events)");

  // ---- 104 イベント、全部 --------------------------------------------------------------------
  std::printf("[synthetic raw, common-mode slope on all channels, decoys]\n");
  driftend::Result r;
  std::string err;
  check(driftend::computeDriftEnd(raw, 0, r, err), "computeDriftEnd ok (" + err + ")");
  checkCount(r.run, 105, "run (from run0105_raw.root)");
  checkCount(r.events, 104, "events");
  // e = 101..103 はヒット無し(基線だけ / 閾値ちょうど / FPN だけ)
  checkCount(r.events_with_hits, 101, "events_with_hits (no-hit events 101..103 not counted)");
  checkCount(r.agets_without_fpn, 0, "agets_without_fpn");

  // 冒頭の手計算: F の p10 / median / p90 = 42 / 50 / 58(おとりを数えると 25 や 30 になる)
  checkNear(r.first_hit_cell.p10, 42, "first_hit_cell.p10");
  checkNear(r.first_hit_cell.median, 50, "first_hit_cell.median");
  checkNear(r.first_hit_cell.p90, 58, "first_hit_cell.p90");
  // last の p10 / median / p90 / p99 = 110 / 150 / 190 / 508(FPN を引かないと全部 508 超、跳ねを数えると 511)
  checkNear(r.last_hit_cell.p10, 110, "last_hit_cell.p10");
  checkNear(r.last_hit_cell.median, 150, "last_hit_cell.median");
  checkNear(r.last_hit_cell.p90, 190, "last_hit_cell.p90");
  checkNear(r.last_hit_cell_p99, 508, "last_hit_cell.p99");
  // edge: 508、508 − 190 = 318、508 − 50 = 458
  checkNear(r.edge.cell_p99, 508, "edge.cell_p99 (= last_hit_cell.p99)");
  checkNear(r.edge.width_cells, 318, "edge.width_cells (p99 - p90)");
  checkNear(r.edge.t_max_cells, 458, "edge.t_max_cells (cell_p99 - first median)");
  // cell ≥ 509 は数えないので last ≥ 509 は 0 個
  checkNear(r.clip_back_frac, 0.0, "clip_back_frac = 0 (cells >= 509 not counted)");

  // ヒスト(512 bin、ヒットありの 101 イベント)
  checkCount(static_cast<long>(r.first_hit_hist.size()), 512, "first_hit_hist size");
  checkCount(static_cast<long>(r.last_hit_hist.size()), 512, "last_hit_hist size");
  checkCount(sum(r.first_hit_hist), 101, "first_hit_hist sum");
  checkCount(sum(r.last_hit_hist), 101, "last_hit_hist sum");
  // F = 40 + r: r = 0..16 は 5 個、17..20 は 4 個。39 と 61 は 0、おとりの 25 / 30 も 0
  checkCount(bin(r.first_hit_hist, 40), 5, "first_hit_hist[40]");
  checkCount(bin(r.first_hit_hist, 56), 5, "first_hit_hist[56]");
  checkCount(bin(r.first_hit_hist, 57), 4, "first_hit_hist[57]");
  checkCount(bin(r.first_hit_hist, 60), 4, "first_hit_hist[60]");
  checkCount(bin(r.first_hit_hist, 39), 0, "first_hit_hist[39]");
  checkCount(bin(r.first_hit_hist, 61), 0, "first_hit_hist[61]");
  checkCount(bin(r.first_hit_hist, 25), 0, "first_hit_hist[25] (FPN decoy / lone-cell noise)");
  checkCount(bin(r.first_hit_hist, 0), 0, "first_hit_hist[0] (settling bump)");
  checkCount(bin(r.first_hit_hist, 1), 0, "first_hit_hist[1] (settling bump)");
  checkCount(bin(r.first_hit_hist, 30), 0, "first_hit_hist[30] (threshold decoy)");
  // L: 100..195 は 1 個ずつ、196 は 0、508 は 5(e = 96..100)、509 / 510 / 511 は 0(跳ね・窓端は数えない)。
  // おとりの 495 / 500 / 単独ノイズの 505 は 0
  checkCount(bin(r.last_hit_hist, 100), 1, "last_hit_hist[100]");
  checkCount(bin(r.last_hit_hist, 195), 1, "last_hit_hist[195]");
  checkCount(bin(r.last_hit_hist, 196), 0, "last_hit_hist[196]");
  checkCount(bin(r.last_hit_hist, 508), 5, "last_hit_hist[508] (e = 96..100 clamp to the range end)");
  checkCount(bin(r.last_hit_hist, 509), 0, "last_hit_hist[509]");
  checkCount(bin(r.last_hit_hist, 510), 0, "last_hit_hist[510] (edge bump)");
  checkCount(bin(r.last_hit_hist, 511), 0, "last_hit_hist[511] (edge bump)");
  checkCount(bin(r.last_hit_hist, 505), 0, "last_hit_hist[505] (lone-cell noise, q < 2000)");
  checkCount(bin(r.last_hit_hist, 495), 0, "last_hit_hist[495] (FPN decoy)");
  checkCount(bin(r.last_hit_hist, 500), 0, "last_hit_hist[500] (threshold decoy)");

  // ---- JSON ------------------------------------------------------------------------------------
  const std::string js = driftend::toJson(r);
  check(contains(js, "\"run\": 105,"), "JSON \"run\": 105");
  check(contains(js, "\"events\": 104,"), "JSON \"events\": 104");
  check(contains(js, "\"events_with_hits\": 101,"), "JSON \"events_with_hits\": 101");
  check(contains(js, "\"hit_cell_range\": [2, 509],"), "JSON hit_cell_range");
  check(contains(js, "\"channel_condition\": \"run_metrics\","), "JSON channel_condition");
  check(contains(js, "\"first_hit_cell\": {\"median\": 50, \"p10\": 42, \"p90\": 58}"),
        "JSON first_hit_cell");
  check(contains(js,
                 "\"last_hit_cell\": {\"median\": 150, \"p10\": 110, \"p90\": 190, \"p99\": 508}"),
        "JSON last_hit_cell");
  check(contains(js, "\"edge\": {\"cell_p99\": 508, \"width_cells\": 318, \"t_max_cells\": 458}"),
        "JSON edge");
  check(contains(js, "\"clip_back_frac\": 0,"),
        "JSON clip_back_frac");
  for (const char *key : {"\"raw_file\"", "\"first_hit_hist\"", "\"last_hit_hist\""})
    check(contains(js, key), std::string("JSON key ") + key);
  const std::vector<long> jsFirst = jsonIntArray(js, "first_hit_hist");
  const std::vector<long> jsLast = jsonIntArray(js, "last_hit_hist");
  checkCount(static_cast<long>(jsFirst.size()), 512, "JSON first_hit_hist has 512 entries");
  checkCount(static_cast<long>(jsLast.size()), 512, "JSON last_hit_hist has 512 entries");
  check(jsFirst == r.first_hit_hist, "JSON first_hit_hist == Result.first_hit_hist");
  check(jsLast == r.last_hit_hist, "JSON last_hit_hist == Result.last_hit_hist");

  // ---- 入口: JSON と PNG を書く ------------------------------------------------------------------
  check(drift_end(raw.c_str(), json.c_str(), png.c_str(), 0) == 0, "drift_end() returns 0");
  check(!js.empty() && readFile(json) == js, "drift_end() wrote the same JSON");
  FileStat_t st;
  check(gSystem->GetPathInfo(png.c_str(), st) == 0 && st.fSize > 0, "drift_end() wrote the PNG");
  check(drift_end((dir + "/missing_raw.root").c_str(), json.c_str()) == 1,
        "drift_end() returns 1 for a missing raw file");

  // ---- maxEvents -----------------------------------------------------------------------------
  // 冒頭の手計算(e = 0..49、補間の分位点)
  std::printf("[maxEvents = 50]\n");
  driftend::Result m;
  check(driftend::computeDriftEnd(raw, 50, m, err), "computeDriftEnd ok (maxEvents 50)");
  checkCount(m.events, 50, "events (maxEvents 50)");
  checkCount(m.events_with_hits, 50, "events_with_hits (maxEvents 50)");
  checkNear(m.first_hit_cell.p10, 41, "first_hit_cell.p10 (maxEvents 50)");
  checkNear(m.first_hit_cell.median, 48, "first_hit_cell.median (maxEvents 50)");
  checkNear(m.first_hit_cell.p90, 58, "first_hit_cell.p90 (maxEvents 50)");
  checkNear(m.last_hit_cell.p10, 104.9, "last_hit_cell.p10 (maxEvents 50)");
  checkNear(m.last_hit_cell.median, 124.5, "last_hit_cell.median (maxEvents 50)");
  checkNear(m.last_hit_cell.p90, 144.1, "last_hit_cell.p90 (maxEvents 50)");
  checkNear(m.last_hit_cell_p99, 148.51, "last_hit_cell.p99 (maxEvents 50)");
  checkNear(m.edge.cell_p99, 148.51, "edge.cell_p99 (maxEvents 50, interpolated)");
  checkNear(m.edge.width_cells, 4.41, "edge.width_cells (maxEvents 50)");
  checkNear(m.edge.t_max_cells, 100.51, "edge.t_max_cells (maxEvents 50)");
  checkNear(m.clip_back_frac, 0.0, "clip_back_frac (maxEvents 50)");
  checkCount(sum(m.last_hit_hist), 50, "last_hit_hist sum (maxEvents 50)");

  // ---- ヒット無しだけの run → 値は null --------------------------------------------------------
  std::printf("[no-hit run: baseline only / threshold decoy only / FPN decoy only]\n");
  driftend::Result z;
  check(driftend::computeDriftEnd(noHitRaw, 0, z, err), "computeDriftEnd ok (no hits)");
  checkCount(z.events, 3, "events (no hits)");
  checkCount(z.events_with_hits, 0, "events_with_hits (no hits)");
  checkNull(z.first_hit_cell.median, "first_hit_cell.median (no hits)");
  checkNull(z.last_hit_cell_p99, "last_hit_cell.p99 (no hits)");
  checkNull(z.edge.cell_p99, "edge.cell_p99 (no hits)");
  checkNull(z.edge.width_cells, "edge.width_cells (no hits)");
  checkNull(z.edge.t_max_cells, "edge.t_max_cells (no hits)");
  checkNull(z.clip_back_frac, "clip_back_frac (no hits)");
  checkCount(sum(z.last_hit_hist), 0, "last_hit_hist sum (no hits)");
  check(contains(driftend::toJson(z),
                 "\"edge\": {\"cell_p99\": null, \"width_cells\": null, \"t_max_cells\": null}"),
        "JSON edge nulls (no hits)");

  // 後片付け
  for (const std::string &q : {raw, noHitRaw, json, png}) gSystem->Unlink(q.c_str());
  gSystem->Unlink(dir.c_str());

  std::printf("%s: %d failure(s)\n", g_failures == 0 ? "GREEN" : "RED", g_failures);
  return g_failures;
}
