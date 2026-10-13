/**
 * TODO/125 レーン C — 言語をまたぐ照合(Rust `stripGeometry` → TS 本番パーサ → TPCReco オラクル)。
 *
 * Rust の `ws_proto_sample --geometry … --strip-geometry-out …` が、monitor が配るのと同じ
 * `stripGeometry` JSON を合成ジオメトリ `tests/fixtures/geometry_xy_synth.dat` から書く。
 * それを本番パーサ(`./json`)で読み、TPCReco `GeometryTPC::MatchCrossPoint(radius = 2.0)` の結果
 * (`match_cases.json`、コミット済み)の全件を、本番の `matchCrossPoint`(`../monitor/xy-projection`)で
 * 再現できるかを確かめる(true/false 一致、true なら点が 1e-9 mm 以内)。
 *
 * 入力は環境変数(`ui/run_ws_conformance.sh` が与える。無ければ skip — 通常の `ng test` は落とさない):
 * - TPCDAQ_STRIP_GEOMETRY   : 生成された stripGeometry JSON
 * - TPCDAQ_XY_ORACLE_DIR    : tests/fixtures/geometry_xy_oracle の絶対パス
 */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

import { matchCrossPoint, type SegmentLike, type Vec2 } from '../monitor/xy-projection';
import { isStripGeometryUnavailable, parseJsonMessage, type StripGeometryMessage } from './json';

const geometryPath = process.env['TPCDAQ_STRIP_GEOMETRY'] ?? '';
const oracleDir = process.env['TPCDAQ_XY_ORACLE_DIR'] ?? '';

/** 発注書の許容(mm)。 */
const TOL = 1e-9;

/** `[set, uStrip, uSection, vStrip, vSection, wStrip, wSection, match, x, y]` */
type CaseRow = [string, number, number, number, number, number, number, boolean, number?, number?];

describe.skipIf(geometryPath === '' || oracleDir === '')(
  'TODO/125 stripGeometry 適合(Rust 生成 → TS 本番パーサ → TPCReco MatchCrossPoint)',
  () => {
    let geometry: StripGeometryMessage;
    let unknownKeys: readonly string[] = [];
    let matchCases: { radius: number; cases: CaseRow[] };
    let segmentsOracle: {
      rangeXY: number[];
      segments: [string, number, number, number, number, number, number, number][];
    };
    /** plane(U,V,W の順)→ "strip/section" → 線分。 */
    let lookup: Map<string, SegmentLike>[];
    let units: Vec2[];

    beforeAll(() => {
      const parsed = parseJsonMessage(readFileSync(geometryPath, 'utf8'));
      if (parsed.kind !== 'stripGeometry') throw new Error(`parsed as ${parsed.kind}`);
      if (isStripGeometryUnavailable(parsed.geometry)) {
        throw new Error(`unavailable: ${parsed.geometry.unavailable}`);
      }
      geometry = parsed.geometry;
      unknownKeys = parsed.unknownKeys;
      matchCases = JSON.parse(readFileSync(join(oracleDir, 'match_cases.json'), 'utf8'));
      segmentsOracle = JSON.parse(readFileSync(join(oracleDir, 'segments.json'), 'utf8'));
      const planes = [geometry.planes.U, geometry.planes.V, geometry.planes.W];
      units = planes.map((p) => [p.unit[0], p.unit[1]] as Vec2);
      lookup = planes.map((plane) => {
        const map = new Map<string, SegmentLike>();
        for (const [strip, section, x0, y0, length] of plane.segments) {
          map.set(`${strip}/${section}`, { x0, y0, length });
        }
        return map;
      });
    });

    it('本番パーサが通り、未知キーは無い', () => {
      expect(geometry.type).toBe('stripGeometry');
      expect(unknownKeys).toEqual([]);
    });

    it('rangeXY が TPCReco の rangeXY と 1e-9 で一致する', () => {
      expect(geometry.rangeXY).toHaveLength(4);
      geometry.rangeXY.forEach((value, i) => {
        expect(Math.abs(value - segmentsOracle.rangeXY[i]), `rangeXY[${i}]`).toBeLessThanOrEqual(
          TOL,
        );
      });
    });

    it('全線分(x0, y0, length)が TPCReco の segments.json と 1e-9 で一致する', () => {
      const names = ['U', 'V', 'W'];
      for (const [plane, section, strip, x0, y0, , , length] of segmentsOracle.segments) {
        const seg = lookup[names.indexOf(plane)].get(`${strip}/${section}`);
        if (seg === undefined)
          throw new Error(`${plane} strip ${strip} section ${section} missing`);
        expect(Math.abs(seg.x0 - x0), `${plane}${strip}/${section} x0`).toBeLessThanOrEqual(TOL);
        expect(Math.abs(seg.y0 - y0), `${plane}${strip}/${section} y0`).toBeLessThanOrEqual(TOL);
        expect(
          Math.abs(seg.length - length),
          `${plane}${strip}/${section} len`,
        ).toBeLessThanOrEqual(TOL);
      }
      expect(segmentsOracle.segments.length).toBe(lookup.reduce((n, m) => n + m.size, 0));
    });

    it('MatchCrossPoint の全件(true/false と点)が TPCReco と一致する', () => {
      expect(matchCases.radius).toBe(2.0);
      let nTrue = 0;
      let nFalse = 0;
      let maxDiff = 0;
      for (const [set, u, su, v, sv, w, sw, match, x, y] of matchCases.cases) {
        const label = `${set} U${u}/${su} V${v}/${sv} W${w}/${sw}`;
        const sU = lookup[0].get(`${u}/${su}`);
        const sV = lookup[1].get(`${v}/${sv}`);
        const sW = lookup[2].get(`${w}/${sw}`);
        if (!sU || !sV || !sW) throw new Error(`${label}: segment missing in stripGeometry`);
        const point = matchCrossPoint(
          sU,
          sV,
          sW,
          units[0],
          units[1],
          units[2],
          geometry.padPitch,
          matchCases.radius,
        );
        expect(point !== null, `${label}: TPCReco match=${match}`).toBe(match);
        if (match && point !== null) {
          nTrue++;
          const dx = Math.abs(point[0] - (x as number));
          const dy = Math.abs(point[1] - (y as number));
          maxDiff = Math.max(maxDiff, dx, dy);
          expect(dx, `${label}: x`).toBeLessThanOrEqual(TOL);
          expect(dy, `${label}: y`).toBeLessThanOrEqual(TOL);
        } else {
          nFalse++;
        }
      }
      console.log(
        `[TODO/125 conformance] compared ${matchCases.cases.length} cases: true=${nTrue} false=${nFalse} max|diff|=${maxDiff}`,
      );
      expect(nTrue + nFalse).toBe(matchCases.cases.length);
      expect(nTrue).toBeGreaterThan(0);
      expect(nFalse).toBeGreaterThan(0);
    });
  },
);
