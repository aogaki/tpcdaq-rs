/**
 * TODO/125 レーン B 項目 2/5 — XY / XZ / YZ 投影(純ロジック)。
 *
 * ジオメトリはテスト内で組む小さな合成物(等間隔の直線 3 族)。値は手計算(各ケースのコメント)。
 */
import type { StripGeometryMessage, StripPlaneGeometry } from '../ws/json';
import type { UvwMessage } from '../ws/wire';
import { BASELINE_CELLS } from './root-histo';
import {
  BASE,
  HEX,
  type Layout,
  type TrackFn,
  addTrack,
  emptyUvw,
  geometryOf,
  setHit,
} from './xy-test-fixtures';
import {
  DEFAULT_HIT_THRESHOLD,
  MAX_CROSS_EVALUATIONS,
  NUMERICAL_TOLERANCE,
  PROJECTION_RADIUS_MM,
  type Projection2d,
  type Projections,
  type SegmentLike,
  type Vec2,
  createProjections,
  crossPoint,
  fillPoint,
  matchCrossPoint,
  projectEvent,
  projectionBlocker,
} from './xy-projection';

// ---------------------------------------------------------------------------------------------
// crossPoint / matchCrossPoint の手計算ケース
// ---------------------------------------------------------------------------------------------

const X_AXIS: Vec2 = [1, 0];
const Y_AXIS: Vec2 = [0, 1];
const seg = (x0: number, y0: number, length: number): SegmentLike => ({ x0, y0, length });

describe('crossPoint — TPCReco GetCrossPoint の式', () => {
  it('定数は TPCReco と同じ(許容 1e-5 / 半径 2.0 / 閾値 50 / 上限 2×10⁶)', () => {
    expect(NUMERICAL_TOLERANCE).toBe(1e-5);
    expect(PROJECTION_RADIUS_MM).toBe(2.0);
    expect(DEFAULT_HIT_THRESHOLD).toBe(50);
    expect(MAX_CROSS_EVALUATIONS).toBe(2_000_000);
  });

  it('直交する 2 本は交点を返す(A: y=0 を x 方向、B: x=3 を y 方向に y=-4 から)', () => {
    // 手計算: w=-1, d=(3,-4), w1=-3, w2=-4 → len1=3, len2=4 → (0+3*1, 0) = (3,0)
    expect(crossPoint(seg(0, 0, 10), seg(3, -4, 10), X_AXIS, Y_AXIS, 2)).toEqual([3, 0]);
  });

  it('平行は null(|w| < 1e-5)', () => {
    expect(crossPoint(seg(0, 0, 10), seg(0, 5, 10), X_AXIS, X_AXIS, 2)).toBeNull();
    // 角度が 1e-6 rad ずれていても平行扱い(w = sin(1e-6) < 1e-5)
    expect(crossPoint(seg(0, 0, 10), seg(0, 5, 10), X_AXIS, [1, 1e-6], 2)).toBeNull();
  });

  it('線分の外(長さ側)は null、端は tol 内なら通る', () => {
    // B は y=-4 から +y へ。交点は B の 4 mm 目。
    expect(crossPoint(seg(0, 0, 10), seg(3, -4, 4), X_AXIS, Y_AXIS, 2)).not.toBeNull();
    expect(crossPoint(seg(0, 0, 10), seg(3, -4, 4 - 5e-6), X_AXIS, Y_AXIS, 2)).not.toBeNull();
    expect(crossPoint(seg(0, 0, 10), seg(3, -4, 4 - 2e-5), X_AXIS, Y_AXIS, 2)).toBeNull();
    expect(crossPoint(seg(0, 0, 10), seg(3, -4, 3), X_AXIS, Y_AXIS, 2)).toBeNull();
  });

  it('始点側は -0.5*padPitch まで許す(padPitch=2 → -1 mm)', () => {
    // B は (3, y0) から +y 方向。交点 y=0 は B の len2 = -y0。
    expect(crossPoint(seg(0, 0, 10), seg(3, 0.9, 10), X_AXIS, Y_AXIS, 2)).not.toBeNull(); // -0.9
    expect(crossPoint(seg(0, 0, 10), seg(3, 1.0, 10), X_AXIS, Y_AXIS, 2)).not.toBeNull(); // -1.0 ちょうど
    expect(crossPoint(seg(0, 0, 10), seg(3, 1.1, 10), X_AXIS, Y_AXIS, 2)).toBeNull(); // -1.1
    // A 側も同じ: 交点 x=3 は A の len1 = 3 - x0
    expect(crossPoint(seg(4, 0, 10), seg(3, -4, 10), X_AXIS, Y_AXIS, 2)).not.toBeNull(); // -1.0
    expect(crossPoint(seg(4.2, 0, 10), seg(3, -4, 10), X_AXIS, Y_AXIS, 2)).toBeNull(); // -1.2
  });
});

describe('matchCrossPoint — TPCReco MatchCrossPoint の式', () => {
  // U: y=0(x 方向)、V: x=0(y 方向)、W: 点 (1,0) を通る 45° 線。
  //  p(U,V)=(0,0)、p(V,W)=(0,-1)、p(W,U)=(1,0)
  //  距離: 1, 1, √2 → radius ≥ √2 で一致。重心 = (1/3, -1/3)。
  const u = seg(-5, 0, 10);
  const v = seg(0, -5, 10);
  const w = seg(1, 0, 10);
  const s = Math.SQRT1_2;
  const unitW: Vec2 = [s, s];
  const pad = 4; // -0.5*pad = -2 ≤ -√2(V×W の W 側 len = -√2 を許す)

  it('radius の境界: √2 以上で一致、未満は不一致。重心を返す', () => {
    const hit = matchCrossPoint(u, v, w, X_AXIS, Y_AXIS, unitW, pad, 1.4143);
    expect(hit?.[0]).toBeCloseTo(1 / 3, 9);
    expect(hit?.[1]).toBeCloseTo(-1 / 3, 9);
    expect(matchCrossPoint(u, v, w, X_AXIS, Y_AXIS, unitW, pad, 1.4141)).toBeNull();
  });

  it('どれか 1 組が平行なら不一致', () => {
    expect(matchCrossPoint(u, v, seg(0, 3, 10), X_AXIS, Y_AXIS, X_AXIS, pad, 100)).toBeNull();
  });

  it('どれか 1 組が線分の外なら不一致', () => {
    // W を (2,1) 始まりにすると V×W の交点 (0,-1) は W 上の len = -2√2 = -2.83 < -pad/2 = -2
    expect(matchCrossPoint(u, v, seg(2, 1, 10), X_AXIS, Y_AXIS, unitW, pad, 100)).toBeNull();
  });
});

// ---------------------------------------------------------------------------------------------
// ビン index の手計算(鏡像・転置を落とす)
// ---------------------------------------------------------------------------------------------

function tinyGeometry(): StripGeometryMessage {
  const plane: StripPlaneGeometry = { unit: [1, 0], pitchUnit: [0, 1], segments: [] };
  return {
    type: 'stripGeometry',
    padPitch: 3,
    stripPitch: 2,
    rangeXY: [-10, 10, -9, 9],
    planes: { U: plane, V: plane, W: plane },
  };
}

const sum = (h: Float64Array): number => h.reduce((a, b) => a + b, 0);

describe('ビン index(values[iy*nx+ix])', () => {
  // nx = trunc(20/2 - 1) = 9(幅 20/9)、ny = trunc(18/3 - 1) = 5(幅 3.6)、nz = nBuckets = 10(-0.5〜9.5)
  const projections: Projections = createProjections(tinyGeometry(), 10);

  it('本数と範囲は rangeXY / pitch / nBuckets から決まる', () => {
    expect([projections.xy.nx, projections.xy.ny]).toEqual([9, 5]);
    expect([projections.xz.nx, projections.xz.ny]).toEqual([9, 10]);
    expect([projections.yz.nx, projections.yz.ny]).toEqual([5, 10]);
    expect([projections.xz.ymin, projections.xz.ymax]).toEqual([-0.5, 9.5]);
    expect([projections.xy.values.length, projections.xz.values.length]).toEqual([45, 90]);
  });

  it('(x,y,bucket) = (5,4,7) は XY[3*9+6]、XZ[7*9+6]、YZ[7*5+3]', () => {
    // ix = floor((5+10)/20*9) = floor(6.75) = 6;  iy = floor((4+9)/18*5) = floor(3.61) = 3
    // iz = floor((7+0.5)/10*10) = 7
    const h = createProjections(tinyGeometry(), 10);
    expect(fillPoint(h, 5, 4, 7, 2.5)).toBe(true);
    expect(h.xy.values[3 * 9 + 6]).toBe(2.5);
    expect(h.xz.values[7 * 9 + 6]).toBe(2.5);
    expect(h.yz.values[7 * 5 + 3]).toBe(2.5);
    for (const grid of [h.xy, h.xz, h.yz]) expect(sum(grid.values)).toBe(2.5); // 他のビンは 0
  });

  it('負の座標は小さい index 側(x=-9, y=-8, bucket 0 → ix=0, iy=0, iz=0)', () => {
    // ix = floor(1/20*9) = 0, iy = floor(1/18*5) = 0
    const h = createProjections(tinyGeometry(), 10);
    fillPoint(h, -9, -8, 0, 1);
    expect(h.xy.values[0]).toBe(1);
    expect(h.xz.values[0]).toBe(1);
    expect(h.yz.values[0]).toBe(1);
  });

  it('上端ちょうどと範囲外は詰めない(false)', () => {
    const h = createProjections(tinyGeometry(), 10);
    expect(fillPoint(h, 10, 0, 3, 1)).toBe(false); // x == xmax は overflow(ROOT と同じ)
    expect(fillPoint(h, 0, -9.5, 3, 1)).toBe(false);
    expect(sum(h.xy.values)).toBe(0);
  });
});

function distanceToSegment(p: [number, number], a: [number, number], b: [number, number]): number {
  const abx = b[0] - a[0];
  const aby = b[1] - a[1];
  const t = Math.max(
    0,
    Math.min(1, ((p[0] - a[0]) * abx + (p[1] - a[1]) * aby) / (abx * abx + aby * aby)),
  );
  return Math.hypot(p[0] - (a[0] + t * abx), p[1] - (a[1] + t * aby));
}

const OPTIONS = { threshold: DEFAULT_HIT_THRESHOLD, radius: PROJECTION_RADIUS_MM };

interface Collected {
  x: number;
  y: number;
  bucket: number;
}

describe('projectEvent — 直線トラック', () => {
  // 原点を通らない非対称な直線: 第 1 象限の (20,5) mm → (60,30) mm、bucket 100 → 200。
  const A: [number, number] = [20, 5];
  const B: [number, number] = [60, 30];
  const track: TrackFn = (bucket) => {
    const t = (bucket - 100) / 100;
    return [A[0] + t * (B[0] - A[0]), A[1] + t * (B[1] - A[1])];
  };

  const geometry = geometryOf(HEX);
  const uvw = emptyUvw(HEX, 300);
  addTrack(uvw, geometry, HEX, track, 100, 200);

  const collected: Collected[] = [];
  const result = projectEvent(uvw, geometry, {
    ...OPTIONS,
    onPoint: (x, y, bucket) => collected.push({ x, y, bucket }),
  });

  it('点が出て、すべて真の直線から 1 stripPitch 以内', () => {
    expect(result).not.toBeNull();
    expect(result!.points).toBeGreaterThan(50);
    expect(result!.points).toBe(collected.length);
    expect(result!.capped).toBe(false);
    for (const point of collected) {
      expect(distanceToSegment([point.x, point.y], A, B)).toBeLessThanOrEqual(1.5);
    }
  });

  it('鏡像・回転を落とす: 各 bucket の点は、その bucket の真の位置の近く', () => {
    // 直線への距離だけでは鏡像でも通る。bucket ごとの真位置に 3 stripPitch 以内を要求する。
    for (const point of collected) {
      const [tx, ty] = track(point.bucket);
      expect(Math.hypot(point.x - tx, point.y - ty)).toBeLessThanOrEqual(3 * 1.5);
    }
  });

  it('点は第 1 象限、bucket が増えると x も y も増える(傾き > 0)', () => {
    expect(collected.every((p) => p.x > 0 && p.y > 0)).toBe(true);
    const n = collected.length;
    const mean = (f: (p: Collected) => number) => collected.reduce((s, p) => s + f(p), 0) / n;
    const mb = mean((p) => p.bucket);
    const slope = (f: (p: Collected) => number) => {
      const mf = mean(f);
      return (
        collected.reduce((s, p) => s + (p.bucket - mb) * (f(p) - mf), 0) /
        collected.reduce((s, p) => s + (p.bucket - mb) ** 2, 0)
      );
    };
    // 真の傾き: dx/db = 0.4 mm/bucket、dy/db = 0.25 mm/bucket
    expect(slope((p) => p.x)).toBeGreaterThan(0.35);
    expect(slope((p) => p.x)).toBeLessThan(0.45);
    expect(slope((p) => p.y)).toBeGreaterThan(0.2);
    expect(slope((p) => p.y)).toBeLessThan(0.3);
    // 重心は経路の中央 (40, 17.5) 付近
    expect(Math.abs(mean((p) => p.x) - 40)).toBeLessThan(3);
    expect(Math.abs(mean((p) => p.y) - 17.5)).toBeLessThan(3);
  });

  it('XY / XZ / YZ のビン: 電荷の重心が x>0,y>0 側・bucket 100..200 の中央にある', () => {
    const r = result!;
    // [横の重心, 縦の重心](ビン中心、電荷加重)
    const centroid = (h: Projection2d): [number, number] => {
      let total = 0;
      let sx = 0;
      let sy = 0;
      for (let iy = 0; iy < h.ny; iy++) {
        for (let ix = 0; ix < h.nx; ix++) {
          const q = h.values[iy * h.nx + ix];
          if (q === 0) continue;
          total += q;
          sx += q * (h.xmin + ((ix + 0.5) * (h.xmax - h.xmin)) / h.nx);
          sy += q * (h.ymin + ((iy + 0.5) * (h.ymax - h.ymin)) / h.ny);
        }
      }
      return [sx / total, sy / total];
    };
    const [cx, cy] = centroid(r.xy);
    expect(cx).toBeGreaterThan(35);
    expect(cx).toBeLessThan(45);
    expect(cy).toBeGreaterThan(14);
    expect(cy).toBeLessThan(21);
    const [xzx, xzb] = centroid(r.xz);
    expect(xzx).toBeGreaterThan(35);
    expect(xzx).toBeLessThan(45);
    expect(xzb).toBeGreaterThan(140);
    expect(xzb).toBeLessThan(160);
    const [yzy, yzb] = centroid(r.yz);
    expect(yzy).toBeGreaterThan(14);
    expect(yzy).toBeLessThan(21);
    expect(yzb).toBeGreaterThan(140);
    expect(yzb).toBeLessThan(160);
  });

  it('電荷は method 0(各 strip を一致数で等分)、ベースライン(100)は引かれている', () => {
    // 1 点の電荷は高々 3 strip 分(各 500)。点の数 × 1500 を超えない。
    const total = sum(result!.xy.values);
    expect(total).toBeGreaterThan(0);
    expect(total).toBeLessThanOrEqual(result!.points * 3 * 500 + 1e-6);
    // 3 枚とも総和が等しい(同じ点を同じ値で詰める)
    expect(sum(result!.xz.values)).toBeCloseTo(total, 6);
    expect(sum(result!.yz.values)).toBeCloseTo(total, 6);
  });
});

describe('projectEvent — 計算しない / 0 点 / incomplete', () => {
  const geometry = geometryOf(HEX);

  it('3 面が揃っていない → null と理由', () => {
    const uvw = emptyUvw(HEX, 50);
    expect(projectEvent([uvw[0], null, uvw[2]], geometry, OPTIONS)).toBeNull();
    expect(projectionBlocker([uvw[0], null, uvw[2]], geometry)).toContain('V');
    expect(projectionBlocker([null, null, null], geometry)).toContain('U/V/W');
  });

  it('eventNumber が違う → null と理由(両方の番号が出る)', () => {
    const uvw = emptyUvw(HEX, 50);
    const other = { ...uvw[1], header: { ...uvw[1].header, eventNumber: 2 } };
    expect(projectEvent([uvw[0], other, uvw[2]], geometry, OPTIONS)).toBeNull();
    const reason = projectionBlocker([uvw[0], other, uvw[2]], geometry);
    expect(reason).toContain('#1');
    expect(reason).toContain('#2');
  });

  it('bucket 数が面で違う → null と理由', () => {
    const uvw = emptyUvw(HEX, 50);
    const other = { ...uvw[2], nBuckets: 40 };
    expect(projectionBlocker([uvw[0], uvw[1], other], geometry)).toContain('bucket');
  });

  it('閾値以下だけ → 0 点(null ではない)、capped ではない', () => {
    const uvw = emptyUvw(HEX, 60);
    // baseline 100 に対して +50 ちょうどは「超えていない」
    for (let plane = 0; plane < 3; plane++) setHit(uvw[plane], 101, 30, 150);
    const result = projectEvent(uvw, geometry, OPTIONS);
    expect(result).not.toBeNull();
    expect(result!.points).toBe(0);
    expect(result!.capped).toBe(false);
  });

  it('閾値は呼び手が決める: 同じイベントでも閾値を下げると点が出る', () => {
    const uvw = emptyUvw(HEX, 60);
    // 3 本の線が (0,0) で交わる中心 strip(101 番)を 30 bucket 目に +80 上げる
    for (let plane = 0; plane < 3; plane++) setHit(uvw[plane], 101, 30, 180);
    expect(projectEvent(uvw, geometry, { ...OPTIONS, threshold: 100 })!.points).toBe(0);
    expect(projectEvent(uvw, geometry, { ...OPTIONS, threshold: 50 })!.points).toBe(1);
  });

  it('incomplete はヘッダから引き継ぐ', () => {
    const complete = projectEvent(emptyUvw(HEX, 30), geometry, OPTIONS);
    const partial = projectEvent(emptyUvw(HEX, 30, 1, true), geometry, OPTIONS);
    expect(complete!.incomplete).toBe(false);
    expect(partial!.incomplete).toBe(true);
  });
});

describe('projectEvent — 打ち切り(capped)', () => {
  const geometry = geometryOf(HEX);

  it('評価回数が上限を超えたら打ち切り、capped: true(小さい上限で確認)', () => {
    const uvw = emptyUvw(HEX, 300);
    addTrack(uvw, geometry, HEX, () => [30, 10], 100, 200);
    const result = projectEvent(uvw, geometry, { ...OPTIONS, maxEvaluations: 10 });
    expect(result!.capped).toBe(true);
    expect(result!.evaluations).toBeGreaterThan(10);
    expect(result!.evaluations).toBeLessThan(10 + 5);
  });

  it('既定の上限 2×10⁶: 全 strip が全 bucket でヒットすると打ち切られ、結果が返る', () => {
    const nBuckets = 100;
    const uvw = emptyUvw(HEX, nBuckets);
    for (const message of uvw) {
      for (let strip = 1; strip <= HEX.n; strip++) {
        for (let b = BASELINE_CELLS; b < nBuckets; b++) setHit(message, strip, b, 1000);
      }
    }
    // 1 bucket あたり U×V = 201² = 40,401 回 → 75 bucket で約 3.03×10⁶ > 2×10⁶
    const result = projectEvent(uvw, geometry, OPTIONS);
    expect(result!.capped).toBe(true);
    expect(result!.evaluations).toBeGreaterThan(MAX_CROSS_EVALUATIONS);
    expect(result!.evaluations).toBeLessThan(MAX_CROSS_EVALUATIONS + 10);
  });
});

// ---------------------------------------------------------------------------------------------
// W 候補の絞り込み = 総当たり(乱数 200 イベント、六角 / 非六角)
// ---------------------------------------------------------------------------------------------

function mulberry32(seed: number): () => number {
  let a = seed;
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** TPCReco `Get2D`(method 0)を、絞り込み無しの総当たりでそのまま書いた参照実装。 */
function bruteForce(
  uvw: readonly UvwMessage[],
  geometry: StripGeometryMessage,
  threshold: number,
  radius: number,
) {
  const nBuckets = uvw[0].nBuckets;
  const projections = createProjections(geometry, nBuckets);
  const planes = [geometry.planes.U, geometry.planes.V, geometry.planes.W];
  const segmentsOf = (plane: number, strip: number) =>
    planes[plane].segments
      .filter((row) => row[0] === strip)
      .map(([, , x0, y0, length]) => ({ x0, y0, length }));
  const baseline = (message: UvwMessage, strip: number) => {
    let total = 0;
    for (let b = 0; b < BASELINE_CELLS; b++) total += message.adc[(strip - 1) * nBuckets + b];
    return total / BASELINE_CELLS;
  };
  const q = (plane: number, strip: number, bucket: number) =>
    uvw[plane].adc[(strip - 1) * nBuckets + bucket] - baseline(uvw[plane], strip);
  let points = 0;
  for (let bucket = 0; bucket < nBuckets; bucket++) {
    const hits = [0, 1, 2].map((plane) => {
      const list: number[] = [];
      for (let strip = 1; strip <= uvw[plane].nStrips; strip++) {
        if (q(plane, strip, bucket) > threshold) list.push(strip);
      }
      return list;
    });
    if (hits.some((list) => list.length === 0)) continue;
    const n = [new Map<number, number>(), new Map<number, number>(), new Map<number, number>()];
    const accepted = new Map<string, [number, number, number, number, number]>();
    for (const u of hits[0]) {
      for (const su of segmentsOf(0, u)) {
        for (const v of hits[1]) {
          for (const sv of segmentsOf(1, v)) {
            for (const w of hits[2]) {
              for (const sw of segmentsOf(2, w)) {
                const p = matchCrossPoint(
                  su,
                  sv,
                  sw,
                  planes[0].unit,
                  planes[1].unit,
                  planes[2].unit,
                  geometry.padPitch,
                  radius,
                );
                if (!p) continue;
                n[0].set(u, (n[0].get(u) ?? 0) + 1);
                n[1].set(v, (n[1].get(v) ?? 0) + 1);
                n[2].set(w, (n[2].get(w) ?? 0) + 1);
                const key = `${u},${v},${w}`;
                if (!accepted.has(key)) accepted.set(key, [u, v, w, p[0], p[1]]);
              }
            }
          }
        }
      }
    }
    for (const [u, v, w, x, y] of accepted.values()) {
      const value =
        q(0, u, bucket) / n[0].get(u)! +
        q(1, v, bucket) / n[1].get(v)! +
        q(2, w, bucket) / n[2].get(w)!;
      points++;
      fillPoint(projections, x, y, bucket, value);
    }
  }
  return { points, ...projections };
}

describe('W 候補の絞り込み = 総当たり(seed 固定の乱数 200 イベント)', () => {
  const cases: [string, Layout][] = [
    ['六角配置 0/-120/-60(V・W は 2 section)', { ...HEX, n: 61, length: 60, sections: [1, 2, 2] }],
    [
      '六角でない配置 11/22/33(V・W は 2 section)',
      { angles: [11, 22, 33], n: 61, d: 1.5, length: 120, sections: [1, 2, 2] },
    ],
  ];

  for (const [name, layout] of cases) {
    it(name, () => {
      const geometry = geometryOf(layout);
      const random = mulberry32(12345);
      let total = 0;
      for (let event = 0; event < 200; event++) {
        const nBuckets = 40;
        const uvw = emptyUvw(layout, nBuckets, event + 1);
        // 各 bucket(26..39)に、各面でランダムな 3〜10 本をヒットさせる(電荷もランダム)
        for (let bucket = 26; bucket < nBuckets; bucket++) {
          for (let plane = 0; plane < 3; plane++) {
            const count = 3 + Math.floor(random() * 8);
            for (let i = 0; i < count; i++) {
              const strip = 1 + Math.floor(random() * layout.n);
              setHit(uvw[plane], strip, bucket, BASE + 100 + Math.floor(random() * 1900));
            }
          }
        }
        const fast = projectEvent(uvw, geometry, OPTIONS)!;
        const slow = bruteForce(uvw, geometry, OPTIONS.threshold, OPTIONS.radius);
        expect(fast.capped).toBe(false);
        expect(fast.points).toBe(slow.points);
        expect(Array.from(fast.xy.values)).toEqual(Array.from(slow.xy.values));
        expect(Array.from(fast.xz.values)).toEqual(Array.from(slow.xz.values));
        expect(Array.from(fast.yz.values)).toEqual(Array.from(slow.yz.values));
        total += slow.points;
      }
      // 両方 0 点の空振りで通っていないこと
      expect(total).toBeGreaterThan(100);
    });
  }
});

// ---------------------------------------------------------------------------------------------
// 性能(合否ではない。結果節用に所要を出す)
// ---------------------------------------------------------------------------------------------

describe('性能 — ELITPC 規模の合成イベント(各面 1001 strip、3 本の飛跡、512 bucket)', () => {
  it('projectEvent の所要を測る', () => {
    const layout: Layout = {
      angles: [0, -120, -60],
      n: 1001,
      d: 1.5,
      length: 300,
      sections: [1, 2, 2],
    };
    const geometry = geometryOf(layout);
    const segments =
      geometry.planes.U.segments.length +
      geometry.planes.V.segments.length +
      geometry.planes.W.segments.length;
    const tracks: [TrackFn, number, number][] = [
      [(b) => [-100 + 0.5 * (b - 50), -40 + 0.2 * (b - 50)], 50, 450],
      [(b) => [60 - 0.3 * (b - 80), 80 - 0.4 * (b - 80)], 80, 400],
      [(b) => [10 + 0.1 * (b - 100), -90 + 0.35 * (b - 100)], 100, 440],
    ];
    const measure = (halfWidth: number) => {
      const uvw = emptyUvw(layout, 512);
      for (const [track, from, to] of tracks) {
        addTrack(uvw, geometry, layout, track, from, to, halfWidth);
      }
      const t0 = performance.now();
      const first = projectEvent(uvw, geometry, OPTIONS); // 初回は索引の構築を含む
      const firstMs = performance.now() - t0;
      const warm: number[] = [];
      let result = first;
      for (let i = 0; i < 5; i++) {
        const t1 = performance.now();
        result = projectEvent(uvw, geometry, OPTIONS);
        warm.push(performance.now() - t1);
      }
      return { result: result!, firstMs, warm };
    };
    // 細い飛跡(各面 1 strip/bucket/飛跡)と、幅のある飛跡(±3 strip)。
    const thin = measure(0);
    const wide = measure(3);
    for (const [label, m] of [
      ['thin', thin],
      ['wide(+-3 strips)', wide],
    ] as const) {
      console.info(
        `[perf] projectEvent ELITPC-size ${label}: segments=${segments} points=${m.result.points} ` +
          `evaluations=${m.result.evaluations} capped=${m.result.capped} ` +
          `first(index build included)=${m.firstMs.toFixed(1)} ms, ` +
          `warm=${m.warm.map((t) => t.toFixed(1)).join(',')} ms`,
      );
    }
    const result = thin.result;
    expect(result!.points).toBeGreaterThan(0);
    expect(result!.capped).toBe(false);
  });
});
