// =============================================================================
// pulser_kernel.C — パルサー run 1 本 → 整形器の応答核(平均波形)と形の指標(TODO/138 §2)。
//
//   root -l -b -q 'pulser_kernel.C("run0090_raw.root", "run0090_pulser.json", "run0090_pulser.png")'
//   root -l -b -q 'pulser_kernel.C("run0090_raw.root", "out.json", nullptr, 100)'   (先頭 100 イベント)
//
// 終了コード 0 = JSON(と PNG)を書いた、1 = 失敗(理由は stderr)。
//
// 入力 = raw ツリー(SPEC §6.7、tools/root_sink/raw_tree.hpp): TTree `raw`、
//   `adc[{4×nAsad}][68][512]/S`(生 ADC、FPN 込み、未読 ch は全セル 0)。先頭次元(= AGET の数)は
//   焼き込まず TLeaf::GetLen() / (68·512) で求める(run_metrics.C と同じ)。
//
// 手順(イベントごと・AGET ごと。定数と FPN の扱いは run_metrics.C と同じ — それを include して使う):
//   1. 読めた FPN ch {11,22,45,56}(全セル 0 でない)のセル毎平均 fpn(t) を作る(読めた FPN が無い
//      AGET は fpn = 0 — 引かない。数は agets_without_fpn として WARNING)
//   2. 各信号 ch(FPN 除く、全セル 0 の未読 ch は除く)で v(t) = ADC(t) − fpn(t) − ped、
//      ped = (ADC − fpn) の cell 1..20 の平均
//   3. ピーク = v 最大の最初の cell、振幅 A = v(ピーク)。A > 100 の ch のうち、生 ADC に 4095 を
//      含む ch は飽和として数えて除き、残りを使う
//   4. 使う ch の v をピークで揃える(offset k = t − ピーク、窓 k = −40 … +300)。AGET ごと・全体で、
//      全イベント・全 ch を足して平均: 生の平均 mean_raw(k) = Σv / n(k)、正規化した平均
//      mean_norm(k) = Σ(v / A) / n(k)。n(k) = その offset が読み出し窓 0..511 の中にあった波形の数
//      (ピークが窓の端に寄るとはみ出した offset は在るものだけで平均 — n を JSON に出し、全体で
//      n(k) < 波形数の offset があれば WARNING)
//
// 指標(AGET ごと + 全体。形の指標は正規化した平均 mean_norm から):
//   waveforms / channels                 使った波形の数(イベント × ch)/ 1 回以上使った ch の数
//   saturated_waveforms / _channels      A > 100 で 4095 を含むので除いた波形の数 / その ch の数
//   amplitude_median_adc                 使った波形の A の中央値
//   rise_10_90_cells                     ピークから前へ見て mean_norm が 0.9 / 0.1 を最初に下回る交点の差
//                                        (隣のセルとの線形補間)
//   fwhm_cells                           0.5 の交点(ピークの前と後、線形補間)の差
//   undershoot.{min_ratio,min_adc,cell}  k = +1…+300 の mean_norm の最小値・その k での mean_raw・その k
//   tail_50_300.{rms,p2p}_{ratio,adc}    k = +50…+300 の mean_norm / mean_raw の 0 を基準にした RMS
//                                        √(Σy² / 個数)と、ピーク間(max − min)= 尾の「揺れ」
//   peak_cell                            使った波形のピーク cell の分布(median / p10 / p90)
//   peak_cell_range_in_event             イベント内の使った ch のピーク cell の max − min の分布
//                                        (= パルサーの同時性。ch が無いイベントは数えない)
//   mean_norm / mean_raw / n             窓 k = −40 … +300 の配列(値が無い所は null)= 139 の核
// 分位点は線形補間(h = (n−1)·q — run_metrics.C の quantile)。値が無ければ JSON は null。
//
// PNG(任意): 正規化した平均応答を AGET ごとに 1 本ずつ重ねた 1 枚(線形、x = −40…+300、
//   y = −0.3…1.05)。
// =============================================================================
#include "run_metrics.C"

#include <TCanvas.h>
#include <TGraph.h>
#include <TH1.h>
#include <TLegend.h>
#include <TLine.h>
#include <TROOT.h>
#include <TSystem.h>

#include <algorithm>
#include <climits>
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

namespace pulserkernel {

using runmetrics::kAdcMax;
using runmetrics::kCells;
using runmetrics::kChannels;
using runmetrics::kFpn;
using runmetrics::kNull;
using runmetrics::kPedHi;
using runmetrics::kPedLo;
using runmetrics::Quantiles;

// ---- 定数(出典つき)-------------------------------------------------------------------------
// TODO/138 §2 の発注: ピーク(ped 減算後)が ped + 100 ADC を超える ch だけ使う。
const double kMinAmplitude = 100.0;
// TODO/138 §2 の発注: ピーク揃えの窓 −40 … +300 cell。
const int kWinLo = -40;
const int kWinHi = 300;
const size_t kWinLen = static_cast<size_t>(kWinHi - kWinLo + 1);  // 341
// TODO/138 §2 の発注: 尾(揺れ)の区間 t = +50 … +300 cell(25 MHz で 2 µs 以降 = §2 の「2 µs 以降の尾」)。
const int kTailLo = 50;
const int kTailHi = 300;

struct Group {
  long waveforms = 0, channels = 0, saturated_waveforms = 0, saturated_channels = 0;
  double amplitude_median_adc = kNull;
  double rise_10_90_cells = kNull, fwhm_cells = kNull;
  double undershoot_min_adc = kNull, undershoot_min_ratio = kNull, undershoot_cell = kNull;
  double tail_rms_adc = kNull, tail_rms_ratio = kNull, tail_p2p_adc = kNull, tail_p2p_ratio = kNull;
  Quantiles peak_cell, peak_cell_range_in_event;
  std::vector<double> mean_norm, mean_raw;  // kWinLen 個、index = k − kWinLo、値が無い所は NaN
  std::vector<long> n;                      // kWinLen 個
};

struct Result {
  long run = -1;  // -1 = 名前から読めない(JSON は null)
  std::string raw_file;
  long events = 0;
  // 信号 ch は読めたが FPN が 1 本も読めなかった (イベント, AGET) の数(run_metrics.C と同じ)
  long agets_without_fpn = 0;
  Group all;
  std::vector<Group> agets;  // adc の先頭次元の順(ELITPC は AsAd ブロック × 4 + AGET)
};

// 集計の途中経過(1 group 分)。
struct Acc {
  std::vector<double> sumNorm = std::vector<double>(kWinLen, 0.0);
  std::vector<double> sumRaw = std::vector<double>(kWinLen, 0.0);
  std::vector<long> n = std::vector<long>(kWinLen, 0);
  std::vector<double> amplitudes, peaks, ranges;
  long saturated = 0;
  int evMin = INT_MAX, evMax = -1;  // このイベントのピーク cell の min / max
};

void addWaveform(Acc &acc, const std::vector<double> &v, int peak, double amp)
{
  for (int k = kWinLo; k <= kWinHi; ++k) {
    const int t = peak + k;
    if (t < 0 || t >= kCells) continue;
    const size_t i = static_cast<size_t>(k - kWinLo);
    acc.sumRaw[i] += v[t];
    acc.sumNorm[i] += v[t] / amp;
    ++acc.n[i];
  }
  acc.amplitudes.push_back(amp);
  acc.peaks.push_back(peak);
  acc.evMin = std::min(acc.evMin, peak);
  acc.evMax = std::max(acc.evMax, peak);
}

// イベントの終わり: このイベントで使った ch があればピーク cell の max − min を 1 個積む。
void endEvent(Acc &acc)
{
  if (acc.evMax >= 0) acc.ranges.push_back(acc.evMax - acc.evMin);
  acc.evMin = INT_MAX;
  acc.evMax = -1;
}

bool hasSaturation(const Short_t *w)
{
  for (int t = 0; t < kCells; ++t)
    if (w[t] == kAdcMax) return true;
  return false;
}

double at(const std::vector<double> &y, int k) { return y[static_cast<size_t>(k - kWinLo)]; }

// ピークから前へ見て y が level を最初に下回る cell k と k+1 の間の交点(線形補間)。
// 窓の端まで下回らない・値が無い(NaN)なら NaN。
double crossingBefore(const std::vector<double> &y, double level)
{
  for (int k = -1; k >= kWinLo; --k) {
    const double a = at(y, k), b = at(y, k + 1);
    if (!std::isfinite(a)) return kNull;
    if (a < level) return k + (level - a) / (b - a);
  }
  return kNull;
}

// ピークから後へ見て y が level を最初に下回る cell k−1 と k の間の交点(線形補間)。
double crossingAfter(const std::vector<double> &y, double level)
{
  for (int k = 1; k <= kWinHi; ++k) {
    const double a = at(y, k - 1), b = at(y, k);
    if (!std::isfinite(b)) return kNull;
    if (b < level) return (k - 1) + (a - level) / (a - b);
  }
  return kNull;
}

// k = lo … hi の y(値のあるものだけ)の √(Σy²/個数) と max − min。
void rmsAndP2p(const std::vector<double> &y, int lo, int hi, double &rms, double &p2p)
{
  double s2 = 0, mx = -1e300, mn = 1e300;
  long cnt = 0;
  for (int k = lo; k <= hi; ++k) {
    const double x = at(y, k);
    if (!std::isfinite(x)) continue;
    s2 += x * x;
    mx = std::max(mx, x);
    mn = std::min(mn, x);
    ++cnt;
  }
  rms = cnt > 0 ? std::sqrt(s2 / cnt) : kNull;
  p2p = cnt > 0 ? mx - mn : kNull;
}

Group finish(const Acc &acc, long channels, long saturatedChannels)
{
  Group g;
  g.waveforms = static_cast<long>(acc.amplitudes.size());
  g.channels = channels;
  g.saturated_waveforms = acc.saturated;
  g.saturated_channels = saturatedChannels;
  g.n = acc.n;
  g.mean_norm.assign(kWinLen, kNull);
  g.mean_raw.assign(kWinLen, kNull);
  for (size_t i = 0; i < kWinLen; ++i) {
    if (acc.n[i] == 0) continue;
    g.mean_norm[i] = acc.sumNorm[i] / acc.n[i];
    g.mean_raw[i] = acc.sumRaw[i] / acc.n[i];
  }
  g.amplitude_median_adc = runmetrics::quantile(acc.amplitudes, 0.5);
  g.peak_cell = runmetrics::quantiles(acc.peaks);
  g.peak_cell_range_in_event = runmetrics::quantiles(acc.ranges);
  if (g.waveforms == 0) return g;

  g.rise_10_90_cells = crossingBefore(g.mean_norm, 0.9) - crossingBefore(g.mean_norm, 0.1);
  g.fwhm_cells = crossingAfter(g.mean_norm, 0.5) - crossingBefore(g.mean_norm, 0.5);
  int kMin = 0;
  for (int k = 1; k <= kWinHi; ++k) {
    const double x = at(g.mean_norm, k);
    if (std::isfinite(x) && (kMin == 0 || x < at(g.mean_norm, kMin))) kMin = k;
  }
  if (kMin > 0) {
    g.undershoot_min_ratio = at(g.mean_norm, kMin);
    g.undershoot_min_adc = at(g.mean_raw, kMin);
    g.undershoot_cell = kMin;
  }
  rmsAndP2p(g.mean_norm, kTailLo, kTailHi, g.tail_rms_ratio, g.tail_p2p_ratio);
  rmsAndP2p(g.mean_raw, kTailLo, kTailHi, g.tail_rms_adc, g.tail_p2p_adc);
  return g;
}

bool computePulserKernel(const std::string &rawPath, int maxEvents, Result &r, std::string &err)
{
  r = Result();
  r.raw_file = runmetrics::resolvedPath(rawPath);
  r.run = runmetrics::runFromName(rawPath);

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

  Long64_t n = tree->GetEntries();
  if (maxEvents > 0) n = std::min<Long64_t>(n, maxEvents);
  std::vector<Acc> accs(static_cast<size_t>(nBlocks));
  Acc all;
  // ch を 1 回以上使った / 飽和で除いた(AGET × 68)
  std::vector<char> used(static_cast<size_t>(nBlocks) * kChannels, 0);
  std::vector<char> saturated(static_cast<size_t>(nBlocks) * kChannels, 0);
  std::vector<double> fpn(kCells), v(kCells);
  for (Long64_t i = 0; i < n; ++i) {
    tree->GetEntry(i);
    for (int b = 0; b < nBlocks; ++b) {
      Acc &acc = accs[static_cast<size_t>(b)];
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
        for (double &x : fpn) x /= nFpn;
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
        // 3. 振幅の下限と飽和
        const double amp = v[peak];
        if (amp <= kMinAmplitude) continue;
        const size_t idx = static_cast<size_t>(b) * kChannels + c;
        if (hasSaturation(w)) {
          ++acc.saturated;
          ++all.saturated;
          saturated[idx] = 1;
          continue;
        }
        // 4. ピーク揃えで足す
        used[idx] = 1;
        addWaveform(acc, v, peak, amp);
        addWaveform(all, v, peak, amp);
      }
      endEvent(acc);
      if (signalRead && nFpn == 0) ++r.agets_without_fpn;
    }
    endEvent(all);
  }

  r.events = static_cast<long>(n);
  long allChannels = 0, allSaturatedChannels = 0;
  for (int b = 0; b < nBlocks; ++b) {
    long ch = 0, sat = 0;
    for (int c = 0; c < kChannels; ++c) {
      ch += used[static_cast<size_t>(b) * kChannels + c];
      sat += saturated[static_cast<size_t>(b) * kChannels + c];
    }
    r.agets.push_back(finish(accs[static_cast<size_t>(b)], ch, sat));
    allChannels += ch;
    allSaturatedChannels += sat;
  }
  r.all = finish(all, allChannels, allSaturatedChannels);
  return true;
}

// ---- JSON ------------------------------------------------------------------------------------
std::string array(const std::vector<double> &y)
{
  std::string s = "[";
  for (size_t i = 0; i < y.size(); ++i) s += (i ? ", " : "") + runmetrics::num(y[i]);
  return s + "]";
}

std::string array(const std::vector<long> &y)
{
  std::string s = "[";
  for (size_t i = 0; i < y.size(); ++i) s += (i ? ", " : "") + std::to_string(y[i]);
  return s + "]";
}

std::string groupBody(const Group &g, const std::string &ind)
{
  using runmetrics::num;
  using runmetrics::obj;
  std::string s;
  s += ind + "\"waveforms\": " + std::to_string(g.waveforms) + ", \"channels\": " +
       std::to_string(g.channels) + ", \"saturated_waveforms\": " +
       std::to_string(g.saturated_waveforms) + ", \"saturated_channels\": " +
       std::to_string(g.saturated_channels) + ",\n";
  s += ind + "\"amplitude_median_adc\": " + num(g.amplitude_median_adc) + ",\n";
  s += ind + "\"rise_10_90_cells\": " + num(g.rise_10_90_cells) + ", \"fwhm_cells\": " +
       num(g.fwhm_cells) + ",\n";
  s += ind + "\"undershoot\": {\"min_adc\": " + num(g.undershoot_min_adc) + ", \"min_ratio\": " +
       num(g.undershoot_min_ratio) + ", \"cell\": " + num(g.undershoot_cell) + "},\n";
  s += ind + "\"tail_50_300\": {\"rms_adc\": " + num(g.tail_rms_adc) + ", \"rms_ratio\": " +
       num(g.tail_rms_ratio) + ", \"p2p_adc\": " + num(g.tail_p2p_adc) + ", \"p2p_ratio\": " +
       num(g.tail_p2p_ratio) + "},\n";
  s += ind + "\"peak_cell\": " + obj(g.peak_cell) + ",\n";
  s += ind + "\"peak_cell_range_in_event\": " + obj(g.peak_cell_range_in_event) + ",\n";
  s += ind + "\"mean_norm\": " + array(g.mean_norm) + ",\n";
  s += ind + "\"mean_raw\": " + array(g.mean_raw) + ",\n";
  s += ind + "\"n\": " + array(g.n) + "\n";
  return s;
}

std::string toJson(const Result &r)
{
  using runmetrics::num;
  std::string s = "{\n";
  s += "  \"run\": " + (r.run < 0 ? std::string("null") : std::to_string(r.run)) + ",\n";
  s += "  \"raw_file\": " + runmetrics::str(r.raw_file) + ",\n";
  s += "  \"events\": " + std::to_string(r.events) + ",\n";
  s += "  \"agets_without_fpn\": " + std::to_string(r.agets_without_fpn) + ",\n";
  s += "  \"min_amplitude_adc\": " + num(kMinAmplitude) + ",\n";
  s += "  \"window_cells\": [" + std::to_string(kWinLo) + ", " + std::to_string(kWinHi) + "],\n";
  s += "  \"all\": {\n" + groupBody(r.all, "    ") + "  },\n";
  s += "  \"agets\": [\n";
  for (size_t b = 0; b < r.agets.size(); ++b) {
    s += "    {\n      \"aget\": " + std::to_string(b) + ",\n" + groupBody(r.agets[b], "      ") +
         "    }" + (b + 1 < r.agets.size() ? "," : "") + "\n";
  }
  s += "  ]\n}\n";
  return s;
}

// ---- PNG -------------------------------------------------------------------------------------
// 正規化した平均応答を AGET ごとに 1 本(波形の無い AGET は描かない)。描いた物は kCanDelete で
// キャンバスに持たせる(キャンバスを閉じるときに消える)。
bool drawPng(const Result &r, const std::string &path)
{
  gROOT->SetBatch(kTRUE);
  TCanvas c("pulser_kernel", "pulser kernel", 1000, 700);
  const std::string title = "normalized mean pulser response (run " +
                            (r.run < 0 ? std::string("?") : std::to_string(r.run)) + ", " +
                            std::to_string(r.events) + " events);cell from peak;amplitude / peak";
  if (!c.DrawFrame(kWinLo, -0.3, kWinHi, 1.05, title.c_str())) return false;
  TLine *zero = new TLine(kWinLo, 0, kWinHi, 0);
  zero->SetLineStyle(3);
  zero->SetBit(kCanDelete);
  zero->Draw();
  const int colors[8] = {kBlack,       kRed + 1,    kBlue + 1, kGreen + 2,
                         kMagenta + 1, kOrange + 7, kCyan + 2, kGray + 2};
  TLegend *legend = new TLegend(0.62, 0.55, 0.89, 0.89);
  legend->SetBit(kCanDelete);
  int drawn = 0;
  for (size_t b = 0; b < r.agets.size(); ++b) {
    const Group &g = r.agets[b];
    if (g.waveforms == 0) continue;
    TGraph *gr = new TGraph();
    for (int k = kWinLo; k <= kWinHi; ++k) {
      const double y = at(g.mean_norm, k);
      if (std::isfinite(y)) gr->SetPoint(gr->GetN(), k, y);
    }
    gr->SetLineColor(colors[drawn % 8]);
    gr->SetLineStyle(static_cast<Style_t>(1 + drawn / 8));
    gr->SetLineWidth(2);
    gr->SetBit(kCanDelete);
    gr->Draw("L SAME");
    legend->AddEntry(gr, ("AGET " + std::to_string(b) + " (" + std::to_string(g.waveforms) +
                          " wf)").c_str(), "l");
    ++drawn;
  }
  if (drawn > 8) legend->SetNColumns(2);  // ELITPC = 16 AGET
  legend->Draw();
  c.SaveAs(path.c_str());
  FileStat_t st;
  return gSystem->GetPathInfo(path.c_str(), st) == 0 && st.fSize > 0;
}

}  // namespace pulserkernel

// 入口。0 = JSON(と PNG)を書いた、1 = 失敗(理由は stderr)。maxEvents ≤ 0 = 全イベント。
int pulser_kernel(const char *rawFile, const char *outJson, const char *outPng = nullptr,
                  int maxEvents = 0)
{
  TStopwatch sw;
  pulserkernel::Result r;
  std::string err;
  if (!pulserkernel::computePulserKernel(rawFile, maxEvents, r, err)) {
    std::fprintf(stderr, "pulser_kernel: ERROR %s\n", err.c_str());
    return 1;
  }
  const std::string json = pulserkernel::toJson(r);
  FILE *fp = std::fopen(outJson, "w");
  const bool written = fp && std::fputs(json.c_str(), fp) >= 0;
  const bool closed = fp && std::fclose(fp) == 0;
  if (!written || !closed) {
    std::fprintf(stderr, "pulser_kernel: ERROR cannot write %s\n", outJson);
    return 1;
  }
  if (outPng && *outPng && !pulserkernel::drawPng(r, outPng)) {
    std::fprintf(stderr, "pulser_kernel: ERROR cannot write %s\n", outPng);
    return 1;
  }
  // 黙らないこと(CLAUDE.md): 引けなかった common mode、使える波形なし、窓のはみ出し
  if (r.agets_without_fpn > 0)
    std::fprintf(stderr,
                 "pulser_kernel: WARNING %ld (event, AGET) had signal channels but no FPN channel "
                 "read — common mode not subtracted there (isFPNRead=false?)\n",
                 r.agets_without_fpn);
  if (r.all.waveforms == 0)
    std::fprintf(stderr,
                 "pulser_kernel: WARNING no channel above %.0f ADC without saturation — metrics are "
                 "null\n",
                 pulserkernel::kMinAmplitude);
  long shortOffsets = 0;
  for (long cnt : r.all.n)
    if (cnt < r.all.waveforms) ++shortOffsets;
  if (shortOffsets > 0)
    std::fprintf(stderr,
                 "pulser_kernel: WARNING %ld of %zu window offsets have fewer samples than "
                 "waveforms (peak too close to the readout window edge) — see \"n\" in the JSON\n",
                 shortOffsets, pulserkernel::kWinLen);
  std::printf("pulser_kernel: %ld events, %ld waveforms used, %ld saturated, %.1f s -> %s\n",
              r.events, r.all.waveforms, r.all.saturated_waveforms, sw.RealTime(), outJson);
  return 0;
}
