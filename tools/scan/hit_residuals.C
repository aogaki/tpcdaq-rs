// =============================================================================
// hit_residuals.C — ストリップヒットの z 残差(飛跡直線との差)の統計(TODO/146 §2)。
// σ_z(ヒット 1 本の z の決定精度)を peaking / HV ごとに実測し、「信号幅を飛跡の誤差に使うか」の判断材料にする。
//
//   root -l -b -q 'hit_residuals.C("run0083_uvw.root", "run0083_tracks.root", "run0083_hit_residuals.json", "run0083_hit_residuals.png")'
//   root -l -b -q 'hit_residuals.C("run0083_uvw.root", "run0083_tracks.root", "out.json", nullptr, 1000)'  (先頭 1000 件)
//
// 終了コード 0 = JSON(と PNG)を書いた、1 = 失敗(理由は stderr)。
//
// 入力 = ユーザー macros の出力(macros は読むだけ。幾何の式はそこから写した — 行番号は macros/ の版):
//   - `_uvw.root` の TTree `uvw`(analyzeUVW.C): `eventId`、`width[3]`、ストリップヒットの並列ベクトル
//     `slView`(0/1/2 = U/V/W)、`slZ`(z [mm])、`slS`(view 内のストリップ位置 [mm] = channel_map.csv の pos、
//     analyzeUVW.C:210)、`slQ`(電荷)
//   - `_tracks.root` の TTree `tracks`(makeTracks.C): `eventId`、`ok`、`xStart..zEnd`。1 エントリ = uvw の
//     1 エントリ(同じ順。eventId が食い違えば失敗)
//
// makeTracks.C の幾何(:7-8 / :50、:23、:128-142、:163-177、:187、:203-204、:223-230):
//   ストリップ座標 s = pitch_v · (x, y)、pitch_U = (−1, 0)、pitch_V = (0.5, +√3/2)、pitch_W = (0.5, −√3/2)
//   ms_v = view v の全ヒットの電荷加重平均の s(手順 1)
//   ヒットの l = (s − ms_v) / (pitch_v · d_xy)(手順 4。|pitch_v · d_xy| < MIN_PROJ = 0.3 の view は使わない)
//   z(l) = z0 + tanθ · l、xy(l) = m + l · d_xy、m = pitch_v · m = ms_v の最小二乗(手順 3)
//   start / end = l の 2–98 % 電荷分位 lo / hi での点(手順 5。頭尻の入れ替え = 手順 6)
// 手順(ok == 1 のイベントだけ):
//   1. ms_v と m を uvw のヒットから makeTracks と同じ式で作り直す(tracks には残っていない)
//   2. d_xy = (end − start)_xy の単位ベクトル。端点を l に戻す: l_S = (start_xy − m)·d_xy、l_E = l_S + |end − start|_xy、
//      tanθ = (z_E − z_S) / (l_E − l_S)、z0 = z_S − tanθ · l_S(d_xy の符号は頭尻で反転するが z(l) は変わらない)。
//      |end − start|_xy < 1e-6 mm は向きが決まらない → events_degenerate に数えて使わない(WARNING)
//   3. 各ヒット: |pitch_v · d_xy| < 0.3 → hits_cut_proj。l = (s − ms_v)/(pitch_v · d_xy) が [l_S, l_E] の外
//      (= makeTracks の 2–98 % 電荷分位の外。閉区間、float の丸めに 1e-3 mm の許容)→ hits_cut_ends。
//      残りの r = slZ − (z0 + tanθ · l)
//      注: makeTracks の quantileRange(:58-71)は両側とも累積を含めて数えるので、1 本の電荷が 2 % を超える
//      ふつうのイベントでは lo = l の最小(低い側は切らない)、hi = 最後から 2 番目(高い側の 1 本だけ切る)
// 指標(使ったヒット。全体・view ごと・電荷 4 分位ごと): n、RMS = √(Σr²/n)(0 基準 — 平均を引かない)、
//   robust σ = 1.4826 × MAD(MAD = |r − 中央値| の中央値)、中央値(バイアス)。
//   電荷 4 分位: 使ったヒットの slQ の分位 q25 / q50 / q75 で [min, q25] / (q25, q50] / (q50, q75] / (q75, max]。
//   width の中央値: uvw の width[3] を view ごとに、使ったイベント(ok == 1 かつ degenerate でない)で。
//   分位点はすべて線形補間 h = (n−1)·q(run_metrics.C の quantile)。値が無ければ JSON は null。
// 注意: 直線は makeTracks がそのヒットも含めて当てはめたもの(自己相関)なので σ は真の値より少し小さく出る。
//   ヒットが多い(≥ 20)ので補正は小さい。
//
// PNG(任意): 左 = 残差のヒスト(view 別に重ねる)、右 = 残差 vs 電荷。
// =============================================================================
#include "run_metrics.C"

#include <TCanvas.h>
#include <TH1.h>
#include <TH2.h>
#include <TLegend.h>
#include <TROOT.h>
#include <TSystem.h>

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

namespace hitres {

using runmetrics::kNull;

// makeTracks.C:50 / :54 と同じ値。
const double kPitch[3][2] = {{-1, 0}, {0.5, 0.8660254}, {0.5, -0.8660254}};
const double kMinProj = 0.3;
// 端の判定の許容 [mm](float の start / end / slS から l を作り直す丸め ≈ 1e-5 mm。ストリップ間隔 1.5 mm より十分小さい)
const double kLTolMm = 1e-3;
// start と end の xy 距離がこれ未満 = 向きが決まらない [mm]
const double kMinSegmentMm = 1e-6;
const double kMadToSigma = 1.4826;
const char *const kViewName[3] = {"U", "V", "W"};

struct Stats {
  long n = 0;
  double rms = kNull, robust_sigma = kNull, median = kNull;
};

struct Quartile {
  double q_lo = kNull, q_hi = kNull;
  Stats s;
};

struct Hit {
  int view;
  double q, r;
};

struct Result {
  long run = -1;  // -1 = 名前から読めない(JSON は null)
  std::string uvw_file, tracks_file;
  long events = 0, events_ok = 0, events_degenerate = 0;
  long hits = 0, hits_cut_ends = 0, hits_cut_proj = 0;
  Stats all, view[3];
  Quartile quartile[4];
  double width_median[3] = {kNull, kNull, kNull};
  std::vector<Hit> used;  // 使ったヒット(PNG 用)
};

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

// "…/run0083_uvw.root" → 83(run_metrics.C の runFromName を `_uvw.root` に読み替えて使う)。形が違えば -1。
long runFromUvwName(const std::string &path)
{
  TString s(path.c_str());
  s.ReplaceAll("_uvw.root", "_raw.root");
  return runmetrics::runFromName(s.Data());
}

bool computeHitResiduals(const std::string &uvwPath, const std::string &tracksPath, int maxEvents,
                         Result &r, std::string &err)
{
  r = Result();
  r.uvw_file = runmetrics::resolvedPath(uvwPath);
  r.tracks_file = runmetrics::resolvedPath(tracksPath);
  r.run = runFromUvwName(uvwPath);

  TFile fu(uvwPath.c_str(), "READ");
  if (fu.IsZombie()) {
    err = "cannot open " + uvwPath;
    return false;
  }
  TTree *tu = fu.Get<TTree>("uvw");
  if (!tu) {
    err = "no TTree 'uvw' in " + uvwPath + " (expected analyzeUVW.C output)";
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
  if (tu->GetEntries() != tt->GetEntries()) {
    err = "uvw has " + std::to_string(tu->GetEntries()) + " entries but tracks has " +
          std::to_string(tt->GetEntries()) + " (not the same run?)";
    return false;
  }

  UInt_t uId = 0, tId = 0;
  float width[3] = {0, 0, 0};
  std::vector<int> *slView = nullptr;
  std::vector<float> *slZ = nullptr, *slS = nullptr, *slQ = nullptr;
  const char *uNames[6] = {"eventId", "width", "slView", "slZ", "slS", "slQ"};
  void *uAddrs[6] = {&uId, width, &slView, &slZ, &slS, &slQ};
  for (int i = 0; i < 6; ++i)
    if (tu->SetBranchAddress(uNames[i], uAddrs[i]) < 0) {
      err = std::string("uvw: cannot read branch '") + uNames[i] + "' in " + uvwPath;
      return false;
    }
  int ok = 0;
  float xs = 0, ys = 0, zs = 0, xe = 0, ye = 0, ze = 0;
  const char *tNames[8] = {"eventId", "ok", "xStart", "yStart", "zStart", "xEnd", "yEnd", "zEnd"};
  void *tAddrs[8] = {&tId, &ok, &xs, &ys, &zs, &xe, &ye, &ze};
  for (int i = 0; i < 8; ++i)
    if (tt->SetBranchAddress(tNames[i], tAddrs[i]) < 0) {
      err = std::string("tracks: cannot read branch '") + tNames[i] + "' in " + tracksPath;
      return false;
    }

  Long64_t n = tu->GetEntries();
  if (maxEvents > 0) n = std::min<Long64_t>(n, maxEvents);
  std::vector<double> widths[3];
  for (Long64_t i = 0; i < n; ++i) {
    tu->GetEntry(i);
    tt->GetEntry(i);
    if (uId != tId) {
      err = "entry " + std::to_string(i) + ": uvw eventId " + std::to_string(uId) +
            " != tracks eventId " + std::to_string(tId) + " (not the same run?)";
      return false;
    }
    if (ok != 1) continue;
    ++r.events_ok;
    const size_t nk = slView->size();
    if (slZ->size() != nk || slS->size() != nk || slQ->size() != nk) {
      err = "event " + std::to_string(uId) + ": slView/slZ/slS/slQ have different lengths";
      return false;
    }

    // 1. ms_v(makeTracks.C:128-142)と anchor m(:163-177)
    double qv[3] = {0, 0, 0}, ms[3] = {0, 0, 0};
    for (size_t k = 0; k < nk; ++k) {
      const int v = slView->at(k);
      if (v < 0 || v > 2) {
        err = "event " + std::to_string(uId) + ": slView = " + std::to_string(v) + " (0/1/2 expected)";
        return false;
      }
      qv[v] += slQ->at(k);
      ms[v] += slQ->at(k) * slS->at(k);
    }
    for (int v = 0; v < 3; ++v) {
      if (qv[v] <= 0) {
        err = "event " + std::to_string(uId) + ": ok == 1 but no hit (charge) in view " +
              kViewName[v] + " (uvw and tracks not from the same run?)";
        return false;
      }
      ms[v] /= qv[v];
    }
    double A00 = 0, A01 = 0, A11 = 0, b0 = 0, b1 = 0;
    for (int v = 0; v < 3; ++v) {
      const double *p = kPitch[v];
      A00 += p[0] * p[0];
      A01 += p[0] * p[1];
      A11 += p[1] * p[1];
      b0 += p[0] * ms[v];
      b1 += p[1] * ms[v];
    }
    const double det = A00 * A11 - A01 * A01;
    const double m[2] = {(A11 * b0 - A01 * b1) / det, (-A01 * b0 + A00 * b1) / det};

    // 2. start → end の直線を makeTracks の l に戻す
    const double dx = xe - xs, dy = ye - ys;
    const double len = std::hypot(dx, dy);
    if (len < kMinSegmentMm) {
      ++r.events_degenerate;
      continue;
    }
    const double ux = dx / len, uy = dy / len;
    const double lS = (xs - m[0]) * ux + (ys - m[1]) * uy;
    const double lE = lS + len;
    const double tanTh = (ze - zs) / len;
    const double z0 = zs - tanTh * lS;
    for (int v = 0; v < 3; ++v) widths[v].push_back(width[v]);

    // 3. ヒットごとの l と残差
    double proj[3];
    for (int v = 0; v < 3; ++v) proj[v] = kPitch[v][0] * ux + kPitch[v][1] * uy;
    for (size_t k = 0; k < nk; ++k) {
      const int v = slView->at(k);
      if (std::fabs(proj[v]) < kMinProj) {
        ++r.hits_cut_proj;
        continue;
      }
      const double l = (slS->at(k) - ms[v]) / proj[v];
      if (l < lS - kLTolMm || l > lE + kLTolMm) {
        ++r.hits_cut_ends;
        continue;
      }
      r.used.push_back({v, slQ->at(k), slZ->at(k) - (z0 + tanTh * l)});
    }
  }
  r.events = static_cast<long>(n);
  r.hits = static_cast<long>(r.used.size());

  // 指標
  std::vector<double> all, perView[3], charges;
  for (const Hit &h : r.used) {
    all.push_back(h.r);
    perView[h.view].push_back(h.r);
    charges.push_back(h.q);
  }
  r.all = stats(all);
  for (int v = 0; v < 3; ++v) {
    r.view[v] = stats(perView[v]);
    r.width_median[v] = runmetrics::quantile(widths[v], 0.5);
  }
  if (!charges.empty()) {
    const double edge[5] = {runmetrics::quantile(charges, 0.0), runmetrics::quantile(charges, 0.25),
                            runmetrics::quantile(charges, 0.5), runmetrics::quantile(charges, 0.75),
                            runmetrics::quantile(charges, 1.0)};
    std::vector<double> bin[4];
    for (const Hit &h : r.used) {
      int b = 3;
      for (int j = 0; j < 3; ++j)
        if (h.q <= edge[j + 1]) {
          b = j;
          break;
        }
      bin[b].push_back(h.r);
    }
    for (int b = 0; b < 4; ++b) {
      r.quartile[b].q_lo = edge[b];
      r.quartile[b].q_hi = edge[b + 1];
      r.quartile[b].s = stats(bin[b]);
    }
  }
  return true;
}

// ---- JSON ------------------------------------------------------------------------------------
std::string obj(const Stats &s)
{
  using runmetrics::num;
  return "{\"n\": " + std::to_string(s.n) + ", \"rms\": " + num(s.rms) +
         ", \"robust_sigma\": " + num(s.robust_sigma) + ", \"median\": " + num(s.median) + "}";
}

std::string toJson(const Result &r)
{
  using runmetrics::num;
  std::string s = "{\n";
  s += "  \"run\": " + (r.run < 0 ? std::string("null") : std::to_string(r.run)) + ",\n";
  s += "  \"uvw_file\": " + runmetrics::str(r.uvw_file) + ",\n";
  s += "  \"tracks_file\": " + runmetrics::str(r.tracks_file) + ",\n";
  s += "  \"events\": " + std::to_string(r.events) + ",\n";
  s += "  \"events_ok\": " + std::to_string(r.events_ok) + ",\n";
  s += "  \"events_degenerate\": " + std::to_string(r.events_degenerate) + ",\n";
  s += "  \"hits\": " + std::to_string(r.hits) + ",\n";
  s += "  \"hits_cut_ends\": " + std::to_string(r.hits_cut_ends) + ",\n";
  s += "  \"hits_cut_proj\": " + std::to_string(r.hits_cut_proj) + ",\n";
  s += "  \"min_proj\": " + num(kMinProj) + ",\n";
  s += "  \"residual_mm\": {\n";
  s += "    \"all\": " + obj(r.all) + ",\n";
  for (int v = 0; v < 3; ++v)
    s += std::string("    \"") + kViewName[v] + "\": " + obj(r.view[v]) + (v < 2 ? ",\n" : "\n");
  s += "  },\n";
  s += "  \"charge_quartiles\": [\n";
  for (int b = 0; b < 4; ++b) {
    const Quartile &q = r.quartile[b];
    s += "    {\"q_lo\": " + num(q.q_lo) + ", \"q_hi\": " + num(q.q_hi) + ", " + obj(q.s).substr(1) +
         (b < 3 ? ",\n" : "\n");
  }
  s += "  ],\n";
  s += "  \"width_median_mm\": {\"U\": " + num(r.width_median[0]) + ", \"V\": " +
       num(r.width_median[1]) + ", \"W\": " + num(r.width_median[2]) + "}\n";
  s += "}\n";
  return s;
}

// ---- PNG -------------------------------------------------------------------------------------
// 左 = 残差のヒスト(view 別)、右 = 残差 vs 電荷。範囲 ±5 robust σ(全体。無ければ ±5 mm)。
// 描いた物は kCanDelete でキャンバスに持たせ、gROOT の一覧には入れない(drift_end.C と同じ)。
bool drawPng(const Result &r, const std::string &path)
{
  gROOT->SetBatch(kTRUE);
  const double R = (std::isfinite(r.all.robust_sigma) && r.all.robust_sigma > 0)
                       ? 5 * r.all.robust_sigma
                       : 5.0;
  double qMax = 1;
  for (const Hit &h : r.used) qMax = std::max(qMax, h.q);
  const std::string runTag = r.run < 0 ? std::string("?") : std::to_string(r.run);
  TCanvas c("hit_residuals", "hit residuals", 1400, 600);
  c.Divide(2, 1);

  c.cd(1);
  const Color_t color[3] = {kBlue + 1, kRed + 1, kGreen + 2};
  TH1D *h[3];
  for (int v = 0; v < 3; ++v) {
    const std::string title = "z residual (run " + runTag + ", " + std::to_string(r.hits) +
                              " hits);slZ - z(l) [mm];hits";
    h[v] = new TH1D((std::string("hit_residuals_") + kViewName[v]).c_str(), title.c_str(), 100, -R, R);
    h[v]->SetDirectory(nullptr);
    h[v]->SetStats(kFALSE);
    h[v]->SetLineWidth(2);
    h[v]->SetLineColor(color[v]);
    h[v]->SetBit(kCanDelete);
  }
  for (const Hit &x : r.used) h[x.view]->Fill(x.r);
  double yMax = 1;
  for (int v = 0; v < 3; ++v) yMax = std::max(yMax, h[v]->GetMaximum());
  h[0]->SetMaximum(1.1 * yMax);
  h[0]->Draw("HIST");
  h[1]->Draw("HIST SAME");
  h[2]->Draw("HIST SAME");
  TLegend *legend = new TLegend(0.62, 0.70, 0.89, 0.89);
  legend->SetBit(kCanDelete);
  char buf[128];
  for (int v = 0; v < 3; ++v) {
    std::snprintf(buf, sizeof(buf), "%s  robust #sigma %.3f mm", kViewName[v], r.view[v].robust_sigma);
    legend->AddEntry(h[v], buf, "l");
  }
  legend->Draw();

  c.cd(2);
  TH2D *hq = new TH2D("hit_residuals_vs_q", "z residual vs strip charge;slQ [ADC];slZ - z(l) [mm]",
                      100, 0, 1.05 * qMax, 100, -R, R);
  hq->SetDirectory(nullptr);
  hq->SetStats(kFALSE);
  hq->SetBit(kCanDelete);
  for (const Hit &x : r.used) hq->Fill(x.q, x.r);
  hq->Draw("COLZ");

  c.SaveAs(path.c_str());
  FileStat_t st;
  return gSystem->GetPathInfo(path.c_str(), st) == 0 && st.fSize > 0;
}

}  // namespace hitres

// 入口。0 = JSON(と PNG)を書いた、1 = 失敗(理由は stderr)。maxEvents ≤ 0 = 全イベント。
int hit_residuals(const char *uvwRoot, const char *tracksRoot, const char *outJson,
                  const char *outPng = nullptr, int maxEvents = 0)
{
  TStopwatch sw;
  hitres::Result r;
  std::string err;
  if (!hitres::computeHitResiduals(uvwRoot, tracksRoot, maxEvents, r, err)) {
    std::fprintf(stderr, "hit_residuals: ERROR %s\n", err.c_str());
    return 1;
  }
  const std::string json = hitres::toJson(r);
  FILE *fp = std::fopen(outJson, "w");
  const bool written = fp && std::fputs(json.c_str(), fp) >= 0;
  const bool closed = fp && std::fclose(fp) == 0;
  if (!written || !closed) {
    std::fprintf(stderr, "hit_residuals: ERROR cannot write %s\n", outJson);
    return 1;
  }
  if (outPng && *outPng && !hitres::drawPng(r, outPng)) {
    std::fprintf(stderr, "hit_residuals: ERROR cannot write %s\n", outPng);
    return 1;
  }
  // 黙らないこと(CLAUDE.md): 使えなかったイベント、ヒット無し
  if (r.events_degenerate > 0)
    std::fprintf(stderr,
                 "hit_residuals: WARNING %ld ok track(s) with start == end (xy) — direction "
                 "undefined, not used\n",
                 r.events_degenerate);
  if (r.hits == 0)
    std::fprintf(stderr, "hit_residuals: WARNING no hit used (%ld ok of %ld events) — metrics are null\n",
                 r.events_ok, r.events);
  std::printf(
      "hit_residuals: %ld events, %ld ok, %ld hits used (cut: %ld at track ends, %ld in views with "
      "|proj| < %.1f), robust sigma %.4f mm (U %.4f V %.4f W %.4f), median %.4f mm, %.1f s -> %s\n",
      r.events, r.events_ok, r.hits, r.hits_cut_ends, r.hits_cut_proj, hitres::kMinProj,
      r.all.robust_sigma, r.view[0].robust_sigma, r.view[1].robust_sigma, r.view[2].robust_sigma,
      r.all.median, sw.RealTime(), outJson);
  return 0;
}
