/**
 * TODO/125 — XY / XZ / YZ のビン(`values[iy*nx+ix]`)と、計算しない状態の理由。
 *
 * 何を詰めるか(直線トラックの再構成)は monitor 側(Rust の `src/reco/` のテスト)。値は手計算(各ケースのコメント)。
 */
import type { StripGeometryMessage, StripPlaneGeometry } from '../ws/json';
import {
  type Projections,
  createProjections,
  fillPoint,
  fillTrackPoints,
  projectionBlocker,
} from './xy-projection';

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

describe('fillTrackPoints — 0x04 の点をビンに詰める', () => {
  it('非対称な 2 点: 既知のビンに q が入り、範囲外は数える', () => {
    // 点 [x, y, t, q, view, track]。tinyGeometry: nx=9, ny=5, nBuckets=10。
    // (5, 4, 7): ix=6, iy=3, iz=7(上のビン index の手計算と同じ)。
    // (-9, -8, 0): ix=0, iy=0, iz=0。 (50, 0, 3): x が範囲外。
    const { projections, outOfRange } = fillTrackPoints(tinyGeometry(), 10, [
      [5, 4, 7, 2.5, 0, 0],
      [-9, -8, 0, 4, 1, 0],
      [50, 0, 3, 1, 2, 0],
    ]);
    expect(projections.xy.values[3 * 9 + 6]).toBe(2.5);
    expect(projections.xz.values[7 * 9 + 6]).toBe(2.5);
    expect(projections.yz.values[7 * 5 + 3]).toBe(2.5);
    expect(projections.xy.values[0]).toBe(4);
    expect(outOfRange).toBe(1);
    expect(sum(projections.xy.values)).toBe(6.5); // 範囲外の点は XY にも詰めない
  });

  it('点が無ければ空のヒスト(nBuckets は引数どおり)', () => {
    const { projections, outOfRange } = fillTrackPoints(tinyGeometry(), 10, []);
    expect(outOfRange).toBe(0);
    expect(sum(projections.xy.values)).toBe(0);
    expect(projections.xz.ny).toBe(10);
  });
});

describe('projectionBlocker — stripGeometry があり、ビンが退化していないこと', () => {
  it('普通の rangeXY → null', () => {
    expect(projectionBlocker(tinyGeometry())).toBeNull();
  });

  it('ビンが作れない rangeXY → degenerate', () => {
    const flat: StripGeometryMessage = { ...tinyGeometry(), rangeXY: [0, 1, 0, 1] };
    expect(projectionBlocker(flat)).toContain('degenerate projection bins');
  });
});
