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
import { ControllerApiService } from '../api/controller-api';
import { parseControllerStatus } from '../api/controller-status';
import { RunControlApiService } from '../api/run-control-api';
import { describeError } from '../logbook/logbook-feed';
import type { SlowPayload } from '../ws/json';
import { WsClientService } from '../ws/ws-client';
import { observeResize } from '../display/resize-observer';
import {
  MAX_TREND_SERIES,
  type SlowHistoryMap,
  appendSlowSamples,
  capSeriesSelection,
  formatAgeMs,
  formatSlowValue,
} from './slow-history';
import {
  type PowerDialog,
  type PowerOutcome,
  buildDialog,
  buttonLabel,
  disabledReason,
  interpretOutcome,
  targetOn,
} from './slow-power';
import { type ChannelRow, type SlowTable, buildSlowTable } from './slow-table';

/** controller の相を取り直す間隔(ボタンの有効/無効用。押した瞬間にも取り直す)。 */
const PHASE_POLL_MS = 5000;

/** age を再計算する刻み(Latest 表の「age」列を進めるだけ — 購読やデータの取得間隔ではない)。 */
const AGE_TICK_MS = 1000;

@Component({
  selector: 'app-slow-view',
  templateUrl: './slow-view.html',
  styleUrl: './slow-view.scss',
})
export class SlowView implements OnDestroy {
  private readonly ws = inject(WsClientService);
  private readonly api = inject(ControllerApiService);
  private readonly control = inject(RunControlApiService);

  readonly maxTrendSeries = MAX_TREND_SERIES;

  /** slowctl の最新ペイロード。`undefined` = `[monitor] slow_sub` 未設定か slowctl 未起動。 */
  readonly slow = computed<SlowPayload | null>(() => this.ws.status()?.slow ?? null);
  readonly noData = computed(() => this.slow() === null);

  private readonly nowMs = signal(Date.now());
  private readonly ageTimer = setInterval(() => this.nowMs.set(Date.now()), AGE_TICK_MS);

  /** Latest: ch 表(機器 × ch 1 行)+ 列に写らないサンプルの Other 表(TODO/120)。 */
  readonly table = computed<SlowTable>(() => {
    const slow = this.slow();
    return slow ? buildSlowTable(slow.samples, this.nowMs()) : { channels: [], other: [] };
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
    void this.refreshPhase();
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

  // ---- TODO/124 HV / LV の ON/OFF(SPEC §11 v1.42 の唯一の例外) ----

  /** controller の相。取れていなければ null(Idle 必須の操作は無効のまま)。 */
  readonly phase = signal<string | null>(null);
  private readonly phaseTimer = setInterval(() => void this.refreshPhase(), PHASE_POLL_MS);
  private readonly sending = signal(false);
  /** 確認ダイアログの対象。null = 閉じている。 */
  readonly pending = signal<{ row: ChannelRow; on: boolean; dialog: PowerDialog } | null>(null);
  readonly outcome = signal<PowerOutcome | null>(null);
  private readonly powerDialog = viewChild<ElementRef<HTMLDialogElement>>('powerDialog');

  private readonly powerCtx = computed(() => ({
    hasToken: this.control.token() !== null,
    phase: this.phase(),
    busy: this.sending(),
  }));

  powerLabel(row: ChannelRow): string {
    return buttonLabel(row);
  }

  /** 押せない理由(押せるなら null)。 */
  powerDisabled(row: ChannelRow): string | null {
    return disabledReason(row, this.powerCtx());
  }

  async refreshPhase(): Promise<void> {
    try {
      const view = parseControllerStatus(await this.api.getStatus());
      this.phase.set(view?.phase ?? null);
    } catch {
      this.phase.set(null); // 取れない = 不明。Idle とは見なさない。
    }
  }

  /** ボタン。相を取り直してから確認ダイアログを開く(古い相で注意文を作らない)。 */
  async askPower(row: ChannelRow): Promise<void> {
    await this.refreshPhase();
    if (this.powerDisabled(row) !== null) return;
    const dialog = buildDialog(row, this.table().channels, this.phase());
    this.outcome.set(null);
    this.pending.set({ row, on: targetOn(row), dialog });
    this.powerDialog()?.nativeElement.showModal();
  }

  cancelPower(): void {
    this.powerDialog()?.nativeElement.close();
    this.pending.set(null);
  }

  async confirmPower(): Promise<void> {
    const target = this.pending();
    this.cancelPower();
    if (!target || this.sending()) return;
    this.sending.set(true);
    try {
      const result = await this.control.postSlowPower(target.row.device, target.row.ch, target.on);
      this.outcome.set(interpretOutcome(target.row, target.on, result));
    } catch (error) {
      this.outcome.set({
        ok: false,
        text: `${target.row.device} ch ${target.row.ch}: ${describeError(error)}`,
        notes: [],
      });
    } finally {
      this.sending.set(false);
    }
  }

  ngOnDestroy(): void {
    clearInterval(this.ageTimer);
    clearInterval(this.phaseTimer);
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
