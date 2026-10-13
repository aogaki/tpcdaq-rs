/**
 * TODO/125 のテスト用の合成ジオメトリ / 合成 0x02(spec からだけ import する。本番コードは使わない)。
 *
 * 等間隔の直線 3 族: 面 p の strip s は、法線方向に `(s - (n+1)/2) * d` ずらした直線
 * (section k >= 1 は更に `0.7*d*k` ずらす = 同じ strip 番号で一直線にならない)。
 * 六角配置(0/-120/-60)なら三面は格子点で一点に交わる。
 */
import type { StripGeometryMessage, StripPlaneGeometry } from '../ws/json';
import type { UvwMessage } from '../ws/wire';

/**
 * メモリ上の localStorage 代用品。テスト環境(Node の実験的 localStorage)では
 * `localStorage` が undefined のことがあるので `vi.stubGlobal('localStorage', ...)` で差し込む。
 */
export function memoryStorage(): Storage {
  const data = new Map<string, string>();
  return {
    get length() {
      return data.size;
    },
    clear: () => data.clear(),
    getItem: (key: string) => data.get(key) ?? null,
    key: (index: number) => [...data.keys()][index] ?? null,
    removeItem: (key: string) => void data.delete(key),
    setItem: (key: string, value: string) => void data.set(key, String(value)),
  };
}

export interface Layout {
  readonly angles: readonly [number, number, number];
  /** 面毎の strip 本数(奇数)。 */
  readonly n: number;
  /** strip の間隔(法線方向、mm)。 */
  readonly d: number;
  /** 線分の長さ。 */
  readonly length: number;
  /** 面毎の section 数(2 本目は法線方向に 0.7*d ずらす = 同じ strip 番号で一直線にならない)。 */
  readonly sections: readonly [number, number, number];
}

export const HEX: Layout = {
  angles: [0, -120, -60],
  n: 201,
  d: 1.5,
  length: 400,
  sections: [1, 1, 1],
};

function planeOf(angleDeg: number, layout: Layout, sections: number): StripPlaneGeometry {
  const a = (angleDeg * Math.PI) / 180;
  const unit: [number, number] = [Math.cos(a), Math.sin(a)];
  const normal: [number, number] = [-Math.sin(a), Math.cos(a)];
  const segments: [number, number, number, number, number][] = [];
  for (let strip = 1; strip <= layout.n; strip++) {
    for (let section = 0; section < sections; section++) {
      const pos = (strip - (layout.n + 1) / 2) * layout.d + section * 0.7 * layout.d;
      segments.push([
        strip,
        section,
        normal[0] * pos - (unit[0] * layout.length) / 2,
        normal[1] * pos - (unit[1] * layout.length) / 2,
        layout.length,
      ]);
    }
  }
  return { unit, pitchUnit: normal, segments };
}

export function geometryOf(layout: Layout): StripGeometryMessage {
  const extent = (layout.n * layout.d) / 2;
  return {
    type: 'stripGeometry',
    padPitch: 1.7320508,
    stripPitch: 1.5,
    rangeXY: [-extent, extent, -extent, extent],
    planes: {
      U: planeOf(layout.angles[0], layout, layout.sections[0]),
      V: planeOf(layout.angles[1], layout, layout.sections[1]),
      W: planeOf(layout.angles[2], layout, layout.sections[2]),
    },
  };
}

/** 点 (x,y) に最も近い strip 番号(1 始まり、section 0 の線)。 */
export function stripAt(
  geometry: StripGeometryMessage,
  layout: Layout,
  plane: number,
  x: number,
  y: number,
): number {
  const unit = [geometry.planes.U, geometry.planes.V, geometry.planes.W][plane].unit;
  return Math.round((-unit[1] * x + unit[0] * y) / layout.d + (layout.n + 1) / 2);
}

export const BASE = 100;
/** baseline(100)を 500 ADC 超え → 既定閾値 50 を十分超える。 */
export const PEAK = 600;

/** 全セル BASE の 3 面(U/V/W)。 */
export function emptyUvw(
  layout: Layout,
  nBuckets: number,
  eventNumber = 1,
  incomplete = false,
): UvwMessage[] {
  return [0, 1, 2].map((plane) => ({
    kind: 'uvw' as const,
    header: { msgType: 2, incomplete, runNumber: 7, eventNumber },
    plane,
    nStrips: layout.n,
    nBuckets,
    adc: new Uint16Array(layout.n * nBuckets).fill(BASE),
  }));
}

export function setHit(message: UvwMessage, strip: number, bucket: number, adc = PEAK): void {
  if (strip < 1 || strip > message.nStrips) return;
  message.adc[(strip - 1) * message.nBuckets + bucket] = adc;
}

export type TrackFn = (bucket: number) => [number, number];

export function addTrack(
  uvw: UvwMessage[],
  geometry: StripGeometryMessage,
  layout: Layout,
  track: TrackFn,
  from: number,
  to: number,
  /** 中心の strip の両側に足す本数(幅のある飛跡)。 */
  halfWidth = 0,
): void {
  for (let bucket = from; bucket <= to; bucket++) {
    const [x, y] = track(bucket);
    for (let plane = 0; plane < 3; plane++) {
      const center = stripAt(geometry, layout, plane, x, y);
      for (let k = -halfWidth; k <= halfWidth; k++) setHit(uvw[plane], center + k, bucket);
    }
  }
}
