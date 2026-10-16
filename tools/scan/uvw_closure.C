// =============================================================================
// uvw_closure.C — U/V/W の閉合残差で 2D(ストリップ面)重心の精度を測る(TODO/148 §2)。146(z 残差)の 2D 版。
//
//   root -l -b -q 'uvw_closure.C("run0083_raw.root", "channel_map.csv", "run0083_tracks.root", "run0083_uvw_closure.json", "run0083_uvw_closure.png")'
//   root -l -b -q 'uvw_closure.C("run0083_raw.root", "channel_map.csv", "run0083_tracks.root", "out.json", nullptr, 200)'  (先頭 200 件)
//   root -l -b -q 'uvw_closure.C("run0083_raw.root", "channel_map.csv", "run0083_tracks.root", "out.json", nullptr, 0, false)'
//     (第 7 引数 hitChannelsOnly = false: 閾値越えのストリップを全部使う — 下の「ヒット ch」参照)
//   root -l -b -q 'uvw_closure.C("run0083_raw.root", "channel_map.csv", "run0083_tracks.root", "out.json", nullptr, 0, true, "ped.json")'
//     (第 8 引数 pedJson: ZS の raw 用。`calib pedestal` の JSON を run_metrics.C と同じ経路(loadPedTable)で読み、
//      ped = 表の mean、FPN の common mode は引かない(v = w − ped)。JSON に ped_source / ped_missing。TODO/153)
//
// 終了コード 0 = JSON(と PNG)を書いた、1 = 失敗(理由は stderr)。
//
// 原理: pitch_U + pitch_V + pitch_W = 0(macros/makeTracks.C:7-8 / :50: pitch_U = (−1, 0)、pitch_V = (0.5, +√3/2)、
//   pitch_W = (0.5, −√3/2))。同じ点 (x, y) のストリップ座標 s_v = pitch_v·(x, y) + o_v(o_v = channel_map の
//   view ごとの原点)の和は s_U + s_V + s_W = Σ o_v = c(点に依らない)。同じ時刻スライス(同じ z)の 3 面の
//   電荷重心は同じ (x, y) を指すので、閉合残差 r = s_U + s_V + s_W − c の広がりが飛跡モデルに依らない 2D 重心の
//   精度。3 面が独立に σ_s なら σ_r = √3·σ_s。pitch の値そのものは使わない(s は map の値をそのまま足す)。
//
// 入力:
//   - raw ツリー(SPEC §6.7、tools/root_sink/raw_tree.hpp): TTree `raw`、`eventId/i`、`adc[{4×nAsad}][68][512]/S`
//     (生 ADC、FPN 込み、未読 ch は全セル 0)。先頭次元 = AGET の数はリーフの要素数から(run_metrics.C と同じ)。
//   - channel_map.csv — 読み方は macros/analyzeUVW.C:97-118 と同じ: 空行と `#` で始まる行を飛ばし、1 行 =
//     `aget,raw_channel,view,strip,s_mm`(view 0/1/2 = U/V/W)。aget = adc の先頭の index(analyzeUVW の
//     adc[a][c][t]、:188-191)、s_mm = view 内のストリップ位置 [mm](analyzeUVW の chPos → slS、:116 / :210)。
//     analyzeUVW は読めない行を黙って飛ばし、範囲外の値は配列の外に書くが、ここでは黙らない(CLAUDE.md)—
//     読めない行、view ∉ {0,1,2}、raw_channel ∉ [0, 68)、aget < 0、FPN ch、(aget, raw_channel) の重複、
//     (view, strip) の重複、ストリップの無い view、raw に無い aget(≥ AGET の数)は失敗。
//   - tracks(makeTracks.C の `_tracks.root`): TTree `tracks` の `eventId`、`ok`。analyzeUVW / makeTracks は raw の
//     全エントリを同じ順に 1 つずつ書く(makeTracks.C:139 / :157 / :200 / :244)ので raw のエントリ i = tracks の
//     エントリ i。エントリ数と各 eventId を突き合わせ、違えば失敗(hit_residuals.C と同じ)。ok == 1 だけ使う。
//
// 手順(ok == 1 のイベント):
//   1. 前処理 = run_metrics.C と同じ: AGET ごとに読めた FPN ch のセル毎平均 fpn(t)(読めた FPN が無ければ 0 —
//      引かない。数は agets_without_fpn として WARNING)、map にあるストリップ ch ごとに
//      v(t) = ADC − fpn − ped、ped = (ADC − fpn) の cell 1..20 の平均。全セル 0 の ch(未読)は使わない。
//   2. 時刻スライス = cell [2, 508)(drift_end.C の範囲 — 先頭の整定と窓端 508..511 の跳ねを除く)を先頭から
//      sliceCells cell ずつ(入口では kSliceCells。端数の cell は使わない)。ストリップの重み
//      w = スライス内で v > 35 の cell の v の和(閾値は cell ごと。sliceCells = 1 なら w = v(t))。
//      w > 0 = 閾値越えのストリップ。
//   3. view ごとに閾値越えのストリップを strip 番号の順に見て、番号が 1 ずつ続く塊を数える(未読・閾値以下・
//      map に無い番号は隙間)。重心 s_v = Σ w·s / Σ w。
//   4. どれかの view に閾値越えがあるスライスだけを数える(何も無いスライスは数えない):
//      0 本の view がある → slices_missing_view、そうでなく塊が 2 つ以上の view がある → slices_multi_cluster
//      (どちらにも当たるときは view 不足として数える)、それ以外(3 面とも 1 塊)= 採用。
//      採用スライスの和 Σ = s_U + s_V + s_W、スライス電荷 Q = 3 面の Σ w、view ごとの本数。
// 指標(採用スライス):
//   c_mm               = Σ の中央値(幾何のオフセット Σ o_v)。r = Σ − c
//   residual_mm        = {n, rms = √(Σr²/n)(c 基準), robust_sigma = 1.4826 × MAD(MAD = |r − 中央値| の中央値)}
//   sigma_per_view_mm  = residual_mm.robust_sigma / √3(1 面の重心精度 σ_s)
//   charge_quartiles   = Q の分位 q25 / q50 / q75 で [min, q25] / (q25, q50] / (q50, q75] / (q75, max] ごとの r の
//                        {q_lo, q_hi, n, rms, robust_sigma, median}(hit_residuals.C の電荷 4 分位と同じ定義)
//   strips_median      = view ごとの塊の本数(重心に使った幅)の中央値
//   分位点はすべて線形補間 h = (n−1)·q(run_metrics.C の quantile)。値が無ければ JSON は null。
// ヒット ch(hitChannelsOnly、既定 true): イベントごとに run_metrics.C / drift_end.C と同じ「ヒット ch」条件 — その ch の
//   ピーク(全 cell の v の最大、最初の cell)> kAdcThreshold かつ q = ピーク ±kPeakHalfCells cell の v > kAdcThreshold の
//   和 ≥ kStripQMin(FPN + ped 減算後の v)— を満たす ch だけを重心の候補にする。満たさない ch は全 cell で無視。
//   halo(飛跡の時刻にほぼ全ストリップに乗る 35–200 ADC — macros/analyzeUVW.C:67-73)は q < 2000 で落ちる。
//   false のときは閾値 35(cell ごと)だけで選ぶ(以前の挙動。halo は slices_multi_cluster / strips_median に出る)。
//   channels_used_median = イベントあたりの候補 ch 数(ok == 1 のイベントのうち)の中央値。false のときの候補 = 読めた(全セル 0
//   でない)ストリップ ch。
//
// スライス t の cell 条件(hitChannelsOnly の true / false 共通): v(t) > kAdcThreshold かつ |t − tpeak_ch| ≤ kSliceWindowCells
//   (kSliceWindowCells = 15、tpeak_ch = その ch の FPN + ped 減算後の v の最大の cell)。別の z に本体がある ch の halo / 尾を外す。
// 設計上の限界: 1 ch に長い堆積(斜めの飛跡が同じ ch に数十 cell 載る)があると、その ch はピーク ±kSliceWindowCells の
//   スライスにしか寄与しない(精度の測定としては近傍の S/N の良いスライスに偏る)。
//
// PNG(任意): 左 = r のヒスト(範囲 ±5 robust σ)、右 = r vs スライス電荷。
// =============================================================================
#include "run_metrics.C"

#include <TCanvas.h>
#include <TH1.h>
#include <TH2.h>
#include <TROOT.h>
#include <TSystem.h>

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <fstream>
#include <set>
#include <sstream>
#include <string>
#include <utility>
#include <vector>

namespace uvwclosure {

using runmetrics::kAdcThreshold;
using runmetrics::kCells;
using runmetrics::kChannels;
using runmetrics::kFpn;
using runmetrics::kNull;
using runmetrics::kPedHi;
using runmetrics::kPedLo;
using runmetrics::kPeakHalfCells;
using runmetrics::kStripQMin;

// スライス幅 [cell](TODO/148 §2: 既定 1 cell、2–4 にまとめたいときはここを変える)。
const int kSliceCells = 1;
// スライス t で ch を使う条件 v(t) > kAdcThreshold かつ |t − tpeak_ch| ≤ kSliceWindowCells(tpeak_ch = その ch の v の最大の cell)。
// 別の z(別の時刻)に本体がある ch の halo / 尾(35–200 ADC)を、そのスライスの重心から外す。固有幅 ≈ 14.5 cell。
const int kSliceWindowCells = 15;
// スライスに使う cell の範囲 [kCellMin, kCellMax)(drift_end.C の kHitCellMin / kHitCellMax と同じ)。
const int kCellMin = 2;
const int kCellMax = runmetrics::kClipEdgeBackMinCell - 1;  // 508
const double kMadToSigma = 1.4826;
const char *const kViewName[3] = {"U", "V", "W"};

// channel_map.csv の 1 行。
struct Strip {
  int aget = 0, channel = 0, view = 0, strip = 0;
  double s = 0;
};

struct ChannelMap {
  std::vector<Strip> strips;
  std::vector<int> byView[3];  // strips の index を strip 番号の昇順に
};

struct Stats {
  long n = 0;
  double rms = kNull, robust_sigma = kNull, median = kNull;
};

struct Quartile {
  double q_lo = kNull, q_hi = kNull;
  Stats s;
};

// 採用スライス(PNG 用): r = Σ − c、q = スライス電荷。
struct Slice {
  double r, q;
};

struct Result {
  long run = -1;  // -1 = 名前から読めない(JSON は null)
  std::string raw_file, channel_map, tracks_file;
  int slice_cells = kSliceCells;
  long events = 0, events_ok = 0;
  // 信号 ch(map のストリップ)は読めたが FPN が 1 本も読めなかった (イベント, AGET) の数(run_metrics.C と同じ)
  long agets_without_fpn = 0;
  std::string ped_source = "cells_1_20";  // pedJson ありなら "table"
  long ped_missing = 0;  // pedJson ありで表に無かった (イベント, ストリップ ch) の数(その ch は飛ばした)
  long slices = 0, slices_missing_view = 0, slices_multi_cluster = 0;
  double c = kNull, sigma_per_view = kNull;
  Stats residual;
  Quartile quartile[4];
  double strips_median[3] = {kNull, kNull, kNull};
  bool hit_channels_only = true;
  double channels_used_median = kNull;  // イベントあたりの候補 ch 数の中央値
  std::vector<Slice> used;
};

// r は c 基準(rms は 0 基準)。中央値・MAD は r 自身の中央値まわり(hit_residuals.C の stats と同じ)。
Stats stats(const std::vector<double> &r)
{
  Stats s;
  s.n = static_cast<long>(r.size());
  if (r.empty()) return s;
  s.rms = runmetrics::rms(r);
  s.median = runmetrics::quantile(r, 0.5);
  std::vector<double> dev;
  dev.reserve(r.size());
  for (double x : r) dev.push_back(std::fabs(x - s.median));
  s.robust_sigma = kMadToSigma * runmetrics::quantile(dev, 0.5);
  return s;
}

// macros/analyzeUVW.C:102-118 と同じ読み方(冒頭)。失敗は理由つきで false。
bool readChannelMap(const std::string &path, ChannelMap &m, std::string &err)
{
  m = ChannelMap();
  std::ifstream in(path.c_str());
  if (!in) {
    err = "cannot open channel map " + path;
    return false;
  }
  std::set<std::pair<int, int>> channels, strips;
  int lineNo = 0;
  for (std::string line; std::getline(in, line);) {
    ++lineNo;
    if (line.find_first_not_of(" \t\r") == std::string::npos || line[0] == '#') continue;
    const std::string where = "channel map " + path + " line " + std::to_string(lineNo) + ": ";
    std::istringstream ss(line);
    Strip st;
    char c1 = 0, c2 = 0, c3 = 0, c4 = 0;
    if (!(ss >> st.aget >> c1 >> st.channel >> c2 >> st.view >> c3 >> st.strip >> c4 >> st.s) ||
        c1 != ',' || c2 != ',' || c3 != ',' || c4 != ',') {
      err = where + "cannot read 'aget,raw_channel,view,strip,s_mm' from '" + line + "'";
      return false;
    }
    if (st.view < 0 || st.view > 2) {
      err = where + "view " + std::to_string(st.view) + " (0/1/2 = U/V/W expected)";
      return false;
    }
    if (st.channel < 0 || st.channel >= kChannels) {
      err = where + "raw_channel " + std::to_string(st.channel) + " outside 0..67";
      return false;
    }
    if (st.aget < 0) {
      err = where + "aget " + std::to_string(st.aget) + " < 0";
      return false;
    }
    if (runmetrics::isFpn(st.channel)) {
      err = where + "raw_channel " + std::to_string(st.channel) + " is an FPN channel";
      return false;
    }
    if (!channels.insert({st.aget, st.channel}).second) {
      err = where + "duplicate (aget, raw_channel) = (" + std::to_string(st.aget) + ", " +
            std::to_string(st.channel) + ")";
      return false;
    }
    if (!strips.insert({st.view, st.strip}).second) {
      err = where + "duplicate (view, strip) = (" + std::to_string(st.view) + ", " +
            std::to_string(st.strip) + ")";
      return false;
    }
    m.byView[st.view].push_back(static_cast<int>(m.strips.size()));
    m.strips.push_back(st);
  }
  for (int v = 0; v < 3; ++v) {
    if (m.byView[v].empty()) {
      err = "channel map " + path + ": no strip for view " + kViewName[v];
      return false;
    }
    std::sort(m.byView[v].begin(), m.byView[v].end(),
              [&m](int a, int b) { return m.strips[a].strip < m.strips[b].strip; });
  }
  return true;
}

// run_metrics.C:439-451 / drift_end.C:158-167 と同じ「ヒット ch」: ピーク(最初の最大)> 閾値 かつ
// ピーク ±kPeakHalfCells の v > 閾値の和 ≥ kStripQMin。v = FPN + ped 減算後。
bool isHitChannel(const double *v)
{
  int peak = 0;
  for (int t = 0; t < kCells; ++t)
    if (v[t] > v[peak]) peak = t;
  if (v[peak] <= kAdcThreshold) return false;
  const int tLo = std::max(0, peak - kPeakHalfCells);
  const int tHi = std::min(kCells - 1, peak + kPeakHalfCells);
  double q = 0;
  for (int t = tLo; t <= tHi; ++t)
    if (v[t] > kAdcThreshold) q += v[t];
  return q >= kStripQMin;
}

bool computeClosure(const std::string &rawPath, const std::string &mapPath, const std::string &tracksPath,
                    int maxEvents, int sliceCells, Result &r, std::string &err, bool hitChannelsOnly,
                    const std::string &pedJson = "")
{
  r = Result();
  r.hit_channels_only = hitChannelsOnly;
  r.raw_file = runmetrics::resolvedPath(rawPath);
  r.channel_map = runmetrics::resolvedPath(mapPath);
  r.tracks_file = runmetrics::resolvedPath(tracksPath);
  r.run = runmetrics::runFromName(rawPath);
  r.slice_cells = sliceCells;
  if (sliceCells < 1) {
    err = "slice width " + std::to_string(sliceCells) + " cells (>= 1 expected)";
    return false;
  }

  ChannelMap map;
  if (!readChannelMap(mapPath, map, err)) return false;

  TFile fr(rawPath.c_str(), "READ");
  if (fr.IsZombie()) {
    err = "cannot open " + rawPath;
    return false;
  }
  TTree *tr = fr.Get<TTree>("raw");
  if (!tr) {
    err = "no TTree 'raw' in " + rawPath + " (SPEC 6.7 raw tree expected)";
    return false;
  }
  TLeaf *leaf = tr->GetLeaf("adc");
  if (!leaf || std::string(leaf->GetTypeName()) != "Short_t" ||
      leaf->GetLen() % (kChannels * kCells) != 0) {
    err = "raw: branch 'adc' is not adc[N][68][512]/S in " + rawPath;
    return false;
  }
  const int nBlocks = leaf->GetLen() / (kChannels * kCells);  // AGET の数(4 × AsAd 数)
  std::vector<std::vector<int>> stripsOfAget(static_cast<size_t>(nBlocks));
  for (size_t i = 0; i < map.strips.size(); ++i) {
    const int a = map.strips[i].aget;
    if (a >= nBlocks) {
      err = "channel map " + mapPath + " has aget " + std::to_string(a) + " but the raw tree has " +
            std::to_string(nBlocks) + " AGETs (adc[" + std::to_string(nBlocks) + "][68][512])";
      return false;
    }
    stripsOfAget[static_cast<size_t>(a)].push_back(static_cast<int>(i));
  }
  std::vector<double> pedTable;
  const bool useTable = !pedJson.empty();
  if (useTable) {
    if (!runmetrics::loadPedTable(pedJson, nBlocks, pedTable, err)) return false;
    r.ped_source = "table";
  }
  std::vector<Short_t> adc(static_cast<size_t>(leaf->GetLen()));
  UInt_t rawId = 0;
  if (tr->SetBranchAddress("adc", adc.data()) < 0 || tr->SetBranchAddress("eventId", &rawId) < 0) {
    err = "raw: cannot read branches 'adc' / 'eventId' in " + rawPath;
    return false;
  }

  TFile ft(tracksPath.c_str(), "READ");
  if (ft.IsZombie()) {
    err = "cannot open " + tracksPath;
    return false;
  }
  TTree *tt = ft.Get<TTree>("tracks");
  if (!tt) {
    err = "no TTree 'tracks' in " + tracksPath + " (expected makeTracks.C output)";
    return false;
  }
  if (tr->GetEntries() != tt->GetEntries()) {
    err = "raw has " + std::to_string(tr->GetEntries()) + " entries but tracks has " +
          std::to_string(tt->GetEntries()) + " (not the same run?)";
    return false;
  }
  UInt_t trkId = 0;
  int ok = 0;
  if (tt->SetBranchAddress("eventId", &trkId) < 0 || tt->SetBranchAddress("ok", &ok) < 0) {
    err = "tracks: cannot read branches 'eventId' / 'ok' in " + tracksPath;
    return false;
  }

  const size_t nStrips = map.strips.size();
  std::vector<double> v(nStrips * kCells);  // ストリップごとの v(t)
  std::vector<char> read(nStrips, 0);       // 読めたストリップ(全セル 0 でない)
  std::vector<int> peakT(nStrips, 0);      // ストリップごとの v が最大の cell(kSliceWindowCells 用)
  std::vector<char> usable(nStrips, 0);     // 重心の候補(hitChannelsOnly ならヒット ch、でなければ read)
  std::vector<double> channelCounts;        // ok イベントごとの候補 ch 数
  std::vector<double> fpn(kCells);
  std::vector<double> sums, charges, widths[3];

  Long64_t n = tr->GetEntries();
  if (maxEvents > 0) n = std::min<Long64_t>(n, maxEvents);
  for (Long64_t i = 0; i < n; ++i) {
    tr->GetEntry(i);
    tt->GetEntry(i);
    if (rawId != trkId) {
      err = "entry " + std::to_string(i) + ": raw eventId " + std::to_string(rawId) +
            " != tracks eventId " + std::to_string(trkId) + " (not the same run?)";
      return false;
    }
    if (ok != 1) continue;
    ++r.events_ok;

    // 1. 前処理(run_metrics.C と同じ)
    for (int b = 0; b < nBlocks; ++b) {
      if (stripsOfAget[static_cast<size_t>(b)].empty()) continue;
      const Short_t *aget = &adc[static_cast<size_t>(b) * kChannels * kCells];
      std::fill(fpn.begin(), fpn.end(), 0.0);
      int nFpn = 0;
      for (int fc : kFpn) {
        if (useTable) break;  // ZS では FPN も抑制される — common mode は引かない(run_metrics.C と同じ)
        const Short_t *w = aget + fc * kCells;
        if (runmetrics::allZero(w)) continue;
        ++nFpn;
        for (int t = 0; t < kCells; ++t) fpn[t] += w[t];
      }
      if (nFpn > 0)
        for (double &x : fpn) x /= nFpn;
      bool signalRead = false;
      for (int k : stripsOfAget[static_cast<size_t>(b)]) {
        const Short_t *w = aget + map.strips[static_cast<size_t>(k)].channel * kCells;
        read[static_cast<size_t>(k)] = !runmetrics::allZero(w);
        usable[static_cast<size_t>(k)] = 0;
        if (!read[static_cast<size_t>(k)]) continue;
        signalRead = true;
        double ped = 0;
        if (useTable) {
          ped = pedTable[static_cast<size_t>(b) * kChannels + map.strips[static_cast<size_t>(k)].channel];
          if (std::isnan(ped)) {  // 表に無い ch は飛ばして数える(黙って別の ped を使わない)
            ++r.ped_missing;
            read[static_cast<size_t>(k)] = 0;
            continue;
          }
        } else {
          for (int t = kPedLo; t <= kPedHi; ++t) ped += w[t] - fpn[t];
          ped /= (kPedHi - kPedLo + 1);
        }
        double *vk = &v[static_cast<size_t>(k) * kCells];
        for (int t = 0; t < kCells; ++t) vk[t] = w[t] - fpn[t] - ped;
        int pt = 0;
        for (int t = 1; t < kCells; ++t)
          if (vk[t] > vk[pt]) pt = t;
        peakT[static_cast<size_t>(k)] = pt;
        usable[static_cast<size_t>(k)] = hitChannelsOnly ? isHitChannel(vk) : 1;
      }
      if (signalRead && !useTable && nFpn == 0) ++r.agets_without_fpn;
    }

    long nUsable = 0;
    for (size_t k = 0; k < nStrips; ++k) nUsable += usable[k];
    channelCounts.push_back(static_cast<double>(nUsable));

    // 2.–4. スライスごと
    for (int t0 = kCellMin; t0 + sliceCells <= kCellMax; t0 += sliceCells) {
      double sw[3] = {0, 0, 0}, sws[3] = {0, 0, 0};
      int nStr[3] = {0, 0, 0}, nClu[3] = {0, 0, 0};
      for (int view = 0; view < 3; ++view) {
        int prev = 0;  // 直前の閾値越えストリップの番号(nStr > 0 のときだけ意味がある)
        for (int k : map.byView[view]) {
          if (!usable[static_cast<size_t>(k)]) continue;
          const double *vk = &v[static_cast<size_t>(k) * kCells];
          const int pt = peakT[static_cast<size_t>(k)];
          double w = 0;
          for (int t = t0; t < t0 + sliceCells; ++t)
            if (vk[t] > kAdcThreshold && std::abs(t - pt) <= kSliceWindowCells) w += vk[t];
          if (w <= 0) continue;
          const Strip &st = map.strips[static_cast<size_t>(k)];
          if (nStr[view] == 0 || st.strip != prev + 1) ++nClu[view];
          prev = st.strip;
          ++nStr[view];
          sw[view] += w;
          sws[view] += w * st.s;
        }
      }
      if (nStr[0] + nStr[1] + nStr[2] == 0) continue;
      if (nStr[0] == 0 || nStr[1] == 0 || nStr[2] == 0) {
        ++r.slices_missing_view;
        continue;
      }
      if (nClu[0] > 1 || nClu[1] > 1 || nClu[2] > 1) {
        ++r.slices_multi_cluster;
        continue;
      }
      sums.push_back(sws[0] / sw[0] + sws[1] / sw[1] + sws[2] / sw[2]);
      charges.push_back(sw[0] + sw[1] + sw[2]);
      for (int view = 0; view < 3; ++view) widths[view].push_back(nStr[view]);
    }
  }
  r.events = static_cast<long>(n);
  r.slices = static_cast<long>(sums.size());
  r.channels_used_median = runmetrics::quantile(channelCounts, 0.5);

  // 指標
  for (int view = 0; view < 3; ++view) r.strips_median[view] = runmetrics::quantile(widths[view], 0.5);
  if (sums.empty()) return true;
  r.c = runmetrics::quantile(sums, 0.5);
  std::vector<double> res;
  res.reserve(sums.size());
  for (size_t k = 0; k < sums.size(); ++k) {
    res.push_back(sums[k] - r.c);
    r.used.push_back({res.back(), charges[k]});
  }
  r.residual = stats(res);
  r.sigma_per_view = r.residual.robust_sigma / std::sqrt(3.0);
  const double edge[5] = {runmetrics::quantile(charges, 0.0), runmetrics::quantile(charges, 0.25),
                          runmetrics::quantile(charges, 0.5), runmetrics::quantile(charges, 0.75),
                          runmetrics::quantile(charges, 1.0)};
  std::vector<double> bin[4];
  for (const Slice &s : r.used) {
    int b = 3;
    for (int j = 0; j < 3; ++j)
      if (s.q <= edge[j + 1]) {
        b = j;
        break;
      }
    bin[b].push_back(s.r);
  }
  for (int b = 0; b < 4; ++b) {
    r.quartile[b].q_lo = edge[b];
    r.quartile[b].q_hi = edge[b + 1];
    r.quartile[b].s = stats(bin[b]);
  }
  return true;
}

// ---- JSON ------------------------------------------------------------------------------------
std::string toJson(const Result &r)
{
  using runmetrics::num;
  using runmetrics::str;
  std::string s = "{\n";
  s += "  \"run\": " + (r.run < 0 ? std::string("null") : std::to_string(r.run)) + ",\n";
  s += "  \"raw_file\": " + str(r.raw_file) + ",\n";
  s += "  \"channel_map\": " + str(r.channel_map) + ",\n";
  s += "  \"tracks_file\": " + str(r.tracks_file) + ",\n";
  s += "  \"events\": " + std::to_string(r.events) + ",\n";
  s += "  \"events_ok\": " + std::to_string(r.events_ok) + ",\n";
  s += "  \"ped_source\": " + str(r.ped_source) + ",\n";
  s += "  \"ped_missing\": " + std::to_string(r.ped_missing) + ",\n";
  s += "  \"slice_cells\": " + std::to_string(r.slice_cells) + ",\n";
  s += "  \"cell_range\": [" + std::to_string(kCellMin) + ", " + std::to_string(kCellMax) + "],\n";
  s += "  \"adc_threshold\": " + num(kAdcThreshold) + ",\n";
  s += "  \"slice_window_cells\": " + std::to_string(kSliceWindowCells) + ",\n";
  s += std::string("  \"hit_channels_only\": ") + (r.hit_channels_only ? "true" : "false") + ",\n";
  s += "  \"channels_used_median\": " + num(r.channels_used_median) + ",\n";
  s += "  \"slices\": " + std::to_string(r.slices) + ",\n";
  s += "  \"slices_missing_view\": " + std::to_string(r.slices_missing_view) + ",\n";
  s += "  \"slices_multi_cluster\": " + std::to_string(r.slices_multi_cluster) + ",\n";
  s += "  \"c_mm\": " + num(r.c) + ",\n";
  s += "  \"residual_mm\": {\"n\": " + std::to_string(r.residual.n) + ", \"rms\": " + num(r.residual.rms) +
       ", \"robust_sigma\": " + num(r.residual.robust_sigma) + "},\n";
  s += "  \"sigma_per_view_mm\": " + num(r.sigma_per_view) + ",\n";
  s += "  \"charge_quartiles\": [\n";
  for (int b = 0; b < 4; ++b) {
    const Quartile &q = r.quartile[b];
    s += "    {\"q_lo\": " + num(q.q_lo) + ", \"q_hi\": " + num(q.q_hi) + ", \"n\": " + std::to_string(q.s.n) +
         ", \"rms\": " + num(q.s.rms) + ", \"robust_sigma\": " + num(q.s.robust_sigma) +
         ", \"median\": " + num(q.s.median) + (b < 3 ? "},\n" : "}\n");
  }
  s += "  ],\n";
  s += "  \"strips_median\": {\"U\": " + num(r.strips_median[0]) + ", \"V\": " + num(r.strips_median[1]) +
       ", \"W\": " + num(r.strips_median[2]) + "}\n";
  s += "}\n";
  return s;
}

// ---- PNG -------------------------------------------------------------------------------------
// 左 = r のヒスト、右 = r vs スライス電荷。範囲 ±5 robust σ(無ければ ±5 mm)。
// 描いた物は kCanDelete でキャンバスに持たせ、gROOT の一覧には入れない(hit_residuals.C と同じ)。
bool drawPng(const Result &r, const std::string &path)
{
  gROOT->SetBatch(kTRUE);
  const double R = (std::isfinite(r.residual.robust_sigma) && r.residual.robust_sigma > 0)
                       ? 5 * r.residual.robust_sigma
                       : 5.0;
  double qMax = 1;
  for (const Slice &s : r.used) qMax = std::max(qMax, s.q);
  const std::string runTag = r.run < 0 ? std::string("?") : std::to_string(r.run);
  TCanvas c("uvw_closure", "uvw closure", 1400, 600);
  c.Divide(2, 1);

  c.cd(1);
  char title[256];
  std::snprintf(title, sizeof(title),
                "U+V+W closure (run %s, %ld slices, c = %.3f mm, robust #sigma %.3f mm);"
                "s_{U} + s_{V} + s_{W} - c [mm];slices",
                runTag.c_str(), r.slices, r.c, r.residual.robust_sigma);
  TH1D *h = new TH1D("uvw_closure_r", title, 100, -R, R);
  h->SetDirectory(nullptr);
  h->SetStats(kFALSE);
  h->SetLineWidth(2);
  h->SetLineColor(kBlue + 1);
  h->SetBit(kCanDelete);
  for (const Slice &s : r.used) h->Fill(s.r);
  h->Draw("HIST");

  c.cd(2);
  TH2D *hq = new TH2D("uvw_closure_r_vs_q",
                      "closure residual vs slice charge;slice charge (U+V+W) [ADC];"
                      "s_{U} + s_{V} + s_{W} - c [mm]",
                      100, 0, 1.05 * qMax, 100, -R, R);
  hq->SetDirectory(nullptr);
  hq->SetStats(kFALSE);
  hq->SetBit(kCanDelete);
  for (const Slice &s : r.used) hq->Fill(s.q, s.r);
  hq->Draw("COLZ");

  c.SaveAs(path.c_str());
  FileStat_t st;
  return gSystem->GetPathInfo(path.c_str(), st) == 0 && st.fSize > 0;
}

}  // namespace uvwclosure

// 入口。0 = JSON(と PNG)を書いた、1 = 失敗(理由は stderr)。maxEvents ≤ 0 = 全イベント。
int uvw_closure(const char *raw, const char *channelMap, const char *tracksRoot, const char *outJson,
                const char *outPng = nullptr, int maxEvents = 0, bool hitChannelsOnly = true,
                const char *pedJson = nullptr)
{
  TStopwatch sw;
  uvwclosure::Result r;
  std::string err;
  if (!uvwclosure::computeClosure(raw, channelMap, tracksRoot, maxEvents, uvwclosure::kSliceCells, r,
                                  err, hitChannelsOnly, pedJson ? pedJson : "")) {
    std::fprintf(stderr, "uvw_closure: ERROR %s\n", err.c_str());
    return 1;
  }
  const std::string json = uvwclosure::toJson(r);
  FILE *fp = std::fopen(outJson, "w");
  const bool written = fp && std::fputs(json.c_str(), fp) >= 0;
  const bool closed = fp && std::fclose(fp) == 0;
  if (!written || !closed) {
    std::fprintf(stderr, "uvw_closure: ERROR cannot write %s\n", outJson);
    return 1;
  }
  if (outPng && *outPng && !uvwclosure::drawPng(r, outPng)) {
    std::fprintf(stderr, "uvw_closure: ERROR cannot write %s\n", outPng);
    return 1;
  }
  // 黙らないこと(CLAUDE.md): 引けなかった common mode、使えるスライス無し
  if (r.agets_without_fpn > 0)
    std::fprintf(stderr,
                 "uvw_closure: WARNING %ld (event, AGET) had strip channels but no FPN channel "
                 "read — common mode not subtracted there (isFPNRead=false?)\n",
                 r.agets_without_fpn);
  if (r.ped_missing > 0)
    std::fprintf(stderr,
                 "uvw_closure: WARNING %ld (event, strip channel) not in the pedestal table — skipped\n",
                 r.ped_missing);
  if (r.slices == 0)
    std::fprintf(stderr,
                 "uvw_closure: WARNING no usable slice (%ld ok of %ld events; dropped %ld missing a view, "
                 "%ld with 2+ clusters) — metrics are null\n",
                 r.events_ok, r.events, r.slices_missing_view, r.slices_multi_cluster);
  std::printf(
      "uvw_closure: %ld events, %ld ok, %ld slices used (dropped: %ld missing a view, %ld with 2+ "
      "clusters), c %.4f mm, robust sigma %.4f mm -> per view %.4f mm, rms %.4f mm, strips U %.1f V %.1f "
      "W %.1f, %.1f s -> %s\n",
      r.events, r.events_ok, r.slices, r.slices_missing_view, r.slices_multi_cluster, r.c,
      r.residual.robust_sigma, r.sigma_per_view, r.residual.rms, r.strips_median[0], r.strips_median[1],
      r.strips_median[2], sw.RealTime(), outJson);
  return 0;
}
