/**
 * Event Display の直線トラック(TODO/125 改訂 2、**表示専用**)。
 *
 * ユーザーの解析マクロ `macros/analyzeUVW.C`(0〜4)と `macros/makeTracks.C`(1〜5)の移植。
 * **1 イベント = 直線 1 本**。Angular / jsroot を import しない純ロジック。
 * 保存系・ワイヤ(生 ADC)には一切触らない。ch 数・strip 数・bucket 数は全部メッセージ由来。
 *
 * macros からの一般化(発注書 125 改訂 2 のとおり。それ以外は macros と同じ式・同じ順):
 * - strip 位置 s は channel_map.csv(mini 専用)ではなく `stripGeometry` から
 *   `s = pitchUnit·(x0, y0)`(その strip の section 番号が最小の線分 — TPCReco Strip2posUVW と同じ)。
 *   pitch ベクトルも macros の PITCH 定数ではなく `stripGeometry` の `pitchUnit`。
 * - makeTracks 2 の閉じた式(mini の pitch 専用)は `var_v = p_vᵀ S p_v` の 3×3 連立に置き換える
 *   (mini の pitch では閉じた式と同じ — spec で確認)。
 * - 0x02 は section を合算した strip × bucket なので、1 strip = analyzeUVW の「1 ch」。
 * - z [mm] は macros の既定値(1200 V / 20 cm / 100 mbar、25 MHz)で、帯選択と z フィットの中だけで使う。
 *   画面の Z は time bucket(`StripHit.t`)。
 * - makeTracks 6(頭尻の向き付け)は移植しない — 頭尻は決めない(macros の WARNING と同じ)。
 */
import type { StripGeometryMessage } from '../ws/json';
import type { UvwMessage } from '../ws/wire';
import { type Projections, createProjections, fillPoint, projectionBlocker } from './xy-projection';

// ---- analyzeUVW.C の定数 --------------------------------------------------------------------
export const ADC_THRESHOLD = 35;
export const PED_LO = 1;
export const PED_HI = 20;
export const PEAK_HALF_CELLS = 5;
export const BAND_MM = 10.0;
/** analyzeUVW.C の MIN_VIEW_HITS(帯フィットに要る strip ヒット数)。 */
export const MIN_VIEW_HITS_BAND = 4;
export const STRIP_Q_MIN = 2000.0;
const DRIFT_HV_V = 1200.0;
const DRIFT_LEN_CM = 20.0;
const PRESSURE_MBAR = 100.0;
const SAMPLING_MHZ = 25.0;
/** analyzeUVW.C `driftVelocityCmUs` の CO2 移動度 [(cm/µs) per (V/cm/mbar)]。 */
const MOBILITY = 0.7233;
/** 1 time bucket の z [mm](macros の既定値、式も同じ順)= 0.173592。 */
export const MM_PER_CELL =
  ((MOBILITY * (DRIFT_HV_V / DRIFT_LEN_CM / PRESSURE_MBAR)) / SAMPLING_MHZ) * 10.0;

// ---- makeTracks.C の定数 --------------------------------------------------------------------
export const QUANTILE = 0.02;
/** makeTracks.C の MIN_VIEW_HITS(各面に要る帯内の strip ヒット数)。 */
export const MIN_VIEW_HITS_TRACK = 3;
export const MIN_EIGEN_GAP_MM2 = 1.0;
export const MIN_PROJ = 0.3;

const VIEW_NAMES = ['U', 'V', 'W'] as const;

export type Vec2 = readonly [number, number];

/** strip 1 本のヒット(analyzeUVW の slView / slZ / slS / slQ の 1 組 + 表示用の strip 番号と時刻)。 */
export interface StripHit {
  /** 0=U, 1=V, 2=W */
  readonly view: number;
  /** 1 始まり */
  readonly strip: number;
  /** strip 位置 [mm](pitch 座標) */
  readonly s: number;
  /** 電荷加重のピーク時刻 [time bucket] */
  readonly t: number;
  /** ドリフト z [mm] = t × MM_PER_CELL */
  readonly z: number;
  /** ピーク窓の積分電荷 [ADC] */
  readonly q: number;
}

/** 表示用の点: トラック上の (x, y) と、そのヒットの時刻・電荷。 */
export interface TrackPoint {
  readonly x: number;
  readonly y: number;
  readonly t: number;
  readonly q: number;
}

export interface TrackOk {
  readonly ok: true;
  /** makeTracks の phi [度](頭尻を決めないので 180° の不定性がある)。 */
  readonly phiDeg: number;
  /** 単位ベクトル (dx, dy, dz)(z は mm)。符号は決めない。 */
  readonly dir: readonly [number, number, number];
  /** 2 % / 98 % の電荷分位点の位置 (x, y, z[mm])。順序は決めない(start = l が小さい側)。 */
  readonly start: readonly [number, number, number];
  readonly end: readonly [number, number, number];
  /** 最も外側のヒット(makeTracks の *F)。 */
  readonly startF: readonly [number, number, number];
  readonly endF: readonly [number, number, number];
  /** 3D 長さ [mm](2–98 %)と最も外側どうし。 */
  readonly length: number;
  readonly lengthF: number;
  /** xy の長さ [mm](2–98 %)。ドリフト速度に依らない。 */
  readonly xyLength: number;
  /** 3 面の帯内ヒットの電荷の和(makeTracks の charge)。 */
  readonly charge: number;
  readonly points: readonly TrackPoint[];
}

export interface TrackNone {
  readonly ok: false;
  readonly reason: string;
}

export type TrackFit = TrackOk | TrackNone;

// ---------------------------------------------------------------------------------------------
// analyzeUVW.C 0: 1 strip 1 ヒット
// ---------------------------------------------------------------------------------------------

/**
 * 面ごとの strip → s [mm]。s = pitchUnit·(x0, y0)、同じ strip の section が複数あれば番号の最小。
 * 並びは U, V, W。
 */
export function stripPositions(
  geometry: StripGeometryMessage,
): [Map<number, number>, Map<number, number>, Map<number, number>] {
  const planes = [geometry.planes.U, geometry.planes.V, geometry.planes.W];
  const [u, v, w] = planes.map((plane) => {
    const [px, py] = plane.pitchUnit;
    const section = new Map<number, number>();
    const positions = new Map<number, number>();
    for (const [strip, sec, x0, y0] of plane.segments) {
      const seen = section.get(strip);
      if (seen !== undefined && seen <= sec) continue;
      section.set(strip, sec);
      positions.set(strip, px * x0 + py * y0);
    }
    return positions;
  });
  return [u, v, w];
}

/**
 * analyzeUVW.C 0 を 1 面に: ペデスタル = cell PED_LO..PED_HI の平均、ピーク = 最初の最大、
 * ピーク − ペデスタル > ADC_THRESHOLD なら ±PEAK_HALF_CELLS の閾値超えのセルで q と電荷加重時刻、
 * q ≥ STRIP_Q_MIN だけ残す(halo cut)。strip 順。
 * s が無い strip(`stripGeometry` に線分が無い)のヒットは `unplaced` に数えて外す。
 */
export function stripHits(
  message: UvwMessage,
  positions: ReadonlyMap<number, number>,
): { hits: StripHit[]; unplaced: number } {
  const { adc, nBuckets, nStrips, plane } = message;
  const last = nBuckets - 1;
  const hits: StripHit[] = [];
  let unplaced = 0;
  for (let strip = 1; strip <= nStrips; strip++) {
    const offset = (strip - 1) * nBuckets;
    let ped = 0;
    for (let t = PED_LO; t <= PED_HI; t++) ped += adc[offset + t];
    ped /= PED_HI - PED_LO + 1;
    let tPeak = 0;
    for (let t = 1; t < nBuckets; t++) if (adc[offset + t] > adc[offset + tPeak]) tPeak = t;
    if (adc[offset + tPeak] - ped <= ADC_THRESHOLD) continue;
    const tLo = Math.max(0, tPeak - PEAK_HALF_CELLS);
    const tHi = Math.min(last, tPeak + PEAK_HALF_CELLS);
    let q = 0;
    let qt = 0;
    for (let t = tLo; t <= tHi; t++) {
      const val = adc[offset + t] - ped;
      if (val <= ADC_THRESHOLD) continue;
      q += val;
      qt += val * t;
    }
    if (q < STRIP_Q_MIN) continue; // halo strip
    const s = positions.get(strip);
    if (s === undefined) {
      unplaced++;
      continue;
    }
    const t = qt / q;
    hits.push({ view: plane, strip, s, t, z: t * MM_PER_CELL, q });
  }
  return { hits, unplaced };
}

// ---------------------------------------------------------------------------------------------
// analyzeUVW.C 1–4: 面ごとの電荷加重 PCA の直線と帯選択
// ---------------------------------------------------------------------------------------------

/** analyzeUVW.C 1–4。ヒットが MIN_VIEW_HITS_BAND 本未満の面は空を返す(帯フィットをしない)。 */
export function bandSelect(hits: readonly StripHit[]): StripHit[] {
  const n = hits.length;
  if (n < MIN_VIEW_HITS_BAND) return [];

  // 1. 電荷加重の PCA 直線
  let sw = 0;
  let mz = 0;
  let ms = 0;
  for (const h of hits) {
    sw += h.q;
    mz += h.q * h.z;
    ms += h.q * h.s;
  }
  mz /= sw; // sw > 0: 全ヒット q >= STRIP_Q_MIN
  ms /= sw;
  let czz = 0;
  let css = 0;
  let czs = 0;
  for (const h of hits) {
    const dz = h.z - mz;
    const ds = h.s - ms;
    czz += h.q * dz * dz;
    css += h.q * ds * ds;
    czs += h.q * dz * ds;
  }
  // 2×2 共分散の主軸(最大固有値)
  const tr = czz + css;
  const det = czz * css - czs * czs;
  const lam = 0.5 * tr + Math.sqrt(Math.max(0.25 * tr * tr - det, 0.0));
  let dz0 = czs;
  let ds0 = lam - czz;
  let nn = Math.hypot(dz0, ds0);
  if (nn < 1e-9) {
    dz0 = 1;
    ds0 = 0;
    nn = 1;
  }
  dz0 /= nn;
  ds0 /= nn;

  // 2.+3.+4. 直線から ±BAND_MM の帯に入るヒットを残す
  const kept: StripHit[] = [];
  for (const h of hits) {
    const perp = -(h.z - mz) * ds0 + (h.s - ms) * dz0;
    if (Math.abs(perp) > BAND_MM) continue;
    kept.push(h);
  }
  return kept;
}

// ---------------------------------------------------------------------------------------------
// makeTracks.C 1–5
// ---------------------------------------------------------------------------------------------

function det3(m: readonly (readonly number[])[]): number {
  return (
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) -
    m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) +
    m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
  );
}

/** 3×3 連立の行列式がこれ未満なら pitch が退化しているとみなす。 */
const MIN_PITCH_DET = 1e-9;

/**
 * makeTracks.C 2 の一般化: 3 面の分散 `var_v = p_vᵀ S p_v` から xy の 2×2 共分散 S を解く
 * (行 = [px², 2·px·py, py²]、未知数 = [Sxx, Sxy, Syy]、Cramer)。pitch が退化していれば `null`。
 */
export function xyCovariance(
  variances: readonly [number, number, number],
  pitch: readonly [Vec2, Vec2, Vec2],
): [number, number, number] | null {
  const rows = pitch.map(([px, py]) => [px * px, 2 * px * py, py * py]);
  const det = det3(rows);
  if (!(Math.abs(det) >= MIN_PITCH_DET)) return null;
  const solve = (col: number): number =>
    det3(rows.map((row, v) => row.map((value, c) => (c === col ? variances[v] : value)))) / det;
  return [solve(0), solve(1), solve(2)];
}

function plural(n: number): string {
  return n === 1 ? 'hit' : 'hits';
}

/**
 * makeTracks.C 1–5(帯選択を通ったヒット → 直線 1 本)。`kept` は U, V, W の順。
 * 頭尻は決めない(6 は移植しない): start = l が小さい側。
 */
export function fitTrack(
  kept: readonly (readonly StripHit[])[],
  pitch: readonly [Vec2, Vec2, Vec2],
): TrackFit {
  // 1. 面ごとの電荷加重の平均と分散
  const nv = [0, 0, 0];
  const qv = [0, 0, 0];
  const ms = [0, 0, 0];
  for (let v = 0; v < 3; v++) {
    for (const h of kept[v]) {
      nv[v]++;
      qv[v] += h.q;
      ms[v] += h.q * h.s;
    }
  }
  const short = [0, 1, 2]
    .filter((v) => nv[v] < MIN_VIEW_HITS_TRACK)
    .map(
      (v) =>
        `${VIEW_NAMES[v]} has ${nv[v]} strip ${plural(nv[v])} in the band ` +
        `(needs ${MIN_VIEW_HITS_TRACK})`,
    );
  if (short.length > 0) return { ok: false, reason: short.join('; ') };
  for (let v = 0; v < 3; v++) ms[v] /= qv[v];
  const variance: [number, number, number] = [0, 0, 0];
  for (let v = 0; v < 3; v++) {
    for (const h of kept[v]) {
      const ds = h.s - ms[v];
      variance[v] += h.q * ds * ds;
    }
    variance[v] /= qv[v];
  }

  // 2. 3 面の射影から xy の共分散 → 主軸 = d_xy
  const covariance = xyCovariance(variance, pitch);
  if (covariance === null) {
    return {
      ok: false,
      reason: 'U/V/W pitch vectors are degenerate (cannot solve the xy covariance)',
    };
  }
  const [sxx, sxy, syy] = covariance;
  const gap = Math.hypot(sxx - syy, 2 * sxy); // lam_max - lam_min
  if (gap < MIN_EIGEN_GAP_MM2) {
    return { ok: false, reason: 'no xy elongation (track along the drift)' };
  }
  const phi = 0.5 * Math.atan2(2 * sxy, sxx - syy);
  const ux = Math.cos(phi);
  const uy = Math.sin(phi);

  // 3. anchor (x0, y0) = pitch_v·(x0, y0) = ms_v の最小二乗。Q = 3 面の電荷
  let a00 = 0;
  let a01 = 0;
  let a11 = 0;
  let b0 = 0;
  let b1 = 0;
  let charge = 0;
  for (let v = 0; v < 3; v++) {
    const [px, py] = pitch[v];
    a00 += px * px;
    a01 += px * py;
    a11 += py * py;
    b0 += px * ms[v];
    b1 += py * ms[v];
    charge += qv[v];
  }
  const det = a00 * a11 - a01 * a01;
  const mx = (a11 * b0 - a01 * b1) / det;
  const my = (-a01 * b0 + a00 * b1) / det;

  // 4. 各ヒットの xy 経路長 l(|pitch_v·d| >= MIN_PROJ の面だけ)と z(l) の電荷加重直線フィット
  const proj = pitch.map(([px, py]) => px * ux + py * uy);
  const lq: { l: number; q: number; t: number }[] = [];
  let w = 0;
  let sl = 0;
  let sz = 0;
  let sll = 0;
  let slz = 0;
  for (let v = 0; v < 3; v++) {
    if (Math.abs(proj[v]) < MIN_PROJ) continue;
    for (const h of kept[v]) {
      const l = (h.s - ms[v]) / proj[v];
      lq.push({ l, q: h.q, t: h.t });
      w += h.q;
      sl += h.q * l;
      sz += h.q * h.z;
      sll += h.q * l * l;
      slz += h.q * l * h.z;
    }
  }
  const varL = w > 0 ? sll / w - (sl / w) * (sl / w) : 0;
  if (!(varL > 0)) {
    return { ok: false, reason: 'no spread along the track (no hits with a usable path length)' };
  }
  const tanTh = (slz / w - (sl / w) * (sz / w)) / varL;
  const z0 = sz / w - (tanTh * sl) / w; // l = 0 での切片
  const nrm = Math.sqrt(1 + tanTh * tanTh);

  // 5. 端点 = l の電荷分位点(QUANTILE .. 1 − QUANTILE)と最も外側のヒット
  const points = lq.map(({ l, q, t }) => ({ x: mx + l * ux, y: my + l * uy, t, q }));
  const sorted = [...lq].sort((a, b) => a.l - b.l || a.q - b.q);
  let total = 0;
  for (const e of sorted) total += e.q;
  let lo = sorted[0].l;
  let hi = sorted[sorted.length - 1].l;
  let cum = 0;
  for (const e of sorted) {
    cum += e.q;
    if (cum / total <= QUANTILE) lo = e.l;
    if (cum / total <= 1 - QUANTILE) hi = e.l;
  }
  const loF = sorted[0].l;
  const hiF = sorted[sorted.length - 1].l;
  const at = (l: number): [number, number, number] => [mx + l * ux, my + l * uy, z0 + l * tanTh];

  return {
    ok: true,
    phiDeg: (phi * 180) / Math.PI,
    dir: [ux / nrm, uy / nrm, tanTh / nrm],
    start: at(lo),
    end: at(hi),
    startF: at(loF),
    endF: at(hiF),
    length: (hi - lo) * nrm,
    lengthF: (hiF - loF) * nrm,
    xyLength: hi - lo,
    charge,
    points,
  };
}

// ---------------------------------------------------------------------------------------------
// 1 イベント(3 面の 0x02)→ 直線 1 本 + XY / XZ / YZ
// ---------------------------------------------------------------------------------------------

export interface TrackEventResult extends Projections {
  /** 面ごとの strip ヒット数(halo cut の後 = analyzeUVW 0 の出力)。 */
  readonly haloHits: readonly [number, number, number];
  /** 帯選択を通った strip ヒット(analyzeUVW の出力 = makeTracks の入力)。U, V, W の順。 */
  readonly selected: readonly [readonly StripHit[], readonly StripHit[], readonly StripHit[]];
  readonly track: TrackFit;
  /** XY / XZ / YZ に詰めた点の数(ok でなければ 0)。 */
  readonly points: number;
  /** 3 枚のどれかのヒスト範囲の外に落ちた点の数。 */
  readonly outOfRange: number;
  /** ヒットがあったのに `stripGeometry` に線分の無い strip の数。 */
  readonly unplaced: number;
  /** どれかの面の 0x02 が incomplete(fragment 欠落で電荷が足りない)。 */
  readonly incomplete: boolean;
}

/**
 * 計算できない理由(無ければ `null`)。投影の前提(`projectionBlocker`)に加えて、
 * ペデスタル窓(cell PED_LO..PED_HI)が収まる bucket 数があること。
 */
export function trackBlocker(
  uvw: readonly (UvwMessage | null)[],
  geometry: StripGeometryMessage,
): string | null {
  const blocker = projectionBlocker(uvw, geometry);
  if (blocker !== null) return blocker;
  const nBuckets = (uvw[0] as UvwMessage).nBuckets;
  if (nBuckets <= PED_HI) {
    return `${nBuckets} time buckets do not cover the pedestal window (cells ${PED_LO}–${PED_HI})`;
  }
  return null;
}

/**
 * 1 イベントの直線トラック(analyzeUVW 0–4 + makeTracks 1–5)と表示用の XY / XZ / YZ。
 * 計算できない状態(`trackBlocker` ≠ null)は `null`。
 * 表示: ok なら帯内ヒットのうち |pitch_v·d| ≥ MIN_PROJ の面のものを (x, y) = anchor + l·d、
 * Z = そのヒットの時刻 [bucket] に、重み q で詰める。
 */
export function trackEvent(
  uvw: readonly (UvwMessage | null)[],
  geometry: StripGeometryMessage,
): TrackEventResult | null {
  if (trackBlocker(uvw, geometry) !== null) return null;
  const messages = uvw as readonly [UvwMessage, UvwMessage, UvwMessage];
  const positions = stripPositions(geometry);
  const pitch: [Vec2, Vec2, Vec2] = [
    geometry.planes.U.pitchUnit,
    geometry.planes.V.pitchUnit,
    geometry.planes.W.pitchUnit,
  ];

  let unplaced = 0;
  const haloHits: [number, number, number] = [0, 0, 0];
  const selected: [StripHit[], StripHit[], StripHit[]] = [[], [], []];
  for (let v = 0; v < 3; v++) {
    const found = stripHits(messages[v], positions[v]);
    unplaced += found.unplaced;
    haloHits[v] = found.hits.length;
    selected[v] = bandSelect(found.hits);
  }

  const track = hitShortage(haloHits, selected) ?? fitTrack(selected, pitch);
  const projections = createProjections(geometry, messages[0].nBuckets);
  let outOfRange = 0;
  const points = track.ok ? track.points : [];
  for (const p of points) {
    if (!fillPoint(projections, p.x, p.y, p.t, p.q)) outOfRange++;
  }
  return {
    ...projections,
    haloHits,
    selected,
    track,
    points: points.length,
    outOfRange,
    unplaced,
    incomplete: messages.some((message) => message.header.incomplete),
  };
}

/**
 * makeTracks 1 の「各面 MIN_VIEW_HITS_TRACK 本」を、どこで足りなくなったかの文にする。
 * halo cut の後で MIN_VIEW_HITS_BAND 本未満なら analyzeUVW が帯フィットをせず(その面は 0 本)、
 * そうでなければ帯の中の本数で判定する — macros と同じ判定(ok ⇔ 全面で帯内 ≥ 3 本)。
 */
function hitShortage(
  haloHits: readonly number[],
  selected: readonly (readonly StripHit[])[],
): TrackNone | null {
  const reasons: string[] = [];
  for (let v = 0; v < 3; v++) {
    const n = haloHits[v];
    if (n < MIN_VIEW_HITS_BAND) {
      reasons.push(
        `${VIEW_NAMES[v]} has ${n} strip ${plural(n)} after the halo cut ` +
          `(needs ${MIN_VIEW_HITS_BAND})`,
      );
    } else if (selected[v].length < MIN_VIEW_HITS_TRACK) {
      const k = selected[v].length;
      reasons.push(
        `${VIEW_NAMES[v]} has ${k} strip ${plural(k)} in the band (needs ${MIN_VIEW_HITS_TRACK})`,
      );
    }
  }
  return reasons.length > 0 ? { ok: false, reason: reasons.join('; ') } : null;
}
