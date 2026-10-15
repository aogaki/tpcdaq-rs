// =============================================================================
// run_metrics.C — run 1 本 → 指標 JSON(TODO/136)。パラメータスキャン(TODO/135)の各点を数字にする。
//
//   root -l -b -q 'run_metrics.C("run0077_raw.root", "run0077_metrics.json", 100)'
//
// ふつうは metrics_for_run.sh から呼ぶ(macros の analyzeUVW.C → makeTracks.C で
// `_tracks.root` を作ってから)。終了コード 0 = JSON を書いた、1 = 失敗(理由は stderr)。
//
// 入力:
//   - raw ツリー(SPEC §6.7、tools/root_sink/raw_tree.hpp): TTree `raw`、`eventTime/l`(CoBo の
//     48 bit タイムスタンプ、10 ns 刻み — src/decoder.rs RESTART_MIN_TIME_GAP の注記)、
//     `adc[{4×nAsad}][68][512]/S`(生 ADC、FPN 込み、未読 ch は全セル 0)。先頭次元は焼き込まず
//     リーフの要素数から求める(mini = 4、ELITPC 4 AsAd = 16)。
//   - 同じ名前の `_tracks.root`(raw の名前の `_raw.root` を `_tracks.root` に置き換えたもの —
//     macros の analyzeUVW.C / makeTracks.C の命名と同じ)。あれば読む、無ければ tracks = null。
//     TTree `tracks`(makeTracks.C): `ok`(int)、`length` / `charge` / `dEdx` / `dirX/Y/Z`(float)。
//   - pressureMbar: TODO/136 の呼び出し形のための引数で、ここでは使わない(z の換算 = 圧力は
//     analyzeUVW.C が `_uvw.root` を作るときに済んでいる — metrics_for_run.sh の --pressure)。
//
// raw からの指標(全イベント、macros 非依存)。ヒットの定義は TODO/136 §5(実データ run0076 で
// 全 ch の基線が窓の中で 20〜40 ADC 上がる common mode が出た)で macros / src/reco に寄せた:
//   1. AGET ごとに、読めた FPN ch(全セル 0 でない)のセル毎平均 fpn(t) を作る(読めた FPN が
//      無い AGET は fpn = 0 — 引かない。数は agets_without_fpn として WARNING に出す)
//   2. 各信号 ch(FPN 除く、全セル 0 = 部分読み出しの未読 ch は除く)について
//      v(t) = ADC(t) − fpn(t) − ped、ped = (ADC − fpn) の cell PED_LO..PED_HI の平均
//   3. ピーク = v の最大の最初の cell。v(ピーク) > ADC_THRESHOLD かつ
//      q = ピーク ±PEAK_HALF_CELLS の閾値超の v の和 ≥ STRIP_Q_MIN(halo cut)の ch = ヒット ch
//      (macros/analyzeUVW.C 0 / src/reco/mod.rs strip_hits と同じ判定。違いは 1 の FPN 減算だけ)
//   ヒットセル = ヒット ch の v > ADC_THRESHOLD のセル(窓全体)。
//   first/last_hit_cell     イベントのヒット ch のピーク cell の最小 / 最大。ヒットのあるイベント
//                           だけの分布の median / p10 / p90
//   clip_front/back_peak    first ≤ 5 / last ≥ 506 のイベント数 ÷ 全イベント数
//                           (ピーク ±5 の電荷窓が窓端に掛かる)
//   clip_front/back_edge    ヒットセルが cell ≤ 2 / ≥ 509 にあるイベント数 ÷ 全イベント数
//   saturated_sample_frac   ヒットセルのうち生 ADC == 4095 の割合(run 全体で数える)
//   saturated_event_frac    4095 のヒットセルを含むイベント数 ÷ 全イベント数
//   pulse_width_cells_median  ヒット ch ごとの「閾値超の連続セル数の最長」の中央値(全ヒット ch)
//   hit_channels_median     イベントごとのヒット ch 数の中央値(全イベント、0 を含む)
//   charge_median           イベントごとの Σ v(ヒットセル)の中央値(全イベント)
//   rate_hz                 (events − 1) / ((最後のエントリの eventTime − 最初の) × 10 ns)
// tracks からの指標(ok != 0 の飛跡だけ。n と ok_frac は全エントリ):
//   length / charge / dEdx の median と IQR(q75 − q25)。向きは makeTracks 6 の頭尻(dir の符号)が
//   コイン投げなので畳む: dip_abs_rms_deg = |atan2(dirZ, √(dirX² + dirY²))| の RMS、
//   phi_axial_rms_deg = atan2(dirY, dirX) を (−90, 90] に畳んだ軸角の RMS。RMS = √(Σx²/n) [deg]。
// ZS データ(TODO/149 §2): CoBo の zero suppression は閾値未満のセルを 0 にする(FPN も抑制される)ので、
// cells 1..20 の平均 ped も FPN の common mode も成り立たない。第 4 引数 pedJson(`calib pedestal` の
// JSON、channels[] {asad, aget, ch, mean, …})があれば: ped = 表の mean(ch ごと、block = asad*4 + aget)、
// FPN 共通モードは引かない、ヒット閾値は従来どおり 35 ADC。表に無い信号 ch は判定できないので飛ばし、
// 数を WARNING に出す(ped_missing)。pedJson 無しなら従来どおり。JSON の ped_source = "table" /
// "cells_1_20"。zs_suspected = イベントの過半数で「(読めた AGET の)信号 ch の過半数が cells 1..20
// 全 0」(ZS の疑い。部分読み出しの未読 ch も全 0 なので区別できない — 疑いに留め、pedJson 無しなら
// run_metrics() が WARNING を 1 回出す)。
// 分位点は線形補間(h = (n−1)·q、numpy の既定と同じ)。値が無ければ JSON は null。
// =============================================================================
#include <TFile.h>
#include <TLeaf.h>
#include <TStopwatch.h>
#include <TString.h>
#include <TTree.h>

#include <algorithm>
#include <climits>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <limits>
#include <string>
#include <vector>

namespace runmetrics {

// ---- 定数(出典つき)-------------------------------------------------------------------------
// src/reco/mod.rs 冒頭(= macros/analyzeUVW.C)の ADC_THRESHOLD / PED_LO / PED_HI と同じ値。
const double kAdcThreshold = 35.0;
const int kPedLo = 1;
const int kPedHi = 20;  // 両端を含む(PED_LO..=PED_HI、20 cells)
// src/reco/mod.rs PEAK_HALF_CELLS(= macros/analyzeUVW.C): ch の電荷 q はピーク ±5 cells の閾値超の和。
const int kPeakHalfCells = 5;
// src/reco/mod.rs STRIP_Q_MIN(= macros/analyzeUVW.C の halo cut)。macros・reco とも
// `q < STRIP_Q_MIN` を捨てる(2000 ちょうどは残す)ので同じ向きにする。
const double kStripQMin = 2000.0;
// src/geometry.rs FPN_RAW_CHANNELS(各 AGET の 68 ch 中の FPN)。
const int kFpn[4] = {11, 22, 45, 56};
// src/msg.rs ITEM_ADC_MAX(12 bit フルスケール)。
const int kAdcMax = 4095;
// SPEC §6.7 のリーフ `adc[..][68][512]` の内側 2 次元(AGET の ch 数とセル数 — 検出器で変わらない)。
const int kChannels = 68;
const int kCells = 512;
// 窓端クリップ(TODO/136 §5-3)。peak = ピーク cell ≤ 5 / ≥ 506(ピーク ±PEAK_HALF_CELLS の
// 電荷窓が cell 0 / 511 に掛かる)、edge = ヒットセルが cell ≤ 2 / ≥ 509(TODO/136 §1 の初版の定義)。
const int kClipPeakFrontMaxCell = kPeakHalfCells;              // 5
const int kClipPeakBackMinCell = kCells - 1 - kPeakHalfCells;  // 506
const int kClipEdgeFrontMaxCell = 2;
const int kClipEdgeBackMinCell = 509;
// CoBo の eventTime の刻み [s](10 ns — src/decoder.rs RESTART_MIN_TIME_GAP の注記)。
const double kTickSeconds = 1e-8;

const double kNull = std::numeric_limits<double>::quiet_NaN();

struct Quantiles {
  double median = kNull, p10 = kNull, p90 = kNull;
};
struct MedianIqr {
  double median = kNull, iqr = kNull;
};
struct TrackMetrics {
  bool present = false;
  long n = 0;
  double ok_frac = kNull;
  MedianIqr length, charge, dEdx;
  double dip_abs_rms_deg = kNull, phi_axial_rms_deg = kNull;
};
struct RunMetrics {
  long run = -1;  // -1 = 名前から読めない(JSON は null)
  std::string raw_file;
  long events = 0;
  double rate_hz = kNull;
  Quantiles first_hit_cell, last_hit_cell;
  double clip_front_peak = kNull, clip_front_edge = kNull;
  double clip_back_peak = kNull, clip_back_edge = kNull;
  double saturated_sample_frac = kNull, saturated_event_frac = kNull;
  double pulse_width_cells_median = kNull, hit_channels_median = kNull, charge_median = kNull;
  TrackMetrics tracks;
  // 信号 ch は読めたが FPN が 1 本も読めなかった (イベント, AGET) の数 = common mode を引けなかった
  // ところ(isFPNRead=false の設定)。JSON のキーは固定(TODO/136 §1)なので出さず、
  // run_metrics() が WARNING で出す。
  long agets_without_fpn = 0;
  // TODO/149: ped の出どころと ZS の疑い
  std::string ped_source = "cells_1_20";
  bool zs_suspected = false;
  long ped_missing = 0;  // pedJson ありで表に無かった (イベント, 信号 ch) の数(その ch は飛ばした)
};

// 線形補間の分位点(h = (n−1)·q)。空なら NaN。
double quantile(std::vector<double> v, double q)
{
  if (v.empty()) return kNull;
  std::sort(v.begin(), v.end());
  const double h = (v.size() - 1) * q;
  const size_t lo = static_cast<size_t>(std::floor(h));
  const size_t hi = std::min(lo + 1, v.size() - 1);
  return v[lo] + (h - lo) * (v[hi] - v[lo]);
}

Quantiles quantiles(const std::vector<double> &v)
{
  Quantiles q;
  q.median = quantile(v, 0.5);
  q.p10 = quantile(v, 0.1);
  q.p90 = quantile(v, 0.9);
  return q;
}

MedianIqr medianIqr(const std::vector<double> &v)
{
  MedianIqr m;
  m.median = quantile(v, 0.5);
  if (!v.empty()) m.iqr = quantile(v, 0.75) - quantile(v, 0.25);
  return m;
}

double rms(const std::vector<double> &v)
{
  if (v.empty()) return kNull;
  double s = 0;
  for (double x : v) s += x * x;
  return std::sqrt(s / v.size());
}

// φ [deg] を (−90, 90] に畳む。makeTracks 2 の phi = 0.5·atan2(…) と同じ値域(dir の符号 =
// 頭尻を決める 6 の前の軸)。φ と φ ± 180 は同じ軸。
double foldAxial(double phiDeg)
{
  if (phiDeg > 90) return phiDeg - 180;
  if (phiDeg <= -90) return phiDeg + 180;
  return phiDeg;
}

bool isFpn(int c)
{
  for (int f : kFpn)
    if (c == f) return true;
  return false;
}

// 部分読み出しで読まれなかった ch(全セル 0 — tools/root_sink/raw_tree.hpp fill_entry)。
bool allZero(const Short_t *w)
{
  for (int t = 0; t < kCells; ++t)
    if (w[t] != 0) return false;
  return true;
}

// 数値を 1 つ読む: obj の中の "key": <number>。無ければ false。
bool numAfterKey(const std::string &obj, const char *key, double &out)
{
  const std::string k = std::string("\"") + key + "\"";
  const size_t p = obj.find(k);
  if (p == std::string::npos) return false;
  const size_t colon = obj.find(':', p + k.size());
  if (colon == std::string::npos) return false;
  const char *start = obj.c_str() + colon + 1;
  char *end = nullptr;
  out = std::strtod(start, &end);
  return end != start;
}

// `calib pedestal` の JSON(src/calib/pedestal.rs: channels[] {asad, aget, ch, mean, rms, runs, is_fpn})
// → mean[block*kChannels + ch](block = asad*4 + aget、無い・範囲外は NaN)。channels の要素は
// 平らなオブジェクトなので '{'…'}' で切って読む。1 件も読めなければ false。
bool loadPedTable(const std::string &path, int nBlocks, std::vector<double> &mean, std::string &err)
{
  FILE *fp = std::fopen(path.c_str(), "rb");
  if (!fp) {
    err = "cannot open pedestal table " + path;
    return false;
  }
  std::string text;
  char buf[65536];
  size_t got;
  while ((got = std::fread(buf, 1, sizeof(buf), fp)) > 0) text.append(buf, got);
  std::fclose(fp);
  size_t pos = text.find("\"channels\"");
  if (pos == std::string::npos) {
    err = "pedestal table " + path + " has no \"channels\" array";
    return false;
  }
  mean.assign(static_cast<size_t>(nBlocks) * kChannels, kNull);
  long loaded = 0;
  while ((pos = text.find('{', pos)) != std::string::npos) {
    const size_t close = text.find('}', pos);
    if (close == std::string::npos) break;
    const std::string o = text.substr(pos, close - pos + 1);
    pos = close + 1;
    double asad, aget, ch, mu;
    if (!numAfterKey(o, "asad", asad) || !numAfterKey(o, "aget", aget) ||
        !numAfterKey(o, "ch", ch) || !numAfterKey(o, "mean", mu))
      continue;
    const long block = static_cast<long>(asad) * 4 + static_cast<long>(aget);
    if (block < 0 || block >= nBlocks || aget < 0 || aget > 3 || ch < 0 || ch >= kChannels) continue;
    mean[static_cast<size_t>(block) * kChannels + static_cast<size_t>(ch)] = mu;
    ++loaded;
  }
  if (loaded == 0) {
    err = "pedestal table " + path + ": no usable channels[] entries (asad/aget/ch/mean)";
    return false;
  }
  return true;
}

// pedJson 無しで ZS の疑いがあるときの WARNING 文(無ければ空)。run_metrics() が 1 回だけ出す。
std::string zsWarning(const RunMetrics &m, bool havePedTable)
{
  if (havePedTable || !m.zs_suspected) return "";
  return "run_metrics: WARNING zero-suppressed data suspected (most signal channels are all-zero in "
         "cells 1..20) but no pedestal table given — ped from cells 1..20 and the FPN common mode "
         "are wrong here; pass a `calib pedestal` JSON (metrics_for_run.sh --ped FILE)\n";
}

// "…/run0077_raw.root" → 77。形が違えば -1。
long runFromName(const std::string &path)
{
  const size_t slash = path.find_last_of('/');
  const std::string base = (slash == std::string::npos) ? path : path.substr(slash + 1);
  const std::string prefix = "run", suffix = "_raw.root";
  if (base.size() <= prefix.size() + suffix.size()) return -1;
  if (base.compare(0, prefix.size(), prefix) != 0) return -1;
  if (base.compare(base.size() - suffix.size(), suffix.size(), suffix) != 0) return -1;
  const std::string digits = base.substr(prefix.size(), base.size() - prefix.size() - suffix.size());
  for (char ch : digits)
    if (ch < '0' || ch > '9') return -1;
  return std::atol(digits.c_str());
}

// macros と同じ命名: `_raw.root` → `_tracks.root`(analyzeUVW.C → `_uvw.root` → makeTracks.C)。
std::string tracksPathFor(const std::string &rawPath)
{
  TString s(rawPath.c_str());
  s.ReplaceAll("_raw.root", "_tracks.root");
  return std::string(s.Data());
}

// シンボリックリンクを解いた絶対パス(metrics_for_run.sh は OUT_DIR のリンクを渡す — JSON には
// データ本体の場所を書く)。解けなければそのまま。
std::string resolvedPath(const std::string &path)
{
  char *p = realpath(path.c_str(), nullptr);
  if (!p) return path;
  const std::string s(p);
  std::free(p);
  return s;
}

bool fileExists(const std::string &path)
{
  FILE *fp = std::fopen(path.c_str(), "rb");
  if (!fp) return false;
  std::fclose(fp);
  return true;
}

bool computeTrackMetrics(const std::string &path, TrackMetrics &t, std::string &err)
{
  TFile f(path.c_str(), "READ");
  if (f.IsZombie()) {
    err = "cannot open " + path;
    return false;
  }
  TTree *tree = f.Get<TTree>("tracks");
  if (!tree) {
    err = "no TTree 'tracks' in " + path + " (expected makeTracks.C output)";
    return false;
  }
  int ok = 0;
  float length = 0, charge = 0, dEdx = 0, dirX = 0, dirY = 0, dirZ = 0;
  const char *names[7] = {"ok", "length", "charge", "dEdx", "dirX", "dirY", "dirZ"};
  void *addrs[7] = {&ok, &length, &charge, &dEdx, &dirX, &dirY, &dirZ};
  for (int i = 0; i < 7; ++i)
    if (tree->SetBranchAddress(names[i], addrs[i]) < 0) {
      err = std::string("tracks: cannot read branch '") + names[i] + "' in " + path;
      return false;
    }
  const double deg = 180.0 / M_PI;
  std::vector<double> vLength, vCharge, vDEdx, vDip, vPhi;
  const Long64_t n = tree->GetEntries();
  for (Long64_t i = 0; i < n; ++i) {
    tree->GetEntry(i);
    if (ok == 0) continue;
    vLength.push_back(length);
    vCharge.push_back(charge);
    vDEdx.push_back(dEdx);
    const double dx = dirX, dy = dirY, dz = dirZ;
    vDip.push_back(std::fabs(std::atan2(dz, std::sqrt(dx * dx + dy * dy)) * deg));
    vPhi.push_back(foldAxial(std::atan2(dy, dx) * deg));
  }
  t.present = true;
  t.n = static_cast<long>(n);
  if (n > 0) t.ok_frac = static_cast<double>(vLength.size()) / n;
  t.length = medianIqr(vLength);
  t.charge = medianIqr(vCharge);
  t.dEdx = medianIqr(vDEdx);
  t.dip_abs_rms_deg = rms(vDip);
  t.phi_axial_rms_deg = rms(vPhi);
  return true;
}

bool computeRunMetrics(const std::string &rawPath, RunMetrics &m, std::string &err,
                       const std::string &pedJson = "")
{
  m = RunMetrics();
  m.raw_file = resolvedPath(rawPath);
  m.run = runFromName(rawPath);

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
  ULong64_t eventTime = 0;
  if (tree->SetBranchAddress("adc", adc.data()) < 0 ||
      tree->SetBranchAddress("eventTime", &eventTime) < 0) {
    err = "raw: cannot read branches 'adc' / 'eventTime' in " + rawPath;
    return false;
  }

  std::vector<double> pedTable;
  const bool useTable = !pedJson.empty();
  if (useTable) {
    if (!loadPedTable(pedJson, nBlocks, pedTable, err)) return false;
    m.ped_source = "table";
  }
  long zsEvents = 0;

  const Long64_t n = tree->GetEntries();
  std::vector<double> firsts, lasts, widths, hitChannels, charges;
  std::vector<double> fpn(kCells), v(kCells);  // AGET 1 つ分の common mode、ch 1 本分の v
  long clipFrontPeak = 0, clipBackPeak = 0, clipFrontEdge = 0, clipBackEdge = 0, satEvents = 0;
  long long hitCells = 0, satCells = 0;
  ULong64_t tFirst = 0, tLast = 0;
  for (Long64_t i = 0; i < n; ++i) {
    tree->GetEntry(i);
    if (i == 0) tFirst = eventTime;
    tLast = eventTime;
    int first = INT_MAX, last = -1, nHit = 0;
    double charge = 0;
    bool saturated = false, edgeFront = false, edgeBack = false;
    long evSig = 0, evZero = 0;  // ZS の疑い: 読めた AGET の信号 ch 数と、cells 1..20 が全 0 の数
    for (int b = 0; b < nBlocks; ++b) {
      const Short_t *aget = &adc[static_cast<size_t>(b) * kChannels * kCells];
      // 1. common mode = 読めた FPN ch のセル毎平均(読めた FPN が無ければ 0 のまま)
      std::fill(fpn.begin(), fpn.end(), 0.0);
      int nFpn = 0;
      for (int fc : kFpn) {
        if (useTable) break;  // ZS では FPN も抑制される — common mode は引かない
        const Short_t *w = aget + fc * kCells;
        if (allZero(w)) continue;
        ++nFpn;
        for (int t = 0; t < kCells; ++t) fpn[t] += w[t];
      }
      if (nFpn > 0)
        for (double &x : fpn) x /= nFpn;
      bool signalRead = false;
      long blkSig = 0, blkZero = 0;
      for (int c = 0; c < kChannels; ++c) {
        if (isFpn(c)) continue;
        const Short_t *w = aget + c * kCells;
        ++blkSig;
        bool zero120 = true;
        for (int t = kPedLo; t <= kPedHi; ++t)
          if (w[t] != 0) zero120 = false;
        if (zero120) ++blkZero;
        if (allZero(w)) continue;
        signalRead = true;
        // 2. ped と v
        double ped = 0;
        if (useTable) {
          ped = pedTable[static_cast<size_t>(b) * kChannels + c];
          if (std::isnan(ped)) {
            ++m.ped_missing;
            continue;
          }
        } else {
          for (int t = kPedLo; t <= kPedHi; ++t) ped += w[t] - fpn[t];
          ped /= (kPedHi - kPedLo + 1);
        }
        int peak = 0;
        for (int t = 0; t < kCells; ++t) {
          v[t] = w[t] - fpn[t] - ped;
          if (v[t] > v[peak]) peak = t;
        }
        // 3. ピークが閾値超、ピーク ±5 の電荷 q ≥ STRIP_Q_MIN
        if (v[peak] <= kAdcThreshold) continue;
        const int tLo = std::max(0, peak - kPeakHalfCells);
        const int tHi = std::min(kCells - 1, peak + kPeakHalfCells);
        double q = 0;
        for (int t = tLo; t <= tHi; ++t)
          if (v[t] > kAdcThreshold) q += v[t];
        if (q < kStripQMin) continue;  // halo
        ++nHit;
        first = std::min(first, peak);
        last = std::max(last, peak);
        int runLen = 0, longest = 0;
        for (int t = 0; t < kCells; ++t) {
          if (v[t] <= kAdcThreshold) {
            runLen = 0;
            continue;
          }
          longest = std::max(longest, ++runLen);
          charge += v[t];
          ++hitCells;
          if (w[t] == kAdcMax) {
            ++satCells;
            saturated = true;
          }
          if (t <= kClipEdgeFrontMaxCell) edgeFront = true;
          if (t >= kClipEdgeBackMinCell) edgeBack = true;
        }
        widths.push_back(longest);
      }
      if (signalRead && !useTable && nFpn == 0) ++m.agets_without_fpn;
      if (signalRead) {
        evSig += blkSig;
        evZero += blkZero;
      }
    }
    if (evSig > 0 && 2 * evZero > evSig) ++zsEvents;
    hitChannels.push_back(nHit);
    charges.push_back(charge);
    if (saturated) ++satEvents;
    if (edgeFront) ++clipFrontEdge;
    if (edgeBack) ++clipBackEdge;
    if (nHit > 0) {
      firsts.push_back(first);
      lasts.push_back(last);
      if (first <= kClipPeakFrontMaxCell) ++clipFrontPeak;
      if (last >= kClipPeakBackMinCell) ++clipBackPeak;
    }
  }

  m.events = static_cast<long>(n);
  m.zs_suspected = n > 0 && 2 * zsEvents > n;
  if (n > 1 && tLast > tFirst) m.rate_hz = (n - 1) / ((tLast - tFirst) * kTickSeconds);
  m.first_hit_cell = quantiles(firsts);
  m.last_hit_cell = quantiles(lasts);
  if (n > 0) {
    m.clip_front_peak = static_cast<double>(clipFrontPeak) / n;
    m.clip_front_edge = static_cast<double>(clipFrontEdge) / n;
    m.clip_back_peak = static_cast<double>(clipBackPeak) / n;
    m.clip_back_edge = static_cast<double>(clipBackEdge) / n;
    m.saturated_event_frac = static_cast<double>(satEvents) / n;
  }
  if (hitCells > 0) m.saturated_sample_frac = static_cast<double>(satCells) / hitCells;
  m.pulse_width_cells_median = quantile(widths, 0.5);
  m.hit_channels_median = quantile(hitChannels, 0.5);
  m.charge_median = quantile(charges, 0.5);

  const std::string tracksPath = tracksPathFor(rawPath);
  if (tracksPath != rawPath && fileExists(tracksPath))
    return computeTrackMetrics(tracksPath, m.tracks, err);
  return true;
}

// ---- JSON ------------------------------------------------------------------------------------
std::string num(double x)
{
  if (!std::isfinite(x)) return "null";
  char buf[64];
  std::snprintf(buf, sizeof(buf), "%.10g", x);
  return buf;
}

std::string str(const std::string &s)
{
  std::string out = "\"";
  for (char ch : s) {
    if (ch == '"' || ch == '\\') {
      out += '\\';
      out += ch;
    } else if (static_cast<unsigned char>(ch) < 0x20) {
      char buf[8];
      std::snprintf(buf, sizeof(buf), "\\u%04x", ch);
      out += buf;
    } else {
      out += ch;
    }
  }
  return out + "\"";
}

std::string obj(const Quantiles &q)
{
  return "{\"median\": " + num(q.median) + ", \"p10\": " + num(q.p10) + ", \"p90\": " + num(q.p90) +
         "}";
}

std::string obj(const MedianIqr &q)
{
  return "{\"median\": " + num(q.median) + ", \"iqr\": " + num(q.iqr) + "}";
}

std::string toJson(const RunMetrics &m)
{
  std::string tracks = "null";
  if (m.tracks.present) {
    const TrackMetrics &t = m.tracks;
    tracks = "{\"n\": " + std::to_string(t.n) + ", \"ok_frac\": " + num(t.ok_frac) +
             ", \"length\": " + obj(t.length) + ", \"charge\": " + obj(t.charge) +
             ", \"dEdx\": " + obj(t.dEdx) + ", \"dip_abs_rms_deg\": " + num(t.dip_abs_rms_deg) +
             ", \"phi_axial_rms_deg\": " + num(t.phi_axial_rms_deg) + "}";
  }
  std::string s = "{\n";
  s += "  \"run\": " + (m.run < 0 ? std::string("null") : std::to_string(m.run)) + ",\n";
  s += "  \"raw_file\": " + str(m.raw_file) + ",\n";
  s += "  \"ped_source\": " + str(m.ped_source) + ",\n";
  s += std::string("  \"zs_suspected\": ") + (m.zs_suspected ? "true" : "false") + ",\n";
  s += "  \"events\": " + std::to_string(m.events) + ",\n";
  s += "  \"rate_hz\": " + num(m.rate_hz) + ",\n";
  s += "  \"first_hit_cell\": " + obj(m.first_hit_cell) + ",\n";
  s += "  \"last_hit_cell\": " + obj(m.last_hit_cell) + ",\n";
  s += "  \"clip_front_peak\": " + num(m.clip_front_peak) + ",\n";
  s += "  \"clip_front_edge\": " + num(m.clip_front_edge) + ",\n";
  s += "  \"clip_back_peak\": " + num(m.clip_back_peak) + ",\n";
  s += "  \"clip_back_edge\": " + num(m.clip_back_edge) + ",\n";
  s += "  \"saturated_sample_frac\": " + num(m.saturated_sample_frac) + ",\n";
  s += "  \"saturated_event_frac\": " + num(m.saturated_event_frac) + ",\n";
  s += "  \"pulse_width_cells_median\": " + num(m.pulse_width_cells_median) + ",\n";
  s += "  \"hit_channels_median\": " + num(m.hit_channels_median) + ",\n";
  s += "  \"charge_median\": " + num(m.charge_median) + ",\n";
  s += "  \"tracks\": " + tracks + "\n";
  s += "}\n";
  return s;
}

}  // namespace runmetrics

// 入口。0 = JSON を書いた、1 = 失敗(理由は stderr。JSON は書かない)。
int run_metrics(const char *rawFile, const char *outJson, double pressureMbar = 100.0,
                const char *pedJson = nullptr)
{
  TStopwatch sw;
  runmetrics::RunMetrics m;
  std::string err;
  if (!runmetrics::computeRunMetrics(rawFile, m, err, pedJson ? pedJson : "")) {
    std::fprintf(stderr, "run_metrics: ERROR %s\n", err.c_str());
    return 1;
  }
  const std::string json = runmetrics::toJson(m);
  FILE *fp = std::fopen(outJson, "w");
  const bool written = fp && std::fputs(json.c_str(), fp) >= 0;
  const bool closed = fp && std::fclose(fp) == 0;
  if (!written || !closed) {
    std::fprintf(stderr, "run_metrics: ERROR cannot write %s\n", outJson);
    return 1;
  }
  (void)pressureMbar;  // 呼び出し形のため(冒頭の注記)
  if (m.agets_without_fpn > 0)
    std::fprintf(stderr,
                 "run_metrics: WARNING %ld (event, AGET) had signal channels but no FPN channel "
                 "read — common mode not subtracted there (isFPNRead=false?)\n",
                 m.agets_without_fpn);
  if (m.ped_missing > 0)
    std::fprintf(stderr,
                 "run_metrics: WARNING %ld (event, channel) not in the pedestal table — skipped\n",
                 m.ped_missing);
  const std::string zsw = runmetrics::zsWarning(m, pedJson && *pedJson);
  if (!zsw.empty()) std::fputs(zsw.c_str(), stderr);
  std::printf("run_metrics: %ld events, tracks %s, %.1f s -> %s\n", m.events,
              m.tracks.present ? "read" : "absent (null)", sw.RealTime(), outJson);
  return 0;
}
