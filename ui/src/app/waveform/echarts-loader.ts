/**
 * ECharts の**遅延ロード**(SPEC §11 / 028 発注書 1)。
 *
 * ngx-echarts のようなラッパは入れない(KISS)。素の動的 `import()` 1 か所だけ。
 * 解決済み Promise をモジュールに持つので取得は 1 回。
 */

export type EChartsModule = typeof import('echarts');
export type EChartsInstance = ReturnType<EChartsModule['init']>;

let pending: Promise<EChartsModule> | null = null;

export function loadECharts(): Promise<EChartsModule> {
  pending ??= import('echarts');
  return pending;
}
