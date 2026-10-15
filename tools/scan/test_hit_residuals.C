// =============================================================================
// test_hit_residuals.C — hit_residuals.C の合成テスト(TODO/146 §2)。
//
//   root -l -b -q tools/scan/test_hit_residuals.C     → exit code = 失敗数(0 = green)
//
// 合成 uvw / tracks を一時ディレクトリに書き、hit_residuals.C で求めたヒットの z 残差の統計が
// 手計算どおりに戻ることを見る。分位点(中央値・MAD・電荷の 4 分位)は線形補間 h = (n−1)·q
// (run_metrics.C の quantile と同じ定義)。robust σ = 1.4826 × MAD、RMS = √(Σr²/n)(0 基準)。
//
// 合成データは makeTracks.C の l の座標系で作る(generate(): 冒頭の幾何を逆にたどる):
//   view v のヒット(l, q, r)→ slS = ms_v + l·(pitch_v·u)、slZ = z0 + tanθ·l + r
//   pitch_U = (−1, 0)、pitch_V = (0.5, +√3/2)、pitch_W = (0.5, −√3/2)、u = (cos φ, sin φ)
//   ms_v は view ごとに勝手に選ぶ(3 つの和 ≠ 0 = どの 1 点とも整合しない — makeTracks の l が
//   view ごとの電荷加重平均 ms_v 基準であることを縛る。start 点を素朴に射影すると
//   tanθ·(Σms_v / 3)/proj_v のずれが出る)。ms_v が本当に電荷加重平均になるよう、各 view の
//   ヒットは Σ q·l = 0 にする(fixture の自己チェック balanced())。
//   tracks の端点 = m + lo·u / m + hi·u(z = z0 + tanθ·lo / hi)、m = makeTracks 3 の最小二乗の anchor
//   = (2/3)·Σ_v pitch_v·ms_v(3 本の pitch は 120° ずつなので Σ p pᵀ = 1.5·I)。swapped は start / end の
//   入れ替え(makeTracks 6 の頭尻)。
//
// [1] run0083(5 イベント、手計算で全部の値を固定):
//   e0 (id 1000) ok=1、φ = 0°、ms = (U −8, V 13, W 2.5)、z0 = 60、tanθ = 0.5、[lo, hi] = [−12, 12]
//   e1 (id 1001) ok=1、φ = 60°、swapped、ms = (U 5, V −7.5, W 11)、z0 = 85、tanθ = −1.25、[−12, 12]
//      e0 / e1 の各 view のヒット: l = ±3(q 12000 = 級 D)、±6(8000 = C)、±9(5000 = B)、
//      ±12(3000 = A、端点ちょうど — 残す)+ 端の外に r = +50 の 2 本(除く): U は l = −20(q 3000)と
//      +30(q 2000)、V は −30(2000)と +20(3000)、W は ±20(2500)。どれも Σ q·l = 0 のままだが、
//      重み無しの平均 l は U +1 / V −1 / W 0 と view ごとに違う(全 view 同じずれなら anchor m が d_xy 方向に
//      同じだけ動いて吸収してしまい、重み無しの ms_v の誤りが見えない — Σ pitch_v pitch_vᵀ = 1.5·I)。
//      e0 で重み無しにすると予測が tanθ·(−δ_v + (2/3)Σ_w δ_w proj_w²) = U −0.25 / V +0.75 / W +0.25 mm ずれる
//      残差 r = b_v + s_c·k:  b = (U 0, V 0.25, W 0.5)、s = (A 2, B 1, C 0.5, D 0.25)、
//      k = e0 の −l 側 −1.5 / +l 側 +0.5、e1 の −l 側 −0.5 / +l 側 +1.5
//   e2 (id 1002) ok=0。U / V だけに z = 999、q = 100000 のゴミ、tracks の端点は e1 のまま
//      (makeTracks は ok=0 で端点を書き換えない)。width = 99 — 全部使わない
//   e3 (id 1003) ok=1、φ = 80°(proj_U = −cos 80° = −0.174、|proj| < 0.3 → U は使わない)、
//      U: l = ±3, ±6(q 5000、r = +50)→ hits_cut_proj 4。V / W: l = −20(q 3000)と +30(q 2000)、r = +50
//      → [−12, 12] の外で hits_cut_ends 4。ms = (0, 4, −4)、z0 = 40、tanθ = 0.2
//   e4 (id 1004) ok=1 だが lo = hi = 0(start == end — 向きが決まらない)→ events_degenerate、
//      ヒット(e0 と同じ配置、r = +50)も width(50)も使わない
//   width[3]: e0 (1, 2, 3)、e1 (1.5, 2.5, 3.5)、e2 (99, 99, 99)、e3 (2, 4, 1)、e4 (50, 50, 50)
//
//   手計算(使うヒット = e0 / e1 の l = ±3..±12、view ごとに 16 個、計 48):
//   S0 = {s_c·k} の 16 個 = ±3, ±1(A)、±1.5, ±0.5(B)、±0.75, ±0.25(C)、±0.375, ±0.125(D)
//     昇順 −3, −1.5, −1, −0.75, −0.5, −0.375, −0.25, −0.125, 0.125, 0.25, 0.375, 0.5, 0.75, 1, 1.5, 3
//     中央値 = (−0.125 + 0.125)/2 = 0、|S0| の中央値 = (0.5 + 0.75)/2 = 0.625
//     Σ S0² = 2(1 + 9) + 2(0.25 + 2.25) + 2(0.0625 + 0.5625) + 2(0.015625 + 0.140625) = 26.5625
//     → 平均 1.66015625
//   view v = b_v + S0 → median = b_v(U 0 / V 0.25 / W 0.5)、MAD = 0.625 → robust σ = 0.926625、
//     RMS = √(b_v² + 1.66015625) = U 1.288470508 / V √1.72265625 = 1.3125 / W √1.91015625 = 1.382084024
//   all(48)= S0 + {0, 0.25, 0.5}: median 0.25。|r − 0.25| = |S0 − 0.25| ∪ |S0| ∪ |S0 + 0.25| の昇順
//     0 ×2, 0.125 ×6, 0.25 ×4, 0.375 ×4, 0.5 ×6, 0.625 ×2(累積 24), 0.75 ×6 … → idx 23 = 0.625、
//     idx 24 = 0.75 → MAD 0.6875 → robust σ = 1.0192875。RMS = √(1.66015625 + 0.3125/3) = 1.328278177
//   電荷: 3000 / 5000 / 8000 / 12000 が 12 個ずつ(n = 48、h = 47·q)
//     q25 = v[11] + 0.75·(v[12] − v[11]) = 3000 + 0.75·2000 = 4500、q50 = h 23.5 → 6500、
//     q75 = h 35.25 → 8000 + 0.25·4000 = 9000。区間 [3000, 4500] / (4500, 6500] / (6500, 9000] / (9000, 12000]
//   級 c の 12 個 = s_c·{±0.5, ±1.5} + {0, 0.25, 0.5}: median 0.25、MAD = 1.0·s_c
//     (A: |r − 0.25| 昇順 0.75, 0.75, 1, 1, 1.25, 1.25, 2.75, … → (1.25 + 2.75)/2 = 2)
//     → robust σ A 2.9652 / B 1.4826 / C 0.7413 / D 0.37065(電荷が大きいほど小さい = S/N 依存の模型)
//     RMS = √(mean(s_c² k²) + 0.3125/3): A √(5 + 0.104166…) = 2.259240285、B 1.16368667、
//     C 0.6454972244、D 0.4269562819
//   counts: events 5、events_ok 4(e0 e1 e3 e4)、events_degenerate 1(e4)、hits 48、
//     hits_cut_ends = e0 6 + e1 6 + e3 4 = 16、hits_cut_proj = 4(e3 の U)
//   width の中央値(使ったイベント e0 e1 e3): U {1, 1.5, 2} → 1.5、V {2, 2.5, 4} → 2.5、W {3, 3.5, 1} → 3
//     (e2 を入れると U 1.75、e4 を入れると U 1.75)
//   maxEvents = 1(e0 だけ): hits 24、view median U (−0.375 + 0.125)/2 = −0.125 / V 0.125 / W 0.375、
//     all median 0.125、all robust σ 0.926625、width (1, 2, 3)
//   (上の値は numpy の median / percentile(linear)でも同じになることを確かめた)
//
// [2] run0084 ガウス残差(既知の σ = 0.8 mm、TRandom3 seed 4146、120 イベント × 3 view × 20 ヒット):
//   φ = 0 / 60 / 120°(全 view |proj| ≥ 0.5)、l = ±1.5k(k = 1..10、同じ q の対)、[lo, hi] = [−15, 15]
//   → hits 7200、切られるヒット 0。robust σ と RMS は σ の 5 % 以内(漸近の相対誤差 √(1.36/n) = 1.4 % と
//   √(1/2n) = 0.8 %)、|median| < 0.04(標準誤差 1.25σ/√n = 0.012)。view ごと(n 2400)は 8 %、
//   電荷 4 分位(n ≈ 1800、q は対で同値なので境界で ±1)は 10 %。
//
// [3] run0090 本物の macros/makeTracks.C との突き合わせ(読むだけ・実行するだけ):
//   真の幾何で uvw を書く: 点 (x, y) = c + λ·u、slS = pitch_v·(x, y) + c_v(view ごとの原点のずれ
//   c_v = (0, 6, −2) — channel_map の原点の模型)、slZ = z0 + tanθ·λ + r。全 view で同じ λ と q:
//   λ = −9, −6, −3, 0, 3, 6, 9, 12、q = 4000, 4000, 3000, 4000, 3000, 3000, 2000, 2000
//   (Σ q·λ = −36000 − 24000 − 9000 + 0 + 9000 + 18000 + 18000 + 24000 = 0 → ms_v = pitch_v·c + c_v で
//    makeTracks の l = λ。各 view の λ の分布が同じなので makeTracks 2 の向きも真の u に一致)
//   r: U = ρ(λ)、V = −ρ(λ)、W = 0、ρ = 0.5, −0.25, 1, 0, −0.75, 0.25, 1.5, −0.5(λ の順)
//   (U と V は同じ λ・同じ q なので Σ q·r = Σ q·λ·r = 0 — 電荷加重の直線の当てはめを動かさない →
//    makeTracks の直線は真の直線のまま、残差は r そのもの)
//   A (id 1) φ = 20°: proj_W = 0.5 cos 20° − (√3/2) sin 20° = 0.174 < 0.3 → W 8 本は使わない
//      c = (15, −10)、z0 = 70、tanθ = 0.4。重い側が l の負側 → makeTracks 6 で start / end が入れ替わる
//   B (id 2) φ = −50°: |proj| = 0.643 / 0.342 / 0.985(全部使う)、λ の符号を反転、c = (−5, 8)、z0 = 30、tanθ = −0.8
//   makeTracks 5 の分位: A は 16 個(U, V)で最初の 1 個の q/ΣQ = 0.08 > 0.02 → lo = 最小、最後の 1 個の前で
//   累積 0.96 ≤ 0.98 → hi = 最後から 2 番目 = 同じ λ = 12 の別 view → 端で切られるヒット 0(端点ちょうどのヒットを残す)
//   手計算: hits = 16 + 24 = 40、hits_cut_proj 8、hits_cut_ends 0。
//     U(16 = ρ × 2): median = 昇順 −0.75, −0.5, −0.25, 0, 0.25, 0.5, 1, 1.5 の (0 + 0.25)/2 = 0.125、
//       |ρ − 0.125| 昇順 0.125, 0.125, 0.375, 0.375, 0.625, 0.875, 0.875, 1.375 → MAD 0.5 → robust σ 0.7413、
//       RMS = √((0.25 + 0.0625 + 1 + 0 + 0.5625 + 0.0625 + 2.25 + 0.25)/8) = √0.5546875 = 0.7447734559
//     V = −U: median −0.125、robust σ 0.7413。W(B の 8 個、全部 0): n 8、median 0、RMS 0
//
// [4] 異常系: 無い uvw / tracks → 1、エントリ数の不一致・eventId の不一致・ok=1 なのに view にヒットが無い → 失敗
// [5] 使えるイベントが無い run(ok=0 だけ)→ 値は null、JSON も null、入口は 0 を返す
// =============================================================================
#include "hit_residuals.C"

#include <TROOT.h>
#include <TRandom3.h>
#include <TSystem.h>

#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <string>
#include <vector>

namespace test_hr {

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

void checkNear(double got, double want, const std::string &what, double tol = 1e-4)
{
  const bool ok = std::isfinite(got) && std::fabs(got - want) <= tol;
  char buf[256];
  std::snprintf(buf, sizeof(buf), "%s: got %.10g want %.10g (tol %g)", what.c_str(), got, want, tol);
  check(ok, buf);
}

// 相対誤差 relTol 以内。
void checkRel(double got, double want, double relTol, const std::string &what)
{
  checkNear(got, want, what, relTol * std::fabs(want));
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

// JSON の anchor(例 `"V": {`)の後ろで最初に出る `"key": <数>` の数。無ければ NaN(null も NaN)。
double jsonNum(const std::string &js, const std::string &anchor, const std::string &key)
{
  size_t p = js.find(anchor);
  if (p == std::string::npos) return std::nan("");
  p = js.find("\"" + key + "\": ", p);
  if (p == std::string::npos) return std::nan("");
  p += key.size() + 4;
  if (js.compare(p, 4, "null") == 0) return std::nan("");
  return std::atof(js.c_str() + p);
}

// ---- 合成 uvw / tracks ------------------------------------------------------------------------
const double kSqrt3Half = std::sqrt(3.0) / 2;
const double kP[3][2] = {{-1, 0}, {0.5, kSqrt3Half}, {0.5, -kSqrt3Half}};  // makeTracks.C:7-8
const double kDeg = M_PI / 180;

struct UvwRow {
  UInt_t eventId = 0;
  float width[3] = {0, 0, 0};
  std::vector<int> view;
  std::vector<float> z, s, q;
};

struct TrackRow {
  UInt_t eventId = 0;
  int ok = 0;
  float xs = 0, ys = 0, zs = 0, xe = 0, ye = 0, ze = 0;
};

bool writeUvw(const std::string &path, const std::vector<UvwRow> &rows)
{
  TFile f(path.c_str(), "RECREATE");
  if (f.IsZombie()) return false;
  TTree t("uvw", "synthetic uvw (test_hit_residuals.C)");
  UInt_t eventId = 0;
  float width[3] = {0, 0, 0};
  std::vector<int> view;
  std::vector<float> z, s, q;
  t.Branch("eventId", &eventId);
  t.Branch("width", width, "width[3]/F");
  t.Branch("slView", &view);
  t.Branch("slZ", &z);
  t.Branch("slS", &s);
  t.Branch("slQ", &q);
  for (const UvwRow &r : rows) {
    eventId = r.eventId;
    for (int v = 0; v < 3; ++v) width[v] = r.width[v];
    view = r.view;
    z = r.z;
    s = r.s;
    q = r.q;
    t.Fill();
  }
  t.Write();
  return true;
}

bool writeTracks(const std::string &path, const std::vector<TrackRow> &rows)
{
  TFile f(path.c_str(), "RECREATE");
  if (f.IsZombie()) return false;
  TTree t("tracks", "synthetic tracks (test_hit_residuals.C)");
  TrackRow r;
  t.Branch("eventId", &r.eventId);
  t.Branch("ok", &r.ok);
  t.Branch("xStart", &r.xs);
  t.Branch("yStart", &r.ys);
  t.Branch("zStart", &r.zs);
  t.Branch("xEnd", &r.xe);
  t.Branch("yEnd", &r.ye);
  t.Branch("zEnd", &r.ze);
  for (const TrackRow &x : rows) {
    r = x;
    t.Fill();
  }
  t.Write();
  return true;
}

// makeTracks の l の座標系でのヒット(view, l, q, r)と直線。冒頭の generate() の式。
struct GenHit {
  int view;
  double l, q, r;
};

struct GenTrack {
  UInt_t id = 0;
  int ok = 1;
  double phiDeg = 0;
  double ms[3] = {0, 0, 0};
  double z0 = 0, tanTh = 0, lo = 0, hi = 0;
  bool swapped = false;
  float width[3] = {0, 0, 0};
};

// fixture の自己チェック: 各 view で Σ q·l = 0(ms_v が電荷加重平均になる条件)。
bool balanced(const std::vector<GenHit> &hits)
{
  double sql[3] = {0, 0, 0};
  for (const GenHit &h : hits) sql[h.view] += h.q * h.l;
  return std::fabs(sql[0]) < 1e-9 && std::fabs(sql[1]) < 1e-9 && std::fabs(sql[2]) < 1e-9;
}

void generate(const GenTrack &g, const std::vector<GenHit> &hits, std::vector<UvwRow> &uvw,
              std::vector<TrackRow> &tracks)
{
  const double ux = std::cos(g.phiDeg * kDeg), uy = std::sin(g.phiDeg * kDeg);
  UvwRow u;
  u.eventId = g.id;
  for (int v = 0; v < 3; ++v) u.width[v] = g.width[v];
  for (const GenHit &h : hits) {
    const double proj = kP[h.view][0] * ux + kP[h.view][1] * uy;
    u.view.push_back(h.view);
    u.s.push_back(static_cast<float>(g.ms[h.view] + h.l * proj));
    u.z.push_back(static_cast<float>(g.z0 + g.tanTh * h.l + h.r));
    u.q.push_back(static_cast<float>(h.q));
  }
  uvw.push_back(u);
  // anchor m = (2/3) Σ pitch_v ms_v
  double mx = 0, my = 0;
  for (int v = 0; v < 3; ++v) {
    mx += kP[v][0] * g.ms[v] * 2.0 / 3.0;
    my += kP[v][1] * g.ms[v] * 2.0 / 3.0;
  }
  TrackRow t;
  t.eventId = g.id;
  t.ok = g.ok;
  double l1 = g.lo, l2 = g.hi;
  if (g.swapped) std::swap(l1, l2);
  t.xs = static_cast<float>(mx + l1 * ux);
  t.ys = static_cast<float>(my + l1 * uy);
  t.zs = static_cast<float>(g.z0 + g.tanTh * l1);
  t.xe = static_cast<float>(mx + l2 * ux);
  t.ye = static_cast<float>(my + l2 * uy);
  t.ze = static_cast<float>(g.z0 + g.tanTh * l2);
  tracks.push_back(t);
}

// [1] の e0 / e1 のヒット(冒頭の表)。kMinus / kPlus = −l 側 / +l 側の k。
std::vector<GenHit> mainHits(double kMinus, double kPlus)
{
  const double bias[3] = {0.0, 0.25, 0.5};
  const double absL[4] = {3, 6, 9, 12};
  const double charge[4] = {12000, 8000, 5000, 3000};  // D C B A
  const double scale[4] = {0.25, 0.5, 1.0, 2.0};
  std::vector<GenHit> hits;
  for (int v = 0; v < 3; ++v) {
    for (int c = 0; c < 4; ++c) {
      hits.push_back({v, -absL[c], charge[c], bias[v] + scale[c] * kMinus});
      hits.push_back({v, +absL[c], charge[c], bias[v] + scale[c] * kPlus});
    }
  }
  // 端の外(r = +50、除く)。view ごとに Σ q·l = 0 だが重み無しの平均 l は U +1 / V −1 / W 0 と違う
  hits.push_back({0, -20, 3000, 50});
  hits.push_back({0, +30, 2000, 50});
  hits.push_back({1, -30, 2000, 50});
  hits.push_back({1, +20, 3000, 50});
  hits.push_back({2, -20, 2500, 50});
  hits.push_back({2, +20, 2500, 50});
  return hits;
}

bool g_balanced = true;  // [1] の fixture の自己チェック

void buildMain(std::vector<UvwRow> &uvw, std::vector<TrackRow> &tracks)
{
  // e0
  GenTrack e0;
  e0.id = 1000;
  e0.phiDeg = 0;
  e0.ms[0] = -8;
  e0.ms[1] = 13;
  e0.ms[2] = 2.5;
  e0.z0 = 60;
  e0.tanTh = 0.5;
  e0.lo = -12;
  e0.hi = 12;
  e0.width[0] = 1;
  e0.width[1] = 2;
  e0.width[2] = 3;
  const std::vector<GenHit> h0 = mainHits(-1.5, 0.5);
  g_balanced = g_balanced && balanced(h0);
  generate(e0, h0, uvw, tracks);
  // e1
  GenTrack e1;
  e1.id = 1001;
  e1.phiDeg = 60;
  e1.swapped = true;
  e1.ms[0] = 5;
  e1.ms[1] = -7.5;
  e1.ms[2] = 11;
  e1.z0 = 85;
  e1.tanTh = -1.25;
  e1.lo = -12;
  e1.hi = 12;
  e1.width[0] = 1.5;
  e1.width[1] = 2.5;
  e1.width[2] = 3.5;
  const std::vector<GenHit> h1 = mainHits(-0.5, 1.5);
  g_balanced = g_balanced && balanced(h1);
  generate(e1, h1, uvw, tracks);
  // e2: ok = 0、U / V だけにゴミ、端点は e1 のまま
  UvwRow u2;
  u2.eventId = 1002;
  for (int v = 0; v < 3; ++v) u2.width[v] = 99;
  for (int v = 0; v < 2; ++v)
    for (int k = 0; k < 3; ++k) {
      u2.view.push_back(v);
      u2.s.push_back(static_cast<float>(k));
      u2.z.push_back(999);
      u2.q.push_back(100000);
    }
  uvw.push_back(u2);
  TrackRow t2 = tracks.back();
  t2.eventId = 1002;
  t2.ok = 0;
  tracks.push_back(t2);
  // e3: φ = 80°、U は |proj| < 0.3、V / W は端の外だけ
  GenTrack e3;
  e3.id = 1003;
  e3.phiDeg = 80;
  e3.ms[0] = 0;
  e3.ms[1] = 4;
  e3.ms[2] = -4;
  e3.z0 = 40;
  e3.tanTh = 0.2;
  e3.lo = -12;
  e3.hi = 12;
  e3.width[0] = 2;
  e3.width[1] = 4;
  e3.width[2] = 1;
  std::vector<GenHit> h3;
  for (double l : {-6.0, -3.0, 3.0, 6.0}) h3.push_back({0, l, 5000, 50});
  for (int v = 1; v < 3; ++v) {
    h3.push_back({v, -20, 3000, 50});
    h3.push_back({v, +30, 2000, 50});
  }
  g_balanced = g_balanced && balanced(h3);
  generate(e3, h3, uvw, tracks);
  // e4: start == end
  GenTrack e4 = e0;
  e4.id = 1004;
  e4.lo = 0;
  e4.hi = 0;
  for (int v = 0; v < 3; ++v) e4.width[v] = 50;
  std::vector<GenHit> h4 = mainHits(-1.5, 0.5);
  for (GenHit &h : h4) h.r = 50;
  generate(e4, h4, uvw, tracks);
}

// [2] ガウス残差。
void buildGauss(std::vector<UvwRow> &uvw, std::vector<TrackRow> &tracks)
{
  TRandom3 rnd(4146);
  const double phis[3] = {0, 60, 120};
  for (int e = 0; e < 120; ++e) {
    GenTrack g;
    g.id = static_cast<UInt_t>(e);
    g.phiDeg = phis[e % 3];
    g.swapped = (e % 2 == 1);
    for (int v = 0; v < 3; ++v) g.ms[v] = 5.0 * v - 3.0 + (e % 5);
    g.z0 = 50 + (e % 11);
    g.tanTh = -1.0 + 0.25 * (e % 9);
    g.lo = -15;
    g.hi = 15;
    for (int v = 0; v < 3; ++v) g.width[v] = 2;
    std::vector<GenHit> hits;
    for (int v = 0; v < 3; ++v)
      for (int k = 1; k <= 10; ++k) {
        const double q = 2000 + 18000 * rnd.Rndm();
        hits.push_back({v, -1.5 * k, q, rnd.Gaus(0, 0.8)});
        hits.push_back({v, +1.5 * k, q, rnd.Gaus(0, 0.8)});
      }
    generate(g, hits, uvw, tracks);
  }
}

// [3] 真の幾何(冒頭)で uvw を書く。tracks は本物の makeTracks.C が作る。
void addTrueEvent(UInt_t id, double phiDeg, double cx, double cy, double z0, double tanTh,
                  double lambdaSign, std::vector<UvwRow> &uvw)
{
  const double lambda[8] = {-9, -6, -3, 0, 3, 6, 9, 12};
  const double charge[8] = {4000, 4000, 3000, 4000, 3000, 3000, 2000, 2000};
  const double rho[8] = {0.5, -0.25, 1, 0, -0.75, 0.25, 1.5, -0.5};
  const double offset[3] = {0, 6, -2};
  const double ux = std::cos(phiDeg * kDeg), uy = std::sin(phiDeg * kDeg);
  UvwRow u;
  u.eventId = id;
  for (int v = 0; v < 3; ++v) u.width[v] = 1;
  for (int v = 0; v < 3; ++v)
    for (int i = 0; i < 8; ++i) {
      const double lam = lambdaSign * lambda[i];
      const double x = cx + lam * ux, y = cy + lam * uy;
      const double r = (v == 0) ? rho[i] : (v == 1) ? -rho[i] : 0.0;
      u.view.push_back(v);
      u.s.push_back(static_cast<float>(kP[v][0] * x + kP[v][1] * y + offset[v]));
      u.z.push_back(static_cast<float>(z0 + tanTh * lam + r));
      u.q.push_back(static_cast<float>(charge[i]));
    }
  uvw.push_back(u);
}

}  // namespace test_hr

int test_hit_residuals()
{
  using namespace test_hr;
  const std::string dir = std::string(gSystem->TempDirectory()) + "/test_hit_residuals_" +
                          std::to_string(gSystem->GetPid());
  gSystem->mkdir(dir.c_str(), kTRUE);
  std::vector<std::string> cleanup;

  // ---- [1] run0083: 手計算の値 ---------------------------------------------------------------
  std::printf("[1] synthetic run0083: offsets per view, swapped ends, ok=0, |proj| < 0.3, ends, degenerate\n");
  const std::string uvw83 = dir + "/run0083_uvw.root", trk83 = dir + "/run0083_tracks.root";
  const std::string json83 = dir + "/run0083_hit_residuals.json", png83 = dir + "/run0083_hit_residuals.png";
  cleanup.insert(cleanup.end(), {uvw83, trk83, json83, png83});
  std::vector<UvwRow> u83;
  std::vector<TrackRow> t83;
  buildMain(u83, t83);
  check(g_balanced, "fixture: sum q*l = 0 per view (ms_v is the charge-weighted mean)");
  check(writeUvw(uvw83, u83) && writeTracks(trk83, t83), "write synthetic uvw / tracks (5 events)");

  hitres::Result r;
  std::string err;
  check(hitres::computeHitResiduals(uvw83, trk83, 0, r, err), "computeHitResiduals ok (" + err + ")");
  checkCount(r.run, 83, "run (from run0083_uvw.root)");
  checkCount(r.events, 5, "events");
  checkCount(r.events_ok, 4, "events_ok (e2 has ok = 0)");
  checkCount(r.events_degenerate, 1, "events_degenerate (e4: start == end)");
  checkCount(r.hits, 48, "hits (16 per view from e0 / e1)");
  checkCount(r.hits_cut_ends, 16, "hits_cut_ends (e0 6 + e1 6 + e3 4)");
  checkCount(r.hits_cut_proj, 4, "hits_cut_proj (e3 U, |proj| = 0.174 < 0.3)");
  // view: median = b_v、robust σ = 1.4826 × 0.625、RMS = √(b_v² + 1.66015625)
  const double wantMedian[3] = {0, 0.25, 0.5};
  const double wantRms[3] = {1.288470508, 1.3125, 1.382084024};
  for (int v = 0; v < 3; ++v) {
    const std::string name = std::string("view ") + hitres::kViewName[v];
    checkCount(r.view[v].n, 16, name + " n");
    checkNear(r.view[v].median, wantMedian[v], name + " median (bias b_v)");
    checkNear(r.view[v].robust_sigma, 0.926625, name + " robust_sigma (1.4826 x 0.625)");
    checkNear(r.view[v].rms, wantRms[v], name + " rms");
  }
  checkCount(r.all.n, 48, "all n");
  checkNear(r.all.median, 0.25, "all median");
  checkNear(r.all.robust_sigma, 1.0192875, "all robust_sigma (1.4826 x 0.6875)");
  checkNear(r.all.rms, 1.328278177, "all rms");
  // 電荷 4 分位: 区間 [3000, 4500] / (4500, 6500] / (6500, 9000] / (9000, 12000]
  const double wantLo[4] = {3000, 4500, 6500, 9000}, wantHi[4] = {4500, 6500, 9000, 12000};
  const double wantSigma[4] = {2.9652, 1.4826, 0.7413, 0.37065};
  const double wantQRms[4] = {2.259240285, 1.16368667, 0.6454972244, 0.4269562819};
  for (int b = 0; b < 4; ++b) {
    const std::string name = "charge quartile " + std::to_string(b);
    checkNear(r.quartile[b].q_lo, wantLo[b], name + " q_lo");
    checkNear(r.quartile[b].q_hi, wantHi[b], name + " q_hi");
    checkCount(r.quartile[b].s.n, 12, name + " n");
    checkNear(r.quartile[b].s.median, 0.25, name + " median");
    checkNear(r.quartile[b].s.robust_sigma, wantSigma[b], name + " robust_sigma");
    checkNear(r.quartile[b].s.rms, wantQRms[b], name + " rms");
  }
  // width の中央値(e0 e1 e3)
  checkNear(r.width_median[0], 1.5, "width median U (e2 / e4 not used)");
  checkNear(r.width_median[1], 2.5, "width median V");
  checkNear(r.width_median[2], 3.0, "width median W");
  checkCount(static_cast<long>(r.used.size()), 48, "hits kept for the PNG");

  // ---- JSON ------------------------------------------------------------------------------------
  const std::string js = hitres::toJson(r);
  check(contains(js, "\"run\": 83,"), "JSON \"run\": 83");
  check(contains(js, "\"events\": 5,"), "JSON \"events\": 5");
  check(contains(js, "\"events_ok\": 4,"), "JSON \"events_ok\": 4");
  check(contains(js, "\"events_degenerate\": 1,"), "JSON \"events_degenerate\": 1");
  check(contains(js, "\"hits\": 48,"), "JSON \"hits\": 48");
  check(contains(js, "\"hits_cut_ends\": 16,"), "JSON \"hits_cut_ends\": 16");
  check(contains(js, "\"hits_cut_proj\": 4,"), "JSON \"hits_cut_proj\": 4");
  check(contains(js, "\"min_proj\": 0.3,"), "JSON \"min_proj\": 0.3");
  for (const char *key : {"\"uvw_file\"", "\"tracks_file\"", "\"residual_mm\"", "\"charge_quartiles\"",
                          "\"width_median_mm\""})
    check(contains(js, key), std::string("JSON key ") + key);
  checkNear(jsonNum(js, "\"all\": {", "robust_sigma"), 1.0192875, "JSON residual_mm.all.robust_sigma");
  checkNear(jsonNum(js, "\"all\": {", "rms"), 1.328278177, "JSON residual_mm.all.rms");
  checkNear(jsonNum(js, "\"V\": {", "median"), 0.25, "JSON residual_mm.V.median");
  checkNear(jsonNum(js, "\"W\": {", "n"), 16, "JSON residual_mm.W.n");
  checkNear(jsonNum(js, "\"charge_quartiles\": [", "q_hi"), 4500, "JSON charge_quartiles[0].q_hi");
  checkNear(jsonNum(js, "\"charge_quartiles\": [", "robust_sigma"), 2.9652,
            "JSON charge_quartiles[0].robust_sigma");
  checkNear(jsonNum(js, "\"width_median_mm\": {", "W"), 3.0, "JSON width_median_mm.W");

  // ---- 入口: JSON と PNG を書く ------------------------------------------------------------------
  check(hit_residuals(uvw83.c_str(), trk83.c_str(), json83.c_str(), png83.c_str(), 0) == 0,
        "hit_residuals() returns 0");
  check(!js.empty() && readFile(json83) == js, "hit_residuals() wrote the same JSON");
  FileStat_t st;
  check(gSystem->GetPathInfo(png83.c_str(), st) == 0 && st.fSize > 0, "hit_residuals() wrote the PNG");

  // ---- maxEvents = 1(e0 だけ)--------------------------------------------------------------------
  std::printf("[1b] maxEvents = 1\n");
  hitres::Result m1;
  check(hitres::computeHitResiduals(uvw83, trk83, 1, m1, err), "computeHitResiduals ok (maxEvents 1)");
  checkCount(m1.events, 1, "events (maxEvents 1)");
  checkCount(m1.events_ok, 1, "events_ok (maxEvents 1)");
  checkCount(m1.hits, 24, "hits (maxEvents 1)");
  checkCount(m1.hits_cut_ends, 6, "hits_cut_ends (maxEvents 1)");
  checkNear(m1.view[0].median, -0.125, "view U median (maxEvents 1)");
  checkNear(m1.view[1].median, 0.125, "view V median (maxEvents 1)");
  checkNear(m1.view[2].median, 0.375, "view W median (maxEvents 1)");
  checkNear(m1.all.median, 0.125, "all median (maxEvents 1)");
  checkNear(m1.all.robust_sigma, 0.926625, "all robust_sigma (maxEvents 1)");
  checkNear(m1.width_median[0], 1.0, "width median U (maxEvents 1)");
  checkNear(m1.width_median[2], 3.0, "width median W (maxEvents 1)");

  // ---- [2] ガウス残差 ---------------------------------------------------------------------------
  std::printf("[2] synthetic run0084: Gaussian residuals, sigma = 0.8 mm\n");
  const std::string uvw84 = dir + "/run0084_uvw.root", trk84 = dir + "/run0084_tracks.root";
  cleanup.insert(cleanup.end(), {uvw84, trk84});
  std::vector<UvwRow> u84;
  std::vector<TrackRow> t84;
  buildGauss(u84, t84);
  check(writeUvw(uvw84, u84) && writeTracks(trk84, t84), "write Gaussian uvw / tracks (120 events)");
  hitres::Result g;
  check(hitres::computeHitResiduals(uvw84, trk84, 0, g, err), "computeHitResiduals ok (Gaussian)");
  checkCount(g.hits, 7200, "hits (Gaussian)");
  checkCount(g.hits_cut_ends + g.hits_cut_proj, 0, "no hit cut (Gaussian, |l| <= 15 = hi)");
  checkRel(g.all.robust_sigma, 0.8, 0.05, "all robust_sigma ~ sigma 0.8 (5 %)");
  checkRel(g.all.rms, 0.8, 0.05, "all rms ~ sigma 0.8 (5 %)");
  checkNear(g.all.median, 0.0, "all median ~ 0", 0.04);
  for (int v = 0; v < 3; ++v)
    checkRel(g.view[v].robust_sigma, 0.8, 0.08,
             std::string("view ") + hitres::kViewName[v] + " robust_sigma ~ 0.8 (8 %)");
  long nq = 0;
  for (int b = 0; b < 4; ++b) {
    nq += g.quartile[b].s.n;
    check(std::labs(g.quartile[b].s.n - 1800) <= 2,
          "charge quartile " + std::to_string(b) + " n = " + std::to_string(g.quartile[b].s.n) +
              " (1800 +- 2)");
    checkRel(g.quartile[b].s.robust_sigma, 0.8, 0.10,
             "charge quartile " + std::to_string(b) + " robust_sigma ~ 0.8 (10 %)");
  }
  checkCount(nq, 7200, "charge quartiles cover every hit");
  check(g.quartile[0].q_lo >= 2000 && g.quartile[3].q_hi <= 20000 &&
            g.quartile[0].q_hi < g.quartile[1].q_hi && g.quartile[1].q_hi < g.quartile[2].q_hi,
        "charge quartile edges inside [2000, 20000] and increasing");

  // ---- [3] 本物の makeTracks.C ------------------------------------------------------------------
  std::printf("[3] run0090: tracks from the real macros/makeTracks.C\n");
  const std::string macro =
      std::string(gSystem->GetDirName(__FILE__).Data()) + "/../../macros/makeTracks.C";
  const std::string uvw90 = dir + "/run0090_uvw.root", trk90 = dir + "/run0090_tracks.root";
  cleanup.insert(cleanup.end(), {uvw90, trk90});
  std::vector<UvwRow> u90;
  addTrueEvent(1, 20, 15, -10, 70, 0.4, +1, u90);
  addTrueEvent(2, -50, -5, 8, 30, -0.8, -1, u90);
  check(writeUvw(uvw90, u90), "write true-geometry uvw (2 events)");
  check(runmetrics::fileExists(macro), "macros/makeTracks.C found at " + macro);
  gROOT->ProcessLine((".L " + macro).c_str());
  gROOT->ProcessLine(("makeTracks(\"" + uvw90 + "\")").c_str());
  check(runmetrics::fileExists(trk90), "makeTracks.C wrote run0090_tracks.root");
  hitres::Result mk;
  check(hitres::computeHitResiduals(uvw90, trk90, 0, mk, err), "computeHitResiduals ok (makeTracks) (" + err + ")");
  checkCount(mk.events_ok, 2, "events_ok (makeTracks)");
  checkCount(mk.hits, 40, "hits (makeTracks: A U+V 16, B 24)");
  checkCount(mk.hits_cut_proj, 8, "hits_cut_proj (makeTracks: A W, |proj| 0.174)");
  checkCount(mk.hits_cut_ends, 0, "hits_cut_ends (makeTracks: boundary hits kept)");
  checkCount(mk.view[0].n, 16, "view U n (makeTracks)");
  checkNear(mk.view[0].median, 0.125, "view U median = median(rho) (makeTracks)");
  checkNear(mk.view[0].robust_sigma, 0.7413, "view U robust_sigma (makeTracks)");
  checkNear(mk.view[0].rms, 0.7447734559, "view U rms (makeTracks)");
  checkNear(mk.view[1].median, -0.125, "view V median = -median(rho) (makeTracks)");
  checkNear(mk.view[1].robust_sigma, 0.7413, "view V robust_sigma (makeTracks)");
  checkCount(mk.view[2].n, 8, "view W n (makeTracks: event B only)");
  checkNear(mk.view[2].median, 0.0, "view W median (makeTracks)");
  checkNear(mk.view[2].rms, 0.0, "view W rms = 0 (makeTracks)");

  // ---- [4] 異常系 --------------------------------------------------------------------------------
  std::printf("[4] errors\n");
  const std::string json = dir + "/err.json";
  cleanup.push_back(json);
  check(hit_residuals((dir + "/missing_uvw.root").c_str(), trk83.c_str(), json.c_str()) == 1,
        "hit_residuals() returns 1 for a missing uvw file");
  check(hit_residuals(uvw83.c_str(), (dir + "/missing_tracks.root").c_str(), json.c_str()) == 1,
        "hit_residuals() returns 1 for a missing tracks file");
  const std::string bad = dir + "/bad_tracks.root";
  cleanup.push_back(bad);
  std::vector<TrackRow> shortRows(t83.begin(), t83.end() - 1);
  check(writeTracks(bad, shortRows), "write tracks with 4 of 5 entries");
  hitres::Result x;
  err.clear();
  check(!hitres::computeHitResiduals(uvw83, bad, 0, x, err) && contains(err, "entries"),
        "entry count mismatch fails (" + err + ")");
  std::vector<TrackRow> shifted = t83;
  shifted[3].eventId = 7777;
  check(writeTracks(bad, shifted), "write tracks with a different eventId at entry 3");
  err.clear();
  check(!hitres::computeHitResiduals(uvw83, bad, 0, x, err) && contains(err, "eventId"),
        "eventId mismatch fails (" + err + ")");
  std::vector<TrackRow> noW = t83;
  noW[2].ok = 1;  // e2 は W にヒットが無い
  check(writeTracks(bad, noW), "write tracks with ok = 1 on e2 (no W hit)");
  err.clear();
  check(!hitres::computeHitResiduals(uvw83, bad, 0, x, err) && contains(err, "no hit"),
        "ok = 1 with a view without hits fails (" + err + ")");

  // ---- [5] 使えるイベントが無い run → null ---------------------------------------------------------
  std::printf("[5] run without usable events\n");
  const std::string uvw85 = dir + "/run0085_uvw.root", trk85 = dir + "/run0085_tracks.root";
  const std::string json85 = dir + "/run0085_hit_residuals.json";
  cleanup.insert(cleanup.end(), {uvw85, trk85, json85});
  std::vector<UvwRow> u85(u83.begin() + 2, u83.begin() + 3);  // e2 だけ
  std::vector<TrackRow> t85(t83.begin() + 2, t83.begin() + 3);
  check(writeUvw(uvw85, u85) && writeTracks(trk85, t85), "write ok=0-only uvw / tracks");
  hitres::Result z;
  check(hitres::computeHitResiduals(uvw85, trk85, 0, z, err), "computeHitResiduals ok (no usable event)");
  checkCount(z.events, 1, "events (no usable event)");
  checkCount(z.hits, 0, "hits (no usable event)");
  checkNull(z.all.robust_sigma, "all robust_sigma (no usable event)");
  checkNull(z.all.rms, "all rms (no usable event)");
  checkNull(z.view[1].median, "view V median (no usable event)");
  checkNull(z.quartile[0].q_lo, "charge quartile 0 q_lo (no usable event)");
  checkNull(z.width_median[0], "width median U (no usable event)");
  check(contains(hitres::toJson(z), "\"all\": {\"n\": 0, \"rms\": null, \"robust_sigma\": null, \"median\": null}"),
        "JSON all nulls (no usable event)");
  check(hit_residuals(uvw85.c_str(), trk85.c_str(), json85.c_str()) == 0,
        "hit_residuals() returns 0 without usable events (WARNING)");

  // 後片付け
  for (const std::string &p : cleanup) gSystem->Unlink(p.c_str());
  gSystem->Unlink(dir.c_str());

  std::printf("%s: %d failure(s)\n", g_failures == 0 ? "GREEN" : "RED", g_failures);
  return g_failures;
}
