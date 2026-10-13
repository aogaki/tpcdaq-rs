// geometry_xy_oracle — TPCReco GeometryTPC を正として strip 線分の mm を JSON に書き出す
// (TODO/125 レーン A-4)。Rust の src/strip_xy.rs(線分・rangeXY)と、UI が受け取る stripGeometry
// (レーン C の言語をまたぐ照合)の照合元。
// MatchCrossPoint の結果(match_cases.json)は 125 改訂 2 で撤去 — Event Display の投影は
// U/V/W 三面一致をやめて macros/analyzeUVW.C + makeTracks.C の移植になった(tools/macro_oracle/)。
//
//   geometry_xy_oracle GEOMETRY.dat SEGMENTS.json
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

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <memory>
#include <tuple>
#include <vector>

#include <TVector2.h>

#include "TPCReco/GeometryTPC.h"
#include "TPCReco/StripTPC.h"

namespace {

const char *const kPlaneName[3] = {"U", "V", "W"};

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

}  // namespace

int main(int argc, char **argv) {
  if (argc != 3) {
    std::fprintf(stderr, "usage: %s GEOMETRY.dat SEGMENTS.json\n", argv[0]);
    return 2;
  }
  GeometryTPC geo(argv[1]);
  if (!geo.IsOK()) {
    std::fprintf(stderr, "GeometryTPC failed to load %s\n", argv[1]);
    return 1;
  }
  const std::vector<Segment> segs = collect(geo);
  std::fprintf(stderr, "segments: %zu\n", segs.size());
  return write_segments(geo, segs, argv[2]) ? 0 : 1;
}
