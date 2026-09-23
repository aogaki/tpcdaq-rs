/**
 * 090 — 波形ビューのグリッド表示レイアウト(純関数)。
 *
 * `gridShape` / `isDense` は Angular にも ECharts にも依存しない。系列数が
 * 増えるほど列数を増やし行を詰める(4 → 8 → 12 列)、という表側のレイアウト
 * 判断だけを持つ。
 */
import { gridShape, isDense } from './waveform-layout';

describe('gridShape(count) — 系列数から列数・行数・行の高さを決める', () => {
  it('16 系列以下は最大 4 列(現状)、行の高さ 190px', () => {
    // 手計算: sqrt(16) = 4 → cols = min(4, 4) = 4、rows = ceil(16/4) = 4。
    expect(gridShape(16)).toEqual({ cols: 4, rows: 4, rowHeightPx: 190 });
  });

  it('64 系列は最大 8 列、行の高さ 140px', () => {
    // 手計算: sqrt(64) = 8 → cols = min(8, 8) = 8、rows = ceil(64/8) = 8。
    expect(gridShape(64)).toEqual({ cols: 8, rows: 8, rowHeightPx: 140 });
  });

  it('272 系列(AsAd 1 枚分)は 12 列 × 23 行、行の高さ 110px', () => {
    // 手計算: sqrt(272) ≈ 16.49 → ceil 17 → cols = min(12, 17) = 12。
    // rows = ceil(272/12) = ceil(22.666…) = 23。
    expect(gridShape(272)).toEqual({ cols: 12, rows: 23, rowHeightPx: 110 });
  });

  it('小さい系列数は正方形に近い列数に収める(4 → 2 列 × 2 行)', () => {
    // 手計算: sqrt(4) = 2 → cols = min(4, 2) = 2、rows = ceil(4/2) = 2。
    expect(gridShape(4)).toEqual({ cols: 2, rows: 2, rowHeightPx: 190 });
  });

  it('count=0 でも例外を投げない(cols/rows は最低 1)', () => {
    expect(gridShape(0)).toEqual({ cols: 1, rows: 1, rowHeightPx: 190 });
  });
});

describe('isDense(seriesCount, pointsPerSeries) — overlay で lttb/large を使うかの判定', () => {
  it('64 系列 × 512 点 = 32,768 点 → 密ではない', () => {
    expect(isDense(64, 512)).toBe(false);
  });

  it('272 系列 × 512 点 = 139,264 点 → 密', () => {
    expect(isDense(272, 512)).toBe(true);
  });

  it('ちょうど 100,000 点は境界(超えたら、なので密ではない)', () => {
    // 手計算: 200 * 500 = 100,000ちょうど。
    expect(isDense(200, 500)).toBe(false);
  });

  it('100,000 を 1 点超えたら密', () => {
    // 手計算: 200 * 500 + 1 = 100,001。系列数はそのまま、点数を +1 して境界を越える。
    expect(isDense(200, 501)).toBe(true);
  });
});
