/**
 * TODO/125 レーン C — 言語をまたぐ照合(Rust `stripGeometry` → TS 本番パーサ → TPCReco オラクル)。
 *
 * Rust の `ws_proto_sample --geometry … --strip-geometry-out …` が、monitor が配るのと同じ
 * `stripGeometry` JSON を合成ジオメトリ `tests/fixtures/geometry_xy_synth.dat` から書く。
 * それを本番パーサ(`./json`)で読み、TPCReco `GeometryTPC` の線分・rangeXY(`segments.json`、
 * コミット済み)と 1e-9 mm で一致することを確かめる。
 * (MatchCrossPoint の照合は 125 改訂 2 で三面一致の投影ごと撤去 — 投影は macros の移植になった。)
 *
 * 入力は環境変数(`ui/run_ws_conformance.sh` が与える。無ければ skip — 通常の `ng test` は落とさない):
 * - TPCDAQ_STRIP_GEOMETRY   : 生成された stripGeometry JSON
 * - TPCDAQ_XY_ORACLE_DIR    : tests/fixtures/geometry_xy_oracle の絶対パス
 */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

import { isStripGeometryUnavailable, parseJsonMessage, type StripGeometryMessage } from './json';

const geometryPath = process.env['TPCDAQ_STRIP_GEOMETRY'] ?? '';
const oracleDir = process.env['TPCDAQ_XY_ORACLE_DIR'] ?? '';

/** 発注書の許容(mm)。 */
const TOL = 1e-9;

/** 線分の最小形(照合に使う分だけ)。 */
interface Segment {
  readonly x0: number;
  readonly y0: number;
  readonly length: number;
}

describe.skipIf(geometryPath === '' || oracleDir === '')(
  'TODO/125 stripGeometry 適合(Rust 生成 → TS 本番パーサ → TPCReco GeometryTPC)',
  () => {
    let geometry: StripGeometryMessage;
    let unknownKeys: readonly string[] = [];
    let segmentsOracle: {
      rangeXY: number[];
      planes: Record<string, { unit: number[]; pitchUnit: number[] }>;
      segments: [string, number, number, number, number, number, number, number][];
    };
    /** plane(U,V,W の順)→ "strip/section" → 線分。 */
    let lookup: Map<string, Segment>[];

    beforeAll(() => {
      const parsed = parseJsonMessage(readFileSync(geometryPath, 'utf8'));
      if (parsed.kind !== 'stripGeometry') throw new Error(`parsed as ${parsed.kind}`);
      if (isStripGeometryUnavailable(parsed.geometry)) {
        throw new Error(`unavailable: ${parsed.geometry.unavailable}`);
      }
      geometry = parsed.geometry;
      unknownKeys = parsed.unknownKeys;
      segmentsOracle = JSON.parse(readFileSync(join(oracleDir, 'segments.json'), 'utf8'));
      const planes = [geometry.planes.U, geometry.planes.V, geometry.planes.W];
      lookup = planes.map((plane) => {
        const map = new Map<string, Segment>();
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

    it('unit / pitchUnit が TPCReco と 1e-9 で一致する(track-fit の s = pitchUnit·(x0, y0))', () => {
      for (const name of ['U', 'V', 'W'] as const) {
        const plane = geometry.planes[name];
        const oracle = segmentsOracle.planes[name];
        for (let i = 0; i < 2; i++) {
          expect(
            Math.abs(plane.unit[i] - oracle.unit[i]),
            `${name} unit[${i}]`,
          ).toBeLessThanOrEqual(TOL);
          expect(
            Math.abs(plane.pitchUnit[i] - oracle.pitchUnit[i]),
            `${name} pitchUnit[${i}]`,
          ).toBeLessThanOrEqual(TOL);
        }
      }
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
  },
);
