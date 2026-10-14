// =============================================================================
// deconv.C — α run 1 本を整形器の応答核(TODO/138 のパルサー核)で逆畳み込みし、前後の電荷・幅・
// 雑音・ピーク時刻のばらつきを JSON にする(TODO/139 §1)。
//
//   root -l -b -q 'deconv.C("run0084_raw.root", "run0105_kernel.json", "run0084_l0.01.json", 0.01, "run0084_l0.01.png")'
//   root -l -b -q 'deconv.C("run0084_raw.root", "run0105_kernel.json", "out.json", 0.001, nullptr, 300)'  (先頭 300 件)
//
// 終了コード 0 = JSON(と PNG)を書いた、1 = 失敗(理由は stderr)。
//
// 入力:
//   - raw ツリー(SPEC §6.7): `adc[{4×nAsad}][68][512]/S`。先頭次元はリーフから(run_metrics.C と同じ)
//   - 核 JSON = pulser_kernel.C の出力。上位 `all` の `mean_norm`(341 点、offset k = −40 … +300 cell、
//     index 40 = k = 0 = ピーク = 1.0)だけを読む(`fpn` / `agets` は読まない)
//   - lambda = λ_rel(≥ 0)。実際の正則化 λ_abs = λ_rel × max_f |H(f)|²(核の大きさに依らない無次元の値。
//     TODO/139 §1 の例 1e-3 / 1e-2 / 1e-1 × max|H|²)。0 = 正則化なしの厳密な逆
//
// 核(TODO/139 §1): mean_norm を h[k](k = −40 … +300、ピークが t = 0)として、尾 +50 … +300 の平均
//   (≈ +1 % の一定オフセット — パルサー run の ped を cell 1..20 で取っているための遅い基線のずれ)を
//   **341 点すべてから**引く。長さ 341 でない・null がある・最大が index 40 でない核は失敗にする。
//
// 方法(TODO/139 §1、周波数領域の Wiener 型): 長さ N = 1024 の円環で
//   H = DFT(h を k mod N に置いたもの)、W(f) = H*(f) / (|H(f)|² + λ_abs)、
//   x̂ = IDFT(DFT(y) · W) の cell 0..511。y = 前処理後の 512 cell、cell 512..1023 は 0 詰め。
//   DFT は ROOT の TVirtualFFT(FFTW)。ROOT が FFTW 無しでビルドされていると TVirtualFFT::FFT が null を
//   返す(Mac の /opt/ROOT がそう)ので、そのときは素朴な O(N²) DFT(係数表つき)に落ちる(JSON の
//   transform = "fftw" / "dft"、dft のときは INFO を出す)。|H|² + λ_abs = 0 の周波数は W = 0 にして数える。
//   x̂ の単位: 核のピークを 1 にしているので、1 cell に来た点電荷は x̂ でも元の波形のピーク高さ(ADC)になる。
//
// 前処理(イベントごと・AGET ごと。run_metrics.C と同じ — pulser_kernel.C 経由で include して定数を使う):
//   読めた FPN ch のセル毎平均 fpn(t) を引き(FPN が読めない AGET は引かず agets_without_fpn に数える)、
//   各信号 ch(FPN 除く、全セル 0 の未読 ch 除く)で v = ADC − fpn − ped、ped = cell 1..20 の平均。
//   ヒット ch = run_metrics.C の定義(ピーク > 35 かつ ピーク ±5 cells の閾値超の和 q ≥ 2000)。
//   それ以外の読めた信号 ch = 雑音 ch(「ped 領域(ヒット無し ch)」)。どちらも逆畳み込みする。
//
// 指標(前 = v、後 = x̂。和・RMS・ピーク・FWHM は cell [2, 509) だけで取る — drift_end.C と同じ範囲で、
//   先頭の整定と窓端の跳ね・0 詰めの境目の折り返しを除く):
//   charge_event              イベントごとの Σ(ヒット ch の [2, 509) の和)の分布(ヒットのあるイベント)。
//                             ratio_median = 後の中央値 / 前の中央値
//   fwhm_cells                ヒット ch ごとの FWHM の分布(ピーク = [2, 509) の最大の最初の cell、
//                             半値の交点は隣のセルとの線形補間、[2, 509) に交点が無ければ数えない)
//   ped_rms                   雑音 ch ごとの [2, 509) の RMS √(Σw²/507)(0 を基準)の中央値。ratio = 後 / 前
//   peak_cell_spread_in_event イベント内のヒット ch のピーク cell の母標準偏差の分布(ヒット 2 本以上)
// 分位点は線形補間(h = (n−1)·q — run_metrics.C の quantile)。値が無ければ JSON は null。
//
// PNG(任意): ヒットのある最初のイベントで前のピークが最大のヒット ch の前(v)と後(x̂)を重ねた 1 枚
//   (x = cell 0..511)+ 指標の表。
// =============================================================================
#include "pulser_kernel.C"

#include <TCanvas.h>
#include <TGraph.h>
#include <TLegend.h>
#include <TLine.h>
#include <TPaveText.h>
#include <TROOT.h>
#include <TSystem.h>
#include <TVirtualFFT.h>

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <string>
#include <vector>

namespace deconvolution {

using runmetrics::kAdcThreshold;
using runmetrics::kCells;
using runmetrics::kChannels;
using runmetrics::kFpn;
using runmetrics::kNull;
using runmetrics::kPeakHalfCells;
using runmetrics::kPedHi;
using runmetrics::kPedLo;
using runmetrics::kStripQMin;
using runmetrics::Quantiles;

// ---- 定数(出典つき)-------------------------------------------------------------------------
// TODO/139 §1: 512 cell の波形を 0 詰めして 1024 点で変換する。
const int kN = 2 * kCells;  // 1024
// 核の窓 k = −40 … +300 = pulser_kernel.C の窓(核 JSON の作り手)。
const int kKernelLo = pulserkernel::kWinLo;                   // −40
const size_t kKernelLen = pulserkernel::kWinLen;              // 341(k = −40 … +300)
const size_t kKernelPeak = static_cast<size_t>(-kKernelLo);  // index 40 = k = 0
// TODO/139 §1: 核の尾の一定オフセットは +50 … +300 の平均(= pulser_kernel.C の尾の区間)。
const int kOffsetLo = pulserkernel::kTailLo;  // +50
const int kOffsetHi = pulserkernel::kTailHi;  // +300
// 指標を取る cell の範囲 [kCellLo, kCellHi) = drift_end.C のヒット cell の範囲(先頭の整定 cell 0, 1 と
// 窓端の跳ね cell 509..511 を除く。上端 509 = run_metrics.C の kClipEdgeBackMinCell)。
const int kCellLo = 2;
const int kCellHi = runmetrics::kClipEdgeBackMinCell;  // 509

// ---- 核 --------------------------------------------------------------------------------------
struct Kernel {
  std::vector<double> h;  // kKernelLen 個、index = k − kKernelLo(index 40 = ピーク)
  double offset = kNull;  // 引いた尾のオフセット(+50 … +300 の平均)
  double sum = kNull;     // オフセットを引いた後の Σh = H(0)
};

// mean_norm(341 点)→ 核。長さ・有限・最大が index 40 を確かめ、尾の平均を全点から引く。
bool kernelFromMeanNorm(const std::vector<double> &meanNorm, Kernel &kern, std::string &err)
{
  kern = Kernel();
  if (meanNorm.size() != kKernelLen) {
    err = "kernel mean_norm has " + std::to_string(meanNorm.size()) + " points, expected " +
          std::to_string(kKernelLen) + " (k = -40 ... +300)";
    return false;
  }
  for (size_t i = 0; i < kKernelLen; ++i) {
    if (!std::isfinite(meanNorm[i])) {
      err = "kernel mean_norm has null at index " + std::to_string(i) + " (k = " +
            std::to_string(static_cast<int>(i) + kKernelLo) + ")";
      return false;
    }
    if (meanNorm[i] > meanNorm[kKernelPeak]) {
      err = "kernel mean_norm maximum is not at index 40 (k = 0): index " + std::to_string(i) +
            " is larger";
      return false;
    }
  }
  double tail = 0;
  for (int k = kOffsetLo; k <= kOffsetHi; ++k) tail += meanNorm[static_cast<size_t>(k - kKernelLo)];
  kern.offset = tail / (kOffsetHi - kOffsetLo + 1);
  kern.h.resize(kKernelLen);
  kern.sum = 0;
  for (size_t i = 0; i < kKernelLen; ++i) {
    kern.h[i] = meanNorm[i] - kern.offset;
    kern.sum += kern.h[i];
  }
  return true;
}

// 核 JSON(pulser_kernel.C の toJson の形)から上位 `all` の `mean_norm` を読む。`"all":` の後で最初の
// `"mean_norm"` が all のもの(toJson は all を fpn / agets より先に書く)。
bool loadKernel(const std::string &path, Kernel &kern, std::string &err)
{
  kern = Kernel();
  FILE *fp = std::fopen(path.c_str(), "r");
  if (!fp) {
    err = "cannot open " + path;
    return false;
  }
  std::string s;
  char buf[4096];
  size_t got;
  while ((got = std::fread(buf, 1, sizeof(buf), fp)) > 0) s.append(buf, got);
  std::fclose(fp);

  const size_t all = s.find("\"all\":");
  if (all == std::string::npos) {
    err = "no \"all\" group in " + path + " (pulser_kernel.C output expected)";
    return false;
  }
  const size_t key = s.find("\"mean_norm\"", all);
  const size_t open = key == std::string::npos ? key : s.find('[', key);
  const size_t close = open == std::string::npos ? open : s.find(']', open);
  if (close == std::string::npos) {
    err = "no complete \"all\".\"mean_norm\" array in " + path;
    return false;
  }
  std::vector<double> y;
  size_t i = open + 1;
  while (i < close) {
    while (i < close && (s[i] == ' ' || s[i] == ',' || s[i] == '\n')) ++i;
    if (i >= close) break;
    if (s.compare(i, 4, "null") == 0) {
      y.push_back(kNull);
      i += 4;
      continue;
    }
    char *end = nullptr;
    const double x = std::strtod(s.c_str() + i, &end);
    if (end == s.c_str() + i) {
      err = "cannot parse \"all\".\"mean_norm\" in " + path;
      return false;
    }
    y.push_back(x);
    i = static_cast<size_t>(end - s.c_str());
  }
  if (!kernelFromMeanNorm(y, kern, err)) {
    err += " in " + path;
    return false;
  }
  return true;
}

// ---- Wiener 逆畳み込み -----------------------------------------------------------------------
// TVirtualFFT::FFT は FFTW が無いと呼ぶたびに "handler not found" を出すので、一度 null が返ったら
// 以後は試さない。
bool g_fftwMissing = false;

class Deconvolver {
 public:
  Deconvolver(const Kernel &kern, double lambdaRel)
      : re_(kN, 0.0), im_(kN, 0.0), tRe_(kN), tIm_(kN), wRe_(kN), wIm_(kN)
  {
    int n = kN;
    if (!g_fftwMissing) {
      fwd_ = TVirtualFFT::FFT(1, &n, "C2CFORWARD ES K");
      if (fwd_) bwd_ = TVirtualFFT::FFT(1, &n, "C2CBACKWARD ES K");
      if (!fwd_ || !bwd_) {
        delete fwd_;
        delete bwd_;
        fwd_ = bwd_ = nullptr;
        g_fftwMissing = true;
      }
    }
    if (!fwd_) {  // O(N²) DFT の係数表: cos / sin(2π m / N)
      cos_.resize(kN);
      sin_.resize(kN);
      for (int m = 0; m < kN; ++m) {
        cos_[m] = std::cos(2 * M_PI * m / kN);
        sin_[m] = std::sin(2 * M_PI * m / kN);
      }
    }
    // H = DFT(h を k mod N に置いたもの)
    for (size_t i = 0; i < kern.h.size(); ++i) {
      const int k = static_cast<int>(i) + kKernelLo;
      re_[(k + kN) % kN] = kern.h[i];
    }
    transform(false);
    for (int f = 0; f < kN; ++f) maxH2_ = std::max(maxH2_, re_[f] * re_[f] + im_[f] * im_[f]);
    lambdaAbs_ = lambdaRel * maxH2_;
    for (int f = 0; f < kN; ++f) {
      const double den = re_[f] * re_[f] + im_[f] * im_[f] + lambdaAbs_;
      if (den == 0) {
        wRe_[f] = wIm_[f] = 0;
        ++zeroBins_;
        continue;
      }
      wRe_[f] = re_[f] / den;  // W = H* / (|H|² + λ)
      wIm_[f] = -im_[f] / den;
    }
  }
  ~Deconvolver()
  {
    delete fwd_;
    delete bwd_;
  }
  Deconvolver(const Deconvolver &) = delete;
  Deconvolver &operator=(const Deconvolver &) = delete;

  // v(512 cell)→ x̂(512 cell)
  void apply(const std::vector<double> &v, std::vector<double> &x)
  {
    for (int t = 0; t < kN; ++t) {
      re_[t] = t < kCells ? v[t] : 0.0;
      im_[t] = 0.0;
    }
    transform(false);
    for (int f = 0; f < kN; ++f) {
      const double r = re_[f] * wRe_[f] - im_[f] * wIm_[f];
      const double i = re_[f] * wIm_[f] + im_[f] * wRe_[f];
      re_[f] = r;
      im_[f] = i;
    }
    transform(true);
    x.assign(re_.begin(), re_.begin() + kCells);
  }

  bool fftw() const { return fwd_ != nullptr; }
  double maxH2() const { return maxH2_; }
  double lambdaAbs() const { return lambdaAbs_; }
  long zeroBins() const { return zeroBins_; }

 private:
  // re_ / im_ をその場で変換。順 = Σ x e^(−2πi fn/N)、逆 = Σ X e^(+2πi fn/N) / N。
  void transform(bool inverse)
  {
    if (fwd_) {
      TVirtualFFT *fft = inverse ? bwd_ : fwd_;
      fft->SetPointsComplex(re_.data(), im_.data());
      fft->Transform();
      fft->GetPointsComplex(re_.data(), im_.data());
    } else {
      const double sgn = inverse ? 1.0 : -1.0;
      for (int f = 0; f < kN; ++f) {
        double sr = 0, si = 0;
        int m = 0;  // (f·n) mod N
        for (int n = 0; n < kN; ++n) {
          const double c = cos_[m], s = sgn * sin_[m];
          sr += re_[n] * c - im_[n] * s;
          si += re_[n] * s + im_[n] * c;
          m += f;
          if (m >= kN) m -= kN;
        }
        tRe_[f] = sr;
        tIm_[f] = si;
      }
      re_.swap(tRe_);
      im_.swap(tIm_);
    }
    if (inverse)
      for (int t = 0; t < kN; ++t) {
        re_[t] /= kN;
        im_[t] /= kN;
      }
  }

  TVirtualFFT *fwd_ = nullptr, *bwd_ = nullptr;
  std::vector<double> cos_, sin_;
  std::vector<double> re_, im_, tRe_, tIm_, wRe_, wIm_;
  double maxH2_ = 0, lambdaAbs_ = 0;
  long zeroBins_ = 0;
};

// ---- 波形の指標(cell [kCellLo, kCellHi))------------------------------------------------------
struct Pulse {
  int peak = -1;       // 最大の最初の cell
  double fwhm = kNull;  // 半値の交点(前・後、線形補間)の差。交点が範囲に無い・ピーク ≤ 0 なら NaN
};

Pulse pulse(const std::vector<double> &w)
{
  Pulse p;
  if (static_cast<int>(w.size()) < kCellHi) return p;
  p.peak = kCellLo;
  for (int t = kCellLo; t < kCellHi; ++t)
    if (w[t] > w[p.peak]) p.peak = t;
  const double half = w[p.peak] / 2;
  if (!(half > 0)) return p;
  double before = kNull, after = kNull;
  for (int t = p.peak - 1; t >= kCellLo; --t)
    if (w[t] < half) {
      before = t + (half - w[t]) / (w[t + 1] - w[t]);
      break;
    }
  for (int t = p.peak + 1; t < kCellHi; ++t)
    if (w[t] < half) {
      after = (t - 1) + (w[t - 1] - half) / (w[t - 1] - w[t]);
      break;
    }
  p.fwhm = after - before;  // どちらかが NaN なら NaN
  return p;
}

double rangeSum(const std::vector<double> &w)
{
  double s = 0;
  for (int t = kCellLo; t < kCellHi; ++t) s += w[t];
  return s;
}

double rangeRms(const std::vector<double> &w)
{
  double s = 0;
  for (int t = kCellLo; t < kCellHi; ++t) s += w[t] * w[t];
  return std::sqrt(s / (kCellHi - kCellLo));
}

// 母標準偏差 √(Σ(x − 平均)²/n)。2 個未満なら NaN。
double stddev(const std::vector<double> &x)
{
  if (x.size() < 2) return kNull;
  double m = 0;
  for (double a : x) m += a;
  m /= x.size();
  double s = 0;
  for (double a : x) s += (a - m) * (a - m);
  return std::sqrt(s / x.size());
}

// ---- run 1 本 --------------------------------------------------------------------------------
struct Side {  // 前(v)または後(x̂)
  Quantiles charge, fwhm, spread;
  double ped_rms = kNull;
};

struct Result {
  long run = -1;  // -1 = 名前から読めない(JSON は null)
  std::string raw_file, kernel_file;
  long events = 0, events_with_hits = 0;
  // 信号 ch は読めたが FPN が 1 本も読めなかった (イベント, AGET) の数(run_metrics.C と同じ)
  long agets_without_fpn = 0;
  long hit_waveforms = 0, noise_waveforms = 0;
  bool fftw = false;  // false = O(N²) DFT
  double lambda_rel = kNull, lambda_abs = kNull, max_h2 = kNull;
  double kernel_offset = kNull, kernel_sum = kNull;
  long zero_denominator_bins = 0;
  Side before, after;
  double charge_ratio_median = kNull;  // after.charge.median / before.charge.median
  double ped_rms_ratio = kNull;        // after.ped_rms / before.ped_rms
  // PNG の 1 波形(ヒットのある最初のイベントで前のピークが最大のヒット ch)
  long png_event = -1;
  int png_aget = -1, png_channel = -1;
  std::vector<double> png_before, png_after;
};

bool computeDeconv(const std::string &rawPath, const std::string &kernelPath, double lambdaRel,
                   int maxEvents, Result &r, std::string &err)
{
  r = Result();
  r.raw_file = runmetrics::resolvedPath(rawPath);
  r.kernel_file = runmetrics::resolvedPath(kernelPath);
  r.run = runmetrics::runFromName(rawPath);
  r.lambda_rel = lambdaRel;
  if (!std::isfinite(lambdaRel) || lambdaRel < 0) {
    err = "lambda must be a finite number >= 0 (got " + runmetrics::num(lambdaRel) + ")";
    return false;
  }
  Kernel kern;
  if (!loadKernel(kernelPath, kern, err)) return false;
  r.kernel_offset = kern.offset;
  r.kernel_sum = kern.sum;

  TFile f(rawPath.c_str(), "READ");
  if (f.IsZombie()) {
    err = "cannot open " + rawPath;
    return false;
  }
  TTree *tree = f.Get<TTree>("raw");
  if (!tree) {
    err = "no TTree 'raw' in " + rawPath + " (SPEC 6.7 raw tree expected)";
    return false;
  }
  TLeaf *leaf = tree->GetLeaf("adc");
  if (!leaf || std::string(leaf->GetTypeName()) != "Short_t" ||
      leaf->GetLen() % (kChannels * kCells) != 0) {
    err = "raw: branch 'adc' is not adc[N][68][512]/S in " + rawPath;
    return false;
  }
  const int nBlocks = leaf->GetLen() / (kChannels * kCells);  // AGET の数(4 × AsAd 数)
  std::vector<Short_t> adc(static_cast<size_t>(leaf->GetLen()));
  if (tree->SetBranchAddress("adc", adc.data()) < 0) {
    err = "raw: cannot read branch 'adc' in " + rawPath;
    return false;
  }

  Deconvolver dec(kern, lambdaRel);
  r.fftw = dec.fftw();
  r.max_h2 = dec.maxH2();
  r.lambda_abs = dec.lambdaAbs();
  r.zero_denominator_bins = dec.zeroBins();

  Long64_t n = tree->GetEntries();
  if (maxEvents > 0) n = std::min<Long64_t>(n, maxEvents);
  std::vector<double> chargeB, chargeA, fwhmB, fwhmA, spreadB, spreadA, rmsB, rmsA;
  std::vector<double> peaksB, peaksA;            // 1 イベント分
  std::vector<double> fpn(kCells), v(kCells), x;  // AGET 1 つ分の common mode、ch 1 本分
  for (Long64_t i = 0; i < n; ++i) {
    tree->GetEntry(i);
    double qB = 0, qA = 0, best = -1;
    int nHit = 0;
    peaksB.clear();
    peaksA.clear();
    for (int b = 0; b < nBlocks; ++b) {
      const Short_t *aget = &adc[static_cast<size_t>(b) * kChannels * kCells];
      // 1. common mode = 読めた FPN ch のセル毎平均(読めた FPN が無ければ 0 のまま)
      std::fill(fpn.begin(), fpn.end(), 0.0);
      int nFpn = 0;
      for (int fc : kFpn) {
        const Short_t *w = aget + fc * kCells;
        if (runmetrics::allZero(w)) continue;
        ++nFpn;
        for (int t = 0; t < kCells; ++t) fpn[t] += w[t];
      }
      if (nFpn > 0)
        for (double &a : fpn) a /= nFpn;
      bool signalRead = false;
      for (int c = 0; c < kChannels; ++c) {
        if (runmetrics::isFpn(c)) continue;
        const Short_t *w = aget + c * kCells;
        if (runmetrics::allZero(w)) continue;
        signalRead = true;
        // 2. ped と v
        double ped = 0;
        for (int t = kPedLo; t <= kPedHi; ++t) ped += w[t] - fpn[t];
        ped /= (kPedHi - kPedLo + 1);
        int peak = 0;
        for (int t = 0; t < kCells; ++t) {
          v[t] = w[t] - fpn[t] - ped;
          if (v[t] > v[peak]) peak = t;
        }
        // 3. ヒット ch = run_metrics.C の定義(ピークが閾値超、ピーク ±5 の電荷 q ≥ STRIP_Q_MIN)
        bool hit = v[peak] > kAdcThreshold;
        if (hit) {
          const int tLo = std::max(0, peak - kPeakHalfCells);
          const int tHi = std::min(kCells - 1, peak + kPeakHalfCells);
          double q = 0;
          for (int t = tLo; t <= tHi; ++t)
            if (v[t] > kAdcThreshold) q += v[t];
          hit = q >= kStripQMin;
        }
        // 4. 逆畳み込み
        dec.apply(v, x);
        if (!hit) {
          rmsB.push_back(rangeRms(v));
          rmsA.push_back(rangeRms(x));
          ++r.noise_waveforms;
          continue;
        }
        ++nHit;
        ++r.hit_waveforms;
        qB += rangeSum(v);
        qA += rangeSum(x);
        const Pulse pb = pulse(v), pa = pulse(x);
        if (std::isfinite(pb.fwhm)) fwhmB.push_back(pb.fwhm);
        if (std::isfinite(pa.fwhm)) fwhmA.push_back(pa.fwhm);
        peaksB.push_back(pb.peak);
        peaksA.push_back(pa.peak);
        if (r.png_event < 0 && v[peak] > best) {  // PNG の候補(このイベントで最大のピーク)
          best = v[peak];
          r.png_aget = b;
          r.png_channel = c;
          r.png_before = v;
          r.png_after = x;
        }
      }
      if (signalRead && nFpn == 0) ++r.agets_without_fpn;
    }
    if (nHit == 0) continue;
    if (r.png_event < 0) r.png_event = static_cast<long>(i);
    ++r.events_with_hits;
    chargeB.push_back(qB);
    chargeA.push_back(qA);
    if (peaksB.size() >= 2) {
      spreadB.push_back(stddev(peaksB));
      spreadA.push_back(stddev(peaksA));
    }
  }

  r.events = static_cast<long>(n);
  r.before.charge = runmetrics::quantiles(chargeB);
  r.after.charge = runmetrics::quantiles(chargeA);
  r.before.fwhm = runmetrics::quantiles(fwhmB);
  r.after.fwhm = runmetrics::quantiles(fwhmA);
  r.before.spread = runmetrics::quantiles(spreadB);
  r.after.spread = runmetrics::quantiles(spreadA);
  r.before.ped_rms = runmetrics::quantile(rmsB, 0.5);
  r.after.ped_rms = runmetrics::quantile(rmsA, 0.5);
  r.charge_ratio_median = r.after.charge.median / r.before.charge.median;  // NaN なら NaN
  r.ped_rms_ratio = r.after.ped_rms / r.before.ped_rms;
  return true;
}

// ---- JSON ------------------------------------------------------------------------------------
std::string toJson(const Result &r)
{
  using runmetrics::num;
  using runmetrics::obj;
  std::string s = "{\n";
  s += "  \"run\": " + (r.run < 0 ? std::string("null") : std::to_string(r.run)) + ",\n";
  s += "  \"raw_file\": " + runmetrics::str(r.raw_file) + ",\n";
  s += "  \"kernel_file\": " + runmetrics::str(r.kernel_file) + ",\n";
  s += "  \"events\": " + std::to_string(r.events) + ",\n";
  s += "  \"events_with_hits\": " + std::to_string(r.events_with_hits) + ",\n";
  s += "  \"agets_without_fpn\": " + std::to_string(r.agets_without_fpn) + ",\n";
  s += "  \"transform\": \"" + std::string(r.fftw ? "fftw" : "dft") + "\",\n";
  s += "  \"lambda_rel\": " + num(r.lambda_rel) + ",\n";
  s += "  \"lambda_abs\": " + num(r.lambda_abs) + ",\n";
  s += "  \"max_h2\": " + num(r.max_h2) + ",\n";
  s += "  \"zero_denominator_bins\": " + std::to_string(r.zero_denominator_bins) + ",\n";
  s += "  \"kernel_offset\": " + num(r.kernel_offset) + ",\n";
  s += "  \"kernel_sum\": " + num(r.kernel_sum) + ",\n";
  s += "  \"cell_range\": [" + std::to_string(kCellLo) + ", " + std::to_string(kCellHi) + "],\n";
  s += "  \"hit_waveforms\": " + std::to_string(r.hit_waveforms) + ",\n";
  s += "  \"noise_waveforms\": " + std::to_string(r.noise_waveforms) + ",\n";
  s += "  \"charge_event\": {\"before\": " + obj(r.before.charge) + ", \"after\": " +
       obj(r.after.charge) + ", \"ratio_median\": " + num(r.charge_ratio_median) + "},\n";
  s += "  \"fwhm_cells\": {\"before\": " + obj(r.before.fwhm) + ", \"after\": " + obj(r.after.fwhm) +
       "},\n";
  s += "  \"ped_rms\": {\"before\": " + num(r.before.ped_rms) + ", \"after\": " +
       num(r.after.ped_rms) + ", \"ratio\": " + num(r.ped_rms_ratio) + "},\n";
  s += "  \"peak_cell_spread_in_event\": {\"before\": " + obj(r.before.spread) + ", \"after\": " +
       obj(r.after.spread) + "}\n";
  s += "}\n";
  return s;
}

// ---- PNG -------------------------------------------------------------------------------------
// 1 ch の前後の重ね + 指標の表。描いた物は kCanDelete でキャンバスに持たせる(pulser_kernel.C と同じ)。
bool drawPng(const Result &r, const std::string &path)
{
  gROOT->SetBatch(kTRUE);
  TCanvas c("deconv", "deconvolution", 1000, 700);
  const bool have = r.png_before.size() == static_cast<size_t>(kCells) &&
                    r.png_after.size() == static_cast<size_t>(kCells);
  double lo = -1, hi = 1;
  if (have) {
    lo = std::min(*std::min_element(r.png_before.begin(), r.png_before.end()),
                  *std::min_element(r.png_after.begin(), r.png_after.end()));
    hi = std::max(*std::max_element(r.png_before.begin(), r.png_before.end()),
                  *std::max_element(r.png_after.begin(), r.png_after.end()));
    const double pad = 0.05 * (hi - lo + 1);
    lo -= pad;
    hi += 0.6 * (hi - lo);  // 上に表の場所
  }
  char title[256];
  std::snprintf(title, sizeof(title),
                "run %s, event %ld, AGET %d ch %d, lambda_rel %g;cell;ADC (before) / ADC "
                "peak-equivalent (after)",
                r.run < 0 ? "?" : std::to_string(r.run).c_str(), r.png_event, r.png_aget,
                r.png_channel, r.lambda_rel);
  if (!c.DrawFrame(0, lo, kCells, hi, have ? title : "no hit channel;cell;ADC")) return false;
  TLine *zero = new TLine(0, 0, kCells, 0);
  zero->SetLineStyle(3);
  zero->SetBit(kCanDelete);
  zero->Draw();
  if (have) {
    TGraph *gb = new TGraph(), *ga = new TGraph();
    for (int t = 0; t < kCells; ++t) {
      gb->SetPoint(t, t, r.png_before[t]);
      ga->SetPoint(t, t, r.png_after[t]);
    }
    gb->SetLineColor(kBlack);
    ga->SetLineColor(kRed + 1);
    for (TGraph *g : {gb, ga}) {
      g->SetLineWidth(2);
      g->SetBit(kCanDelete);
      g->Draw("L SAME");
    }
    TLegend *legend = new TLegend(0.12, 0.80, 0.40, 0.89);
    legend->SetBit(kCanDelete);
    legend->AddEntry(gb, "before (v)", "l");
    legend->AddEntry(ga, "after (deconvolved)", "l");
    legend->Draw();
  }
  TPaveText *table = new TPaveText(0.45, 0.62, 0.89, 0.89, "NDC");
  table->SetBit(kCanDelete);
  table->SetTextAlign(12);
  table->SetFillColor(kWhite);
  char line[256];
  std::snprintf(line, sizeof(line), "%ld events, %ld with hits, transform %s", r.events,
                r.events_with_hits, r.fftw ? "fftw" : "dft");
  table->AddText(line);
  std::snprintf(line, sizeof(line), "charge/event median: %.4g -> %.4g (ratio %.4g)",
                r.before.charge.median, r.after.charge.median, r.charge_ratio_median);
  table->AddText(line);
  std::snprintf(line, sizeof(line), "FWHM median [cell]: %.4g -> %.4g", r.before.fwhm.median,
                r.after.fwhm.median);
  table->AddText(line);
  std::snprintf(line, sizeof(line), "ped RMS (no-hit ch): %.4g -> %.4g (ratio %.4g)",
                r.before.ped_rms, r.after.ped_rms, r.ped_rms_ratio);
  table->AddText(line);
  std::snprintf(line, sizeof(line), "peak-cell spread median: %.4g -> %.4g",
                r.before.spread.median, r.after.spread.median);
  table->AddText(line);
  table->Draw();
  c.SaveAs(path.c_str());
  FileStat_t st;
  return gSystem->GetPathInfo(path.c_str(), st) == 0 && st.fSize > 0;
}

}  // namespace deconvolution

// 入口。0 = JSON(と PNG)を書いた、1 = 失敗(理由は stderr)。lambda = λ_rel(≥ 0、λ_abs = λ_rel·max|H|²)。
// maxEvents ≤ 0 = 全イベント。
int deconv(const char *rawFile, const char *kernelJson, const char *outJson, double lambda,
           const char *outPng = nullptr, int maxEvents = 0)
{
  TStopwatch sw;
  deconvolution::Result r;
  std::string err;
  if (!deconvolution::computeDeconv(rawFile, kernelJson, lambda, maxEvents, r, err)) {
    std::fprintf(stderr, "deconv: ERROR %s\n", err.c_str());
    return 1;
  }
  const std::string json = deconvolution::toJson(r);
  FILE *fp = std::fopen(outJson, "w");
  const bool written = fp && std::fputs(json.c_str(), fp) >= 0;
  const bool closed = fp && std::fclose(fp) == 0;
  if (!written || !closed) {
    std::fprintf(stderr, "deconv: ERROR cannot write %s\n", outJson);
    return 1;
  }
  if (outPng && *outPng && !deconvolution::drawPng(r, outPng)) {
    std::fprintf(stderr, "deconv: ERROR cannot write %s\n", outPng);
    return 1;
  }
  // 黙らないこと(CLAUDE.md): 変換の経路、引けなかった common mode、W = 0 にした周波数、空の指標
  if (!r.fftw)
    std::fprintf(stderr,
                 "deconv: INFO TVirtualFFT unavailable (ROOT built without FFTW) — using the "
                 "O(N^2) DFT fallback (\"transform\": \"dft\")\n");
  if (r.agets_without_fpn > 0)
    std::fprintf(stderr,
                 "deconv: WARNING %ld (event, AGET) had signal channels but no FPN channel read — "
                 "common mode not subtracted there (isFPNRead=false?)\n",
                 r.agets_without_fpn);
  if (r.zero_denominator_bins > 0)
    std::fprintf(stderr,
                 "deconv: WARNING %ld frequency bins had |H|^2 + lambda = 0 — W set to 0 there\n",
                 r.zero_denominator_bins);
  if (r.hit_waveforms == 0)
    std::fprintf(stderr, "deconv: WARNING no hit channel — charge / FWHM / spread are null\n");
  if (r.noise_waveforms == 0)
    std::fprintf(stderr, "deconv: WARNING no channel without hits — ped_rms is null\n");
  std::printf("deconv: %ld events, %ld hit / %ld noise waveforms, lambda_rel %g, charge ratio %.4g, "
              "ped RMS ratio %.4g, %.1f s -> %s\n",
              r.events, r.hit_waveforms, r.noise_waveforms, r.lambda_rel, r.charge_ratio_median,
              r.ped_rms_ratio, sw.RealTime(), outJson);
  return 0;
}
