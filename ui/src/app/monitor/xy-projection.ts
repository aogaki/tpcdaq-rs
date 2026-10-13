/**
 * Event Display の XY / XZ / YZ 投影(TODO/125、**表示専用**)。
 *
 * Angular / jsroot を import しない純ロジック。UVW の 3 面の 0x02(生 ADC)と
 * `stripGeometry`(mm の線分)から、TPCReco の `EventTPC::Get2D`(method 0)と同じ手順で
 * 「同じ time bucket の U/V/W 三面一致」の点を 3 つの 2D ヒストに詰める。
 *
 * 移植元(reference/TPCReco、読み取りのみ):
 * - `GeometryTPC::GetCrossPoint` / `MatchCrossPoint`(HIGS2026_online の GeometryTPC.cpp)
 * - `EventTPC::Get2D`(tpcreco-v0.09 の EventTPC.cpp)
 *
 * 保存系・ワイヤ(生 ADC)には一切触らない。ベースラインは Event Display と同じ
 * 「先頭 25 cell 平均」(`stripBaseline`)を引くだけ。ch 数・strip 数・ビン数は全部メッセージ由来。
 *
 * 既知の差(意図的、KISS): TPCReco はクラスタ(seed + 近傍)からヒットを取るが、
 * こちらは**閾値を超えたセルだけ**。
 */
import type { StripGeometryMessage } from '../ws/json';
import type { UvwMessage } from '../ws/wire';
import { stripBaseline } from './root-histo';

/** TPCReco `Utils::NUMERICAL_TOLERANCE`(UtilsMath.h:11)。 */
export const NUMERICAL_TOLERANCE = 1e-5;

/** 三面一致の許容半径 [mm]。TPCReco v0.09 `HistoManager::get2DReconstruction` の `radius = 2.0`。固定。 */
export const PROJECTION_RADIUS_MM = 2.0;

/** 既定の閾値 [ADC](baseline 超過)。TPCReco GUI `config_DotFinder_NGRAW.json` の `hitThr`。 */
export const DEFAULT_HIT_THRESHOLD = 50;

/** `crossPoint` の呼び出し回数の安全弁。2×10⁷ だとメインスレッドが 1〜2 秒止まる(Fable 見積もり)。 */
export const MAX_CROSS_EVALUATIONS = 2_000_000;

/** W 候補の絞り込みに足す余裕 [mm](浮動小数の丸め用。最終判定は `matchCrossPoint`)。 */
const PRUNE_MARGIN = 1e-9;

export type Vec2 = readonly [number, number];

/** `crossPoint` が読む線分の最小形。 */
export interface SegmentLike {
  readonly x0: number;
  readonly y0: number;
  readonly length: number;
}

/**
 * 2 本の線分の交点(TPCReco `GetCrossPoint`)。平行・有効範囲外なら `null`。
 * 有効範囲 = (x0,y0) から unit 方向に `[-0.5*padPitch - tol, length + tol]`。
 * `x0,y0` は reference point 込み(平行移動なので式は変わらない)。
 */
export function crossPoint(
  a: SegmentLike,
  b: SegmentLike,
  unitA: Vec2,
  unitB: Vec2,
  padPitch: number,
): [number, number] | null {
  const w = -unitA[0] * unitB[1] + unitA[1] * unitB[0];
  if (Math.abs(w) < NUMERICAL_TOLERANCE) return null;
  const dx = b.x0 - a.x0;
  const dy = b.y0 - a.y0;
  const w1 = -dx * unitB[1] + dy * unitB[0];
  const w2 = unitA[0] * dy - unitA[1] * dx;
  const len1 = w1 / w;
  const len2 = w2 / w;
  const residual = 0.5 * padPitch;
  if (
    len1 < -residual - NUMERICAL_TOLERANCE ||
    len2 < -residual - NUMERICAL_TOLERANCE ||
    len1 > a.length + NUMERICAL_TOLERANCE ||
    len2 > b.length + NUMERICAL_TOLERANCE
  ) {
    return null;
  }
  return [a.x0 + len1 * unitA[0], a.y0 + len1 * unitA[1]];
}

/**
 * 3 本の線分が radius [mm] 以内で交わるか(TPCReco `MatchCrossPoint`)。
 * 3 つの交点(1,2)(2,3)(3,1)が互いに radius 以内なら、その重心を返す。
 */
export function matchCrossPoint(
  s1: SegmentLike,
  s2: SegmentLike,
  s3: SegmentLike,
  unit1: Vec2,
  unit2: Vec2,
  unit3: Vec2,
  padPitch: number,
  radius: number,
): [number, number] | null {
  const p0 = crossPoint(s1, s2, unit1, unit2, padPitch);
  if (p0 === null) return null;
  const p1 = crossPoint(s2, s3, unit2, unit3, padPitch);
  if (p1 === null) return null;
  const p2 = crossPoint(s3, s1, unit3, unit1, padPitch);
  if (p2 === null) return null;
  const r2 = radius * radius;
  if (dist2(p0, p1) > r2 || dist2(p1, p2) > r2 || dist2(p2, p0) > r2) {
    return null;
  }
  return [(p0[0] + p1[0] + p2[0]) / 3, (p0[1] + p1[1] + p2[1]) / 3];
}

function dist2(a: readonly [number, number], b: readonly [number, number]): number {
  const dx = a[0] - b[0];
  const dy = a[1] - b[1];
  return dx * dx + dy * dy;
}

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

// ---------------------------------------------------------------------------------------------
// 線分の索引(ジオメトリ 1 つにつき 1 回)
// ---------------------------------------------------------------------------------------------

interface IndexedSegment extends SegmentLike {
  readonly strip: number;
  readonly section: number;
  /** 線の法線 n = (-uy, ux) と c = n·(x0,y0)。W 候補の絞り込み用。 */
  readonly nx: number;
  readonly ny: number;
  readonly c: number;
}

interface PlaneIndex {
  readonly unit: Vec2;
  /** strip 番号 → その strip の全 section の線分(並びは配信順 = section 昇順)。 */
  readonly byStrip: Map<number, IndexedSegment[]>;
}

type GeometryIndex = readonly [PlaneIndex, PlaneIndex, PlaneIndex];

const indexCache = new WeakMap<StripGeometryMessage, GeometryIndex>();

function buildPlaneIndex(plane: StripGeometryMessage['planes']['U']): PlaneIndex {
  const [ux, uy] = plane.unit;
  const nx = -uy;
  const ny = ux;
  const byStrip = new Map<number, IndexedSegment[]>();
  for (const [strip, section, x0, y0, length] of plane.segments) {
    const segment: IndexedSegment = {
      strip,
      section,
      x0,
      y0,
      length,
      nx,
      ny,
      c: nx * x0 + ny * y0,
    };
    const list = byStrip.get(strip);
    if (list) list.push(segment);
    else byStrip.set(strip, [segment]);
  }
  return { unit: plane.unit, byStrip };
}

function indexOf(geometry: StripGeometryMessage): GeometryIndex {
  let index = indexCache.get(geometry);
  if (!index) {
    index = [
      buildPlaneIndex(geometry.planes.U),
      buildPlaneIndex(geometry.planes.V),
      buildPlaneIndex(geometry.planes.W),
    ];
    indexCache.set(geometry, index);
  }
  return index;
}

// ---------------------------------------------------------------------------------------------
// 投影
// ---------------------------------------------------------------------------------------------

export interface ProjectionOptions {
  /** ベースライン超過 [ADC]。これを超えたセルがヒット。 */
  readonly threshold: number;
  /** 三面一致の許容半径 [mm]。 */
  readonly radius: number;
  /** `crossPoint` 呼び出し回数の上限(既定 `MAX_CROSS_EVALUATIONS`)。 */
  readonly maxEvaluations?: number;
  /** 詰めた点ごとの通知(テスト・診断用)。 */
  readonly onPoint?: (x: number, y: number, bucket: number, value: number) => void;
}

export interface ProjectionResult extends Projections {
  /** 三面一致の点の数((u,v,w) の組、重複は最初の一致だけ)。 */
  readonly points: number;
  /** `crossPoint` の呼び出し回数(U×V の探索 + `matchCrossPoint` 内の 3 回)。 */
  readonly evaluations: number;
  /** 上限を超えて打ち切った(結果は途中まで)。 */
  readonly capped: boolean;
  /** 3 枚のどれかのヒスト範囲の外に落ちて詰められなかった点の数。 */
  readonly outOfRange: number;
  /** どれかの面の 0x02 が incomplete(fragment 欠落で合算値が落ちている)。 */
  readonly incomplete: boolean;
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

/** 点の組の一意キー(u,v,w の strip 番号)。strip 番号は数千まで → 倍精度で衝突しない。 */
function tripletKey(u: number, v: number, w: number): number {
  return (u * 100_000 + v) * 100_000 + w;
}

function bump(map: Map<number, number>, strip: number): void {
  map.set(strip, (map.get(strip) ?? 0) + 1);
}

/**
 * 1 イベントを XY / XZ / YZ に投影する。計算できない状態(`projectionBlocker` ≠ null)は `null`。
 *
 * 全 time bucket について、3 面ともヒットのある bucket で U×V の各線分の組の交点 p0 を求め、
 * W は「p0 と W の直線の距離 ≤ radius」の線分だけ `matchCrossPoint` にかける
 * (V×W の交点は W の直線上にあり、|p0 - p_VW| ≤ radius なら必要条件。結果は総当たりと同じ)。
 * 電荷は method 0: `q_u/n_u + q_v/n_v + q_w/n_w`(n は各 strip の一致数。TPCReco と同じく
 * 重複チェックの**前**に数える)。
 */
export function projectEvent(
  uvw: readonly (UvwMessage | null)[],
  geometry: StripGeometryMessage,
  options: ProjectionOptions,
): ProjectionResult | null {
  if (projectionBlocker(uvw, geometry) !== null) return null;
  const messages = uvw as readonly [UvwMessage, UvwMessage, UvwMessage];
  const nBuckets = messages[0].nBuckets;
  const { threshold, radius } = options;
  const maxEvaluations = options.maxEvaluations ?? MAX_CROSS_EVALUATIONS;
  const padPitch = geometry.padPitch;
  const [indexU, indexV, indexW] = indexOf(geometry);
  const projections = createProjections(geometry, nBuckets);

  // 面毎・bucket 毎のヒット strip(strip 昇順)と、ベースライン。
  const baselines: Float64Array[] = [];
  const hits: number[][][] = [];
  for (const message of messages) {
    const base = new Float64Array(message.nStrips);
    const perBucket: number[][] = Array.from({ length: nBuckets }, () => []);
    for (let s = 0; s < message.nStrips; s++) {
      const offset = s * nBuckets;
      const baseline = stripBaseline(message.adc, offset, nBuckets);
      base[s] = baseline;
      for (let b = 0; b < nBuckets; b++) {
        if (message.adc[offset + b] - baseline > threshold) perBucket[b].push(s + 1);
      }
    }
    baselines.push(base);
    hits.push(perBucket);
  }
  const charge = (plane: number, strip: number, bucket: number): number =>
    messages[plane].adc[(strip - 1) * nBuckets + bucket] - baselines[plane][strip - 1];

  let points = 0;
  let evaluations = 0;
  let outOfRange = 0;
  let capped = false;

  for (let bucket = 0; bucket < nBuckets && !capped; bucket++) {
    const hitsU = hits[0][bucket];
    const hitsV = hits[1][bucket];
    const hitsW = hits[2][bucket];
    if (hitsU.length === 0 || hitsV.length === 0 || hitsW.length === 0) continue;

    // W 側の候補(この bucket でヒットした strip の全 section)。
    const candidatesW: IndexedSegment[] = [];
    for (const strip of hitsW) {
      const list = indexW.byStrip.get(strip);
      if (list) candidatesW.push(...list);
    }
    if (candidatesW.length === 0) continue;

    const nMatch: [Map<number, number>, Map<number, number>, Map<number, number>] = [
      new Map(),
      new Map(),
      new Map(),
    ];
    // 最初の一致だけ採る。挿入順 = 走査順(総当たりと同じ)。
    const accepted = new Map<number, [number, number, number, number, number]>();

    search: for (const u of hitsU) {
      for (const segU of indexU.byStrip.get(u) ?? []) {
        for (const v of hitsV) {
          for (const segV of indexV.byStrip.get(v) ?? []) {
            if (++evaluations > maxEvaluations) {
              capped = true;
              break search;
            }
            const p0 = crossPoint(segU, segV, indexU.unit, indexV.unit, padPitch);
            if (p0 === null) continue;
            for (const segW of candidatesW) {
              if (Math.abs(segW.nx * p0[0] + segW.ny * p0[1] - segW.c) > radius + PRUNE_MARGIN) {
                continue;
              }
              evaluations += 3;
              const position = matchCrossPoint(
                segU,
                segV,
                segW,
                indexU.unit,
                indexV.unit,
                indexW.unit,
                padPitch,
                radius,
              );
              if (position === null) continue;
              bump(nMatch[0], u);
              bump(nMatch[1], v);
              bump(nMatch[2], segW.strip);
              const key = tripletKey(u, v, segW.strip);
              if (!accepted.has(key)) {
                accepted.set(key, [u, v, segW.strip, position[0], position[1]]);
              }
            }
          }
        }
      }
    }

    for (const [u, v, w, x, y] of accepted.values()) {
      const value =
        charge(0, u, bucket) / (nMatch[0].get(u) ?? 1) +
        charge(1, v, bucket) / (nMatch[1].get(v) ?? 1) +
        charge(2, w, bucket) / (nMatch[2].get(w) ?? 1);
      points++;
      options.onPoint?.(x, y, bucket, value);
      if (!fillPoint(projections, x, y, bucket, value)) outOfRange++;
    }
  }

  return {
    ...projections,
    points,
    evaluations,
    capped,
    outOfRange,
    incomplete: messages.some((message) => message.header.incomplete),
  };
}
