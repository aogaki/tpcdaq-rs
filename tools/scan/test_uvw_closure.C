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
//   e0 (id 500, ok 1) スライス i = 0..7 を cell 500..507(範囲 [2, 508) の最後の cell 507 を含む。kSliceWindowCells = 15 以内に収める — 旧: 100..106 と 507)に
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
//      cell 304..308 U と V だけ(5)、cell 309 U だけ(1)、
//      cell 310      U が塊 2 つ + V、W 無し(view 不足を先に数える)(1)、
//      cell 311      U と V、W は j 11..13 に高さ 35 ちょうど(> 35 でない)(1)     → slices_missing_view 8
//      (旧: 310..314 / 315 / 316 / 320。同じストリップの堆積を kSliceWindowCells = 15 以内に詰めた。個数は同じ)
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
// [2] run0084 ガウス(既知の σ = 0.4 mm、TRandom3 seed 4148、600 イベント × cell 100..109 = 6000 スライス(1 ch の堆積を窓 ±15 cell 以内に収める — 旧: 60 イベント × 100 cell)):
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
//
// [6] hitChannelsOnly(halo を除く ch 条件 = run_metrics.C / drift_end.C の「ヒット ch」: ピーク > 35 かつ
//     ピーク ±5 cell の v > 35 の和 q ≥ 2000。FPN + ped 減算後)。run0087、2 イベント、点 (u', v') = (2, -4)
//     (w' = 2。pitch 2 mm の格子上 → D = 0)、T 3200 の 3 本 (960, 1280, 960) → 重心は中心ストリップちょうど:
//   e0 (id 700) cell 100..104 ε = 0、cell 105..109 ε_U = +2(U が 1 ストリップずれる)
//   e1 (id 701) cell 100..104 ε = 0、cell 105..109 ε_U = -4(U が 2 ストリップずれる)
//   Σ = 4.5 + ε_U → e0 {4.5 x5, 6.5 x5}、e1 {4.5 x5, 0.5 x5}。20 スライス、c = 4.5(ソート 20 個の 10・11 番目が 4.5)。
//   r = {-4 x5, 0 x10, +2 x5} → 中央値 0、|r| = {0 x10, 2 x5, 4 x5} → MAD = (0 + 2)/2 = 1 → robust σ 1.4826、
//   RMS = sqrt((16·5 + 4·5)/20) = sqrt5 = 2.2360680。本数は全 view 3。Q = 3 x 3200 = 9600。
//   ヒット ch(各ストリップのピークは最初の活性 cell、窓 = ピーク..+5 の活性 5 cell 以上で q ≥ 4800 ≥ 2000):
//     e0 = U j 12..14 ∪ 13..15 = 4 + V j 9..11 = 3 + W j 12..14 = 3 → 10
//     e1 = U j 12..14 ∪ 10..12 = 5 + V 3 + W 3 → 11     → channels_used_median = (10 + 11)/2 = 10.5
//   halo = cell 100..109 に map の全ストリップ +60(q = 60 x 6 = 360 < 2000 → ヒット ch にならない)。ただし U の
//   ヒット ch で堆積の無い cell は除く(ヒット ch の halo は正当に重心に入るので、検査対象外にする)。
//   hitChannelsOnly = true は halo の有無で c / robust σ / RMS / 本数 / スライス数 / channels_used_median が同じ
//   (トラックの 3 本は (B+60, C+60, B+60) と対称なので重心は動かない)。電荷 Q だけ +60 x 3 本 x 3 面 = +540。
//   hitChannelsOnly = false: halo 無しでは true と同じ(全部ヒット)、channels_used_median = 読めた 72 ストリップ。
//   halo 有りでは U のヒット ch の堆積の無い cell が halo の隙間になる(U j 15 など)→ 全スライスが U で塊 2 つ
//   (slices_multi_cluster 20、slices 0)。
// [6b] run0088(1 イベント = fillHit の ev 2: 10 cell とも ε = 0 の同じ点、halo は全ストリップ全 cell。隙間無し):
//   hit only = halo を無視 → 本数 3、c = 4.5、channels_used_median = 3 x 3 = 9。
//   all strips = 24 本全部 1 塊 → 本数 24、10 スライス、channels_used_median 72、
//   重心 = (3200·s_t + 60·24·s̄)/4640、s̄ = o - 1(24 本の一様な halo の重心)→ Σ = (3200·4.5 + 1440·1.5)/4640 = 3.5689655
// [7] スライス窓(kSliceWindowCells = 15: スライス t で ch を使うのは v(t) > 35 かつ |t − tpeak| ≤ 15、tpeak = その ch の v の
//     最大の最初の cell)。run0089/90/91、各 1 イベント、点 (2, -4)(D = 0)T 3200 の 3 本を cell 100..104 に(本体。tpeak = 100)、
//     中心ストリップ(U j 13、V j 10、W j 13)に 200 ADC(> 35、ヒット ch 条件の q には入らない)の尾 3 cell:
//     tail 無し(run0089)  = 5 スライス、Σ = 4.5 x 5、c = 4.5。
//     尾が cell 124..126(tpeak から 24 cell、本体の最後から 20)= 窓外 → 使われない: 5 スライス、本数・c は尾無しと一致。
//     尾が cell 110..112(tpeak から 10..12 cell)= 窓内 → 使われる: 8 スライス(尾の 3 つは各 view 1 本、重心 = 中心ストリップ
//     ちょうど → Σ = 4.5)、c = 4.5、robust σ 0。hitChannelsOnly の true / false とも同じ。JSON に "slice_window_cells": 15。
// 設計上の限界(uvw_closure.C 冒頭と同じ): 1 ch に長い堆積(斜めの飛跡が同じ ch に数十 cell 載る)があると、その ch は
//   ピーク ±15 cell のスライスにしか寄与しない。[1] の cell 500..507、[2] の cell 100..109、[1] e3 の 300..311 は
//   この窓に収めてある(旧: 100..106 と 507、100..199、300..320)。
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
    const int cells[8] = {500, 501, 502, 503, 504, 505, 506, 507};
    for (int i = 0; i < 8; ++i) mainSlice(0, i, cells[i]);
    simpleSlice(0, 7, 2.0, 192);  // 小振幅(B 58 / C 76): 他の堆積と同じストリップを共有しても tpeak を奪わない
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
    for (int t = 304; t <= 308; ++t) simpleSlice(t, 1 | 2);
    simpleSlice(309, 1);
    simpleSlice(310, 1 | 2);
    addStrip(0, 18, 310, 200);
    simpleSlice(311, 1 | 2);
    for (int j = 11; j <= 13; ++j) addStrip(2, j, 311, 35);
  }
}

// [2] ガウス。
TRandom3 g_rnd(4148);

void fillGauss(int e)
{
  fillBaseline();
  const double x = -3 + 0.75 * (e % 9), y = -2.5 + 0.5 * (e % 11);
  for (int t = 100; t < 110; ++t)
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


// [6] hitChannelsOnly。g_halo = true で cell 100..109 に map の全ストリップ +60。
bool g_halo = false;

void fillHit(int ev)
{
  fillBaseline();
  const double epsA[3] = {0, 0, 0};
  const double epsB[3] = {ev == 0 ? 2.0 : (ev == 1 ? -4.0 : 0.0), 0, 0};  // ev 2 = ずれ無し(10 cell とも同じ点)
  const int T[3] = {3200, 3200, 3200}, n[3] = {3, 3, 3};
  for (int t = 100; t < 105; ++t) depositPoint(2, -4, epsA, T, n, t);
  for (int t = 105; t < 110; ++t) depositPoint(2, -4, epsB, T, n, t);
  if (!g_halo) return;
  // 同じイベントのどこかで堆積のあるストリップ(U だけがずれる)は、その堆積の無い cell には halo を足さない
  // (足すと hit ch の halo が重心に入る — hit ch は正当に使われるので、ここでは検査したい「hit ch でない
  // ストリップの halo」だけを入れる)。U: A = j 12..14、B = e0 j 13..15 / e1 j 10..12。V j 9..11、W j 12..14 は全 cell 同じ。
  for (int t = 100; t < 110; ++t) {
    const bool a = t < 105;
    const int uLo = a ? 12 : (ev == 0 ? 13 : (ev == 1 ? 10 : 12)), uHi = uLo + 2;
    const int uEverLo = ev == 1 ? 10 : 12, uEverHi = ev == 0 ? 15 : 14;
    for (int j = 0; j < kStripsPerView; ++j) {
      const bool uEver = j >= uEverLo && j <= uEverHi;
      if (!uEver || (j >= uLo && j <= uHi)) addStrip(0, j, t, 60);
      addStrip(1, j, t, 60);
      addStrip(2, j, t, 60);
    }
  }
}

// [7] スライス窓。g_tailLo = 尾の最初の cell(0 = 尾無し)。
int g_tailLo = 0;

void fillWindow(int)
{
  fillBaseline();
  const double eps[3] = {0, 0, 0};
  const int T[3] = {3200, 3200, 3200}, n[3] = {3, 3, 3};
  for (int t = 100; t < 105; ++t) depositPoint(2, -4, eps, T, n, t);
  if (g_tailLo == 0) return;
  for (int t = g_tailLo; t < g_tailLo + 3; ++t) {
    addStrip(0, 13, t, 200);
    addStrip(1, 10, t, 200);
    addStrip(2, 13, t, 200);
  }
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

// [8] ZS + pedestal 表(TODO/153)。fillGaussZs = [2] と同じガウスの点を ZS にしたもの: FPN は全セル 0、信号 ch は
// baseline(a, c) からの高さが 20 未満のセルを 0 にする(足した高さ > 35 は残る。common mode t/13 ≤ 7 は
// 100..109 では消える)。表の mean = baseline(a, c)(ped 窓の ±2 は平均 0)。
bool g_zs = false;

void fillGaussZs(int e)
{
  fillGauss(e);
  if (!g_zs) return;
  for (int a = 0; a < 4; ++a)
    for (int c = 0; c < 68; ++c)
      for (int t = 0; t < 512; ++t)
        if (runmetrics::isFpn(c) || g_adc[a][c][t] - baseline(a, c) < 20) g_adc[a][c][t] = 0;
}

// skipA < 0 なら全 ch、でなければ (skipA, skipC) を除く。
bool writePedTable(const std::string &path, int skipA, int skipC)
{
  FILE *fp = std::fopen(path.c_str(), "w");
  if (!fp) return false;
  std::fputs("{\"meta\": {\"note\": \"synthetic\"}, \"channels\": [\n", fp);
  bool first = true;
  for (int a = 0; a < 4; ++a)
    for (int c = 0; c < 68; ++c) {
      if (a == skipA && c == skipC) continue;
      std::fprintf(fp, "%s{\"asad\": 0, \"aget\": %d, \"ch\": %d, \"mean\": %d.0, \"rms\": 1.5, \"runs\": 1, \"is_fpn\": %s}",
                   first ? "" : ",\n", a, c, baseline(a, c), runmetrics::isFpn(c) ? "true" : "false");
      first = false;
    }
  std::fputs("\n]}\n", fp);
  return std::fclose(fp) == 0;
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
  check(uvwclosure::computeClosure(raw83, map, trk83, 0, 1, r, err, false), "computeClosure ok (" + err + ")");
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
  check(uvw_closure(raw83.c_str(), map.c_str(), trk83.c_str(), json83.c_str(), png83.c_str(), 0, false) == 0,
        "uvw_closure() returns 0");
  check(!js.empty() && readFile(json83) == js, "uvw_closure() wrote the same JSON");
  FileStat_t st;
  check(gSystem->GetPathInfo(png83.c_str(), st) == 0 && st.fSize > 0, "uvw_closure() wrote the PNG");

  // ---- maxEvents = 1(e0 だけ)--------------------------------------------------------------------
  std::printf("[1b] maxEvents = 1\n");
  uvwclosure::Result m1;
  check(uvwclosure::computeClosure(raw83, map, trk83, 1, 1, m1, err, false), "computeClosure ok (maxEvents 1)");
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
  check(writeRaw(raw84, 600, fillGauss, 0), "write Gaussian raw (600 events x 10 slices)");
  check(g_fixtureOk, "fixture (Gaussian): every deposited strip > 35, inside the plane");
  std::vector<UInt_t> ids84;
  std::vector<int> oks84;
  for (int e = 0; e < 600; ++e) {
    ids84.push_back(static_cast<UInt_t>(e));
    oks84.push_back(1);
  }
  check(writeTracks(trk84, ids84, oks84), "write Gaussian tracks (all ok)");
  uvwclosure::Result g;
  check(uvwclosure::computeClosure(raw84, map, trk84, 0, 1, g, err, false), "computeClosure ok (Gaussian)");
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
  check(uvwclosure::computeClosure(raw86, map, trk86, 0, 1, w1, err, false), "computeClosure ok (sliceCells 1)");
  checkCount(w1.slices, 2, "slices (sliceCells 1)");
  checkNear(w1.c, 5.0625, "c (sliceCells 1: (6.0 + 4.125) / 2)");
  checkNear(w1.residual.robust_sigma, 1.3899375, "robust_sigma (sliceCells 1: 1.4826 x 0.9375)");
  checkNear(w1.residual.rms, 0.9375, "rms (sliceCells 1)");
  check(uvwclosure::computeClosure(raw86, map, trk86, 0, 2, w2, err, false), "computeClosure ok (sliceCells 2)");
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
  check(!uvwclosure::computeClosure(raw86, map, trk86, 0, 0, w0, err, false), "sliceCells 0 fails (" + err + ")");

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
  check(!uvwclosure::computeClosure(raw83, map, bad, 0, 1, x, err, false) && contains(err, "entries"),
        "entry count mismatch fails (" + err + ")");
  check(writeTracks(bad, {500, 501, 7777, 503}, {1, 1, 0, 1}), "write tracks with eventId 7777 at entry 2");
  err.clear();
  check(!uvwclosure::computeClosure(raw83, map, bad, 0, 1, x, err, false) && contains(err, "eventId"),
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
    check(!uvwclosure::computeClosure(raw83, badMap, trk83, 0, 1, x, err, false) && contains(err, bm.keyword),
          "map with " + bm.what + " fails (" + err + ")");
  }
  check(writeLines(badMap, mapLines(false)), "write map without W strips");
  err.clear();
  check(!uvwclosure::computeClosure(raw83, badMap, trk83, 0, 1, x, err, false) && contains(err, "no strip"),
        "map without W strips fails (" + err + ")");

  // ---- [5] 使えるスライスが無い run → null ---------------------------------------------------------
  std::printf("[5] run without usable slices\n");
  const std::string raw85 = dir + "/run0085_raw.root", trk85 = dir + "/run0085_tracks.root";
  const std::string json85 = dir + "/run0085_uvw_closure.json";
  cleanup.insert(cleanup.end(), {raw85, trk85, json85});
  check(writeRaw(raw85, 1, [](int) { fillMain(2); }, 502), "write ok=0-only raw (e2)");
  check(writeTracks(trk85, {502}, {0}), "write ok=0-only tracks");
  uvwclosure::Result z;
  check(uvwclosure::computeClosure(raw85, map, trk85, 0, 1, z, err, false), "computeClosure ok (no usable slice)");
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

  // ---- [6] hitChannelsOnly -----------------------------------------------------------------------
  std::printf("[6] hitChannelsOnly: halo on every strip is ignored\n");
  const std::string raw87 = dir + "/run0087_raw.root", trk87 = dir + "/run0087_tracks.root";
  const std::string raw87h = dir + "/run0087h_raw.root";
  const std::string json87 = dir + "/run0087_uvw_closure.json";
  cleanup.insert(cleanup.end(), {raw87, trk87, raw87h, json87});
  g_fixtureOk = true;
  g_inexact = 0;
  g_halo = false;
  check(writeRaw(raw87, 2, fillHit, 700), "write hit-channel raw without halo (2 events)");
  g_halo = true;
  check(writeRaw(raw87h, 2, fillHit, 700), "write hit-channel raw with halo +60 on every strip");
  g_halo = false;
  check(g_fixtureOk, "fixture (hit channels): every deposited strip > 35, ADC inside 0..4095");
  checkCount(g_inexact, 0, "fixture (hit channels): centroids exact (D = 0)");
  check(writeTracks(trk87, {700, 701}, {1, 1}), "write hit-channel tracks");
  uvwclosure::Result hn, hh, an, ah;
  check(uvwclosure::computeClosure(raw87, map, trk87, 0, 1, hn, err, true), "computeClosure ok (hit only, no halo)");
  check(uvwclosure::computeClosure(raw87h, map, trk87, 0, 1, hh, err, true), "computeClosure ok (hit only, halo)");
  check(uvwclosure::computeClosure(raw87, map, trk87, 0, 1, an, err, false), "computeClosure ok (all strips, no halo)");
  check(uvwclosure::computeClosure(raw87h, map, trk87, 0, 1, ah, err, false), "computeClosure ok (all strips, halo)");
  const uvwclosure::Result *same[2] = {&hn, &hh};
  const char *sameName[2] = {"hit only, no halo", "hit only, halo"};
  for (int k = 0; k < 2; ++k) {
    const uvwclosure::Result &q = *same[k];
    const std::string nm = sameName[k];
    check(q.hit_channels_only, nm + ": hit_channels_only");
    checkCount(q.slices, 20, nm + ": slices");
    checkCount(q.slices_missing_view + q.slices_multi_cluster, 0, nm + ": no discarded slice");
    checkNear(q.c, 4.5, nm + ": c");
    checkNear(q.residual.robust_sigma, 1.4826, nm + ": robust_sigma (1.4826 x 1)");
    checkNear(q.residual.rms, std::sqrt(5.0), nm + ": rms (sqrt 5)");
    for (int v = 0; v < 3; ++v)
      checkNear(q.strips_median[v], 3, nm + ": strips median view " + std::to_string(v));
    checkNear(q.channels_used_median, 10.5, nm + ": channels_used_median ((10 + 11) / 2)");
  }
  checkNear(hn.quartile[0].q_lo, 9600, "hit only, no halo: slice charge 3 x 3200");
  checkNear(hh.quartile[0].q_lo, 9600 + 9 * 60, "hit only, halo: slice charge + 9 track strips x 60");
  check(!an.hit_channels_only, "all strips: hit_channels_only false");
  checkCount(an.slices, 20, "all strips, no halo: slices");
  checkNear(an.c, 4.5, "all strips, no halo: c (every deposited strip is a hit)");
  checkNear(an.residual.robust_sigma, 1.4826, "all strips, no halo: robust_sigma");
  checkNear(an.channels_used_median, 72, "all strips: channels_used_median = 72 read strips");
  checkCount(ah.slices, 0, "all strips, halo: slices (U halo gap beside the hit strips)");
  checkCount(ah.slices_multi_cluster, 20, "all strips, halo: slices_multi_cluster (halo clusters in U)");
  checkNear(ah.channels_used_median, 72, "all strips, halo: channels_used_median = 72");
  const std::string jh = uvwclosure::toJson(hh);
  check(contains(jh, "\"hit_channels_only\": true,"), "JSON \"hit_channels_only\": true");
  checkNear(jsonNum(jh, "{", "channels_used_median"), 10.5, "JSON channels_used_median");
  const std::string ja = uvwclosure::toJson(ah);
  check(contains(ja, "\"hit_channels_only\": false,"), "JSON \"hit_channels_only\": false");
  checkNear(jsonNum(ja, "{", "channels_used_median"), 72, "JSON channels_used_median (all strips)");
  // [6b] 隙間の無い halo
  std::printf("[6b] hitChannelsOnly: uniform halo, no shift\n");
  const std::string raw88 = dir + "/run0088_raw.root", trk88 = dir + "/run0088_tracks.root";
  cleanup.insert(cleanup.end(), {raw88, trk88});
  g_halo = true;
  check(writeRaw(raw88, 1, [](int) { fillHit(2); }, 800), "write uniform-halo raw (1 event)");
  g_halo = false;
  check(writeTracks(trk88, {800}, {1}), "write uniform-halo tracks");
  uvwclosure::Result un, ua;
  check(uvwclosure::computeClosure(raw88, map, trk88, 0, 1, un, err, true), "computeClosure ok (uniform halo, hit only)");
  check(uvwclosure::computeClosure(raw88, map, trk88, 0, 1, ua, err, false), "computeClosure ok (uniform halo, all strips)");
  checkCount(un.slices, 10, "uniform halo, hit only: slices");
  checkNear(un.c, 4.5, "uniform halo, hit only: c (halo ignored)");
  for (int v = 0; v < 3; ++v)
    checkNear(un.strips_median[v], 3, "uniform halo, hit only: strips median view " + std::to_string(v));
  checkNear(un.channels_used_median, 9, "uniform halo, hit only: channels_used_median (3 x 3)");
  checkCount(ua.slices, 10, "uniform halo, all strips: slices (24 strips = one cluster)");
  checkNear(ua.c, 16560.0 / 4640.0, "uniform halo, all strips: c = (3200 x 4.5 + 1440 x 1.5) / 4640");
  for (int v = 0; v < 3; ++v)
    checkNear(ua.strips_median[v], 24, "uniform halo, all strips: strips median view " + std::to_string(v));
  checkNear(ua.channels_used_median, 72, "uniform halo, all strips: channels_used_median");
  // 入口の既定 = hitChannelsOnly true(halo を無視する)、false を渡すと halo が入る
  check(uvw_closure(raw87h.c_str(), map.c_str(), trk87.c_str(), json87.c_str()) == 0, "uvw_closure() default returns 0");
  check(contains(readFile(json87), "\"hit_channels_only\": true,"), "uvw_closure() default is hitChannelsOnly = true");
  check(uvw_closure(raw87h.c_str(), map.c_str(), trk87.c_str(), json87.c_str(), nullptr, 0, false) == 0,
        "uvw_closure(..., false) returns 0");
  check(contains(readFile(json87), "\"hit_channels_only\": false,"), "uvw_closure(..., false) writes false");

  // ---- [7] スライス窓 ------------------------------------------------------------------------------
  std::printf("[7] slice window: a tail 20 cells from the body is ignored, 10 cells is used\n");
  const int tails[3] = {0, 124, 110};
  const char *tailName[3] = {"no tail", "tail at 124..126 (20 cells from the body)", "tail at 110..112 (10 cells)"};
  const long wantSlices[3] = {5, 5, 8};
  const std::string trk89 = dir + "/run0089_tracks.root";
  cleanup.push_back(trk89);
  check(writeTracks(trk89, {900}, {1}), "write window tracks");
  for (int k = 0; k < 3; ++k) {
    const std::string rawW = dir + "/run0089_" + std::to_string(k) + "_raw.root";
    cleanup.push_back(rawW);
    g_tailLo = tails[k];
    g_fixtureOk = true;
    g_inexact = 0;
    check(writeRaw(rawW, 1, fillWindow, 900), std::string("write window raw: ") + tailName[k]);
    g_tailLo = 0;
    check(g_fixtureOk, std::string("fixture (window): ") + tailName[k]);
    checkCount(g_inexact, 0, std::string("fixture (window): centroids exact: ") + tailName[k]);
    for (int hitOnly = 0; hitOnly < 2; ++hitOnly) {
      uvwclosure::Result w;
      const std::string nm = std::string(tailName[k]) + (hitOnly ? ", hit only" : ", all strips");
      check(uvwclosure::computeClosure(rawW, map, trk89, 0, 1, w, err, hitOnly == 1), "computeClosure ok (" + nm + ")");
      checkCount(w.slices, wantSlices[k], nm + ": slices");
      checkCount(w.slices_missing_view + w.slices_multi_cluster, 0, nm + ": no discarded slice");
      checkNear(w.c, 4.5, nm + ": c");
      checkNear(w.residual.robust_sigma, 0, nm + ": robust_sigma");
    }
  }
  {
    uvwclosure::Result w;
    check(uvwclosure::computeClosure(dir + "/run0089_0_raw.root", map, trk89, 0, 1, w, err, true), "computeClosure ok (JSON)");
    check(contains(uvwclosure::toJson(w), "\"slice_window_cells\": 15,"), "JSON \"slice_window_cells\": 15");
  }

  // ---- [8] ZS + pedestal 表(TODO/153)-----------------------------------------------------------------
  std::printf("[8] ZS raw + pedestal table: sigma_per_view matches the non-ZS value (5 %%)\n");
  const std::string rawP = dir + "/run0091_raw.root", rawZ = dir + "/run0092_raw.root";
  const std::string trkP = dir + "/run0091_tracks.root", trkZ = dir + "/run0092_tracks.root";
  const std::string pedT = dir + "/ped.json", pedMiss = dir + "/ped_missing.json";
  cleanup.insert(cleanup.end(), {rawP, rawZ, trkP, trkZ, pedT, pedMiss});
  std::vector<UInt_t> ids8;
  for (UInt_t i = 0; i < 200; ++i) ids8.push_back(i);
  const std::vector<int> oks8(200, 1);
  g_rnd.SetSeed(5148);
  g_zs = false;
  check(writeRaw(rawP, 200, fillGaussZs, 0), "write non-ZS Gaussian raw (200 events)");
  g_rnd.SetSeed(5148);
  g_zs = true;
  check(writeRaw(rawZ, 200, fillGaussZs, 0), "write ZS Gaussian raw (same points)");
  g_zs = false;
  check(writeTracks(trkP, ids8, oks8) && writeTracks(trkZ, ids8, oks8), "write tracks (200 ok)");
  check(writePedTable(pedT, -1, -1), "write pedestal table");
  int missA = 0, missC = 0;
  stripChannel(0, 12, missA, missC);  // 点の中心に近い U ストリップ
  check(writePedTable(pedMiss, missA, missC), "write pedestal table without one U strip channel");
  uvwclosure::Result p8, z8, zn8, pm8;
  check(uvwclosure::computeClosure(rawP, map, trkP, 0, 1, p8, err, true), "non-ZS, no table: ok (" + err + ")");
  check(uvwclosure::computeClosure(rawZ, map, trkZ, 0, 1, z8, err, true, pedT), "ZS + table: ok (" + err + ")");
  check(uvwclosure::computeClosure(rawP, map, trkP, 0, 1, pm8, err, true, pedT), "non-ZS + table: ok (" + err + ")");
  checkRel(z8.sigma_per_view, p8.sigma_per_view, 0.05, "test_zs_table_matches_non_zs: ZS + table sigma_per_view vs non-ZS");
  checkRel(pm8.sigma_per_view, p8.sigma_per_view, 0.05, "non-ZS + table sigma_per_view vs non-ZS");
  check(z8.slices > 1000, "ZS + table: slices used (got " + std::to_string(z8.slices) + ")");
  check(p8.ped_source == "cells_1_20" && z8.ped_source == "table", "ped_source: cells_1_20 without a table, table with one");
  checkCount(z8.ped_missing, 0, "ZS + table: ped_missing");
  checkCount(z8.agets_without_fpn, 0, "ZS + table: agets_without_fpn == 0 (FPN not used, no WARNING)");
  const std::string jz8 = uvwclosure::toJson(z8);
  check(contains(jz8, "\"ped_source\": \"table\",") && contains(jz8, "\"ped_missing\": 0,"), "JSON ped_source table / ped_missing 0");
  check(contains(uvwclosure::toJson(p8), "\"ped_source\": \"cells_1_20\","), "JSON ped_source cells_1_20 without a table");
  // 表に無い ch は飛ばして数える(黙って別の ped を使わない)
  check(uvwclosure::computeClosure(rawZ, map, trkZ, 0, 1, zn8, err, true, pedMiss), "ZS + table missing a strip: ok (" + err + ")");
  check(zn8.ped_missing > 0, "table missing a strip: ped_missing > 0 (got " + std::to_string(zn8.ped_missing) + ")");
  // 壊れた・無い表はエラー
  uvwclosure::Result em8;
  check(!uvwclosure::computeClosure(rawZ, map, trkZ, 0, 1, em8, err, true, dir + "/nope.json") && !err.empty(), "missing pedestal file: error");
  // 入口の第 8 引数
  const std::string json8 = dir + "/run0092_uvw_closure.json";
  cleanup.push_back(json8);
  check(uvw_closure(rawZ.c_str(), map.c_str(), trkZ.c_str(), json8.c_str(), nullptr, 0, true, pedT.c_str()) == 0, "uvw_closure() with pedJson returns 0");
  check(contains(readFile(json8), "\"ped_source\": \"table\","), "uvw_closure() JSON ped_source table");

  // 後片付け
  for (const std::string &p : cleanup) gSystem->Unlink(p.c_str());
  gSystem->Unlink(dir.c_str());

  std::printf("%s: %d failure(s)\n", g_failures == 0 ? "GREEN" : "RED", g_failures);
  return g_failures;
}
