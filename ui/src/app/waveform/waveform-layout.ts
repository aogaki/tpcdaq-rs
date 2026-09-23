/**
 * 090 — 波形ビューの `grid` 表示レイアウト(純関数)。
 *
 * Angular にも ECharts にも依存しない(vitest から直接 import する)。
 * AsAd 1 枚分(4 AGET × 68 ch = 272 系列)まで系列数が増えても、小グリッドが
 * 画面に収まるように列数と行の高さを段階的に詰める。
 */

export interface GridShape {
  readonly cols: number;
  readonly rows: number;
  /** 1 行あたりの高さ(px)。`chartHeightPx = rows * rowHeightPx` に使う。 */
  readonly rowHeightPx: number;
}

/**
 * 系列数から列数・行数・行の高さを決める。
 * - count ≤ 16: 最大 4 列、190 px/行(現状)
 * - count ≤ 64: 最大 8 列、140 px/行
 * - それ以上(最大 272 = AsAd 1 枚分): 12 列、110 px/行
 *
 * 列数は `min(上限, ceil(sqrt(count)))` — 系列が少ないうちは正方形に近い
 * 並びにし、上限に達したら列数を固定する(現状の 4 列ロジックをそのまま延長)。
 */
export function gridShape(count: number): GridShape {
  const cap = count <= 16 ? 4 : count <= 64 ? 8 : 12;
  const rowHeightPx = count <= 16 ? 190 : count <= 64 ? 140 : 110;
  const cols = Math.max(1, Math.min(cap, Math.ceil(Math.sqrt(count))));
  const rows = Math.max(1, Math.ceil(count / cols));
  return { cols, rows, rowHeightPx };
}

/**
 * overlay 表示で `sampling: 'lttb'` / `large: true`(ECharts 6 の line series)を
 * 使うべきかどうか。系列数 × 1 系列あたりの点数が 100,000 を超えたら密とみなす。
 */
export function isDense(seriesCount: number, pointsPerSeries: number): boolean {
  return seriesCount * pointsPerSeries > 100_000;
}
