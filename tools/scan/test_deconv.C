// =============================================================================
// test_deconv.C — deconv.C の合成テスト(TODO/139 §2)。
//
//   root -l -b -q tools/scan/test_deconv.C     → exit code = 失敗数(0 = green)
//
// 3 部構成。期待値の出典は各 check の上のコメント(手計算)。
//
// [1] 核の読み込み: pulser_kernel.C の toJson で書いた核 JSON(`all.mean_norm` 341 点、index 40 = ピーク)
//     を読み、オフセット(+50…+300 の平均)を全点から引くこと・`all` 以外(fpn / agets)を読まないこと・
//     壊れた入力(無いファイル / `all` 無し / null / 長さ違い / ピークが index 40 でない)で失敗することを見る。
//
// [2] 逆畳み込みの数学(関数単位、double の配列):
//     核 = CR-RC⁴(τ = 2 cell、ピーク k = 0 で 1、k ≥ +50 は 0 で切る — オフセット 0):
//       h(k) = (u/8)⁴ · e^(4 − u/2)、u = k + 8 > 0(u = 8 で最大 1)
//     真の電流 x = 矩形 cell 150..159 に 100 + 指数尾 100·e^(−(t−159)/8)(cell 160..299)。
//       Σx = 1000 + 100·Σ_{j=1}^{140} e^(−j/8) = 1000 + 100·e^(−1/8)(1 − e^(−17.5))/(1 − e^(−1/8))
//          = 1751.041377
//       FWHM(x) = 半値 50 の交点の差。前 = 149 + (50 − 0)/(100 − 0) = 149.5、
//       後 = 164 + (a − 50)/(a − b)、a = 100e^(−5/8) = 53.52614、b = 100e^(−6/8) = 47.23666
//          → 164.56064 → FWHM = 15.06064
//     y = h ∗ x(テストの中で直接の線形畳み込み — DFT とは独立の計算)。
//     - λ = 0 → x̂ = x(厳密)
//     - 雑音なしの電荷: Σx̂ = Σx / (1 + λ_rel)。Wiener の DC 利得 W(0) = H(0)/(H(0)² + λ_rel·max|H|²)
//       で、非負の核は max|H| = H(0) なので Σx̂ = Σy/H(0) · 1/(1 + λ_rel) = Σx/(1 + λ_rel)
//     - 雑音(σ = 2 ADC、TRandom3 seed 12345)あり、λ_rel = 1e-3 → 電荷が Σx の 1 % 以内
//     - λ_rel = 1e-3 → 1e-2 → 1e-1 で、雑音だけの入力の RMS が下がり、雑音なしの FWHM が広がる(単調)。
//       注: λ = 0 の FWHM(15.06 = 真値)は λ_rel = 1e-3(≈ 14.4)より広い — 矩形の縁で Gibbs の
//       行き過ぎがピークを持ち上げて半値が上がるため。単調性は 3 点の運用値で見る(numpy で確認済み)
//
// [3] raw → JSON(computeDeconv と入口 deconv())。核 = 二進の短い核(全セル整数になる):
//       S(k)·64 = 8, 32, 63, 48, 32, 20, 12, 6, 2(k = −2 … +6)、他は 0
//       核 JSON の mean_norm = S + 1/64(全 341 点にオフセット — ピーク 63/64 + 1/64 = 1)
//       ΣS = 223/64 = 3.484375、max|S|² = S(0)² = (223/64)² = 12.140869140625(非負の核)、
//       S(π) = Σ S(k)(−1)^k = (8 − 32 + 63 − 48 + 32 − 20 + 12 − 6 + 2)/64 = 11/64
//     電流の形(どちらも × 64 で、S と畳み込むと全セル整数):
//       形 A: 64·[1, 2, 4, 8, 4, 2, 1](cell t0 … t0+6)、Σ = 1408、ピーク t0+3、FWHM = 2
//             (半値 256: 前の交点 1 + (256−128)/(256−128) = 2、後 4 + 0 = 4)
//       形 B: 64·[8, 4, 2, 1](cell t0 … t0+3)、Σ = 960、ピーク t0、FWHM = 1.5
//             (半値 256: 前 −1 + 256/512 = −0.5、後 1 + 0 = 1)
//     S と畳み込んだ ADC 波形 Y(τ)、τ = t − t0(手計算 Y(τ) = Σ_j m(j)·S·64(τ − j)):
//       Y_A(τ = −2 … 12) = 8, 48, 159, 366, 668, 924, 888, 716, 509, 316, 176, 84, 32, 10, 2(Σ 4906 = 22·223)
//       Y_B(τ = −2 … 9)  = 64, 288, 648, 708, 606, 447, 288, 168, 84, 32, 10, 2(Σ 3345 = 15·223)
//       ピーク: Y_A は τ = 3(924)、Y_B は τ = 1(708 — 電流のピーク τ = 0 より 1 cell 遅い)
//       FWHM(Y_A) = (6 + 47/193) − (1 + 96/302) = 4.92564252(半値 462)
//       FWHM(Y_B) = (3 + 93/159) − (−1 + 66/360) = 4.40157233(半値 354)
//     ヒット ch の条件(run_metrics.C): Y_B のピーク ±5 の閾値 35 超の和 = 3345 − 10 − 2 = 3333 ≥ 2000、
//       Y_A = 4770 ≥ 2000 → ヒット
//
//   読む ch(partial readout の模型 — 他の信号 ch は全セル 0 = 未読で、数えない):
//     FPN ch {11, 22, 45, 56} は全 AGET、信号 ch は AGET 0: 3, 4, 5, 50 / AGET 1: 40, 41, 42 /
//     AGET 2: 12, 13, 14 / AGET 3: 20, 21, 30(イベントあたり 13 本)
//   読む ch の基線 = b(a,c) + cm(t)、b = 200 + 3c + 50a、cm = t/13(整数除算 — 他のテストと同じ)。
//   FPN を引けば common mode は消え、ヒット ch は v = Y ちょうど(ped 窓 cell 1..20 は平ら)。
//   ヒット無しの信号 ch は全セルに ±3 の交互(偶数 cell +3、奇数 −3。ped 窓の平均 0)→ v = ±3、
//     RMS(cell [2, 509))= 3。逆畳み込み後は窓の中ほどで 3·|W(π)|、W(π) = S(π)/(S(π)² + λ_abs)
//     → λ = 0: 3·64/11 = 17.454545、λ_rel = 0.01: 3·(11/64)/((11/64)² + 0.01·(223/64)²) = 3.415873、
//       λ_rel = 0.1: 0.414614(窓端の効果は ≤ 0.07 % — numpy で確認)
//   イベント e = 0 … 11:
//     e = 0..9: AGET 0 ch 3  = 形 B ×1 @ t0 = 100+e(ピーク 前 101+e / 後 100+e)
//               AGET 1 ch 40 = 形 A ×1 @ t0 = 103+e(前 / 後とも 106+e)
//               AGET 2 ch 12 = 形 B ×2 @ t0 = 101+e(偶数 e だけ。前 102+e / 後 101+e)
//     e = 10:   ヒット無し
//     e = 11:   AGET 0 ch 3 = 形 B ×1 @ t0 = 150 だけ(ヒット 1 本 — ばらつきには数えない)
//   全イベント共通のおとり:
//     halo:    AGET 0 ch 50 の cell 300 に +300(v = 303、q = 303 < 2000 → ヒットでない、雑音 ch に数える)
//     FPN:     AGET 3 の FPN ch 45 に 4·Y_B @ t0 = 200(FPN の平均に Y_B が入り、AGET 3 の信号 ch は
//              v = ±3 − Y_B。FPN ch を信号として使うと v = 3·Y_B でヒットになり電荷が変わる)
//     halo と AGET 3 の ch は RMS が ±3 の ch より大きい(前も後も — numpy で確認)。それが 4 本 × 12 = 48、
//     ±3 だけの ch が 130 − 48 = 82 本なので、中央値(h = 64.5)は ±3 の ch の値のまま。
//
//   数(手計算): 雑音 ch = 読めた信号 ch(13)− ヒット ch
//     e 偶数 13 − 3 = 10(×5)、奇数 11(×5)、e = 10: 13、e = 11: 12 → 計 130
//     ヒット波形 = 3·5 + 2·5 + 1 = 26
//   イベントの電荷(前 = Σ Y、後 = Σ 電流 / (1 + λ_rel)):
//     偶数 e: 3345 + 4906 + 2·3345 = 14941 / 960 + 1408 + 1920 = 4288
//     奇数 e: 3345 + 4906 = 8251 / 960 + 1408 = 2368
//     e = 11: 3345 / 960
//     昇順(n = 11、h = 10q): 前 [3345, 8251×5, 14941×5] → median = idx 5 = 8251、p10 = idx 1 = 8251、
//       p90 = idx 9 = 14941。後(λ = 0)は同じ並びで 2368 / 2368 / 4288
//   FWHM(26 波形: 形 B 16 本、形 A 10 本。昇順 h = 25q): median h = 12.5 → B、p10 → B、p90 h = 22.5 → A
//     前 4.40157233 / 4.40157233 / 4.92564252、後(λ = 0)1.5 / 1.5 / 2
//   ピーク cell の ch 間ばらつき(母標準偏差、ヒット 2 本以上のイベント 10 個):
//     前: 偶数 {101, 106, 102}+e → √(14/3) = 2.16024690、奇数 {101, 106} → 2.5
//     後: 偶数 {100, 106, 101}+e → √(62/9) = 2.62466929、奇数 {100, 106} → 3
//     昇順(n = 10、h = 9q): median = (idx 4 + idx 5)/2、p10 h = 0.9 → 小さい方、p90 h = 8.1 → 大きい方
//   PNG の波形: ヒットのある最初のイベント(e = 0)で前のピークが最大の ch = AGET 2 ch 12(2·708 = 1416)
// =============================================================================
#include "deconv.C"

#include <TRandom3.h>
#include <TSystem.h>

#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

namespace test_dc {

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
  std::snprintf(buf, sizeof(buf), "%s: got %.10g want %.10g (tol %.1g)", what.c_str(), got, want,
                tol);
  check(ok, buf);
}

// 相対誤差 |got/want − 1| ≤ rel
void checkRel(double got, double want, const std::string &what, double rel)
{
  const bool ok = std::isfinite(got) && std::fabs(got / want - 1.0) <= rel;
  char buf[256];
  std::snprintf(buf, sizeof(buf), "%s: got %.10g want %.10g (rel %.1g)", what.c_str(), got, want,
                rel);
  check(ok, buf);
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

bool writeText(const std::string &path, const std::string &text)
{
  FILE *fp = std::fopen(path.c_str(), "w");
  if (!fp) return false;
  const bool ok = std::fputs(text.c_str(), fp) >= 0;
  return std::fclose(fp) == 0 && ok;
}

// ---- 核 ----------------------------------------------------------------------------------------
const int kLen = 341;  // k = −40 … +300
size_t at(int k) { return static_cast<size_t>(k + 40); }

// 二進の短い核 S(k)(冒頭 [3])
const int kS64[9] = {8, 32, 63, 48, 32, 20, 12, 6, 2};
double dyadic(int k) { return (k >= -2 && k <= 6) ? kS64[k + 2] / 64.0 : 0.0; }

// CR-RC⁴、τ = 2 cell、ピーク k = 0 で 1、k ≥ 50 は 0(冒頭 [2])
double crrc(int k)
{
  const double u = k + 8.0;
  if (u <= 0 || k >= 50) return 0.0;
  return std::pow(u / 8.0, 4) * std::exp(4.0 - u / 2.0);
}

std::vector<double> sampled(double (*h)(int), double offset)
{
  std::vector<double> y(kLen);
  for (int k = -40; k <= 300; ++k) y[at(k)] = h(k) + offset;
  return y;
}

// pulser_kernel.C の toJson で核 JSON を書く。all = meanNorm、fpn と agets[0] はおとり(別の形)。
bool writeKernelJson(const std::string &path, const std::vector<double> &meanNorm)
{
  pulserkernel::Result r;
  r.run = 105;
  r.raw_file = "/synthetic/run0105_raw.root";
  r.events = 602;
  r.subtract_fpn = false;
  r.hasFpn = true;
  r.all.waveforms = 1000;
  r.all.channels = 256;
  r.all.mean_norm = meanNorm;
  r.all.mean_raw.resize(meanNorm.size());
  for (size_t i = 0; i < meanNorm.size(); ++i) r.all.mean_raw[i] = 940.0 * meanNorm[i];
  r.all.n.assign(meanNorm.size(), 1000);
  // おとり: fpn は平ら 0.5、agets[0] は k = 0 で 1、他は 0.25
  r.fpn.mean_norm.assign(kLen, 0.5);
  r.fpn.n.assign(kLen, 10);
  pulserkernel::Group g;
  g.mean_norm.assign(kLen, 0.25);
  g.mean_norm[at(0)] = 1.0;
  g.n.assign(kLen, 10);
  r.agets.push_back(g);
  return writeText(path, pulserkernel::toJson(r));
}

// ---- 関数単位の道具 ----------------------------------------------------------------------------
// y(t) = Σ_k h(k)·x(t − k)、t = 0..511(DFT と独立の直接計算)
std::vector<double> convolve(const std::vector<double> &x, double (*h)(int))
{
  std::vector<double> y(512, 0.0);
  for (int t = 0; t < 512; ++t)
    for (int k = -40; k <= 300; ++k) {
      const int j = t - k;
      if (j >= 0 && j < 512) y[t] += h(k) * x[j];
    }
  return y;
}

double maxAbsDiff(const std::vector<double> &a, const std::vector<double> &b)
{
  double m = 0;
  for (size_t i = 0; i < a.size() && i < b.size(); ++i) m = std::max(m, std::fabs(a[i] - b[i]));
  return a.size() == b.size() ? m : 1e300;
}

// ---- 合成 raw(冒頭 [3])-----------------------------------------------------------------------
Short_t g_adc[4][68][512];

const int kYA[15] = {8, 48, 159, 366, 668, 924, 888, 716, 509, 316, 176, 84, 32, 10, 2};  // τ = −2..12
const int kYB[12] = {64, 288, 648, 708, 606, 447, 288, 168, 84, 32, 10, 2};             // τ = −2..9

int baseline(int a, int c) { return 200 + 3 * c + 50 * a; }
int commonMode(int t) { return t / 13; }

// ch を平らな基線(+ cm)に戻して形 Y·scale を t0 に置く(Y の先頭は τ = −2)
void putPulse(int a, int c, const int *Y, int len, int scale, int t0)
{
  for (int t = 0; t < 512; ++t) g_adc[a][c][t] = static_cast<Short_t>(baseline(a, c) + commonMode(t));
  for (int i = 0; i < len; ++i) {
    const int t = t0 - 2 + i;
    g_adc[a][c][t] = static_cast<Short_t>(g_adc[a][c][t] + scale * Y[i]);
  }
}

// 読む信号 ch(冒頭 [3]。−1 = 無し)
const int kRead[4][4] = {{3, 4, 5, 50}, {40, 41, 42, -1}, {12, 13, 14, -1}, {20, 21, 30, -1}};

void fillEvent(int e)
{
  for (int a = 0; a < 4; ++a)
    for (int c = 0; c < 68; ++c)
      for (int t = 0; t < 512; ++t) g_adc[a][c][t] = 0;  // 未読
  for (int a = 0; a < 4; ++a) {
    for (int c : runmetrics::kFpn)
      for (int t = 0; t < 512; ++t) g_adc[a][c][t] = static_cast<Short_t>(baseline(a, c) + commonMode(t));
    for (int c : kRead[a]) {
      if (c < 0) continue;
      for (int t = 0; t < 512; ++t)
        g_adc[a][c][t] = static_cast<Short_t>(baseline(a, c) + commonMode(t) + ((t % 2 == 0) ? 3 : -3));
    }
  }
  g_adc[0][50][300] = static_cast<Short_t>(g_adc[0][50][300] + 300);  // halo
  // FPN のおとり: ch 45 に 4·Y_B @ 200(平らな基線に戻してから)
  for (int t = 0; t < 512; ++t) g_adc[3][45][t] = static_cast<Short_t>(baseline(3, 45) + commonMode(t));
  for (int i = 0; i < 12; ++i) g_adc[3][45][198 + i] = static_cast<Short_t>(g_adc[3][45][198 + i] + 4 * kYB[i]);
  if (e <= 9) {
    putPulse(0, 3, kYB, 12, 1, 100 + e);
    putPulse(1, 40, kYA, 15, 1, 103 + e);
    if (e % 2 == 0) putPulse(2, 12, kYB, 12, 2, 101 + e);
  } else if (e == 11) {
    putPulse(0, 3, kYB, 12, 1, 150);
  }
}

bool writeRaw(const std::string &path, int nEvents)
{
  TFile f(path.c_str(), "RECREATE");
  if (f.IsZombie()) return false;
  TTree tree("raw", "synthetic raw (test_deconv.C)");
  UInt_t eventId = 0;
  ULong64_t eventTime = 0;
  tree.Branch("eventId", &eventId);
  tree.Branch("eventTime", &eventTime);
  tree.Branch("adc", g_adc, "adc[4][68][512]/S");
  for (int k = 0; k < nEvents; ++k) {
    fillEvent(k);
    eventId = static_cast<UInt_t>(k);
    eventTime = 1000ULL + static_cast<ULong64_t>(k) * 5000000ULL;
    tree.Fill();
  }
  tree.Write();
  return true;
}

// W(π) の手計算(冒頭 [3])
double wPi(double lambdaRel)
{
  const double s = 11.0 / 64.0, s0 = 223.0 / 64.0;
  return s / (s * s + lambdaRel * s0 * s0);
}

}  // namespace test_dc

int test_deconv()
{
  using namespace test_dc;
  namespace dc = deconvolution;
  const std::string dir =
      std::string(gSystem->TempDirectory()) + "/test_deconv_" + std::to_string(gSystem->GetPid());
  gSystem->mkdir(dir.c_str(), kTRUE);
  std::string err;

  // ==== [1] 核の読み込み ===========================================================================
  std::printf("[kernel JSON]\n");
  const std::string kernelJson = dir + "/run0105_kernel.json";
  check(writeKernelJson(kernelJson, sampled(dyadic, 1.0 / 64.0)), "write dyadic kernel JSON");
  dc::Kernel kern;
  check(dc::loadKernel(kernelJson, kern, err), "loadKernel ok (" + err + ")");
  checkCount(static_cast<long>(kern.h.size()), 341, "kernel length (k = -40 … +300)");
  if (kern.h.size() == 341) {
    // オフセット = +50…+300 の平均 = 1/64(mean_norm = S + 1/64、S は +50 以降 0)
    checkNear(kern.offset, 1.0 / 64.0, "kernel offset = mean of +50…+300");
    // 引いた後 = S そのもの: ピーク(index 40)= 63/64、k = −2 → 8/64、k = +6 → 2/64
    checkNear(kern.h[at(0)], 63.0 / 64.0, "h(0) = 63/64 (peak at index 40)");
    checkNear(kern.h[at(-2)], 8.0 / 64.0, "h(-2) = 8/64");
    checkNear(kern.h[at(1)], 48.0 / 64.0, "h(+1) = 48/64 (all, not the fpn / agets decoys)");
    checkNear(kern.h[at(6)], 2.0 / 64.0, "h(+6) = 2/64");
    // オフセットは尾だけでなく全点から引く: k = −40 も 0(尾だけなら 1/64 が残る)
    checkNear(kern.h[at(-40)], 0.0, "h(-40) = 0 (offset removed from all points)");
    checkNear(kern.h[at(300)], 0.0, "h(+300) = 0");
    double tail = 0;
    for (int k = 50; k <= 300; ++k) tail += kern.h[at(k)];
    checkNear(tail / 251.0, 0.0, "mean of h over +50…+300 = 0 after removal");
  }
  // ΣS = 223/64
  checkNear(kern.sum, 223.0 / 64.0, "kernel sum = 223/64");

  // 壊れた入力
  check(!dc::loadKernel(dir + "/missing_kernel.json", kern, err) && !err.empty(),
        "loadKernel fails on a missing file (" + err + ")");
  const std::string noAll = dir + "/no_all.json";
  writeText(noAll, "{\n  \"run\": 105,\n  \"fpn\": null\n}\n");
  check(!dc::loadKernel(noAll, kern, err), "loadKernel fails without \"all\" (" + err + ")");
  const std::string withNull = dir + "/with_null.json";
  {
    std::vector<double> y = sampled(dyadic, 1.0 / 64.0);
    y[at(-40)] = runmetrics::kNull;  // toJson は null と書く
    writeKernelJson(withNull, y);
  }
  check(!dc::loadKernel(withNull, kern, err), "loadKernel fails on null in mean_norm (" + err + ")");
  const std::string shortJson = dir + "/short.json";
  {
    std::vector<double> y = sampled(dyadic, 1.0 / 64.0);
    y.pop_back();  // 340 点
    writeKernelJson(shortJson, y);
  }
  check(!dc::loadKernel(shortJson, kern, err), "loadKernel fails on 340 points (" + err + ")");
  const std::string offPeak = dir + "/off_peak.json";
  {
    std::vector<double> y = sampled(dyadic, 1.0 / 64.0);
    y[at(10)] = 1.2;  // index 40 より大きい値
    writeKernelJson(offPeak, y);
  }
  check(!dc::loadKernel(offPeak, kern, err),
        "loadKernel fails when the maximum is not at index 40 (" + err + ")");
  const std::string truncated = dir + "/truncated.json";
  writeText(truncated, "{\n  \"all\": {\n    \"mean_norm\": [0, 0.5, 1\n");
  check(!dc::loadKernel(truncated, kern, err), "loadKernel fails on an unterminated array (" + err + ")");

  // ==== [2] 逆畳み込みの数学(関数単位)==============================================================
  // 核の位置: v = S を cell 200 に置いた波形(v(200 + k) = S(k))→ λ = 0 で x̂ = δ(200)
  // (index 40 = k = 0 が x̂ の時刻 0 に対応)
  std::printf("[deconvolution: alignment and exact inverse]\n");
  dc::Kernel dy;
  check(dc::kernelFromMeanNorm(sampled(dyadic, 1.0 / 64.0), dy, err), "kernelFromMeanNorm (dyadic)");
  {
    dc::Deconvolver d(dy, 0.0);
    std::printf("  (transform: %s)\n", d.fftw() ? "TVirtualFFT / FFTW" : "O(N^2) DFT fallback");
    std::vector<double> v(512, 0.0), x, want(512, 0.0);
    for (int k = -2; k <= 6; ++k) v[200 + k] = dyadic(k);
    want[200] = 1.0;
    d.apply(v, x);
    checkNear(maxAbsDiff(x, want), 0.0, "delta: x̂ = δ(200) at λ = 0 (max |x̂ − δ|)", 1e-9);
    checkNear(d.maxH2(), 12.140869140625, "max|H|² = (223/64)² (DC of a non-negative kernel)", 1e-9);
    checkCount(d.zeroBins(), 0, "no zero denominators");
  }

  std::printf("[deconvolution: CR-RC^4 kernel, rectangle + exponential tail current]\n");
  dc::Kernel cr;
  check(dc::kernelFromMeanNorm(sampled(crrc, 0.0), cr, err), "kernelFromMeanNorm (CR-RC^4)");
  checkNear(cr.offset, 0.0, "CR-RC^4 offset = 0 (zero beyond +49)");
  std::vector<double> xTrue(512, 0.0);
  for (int t = 150; t <= 159; ++t) xTrue[t] = 100.0;
  for (int t = 160; t <= 299; ++t) xTrue[t] = 100.0 * std::exp(-(t - 159) / 8.0);
  double sumX = 0;
  for (double q : xTrue) sumX += q;
  // 冒頭 [2]: Σx = 1751.041377、FWHM(x) = 15.06064
  checkNear(sumX, 1751.041377, "true current: Σx (hand)", 1e-6);
  checkNear(dc::pulse(xTrue).fwhm, 15.0606410, "true current: FWHM (hand)", 1e-6);
  checkCount(dc::pulse(xTrue).peak, 150, "true current: peak cell = 150 (first of the plateau)");
  const std::vector<double> yClean = convolve(xTrue, crrc);
  TRandom3 rng(12345);
  std::vector<double> noise(512);
  for (double &n : noise) n = rng.Gaus(0.0, 2.0);
  std::vector<double> yNoisy(512);
  for (int t = 0; t < 512; ++t) yNoisy[t] = yClean[t] + noise[t];
  {
    dc::Deconvolver d(cr, 0.0);
    std::vector<double> x;
    d.apply(yClean, x);
    checkNear(maxAbsDiff(x, xTrue), 0.0, "λ = 0: x̂ = x exactly (max |x̂ − x|)", 1e-6);
    checkNear(dc::rangeSum(x), sumX, "λ = 0: charge = Σx", 1e-6);
  }
  const double lambdas[3] = {1e-3, 1e-2, 1e-1};
  double rmsPrev = 1e300, fwhmPrev = 0;
  for (double lr : lambdas) {
    char tag[64];
    std::snprintf(tag, sizeof(tag), "λ_rel = %g", lr);
    dc::Deconvolver d(cr, lr);
    checkNear(d.lambdaAbs(), lr * d.maxH2(), std::string(tag) + ": λ_abs = λ_rel·max|H|²", 1e-12);
    std::vector<double> x, xn, xo;
    d.apply(yClean, x);
    checkRel(dc::rangeSum(x), sumX / (1.0 + lr), std::string(tag) + ": clean charge = Σx/(1+λ_rel)",
             1e-6);
    d.apply(noise, xn);
    const double rms = dc::rangeRms(xn);
    const double fwhm = dc::pulse(x).fwhm;
    std::printf("  (%s: noise RMS %.4f, clean FWHM %.4f)\n", tag, rms, fwhm);
    check(rms < rmsPrev, std::string(tag) + ": noise RMS decreases with λ");
    check(fwhm > fwhmPrev, std::string(tag) + ": clean FWHM widens with λ");
    rmsPrev = rms;
    fwhmPrev = fwhm;
    if (lr == 1e-3) {
      d.apply(yNoisy, xo);
      checkRel(dc::rangeSum(xo), sumX, "λ_rel = 1e-3 with σ = 2 noise: charge within 1 % of Σx", 0.01);
    }
  }

  // ==== [3] raw → JSON ===========================================================================
  const std::string raw = dir + "/run0084_raw.root";
  check(writeRaw(raw, 12), "write synthetic raw (12 events)");
  const double sqrt14_3 = std::sqrt(14.0 / 3.0), sqrt62_9 = std::sqrt(62.0 / 9.0);
  const double fwhmA = (6.0 + 47.0 / 193.0) - (1.0 + 96.0 / 302.0);   // 4.92564252
  const double fwhmB = (3.0 + 93.0 / 159.0) - (-1.0 + 66.0 / 360.0);  // 4.40157233

  std::printf("[raw, λ_rel = 0 (exact inverse)]\n");
  dc::Result r0;
  err.clear();
  check(dc::computeDeconv(raw, kernelJson, 0.0, 0, r0, err), "computeDeconv ok (" + err + ")");
  checkCount(r0.run, 84, "run (from run0084_raw.root)");
  checkCount(r0.events, 12, "events");
  checkCount(r0.events_with_hits, 11, "events_with_hits (e = 10 has none)");
  checkCount(r0.agets_without_fpn, 0, "agets_without_fpn");
  checkCount(r0.hit_waveforms, 26, "hit_waveforms (3·5 + 2·5 + 1; FPN decoy and halo not hits)");
  checkCount(r0.noise_waveforms, 130, "noise_waveforms (10·5 + 11·5 + 13 + 12; unread excluded)");
  checkNear(r0.kernel_offset, 1.0 / 64.0, "kernel_offset");
  checkNear(r0.kernel_sum, 223.0 / 64.0, "kernel_sum");
  checkNear(r0.max_h2, 12.140869140625, "max_h2", 1e-9);
  checkNear(r0.lambda_abs, 0.0, "lambda_abs (λ = 0)");
  checkCount(r0.zero_denominator_bins, 0, "zero_denominator_bins");
  // 電荷(イベントごと)
  checkNear(r0.before.charge.median, 8251, "charge before median");
  checkNear(r0.before.charge.p10, 8251, "charge before p10");
  checkNear(r0.before.charge.p90, 14941, "charge before p90");
  checkNear(r0.after.charge.median, 2368, "charge after median (= Σ current)", 1e-6);
  checkNear(r0.after.charge.p10, 2368, "charge after p10", 1e-6);
  checkNear(r0.after.charge.p90, 4288, "charge after p90", 1e-6);
  checkNear(r0.charge_ratio_median, 2368.0 / 8251.0, "charge_ratio_median = after / before medians",
            1e-9);
  // FWHM(ヒット ch ごと)
  checkNear(r0.before.fwhm.median, fwhmB, "FWHM before median (shape B)", 1e-6);
  checkNear(r0.before.fwhm.p10, fwhmB, "FWHM before p10", 1e-6);
  checkNear(r0.before.fwhm.p90, fwhmA, "FWHM before p90 (shape A)", 1e-6);
  checkNear(r0.after.fwhm.median, 1.5, "FWHM after median (current B)", 1e-6);
  checkNear(r0.after.fwhm.p90, 2.0, "FWHM after p90 (current A)", 1e-6);
  // 雑音 ch の RMS
  checkNear(r0.before.ped_rms, 3.0, "ped RMS before (±3 pattern)");
  checkRel(r0.after.ped_rms, 3.0 * wPi(0.0), "ped RMS after = 3·64/11", 2e-3);
  // ピーク cell の ch 間ばらつき
  checkNear(r0.before.spread.median, (sqrt14_3 + 2.5) / 2, "peak spread before median", 1e-9);
  checkNear(r0.before.spread.p10, sqrt14_3, "peak spread before p10", 1e-9);
  checkNear(r0.before.spread.p90, 2.5, "peak spread before p90", 1e-9);
  checkNear(r0.after.spread.median, (sqrt62_9 + 3.0) / 2, "peak spread after median", 1e-9);
  checkNear(r0.after.spread.p10, sqrt62_9, "peak spread after p10", 1e-9);
  checkNear(r0.after.spread.p90, 3.0, "peak spread after p90", 1e-9);
  // PNG の波形
  checkCount(r0.png_event, 0, "png waveform: event 0");
  checkCount(r0.png_aget, 2, "png waveform: AGET 2");
  checkCount(r0.png_channel, 12, "png waveform: ch 12 (largest peak 1416)");
  if (r0.png_before.size() == 512 && r0.png_after.size() == 512) {
    checkNear(r0.png_before[102], 1416, "png before(102) = 2·708");
    checkNear(r0.png_after[101], 1024, "png after(101) = 2·64·8", 1e-6);
  } else {
    check(false, "png waveforms have 512 cells");
  }
  const std::string js0 = dc::toJson(r0);
  check(contains(js0, std::string("\"transform\": \"") + (r0.fftw ? "fftw" : "dft") + "\""),
        "JSON transform");

  std::printf("[raw, λ_rel = 0.01 and 0.1]\n");
  dc::Result r1, r2;
  check(dc::computeDeconv(raw, kernelJson, 0.01, 0, r1, err), "computeDeconv ok (0.01)");
  check(dc::computeDeconv(raw, kernelJson, 0.1, 0, r2, err), "computeDeconv ok (0.1)");
  checkNear(r1.lambda_rel, 0.01, "lambda_rel 0.01");
  checkNear(r1.lambda_abs, 0.12140869140625, "lambda_abs = 0.01·(223/64)²", 1e-12);
  // 電荷 = Σ 電流 / (1 + λ_rel)(DC 利得。窓外への漏れは ≤ 1e-6 — numpy で確認)
  checkRel(r1.after.charge.median, 2368 / 1.01, "λ 0.01: charge after median = 2368/1.01", 1e-5);
  checkRel(r1.after.charge.p90, 4288 / 1.01, "λ 0.01: charge after p90 = 4288/1.01", 1e-5);
  checkRel(r2.after.charge.median, 2368 / 1.1, "λ 0.1: charge after median = 2368/1.1", 1e-5);
  checkNear(r1.before.charge.median, 8251, "λ 0.01: charge before unchanged");
  // 雑音の RMS は λ とともに下がる(手計算 3·|W(π)|)、前は不変
  checkNear(r1.before.ped_rms, 3.0, "λ 0.01: ped RMS before unchanged");
  checkRel(r1.after.ped_rms, 3.0 * wPi(0.01), "λ 0.01: ped RMS after = 3·|W(π)| = 3.415873", 2e-3);
  checkRel(r2.after.ped_rms, 3.0 * wPi(0.1), "λ 0.1: ped RMS after = 3·|W(π)| = 0.414614", 2e-3);
  checkRel(r1.ped_rms_ratio, wPi(0.01), "λ 0.01: ped_rms_ratio = after / before = |W(π)|", 2e-3);
  check(r0.after.ped_rms > r1.after.ped_rms && r1.after.ped_rms > r2.after.ped_rms,
        "ped RMS after decreases with λ (0 > 0.01 > 0.1)");
  // 幅は λ とともに広がる(形 B: 1.5 → 2.43 → 3.57 — numpy)
  check(r0.after.fwhm.median < r1.after.fwhm.median && r1.after.fwhm.median < r2.after.fwhm.median,
        "FWHM after median widens with λ (0 < 0.01 < 0.1)");
  // ピーク cell は λ で動かない(形 A は対称、形 B は電流のピーク — numpy)
  checkNear(r1.after.spread.median, (sqrt62_9 + 3.0) / 2, "λ 0.01: peak spread after median", 1e-9);

  // ---- JSON ------------------------------------------------------------------------------------
  const std::string js1 = dc::toJson(r1);
  check(contains(js1, "\"run\": 84,"), "JSON run");
  check(contains(js1, "\"events\": 12,"), "JSON events");
  check(contains(js1, "\"lambda_rel\": 0.01,"), "JSON lambda_rel");
  check(contains(js1, "\"cell_range\": [2, 509],"), "JSON cell_range");
  check(contains(js1, "\"kernel_offset\": 0.015625,"), "JSON kernel_offset");
  check(contains(js1, "\"kernel_sum\": 3.484375,"), "JSON kernel_sum");
  check(contains(js1, "\"hit_waveforms\": 26,"), "JSON hit_waveforms");
  check(contains(js1, "\"noise_waveforms\": 130,"), "JSON noise_waveforms");
  check(contains(js1, "\"before\": {\"median\": 8251, \"p10\": 8251, \"p90\": 14941}"),
        "JSON charge before object");
  check(contains(js1, "\"ped_rms\": {\"before\": 3, \"after\": "), "JSON ped_rms object");
  for (const char *key :
       {"\"raw_file\"", "\"kernel_file\"", "\"events_with_hits\"", "\"agets_without_fpn\"",
        "\"transform\"", "\"lambda_abs\"", "\"max_h2\"", "\"zero_denominator_bins\"",
        "\"charge_event\"", "\"ratio_median\"", "\"fwhm_cells\"", "\"ratio\"",
        "\"peak_cell_spread_in_event\"", "\"after\""})
    check(contains(js1, key), std::string("JSON key ") + key);

  // ---- 入口 ----------------------------------------------------------------------------------
  const std::string outJson = dir + "/run0084_deconv.json";
  const std::string outPng = dir + "/run0084_deconv.png";
  check(deconv(raw.c_str(), kernelJson.c_str(), outJson.c_str(), 0.01, outPng.c_str()) == 0,
        "deconv() returns 0");
  check(readFile(outJson) == js1, "deconv() wrote the same JSON");
  check(contains(readFile(outJson), "\"kernel_file\": \"" + runmetrics::resolvedPath(kernelJson)),
        "JSON kernel_file = resolved kernel path");
  FileStat_t st;
  check(gSystem->GetPathInfo(outPng.c_str(), st) == 0 && st.fSize > 0, "deconv() wrote the PNG");
  check(deconv((dir + "/missing_raw.root").c_str(), kernelJson.c_str(), outJson.c_str(), 0.01) == 1,
        "deconv() returns 1 for a missing raw file");
  check(deconv(raw.c_str(), (dir + "/missing_kernel.json").c_str(), outJson.c_str(), 0.01) == 1,
        "deconv() returns 1 for a missing kernel file");
  check(deconv(raw.c_str(), kernelJson.c_str(), outJson.c_str(), -0.1) == 1,
        "deconv() returns 1 for a negative λ");

  // ---- maxEvents -----------------------------------------------------------------------------
  // 先頭 3 イベント(e = 0, 1, 2): ヒット 3 + 2 + 3 = 8、雑音 10 + 11 + 10 = 31、
  // 電荷 前 [14941, 8251, 14941] → median 14941
  std::printf("[maxEvents = 3]\n");
  dc::Result m;
  check(dc::computeDeconv(raw, kernelJson, 0.0, 3, m, err), "computeDeconv ok (maxEvents 3)");
  checkCount(m.events, 3, "events (maxEvents 3)");
  checkCount(m.events_with_hits, 3, "events_with_hits (maxEvents 3)");
  checkCount(m.hit_waveforms, 8, "hit_waveforms (maxEvents 3)");
  checkCount(m.noise_waveforms, 31, "noise_waveforms (maxEvents 3)");
  checkNear(m.before.charge.median, 14941, "charge before median (maxEvents 3)");

  // 後片付け
  for (const std::string &q :
       {kernelJson, noAll, withNull, shortJson, offPeak, truncated, raw, outJson, outPng})
    gSystem->Unlink(q.c_str());
  gSystem->Unlink(dir.c_str());

  std::printf("%s: %d failure(s)\n", g_failures == 0 ? "GREEN" : "RED", g_failures);
  return g_failures;
}
