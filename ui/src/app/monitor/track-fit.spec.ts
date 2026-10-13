/**
 * TODO/125 改訂 2 レーン D — macros/analyzeUVW.C + makeTracks.C の移植(純ロジック)。
 *
 * 値は手計算(各ケースのコメント)。合成ジオメトリ・合成 0x02 は `track-test-fixtures.ts`。
 * 実データの照合は env-gated の `track-fit-oracle.spec.ts`。
 */
import type { StripGeometryMessage } from '../ws/json';
import type { UvwMessage } from '../ws/wire';
import {
  ADC_THRESHOLD,
  BAND_MM,
  MIN_EIGEN_GAP_MM2,
  MIN_PROJ,
  MIN_VIEW_HITS_BAND,
  MIN_VIEW_HITS_TRACK,
  MM_PER_CELL,
  PEAK_HALF_CELLS,
  PED_HI,
  PED_LO,
  QUANTILE,
  STRIP_Q_MIN,
  type StripHit,
  type TrackOk,
  type Vec2,
  bandSelect,
  fitTrack,
  stripHits,
  stripPositions,
  trackEvent,
  xyCovariance,
} from './track-fit';
import {
  BASE,
  ELITPC_ANGLES,
  MINI_ANGLES,
  type Track,
  addHalo,
  addPulse,
  addTrack,
  emptyEvent,
  layoutOf,
  pitchUnits,
  syntheticGeometry,
} from './track-test-fixtures';
import type { Projection2d } from './xy-projection';

/** mini の pitch(makeTracks.C の PITCH を √3/2 の厳密値で)。 */
const SQRT3_2 = Math.sqrt(3) / 2;
const MINI_PITCH: [Vec2, Vec2, Vec2] = [
  [-1, 0],
  [0.5, SQRT3_2],
  [0.5, -SQRT3_2],
];

/** 単一面の 0x02(全セル 250)。 */
function planeMessage(nStrips: number, nBuckets = 512, plane = 1): UvwMessage {
  return {
    kind: 'uvw',
    header: { msgType: 2, incomplete: false, runNumber: 7, eventNumber: 1 },
    plane,
    nStrips,
    nBuckets,
    adc: new Uint16Array(nStrips * nBuckets).fill(250),
  };
}

function setCell(message: UvwMessage, strip: number, bucket: number, adc: number): void {
  message.adc[(strip - 1) * message.nBuckets + bucket] = adc;
}

/** fitTrack の入力を手で作る(t は z から逆算)。 */
function hit(view: number, s: number, z: number, q: number, strip = 0): StripHit {
  return { view, strip, s, t: z / MM_PER_CELL, z, q };
}

// ---------------------------------------------------------------------------------------------
describe('定数は macros と同じ値', () => {
  it('analyzeUVW.C / makeTracks.C の定数', () => {
    expect(ADC_THRESHOLD).toBe(35);
    expect([PED_LO, PED_HI]).toEqual([1, 20]);
    expect(PEAK_HALF_CELLS).toBe(5);
    expect(STRIP_Q_MIN).toBe(2000);
    expect(BAND_MM).toBe(10);
    expect(MIN_VIEW_HITS_BAND).toBe(4);
    expect(MIN_VIEW_HITS_TRACK).toBe(3);
    expect(QUANTILE).toBe(0.02);
    expect(MIN_EIGEN_GAP_MM2).toBe(1.0);
    expect(MIN_PROJ).toBe(0.3);
  });

  it('z の換算 = 0.7233 × (1200 V / 20 cm / 100 mbar) / 25 MHz × 10 = 0.173592 mm/cell', () => {
    expect(MM_PER_CELL).toBeCloseTo(0.173592, 12);
  });
});

// ---------------------------------------------------------------------------------------------
describe('stripHits — analyzeUVW.C 0(1 strip 1 ヒット、ピーク ±5 cell、halo cut)', () => {
  // 6 strip × 512 cell、ベースライン 250。s は任意の非対称な値。
  const positions = new Map([
    [1, -7.25],
    [2, -2.5],
    [3, 0.75],
    [4, 4.0],
    [5, 9.5],
    [6, 13.0],
  ]);
  const message = planeMessage(6);
  // strip 1: halo — ピーク 300 + 両隣 150 → q = 600 < 2000 で落ちる
  setCell(message, 1, 29, 400);
  setCell(message, 1, 30, 550);
  setCell(message, 1, 31, 400);
  // strip 2: ペデスタル窓 1..20 = 245×10 + 255×10 → 250。cell 0 は窓の外(0 でも効かない)。
  //   cell 40: +30(≤ 35 で除外)、41: +1000、42: +1500(ピーク)、43: +1000、44: +36(> 35 で採用)、
  //   47: +20(窓内だが除外)、48: +500(ピーク+6 = 窓の外)。
  //   q = 1000 + 1500 + 1000 + 36 = 3536、Σ q·t = 41000 + 63000 + 43000 + 1584 = 148584
  setCell(message, 2, 0, 0);
  for (let t = 1; t <= 10; t++) setCell(message, 2, t, 245);
  for (let t = 11; t <= 20; t++) setCell(message, 2, t, 255);
  setCell(message, 2, 40, 280);
  setCell(message, 2, 41, 1250);
  setCell(message, 2, 42, 1750);
  setCell(message, 2, 43, 1250);
  setCell(message, 2, 44, 286);
  setCell(message, 2, 47, 270);
  setCell(message, 2, 48, 750);
  // strip 3: ピークがちょうどペデスタル + 35 → `<= ADC_THRESHOLD` で落ちる
  setCell(message, 3, 60, 285);
  // strip 4: 同じ高さの山が 2 つ(cell 100 と 300)→ 最初の最大(100)を採る。q = 3500、t = 100
  for (const c of [100, 300]) {
    setCell(message, 4, c - 1, 1250);
    setCell(message, 4, c, 1750);
    setCell(message, 4, c + 1, 1250);
  }
  // strip 5: 末尾(cell 509..511)。窓は 505..511 で止まる(strip 6 の cell 0.. を読まない)。
  //   q = 3500、Σ q·t = 509000 + 765000 + 511000 → t = 510
  setCell(message, 5, 509, 1250);
  setCell(message, 5, 510, 1750);
  setCell(message, 5, 511, 1250);
  // strip 6: cell 0..3 = 2250 → ペデスタル = (3×2250 + 17×250)/20 = 550、ピーク = cell 0(最初の最大)、
  //   窓 0..5: 1700 × 4(cell 4,5 は −300 で除外)→ q = 6800、t = 1700×(0+1+2+3)/6800 = 1.5
  for (let t = 0; t <= 3; t++) setCell(message, 6, t, 2250);

  const { hits, unplaced } = stripHits(message, positions);

  it('halo・閾値ちょうどは落ち、strip 2/4/5/6 が strip 順に 1 ヒットずつ', () => {
    expect(hits.map((h) => h.strip)).toEqual([2, 4, 5, 6]);
    expect(unplaced).toBe(0);
    expect(hits.every((h) => h.view === 1)).toBe(true);
  });

  it('strip 2: 閾値以下と窓の外を除いた q と電荷加重時刻(手計算)', () => {
    const h = hits[0];
    expect(h.q).toBe(3536);
    expect(h.t).toBeCloseTo(148584 / 3536, 12);
    expect(h.z).toBeCloseTo((148584 / 3536) * MM_PER_CELL, 12);
    expect(h.s).toBe(-2.5);
  });

  it('ピークの同値は最初の最大(strip 4 は cell 100 の山だけ)', () => {
    expect(hits[1].q).toBe(3500);
    expect(hits[1].t).toBe(100);
  });

  it('窓は最終 cell で止まる(strip 5)、ペデスタルは cell 1..20(strip 6)', () => {
    expect(hits[2].q).toBe(3500);
    expect(hits[2].t).toBe(510);
    expect(hits[3].q).toBe(6800);
    expect(hits[3].t).toBe(1.5);
  });

  it('stripGeometry に線分の無い strip のヒットは数えて外す(silent にしない)', () => {
    const partial = new Map(positions);
    partial.delete(4);
    const result = stripHits(message, partial);
    expect(result.hits.map((h) => h.strip)).toEqual([2, 5, 6]);
    expect(result.unplaced).toBe(1);
  });
});

// ---------------------------------------------------------------------------------------------
describe('stripPositions — s = pitchUnit·(x0, y0)(section 番号が最小の線分)', () => {
  it('section 0 と 1 があれば 0 を採る。並びに依らない', () => {
    const geometry: StripGeometryMessage = {
      type: 'stripGeometry',
      padPitch: 1.7320508,
      stripPitch: 1.5,
      rangeXY: [-10, 10, -10, 10],
      planes: {
        // U: pitch (0, 1) → s = y0。strip 3 は section 1 が先に来ても section 0(y0 = 4.5)を採る。
        U: {
          unit: [1, 0],
          pitchUnit: [0, 1],
          segments: [
            [3, 1, 0, 9.9, 5],
            [3, 0, -2, 4.5, 5],
            [7, 0, 1, -3, 5],
          ],
        },
        // V: pitch (0.6, 0.8) → s = 0.6·x0 + 0.8·y0 = 0.6·5 + 0.8·(−2.5) = 1
        V: { unit: [0.8, -0.6], pitchUnit: [0.6, 0.8], segments: [[2, 0, 5, -2.5, 5]] },
        W: { unit: [1, 0], pitchUnit: [0, 1], segments: [] },
      },
    };
    const [u, v, w] = stripPositions(geometry);
    expect([...u.entries()]).toEqual([
      [3, 4.5],
      [7, -3],
    ]);
    expect(v.get(2)).toBeCloseTo(1, 12);
    expect(w.size).toBe(0);
  });
});

// ---------------------------------------------------------------------------------------------
describe('bandSelect — analyzeUVW.C 1–4(電荷加重 PCA の直線から ±10 mm)', () => {
  // 平らなトラック: z = 50 mm、s = −15..15(1.5 mm 刻み、21 本、q = 5000)。
  // 外れ値: s = 0.75 で z = 50 ± 12(帯の外)と 50 ± 9.5(帯の中)、q = 3000。
  // 対称なので mz = 50、czs = 0、css ≫ czz → 主軸 = s 軸、perp = −(z − 50)。
  const line: StripHit[] = [];
  for (let k = -10; k <= 10; k++) line.push(hit(0, 1.5 * k, 50, 5000));
  const outside = [hit(0, 0.75, 62, 3000), hit(0, 0.75, 38, 3000)];
  const inside = [hit(0, 0.75, 59.5, 3000), hit(0, 0.75, 40.5, 3000)];
  const kept = bandSelect([...line, ...outside, ...inside]);

  it('帯から 10 mm を超える strip は落ち、10 mm 以内は残る', () => {
    expect(kept).toHaveLength(23);
    expect(kept.some((h) => h.z === 62 || h.z === 38)).toBe(false);
    expect(kept.filter((h) => h.z === 59.5 || h.z === 40.5)).toHaveLength(2);
  });

  it('4 本未満の面は帯フィットをしない(空を返す)', () => {
    expect(bandSelect(line.slice(0, 3))).toEqual([]);
    expect(bandSelect(line.slice(0, 4))).toHaveLength(4);
  });
});

// ---------------------------------------------------------------------------------------------
describe('xyCovariance — makeTracks.C 2 の一般化(3×3 連立)', () => {
  it('mini の pitch では macros の閉じた式と一致する', () => {
    const variances: [number, number, number] = [2.0, 7.5, 3.25];
    // macros: Sxx = var_U、Sxy = (var_V − var_W)/√3、Syy = (var_V + var_W − 0.5·var_U)/1.5
    const closed = [2.0, (7.5 - 3.25) / Math.sqrt(3), (7.5 + 3.25 - 0.5 * 2.0) / 1.5];
    for (const pitch of [MINI_PITCH, pitchUnits(MINI_ANGLES)]) {
      const s = xyCovariance(variances, pitch)!;
      expect(s).not.toBeNull();
      s.forEach((value, i) => expect(value).toBeCloseTo(closed[i], 12));
    }
  });

  it('ELITPC の pitch でも S を取り戻す(var_v = p_vᵀ S p_v の往復)', () => {
    const [sxx, sxy, syy] = [2.0, -0.7, 5.5];
    const pitch = pitchUnits(ELITPC_ANGLES);
    const variances = pitch.map(
      ([px, py]) => px * px * sxx + 2 * px * py * sxy + py * py * syy,
    ) as [number, number, number];
    const s = xyCovariance(variances, pitch)!;
    expect(s[0]).toBeCloseTo(sxx, 12);
    expect(s[1]).toBeCloseTo(sxy, 12);
    expect(s[2]).toBeCloseTo(syy, 12);
  });

  it('pitch が退化(2 面が平行)なら null', () => {
    expect(
      xyCovariance(
        [1, 2, 3],
        [
          [1, 0],
          [-1, 0],
          [0, 1],
        ],
      ),
    ).toBeNull();
  });
});

// ---------------------------------------------------------------------------------------------
describe('fitTrack — makeTracks.C 1–5(手計算)', () => {
  // y = 2 の x 方向のトラック、x = 4, 7, 10, 13, 16、z = 10 + 0.5·(x − 10)、q = 1000(全 15 ヒット)。
  //   U: s = −x、V: s = 0.5x + √3、W: s = 0.5x − √3(mini の pitch)
  //   var_U = 18、var_V = var_W = 4.5 → Sxx = 18、Sxy = 0、Syy = 0 → φ = 0、d = (1, 0)
  //   ms = (−10, 5 + √3, 5 − √3) → anchor = (10, 2)。l = x − 10 ∈ {−6, −3, 0, 3, 6}
  //   z = 10 + 0.5·l → tanθ = 0.5、z0 = 10。2 % の位置: 1 本目で 1/15 > 0.02 → lo = −6、
  //   98 %: 14/15 ≤ 0.98 → hi = 6。start = (4, 2, 7)、end = (16, 2, 13)、
  //   length = 12·√1.25、dir = (2, 0, 1)/√5、charge = 15000
  const xs = [4, 7, 10, 13, 16];
  const s3 = Math.sqrt(3);
  const kept = [
    xs.map((x) => hit(0, -x, 10 + 0.5 * (x - 10), 1000)),
    xs.map((x) => hit(1, 0.5 * x + s3, 10 + 0.5 * (x - 10), 1000)),
    xs.map((x) => hit(2, 0.5 * x - s3, 10 + 0.5 * (x - 10), 1000)),
  ];
  const fit = fitTrack(kept, MINI_PITCH);

  it('ok、φ = 0、方向・端点・長さ・電荷が手計算どおり', () => {
    expect(fit.ok).toBe(true);
    const t = fit as TrackOk;
    expect(t.phiDeg).toBeCloseTo(0, 9);
    expect(t.dir[0]).toBeCloseTo(2 / Math.sqrt(5), 9);
    expect(t.dir[1]).toBeCloseTo(0, 9);
    expect(t.dir[2]).toBeCloseTo(1 / Math.sqrt(5), 9);
    [4, 2, 7].forEach((v, i) => expect(t.start[i]).toBeCloseTo(v, 9));
    [16, 2, 13].forEach((v, i) => expect(t.end[i]).toBeCloseTo(v, 9));
    [4, 2, 7].forEach((v, i) => expect(t.startF[i]).toBeCloseTo(v, 9));
    [16, 2, 13].forEach((v, i) => expect(t.endF[i]).toBeCloseTo(v, 9));
    expect(t.length).toBeCloseTo(12 * Math.sqrt(1.25), 9);
    expect(t.lengthF).toBeCloseTo(12 * Math.sqrt(1.25), 9);
    expect(t.xyLength).toBeCloseTo(12, 9);
    expect(t.charge).toBe(15000);
  });

  it('表示用の点: 各ヒットを anchor + l·d に置き、bucket はそのヒットの時刻', () => {
    const t = fit as TrackOk;
    expect(t.points).toHaveLength(15);
    for (const p of t.points) {
      expect(p.y).toBeCloseTo(2, 9);
      // z = 10 + 0.5·(x − 10) → x = 2z − 10
      expect(p.x).toBeCloseTo(2 * p.t * MM_PER_CELL - 10, 9);
      expect(p.q).toBe(1000);
    }
  });

  it('面のヒットが 3 本未満 → ok でない(理由に面と本数)', () => {
    const result = fitTrack([kept[0], kept[1].slice(0, 2), kept[2]], MINI_PITCH);
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.reason).toBe('V has 2 strip hits in the band (needs 3)');
  });

  it('pitch が退化 → ok でない', () => {
    const result = fitTrack(kept, [
      [1, 0],
      [1, 0],
      [0, 1],
    ]);
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.reason).toContain('degenerate');
  });

  it('xy に伸びていない(全面で同じ分散 → S = c·I)→ no xy elongation', () => {
    // 各面 s = −3, −1.5, 0, 1.5, 3(q 同じ)→ var = 1.5²·(4+1+0+1+4)/5 = 4.5 → S = 4.5·I、gap = 0
    const blob = [0, 1, 2].map((view) =>
      [-2, -1, 0, 1, 2].map((k) => hit(view, 1.5 * k, 30 + 4 * k, 3000)),
    );
    const result = fitTrack(blob, MINI_PITCH);
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.reason).toBe('no xy elongation (track along the drift)');
  });

  it('どの面も |pitch·d| < 0.3 → 経路長の分散が無い → ok でない', () => {
    // pitch U = (1,0)、V/W = (cos 10°, ±sin 10°)。S = diag(0.1, 100) になる分散を与える
    // → d = (0, 1)、|pitch·d| = 0, 0.174, 0.174 < 0.3 → l を持つヒットが無い → varL = 0
    const c = Math.cos((10 * Math.PI) / 180);
    const sn = Math.sin((10 * Math.PI) / 180);
    const pitch: [Vec2, Vec2, Vec2] = [
      [1, 0],
      [c, sn],
      [c, -sn],
    ];
    const variances = pitch.map(([px, py]) => px * px * 0.1 + py * py * 100);
    // 3 本(−a, 0, a、q 同じ)の分散 = 2a²/3 → a = √(1.5·var)
    const hits = variances.map((v, view) => {
      const a = Math.sqrt(1.5 * v);
      return [-a, 0, a].map((s) => hit(view, s, 40, 3000));
    });
    const result = fitTrack(hits, pitch);
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.reason).toContain('no spread along the track');
  });
});

// ---------------------------------------------------------------------------------------------
// 合成ジオメトリ上の既知の直線(mini と ELITPC の角度)
// ---------------------------------------------------------------------------------------------

/**
 * 原点を通らない非対称な直線: 第 1 象限の (10, 4) mm → (90, 26) mm、bucket 150 → 250(xy 長さ 83 mm)。
 * 傾き dy/dx = 22/80 = 0.275(転置なら 3.6、y 鏡像なら負)。φ = atan2(22, 80) = 15.38°。
 * どの strip 方向(mini 90/−30/30、ELITPC 0/−120/−60)とも 14.6° 以上離れる → 最も寝る面でも 15 本の
 * strip を横切る(短い飛跡だと、その面の端の strip の離散化だけで φ が 1° 近く動く)。
 */
const TRACK: Track = { a: [10, 4, 150], b: [90, 26, 250] };

/** 真の直線上の、始点から長さの割合 f の点 (x, y, z[mm])。 */
function alongTrack(f: number): [number, number, number] {
  const at = (i: number) => TRACK.a[i] + f * (TRACK.b[i] - TRACK.a[i]);
  return [at(0), at(1), at(2) * MM_PER_CELL];
}

function distance3(p: readonly number[], q: readonly number[]): number {
  return Math.hypot(p[0] - q[0], p[1] - q[1], p[2] - q[2]);
}

/** ビン中心の座標。 */
function binCenter(h: Projection2d, ix: number, iy: number): [number, number] {
  return [
    h.xmin + ((ix + 0.5) * (h.xmax - h.xmin)) / h.nx,
    h.ymin + ((iy + 0.5) * (h.ymax - h.ymin)) / h.ny,
  ];
}

function filledBins(h: Projection2d): { x: number; y: number; q: number }[] {
  const out: { x: number; y: number; q: number }[] = [];
  for (let iy = 0; iy < h.ny; iy++) {
    for (let ix = 0; ix < h.nx; ix++) {
      const q = h.values[iy * h.nx + ix];
      if (q !== 0) out.push({ ...pointOf(binCenter(h, ix, iy)), q });
    }
  }
  return out;
}

function pointOf([x, y]: [number, number]): { x: number; y: number } {
  return { x, y };
}

/** 電荷加重の傾き d(縦)/d(横)。 */
function slope(bins: { x: number; y: number; q: number }[]): number {
  const w = bins.reduce((s, b) => s + b.q, 0);
  const mx = bins.reduce((s, b) => s + b.q * b.x, 0) / w;
  const my = bins.reduce((s, b) => s + b.q * b.y, 0) / w;
  const sxy = bins.reduce((s, b) => s + b.q * (b.x - mx) * (b.y - my), 0);
  const sxx = bins.reduce((s, b) => s + b.q * (b.x - mx) ** 2, 0);
  return sxy / sxx;
}

for (const [label, angles] of [
  ['mini 90/-30/30', MINI_ANGLES],
  ['ELITPC 0/-120/-60', ELITPC_ANGLES],
] as const) {
  describe(`trackEvent — 既知の直線(${label})`, () => {
    const layout = layoutOf(angles);
    const geometry = syntheticGeometry(layout);
    const uvw = emptyEvent(layout);
    addTrack(uvw, layout, TRACK);
    const result = trackEvent(uvw, geometry)!;
    const track = result.track as TrackOk;
    // 電荷は一様(dE/dx 一定)→ 2 % / 98 % の電荷分位点の真の位置は長さの 2 % / 98 %
    const true2 = alongTrack(0.02);
    const true98 = alongTrack(0.98);

    it('ok、φ は真の方向 atan2(22, 80) = 15.38° から 0.5° 以内', () => {
      expect(result.track.ok).toBe(true);
      const truePhi = (Math.atan2(22, 80) * 180) / Math.PI;
      // 頭尻は決めない → 180° の不定性
      const diff = Math.abs(((track.phiDeg - truePhi + 270) % 180) - 90);
      expect(diff).toBeLessThan(0.5);
    });

    it('2–98 % の端点が真の直線の 2 % / 98 % の点から 2 mm 以内(順序は問わない)', () => {
      const direct = Math.max(distance3(track.start, true2), distance3(track.end, true98));
      const swapped = Math.max(distance3(track.start, true98), distance3(track.end, true2));
      expect(Math.min(direct, swapped)).toBeLessThan(2);
    });

    it('z の傾きの向き: xy で終点側へ進むと z(bucket)が増える', () => {
      const along =
        track.dir[0] * (TRACK.b[0] - TRACK.a[0]) + track.dir[1] * (TRACK.b[1] - TRACK.a[1]);
      expect(Math.sign(track.dir[2])).toBe(Math.sign(along));
      expect(track.dir[2]).not.toBe(0);
    });

    it('鏡像・回転を落とす: XY の点はすべて第 1 象限のトラックの外接矩形の中', () => {
      const bins = filledBins(result.xy);
      expect(bins.length).toBeGreaterThan(20);
      for (const b of bins) {
        expect(b.x).toBeGreaterThan(7);
        expect(b.x).toBeLessThan(93);
        expect(b.y).toBeGreaterThan(1);
        expect(b.y).toBeLessThan(29);
      }
      // 傾き dy/dx = 22/80 = 0.275(転置なら 3.6、鏡像なら負)
      expect(slope(bins)).toBeGreaterThan(0.22);
      expect(slope(bins)).toBeLessThan(0.33);
    });

    it('XZ / YZ: bucket は 150..250 の中で、x・y が増えると増える', () => {
      for (const h of [result.xz, result.yz]) {
        const bins = filledBins(h);
        expect(bins.length).toBeGreaterThan(20);
        for (const b of bins) {
          expect(b.y).toBeGreaterThan(145);
          expect(b.y).toBeLessThan(255);
        }
      }
      // bucket/x = 100/80 = 1.25、bucket/y = 100/22 = 4.5
      expect(slope(filledBins(result.xz))).toBeGreaterThan(1.1);
      expect(slope(filledBins(result.xz))).toBeLessThan(1.4);
      expect(slope(filledBins(result.yz))).toBeGreaterThan(4.0);
      expect(slope(filledBins(result.yz))).toBeLessThan(5.0);
    });

    it('3 枚の総和 = 表示点の電荷の和、範囲外は 0', () => {
      const total = track.points.reduce((s, p) => s + p.q, 0);
      const sum = (h: Projection2d) => h.values.reduce((s, v) => s + v, 0);
      expect(result.points).toBe(track.points.length);
      expect(result.outOfRange).toBe(0);
      expect(sum(result.xy)).toBeCloseTo(total, 6);
      expect(sum(result.xz)).toBeCloseTo(total, 6);
      expect(sum(result.yz)).toBeCloseTo(total, 6);
    });
  });
}

// ---------------------------------------------------------------------------------------------
describe('trackEvent — halo・帯・ok でない理由・計算しない状態', () => {
  const layout = layoutOf(MINI_ANGLES);
  const geometry = syntheticGeometry(layout);

  it('halo(q < 2000)の strip は落ちる: 全 strip に halo を足しても結果は同じ', () => {
    const clean = emptyEvent(layout);
    addTrack(clean, layout, TRACK);
    const haloed = emptyEvent(layout);
    addTrack(haloed, layout, TRACK);
    addHalo(haloed, 400); // トラック(150..250)から離れた時刻
    const a = trackEvent(clean, geometry)!;
    const b = trackEvent(haloed, geometry)!;
    expect(b.haloHits).toEqual(a.haloHits);
    expect(b.selected.map((v) => v.map((h) => h.strip))).toEqual(
      a.selected.map((v) => v.map((h) => h.strip)),
    );
    expect((b.track as TrackOk).phiDeg).toBeCloseTo((a.track as TrackOk).phiDeg, 9);
  });

  it('帯から 10 mm を超えて離れた strip は落ちる(U のトラックの端の隣に時刻の離れたパルス 1 本)', () => {
    const uvw = emptyEvent(layout);
    addTrack(uvw, layout, TRACK);
    // U は s = −x。トラックは s ∈ [−90, −10]、z = 26.0..43.4 mm(dz/ds = −0.217)。
    // strip 7 = s −91.5 mm(トラックの外の隣)に bucket 330(z = 57.3 mm)のパルス:
    // 直線の延長(z ≈ 43.7)から Δz ≈ 13.6 → 直線との距離 ≈ 13.6·cos(12.2°) ≈ 13.3 mm > 10。
    // 遠すぎる外れ値は PCA の直線そのものを引っ張る(macros と同じ — 頑健ではない)ので、近くに置く。
    addPulse(uvw[0], 7, 330, 450);
    const result = trackEvent(uvw, geometry)!;
    expect(result.haloHits[0]).toBe(result.selected[0].length + 1);
    expect(result.selected[0].some((h) => h.strip === 7)).toBe(false);
    expect(result.track.ok).toBe(true);
  });

  it('halo だけ → no track: 各面のヒット数と必要数を文で', () => {
    const uvw = emptyEvent(layout);
    addHalo(uvw, 200);
    const result = trackEvent(uvw, geometry)!;
    expect(result.track.ok).toBe(false);
    if (!result.track.ok) {
      expect(result.track.reason).toBe(
        'U has 0 strip hits after the halo cut (needs 4); ' +
          'V has 0 strip hits after the halo cut (needs 4); ' +
          'W has 0 strip hits after the halo cut (needs 4)',
      );
    }
    expect(result.points).toBe(0);
    expect(result.xy.values.every((v) => v === 0)).toBe(true);
  });

  it('1 面だけ 3 本 → その面だけ理由に出る(単数・複数も正しく)', () => {
    const uvw = emptyEvent(layout);
    addTrack(uvw, layout, TRACK);
    // W のトラックの strip を 3 本だけ残す: W の全セルを BASE に戻してから 3 本だけ足す
    uvw[2].adc.fill(BASE);
    for (const strip of [80, 81, 82]) addPulse(uvw[2], strip, 200, 600);
    const result = trackEvent(uvw, geometry)!;
    expect(result.track.ok).toBe(false);
    if (!result.track.ok) {
      expect(result.track.reason).toBe('W has 3 strip hits after the halo cut (needs 4)');
    }
    uvw[2].adc.fill(BASE);
    addPulse(uvw[2], 80, 200, 600);
    const one = trackEvent(uvw, geometry)!;
    if (!one.track.ok) {
      expect(one.track.reason).toBe('W has 1 strip hit after the halo cut (needs 4)');
    }
  });

  it('ドリフト方向のトラック(xy に伸びない)→ no xy elongation', () => {
    // (0,0) の真上: 各面で s = −3..3 の 5 本(中央 strip ±2)に、時刻をずらしたパルス
    const uvw = emptyEvent(layout);
    const center = (layout.n + 1) / 2;
    for (let plane = 0; plane < 3; plane++) {
      for (let k = -2; k <= 2; k++) addPulse(uvw[plane], center + k, 200 + 30 * k, 600);
    }
    const result = trackEvent(uvw, geometry)!;
    expect(result.track.ok).toBe(false);
    if (!result.track.ok)
      expect(result.track.reason).toBe('no xy elongation (track along the drift)');
  });

  it('incomplete はヘッダから引き継ぐ', () => {
    const uvw = emptyEvent(layout, 512, 3, true);
    addTrack(uvw, layout, TRACK);
    expect(trackEvent(uvw, geometry)!.incomplete).toBe(true);
  });

  it('3 面が揃っていない / bucket がペデスタル窓に足りない → null', () => {
    const uvw = emptyEvent(layout);
    expect(trackEvent([uvw[0], uvw[1], null], geometry)).toBeNull();
    expect(trackEvent(emptyEvent(layout, 20), geometry)).toBeNull();
    expect(trackEvent(emptyEvent(layout, 21), geometry)).not.toBeNull();
  });

  it('stripGeometry に無い strip のヒットは unplaced に数える', () => {
    const uvw = emptyEvent(layout);
    addTrack(uvw, layout, TRACK);
    const trimmed: StripGeometryMessage = {
      ...geometry,
      planes: {
        ...geometry.planes,
        V: {
          ...geometry.planes.V,
          segments: geometry.planes.V.segments.filter(([strip]) => strip !== 90),
        },
      },
    };
    const full = trackEvent(uvw, geometry)!;
    expect(full.selected[1].some((h) => h.strip === 90)).toBe(true);
    const result = trackEvent(uvw, trimmed)!;
    expect(result.unplaced).toBe(1);
    expect(result.haloHits[1]).toBe(full.haloHits[1] - 1);
  });
});
