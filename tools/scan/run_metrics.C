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
//     `adc[{4×nAsad}][68][512]/S`(生 ADC、FPN 込み、未読 ch は 0)。先頭次元は焼き込まず
//     リーフの要素数から求める(mini = 4、ELITPC 4 AsAd = 16)。
//   - 同じ名前の `_tracks.root`(raw の名前の `_raw.root` を `_tracks.root` に置き換えたもの —
//     macros の analyzeUVW.C / makeTracks.C の命名と同じ)。あれば読む、無ければ tracks = null。
//     TTree `tracks`(makeTracks.C): `ok`(int)、`length` / `charge` / `dEdx` / `dirX/Y/Z`(float)。
//   - pressureMbar: TODO/136 の呼び出し形のための引数で、ここでは使わない(z の換算 = 圧力は
//     analyzeUVW.C が `_uvw.root` を作るときに済んでいる — metrics_for_run.sh の --pressure)。
//
// raw からの指標(全イベント、macros 非依存)。1 イベントの各信号 ch(FPN 除く)について
//   ペデスタル = cell PED_LO..PED_HI の平均、ヒットセル = ADC − ped > ADC_THRESHOLD のセル、
//   ヒット ch = ヒットセルが 1 つでもある ch:
//   first/last_hit_cell     イベントのヒットセルの最小 / 最大(全 ch)。ヒットのあるイベントだけの
//                           分布の median / p10 / p90
//   clip_front / clip_back  first ≤ 2 / last ≥ 509 のイベント数 ÷ 全イベント数
//   saturated_sample_frac   ヒットセルのうち ADC == 4095 の割合(run 全体で数える)
//   saturated_event_frac    4095 のヒットセルを含むイベント数 ÷ 全イベント数
//   pulse_width_cells_median  ヒット ch ごとの「閾値超の連続セル数の最長」の中央値(全ヒット ch)
//   hit_channels_median     イベントごとのヒット ch 数の中央値(全イベント、0 を含む)
//   charge_median           イベントごとの Σ(ADC − ped)(ヒットセル)の中央値(全イベント)
//   rate_hz                 (events − 1) / ((最後のエントリの eventTime − 最初の) × 10 ns)
// tracks からの指標(ok != 0 の飛跡だけ。n と ok_frac は全エントリ):
//   length / charge / dEdx の median と IQR(q75 − q25)、
//   dip = atan2(dirZ, √(dirX² + dirY²))、φ = atan2(dirY, dirX)[deg] の RMS = √(Σx²/n)。
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
// src/geometry.rs FPN_RAW_CHANNELS(各 AGET の 68 ch 中の FPN)。
const int kFpn[4] = {11, 22, 45, 56};
// src/msg.rs ITEM_ADC_MAX(12 bit フルスケール)。
const int kAdcMax = 4095;
// SPEC §6.7 のリーフ `adc[..][68][512]` の内側 2 次元(AGET の ch 数とセル数 — 検出器で変わらない)。
const int kChannels = 68;
const int kCells = 512;
// 窓端クリップの判定(TODO/136: 最初のヒットが cell ≤ 2、最後が ≥ 509)。
const int kClipFrontMaxCell = 2;
const int kClipBackMinCell = 509;
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
  double dip_rms_deg = kNull, phi_rms_deg = kNull;
};
struct RunMetrics {
  long run = -1;  // -1 = 名前から読めない(JSON は null)
  std::string raw_file;
  long events = 0;
  double rate_hz = kNull;
  Quantiles first_hit_cell, last_hit_cell;
  double clip_front = kNull, clip_back = kNull;
  double saturated_sample_frac = kNull, saturated_event_frac = kNull;
  double pulse_width_cells_median = kNull, hit_channels_median = kNull, charge_median = kNull;
  TrackMetrics tracks;
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

bool isFpn(int c)
{
  for (int f : kFpn)
    if (c == f) return true;
  return false;
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
    vDip.push_back(std::atan2(dz, std::sqrt(dx * dx + dy * dy)) * deg);
    vPhi.push_back(std::atan2(dy, dx) * deg);
  }
  t.present = true;
  t.n = static_cast<long>(n);
  if (n > 0) t.ok_frac = static_cast<double>(vLength.size()) / n;
  t.length = medianIqr(vLength);
  t.charge = medianIqr(vCharge);
  t.dEdx = medianIqr(vDEdx);
  t.dip_rms_deg = rms(vDip);
  t.phi_rms_deg = rms(vPhi);
  return true;
}

bool computeRunMetrics(const std::string &rawPath, RunMetrics &m, std::string &err)
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

  const Long64_t n = tree->GetEntries();
  std::vector<double> firsts, lasts, widths, hitChannels, charges;
  long clipFront = 0, clipBack = 0, satEvents = 0;
  long long hitCells = 0, satCells = 0;
  ULong64_t tFirst = 0, tLast = 0;
  for (Long64_t i = 0; i < n; ++i) {
    tree->GetEntry(i);
    if (i == 0) tFirst = eventTime;
    tLast = eventTime;
    int first = INT_MAX, last = -1, nHit = 0;
    double charge = 0;
    bool saturated = false;
    for (int b = 0; b < nBlocks; ++b) {
      for (int c = 0; c < kChannels; ++c) {
        if (isFpn(c)) continue;
        const Short_t *w = &adc[(static_cast<size_t>(b) * kChannels + c) * kCells];
        double ped = 0;
        for (int t = kPedLo; t <= kPedHi; ++t) ped += w[t];
        ped /= (kPedHi - kPedLo + 1);
        int chFirst = -1, chLast = -1, runLen = 0, longest = 0;
        for (int t = 0; t < kCells; ++t) {
          const double v = w[t] - ped;
          if (v <= kAdcThreshold) {
            runLen = 0;
            continue;
          }
          if (chFirst < 0) chFirst = t;
          chLast = t;
          longest = std::max(longest, ++runLen);
          charge += v;
          ++hitCells;
          if (w[t] == kAdcMax) {
            ++satCells;
            saturated = true;
          }
        }
        if (chFirst < 0) continue;
        ++nHit;
        widths.push_back(longest);
        first = std::min(first, chFirst);
        last = std::max(last, chLast);
      }
    }
    hitChannels.push_back(nHit);
    charges.push_back(charge);
    if (saturated) ++satEvents;
    if (nHit > 0) {
      firsts.push_back(first);
      lasts.push_back(last);
      if (first <= kClipFrontMaxCell) ++clipFront;
      if (last >= kClipBackMinCell) ++clipBack;
    }
  }

  m.events = static_cast<long>(n);
  if (n > 1 && tLast > tFirst) m.rate_hz = (n - 1) / ((tLast - tFirst) * kTickSeconds);
  m.first_hit_cell = quantiles(firsts);
  m.last_hit_cell = quantiles(lasts);
  if (n > 0) {
    m.clip_front = static_cast<double>(clipFront) / n;
    m.clip_back = static_cast<double>(clipBack) / n;
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
             ", \"dEdx\": " + obj(t.dEdx) + ", \"dip_rms_deg\": " + num(t.dip_rms_deg) +
             ", \"phi_rms_deg\": " + num(t.phi_rms_deg) + "}";
  }
  std::string s = "{\n";
  s += "  \"run\": " + (m.run < 0 ? std::string("null") : std::to_string(m.run)) + ",\n";
  s += "  \"raw_file\": " + str(m.raw_file) + ",\n";
  s += "  \"events\": " + std::to_string(m.events) + ",\n";
  s += "  \"rate_hz\": " + num(m.rate_hz) + ",\n";
  s += "  \"first_hit_cell\": " + obj(m.first_hit_cell) + ",\n";
  s += "  \"last_hit_cell\": " + obj(m.last_hit_cell) + ",\n";
  s += "  \"clip_front\": " + num(m.clip_front) + ",\n";
  s += "  \"clip_back\": " + num(m.clip_back) + ",\n";
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
int run_metrics(const char *rawFile, const char *outJson, double pressureMbar = 100.0)
{
  TStopwatch sw;
  runmetrics::RunMetrics m;
  std::string err;
  if (!runmetrics::computeRunMetrics(rawFile, m, err)) {
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
  std::printf("run_metrics: %ld events, tracks %s, %.1f s -> %s\n", m.events,
              m.tracks.present ? "read" : "absent (null)", sw.RealTime(), outJson);
  return 0;
}
