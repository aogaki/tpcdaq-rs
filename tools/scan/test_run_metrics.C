// =============================================================================
// test_run_metrics.C — run_metrics.C の合成テスト(TODO/136 §2、§5 追補)。
//
//   root -l -b -q tools/scan/test_run_metrics.C       → exit code = 失敗数(0 = green)
//
// 合成 raw(mini = adc[4][68][512])と合成 tracks を一時ディレクトリに書き、run_metrics.C の
// 関数で数えた値が手計算どおりになることを見る。期待値の出典は各 check の上のコメント(手計算)。
// 分位点は線形補間 h = (n−1)·q(run_metrics.C の quantile と同じ定義)。
//
// §5 追補: 実データ run0076 では全 ch(FPN 含む)の基線が窓の中で 20〜40 ADC 単調に上がり、
// 初版(ped = cell 1..20 だけ)では全 ch が窓の後ろでヒットした。合成 raw にも全 ch 共通の
// 傾き s(t) を入れ、FPN で引けること・電荷の下限・ピークセルの帯位置・向きの畳み込みを見る。
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
// 全 ch(FPN 含む)の基線 = b(a,c) + s(t):
//   b(a,c) = 200 + 3c + 50a(ch ごとに違う定数)
//   s(t)   = t / 13(整数除算。cell 0 で 0、cell 511 で 39 — run0076 の「窓の中で 20〜40 ADC
//            単調に上がる」common mode の模型。FPN にも同じだけ載る)
// 信号 ch だけペデスタル窓 cell 1..20 に ±2 の交互(奇数 cell −2、偶数 cell +2 → 平均 0)。
//
// FPN を引いた後の値(run_metrics.C の手順どおり):
//   fpn(t) = AGET a の FPN 4 ch のセル毎平均 = B(a) + s(t)、B(a) = 200 + 50a + 3·(11+22+45+56)/4
//          = 300.5 + 50a
//   ped    = mean_{t=1..20}(w − fpn) = b(a,c) − B(a)(交互は平均 0)
//   v(t)   = w(t) − fpn(t) − ped = パルスの高さそのもの(s(t) は消える)
// 初版(FPN を引かない)では v(t) = s(t) − 0.4(ped 窓の s の平均 = 8 cells の 1 / 20)+ パルス
//   → s(t) ≥ 36 の cell 468..511 で全 256 信号 ch が閾値 35 を超える(= run0076 の症状)。
Short_t g_adc[4][68][512];

int baseline(int a, int c) { return 200 + 3 * c + 50 * a; }
int slope(int t) { return t / 13; }

void fillBaseline()
{
  for (int a = 0; a < 4; ++a)
    for (int c = 0; c < 68; ++c)
      for (int t = 0; t < 512; ++t) {
        int v = baseline(a, c) + slope(t);
        if (!runmetrics::isFpn(c) && t >= 1 && t <= 20) v += (t % 2 == 1) ? -2 : +2;
        g_adc[a][c][t] = static_cast<Short_t>(v);
      }
}

// 今の値に足す: cell lo..hi に +H、ピーク cell にさらに +bump(peak < 0 なら bump なし)。
void pulse(int a, int c, int lo, int hi, int height, int peak = -1, int bump = 0)
{
  for (int t = lo; t <= hi; ++t) g_adc[a][c][t] = static_cast<Short_t>(g_adc[a][c][t] + height);
  if (peak >= 0) g_adc[a][c][peak] = static_cast<Short_t>(g_adc[a][c][peak] + bump);
}

void setRaw(int a, int c, int tLo, int tHi, int value)
{
  for (int t = tLo; t <= tHi; ++t) g_adc[a][c][t] = static_cast<Short_t>(value);
}

// FPN ch(11/22/45/56)にだけ載せる派手な信号。FPN をヒットとして数えれば first = 0 / last = 511 /
// 飽和 / ヒット ch が増える。FPN の平均に入るので同じ AGET の信号 ch は負に振れる(ヒットしない)。
void fpnDecoys()
{
  setRaw(0, 11, 50, 60, 4095);  // FPN 11: 飽和
  pulse(2, 45, 495, 511, 1000);  // FPN 45: 後ろの窓端
  pulse(3, 56, 0, 0, 500);       // FPN 56: cell 0(前の窓端)
  pulse(1, 22, 100, 130, 300);   // FPN 22
}

// ヒット ch の候補(FPN でない、AGET をまたいで非対称)。通常は cell 100..130 の矩形 H に
// ピーク cell だけ +20。ピーク ±5 = 11 cells が矩形の中 → 電荷 q = 11·H + 20、全体 = 31·H + 20。
//   P0 (0,3)  H 200 peak 110: q 2220、全体 6220
//   P1 (1,40) H 300 peak 115: q 3320、全体 9320
//   P2 (3,67) H 250 peak 120: q 2770、全体 7770
//   P3 (2,12) H 400 peak 125: q 4420、全体 12420
struct PoolCh {
  int a, c, h, peak;
};
const PoolCh kPool[4] = {{0, 3, 200, 110}, {1, 40, 300, 115}, {3, 67, 250, 120}, {2, 12, 400, 125}};
const int kBump = 20;

void regular(int p)
{
  pulse(kPool[p].a, kPool[p].c, 100, 130, kPool[p].h, kPool[p].peak, kBump);
}

// イベント k の候補 ch 数 n_k = 1 + (k mod 4) = [1,2,3,4,1,2,3,4,1,2](kPool の先頭 n_k 個)。
// 全イベントに halo ch H1 (1,5): cell 100..130 の H 150、peak 118 +20 → q = 11·150 + 20 = 1670
// < 2000(STRIP_Q_MIN)→ 閾値は超えるがヒットではない。特別なイベント(他は regular):
//   k=1: P1 を cell 500..511、peak 508(後ろのピーク・端の両方)。q = 503..511 の 9 cells
//        = 9·300 + 20 = 2720、全体 12·300 + 20 = 3620、幅 12
//   k=2: P0 を cell 0 だけ H 2500(前のピーク・端の両方)。q = 全体 = 2500、幅 1
//   k=3: halo ch H2 (3,50): cell 505..511 の H 100、peak 511 +20 → q = 506..511 の 6 cells
//        = 620 < 2000 → ヒットでない(端の閾値超・ピーク 511 でもクリップに数えない)
//   k=4: Q2000 (2,30): cell 200..230 の H 180、peak 215 +20 → q = 10·180 + 200 = 2000 ちょうど
//        → ヒット(macros は q < STRIP_Q_MIN を捨てる = 2000 は残す)、全体 31·180 + 20 = 5600
//        Q1999 (2,31): 同じで peak +19 → q = 1999 → ヒットでない
//   k=5: P1 の cell 100..130 を生 4095(飽和)。v = 4095 − (B(1) + s(t)) − (b(1,40) − B(1))
//        = 4095 − 370 − s(t) = 3725 − s(t) → ピーク = s 最小の最初の cell = 100(s(100..103) = 7)。
//        q = cell 100..105 = 6·3725 − (7·4 + 8·2) = 22306。
//        全体 = 31·3725 − Σs(100..130) = 115475 − (7·4 + 8·13 + 9·13 + 10·1 = 259) = 115216
//   k=6: P0 の cell 0 に +60(前の端の閾値超、ピークは 110 のまま)→ 全体 6220 + 60 = 6280
//   k=7: P3 を cell 490..511、peak 500(後ろの端だけ、ピーク 500 < 506)。q = 495..505 の
//        11 cells = 4420、全体 22·400 + 20 = 8820、幅 22
//   k=8: P0 の cell 511 に +60(後ろの端だけ)→ 全体 6280
//   k=9: P1 を cell 498..507、peak 506(後ろのピークだけ、閾値超は 507 まで)。q = 501..507 の
//        7 cells = 7·300 + 20 = 2120、全体 10·300 + 20 = 3020、幅 10
// eventTime = 2^40 + k·5,000,000(10 ns 刻み → 50 ms 間隔、48 bit 域で 32 bit に収まらない値)
const ULong64_t kT0 = 1099511627776ULL;  // 2^40
const ULong64_t kDt = 5000000ULL;

void fillEvent(int k)
{
  fillBaseline();
  pulse(1, 5, 100, 130, 150, 118, kBump);  // halo H1
  const int n = 1 + (k % 4);
  for (int p = 0; p < n; ++p) {
    if (k == 1 && p == 1) {
      pulse(1, 40, 500, 511, 300, 508, kBump);
    } else if (k == 2 && p == 0) {
      pulse(0, 3, 0, 0, 2500);
    } else if (k == 5 && p == 1) {
      setRaw(1, 40, 100, 130, 4095);
    } else if (k == 7 && p == 3) {
      pulse(2, 12, 490, 511, 400, 500, kBump);
    } else if (k == 9 && p == 1) {
      pulse(1, 40, 498, 507, 300, 506, kBump);
    } else {
      regular(p);
    }
  }
  if (k == 3) pulse(3, 50, 505, 511, 100, 511, kBump);  // halo H2
  if (k == 4) {
    pulse(2, 30, 200, 230, 180, 215, 20);  // Q2000
    pulse(2, 31, 200, 230, 180, 215, 19);  // Q1999
  }
  if (k == 6) pulse(0, 3, 0, 0, 60);
  if (k == 8) pulse(0, 3, 511, 511, 60);
}

bool writeRaw(const std::string &path, int nEvents, void (*fill)(int), ULong64_t t0, ULong64_t dt)
{
  TFile f(path.c_str(), "RECREATE");
  if (f.IsZombie()) return false;
  TTree tree("raw", "synthetic raw (test_run_metrics.C)");
  UInt_t eventId = 0;
  ULong64_t eventTime = 0;
  tree.Branch("eventId", &eventId);
  tree.Branch("eventTime", &eventTime);
  tree.Branch("adc", g_adc, "adc[4][68][512]/S");
  for (int k = 0; k < nEvents; ++k) {
    fill(k);
    eventId = static_cast<UInt_t>(k);
    eventTime = t0 + static_cast<ULong64_t>(k) * dt;
    tree.Fill();
  }
  tree.Write();
  return true;
}

// FPN ch にしか信号の無いイベント(全 ch に傾き s(t))→ ヒット 0。
void fillFpnOnly(int)
{
  fillBaseline();
  fpnDecoys();
}

// 部分読み出し(未読 ch は全セル 0 — tools/root_sink/raw_tree.hpp fill_entry):
//   AGET 0: FPN 4 ch が未読(isFPNRead=false の pulser 設定の形)。読めた信号 ch は P0 (0,3) だけ、
//           傾き無しの b(0,3) + ±2 + 通常の P0 → common mode を引けないので ped だけ。v = パルス
//           → ピーク 110、全体 6220。
//   AGET 1: FPN 4 ch と信号 ch P1 (1,40) だけ読めた。両方に s(t) と common mode の凹み
//           d(t) = −250(cell 300..330)、cell 315 だけ −270 → FPN で消えて P1 は通常どおり
//           (ピーク 115、全体 9320)。未読(全 0)の信号 ch 63 本を FPN で引くと
//           v = −(s + d) + 0.4 → cell 315 をピークに q ≈ 2512 ≥ 2000 になる → 除かないと偽ヒット。
//   AGET 2, 3: 全 ch 未読。
// → 1 イベントあたりヒット 2 本、FPN の無い AGET(読めた信号 ch あり)= AGET 0 の 1 つ。
int dip(int t)
{
  if (t == 315) return -270;
  return (t >= 300 && t <= 330) ? -250 : 0;
}

void fillPartial(int)
{
  std::memset(g_adc, 0, sizeof(g_adc));
  for (int t = 0; t < 512; ++t) {
    int v = baseline(0, 3);
    if (t >= 1 && t <= 20) v += (t % 2 == 1) ? -2 : +2;
    g_adc[0][3][t] = static_cast<Short_t>(v);
  }
  pulse(0, 3, 100, 130, 200, 110, kBump);
  for (int f : runmetrics::kFpn)
    for (int t = 0; t < 512; ++t)
      g_adc[1][f][t] = static_cast<Short_t>(baseline(1, f) + slope(t) + dip(t));
  for (int t = 0; t < 512; ++t) {
    int v = baseline(1, 40) + slope(t) + dip(t);
    if (t >= 1 && t <= 20) v += (t % 2 == 1) ? -2 : +2;
    g_adc[1][40][t] = static_cast<Short_t>(v);
  }
  pulse(1, 40, 100, 130, 300, 115, kBump);
}

// ---- 合成 tracks(makeTracks.C の "tracks" と同じ名前・型: ok = int、他は float)--------------
// e0: ok, dir (1,0,0)            → dip 0、φ 0
// e1: ok, dir (0, 1/√2, 1/√2)    → dip 45、φ 90
// e2: ok=0(ゴミ値 — 統計から外れること)
// e3: ok, dir (1/√2, −1/√2, 0)   → dip 0、φ −45
// e4: ok, dir (0.5, 0, −√3/2)    → dip −60、φ 0
// flipped = true: e0 / e1 / e3 の dir を反転(makeTracks 6 の頭尻はコイン投げ)。
//   e0 (−1,0,0) → φ ±180 → 畳んで 0、e1 (0,−1/√2,−1/√2) → φ −90 → 畳んで 90、dip −45 → |45|、
//   e3 (−1/√2, 1/√2, 0) → φ 135 → 畳んで −45。畳んだ値は反転前と同じ。
bool writeSyntheticTracks(const std::string &path, bool flipped)
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
    bool flip;
  };
  const Row rows[5] = {{1, 40, 1000, 25, 1, 0, 0, true},
                       {1, 50, 3000, 60, 0, r2, r2, true},
                       {0, 999, 99999, 999, 0, 0, 1, false},
                       {1, 60, 2000, 40, r2, -r2, 0, true},
                       {1, 80, 6000, 75, 0.5f, 0, -r3, false}};
  for (const Row &r : rows) {
    const float sign = (flipped && r.flip) ? -1.0f : 1.0f;
    ok = r.ok;
    length = r.length;
    charge = r.charge;
    dEdx = r.dEdx;
    dirX = sign * r.dx;
    dirY = sign * r.dy;
    dirZ = sign * r.dz;
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


// ---- ZS 対応(TODO/149 §2)の合成 raw ------------------------------------------------------------
// 傾き s(t) の無い平らな基線 b(a,c) = 200 + 3c + 50a に、全セル 奇数 cell −2 / 偶数 cell +2(512 cells
// で平均ちょうど 0 → 表の mean = b)。pulse は kPool の P0..P(n_k−1)、n_k = 1 + k(k = 0..3 → 1..4 本)。
// ZS 版 = 閾値 thr(a,c) = b + 10 未満のセルを 0 にしたもの(FPN ch は全セル b ± 2 < thr → 全 0)。
//   パルスのセルは b + 200 以上 ± 2 なので残る。
// 手計算: ch の全体 = 31·H + 20(bump)+ 雑音の和。cell 100..130 は偶数 16 個(+2)・奇数 15 個(−2) → +2。
//   P0 H200: 6222、P1 300: 9322。k0 = 6222、k1 = 6222 + 9322 = 15544、k2 = 15544 + 7772(P2 H250) = 23316、
//   k3 = 23316 + 12422(P3 H400) = 35738。charge_median(4 イベント、h = 1.5)= (15544 + 23316)/2 = 19430。
//   ヒット ch 数 [1,2,3,4] → median 2.5(halo 無し)。
namespace test_rm {
bool g_zsMode = false;

void fillFlat(int k)
{
  for (int a = 0; a < 4; ++a)
    for (int c = 0; c < 68; ++c)
      for (int t = 0; t < 512; ++t)
        g_adc[a][c][t] = static_cast<Short_t>(baseline(a, c) + ((t % 2 == 1) ? -2 : +2));
  const int n = 1 + (k % 4);
  for (int p = 0; p < n; ++p) regular(p);
  if (g_zsMode)
    for (int a = 0; a < 4; ++a)
      for (int c = 0; c < 68; ++c)
        for (int t = 0; t < 512; ++t)
          if (g_adc[a][c][t] < baseline(a, c) + 10) g_adc[a][c][t] = 0;
}
void fillFlatPlain(int k)
{
  g_zsMode = false;
  fillFlat(k);
}
void fillFlatZs(int k)
{
  g_zsMode = true;
  fillFlat(k);
  g_zsMode = false;
}

// calib pedestal の JSON(src/calib/pedestal.rs の形)。skipAsad/aget/ch が負でなければその ch を省く。
bool writePedTable(const std::string &path, int skipA, int skipC)
{
  FILE *fp = std::fopen(path.c_str(), "w");
  if (!fp) return false;
  std::fputs("{\"meta\": {\"note\": \"synthetic\"}, \"channels\": [\n", fp);
  bool first = true;
  for (int a = 0; a < 4; ++a)
    for (int c = 0; c < 68; ++c) {
      if (a == skipA && c == skipC) continue;
      std::fprintf(fp,
                   "%s{\"asad\": 0, \"aget\": %d, \"ch\": %d, \"mean\": %d.0, \"rms\": 1.5, "
                   "\"runs\": 1, \"is_fpn\": %s}",
                   first ? "" : ",\n", a, c, baseline(a, c), runmetrics::isFpn(c) ? "true" : "false");
      first = false;
    }
  std::fputs("\n]}\n", fp);
  return std::fclose(fp) == 0;
}
} // namespace test_rm

int test_run_metrics()
{
  using namespace test_rm;
  const std::string dir = std::string(gSystem->TempDirectory()) + "/test_run_metrics_" +
                          std::to_string(gSystem->GetPid());
  gSystem->mkdir(dir.c_str(), kTRUE);
  const std::string raw = dir + "/run0042_raw.root";
  const std::string tracks = dir + "/run0042_tracks.root";
  const std::string fpnRaw = dir + "/run0043_raw.root";
  const std::string partialRaw = dir + "/run0044_raw.root";
  const std::string json = dir + "/run0042_metrics.json";
  gSystem->Unlink(tracks.c_str());
  check(writeRaw(raw, 10, fillEvent, kT0, kDt), "write synthetic raw");
  // FPN だけ: 3 イベント、1 s 間隔(10 ns 刻みで 100,000,000)
  check(writeRaw(fpnRaw, 3, fillFpnOnly, 1000ULL, 100000000ULL), "write FPN-only raw");
  check(writeRaw(partialRaw, 2, fillPartial, 0ULL, 100000000ULL), "write partial-readout raw");

  // ---- tracks 無し ------------------------------------------------------------------------------
  std::printf("[synthetic raw with a common-mode slope, no tracks]\n");
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

  // イベントごとのヒット(ピーク cell)。halo H1 / H2 / Q1999 は入らない:
  //   k0 [110]、k1 [110,508]、k2 [0,115,120]、k3 [110,115,120,125]、k4 [110,215]、
  //   k5 [110,100]、k6 [110,115,120]、k7 [110,115,120,500]、k8 [110]、k9 [110,506]
  // 帯の位置 first = ピークの min: [110,110,0,110,110,100,110,110,110,110] → 昇順 [0,100,110×8]
  //   median h=4.5 → 110、p10 h=0.9 → 0 + 0.9·100 = 90、p90 h=8.1 → 110
  checkNear(m.first_hit_cell.median, 110, "first_hit_cell.median");
  checkNear(m.first_hit_cell.p10, 90, "first_hit_cell.p10");
  checkNear(m.first_hit_cell.p90, 110, "first_hit_cell.p90");
  // last = ピークの max: [110,508,120,125,215,110,120,500,110,506]
  //   → 昇順 [110,110,110,120,120,125,215,500,506,508]
  //   median h=4.5 → (120+125)/2 = 122.5、p10 h=0.9 → 110、p90 h=8.1 → 506 + 0.1·2 = 506.2
  checkNear(m.last_hit_cell.median, 122.5, "last_hit_cell.median");
  checkNear(m.last_hit_cell.p10, 110, "last_hit_cell.p10");
  checkNear(m.last_hit_cell.p90, 506.2, "last_hit_cell.p90");

  // 窓端クリップ(÷ 全イベント 10):
  //   peak: ピーク ≤ 5 = k2(0)→ 前 0.1。ピーク ≥ 506 = k1(508)、k9(506)→ 後ろ 0.2
  //         (k7 の 500 と halo H2 の 511 は数えない)
  //   edge: ヒット ch の閾値超が cell ≤ 2 = k2(cell 0)、k6(cell 0 の +60)→ 前 0.2。
  //         ≥ 509 = k1(511)、k7(511)、k8(cell 511 の +60)→ 後ろ 0.3(k9 は 507 まで、
  //         halo H2 はヒット ch でない)
  checkNear(m.clip_front_peak, 0.1, "clip_front_peak");
  checkNear(m.clip_back_peak, 0.2, "clip_back_peak");
  checkNear(m.clip_front_edge, 0.2, "clip_front_edge");
  checkNear(m.clip_back_edge, 0.3, "clip_back_edge");

  // 飽和: ヒット ch の閾値超セル数(イベント順)
  //   k0 31、k1 31+12 = 43、k2 1+31+31 = 63、k3 124、k4 31+31 = 62、k5 62、k6 1+31·3 = 94、
  //   k7 31·3+22 = 115、k8 32、k9 31+10 = 41 → 計 667。うち 4095 は k=5 の P1 の 31 cells
  checkNear(m.saturated_sample_frac, 31.0 / 667.0, "saturated_sample_frac", 1e-12);
  checkNear(m.saturated_event_frac, 0.1, "saturated_event_frac");

  // パルス幅(ヒット ch ごとの最長連続): 24 本(Σ ヒット数)= [1(k2 P0), 12(k1 P1), 22(k7 P3),
  //   10(k9 P1), 31×20] → 昇順 [1,10,12,22,31×20]、h = 23·0.5 = 11.5 → 31
  checkNear(m.pulse_width_cells_median, 31, "pulse_width_cells_median");

  // ヒット ch 数: [1,2,3,4,2,2,3,4,1,2] → 昇順 [1,1,2,2,2,2,3,3,4,4] → 中央値 2
  // (初版は傾きで全 256 信号 ch がヒット → 256)
  checkNear(m.hit_channels_median, 2, "hit_channels_median");

  // 総電荷(ヒット ch の閾値超の和、イベント順):
  //   k0 6220、k1 6220+3620 = 9840、k2 2500+9320+7770 = 19590、
  //   k3 6220+9320+7770+12420 = 35730、k4 6220+5600 = 11820、k5 6220+115216 = 121436、
  //   k6 6280+9320+7770 = 23370、k7 6220+9320+7770+8820 = 32130、k8 6280、k9 6220+3020 = 9240
  //   → 昇順 [6220,6280,9240,9840,11820,19590,23370,32130,35730,121436]
  //   → (11820+19590)/2 = 15705
  checkNear(m.charge_median, 15705, "charge_median");

  // tracks ファイルが無い → tracks は null
  check(!m.tracks.present, "tracks absent -> not present");
  const std::string js = runmetrics::toJson(m);
  check(contains(js, "\"tracks\": null"), "JSON has \"tracks\": null");
  check(contains(js, "\"run\": 42,"), "JSON has \"run\": 42");
  check(contains(js, "\"last_hit_cell\": {\"median\": 122.5, \"p10\": 110, \"p90\": 506.2}"),
        "JSON last_hit_cell object");
  check(contains(js, "\"clip_front_peak\": 0.1,\n  \"clip_front_edge\": 0.2,\n"
                     "  \"clip_back_peak\": 0.2,\n  \"clip_back_edge\": 0.3,\n"),
        "JSON clip_* values");
  for (const char *key :
       {"\"run\"", "\"raw_file\"", "\"events\"", "\"rate_hz\"", "\"first_hit_cell\"",
        "\"last_hit_cell\"", "\"clip_front_peak\"", "\"clip_front_edge\"", "\"clip_back_peak\"",
        "\"clip_back_edge\"", "\"saturated_sample_frac\"", "\"saturated_event_frac\"",
        "\"pulse_width_cells_median\"", "\"hit_channels_median\"", "\"charge_median\"",
        "\"tracks\""})
    check(contains(js, key), std::string("JSON key ") + key);
  // 初版のキー(§5 で peak / edge に分けた)は残さない
  check(!contains(js, "\"clip_front\":") && !contains(js, "\"clip_back\":"),
        "JSON has no old clip_front / clip_back");

  // エントリ関数がファイルを書く(終了コード 0)
  check(run_metrics(raw.c_str(), json.c_str(), 100.0) == 0, "run_metrics() returns 0");
  check(readFile(json) == js, "run_metrics() wrote the same JSON");

  // ---- FPN だけに信号 → 何もヒットしない --------------------------------------------------------
  std::printf("[synthetic raw, FPN channels only (+ slope)]\n");
  runmetrics::RunMetrics f;
  check(runmetrics::computeRunMetrics(fpnRaw, f, err), "computeRunMetrics ok (FPN only)");
  check(f.events == 3, "events == 3");
  checkNear(f.hit_channels_median, 0, "hit_channels_median (FPN ignored)");
  checkNear(f.charge_median, 0, "charge_median (FPN ignored)");
  checkNull(f.first_hit_cell.median, "first_hit_cell.median (no hit)");
  checkNull(f.last_hit_cell.p90, "last_hit_cell.p90 (no hit)");
  checkNull(f.saturated_sample_frac, "saturated_sample_frac (no hit cells)");
  checkNear(f.saturated_event_frac, 0, "saturated_event_frac (FPN ignored)");
  checkNull(f.pulse_width_cells_median, "pulse_width_cells_median (no hit)");
  // 3 イベント、1 s 間隔 → (3−1) / 2 s = 1 Hz
  checkNear(f.rate_hz, 1.0, "rate_hz (FPN only)");
  checkNear(f.clip_front_peak, 0, "clip_front_peak (FPN ignored)");
  checkNear(f.clip_front_edge, 0, "clip_front_edge (FPN ignored)");
  checkNear(f.clip_back_peak, 0, "clip_back_peak (FPN ignored)");
  checkNear(f.clip_back_edge, 0, "clip_back_edge (FPN ignored)");
  const std::string fjs = runmetrics::toJson(f);
  check(contains(fjs, "\"first_hit_cell\": {\"median\": null, \"p10\": null, \"p90\": null}"),
        "JSON null quantiles");

  // ---- 部分読み出し(未読 ch = 全セル 0)-------------------------------------------------------
  std::printf("[synthetic raw, partial readout]\n");
  runmetrics::RunMetrics p;
  check(runmetrics::computeRunMetrics(partialRaw, p, err), "computeRunMetrics ok (partial)");
  // 1 イベントあたり P0(AGET 0、FPN 無し)と P1(AGET 1)の 2 本。未読の 63 本は除く
  checkNear(p.hit_channels_median, 2, "hit_channels_median (all-zero channels skipped)");
  checkNear(p.first_hit_cell.median, 110, "first_hit_cell.median (partial)");
  checkNear(p.last_hit_cell.median, 115, "last_hit_cell.median (partial)");
  // 6220 + 9320(AGET 1 の凹みは FPN で消える)
  checkNear(p.charge_median, 15540, "charge_median (partial)");
  // FPN が未読で信号 ch は読めた AGET = AGET 0 × 2 イベント(全 ch 未読の AGET 2/3 は数えない)
  check(p.agets_without_fpn == 2, "agets_without_fpn == 2 (got " +
                                      std::to_string(p.agets_without_fpn) + ")");

  // ---- tracks あり ------------------------------------------------------------------------------
  std::printf("[synthetic raw + tracks]\n");
  check(writeSyntheticTracks(tracks, false), "write synthetic tracks");
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
  // |dip| [0,45,0,60] → √((0+2025+0+3600)/4) = √1406.25 = 37.5(float の dir なので 1e-4 deg)
  checkNear(t.tracks.dip_abs_rms_deg, 37.5, "tracks.dip_abs_rms_deg", 1e-4);
  // φ を (−90, 90] に畳む: [0,90,−45,0] → √((0+8100+2025+0)/4) = √2531.25 = 50.3115…
  checkNear(t.tracks.phi_axial_rms_deg, std::sqrt(2531.25), "tracks.phi_axial_rms_deg", 1e-4);
  const std::string tjs = runmetrics::toJson(t);
  check(contains(tjs, "\"tracks\": {\"n\": 5, \"ok_frac\": 0.8, "
                      "\"length\": {\"median\": 55, \"iqr\": 17.5}, "
                      "\"charge\": {\"median\": 2500, \"iqr\": 2000}, "
                      "\"dEdx\": {\"median\": 50, \"iqr\": 27.5}, "
                      "\"dip_abs_rms_deg\": "),
        "JSON tracks object");
  check(contains(tjs, "\"phi_axial_rms_deg\": ") && !contains(tjs, "\"phi_rms_deg\"") &&
            !contains(tjs, "\"dip_rms_deg\""),
        "JSON tracks keys: dip_abs_rms_deg / phi_axial_rms_deg (no old names)");

  // ---- tracks の頭尻を反転(e0 / e1 / e3)→ 畳んだ指標は変わらない --------------------------------
  // 反転後の生の φ = [±180, −90, 135, 0] なら RMS は √((32400+8100+18225+0)/4) = 121.2… に化ける。
  // 畳んだ φ = [0, 90, −45, 0]、|dip| = [0, 45, 0, 60] → 上と同じ値。
  std::printf("[synthetic raw + tracks, head/tail flipped]\n");
  check(writeSyntheticTracks(tracks, true), "write flipped synthetic tracks");
  runmetrics::RunMetrics r;
  check(runmetrics::computeRunMetrics(raw, r, err), "computeRunMetrics ok (flipped tracks)");
  checkNear(r.tracks.dip_abs_rms_deg, 37.5, "flipped: tracks.dip_abs_rms_deg", 1e-4);
  checkNear(r.tracks.phi_axial_rms_deg, std::sqrt(2531.25), "flipped: tracks.phi_axial_rms_deg",
            1e-4);
  checkNear(r.tracks.phi_axial_rms_deg, t.tracks.phi_axial_rms_deg,
            "flipped == unflipped: phi_axial_rms_deg", 1e-9);

  // ---- ZS 対応(TODO/149 §2)-----------------------------------------------------------------------
  std::printf("[ZS: pedestal table vs cells 1..20]\n");
  const std::string plainRaw = dir + "/run0045_raw.root";
  const std::string zsRaw = dir + "/run0046_raw.root";
  const std::string ped = dir + "/ped.json";
  const std::string pedMissing = dir + "/ped_missing.json";
  check(writeRaw(plainRaw, 4, fillFlatPlain, 0ULL, 100000000ULL), "write flat non-ZS raw");
  check(writeRaw(zsRaw, 4, fillFlatZs, 0ULL, 100000000ULL), "write flat ZS raw");
  check(writePedTable(ped, -1, -1), "write pedestal table");
  check(writePedTable(pedMissing, 0, 3), "write pedestal table without (AGET 0, ch 3)");
  runmetrics::RunMetrics nz, zt, zn, nt;
  check(runmetrics::computeRunMetrics(plainRaw, nz, err), "non-ZS, no table: ok");
  check(runmetrics::computeRunMetrics(plainRaw, nt, err, ped), "non-ZS, table: ok");
  check(runmetrics::computeRunMetrics(zsRaw, zt, err, ped), "ZS, table: ok");
  check(runmetrics::computeRunMetrics(zsRaw, zn, err), "ZS, no table: ok");
  // test_zs_table_matches_non_zs: 電荷・ヒット ch が非 ZS と一致(±5 %)
  // 表なしは FPN の common mode を引くので雑音も消える: 6220, 15540, 23310 → (15540 + 23310)/2 = 19425
  checkNear(nz.charge_median, 19425, "non-ZS no table: charge_median");
  checkNear(zt.charge_median, 19430, "ZS + table: charge_median", 19430 * 0.05);
  checkNear(zt.charge_median, nz.charge_median, "ZS + table == non-ZS: charge_median (5 %)",
            nz.charge_median * 0.05);
  checkNear(zt.hit_channels_median, 2.5, "ZS + table: hit_channels_median");
  checkNear(nt.charge_median, 19430, "non-ZS + table: charge_median", 19430 * 0.05);
  checkNear(zt.first_hit_cell.median, nz.first_hit_cell.median, "ZS + table == non-ZS: first_hit_cell");
  // ped_source と zs_suspected
  check(nz.ped_source == "cells_1_20" && !nz.zs_suspected, "non-ZS no table: cells_1_20, not suspected");
  check(zt.ped_source == "table", "ZS + table: ped_source == table");
  check(zt.agets_without_fpn == 0, "ZS + table: no FPN-missing count (common mode not used)");
  check(zt.ped_missing == 0, "ZS + table: ped_missing == 0");
  const std::string zjs = runmetrics::toJson(zt);
  check(contains(zjs, "\"ped_source\": \"table\"") && contains(zjs, "\"zs_suspected\": "),
        "JSON has ped_source and zs_suspected");
  check(contains(runmetrics::toJson(nz), "\"ped_source\": \"cells_1_20\""),
        "JSON ped_source == cells_1_20 without a table");
  // test_zs_suspected_without_table: ZS で表なし → 疑い + WARNING。非 ZS はどちらも出ない
  check(zn.zs_suspected, "ZS, no table: zs_suspected == true");
  check(contains(runmetrics::zsWarning(zn, false), "WARNING"), "ZS, no table: WARNING text");
  check(runmetrics::zsWarning(zt, true).empty(), "ZS + table: no WARNING");
  check(!nz.zs_suspected && runmetrics::zsWarning(nz, false).empty(),
        "non-ZS: not suspected, no WARNING");
  // ZS で表なし = 誤った ped(cells 1..20 が 0)→ 電荷が非 ZS から大きく外れる(WARNING が要る理由)
  check(std::fabs(zn.charge_median - 19430) > 19430 * 0.05, "ZS, no table: charge is wrong (>5 % off)");
  // 表に無い ch は飛ばして数える(黙って別の ped を使わない): (AGET 0, ch 3) = P0 は全 4 イベントにある
  runmetrics::RunMetrics pm;
  check(runmetrics::computeRunMetrics(zsRaw, pm, err, pedMissing), "ZS, table missing a ch: ok");
  check(pm.ped_missing == 4, "ped_missing == 4 (got " + std::to_string(pm.ped_missing) + ")");
  // P0 を除くとヒット ch 数 [0,1,2,3] → median 1.5
  checkNear(pm.hit_channels_median, 1.5, "table missing P0: hit_channels_median");
  // 壊れた・無い表はエラー
  runmetrics::RunMetrics em;
  check(!runmetrics::computeRunMetrics(zsRaw, em, err, dir + "/nope.json") && !err.empty(),
        "missing pedestal file: error");
  // run_metrics() 入口の第 4 引数
  const std::string zjson = dir + "/run0046_metrics.json";
  check(run_metrics(zsRaw.c_str(), zjson.c_str(), 100.0, ped.c_str()) == 0, "run_metrics() with pedJson");
  check(contains(readFile(zjson), "\"ped_source\": \"table\""), "run_metrics() JSON ped_source table");
  for (const std::string &q : {plainRaw, zsRaw, ped, pedMissing, zjson}) gSystem->Unlink(q.c_str());

  // 後片付け
  for (const std::string &q : {raw, tracks, fpnRaw, partialRaw, json}) gSystem->Unlink(q.c_str());
  gSystem->Unlink(dir.c_str());

  std::printf("%s: %d failure(s)\n", g_failures == 0 ? "GREEN" : "RED", g_failures);
  return g_failures;
}
