// =============================================================================
// test_uvw_closure.C — uvw_closure.C の合成テスト(TODO/148 §2)。
//
//   root -l -b -q tools/scan/test_uvw_closure.C     → exit code = 失敗数(0 = green)
//
// 合成 raw(mini = adc[4][68][512])・合成 channel_map・合成 tracks を一時ディレクトリに書き、uvw_closure.C で
// 求めた閉合残差の統計が手計算どおりに戻ることを見る。分位点(中央値・MAD・電荷の 4 分位)は線形補間
// h = (n−1)·q(run_metrics.C の quantile と同じ定義)。robust σ = 1.4826 × MAD、RMS = √(Σr²/n)(c 基準)。
// (下の手計算の値は numpy の median / percentile(linear)でも同じになることを確かめた)
//
// 合成 channel_map(小さな 3 面、pitch 2 mm — 実物の 1.5 mm と違う値にして、コードが pitch を焼き込んで
// いないことを縛る): view v ごとに 24 本、j = 0..23、strip 番号 = j + 1、s = o_v + 2·(j − 12)、
//   原点 o = (U 1.25, V −3.5, W 6.75)→ 閉合の定数 c = Σ o_v = 4.5(pitch の和 = 0 なので点に依らない)。
//   (aget, ch) は map を読まないと解けない並び: U = AGET 0 の信号 ch の降順、V = j 偶数 AGET 1 / 奇数 AGET 2、
//   W = AGET 3 の信号 ch を 7 刻みの循環。ファイルは見出し(#)、空行、W を j の降順で書く(strip 番号で並べ直すこと)。
//
// 基線(全 ch、FPN 含む)= 200 + 3c + 50a + t/13(整数除算 — common mode の傾き)+ AGET 1 だけ cell 400..409 に
//   +80(AGET の common mode)。信号 ch は ped 窓 cell 1..20 に ±2 の交互(平均 0)。FPN を引けば v = 足した高さ
//   ちょうど。引かなければ AGET 1 の V ストリップ(偶数 j)が 400..409 で、全ストリップが cell ≥ 468 で閾値を
//   越える(数え上げが全部変わる)。AGET 0 ch 0(map に無い)に cell 100..107 で +1000(使われない)。
//
// ストリップへの置き方 depositAt(view, s*, T, 本数): 最寄りのストリップ k(δ = s* − s_k ∈ [−1, 1] mm)を中心に
//   3 本 (B − D, C, B + D) か 5 本 (E, B − D, C, B + D, E)、B = round(0.3T)、E = round(0.08T)(5 本のとき)、
//   C = T − 2B − 2E、D = round(δ·T/(2·pitch))。重心 = s_k + 2·pitch·D/T(E は対称で重心を動かさない)。
//   T が 64 の倍数で δ が 1/8 の倍数なら D は整数 = 重心は s* ちょうど(fixture の自己チェック g_inexact)。
//   どの本も > 35(fixture の自己チェック g_fixtureOk)。
// 点の置き方: 点の (u', v') = (s_U − o_U, s_V − o_V)、w' = −u' − v'(pitch の和 0 — どの (x, y) にも対応する)。
//   s*_v = o_v + (u', v', w')_v + ε_v、和 = 4.5 + Σε。
//
// [1] run0083(4 イベント、手計算で全部の値を固定):
//   e0 (id 500, ok 1) スライス i = 0..7 を cell 100..106 と 507(範囲 [2, 508) の最後の cell)に
//   e1 (id 501, ok 1) スライス i = 0..7 を cell 200..207 に
//      級 = i/2 → A A B B C C D D、view ごとの T = T_c + (0, +128, −64)、T_c = A 1280 / B 1920 / C 2560 / D 3840
//      → スライス電荷 Q = 3T_c + 64 = A 3904 / B 5824 / C 7744 / D 11584。
//      閉合 ρ = Σε = s_c·k、s = (A 2, B 1, C 0.5, D 0.25)、k = e0 は i 偶数 −1.5 / 奇数 +0.5、e1 は −0.5 / +1.5。
//      ε = η·(−1)^i + ρ·[v == i mod 3]、η = (0.5, −0.25, −0.25)(和 0 — 位置だけ動かす)。
//      点: e0 (u', v') = (−2 + 0.75i, 3 − 0.5i)、e1 (1 − 0.5i, −2 + 0.75i)。
//      本数: U 3、V は C / D で 5・A / B で 3、W 5 → 本数の中央値 U 3 / V (3 + 5)/2 = 4 / W 5。
//   e0 のおとり(数えない): cell 0 と 508..511 に ε = (2, 2, 2)(ρ = 6)の正しい 3 面スライス(範囲 [2, 508) の外)
//   e2 (id 502, ok 0) cell 100..107 に ρ = 6 の正しいスライス 8 個 → 使わない
//   e3 (id 503, ok 1) 捨てるスライスだけ(基本 = 点 (0, 0)、ε = 0、T 1280、3 本 → どの view も j 11..13):
//      cell 300..302 V に j 18 の 1 本(200)を足す → 塊 2 つ(3)
//      cell 303      U に j 15 の 1 本を足す(j 14 が空 = 1 本の隙間)→ 塊 2 つ(1)  → slices_multi_cluster 4
//      cell 310..314 U と V だけ(5)、cell 315 U だけ(1)、
//      cell 316      U が塊 2 つ + V、W 無し(view 不足を先に数える)(1)、
//      cell 320      U と V、W は j 11..13 に高さ 35 ちょうど(> 35 でない)(1)     → slices_missing_view 8
//   手計算(採用 = e0 / e1 の 16 スライス、r = ρ の 16 個):
//     ρ = A ±3, ±1、B ±1.5, ±0.5、C ±0.75, ±0.25、D ±0.375, ±0.125 → 中央値 0 → c = 4.5(= Σ o)
//     |ρ| の中央値 (0.5 + 0.75)/2 = 0.625 → robust σ = 0.926625、σ_s = 0.926625/√3 = 0.5349871932
//     Σρ² = 2(9 + 1) + 2(2.25 + 0.25) + 2(0.5625 + 0.0625) + 2(0.140625 + 0.015625) = 26.5625
//       → RMS = √(26.5625/16) = 1.288470508
//     電荷: 4 個ずつ、h = 15·q: q25 = 3904 + 0.75·1920 = 5344、q50 = (5824 + 7744)/2 = 6784、
//       q75 = 7744 + 0.25·3840 = 8704 → 級ごと 4 個 = s_c·{±0.5, ±1.5}: 中央値 0、MAD = s_c
//       → robust σ A 2.9652 / B 1.4826 / C 0.7413 / D 0.37065、RMS = s_c·√1.25 = 2.236067977 / 1.118033989 /
//       0.5590169944 / 0.2795084972
//     counts: events 4、events_ok 3、slices 16、missing 8、multi 4
//   maxEvents = 1(e0 だけ): ρ = −3, 1, −1.5, 0.5, −0.75, 0.25, −0.375, 0.125 → 中央値 −0.125 → c = 4.375、
//     r = ρ + 0.125 → |r| の中央値 0.625 → robust σ 0.926625、RMS = √(12.46875/8) = 1.248436522、slices 8
//
// [2] run0084 ガウス(既知の σ = 0.4 mm、TRandom3 seed 4148、60 イベント × cell 100..199 = 6000 スライス):
//   イベントごとの点 (x, y) = (−3 + 0.75·(e mod 9), −2.5 + 0.5·(e mod 11))、s*_v = o_v + pitch_v·(x, y) + ε_v、
//   pitch_U = (−1, 0)、pitch_V = (0.5, +√3/2)、pitch_W = (0.5, −√3/2)(makeTracks.C:7-8)、ε_v ~ N(0, 0.4) を 3 面
//   独立に、T は view ごとに 1280 + [0, 5120) の一様な整数(64 の倍数に限ると Q が 4 分位の境界で同値になり
//   n が ±2 に収まらない)。→ σ_r = √3·0.4 = 0.6928203230、σ_s = 0.4。
//   robust σ / RMS / σ_s は 5 % 以内(漸近の相対誤差 √(1.36/n) = 1.5 %、√(1/2n) = 0.9 %)、
//   c は 4.5 ± 0.05(中央値の標準誤差 1.2533·σ_r/√n = 0.011)、電荷 4 分位は n = 1500 ± 2・robust σ 10 % 以内。
//   D の丸めの重心誤差 ≤ 2·pitch·0.5/T ≤ 0.0016 mm(無視できる)。
//
// [3] run0086 スライス幅(computeClosure の sliceCells、入口は kSliceCells = 1 固定):
//   1 イベント、点 (0.5, −1)。cell 100: ε = (1.5, 0, 0)、T 1280 → 和 6.0。cell 101: ε = (0, −0.375, 0)、
//   T 2560 → 和 4.125。W の離れた j 20 に cell 100 と 101 で 30 ずつ(cell ごとには閾値未満、足すと 60)。
//   sliceCells = 1: スライス 2、c = (6.0 + 4.125)/2 = 5.0625、r = ±0.9375 → robust σ 1.3899375、RMS 0.9375
//   sliceCells = 2: スライス [100, 101] 1 個、view ごとの重心 = 電荷加重
//     U (1280·(o + 2.0) + 2560·(o + 0.5))/3840 = o + 1.0、V o − 1.25、W o + 0.5 → c = 4.5 + 0.25 = 4.75、
//     Q = 3·1280 + 3·2560 = 11520、本数 U 4(j 12..14 ∪ 11..13)/ V 3 / W 3(j 20 は閾値を cell ごとに見るので入らない)
//
// [4] 異常系: 無い raw / map / tracks → 1、エントリ数・eventId の不一致、map の不正(読めない行、view 3、ch 68、
//     aget −1、FPN ch、raw に無い aget、(aget, ch) と (view, strip) の重複、W のストリップ無し)→ 失敗
//     (理由ごとに別の語で縛る)、sliceCells 0 → 失敗
// [5] 使えるスライスが無い run(ok=0 だけ)→ 値は null、入口は 0 を返す
// =============================================================================
#include "uvw_closure.C"

#include <TRandom3.h>
#include <TSystem.h>

#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <string>
#include <vector>

namespace test_uc {

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

void checkNear(double got, double want, const std::string &what, double tol = 1e-6)
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

// JSON の anchor の後ろで最初に出る `"key": <数>` の数。無ければ NaN(null も NaN)。
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

// ---- 合成 channel_map --------------------------------------------------------------------------
const double kPitchMm = 2.0;
const int kStripsPerView = 24;
const int kCenterJ = 12;
const double kOrigin[3] = {1.25, -3.5, 6.75};  // Σ = 4.5
const double kOriginSum = 4.5;
const double kSqrt3Half = std::sqrt(3.0) / 2;
const double kP[3][2] = {{-1, 0}, {0.5, kSqrt3Half}, {0.5, -kSqrt3Half}};  // makeTracks.C:7-8

// 0..67 から FPN を除いた 64 本(昇順)。
std::vector<int> signalChannels()
{
  std::vector<int> sig;
  for (int c = 0; c < 68; ++c)
    if (!runmetrics::isFpn(c)) sig.push_back(c);
  return sig;
}

// view v の j 番目のストリップの (aget, ch)(冒頭)。
void stripChannel(int view, int j, int &aget, int &ch)
{
  const std::vector<int> sig = signalChannels();
  if (view == 0) {
    aget = 0;
    ch = sig[63 - j];
  } else if (view == 1) {
    aget = (j % 2 == 0) ? 1 : 2;
    ch = (j % 2 == 0) ? sig[5 + j / 2] : sig[10 + j / 2];
  } else {
    aget = 3;
    ch = sig[(7 * j + 3) % 64];
  }
}

double stripS(int view, int j) { return kOrigin[view] + kPitchMm * (j - kCenterJ); }

std::string mapLine(int view, int j)
{
  int a = 0, c = 0;
  stripChannel(view, j, a, c);
  char buf[128];
  std::snprintf(buf, sizeof(buf), "%d,%d,%d,%d,%.4f", a, c, view, j + 1, stripS(view, j));
  return buf;
}

// 見出し、U(j 昇順)、空行、V(j 昇順)、W(j 降順)。withW = false なら W を書かない。
std::vector<std::string> mapLines(bool withW = true)
{
  std::vector<std::string> lines;
  lines.push_back("# aget,raw_channel,view(0=U 1=V 2=W),strip,s_mm  (synthetic, test_uvw_closure.C)");
  for (int j = 0; j < kStripsPerView; ++j) lines.push_back(mapLine(0, j));
  lines.push_back("");
  for (int j = 0; j < kStripsPerView; ++j) lines.push_back(mapLine(1, j));
  if (withW)
    for (int j = kStripsPerView - 1; j >= 0; --j) lines.push_back(mapLine(2, j));
  return lines;
}

bool writeLines(const std::string &path, const std::vector<std::string> &lines)
{
  FILE *fp = std::fopen(path.c_str(), "w");
  if (!fp) return false;
  for (const std::string &l : lines) std::fprintf(fp, "%s\n", l.c_str());
  return std::fclose(fp) == 0;
}

// ---- 合成 raw ---------------------------------------------------------------------------------
Short_t g_adc[4][68][512];
bool g_fixtureOk = true;  // どの置いた本も > 35、ADC が 0..4095、ストリップが範囲内
int g_inexact = 0;        // D が整数でなかった回数([1] / [3] は 0 であること)

int baseline(int a, int c) { return 200 + 3 * c + 50 * a; }
int commonMode(int a, int t) { return t / 13 + ((a == 1 && t >= 400 && t <= 409) ? 80 : 0); }

void fillBaseline()
{
  for (int a = 0; a < 4; ++a)
    for (int c = 0; c < 68; ++c)
      for (int t = 0; t < 512; ++t) {
        int v = baseline(a, c) + commonMode(a, t);
        if (!runmetrics::isFpn(c) && t >= 1 && t <= 20) v += (t % 2 == 1) ? -2 : +2;
        g_adc[a][c][t] = static_cast<Short_t>(v);
      }
}

void add(int a, int c, int t, int h)
{
  const int v = g_adc[a][c][t] + h;
  if (v < 0 || v > 4095) g_fixtureOk = false;
  g_adc[a][c][t] = static_cast<Short_t>(std::min(std::max(v, 0), 4095));
}

void addStrip(int view, int j, int t, int h)
{
  if (j < 0 || j >= kStripsPerView) {
    g_fixtureOk = false;
    return;
  }
  int a = 0, c = 0;
  stripChannel(view, j, a, c);
  add(a, c, t, h);
}

// 冒頭の depositAt。
void depositAt(int view, double sStar, int T, int nStrips, int t)
{
  const int k = kCenterJ + static_cast<int>(std::lround((sStar - kOrigin[view]) / kPitchMm));
  const double exactD = (sStar - stripS(view, k)) * T / (2 * kPitchMm);
  const int D = static_cast<int>(std::lround(exactD));
  if (std::fabs(exactD - D) > 1e-9) ++g_inexact;
  const int B = static_cast<int>(std::lround(0.3 * T));
  const int E = (nStrips == 5) ? static_cast<int>(std::lround(0.08 * T)) : 0;
  const int C = T - 2 * B - 2 * E;
  if (B - std::abs(D) <= 35 || C <= 35 || (nStrips == 5 && E <= 35)) g_fixtureOk = false;
  addStrip(view, k - 1, t, B - D);
  addStrip(view, k, t, C);
  addStrip(view, k + 1, t, B + D);
  if (nStrips == 5) {
    addStrip(view, k - 2, t, E);
    addStrip(view, k + 2, t, E);
  }
}

// 点 (u', v')(w' = −u' − v')に view ごとのずれ eps を足して 3 面に置く。
void depositPoint(double u, double vv, const double eps[3], const int T[3], const int nStrips[3], int t)
{
  const double tr[3] = {u, vv, -u - vv};
  for (int v = 0; v < 3; ++v) depositAt(v, kOrigin[v] + tr[v] + eps[v], T[v], nStrips[v], t);
}

// 点 (0, 0)、ε = 0(または e)、T、3 本 の 3 面スライス。views = 置く view のビット(1 U、2 V、4 W)。
void simpleSlice(int t, int views, double e = 0, int T = 1280)
{
  for (int v = 0; v < 3; ++v)
    if (views & (1 << v)) depositAt(v, kOrigin[v] + e, T, 3, t);
}

// [1] の e0 / e1 のスライス i(冒頭の表)。
const int kTClass[4] = {1280, 1920, 2560, 3840};
const double kScale[4] = {2.0, 1.0, 0.5, 0.25};
const int kDeltaT[3] = {0, 128, -64};

void mainSlice(int e, int i, int t)
{
  const int cls = i / 2;
  const double k = (e == 0) ? ((i % 2 == 0) ? -1.5 : 0.5) : ((i % 2 == 0) ? -0.5 : 1.5);
  const double rho = kScale[cls] * k;
  const double sign = (i % 2 == 0) ? 1.0 : -1.0;
  double eps[3] = {0.5 * sign, -0.25 * sign, -0.25 * sign};
  eps[i % 3] += rho;
  const double u = (e == 0) ? -2 + 0.75 * i : 1 - 0.5 * i;
  const double vv = (e == 0) ? 3 - 0.5 * i : -2 + 0.75 * i;
  int T[3];
  for (int v = 0; v < 3; ++v) T[v] = kTClass[cls] + kDeltaT[v];
  const int n[3] = {3, cls >= 2 ? 5 : 3, 5};
  depositPoint(u, vv, eps, T, n, t);
}

// ρ = 6 の正しい 3 面スライス(おとり)。
void decoySlice(int t) { simpleSlice(t, 7, 2.0); }

void fillMain(int ev)
{
  fillBaseline();
  for (int t = 100; t <= 107; ++t) add(0, 0, t, 1000);  // map に無い ch
  if (ev == 0) {
    const int cells[8] = {100, 101, 102, 103, 104, 105, 106, 507};
    for (int i = 0; i < 8; ++i) mainSlice(0, i, cells[i]);
    decoySlice(0);
    for (int t = 508; t <= 511; ++t) decoySlice(t);
  } else if (ev == 1) {
    for (int i = 0; i < 8; ++i) mainSlice(1, i, 200 + i);
  } else if (ev == 2) {
    for (int t = 100; t <= 107; ++t) decoySlice(t);
  } else if (ev == 3) {
    for (int t = 300; t <= 302; ++t) {
      simpleSlice(t, 7);
      addStrip(1, 18, t, 200);
    }
    simpleSlice(303, 7);
    addStrip(0, 15, 303, 200);
    for (int t = 310; t <= 314; ++t) simpleSlice(t, 1 | 2);
    simpleSlice(315, 1);
    simpleSlice(316, 1 | 2);
    addStrip(0, 18, 316, 200);
    simpleSlice(320, 1 | 2);
    for (int j = 11; j <= 13; ++j) addStrip(2, j, 320, 35);
  }
}

// [2] ガウス。
TRandom3 g_rnd(4148);

void fillGauss(int e)
{
  fillBaseline();
  const double x = -3 + 0.75 * (e % 9), y = -2.5 + 0.5 * (e % 11);
  for (int t = 100; t < 200; ++t)
    for (int v = 0; v < 3; ++v) {
      const double sStar = kOrigin[v] + kP[v][0] * x + kP[v][1] * y + g_rnd.Gaus(0, 0.4);
      const int T = 1280 + static_cast<int>(5120 * g_rnd.Rndm());
      depositAt(v, sStar, T, 3, t);
    }
}

// [3] スライス幅。
void fillSliceWidth(int)
{
  fillBaseline();
  const int n[3] = {3, 3, 3};
  const double eps100[3] = {1.5, 0, 0}, eps101[3] = {0, -0.375, 0};
  const int T100[3] = {1280, 1280, 1280}, T101[3] = {2560, 2560, 2560};
  depositPoint(0.5, -1, eps100, T100, n, 100);
  depositPoint(0.5, -1, eps101, T101, n, 101);
  addStrip(2, 20, 100, 30);
  addStrip(2, 20, 101, 30);
}

bool writeRaw(const std::string &path, int nEvents, void (*fill)(int), UInt_t firstId)
{
  TFile f(path.c_str(), "RECREATE");
  if (f.IsZombie()) return false;
  TTree tree("raw", "synthetic raw (test_uvw_closure.C)");
  UInt_t eventId = 0;
  ULong64_t eventTime = 0;
  tree.Branch("eventId", &eventId);
  tree.Branch("eventTime", &eventTime);
  tree.Branch("adc", g_adc, "adc[4][68][512]/S");
  for (int k = 0; k < nEvents; ++k) {
    fill(k);
    eventId = firstId + static_cast<UInt_t>(k);
    eventTime = 1000ULL + static_cast<ULong64_t>(k) * 5000000ULL;
    tree.Fill();
  }
  tree.Write();
  return true;
}

// tracks(makeTracks.C と同じ型: eventId = UInt_t、ok = int)。他の枝は uvw_closure は読まない。
bool writeTracks(const std::string &path, const std::vector<UInt_t> &ids, const std::vector<int> &oks)
{
  TFile f(path.c_str(), "RECREATE");
  if (f.IsZombie()) return false;
  TTree t("tracks", "synthetic tracks (test_uvw_closure.C)");
  UInt_t eventId = 0;
  int ok = 0;
  t.Branch("eventId", &eventId);
  t.Branch("ok", &ok);
  for (size_t i = 0; i < ids.size(); ++i) {
    eventId = ids[i];
    ok = oks[i];
    t.Fill();
  }
  t.Write();
  return true;
}

}  // namespace test_uc

int test_uvw_closure()
{
  using namespace test_uc;
  const std::string dir = std::string(gSystem->TempDirectory()) + "/test_uvw_closure_" +
                          std::to_string(gSystem->GetPid());
  gSystem->mkdir(dir.c_str(), kTRUE);
  std::vector<std::string> cleanup;
  const std::string map = dir + "/channel_map.csv";
  cleanup.push_back(map);
  check(writeLines(map, mapLines()), "write synthetic channel_map (3 x 24 strips, pitch 2 mm)");

  // ---- [1] run0083: 手計算の値 ---------------------------------------------------------------
  std::printf("[1] synthetic run0083: offsets, charge classes, ok=0, window edges, multi-cluster, missing view\n");
  const std::string raw83 = dir + "/run0083_raw.root", trk83 = dir + "/run0083_tracks.root";
  const std::string json83 = dir + "/run0083_uvw_closure.json", png83 = dir + "/run0083_uvw_closure.png";
  cleanup.insert(cleanup.end(), {raw83, trk83, json83, png83});
  g_fixtureOk = true;
  g_inexact = 0;
  check(writeRaw(raw83, 4, fillMain, 500), "write synthetic raw (4 events)");
  check(g_fixtureOk, "fixture: every deposited strip > 35, ADC inside 0..4095, strips inside the plane");
  checkCount(g_inexact, 0, "fixture: every centroid exactly on s* (D integer)");
  check(writeTracks(trk83, {500, 501, 502, 503}, {1, 1, 0, 1}), "write synthetic tracks (ok 1 1 0 1)");

  uvwclosure::Result r;
  std::string err;
  check(uvwclosure::computeClosure(raw83, map, trk83, 0, 1, r, err), "computeClosure ok (" + err + ")");
  checkCount(r.run, 83, "run (from run0083_raw.root)");
  checkCount(r.events, 4, "events");
  checkCount(r.events_ok, 3, "events_ok (e2 has ok = 0)");
  checkCount(r.slices, 16, "slices (e0 8 incl. cell 507, e1 8; not cell 0 / 508..511, not e2)");
  checkCount(r.slices_missing_view, 8, "slices_missing_view (e3: 5 + 1 + 1 (also 2 clusters) + 1 (W = 35))");
  checkCount(r.slices_multi_cluster, 4, "slices_multi_cluster (e3: V far strip x3, U one-strip gap x1)");
  checkCount(r.agets_without_fpn, 0, "agets_without_fpn");
  checkNear(r.c, kOriginSum, "c = median of s_U + s_V + s_W = sum of the map origins 4.5");
  checkCount(r.residual.n, 16, "residual n");
  checkNear(r.residual.robust_sigma, 0.926625, "residual robust_sigma (1.4826 x 0.625)");
  checkNear(r.residual.rms, 1.288470508, "residual rms (sqrt(26.5625 / 16))");
  checkNear(r.sigma_per_view, 0.5349871932, "sigma_per_view = robust_sigma / sqrt(3)");
  const double wantLo[4] = {3904, 5344, 6784, 8704}, wantHi[4] = {5344, 6784, 8704, 11584};
  const double wantSigma[4] = {2.9652, 1.4826, 0.7413, 0.37065};
  const double wantQRms[4] = {2.236067977, 1.118033989, 0.5590169944, 0.2795084972};
  for (int b = 0; b < 4; ++b) {
    const std::string name = "charge quartile " + std::to_string(b);
    checkNear(r.quartile[b].q_lo, wantLo[b], name + " q_lo");
    checkNear(r.quartile[b].q_hi, wantHi[b], name + " q_hi");
    checkCount(r.quartile[b].s.n, 4, name + " n");
    checkNear(r.quartile[b].s.median, 0.0, name + " median");
    checkNear(r.quartile[b].s.robust_sigma, wantSigma[b], name + " robust_sigma");
    checkNear(r.quartile[b].s.rms, wantQRms[b], name + " rms");
  }
  checkNear(r.strips_median[0], 3, "strips median U (3 strips)");
  checkNear(r.strips_median[1], 4, "strips median V (3 x 8, 5 x 8)");
  checkNear(r.strips_median[2], 5, "strips median W (5 strips)");
  checkCount(static_cast<long>(r.used.size()), 16, "slices kept for the PNG");

  // ---- JSON ------------------------------------------------------------------------------------
  const std::string js = uvwclosure::toJson(r);
  check(contains(js, "\"run\": 83,"), "JSON \"run\": 83");
  check(contains(js, "\"events\": 4,"), "JSON \"events\": 4");
  check(contains(js, "\"events_ok\": 3,"), "JSON \"events_ok\": 3");
  check(contains(js, "\"slice_cells\": 1,"), "JSON \"slice_cells\": 1");
  check(contains(js, "\"cell_range\": [2, 508],"), "JSON \"cell_range\": [2, 508]");
  check(contains(js, "\"adc_threshold\": 35,"), "JSON \"adc_threshold\": 35");
  check(contains(js, "\"slices\": 16,"), "JSON \"slices\": 16");
  check(contains(js, "\"slices_missing_view\": 8,"), "JSON \"slices_missing_view\": 8");
  check(contains(js, "\"slices_multi_cluster\": 4,"), "JSON \"slices_multi_cluster\": 4");
  for (const char *key : {"\"raw_file\"", "\"channel_map\"", "\"tracks_file\"", "\"residual_mm\"",
                          "\"charge_quartiles\"", "\"strips_median\""})
    check(contains(js, key), std::string("JSON key ") + key);
  checkNear(jsonNum(js, "{", "c_mm"), 4.5, "JSON c_mm");
  checkNear(jsonNum(js, "\"residual_mm\": {", "robust_sigma"), 0.926625, "JSON residual_mm.robust_sigma");
  checkNear(jsonNum(js, "\"residual_mm\": {", "rms"), 1.288470508, "JSON residual_mm.rms");
  checkNear(jsonNum(js, "\"residual_mm\": {", "n"), 16, "JSON residual_mm.n");
  checkNear(jsonNum(js, "{", "sigma_per_view_mm"), 0.5349871932, "JSON sigma_per_view_mm");
  checkNear(jsonNum(js, "\"charge_quartiles\": [", "q_hi"), 5344, "JSON charge_quartiles[0].q_hi");
  checkNear(jsonNum(js, "\"charge_quartiles\": [", "robust_sigma"), 2.9652,
            "JSON charge_quartiles[0].robust_sigma");
  checkNear(jsonNum(js, "\"strips_median\": {", "V"), 4, "JSON strips_median.V");

  // ---- 入口: JSON と PNG を書く ------------------------------------------------------------------
  check(uvw_closure(raw83.c_str(), map.c_str(), trk83.c_str(), json83.c_str(), png83.c_str(), 0) == 0,
        "uvw_closure() returns 0");
  check(!js.empty() && readFile(json83) == js, "uvw_closure() wrote the same JSON");
  FileStat_t st;
  check(gSystem->GetPathInfo(png83.c_str(), st) == 0 && st.fSize > 0, "uvw_closure() wrote the PNG");

  // ---- maxEvents = 1(e0 だけ)--------------------------------------------------------------------
  std::printf("[1b] maxEvents = 1\n");
  uvwclosure::Result m1;
  check(uvwclosure::computeClosure(raw83, map, trk83, 1, 1, m1, err), "computeClosure ok (maxEvents 1)");
  checkCount(m1.events, 1, "events (maxEvents 1)");
  checkCount(m1.events_ok, 1, "events_ok (maxEvents 1)");
  checkCount(m1.slices, 8, "slices (maxEvents 1)");
  checkCount(m1.slices_missing_view + m1.slices_multi_cluster, 0, "no discarded slice (maxEvents 1)");
  checkNear(m1.c, 4.375, "c (maxEvents 1: median rho = -0.125)");
  checkNear(m1.residual.robust_sigma, 0.926625, "robust_sigma (maxEvents 1)");
  checkNear(m1.residual.rms, 1.248436522, "rms (maxEvents 1: sqrt(12.46875 / 8))");

  // ---- [2] ガウス --------------------------------------------------------------------------------
  std::printf("[2] synthetic run0084: Gaussian per-plane scatter, sigma = 0.4 mm\n");
  const std::string raw84 = dir + "/run0084_raw.root", trk84 = dir + "/run0084_tracks.root";
  cleanup.insert(cleanup.end(), {raw84, trk84});
  g_fixtureOk = true;
  check(writeRaw(raw84, 60, fillGauss, 0), "write Gaussian raw (60 events x 100 slices)");
  check(g_fixtureOk, "fixture (Gaussian): every deposited strip > 35, inside the plane");
  std::vector<UInt_t> ids84;
  std::vector<int> oks84;
  for (int e = 0; e < 60; ++e) {
    ids84.push_back(static_cast<UInt_t>(e));
    oks84.push_back(1);
  }
  check(writeTracks(trk84, ids84, oks84), "write Gaussian tracks (all ok)");
  uvwclosure::Result g;
  check(uvwclosure::computeClosure(raw84, map, trk84, 0, 1, g, err), "computeClosure ok (Gaussian)");
  checkCount(g.slices, 6000, "slices (Gaussian)");
  checkCount(g.slices_missing_view + g.slices_multi_cluster, 0, "no discarded slice (Gaussian)");
  checkNear(g.c, kOriginSum, "c ~ sum of origins 4.5 (Gaussian)", 0.05);
  checkRel(g.residual.robust_sigma, std::sqrt(3.0) * 0.4, 0.05, "robust_sigma ~ sqrt(3) x 0.4 (5 %)");
  checkRel(g.residual.rms, std::sqrt(3.0) * 0.4, 0.05, "rms ~ sqrt(3) x 0.4 (5 %)");
  checkRel(g.sigma_per_view, 0.4, 0.05, "sigma_per_view ~ 0.4 (5 %)");
  long nq = 0;
  for (int b = 0; b < 4; ++b) {
    nq += g.quartile[b].s.n;
    check(std::labs(g.quartile[b].s.n - 1500) <= 2,
          "charge quartile " + std::to_string(b) + " n = " + std::to_string(g.quartile[b].s.n) +
              " (1500 +- 2)");
    checkRel(g.quartile[b].s.robust_sigma, std::sqrt(3.0) * 0.4, 0.10,
             "charge quartile " + std::to_string(b) + " robust_sigma ~ sqrt(3) x 0.4 (10 %)");
  }
  checkCount(nq, 6000, "charge quartiles cover every slice");
  checkNear(g.strips_median[0], 3, "strips median U (Gaussian)");

  // ---- [3] スライス幅 ----------------------------------------------------------------------------
  std::printf("[3] synthetic run0086: slice width 1 vs 2 cells\n");
  const std::string raw86 = dir + "/run0086_raw.root", trk86 = dir + "/run0086_tracks.root";
  cleanup.insert(cleanup.end(), {raw86, trk86});
  g_fixtureOk = true;
  g_inexact = 0;
  check(writeRaw(raw86, 1, fillSliceWidth, 900), "write slice-width raw (1 event)");
  check(g_fixtureOk, "fixture (slice width): every deposited strip > 35");
  checkCount(g_inexact, 0, "fixture (slice width): centroids exact");
  check(writeTracks(trk86, {900}, {1}), "write slice-width tracks");
  uvwclosure::Result w1, w2;
  check(uvwclosure::computeClosure(raw86, map, trk86, 0, 1, w1, err), "computeClosure ok (sliceCells 1)");
  checkCount(w1.slices, 2, "slices (sliceCells 1)");
  checkNear(w1.c, 5.0625, "c (sliceCells 1: (6.0 + 4.125) / 2)");
  checkNear(w1.residual.robust_sigma, 1.3899375, "robust_sigma (sliceCells 1: 1.4826 x 0.9375)");
  checkNear(w1.residual.rms, 0.9375, "rms (sliceCells 1)");
  check(uvwclosure::computeClosure(raw86, map, trk86, 0, 2, w2, err), "computeClosure ok (sliceCells 2)");
  checkCount(w2.slice_cells, 2, "slice_cells (sliceCells 2)");
  checkCount(w2.slices, 1, "slices (sliceCells 2: cells 100..101 in one slice)");
  checkCount(w2.slices_multi_cluster, 0, "W 30 + 30 is below the per-cell threshold (sliceCells 2)");
  checkNear(w2.c, 4.75, "c (sliceCells 2: charge-weighted over the two cells)");
  checkNear(w2.quartile[0].q_lo, 11520, "slice charge (sliceCells 2: 3 x 1280 + 3 x 2560)");
  checkNear(w2.strips_median[0], 4, "strips U (sliceCells 2: j 11..14)");
  checkNear(w2.strips_median[1], 3, "strips V (sliceCells 2)");
  checkNear(w2.strips_median[2], 3, "strips W (sliceCells 2: far j 20 not counted)");
  check(contains(uvwclosure::toJson(w2), "\"slice_cells\": 2,"), "JSON \"slice_cells\": 2");
  uvwclosure::Result w0;
  err.clear();
  check(!uvwclosure::computeClosure(raw86, map, trk86, 0, 0, w0, err), "sliceCells 0 fails (" + err + ")");

  // ---- [4] 異常系 --------------------------------------------------------------------------------
  std::printf("[4] errors\n");
  const std::string json = dir + "/err.json";
  cleanup.push_back(json);
  check(uvw_closure((dir + "/missing_raw.root").c_str(), map.c_str(), trk83.c_str(), json.c_str()) == 1,
        "uvw_closure() returns 1 for a missing raw file");
  check(uvw_closure(raw83.c_str(), (dir + "/missing_map.csv").c_str(), trk83.c_str(), json.c_str()) == 1,
        "uvw_closure() returns 1 for a missing channel map");
  check(uvw_closure(raw83.c_str(), map.c_str(), (dir + "/missing_tracks.root").c_str(), json.c_str()) == 1,
        "uvw_closure() returns 1 for a missing tracks file");
  const std::string bad = dir + "/bad_tracks.root";
  cleanup.push_back(bad);
  uvwclosure::Result x;
  check(writeTracks(bad, {500, 501, 502}, {1, 1, 0}), "write tracks with 3 of 4 entries");
  err.clear();
  check(!uvwclosure::computeClosure(raw83, map, bad, 0, 1, x, err) && contains(err, "entries"),
        "entry count mismatch fails (" + err + ")");
  check(writeTracks(bad, {500, 501, 7777, 503}, {1, 1, 0, 1}), "write tracks with eventId 7777 at entry 2");
  err.clear();
  check(!uvwclosure::computeClosure(raw83, map, bad, 0, 1, x, err) && contains(err, "eventId"),
        "eventId mismatch fails (" + err + ")");

  const std::string badMap = dir + "/bad_map.csv";
  cleanup.push_back(badMap);
  struct BadMap {
    std::string extra, keyword, what;
  };
  // U の j = 0 の (aget, ch) をもう一度(strip 99 で)。ch 1 はどの view も使わない(U は AGET 0 の 67..40 付近)
  int a0 = 0, c0 = 0;
  stripChannel(0, 0, a0, c0);
  const std::string dupChannel = std::to_string(a0) + "," + std::to_string(c0) + ",0,99,0.0";
  const std::vector<BadMap> badMaps = {
      {"0;1;0;99;0.0", "cannot read", "unreadable line"},
      {"0,1,3,99,0.0", "view 3", "view 3"},
      {"0,68,0,99,0.0", "outside 0..67", "raw_channel 68"},
      {"-1,1,0,99,0.0", "aget -1", "aget -1"},
      {"0,11,0,99,0.0", "FPN", "FPN channel 11"},
      {"4,1,0,99,0.0", "AGETs", "aget 4 with a 4-AGET raw tree"},
      {dupChannel, "duplicate (aget", "duplicate (aget, raw_channel)"},
      {"0,1,0,1,0.0", "duplicate (view", "duplicate (view, strip)"},
  };
  for (const BadMap &bm : badMaps) {
    std::vector<std::string> lines = mapLines();
    lines.push_back(bm.extra);
    check(writeLines(badMap, lines), "write map with " + bm.what + " (" + bm.extra + ")");
    err.clear();
    check(!uvwclosure::computeClosure(raw83, badMap, trk83, 0, 1, x, err) && contains(err, bm.keyword),
          "map with " + bm.what + " fails (" + err + ")");
  }
  check(writeLines(badMap, mapLines(false)), "write map without W strips");
  err.clear();
  check(!uvwclosure::computeClosure(raw83, badMap, trk83, 0, 1, x, err) && contains(err, "no strip"),
        "map without W strips fails (" + err + ")");

  // ---- [5] 使えるスライスが無い run → null ---------------------------------------------------------
  std::printf("[5] run without usable slices\n");
  const std::string raw85 = dir + "/run0085_raw.root", trk85 = dir + "/run0085_tracks.root";
  const std::string json85 = dir + "/run0085_uvw_closure.json";
  cleanup.insert(cleanup.end(), {raw85, trk85, json85});
  check(writeRaw(raw85, 1, [](int) { fillMain(2); }, 502), "write ok=0-only raw (e2)");
  check(writeTracks(trk85, {502}, {0}), "write ok=0-only tracks");
  uvwclosure::Result z;
  check(uvwclosure::computeClosure(raw85, map, trk85, 0, 1, z, err), "computeClosure ok (no usable slice)");
  checkCount(z.events, 1, "events (no usable slice)");
  checkCount(z.slices, 0, "slices (no usable slice)");
  checkNull(z.c, "c (no usable slice)");
  checkNull(z.residual.robust_sigma, "robust_sigma (no usable slice)");
  checkNull(z.residual.rms, "rms (no usable slice)");
  checkNull(z.sigma_per_view, "sigma_per_view (no usable slice)");
  checkNull(z.quartile[0].q_lo, "charge quartile 0 q_lo (no usable slice)");
  checkNull(z.strips_median[0], "strips median U (no usable slice)");
  const std::string jz = uvwclosure::toJson(z);
  check(contains(jz, "\"c_mm\": null,") && contains(jz, "\"sigma_per_view_mm\": null,"),
        "JSON c_mm / sigma_per_view_mm null (no usable slice)");
  check(uvw_closure(raw85.c_str(), map.c_str(), trk85.c_str(), json85.c_str()) == 0,
        "uvw_closure() returns 0 without usable slices (WARNING)");

  // 後片付け
  for (const std::string &p : cleanup) gSystem->Unlink(p.c_str());
  gSystem->Unlink(dir.c_str());

  std::printf("%s: %d failure(s)\n", g_failures == 0 ? "GREEN" : "RED", g_failures);
  return g_failures;
}
