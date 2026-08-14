/**
 * 波形ビュー(R13 / SPEC §10.2 `0x03 Waveforms`)。
 *
 * - **表示中だけ** `waveforms` を購読する(027 の `ws.setWaveforms`)。
 * - 選択は 3 段: **(cobo,asad) → AGET → ch**。`nAget` / `nCh` / `nBuckets` は
 *   メッセージ由来(焼き込み禁止)。
 * - **生 ADC・FPN 込み・減算なし**をそのまま描く(R13)。
 * - クライアント側で間引く(系列数の上限 + サンプルの stride)。
 *   **間引いた件数は必ず画面に出す**(モニタ系は落としてよいが silent にしない)。
 * - 描画の刻みは `DisplayClock`(freeze / interval)。購読は止めない。
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

import { DisplayControls } from '../display/display-controls';
import type { WaveformsMessage } from '../ws/wire';
import { WsClientService } from '../ws/ws-client';
import { DisplayClock } from '../display/display-clock';
import { type EChartsInstance, loadECharts } from './echarts-loader';
import {
  type WaveformPlot,
  type WaveformSelection,
  selectWaveforms,
  updateWaveformCache,
} from './waveform-select';

/** 描画系列数の上限の選択肢(表示側の都合。検出器の値ではない)。 */
const SERIES_LIMITS = [4, 8, 16, 32, 64] as const;
/** 1 系列あたりの点数の上限の選択肢。 */
const POINT_LIMITS = [128, 256, 512, 1024] as const;

type PlotMode = 'overlay' | 'grid';

@Component({
  selector: 'app-waveform-view',
  imports: [DisplayControls],
  templateUrl: './waveform-view.html',
  styleUrl: './waveform-view.scss',
})
export class WaveformView implements OnDestroy {
  private readonly ws = inject(WsClientService);
  private readonly clock = inject(DisplayClock);

  readonly seriesLimits = SERIES_LIMITS;
  readonly pointLimits = POINT_LIMITS;

  /** (cobo,asad) 毎の最新(`ws.waveforms()` は最新 1 通しか持たないため)。 */
  private readonly cache = signal<ReadonlyMap<string, WaveformsMessage>>(new Map());
  readonly keys = computed(() => [...this.cache().keys()].sort());

  readonly selectedKey = signal<string | null>(null);
  readonly agets = signal<readonly number[]>([]);
  readonly chFrom = signal(0);
  readonly chTo = signal(0);
  readonly maxSeries = signal<number>(16);
  readonly maxPoints = signal<number>(512);
  readonly mode = signal<PlotMode>('overlay');

  /** 実際に描いた結果(間引きの申告はここから出す = 画面と一致する)。 */
  readonly rendered = signal<WaveformPlot | null>(null);
  readonly issue = signal<string | null>('waiting for waveforms (0x03)');

  /** 選ばれている (cobo,asad) の最新メッセージ。 */
  readonly message = computed(() => {
    const key = this.selectedKey();
    return key === null ? null : (this.cache().get(key) ?? null);
  });

  readonly selection = computed<WaveformSelection>(() => ({
    agets: this.agets(),
    channels: rangeInclusive(this.chFrom(), this.chTo()),
    maxSeries: this.maxSeries(),
    maxPoints: this.maxPoints(),
  }));

  /** メッセージ由来の AGET 一覧(焼き込み禁止)。 */
  readonly agetChoices = computed(() => {
    const message = this.message();
    return message ? [...Array(message.nAget).keys()] : [];
  });

  /**
   * 画面に**今出ているイベント**(描いたときに更新する)。freeze 中にラベルだけ
   * 進んで絵と食い違う、という嘘をつかないため。
   */
  private readonly shownMessage = signal<WaveformsMessage | null>(null);

  readonly eventLabel = computed(() => {
    const message = this.shownMessage();
    if (!message) return 'event —';
    return `run ${message.header.runNumber} · event #${message.header.eventNumber}`;
  });

  readonly incomplete = computed(() => this.shownMessage()?.header.incomplete ?? false);

  private readonly host = viewChild<ElementRef<HTMLElement>>('chart');
  private chart: EChartsInstance | null = null;
  private observer: ResizeObserver | null = null;
  private busy = false;
  private defaultsApplied = false;

  constructor() {
    // 表示中だけ 0x03 を流してもらう(§10.3 の既定は waveforms OFF)。
    this.ws.setWaveforms(true);

    effect(() => {
      const message = this.ws.waveforms();
      if (!message) return;
      this.cache.set(updateWaveformCache(untracked(this.cache), message));
      this.applyDefaults(message);
    });

    effect(() => {
      // freeze 中は tick が進まない = 描き直さない(§5.4)。
      this.clock.tick();
      // 選択・モードの変更は刻みを待たずに反映する(ユーザー操作なので)。
      const selection = this.selection();
      const mode = this.mode();
      void this.render(untracked(this.message), selection, mode);
    });

    effect(() => {
      const host = this.host()?.nativeElement;
      if (!host || this.observer) return;
      this.observer = new ResizeObserver(() => this.chart?.resize());
      this.observer.observe(host);
    });
  }

  ngOnDestroy(): void {
    // 離脱したら購読を戻す(帯域制御 — §10.3)。
    this.ws.setWaveforms(false);
    this.observer?.disconnect();
    this.observer = null;
    this.chart?.dispose();
    this.chart = null;
  }

  /** グリッド表示では系列数に応じて高さを伸ばす。 */
  readonly chartHeightPx = computed(() => {
    const plot = this.rendered();
    if (this.mode() === 'overlay' || !plot || plot.series.length === 0) return 520;
    return Math.max(320, gridShape(plot.series.length).rows * 190);
  });

  selectKey(event: Event): void {
    this.selectedKey.set((event.target as HTMLSelectElement).value);
  }

  toggleAget(aget: number): void {
    const current = this.agets();
    this.agets.set(
      current.includes(aget)
        ? current.filter((value) => value !== aget)
        : [...current, aget].sort(),
    );
  }

  setChFrom(event: Event): void {
    this.chFrom.set(numberFrom(event, this.chFrom()));
  }

  setChTo(event: Event): void {
    this.chTo.set(numberFrom(event, this.chTo()));
  }

  setMaxSeries(event: Event): void {
    this.maxSeries.set(numberFrom(event, this.maxSeries()));
  }

  setMaxPoints(event: Event): void {
    this.maxPoints.set(numberFrom(event, this.maxPoints()));
  }

  allChannels(): void {
    const message = this.message();
    if (!message) return;
    this.chFrom.set(0);
    this.chTo.set(message.nCh - 1);
  }

  /** 最初のメッセージが来たときだけ、選択の既定をメッセージから作る。 */
  private applyDefaults(message: WaveformsMessage): void {
    if (this.selectedKey() === null) {
      this.selectedKey.set([...this.cache().keys()].sort()[0] ?? null);
    }
    if (this.defaultsApplied) return;
    this.defaultsApplied = true;
    this.agets.set([...Array(message.nAget).keys()]);
    this.chFrom.set(0);
    this.chTo.set(message.nCh - 1);
  }

  private async render(
    message: WaveformsMessage | null,
    selection: WaveformSelection,
    mode: PlotMode,
  ): Promise<void> {
    const host = this.host()?.nativeElement;
    if (!host) return;
    if (!message) {
      this.issue.set('waiting for waveforms (0x03)');
      return;
    }
    if (this.busy) return;

    this.busy = true;
    try {
      const echarts = await loadECharts();
      this.chart ??= echarts.init(host, null, { renderer: 'canvas' });
      const plot = selectWaveforms(message, selection);
      this.chart.setOption(buildOption(plot, mode), { notMerge: true });
      this.chart.resize();
      this.rendered.set(plot);
      this.shownMessage.set(message);
      this.issue.set(plot.series.length === 0 ? 'no channel selected' : null);
    } catch (error) {
      this.issue.set(`chart failed: ${String(error)}`);
    } finally {
      this.busy = false;
    }
  }
}

function numberFrom(event: Event, fallback: number): number {
  const value = Number((event.target as HTMLInputElement | HTMLSelectElement).value);
  return Number.isFinite(value) ? value : fallback;
}

function rangeInclusive(from: number, to: number): number[] {
  const lo = Math.max(0, Math.min(from, to));
  const hi = Math.max(from, to);
  const out: number[] = [];
  for (let value = lo; value <= hi; value++) out.push(value);
  return out;
}

/** グリッド表示の並び(横は最大 4 列)。 */
function gridShape(count: number): { cols: number; rows: number } {
  const cols = Math.max(1, Math.min(4, Math.ceil(Math.sqrt(count))));
  return { cols, rows: Math.max(1, Math.ceil(count / cols)) };
}

const AXIS_COLOR = '#7f8c99';
const TEXT_COLOR = '#cfd8dc';

function seriesName(aget: number, ch: number): string {
  return `A${aget}/ch${ch}`;
}

function buildOption(plot: WaveformPlot, mode: PlotMode): EChartsOption {
  const common = {
    backgroundColor: 'transparent',
    animation: false,
    textStyle: { color: TEXT_COLOR },
  };
  const axisCommon = {
    axisLine: { lineStyle: { color: AXIS_COLOR } },
    splitLine: { lineStyle: { color: 'rgba(127,140,153,0.2)' } },
  };

  if (mode === 'overlay') {
    return {
      ...common,
      grid: { left: 64, right: 24, top: 28, bottom: 56 },
      tooltip: { trigger: 'axis' },
      legend: { type: 'scroll', bottom: 0, textStyle: { color: TEXT_COLOR } },
      xAxis: {
        type: 'value',
        name: 'time bucket',
        min: 0,
        max: Math.max(0, plot.nBuckets - 1),
        ...axisCommon,
      },
      yAxis: { type: 'value', name: 'raw ADC', ...axisCommon },
      series: plot.series.map((entry) => ({
        name: seriesName(entry.aget, entry.ch),
        type: 'line' as const,
        showSymbol: false,
        lineStyle: { width: 1 },
        data: entry.points,
      })),
    };
  }

  const { cols, rows } = gridShape(plot.series.length);
  const cellW = 100 / cols;
  const cellH = 100 / rows;
  return {
    ...common,
    tooltip: { trigger: 'axis' },
    title: plot.series.map((entry, index) => ({
      text: seriesName(entry.aget, entry.ch),
      left: `${(index % cols) * cellW + 4}%`,
      top: `${Math.floor(index / cols) * cellH + 1}%`,
      textStyle: { fontSize: 11, color: TEXT_COLOR },
    })),
    grid: plot.series.map((_entry, index) => ({
      left: `${(index % cols) * cellW + 6}%`,
      top: `${Math.floor(index / cols) * cellH + 5}%`,
      width: `${cellW - 10}%`,
      height: `${cellH - 10}%`,
    })),
    xAxis: plot.series.map((_entry, index) => ({
      gridIndex: index,
      type: 'value' as const,
      min: 0,
      max: Math.max(0, plot.nBuckets - 1),
      ...axisCommon,
    })),
    yAxis: plot.series.map((_entry, index) => ({
      gridIndex: index,
      type: 'value' as const,
      ...axisCommon,
    })),
    series: plot.series.map((entry, index) => ({
      name: seriesName(entry.aget, entry.ch),
      type: 'line' as const,
      xAxisIndex: index,
      yAxisIndex: index,
      showSymbol: false,
      lineStyle: { width: 1 },
      data: entry.points,
    })),
  };
}
