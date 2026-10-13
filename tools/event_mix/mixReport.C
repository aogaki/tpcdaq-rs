// mixReport.C — 重ね合わせイベントの単一飛跡フィットが「元の 2 本のどちらに寄るか」の記録(TODO/128)。
//   root -l -b -q 'mixReport.C("mix_raw.root", "src_raw_tracks.root", "mix_raw_tracks.root")'
//     mix_raw.root          event_mix の出力(mix ツリーの srcEntry を使う)
//     src_raw_tracks.root   元データに macros(analyzeUVW → makeTracks)を掛けた tracks
//     mix_raw_tracks.root   mix_raw.root に同じ macros を掛けた tracks
// 向きは xy 平面の直線の角度 phi = atan2(dirY, dirX) を 180 度で畳んだもの(頭尻は決まらない)。
// 出力は stdout のみ(実データ由来の数値 — リポには記録を残さず、結果は TODO の結果節に要約する)。
#include <TFile.h>
#include <TTree.h>

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <vector>

namespace {
// 直線の向きの差 [0, 90] 度
double lineDiff(double p, double q) {
  double d = std::fabs(p - q);
  d = std::fmod(d, 180.0);
  return d > 90.0 ? 180.0 - d : d;
}
double phiLine(double dx, double dy) {
  double p = std::atan2(dy, dx) * 180.0 / M_PI;
  p = std::fmod(p + 360.0, 180.0);
  return p;
}
double median(std::vector<double> v) {
  if (v.empty()) return 0;
  std::sort(v.begin(), v.end());
  return v[v.size() / 2];
}
}  // namespace

void mixReport(const char* mixFile, const char* srcTracks, const char* mixTracks) {
  TFile fm(mixFile), fs(srcTracks), ft(mixTracks);
  auto* mix = (TTree*)fm.Get("mix");
  auto* st = (TTree*)fs.Get("tracks");
  auto* mt = (TTree*)ft.Get("tracks");
  if (!mix || !st || !mt) {
    printf("ERROR: missing tree\n");
    return;
  }
  // 元データの飛跡を entry 番号で引く
  std::vector<int> sOk(st->GetEntries());
  std::vector<double> sPhi(st->GetEntries());
  {
    Int_t ok;
    Float_t dx, dy;
    st->SetBranchAddress("ok", &ok);
    st->SetBranchAddress("dirX", &dx);
    st->SetBranchAddress("dirY", &dy);
    for (Long64_t i = 0; i < st->GetEntries(); ++i) {
      st->GetEntry(i);
      sOk[i] = ok;
      sPhi[i] = phiLine(dx, dy);
    }
  }
  Int_t nSrc, srcEntry[8];
  mix->SetBranchAddress("nSrc", &nSrc);
  mix->SetBranchAddress("srcEntry", srcEntry);
  Int_t ok;
  Float_t dx, dy;
  mt->SetBranchAddress("ok", &ok);
  mt->SetBranchAddress("dirX", &dx);
  mt->SetBranchAddress("dirY", &dy);

  const Long64_t n = mix->GetEntries();
  int nOk = 0, nBothSrcOk = 0, nNear = 0, nBetween = 0, nOther = 0;
  std::vector<double> sep, dMin, dMax, frac;
  for (Long64_t i = 0; i < n; ++i) {
    mix->GetEntry(i);
    mt->GetEntry(i);
    if (ok) ++nOk;
    if (!ok || nSrc != 2 || !sOk[srcEntry[0]] || !sOk[srcEntry[1]]) continue;
    ++nBothSrcOk;
    const double pa = sPhi[srcEntry[0]], pb = sPhi[srcEntry[1]], pf = phiLine(dx, dy);
    const double dab = lineDiff(pa, pb), da = lineDiff(pf, pa), db = lineDiff(pf, pb);
    sep.push_back(dab);
    dMin.push_back(std::min(da, db));
    dMax.push_back(std::max(da, db));
    // 「間」= 2 本のなす小さい方の弧の中(da + db が dab に一致、1 度の余裕)
    const bool between = std::fabs(da + db - dab) < 1.0;
    if (std::min(da, db) <= 5.0) ++nNear;
    else if (between) ++nBetween;
    else ++nOther;
    if (dab > 10.0) frac.push_back(std::min(da, db) / dab);
  }
  printf("pairs                      : %lld\n", n);
  printf("mixed fit ok               : %d\n", nOk);
  printf("ok and both sources ok     : %d\n", nBothSrcOk);
  printf("source-source angle  median: %.1f deg (deg of lines, 0..90)\n", median(sep));
  printf("fit to nearer source median: %.1f deg\n", median(dMin));
  printf("fit to farther source  med.: %.1f deg\n", median(dMax));
  printf("fit within 5 deg of a source: %d\n", nNear);
  printf("fit between (not near)     : %d\n", nBetween);
  printf("fit outside both           : %d\n", nOther);
  printf("pairs with separation >10 deg: %zu, median nearer/separation = %.2f\n", frac.size(), median(frac));
}
