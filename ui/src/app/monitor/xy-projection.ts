/**
 * Event Display の XY / XZ / YZ ヒスト(TODO/125、**表示専用**)— ビンと詰め方だけ。
 *
 * Angular / jsroot を import しない純ロジック。何を詰めるか(直線トラックのヒット)は
 * `track-fit.ts`(macros/analyzeUVW.C + makeTracks.C の移植 — 125 改訂 2)。
 * ビン: X 幅 = stripPitch、Y 幅 = padPitch、範囲 = `stripGeometry.rangeXY`(TPCReco と同じ本数)、
 * Z = time bucket そのまま。ch 数・strip 数・ビン数は全部メッセージ由来。
 */
import type { StripGeometryMessage } from '../ws/json';
import type { UvwMessage } from '../ws/wire';

// ---------------------------------------------------------------------------------------------
// 2D ヒスト
// ---------------------------------------------------------------------------------------------

/** 2D ヒスト 1 枚。`values[iy*nx + ix]`(0x11 Histo2d と同じ iy 外側 row-major)。 */
export interface Projection2d {
  readonly nx: number;
  readonly ny: number;
  readonly xmin: number;
  readonly xmax: number;
  readonly ymin: number;
  readonly ymax: number;
  readonly values: Float64Array;
}

export interface Projections {
  /** 横 X [mm] × 縦 Y [mm] */
  readonly xy: Projection2d;
  /** 横 X [mm] × 縦 time bucket */
  readonly xz: Projection2d;
  /** 横 Y [mm] × 縦 time bucket */
  readonly yz: Projection2d;
}

function makeProjection(
  nx: number,
  ny: number,
  xmin: number,
  xmax: number,
  ymin: number,
  ymax: number,
): Projection2d {
  return { nx, ny, xmin, xmax, ymin, ymax, values: new Float64Array(nx * ny) };
}

/**
 * ビンの本数(TPCReco と同じ `(int)((max-min)/pitch - 1)`)。X 幅 = stripPitch、Y 幅 = padPitch。
 * Z は time bucket そのまま(nBuckets 本、-0.5〜nBuckets-0.5)。
 */
export function projectionBins(geometry: StripGeometryMessage): { nx: number; ny: number } {
  const [xmin, xmax, ymin, ymax] = geometry.rangeXY;
  return {
    nx: Math.trunc((xmax - xmin) / geometry.stripPitch - 1),
    ny: Math.trunc((ymax - ymin) / geometry.padPitch - 1),
  };
}

/** 空の XY / XZ / YZ を作る。 */
export function createProjections(geometry: StripGeometryMessage, nBuckets: number): Projections {
  const { nx, ny } = projectionBins(geometry);
  const [xmin, xmax, ymin, ymax] = geometry.rangeXY;
  const zmin = -0.5;
  const zmax = nBuckets - 0.5;
  return {
    xy: makeProjection(nx, ny, xmin, xmax, ymin, ymax),
    xz: makeProjection(nx, nBuckets, xmin, xmax, zmin, zmax),
    yz: makeProjection(ny, nBuckets, ymin, ymax, zmin, zmax),
  };
}

/** ビン index(ROOT の Fill と同じ: 上端ちょうどは overflow = 範囲外)。範囲外は -1。 */
function binIndex(value: number, min: number, max: number, n: number): number {
  const index = Math.floor(((value - min) / (max - min)) * n);
  return index >= 0 && index < n ? index : -1;
}

function fillBin(h: Projection2d, x: number, y: number, value: number): boolean {
  const ix = binIndex(x, h.xmin, h.xmax, h.nx);
  const iy = binIndex(y, h.ymin, h.ymax, h.ny);
  if (ix < 0 || iy < 0) return false;
  h.values[iy * h.nx + ix] += value;
  return true;
}

/**
 * 1 点を 3 枚に詰める(XY ← (x,y)、XZ ← (x,bucket)、YZ ← (y,bucket))。
 * 3 枚とも範囲内なら `true`、1 枚でも範囲外なら `false`(範囲外の枚だけ詰めない)。
 */
export function fillPoint(
  projections: Projections,
  x: number,
  y: number,
  bucket: number,
  value: number,
): boolean {
  const a = fillBin(projections.xy, x, y, value);
  const b = fillBin(projections.xz, x, bucket, value);
  const c = fillBin(projections.yz, y, bucket, value);
  return a && b && c;
}

/**
 * 計算できない理由(無ければ `null`)。呼び手(画面)はこれをそのまま文にする。
 * 3 面が揃い、同じイベントで、bucket 数が一致し、ヒストの本数が 1 以上であること。
 */
export function projectionBlocker(
  uvw: readonly (UvwMessage | null)[],
  geometry: StripGeometryMessage,
): string | null {
  const missing = ['U', 'V', 'W'].filter((_, plane) => !uvw[plane]);
  if (missing.length > 0) return `waiting for ${missing.join('/')} (0x02)`;
  const [u, v, w] = uvw as readonly [UvwMessage, UvwMessage, UvwMessage];
  const events = [u, v, w].map((message) => message.header.eventNumber);
  if (events[0] !== events[1] || events[1] !== events[2]) {
    return `U/V/W are from different events (#${events.join(' / #')})`;
  }
  if (u.nBuckets !== v.nBuckets || v.nBuckets !== w.nBuckets) {
    return `U/V/W bucket counts differ (${u.nBuckets} / ${v.nBuckets} / ${w.nBuckets})`;
  }
  const { nx, ny } = projectionBins(geometry);
  if (!(nx >= 1) || !(ny >= 1) || u.nBuckets < 1) {
    return `degenerate projection bins (nx=${nx}, ny=${ny}, buckets=${u.nBuckets})`;
  }
  return null;
}
