/**
 * TODO/125 改訂 2 のテスト用の合成ジオメトリ / 合成 0x02(spec からだけ import する。本番コードは使わない)。
 *
 * ジオメトリ: 面 p の strip 方向 = (cos θ_p, sin θ_p)、pitch 方向は TPCReco `GeometryTPC::Load` の式
 * (pitch_U = -(W+V).Unit()、pitch_V = (U+W).Unit()、pitch_W = (V-U).Unit())。strip k の中心線は
 * pitch 座標 s_k = (k - (n+1)/2) * pitch を通る長さ `length` の線分(中央が s_k·pitchUnit)。
 * 六角配置(mini 90/-30/30、ELITPC 0/-120/-60)では pitch ⟂ strip なので `pitchUnit·(x0,y0) = s_k`。
 *
 * 0x02: 全セル BASE の上に、直線トラックが横切る strip ごとにガウス形のパルスを 1 つ足す。
 * パルスの中心 bucket はトラックの xy 位置から線形に決まる(z の傾きを持つ)。
 */
import type { StripGeometryMessage, StripPlaneGeometry } from '../ws/json';
import type { UvwMessage } from '../ws/wire';

export type Vec2 = readonly [number, number];

/** mini eTPC の strip 角度(U/V/W、度)。 */
export const MINI_ANGLES = [90, -30, 30] as const;
/** ELITPC の strip 角度(U/V/W、度)。 */
export const ELITPC_ANGLES = [0, -120, -60] as const;

export interface Layout {
  readonly angles: readonly [number, number, number];
  /** 面ごとの strip 本数(奇数 → 中央の strip が s = 0)。 */
  readonly n: number;
  /** strip の間隔 [mm]。 */
  readonly pitch: number;
  /** 線分の長さ [mm]。 */
  readonly length: number;
}

/** ±100 mm を覆う 135 本 × 1.5 mm。 */
export function layoutOf(angles: readonly [number, number, number]): Layout {
  return { angles, n: 135, pitch: 1.5, length: 300 };
}

function unitOf(deg: number): Vec2 {
  const a = (deg * Math.PI) / 180;
  return [Math.cos(a), Math.sin(a)];
}

function normalized(x: number, y: number): Vec2 {
  const norm = Math.hypot(x, y);
  return [x / norm, y / norm];
}

/** TPCReco `GeometryTPC::Load`(GeometryTPC.cpp:271-283)と同じ式の pitch 単位ベクトル。 */
export function pitchUnits(angles: readonly [number, number, number]): [Vec2, Vec2, Vec2] {
  const [u, v, w] = angles.map(unitOf);
  const pu = normalized(w[0] + v[0], w[1] + v[1]);
  return [
    [-pu[0], -pu[1]],
    normalized(u[0] + w[0], u[1] + w[1]),
    normalized(v[0] - u[0], v[1] - u[1]),
  ];
}

/** strip k(1 始まり)の pitch 座標 [mm]。 */
export function stripS(layout: Layout, strip: number): number {
  return (strip - (layout.n + 1) / 2) * layout.pitch;
}

export function syntheticGeometry(layout: Layout): StripGeometryMessage {
  const pitch = pitchUnits(layout.angles);
  const planes = layout.angles.map((deg, p): StripPlaneGeometry => {
    const unit = unitOf(deg);
    const segments: [number, number, number, number, number][] = [];
    for (let strip = 1; strip <= layout.n; strip++) {
      const s = stripS(layout, strip);
      segments.push([
        strip,
        0,
        s * pitch[p][0] - (unit[0] * layout.length) / 2,
        s * pitch[p][1] - (unit[1] * layout.length) / 2,
        layout.length,
      ]);
    }
    return { unit, pitchUnit: pitch[p], segments };
  });
  const extent = (layout.n * layout.pitch) / 2;
  return {
    type: 'stripGeometry',
    padPitch: layout.pitch * (2 / Math.sqrt(3)),
    stripPitch: layout.pitch,
    rangeXY: [-extent, extent, -extent, extent],
    planes: { U: planes[0], V: planes[1], W: planes[2] },
  };
}

/** 全セルの値(ベースライン)。 */
export const BASE = 250;

/** 全セル BASE の 3 面(U/V/W)。 */
export function emptyEvent(
  layout: Layout,
  nBuckets = 512,
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

/**
 * strip 1 本にガウス形のパルスを足す(中心 `center` [bucket]、σ [cell]、高さ `amplitude` [ADC])。
 * ±6σ の外は足さない。値は 4095 で止める。
 */
export function addPulse(
  message: UvwMessage,
  strip: number,
  center: number,
  amplitude: number,
  sigma = 2,
): void {
  if (strip < 1 || strip > message.nStrips) return;
  const offset = (strip - 1) * message.nBuckets;
  const lo = Math.max(0, Math.ceil(center - 6 * sigma));
  const hi = Math.min(message.nBuckets - 1, Math.floor(center + 6 * sigma));
  for (let t = lo; t <= hi; t++) {
    const add = Math.round(amplitude * Math.exp(-0.5 * ((t - center) / sigma) ** 2));
    message.adc[offset + t] = Math.min(4095, message.adc[offset + t] + add);
  }
}

/** 直線トラック: (x, y) [mm] と bucket が始点 a から終点 b まで線形に変わる。 */
export interface Track {
  readonly a: readonly [x: number, y: number, bucket: number];
  readonly b: readonly [x: number, y: number, bucket: number];
}

/**
 * 一様な dE/dx の直線トラック: strip k は自分の帯 [s_k − pitch/2, s_k + pitch/2] に入るトラックの部分の
 * 電荷を集める(端の strip は一部だけ)。パルスの高さ = `amplitude` × (その部分の xy 長さ / pitch)、
 * 中心 bucket = その部分の中点の時刻。xy 長さ = pitch 座標の重なり / |pitch·d_xy|。
 * 帯を丸ごと横切る |pitch·d_xy| = 1 の strip: 高さ 1100 ADC・σ 2 cell → 積分電荷 ≈ 1100·2·√(2π) ≈ 5500 ADC
 * (macros/analyzeUVW.C の注記「track strips peak at ~5500」に合わせる。端の strip は一部だけなので
 * halo cut 2000 で落ちることがある — 実データと同じ)。
 * トラックが strip と平行な面(|pitch·d_xy| = 0)は扱わない(テストで使わない)。
 */
export function addTrack(
  uvw: readonly UvwMessage[],
  layout: Layout,
  track: Track,
  amplitude = 1100,
): void {
  const pitch = pitchUnits(layout.angles);
  const dx = track.b[0] - track.a[0];
  const dy = track.b[1] - track.a[1];
  const xyLength = Math.hypot(dx, dy);
  for (let plane = 0; plane < 3; plane++) {
    const [px, py] = pitch[plane];
    const sA = px * track.a[0] + py * track.a[1];
    const sB = px * track.b[0] + py * track.b[1];
    const proj = Math.abs(sB - sA) / xyLength; // |pitch·d_xy|
    for (let strip = 1; strip <= layout.n; strip++) {
      const s = stripS(layout, strip);
      const lo = Math.max(Math.min(sA, sB), s - layout.pitch / 2);
      const hi = Math.min(Math.max(sA, sB), s + layout.pitch / 2);
      if (!(hi > lo)) continue;
      const f = ((lo + hi) / 2 - sA) / (sB - sA);
      const height = (amplitude * (hi - lo)) / proj / layout.pitch;
      addPulse(uvw[plane], strip, track.a[2] + f * (track.b[2] - track.a[2]), height);
    }
  }
}

/** 全 strip に小さなパルス(halo — 積分電荷 ≈ 100·2·√(2π) ≈ 500 < 2000)を足す。 */
export function addHalo(uvw: readonly UvwMessage[], center: number, amplitude = 100): void {
  for (const message of uvw) {
    for (let strip = 1; strip <= message.nStrips; strip++) {
      addPulse(message, strip, center, amplitude);
    }
  }
}
