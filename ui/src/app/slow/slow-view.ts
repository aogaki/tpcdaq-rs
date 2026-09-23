/**
 * Slow control ビュー(TODO/079 §2.8 — ガス系 Bronkhorst FLOW-BUS / 環境 Arduino)。
 *
 * `power/` の置き換え(P6 の PSU プレースホルダはここでは扱わない — 別の話)。
 *
 * SPEC §11 安全境界: **読み取り・記録・アラームのみ**。setpoint / バルブ / HV を動かす
 * ボタンは一切無い(存在させない、既定でグレーアウトではなく型として無い)。
 *
 * 描画(ECharts の遅延ロード)は `waveform-view.ts` と同じ流儀で untested(jsdom に
 * canvas 2D が無く、`jsroot-loader.spec.ts` / `root-histo.spec.ts` 同様、純ロジックは
 * `slow-history.ts` 側でテストし、ここは配線だけにする)。
 */
import {
  Component,
  ElementRef,
  OnDestroy,
  computed,
  effect,
  inject,
  signal,
  untracked,
  viewChild,
} from '@angular/core';
import type { EChartsOption } from 'echarts';

import { type EChartsInstance, loadECharts } from '../waveform/echarts-loader';
import type { SlowPayload } from '../ws/json';
import { WsClientService } from '../ws/ws-client';
import { observeResize } from '../display/resize-observer';
import {
  MAX_TREND_SERIES,
  type SlowHistoryMap,
  type SlowRow,
  appendSlowSamples,
  capSeriesSelection,
  formatAgeMs,
  formatSlowValue,
  sortedSlowRows,
} from './slow-history';

/** age を再計算する刻み(Latest 表の「age」列を進めるだけ — 購読やデータの取得間隔ではない)。 */
const AGE_TICK_MS = 1000;

@Component({
  selector: 'app-slow-view',
  templateUrl: './slow-view.html',
  styleUrl: './slow-view.scss',
})
export class SlowView implements OnDestroy {
  private readonly ws = inject(WsClientService);

  readonly maxTrendSeries = MAX_TREND_SERIES;

  /** slowctl の最新ペイロード。`undefined` = `[monitor] slow_sub` 未設定か slowctl 未起動。 */
  readonly slow = computed<SlowPayload | null>(() => this.ws.status()?.slow ?? null);
  readonly noData = computed(() => this.slow() === null);

  private readonly nowMs = signal(Date.now());
  private readonly ageTimer = setInterval(() => this.nowMs.set(Date.now()), AGE_TICK_MS);

  readonly rows = computed<readonly SlowRow[]>(() => {
    const slow = this.slow();
    return slow ? sortedSlowRows(slow.samples, this.nowMs()) : [];
  });

  private readonly history = signal<SlowHistoryMap>(new Map());
  readonly seriesKeys = computed(() => [...this.history().keys()].sort());
  readonly selectedKeys = signal<readonly string[]>([]);
  readonly issue = signal<string | null>(null);

  private lastSlowTsMs: number | null = null;
  private defaultsApplied = false;

  private readonly host = viewChild<ElementRef<HTMLElement>>('chart');
  private chart: EChartsInstance | null = null;
  private busy = false;

  constructor() {
    // 到着ごとにリングバッファへ足す(同じ ts_ms を二重に足さない — slowctl の poll_hz より
    // status の 1 Hz 集約が速い場合の重複防止)。
    effect(() => {
      const slow = this.slow();
      if (!slow) return;
      if (slow.ts_ms === untracked(() => this.lastSlowTsMs)) return;
      this.lastSlowTsMs = slow.ts_ms;
      this.history.set(appendSlowSamples(untracked(this.history), slow.samples));
      this.applyDefaultSelection();
    });

    effect(() => {
      const keys = this.selectedKeys();
      const history = this.history();
      void this.render(keys, history);
    });

    observeResize(
      () => this.host()?.nativeElement,
      () => this.chart?.resize(),
    );
  }

  ngOnDestroy(): void {
    clearInterval(this.ageTimer);
    this.chart?.dispose();
    this.chart = null;
  }

  /** 選ばれていなければ足す、選ばれていれば外す。上限超えは silent に落とさず issue に出す。 */
  toggleSeries(key: string): void {
    const current = this.selectedKeys();
    if (current.includes(key)) {
      this.selectedKeys.set(current.filter((value) => value !== key));
      return;
    }
    const { kept, droppedCount } = capSeriesSelection([...current, key]);
    if (droppedCount > 0) {
      this.issue.set(`max ${MAX_TREND_SERIES} trend series — deselect one first`);
      return;
    }
    this.issue.set(null);
    this.selectedKeys.set(kept);
  }

  formatValue(value: number): string {
    return formatSlowValue(value);
  }

  formatAge(ageMs: number): string {
    return formatAgeMs(ageMs);
  }

  /** 最初にデータが来たときだけ、系列の既定(先頭 1 本)を選ぶ。 */
  private applyDefaultSelection(): void {
    if (this.defaultsApplied) return;
    const keys = [...this.history().keys()].sort();
    if (keys.length === 0) return;
    this.defaultsApplied = true;
    this.selectedKeys.set([keys[0]]);
  }

  private async render(keys: readonly string[], history: SlowHistoryMap): Promise<void> {
    const host = this.host()?.nativeElement;
    if (!host || keys.length === 0) return;
    if (this.busy) return;

    this.busy = true;
    try {
      const echarts = await loadECharts();
      this.chart ??= echarts.init(host, null, { renderer: 'canvas' });
      this.chart.setOption(buildOption(keys, history), { notMerge: true });
      this.chart.resize();
    } catch (error) {
      this.issue.set(`chart failed: ${String(error)}`);
    } finally {
      this.busy = false;
    }
  }
}

const AXIS_COLOR = '#7f8c99';
const TEXT_COLOR = '#cfd8dc';

function buildOption(keys: readonly string[], history: SlowHistoryMap): EChartsOption {
  const axisCommon = {
    axisLine: { lineStyle: { color: AXIS_COLOR } },
    splitLine: { lineStyle: { color: 'rgba(127,140,153,0.2)' } },
  };
  return {
    backgroundColor: 'transparent',
    animation: false,
    textStyle: { color: TEXT_COLOR },
    tooltip: { trigger: 'axis' },
    legend: { type: 'scroll', bottom: 0, textStyle: { color: TEXT_COLOR } },
    grid: { left: 64, right: 24, top: 28, bottom: 56 },
    xAxis: { type: 'time', ...axisCommon },
    yAxis: { type: 'value', ...axisCommon },
    series: keys.map((key) => ({
      name: key,
      type: 'line' as const,
      showSymbol: false,
      lineStyle: { width: 1 },
      data: (history.get(key) ?? []).map((point) => [point.ts_ms, point.value]),
    })),
  };
}
