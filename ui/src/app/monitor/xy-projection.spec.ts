/**
 * TODO/125 — XY / XZ / YZ のビン(`values[iy*nx+ix]`)と、計算しない状態の理由。
 *
 * 何を詰めるか(macros 移植の直線トラック)は `track-fit.spec.ts`。値は手計算(各ケースのコメント)。
 */
import type { StripGeometryMessage, StripPlaneGeometry } from '../ws/json';
import { emptyEvent, layoutOf, MINI_ANGLES, syntheticGeometry } from './track-test-fixtures';
import { type Projections, createProjections, fillPoint, projectionBlocker } from './xy-projection';

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

describe('projectionBlocker — 計算しない理由を文で', () => {
  const layout = layoutOf(MINI_ANGLES);
  const geometry = syntheticGeometry(layout);

  it('3 面が揃っていない → 足りない面の名前', () => {
    const uvw = emptyEvent(layout, 50);
    expect(projectionBlocker([uvw[0], null, uvw[2]], geometry)).toBe('waiting for V (0x02)');
    expect(projectionBlocker([null, null, null], geometry)).toBe('waiting for U/V/W (0x02)');
    expect(projectionBlocker(uvw, geometry)).toBeNull();
  });

  it('eventNumber が違う → 3 面の番号が出る', () => {
    const uvw = emptyEvent(layout, 50);
    const other = { ...uvw[1], header: { ...uvw[1].header, eventNumber: 2 } };
    expect(projectionBlocker([uvw[0], other, uvw[2]], geometry)).toBe(
      'U/V/W are from different events (#1 / #2 / #1)',
    );
  });

  it('bucket 数が面で違う → 3 面の bucket 数が出る', () => {
    const uvw = emptyEvent(layout, 50);
    const other = { ...uvw[2], nBuckets: 40 };
    expect(projectionBlocker([uvw[0], uvw[1], other], geometry)).toBe(
      'U/V/W bucket counts differ (50 / 50 / 40)',
    );
  });

  it('ビンが作れない rangeXY → degenerate', () => {
    const uvw = emptyEvent(layout, 50);
    const flat: StripGeometryMessage = { ...geometry, rangeXY: [0, 1, 0, 1] };
    expect(projectionBlocker(uvw, flat)).toContain('degenerate projection bins');
  });
});
