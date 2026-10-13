// geometry_xy_oracle — TPCReco GeometryTPC を正として strip 線分の mm と MatchCrossPoint の結果を
// JSON に書き出す(TODO/125 レーン A-4)。Rust の src/strip_xy.rs(線分)と UI の
// xy-projection.ts(MatchCrossPoint、レーン C)の照合元。
//
//   geometry_xy_oracle GEOMETRY.dat SEGMENTS.json [MATCH_CASES.json]
//
// TPCReco のソースはリポに入れない — Makefile が TPCDAQ_TPCRECO_DIR から直コンパイルする。
//
// SEGMENTS.json:
//   {"padSize", "padPitch", "stripPitch", "referencePoint":[x,y],
//    "planes":{"U":{"unit":[x,y],"pitchUnit":[x,y]}, "V":..., "W":...},
//    "rangeXY":[xmin,xmax,ymin,ymax],          // GeometryTPC::rangeXY() = TH2Poly の軸(Get2D の
//                                               //  ビン範囲)。SetFloat(true) で菱形 pad の頂点まで広がる
//    "rangeXYCentreLines":[xmin,xmax,ymin,ymax], // 参考: 全線分の Start()/End() の外接矩形
//                                               //  (InitTH2Poly が TH2Poly を作るときの初期値)
//    "segments":[[plane, section, strip, x0, y0, ux, uy, length], ...]}
//   segments は mapByStrip の順(plane → section → strip)。x0,y0 = reference point + Offset()、
//   length = StripTPC::Length()(= pad_pitch × int(LEN_PADS))。
//
// MATCH_CASES.json:
//   {"radius":2.0, "numericalTolerance":1e-5,
//    "columns":["set","uStrip","uSection","vStrip","vSection","wStrip","wSection","match","x","y"],
//    "cases":[["r"|"b", ...], ...]}
//   GeometryTPC::MatchCrossPoint(U, V, W, radius) の結果。match=false なら x,y は null。
//   set "r" = 乱数(mt19937 seed 固定)の (u,v,w) 1000 組、"b" = 境界集合:
//     (1) U×V 交点に最も近い W(直線距離)とその ±1・±2 本
//     (2) 3 対の交点がどれも線分の範囲内(±1e-3 mm)で、どれかが範囲の端
//         ([-0.5·pad_pitch, length] の両端)から 1e-3 mm 以内(200 件に達したら打ち切り)
//   どちらも (u,v,w) の strip 番号について、存在する section の全組み合わせを出す。

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <memory>
#include <random>
#include <set>
#include <string>
#include <tuple>
#include <vector>

#include <TVector2.h>

#include "TPCReco/GeometryTPC.h"
#include "TPCReco/StripTPC.h"
#include "TPCReco/UtilsMath.h"

namespace {

const char *const kPlaneName[3] = {"U", "V", "W"};
const double kRadius = 2.0;          // TPCReco v0.09 HistoManager::get2DReconstruction
const unsigned kRandomTriplets = 1000;
const unsigned kRandomSeed = 20261013;
const unsigned kMaxUvPairs = 80;     // 境界集合 (1) に使う U×V 対の数(等間隔に抜く)
const unsigned kMaxEdgeCases = 200;  // 境界集合 (2) の上限
const double kEdgeWindow = 1e-3;     // [mm]

using StripPtr = std::shared_ptr<StripTPC>;

SectionIndexList sorted_sections(const GeometryTPC &geo, int dir) {
  SectionIndexList sections = geo.GetDirSectionIndexList(dir);
  std::sort(sections.begin(), sections.end());
  return sections;
}

struct Segment {
  int dir;
  int section;
  int num;
  StripPtr strip;
};

// mapByStrip と同じ順(dir → section → num)で、実在する strip を全部集める。
std::vector<Segment> collect(const GeometryTPC &geo) {
  std::vector<Segment> out;
  for (int dir = 0; dir < 3; ++dir) {
    for (int section : sorted_sections(geo, dir)) {
      for (int num = geo.GetDirMinStrip(dir, section);
           num <= geo.GetDirMaxStrip(dir, section); ++num) {
        StripPtr s = geo.GetStripByDir(dir, section, num);
        if (s) out.push_back({dir, section, num, s});
      }
    }
  }
  return out;
}

void print_double(FILE *f, double v) { std::fprintf(f, "%.17g", v); }

void print_pair(FILE *f, double a, double b) {
  std::fputc('[', f);
  print_double(f, a);
  std::fputc(',', f);
  print_double(f, b);
  std::fputc(']', f);
}

bool write_segments(const GeometryTPC &geo, const std::vector<Segment> &segs,
                    const char *path) {
  FILE *f = std::fopen(path, "w");
  if (!f) {
    std::perror(path);
    return false;
  }
  const TVector2 ref = geo.GetReferencePoint();
  double xmin = 1e30, xmax = -1e30, ymin = 1e30, ymax = -1e30;
  for (const Segment &s : segs) {
    const TVector2 a = s.strip->Start();
    const TVector2 b = s.strip->End();
    for (const TVector2 &p : {a, b}) {
      xmin = std::min(xmin, p.X());
      xmax = std::max(xmax, p.X());
      ymin = std::min(ymin, p.Y());
      ymax = std::max(ymax, p.Y());
    }
  }
  double txmin, txmax, tymin, tymax;
  std::tie(txmin, txmax, tymin, tymax) = geo.rangeXY();

  std::fprintf(f, "{\"padSize\":");
  print_double(f, geo.GetPadSize());
  std::fprintf(f, ",\"padPitch\":");
  print_double(f, geo.GetPadPitch());
  std::fprintf(f, ",\"stripPitch\":");
  print_double(f, geo.GetStripPitch());
  std::fprintf(f, ",\"referencePoint\":");
  print_pair(f, ref.X(), ref.Y());
  std::fprintf(f, ",\n\"planes\":{");
  for (int dir = 0; dir < 3; ++dir) {
    const TVector2 u = geo.GetStripUnitVector(dir);
    const TVector2 p = geo.GetStripPitchVector(dir);
    std::fprintf(f, "%s\"%s\":{\"unit\":", dir ? "," : "", kPlaneName[dir]);
    print_pair(f, u.X(), u.Y());
    std::fprintf(f, ",\"pitchUnit\":");
    print_pair(f, p.X(), p.Y());
    std::fputc('}', f);
  }
  std::fprintf(f, "},\n\"rangeXY\":[");
  print_double(f, txmin);
  std::fputc(',', f);
  print_double(f, txmax);
  std::fputc(',', f);
  print_double(f, tymin);
  std::fputc(',', f);
  print_double(f, tymax);
  std::fprintf(f, "],\n\"rangeXYCentreLines\":[");
  print_double(f, xmin);
  std::fputc(',', f);
  print_double(f, xmax);
  std::fputc(',', f);
  print_double(f, ymin);
  std::fputc(',', f);
  print_double(f, ymax);
  std::fprintf(f, "],\n\"segments\":[");
  for (size_t i = 0; i < segs.size(); ++i) {
    const Segment &s = segs[i];
    const TVector2 x0 = ref + s.strip->Offset();
    const TVector2 u = s.strip->Unit();
    std::fprintf(f, "%s\n[\"%s\",%d,%d,", i ? "," : "", kPlaneName[s.dir], s.section, s.num);
    print_double(f, x0.X());
    std::fputc(',', f);
    print_double(f, x0.Y());
    std::fputc(',', f);
    print_double(f, u.X());
    std::fputc(',', f);
    print_double(f, u.Y());
    std::fputc(',', f);
    print_double(f, s.strip->Length());
    std::fputc(']', f);
  }
  std::fprintf(f, "]}\n");
  std::fclose(f);
  return true;
}

// 選択用の複製(結果は必ず GeometryTPC::MatchCrossPoint から取る)。GetCrossPoint と同じ式で
// 対の交点の「線分上の位置」len1/len2 を返す。平行なら false。
bool cross_lengths(const StripPtr &a, const StripPtr &b, double &len1, double &len2) {
  const TVector2 u1 = a->Unit();
  const TVector2 u2 = b->Unit();
  const double w = -u1.X() * u2.Y() + u1.Y() * u2.X();
  if (std::fabs(w) < Utils::NUMERICAL_TOLERANCE) return false;
  const TVector2 off = b->Offset() - a->Offset();
  len1 = (-off.X() * u2.Y() + off.Y() * u2.X()) / w;
  len2 = (u1.X() * off.Y() - u1.Y() * off.X()) / w;
  return true;
}

using CaseKey = std::tuple<int, int, int, int, int, int>;  // u, su, v, sv, w, sw

struct Cases {
  std::set<CaseKey> seen_boundary;
  std::vector<std::string> lines;
};

void emit(const GeometryTPC &geo, Cases &cases, const char *set, int u, int su, int v, int sv,
          int w, int sw) {
  if (set[0] == 'b' && !cases.seen_boundary.insert(CaseKey(u, su, v, sv, w, sw)).second) return;
  TVector2 point;
  const bool ok = geo.MatchCrossPoint(geo.GetStripByDir(0, su, u), geo.GetStripByDir(1, sv, v),
                                      geo.GetStripByDir(2, sw, w), kRadius, point);
  char buf[256];
  if (ok) {
    std::snprintf(buf, sizeof buf, "[\"%s\",%d,%d,%d,%d,%d,%d,true,%.17g,%.17g]", set, u, su, v,
                  sv, w, sw, point.X(), point.Y());
  } else {
    std::snprintf(buf, sizeof buf, "[\"%s\",%d,%d,%d,%d,%d,%d,false,null,null]", set, u, su, v,
                  sv, w, sw);
  }
  cases.lines.push_back(buf);
}

// (u,v,w) の strip 番号について、存在する section の全組み合わせを出す。
void emit_all_sections(const GeometryTPC &geo, Cases &cases, const char *set, int u, int v,
                       int w) {
  for (int su : sorted_sections(geo, 0)) {
    if (!geo.GetStripByDir(0, su, u)) continue;
    for (int sv : sorted_sections(geo, 1)) {
      if (!geo.GetStripByDir(1, sv, v)) continue;
      for (int sw : sorted_sections(geo, 2)) {
        if (!geo.GetStripByDir(2, sw, w)) continue;
        emit(geo, cases, set, u, su, v, sv, w, sw);
      }
    }
  }
}

bool near_edge(double len, double length, double pad_pitch) {
  return std::fabs(len + 0.5 * pad_pitch) < kEdgeWindow || std::fabs(len - length) < kEdgeWindow;
}

bool within(double len, double length, double pad_pitch) {
  return len >= -0.5 * pad_pitch - kEdgeWindow && len <= length + kEdgeWindow;
}

bool write_match_cases(const GeometryTPC &geo, const std::vector<Segment> &segs,
                       const char *path) {
  std::vector<const Segment *> by_dir[3];
  for (const Segment &s : segs) by_dir[s.dir].push_back(&s);
  for (int dir = 0; dir < 3; ++dir) {
    if (by_dir[dir].empty()) {
      std::fprintf(stderr, "no %s strips — no match cases\n", kPlaneName[dir]);
      return false;
    }
  }
  Cases cases;
  const double pp = geo.GetPadPitch();

  // 乱数: 各面の merged 範囲から一様に strip 番号を選ぶ(mt19937 の生出力 % 幅 = 処理系非依存)。
  std::mt19937 rng(kRandomSeed);
  auto pick = [&](int dir) {
    const int lo = geo.GetDirMinStripMerged(dir);
    const int hi = geo.GetDirMaxStripMerged(dir);
    return lo + static_cast<int>(rng() % static_cast<unsigned>(hi - lo + 1));
  };
  for (unsigned i = 0; i < kRandomTriplets; ++i) {
    const int u = pick(0);
    const int v = pick(1);
    const int w = pick(2);
    emit_all_sections(geo, cases, "r", u, v, w);
  }

  // 境界集合 (1): U×V 交点に最も近い W とその ±1・±2 本。
  std::vector<std::tuple<int, int, TVector2>> uv;  // u, v, 交点
  for (const Segment *a : by_dir[0]) {
    for (const Segment *b : by_dir[1]) {
      TVector2 p;
      if (geo.GetCrossPoint(a->strip, b->strip, p)) uv.emplace_back(a->num, b->num, p);
    }
  }
  const size_t stride = std::max<size_t>(1, uv.size() / kMaxUvPairs);
  for (size_t i = 0; i < uv.size(); i += stride) {
    int u, v;
    TVector2 p;
    std::tie(u, v, p) = uv[i];
    const Segment *best = nullptr;
    double best_dist = 1e30;
    for (const Segment *c : by_dir[2]) {
      const TVector2 base = geo.GetReferencePoint() + c->strip->Offset();
      const TVector2 d = p - base;
      const double dist = std::fabs(d.X() * c->strip->Unit().Y() - d.Y() * c->strip->Unit().X());
      if (dist < best_dist) {
        best_dist = dist;
        best = c;
      }
    }
    for (int k = -2; k <= 2; ++k) emit_all_sections(geo, cases, "b", u, v, best->num + k);
  }

  // 境界集合 (2): 3 対とも線分の範囲内(±1e-3 mm)で、どれかの対が範囲の端から 1e-3 mm 以内。
  unsigned edge_cases = 0;
  for (const Segment *a : by_dir[0]) {
    for (const Segment *b : by_dir[1]) {
      double ab1, ab2;
      if (!cross_lengths(a->strip, b->strip, ab1, ab2)) continue;
      if (!within(ab1, a->strip->Length(), pp) || !within(ab2, b->strip->Length(), pp)) continue;
      for (const Segment *c : by_dir[2]) {
        double bc1, bc2, ca1, ca2;
        if (!cross_lengths(b->strip, c->strip, bc1, bc2)) continue;
        if (!cross_lengths(c->strip, a->strip, ca1, ca2)) continue;
        if (!within(bc2, c->strip->Length(), pp) || !within(bc1, b->strip->Length(), pp) ||
            !within(ca1, c->strip->Length(), pp) || !within(ca2, a->strip->Length(), pp))
          continue;
        const bool edge = near_edge(ab1, a->strip->Length(), pp) ||
                          near_edge(ab2, b->strip->Length(), pp) ||
                          near_edge(bc1, b->strip->Length(), pp) ||
                          near_edge(bc2, c->strip->Length(), pp) ||
                          near_edge(ca1, c->strip->Length(), pp) ||
                          near_edge(ca2, a->strip->Length(), pp);
        if (!edge || edge_cases >= kMaxEdgeCases) continue;
        const size_t before = cases.lines.size();
        emit_all_sections(geo, cases, "b", a->num, b->num, c->num);
        edge_cases += static_cast<unsigned>(cases.lines.size() - before);
      }
    }
  }

  FILE *f = std::fopen(path, "w");
  if (!f) {
    std::perror(path);
    return false;
  }
  std::fprintf(f, "{\"radius\":");
  print_double(f, kRadius);
  std::fprintf(f, ",\"numericalTolerance\":");
  print_double(f, Utils::NUMERICAL_TOLERANCE);
  std::fprintf(f,
               ",\n\"columns\":[\"set\",\"uStrip\",\"uSection\",\"vStrip\",\"vSection\","
               "\"wStrip\",\"wSection\",\"match\",\"x\",\"y\"],\n\"cases\":[");
  for (size_t i = 0; i < cases.lines.size(); ++i) {
    std::fprintf(f, "%s\n%s", i ? "," : "", cases.lines[i].c_str());
  }
  std::fprintf(f, "]}\n");
  std::fclose(f);
  std::fprintf(stderr, "match cases: %zu (edge cases %u)\n", cases.lines.size(), edge_cases);
  return true;
}

}  // namespace

int main(int argc, char **argv) {
  if (argc != 3 && argc != 4) {
    std::fprintf(stderr, "usage: %s GEOMETRY.dat SEGMENTS.json [MATCH_CASES.json]\n", argv[0]);
    return 2;
  }
  GeometryTPC geo(argv[1]);
  if (!geo.IsOK()) {
    std::fprintf(stderr, "GeometryTPC failed to load %s\n", argv[1]);
    return 1;
  }
  const std::vector<Segment> segs = collect(geo);
  std::fprintf(stderr, "segments: %zu\n", segs.size());
  if (!write_segments(geo, segs, argv[2])) return 1;
  if (argc == 4 && !write_match_cases(geo, segs, argv[3])) return 1;
  return 0;
}
