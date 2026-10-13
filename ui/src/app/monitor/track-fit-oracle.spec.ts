/**
 * TODO/125 改訂 2 レーン D — **本物の** macros/analyzeUVW.C + makeTracks.C との照合(env-gated)。
 *
 * 入力(どちらも実データ由来 — ローカルのみ、リポに入れない。無ければ skip):
 * - TPCDAQ_MACRO_ORACLE          : `tools/macro_oracle/run_macro_oracle.sh` の出力 JSON
 *                                   (raw の adc・uvw の strip ヒット・tracks の全ブランチ・channel_map.csv)
 * - TPCDAQ_MACRO_ORACLE_GEOMETRY : 同じ検出器の stripGeometry JSON
 *                                   (`ws_proto_sample --geometry <.dat> --strip-geometry-out <json>`)
 *
 * adc から 0x02 を組み(channel_map の (aget, ch) → (view, strip)、section 合算 = 実機の monitor と同じ)、
 * 本番の `trackEvent` を 2 通りに回す:
 * 1. **production**: s を stripGeometry から(本番どおり)。macros の s は channel_map.csv(小数 3 桁)
 *    なので、トラックの量は s の丸め(≤ 5e-4 mm)の分だけずれうる。
 * 2. **csv-s**: stripGeometry の線分を channel_map.csv の s に置き換えた複製で回す。入力の違いは
 *    macros の出力が float32 で書かれていることだけ → アルゴリズムの移植そのものを細かい許容で見る。
 */
import { readFileSync } from 'node:fs';

import {
  type StripGeometryMessage,
  isStripGeometryUnavailable,
  parseJsonMessage,
} from '../ws/json';
import type { UvwMessage } from '../ws/wire';
import { type StripHit, type TrackEventResult, type TrackOk, trackEvent } from './track-fit';

const oraclePath = process.env['TPCDAQ_MACRO_ORACLE'] ?? '';
const geometryPath = process.env['TPCDAQ_MACRO_ORACLE_GEOMETRY'] ?? '';

/** `[aget, ch, view, strip, s_mm]` */
type MapRow = [number, number, number, number, number];

interface OracleTrack {
  ok: number;
  xStart: number;
  yStart: number;
  zStart: number;
  xEnd: number;
  yEnd: number;
  zEnd: number;
  dirX: number;
  dirY: number;
  dirZ: number;
  length: number;
  charge: number;
  xStartF: number;
  yStartF: number;
  zStartF: number;
  xEndF: number;
  yEndF: number;
  zEndF: number;
  lengthF: number;
}

interface OracleEvent {
  eventId: number;
  adc: number[][];
  uvw: { view: number[]; z: number[]; s: number[]; q: number[] };
  track: OracleTrack;
}

interface Oracle {
  channelMap: MapRow[];
  events: OracleEvent[];
}

type Point3 = readonly [number, number, number];

/** float32 の 1 ulp(相対)。macros の slZ / slQ は float で書かれる。 */
const FLOAT32_REL = 2 ** -23;

/**
 * 許容(実測の最大差は TODO/125 の結果節に記録する)。
 * - ヒットの z・q: float32 の丸め(相対 2⁻²³)。どちらの経路でも同じ(s に依らない)。
 * - トラックの量:
 *   - csv-s: 入力の違いは float32 の丸めだけ(s・z ≲ 100 mm → 1 ulp ≈ 7.6e-6 mm)。位置・長さは
 *     その数 ulp(2e-5 mm)、方向 1e-6、電荷(float の q の和)は相対 2⁻²³。
 *   - production: さらに channel_map.csv の s の丸め(小数 3 桁 → strip ごとに ≤ 5e-4 mm)が入る。
 *     位置・長さはその 2 倍(1e-3 mm)、方向は 5e-4 mm / 25 mm の腕 = 2e-5。
 */
const TOL = {
  'csv-s': { position: 2e-5, direction: 1e-6, length: 2e-5, chargeRel: FLOAT32_REL },
  production: { position: 1e-3, direction: 2e-5, length: 1e-3, chargeRel: FLOAT32_REL },
} as const;

type Mode = keyof typeof TOL;

interface Stats {
  events: number;
  okBoth: number;
  notOkBoth: number;
  okMismatch: string[];
  hitSetMismatch: string[];
  hitsCompared: number;
  maxZ: number;
  maxQRel: number;
  maxPosition: number;
  maxDirection: number;
  maxLength: number;
  maxChargeRel: number;
  worst: string;
}

function emptyStats(): Stats {
  return {
    events: 0,
    okBoth: 0,
    notOkBoth: 0,
    okMismatch: [],
    hitSetMismatch: [],
    hitsCompared: 0,
    maxZ: 0,
    maxQRel: 0,
    maxPosition: 0,
    maxDirection: 0,
    maxLength: 0,
    maxChargeRel: 0,
    worst: '',
  };
}

function dist(p: Point3, q: Point3): number {
  return Math.hypot(p[0] - q[0], p[1] - q[1], p[2] - q[2]);
}

/** 順序を問わない端点の組の差(良い方の対応の、悪い方の端点の距離)。 */
function pairDistance(a: [Point3, Point3], b: [Point3, Point3]): number {
  return Math.min(
    Math.max(dist(a[0], b[0]), dist(a[1], b[1])),
    Math.max(dist(a[0], b[1]), dist(a[1], b[0])),
  );
}

describe.skipIf(oraclePath === '' || geometryPath === '')(
  'TODO/125 macros オラクル(本物の analyzeUVW.C + makeTracks.C と track-fit.ts)',
  () => {
    let oracle: Oracle;
    let geometry: StripGeometryMessage;
    let csvGeometry: StripGeometryMessage;
    /** view → "s(float の値そのもの)" → strip(macros の slS を strip に戻す)。 */
    let stripOfS: Map<string, number>;
    const stats: Record<Mode, Stats> = { 'csv-s': emptyStats(), production: emptyStats() };

    beforeAll(() => {
      oracle = JSON.parse(readFileSync(oraclePath, 'utf8')) as Oracle;
      const parsed = parseJsonMessage(readFileSync(geometryPath, 'utf8'));
      if (parsed.kind !== 'stripGeometry' || isStripGeometryUnavailable(parsed.geometry)) {
        throw new Error(`${geometryPath} is not a usable stripGeometry`);
      }
      geometry = parsed.geometry;
      stripOfS = new Map(oracle.channelMap.map(([, , view, strip, s]) => [`${view}:${s}`, strip]));

      // csv-s: 各 strip の線分を「pitch 座標が channel_map の s」の位置へ(pitch 方向に)置き直す
      const sOf = new Map(oracle.channelMap.map(([, , view, strip, s]) => [`${view}:${strip}`, s]));
      const planes = (['U', 'V', 'W'] as const).map((name, view) => {
        const plane = geometry.planes[name];
        const [px, py] = plane.pitchUnit;
        return {
          ...plane,
          segments: plane.segments.map(([strip, section, x0, y0, length]) => {
            const s = sOf.get(`${view}:${strip}`);
            if (s === undefined) return [strip, section, x0, y0, length] as const;
            const shift = s - (px * x0 + py * y0);
            return [strip, section, x0 + shift * px, y0 + shift * py, length] as const;
          }),
        };
      });
      csvGeometry = { ...geometry, planes: { U: planes[0], V: planes[1], W: planes[2] } };

      for (const event of oracle.events) {
        const uvw = buildUvw(event);
        compare('production', event, trackEvent(uvw, geometry));
        compare('csv-s', event, trackEvent(uvw, csvGeometry));
      }
      for (const mode of ['production', 'csv-s'] as const) {
        const s = stats[mode];
        console.info(
          `[TODO/125 macro oracle ${mode}] events=${s.events} ok(both)=${s.okBoth} ` +
            `notOk(both)=${s.notOkBoth} okMismatch=${s.okMismatch.length} ` +
            `hitSetMismatch=${s.hitSetMismatch.length} hits=${s.hitsCompared} ` +
            `max|dz|=${s.maxZ.toExponential(2)}mm max|dq|/q=${s.maxQRel.toExponential(2)} ` +
            `max|dpos|=${s.maxPosition.toExponential(2)}mm max|ddir|=${s.maxDirection.toExponential(2)} ` +
            `max|dlen|=${s.maxLength.toExponential(2)}mm max|dQ|/Q=${s.maxChargeRel.toExponential(2)} ` +
            `worst: ${s.worst}`,
        );
      }
    });

    /** 1 イベントの adc(channel_map の行順)→ U/V/W の 0x02(strip × 512、section 合算)。 */
    function buildUvw(event: OracleEvent): UvwMessage[] {
      const planes = [geometry.planes.U, geometry.planes.V, geometry.planes.W];
      const nBuckets = event.adc[0].length;
      const messages = planes.map((plane, view) => {
        const nStrips = Math.max(...plane.segments.map(([strip]) => strip));
        return {
          kind: 'uvw' as const,
          header: { msgType: 2, incomplete: false, runNumber: 70, eventNumber: event.eventId },
          plane: view,
          nStrips,
          nBuckets,
          adc: new Uint16Array(nStrips * nBuckets),
        };
      });
      oracle.channelMap.forEach(([, , view, strip], k) => {
        const target = messages[view];
        const offset = (strip - 1) * nBuckets;
        const wave = event.adc[k];
        for (let t = 0; t < nBuckets; t++) target.adc[offset + t] += wave[t];
      });
      return messages;
    }

    function compare(mode: Mode, event: OracleEvent, result: TrackEventResult | null): void {
      const s = stats[mode];
      s.events++;
      const label = `event ${event.eventId}`;
      if (result === null) throw new Error(`${label}: trackEvent returned null`);

      // ヒット: (view, strip) の集合と、z・q
      const ours = new Map<string, StripHit>();
      for (const hits of result.selected) for (const h of hits) ours.set(`${h.view}:${h.strip}`, h);
      const theirs = new Map<string, { z: number; q: number }>();
      event.uvw.view.forEach((view, k) => {
        const strip = stripOfS.get(`${view}:${event.uvw.s[k]}`);
        if (strip === undefined) throw new Error(`${label}: slS ${event.uvw.s[k]} not in the map`);
        theirs.set(`${view}:${strip}`, { z: event.uvw.z[k], q: event.uvw.q[k] });
      });
      const sameSet = ours.size === theirs.size && [...ours.keys()].every((key) => theirs.has(key));
      if (!sameSet) {
        const extra = [...ours.keys()].filter((key) => !theirs.has(key));
        const missing = [...theirs.keys()].filter((key) => !ours.has(key));
        s.hitSetMismatch.push(`${label}: ours-only [${extra}] macro-only [${missing}]`);
      }
      for (const [key, macro] of theirs) {
        const h = ours.get(key);
        if (!h) continue;
        s.hitsCompared++;
        s.maxZ = Math.max(s.maxZ, Math.abs(h.z - macro.z));
        s.maxQRel = Math.max(s.maxQRel, Math.abs(h.q - macro.q) / macro.q);
      }

      // トラック
      const track = result.track;
      if ((event.track.ok === 1) !== track.ok) {
        s.okMismatch.push(
          `${label}: macro ok=${event.track.ok}, ours ${track.ok ? 'ok' : track.reason}`,
        );
        return;
      }
      if (!track.ok) {
        s.notOkBoth++;
        return;
      }
      s.okBoth++;
      const m = event.track;
      const t = track as TrackOk;
      const position = Math.max(
        pairDistance(
          [t.start, t.end],
          [
            [m.xStart, m.yStart, m.zStart],
            [m.xEnd, m.yEnd, m.zEnd],
          ],
        ),
        pairDistance(
          [t.startF, t.endF],
          [
            [m.xStartF, m.yStartF, m.zStartF],
            [m.xEndF, m.yEndF, m.zEndF],
          ],
        ),
      );
      const dir: Point3 = [m.dirX, m.dirY, m.dirZ];
      const direction = Math.min(dist(t.dir, dir), dist(t.dir, [-dir[0], -dir[1], -dir[2]]));
      const length = Math.max(Math.abs(t.length - m.length), Math.abs(t.lengthF - m.lengthF));
      const chargeRel = Math.abs(t.charge - m.charge) / m.charge;
      if (position > s.maxPosition) {
        s.worst = `${label} (position ${position.toExponential(2)} mm)`;
      }
      s.maxPosition = Math.max(s.maxPosition, position);
      s.maxDirection = Math.max(s.maxDirection, direction);
      s.maxLength = Math.max(s.maxLength, length);
      s.maxChargeRel = Math.max(s.maxChargeRel, chargeRel);
    }

    it('channel_map.csv の s と stripGeometry の s(pitchUnit·(x0, y0))が 1e-3 mm 以内', () => {
      const planes = [geometry.planes.U, geometry.planes.V, geometry.planes.W];
      let max = 0;
      for (const [, , view, strip, s] of oracle.channelMap) {
        const plane = planes[view];
        const segment = plane.segments.find(([n, section]) => n === strip && section === 0);
        if (!segment) throw new Error(`view ${view} strip ${strip} has no section-0 segment`);
        const sGeometry = plane.pitchUnit[0] * segment[2] + plane.pitchUnit[1] * segment[3];
        max = Math.max(max, Math.abs(sGeometry - s));
      }
      console.info(`[TODO/125 macro oracle] max|s_csv - s_geometry| = ${max.toExponential(2)} mm`);
      expect(max).toBeLessThanOrEqual(1e-3);
    });

    for (const mode of ['production', 'csv-s'] as const) {
      describe(mode, () => {
        it('全イベントで ok / not ok が一致し、帯内ヒットの (view, strip) の集合が一致する', () => {
          const s = stats[mode];
          expect(s.events).toBe(oracle.events.length);
          expect(s.okMismatch).toEqual([]);
          expect(s.hitSetMismatch).toEqual([]);
          expect(s.okBoth + s.notOkBoth).toBe(s.events);
        });

        it('ヒットの z・q が float32 の丸め(相対 2⁻²³)以内', () => {
          const s = stats[mode];
          expect(s.hitsCompared).toBeGreaterThan(0);
          // z ≤ 512 cell × 0.1736 mm ≈ 89 mm → 2⁻²³ × 89 ≈ 1.1e-5 mm
          expect(s.maxZ).toBeLessThanOrEqual(FLOAT32_REL * 89);
          expect(s.maxQRel).toBeLessThanOrEqual(FLOAT32_REL);
        });

        it('トラック: 方向(符号は問わない)・端点(2–98 % と最外、順序は問わない)・長さ・電荷', () => {
          const s = stats[mode];
          const tol = TOL[mode];
          expect(s.okBoth).toBeGreaterThan(0);
          expect(s.maxDirection).toBeLessThanOrEqual(tol.direction);
          expect(s.maxPosition).toBeLessThanOrEqual(tol.position);
          expect(s.maxLength).toBeLessThanOrEqual(tol.length);
          expect(s.maxChargeRel).toBeLessThanOrEqual(tol.chargeRel);
        });
      });
    }
  },
);
