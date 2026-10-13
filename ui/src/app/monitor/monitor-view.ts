/**
 * モニタビュー(SPEC §5.2 の 9 ヒスト + R9 のイベント表示 + TODO/125 の XY/XZ/YZ 投影)。
 *
 * - 3×3 グリッド: 行 = StripTime / Charge / ChargeMax、列 = U / V / W(028 発注書 2)。
 * - イベント表示 = 1 段目に `0x02 Uvw` の 3 面(**JSROOT の TH2**)、2 段目に XY / XZ / YZ 投影。
 *   投影は **UI で計算する表示専用の変換**(`xy-projection.ts`)。保存系にもワイヤにも触らない。
 *   計算は表示刻み(DisplayClock の tick)に同期し、Event display タブが開いているときだけ。
 *   同じ (イベント, 閾値, ジオメトリ) では再計算しない(入力が変わらなければシグナルが変わらない)。
 *   イベント ID(run / eventNumber)と incomplete フラグは**タブに関係なく常時表示**。
 * - AsAd 電子回路座標パネル(077)は Waveform ビューへ移した。monitor タブは 0x03 を購読しない。
 * - freeze / interval はクライアント側だけ(§5.4)。WS の購読は止めない。
 *
 * ビン数・strip 数・軸範囲は**一切書かない**。全部メッセージから来る。
 */
import { Component, computed, effect, inject, signal, untracked } from '@angular/core';
import { MatTabsModule } from '@angular/material/tabs';

import { DisplayClock } from '../display/display-clock';
import { DisplayControls } from '../display/display-controls';
import { isStripGeometryUnavailable } from '../ws/json';
import type { UvwMessage } from '../ws/wire';
import { WsClientService } from '../ws/ws-client';
import { isValidThreshold, loadThreshold, saveThreshold } from './event-display-settings';
import { JsrootPanel } from './jsroot-panel';
import {
  HISTO_ROWS,
  PLANE_NAMES,
  PROJECTION_SPECS,
  type ProjectionMessage,
  histoPanelSpec,
  uvwPanelSpec,
} from './root-histo';
import {
  MAX_CROSS_EVALUATIONS,
  PROJECTION_RADIUS_MM,
  type ProjectionResult,
  projectEvent,
  projectionBlocker,
} from './xy-projection';

/** `mat-tab-group` 内の Event display タブの位置(Histograms = 0)。 */
const EVENT_TAB_INDEX = 1;

@Component({
  selector: 'app-monitor-view',
  imports: [MatTabsModule, DisplayControls, JsrootPanel],
  templateUrl: './monitor-view.html',
  styleUrl: './monitor-view.scss',
})
export class MonitorView {
  private readonly ws = inject(WsClientService);

  /** 3×3 グリッド(§5.2 の表そのもの)。 */
  readonly rows = HISTO_ROWS.map((row) => ({
    row,
    cells: PLANE_NAMES.map((plane, index) => ({
      plane,
      id: row.ids[index],
      spec: histoPanelSpec(row, index),
    })),
  }));

  /** イベント表示の 1 段目(U/V/W)。 */
  readonly planes = PLANE_NAMES.map((plane, index) => ({
    plane,
    index,
    spec: uvwPanelSpec(index),
  }));

  /** イベント表示の 2 段目(XY / XZ / YZ)。 */
  readonly projectionPanels = PROJECTION_SPECS.map((spec, index) => ({ index, spec }));

  private readonly histos = this.ws.histos;
  readonly uvw = this.ws.uvwByPlane;

  /**
   * 画面に**今出ているイベント**の 3 面(0x02)。freeze / interval と同じ刻みで更新するので、
   * ラベルと絵が食い違わない(status バーは別物で、freeze 中も動き続ける)。
   */
  private readonly shownUvw = signal<readonly (UvwMessage | null)[]>([null, null, null]);
  private readonly clock = inject(DisplayClock);

  /** 3 面のうち最新のもの(3 面は同じイベントから作られる)。 */
  private readonly shownEvent = computed<UvwMessage | null>(() => {
    let latest: UvwMessage | null = null;
    for (const message of this.shownUvw()) {
      if (!message) continue;
      if (!latest || message.header.eventNumber >= latest.header.eventNumber) latest = message;
    }
    return latest;
  });

  /** TODO/125: 投影の閾値 [ADC](ブラウザごとに localStorage へ記憶。使えなくても既定値で動く)。 */
  readonly threshold = signal(loadThreshold());
  readonly radiusMm = PROJECTION_RADIUS_MM;

  /** Event display タブが開いているか(閉じているときは投影を計算しない)。 */
  private readonly eventTabOpen = signal(false);
  private readonly projection = signal<ProjectionResult | null>(null);
  /** 投影を計算した回数(診断用。同じ入力で増えないことをテストで確かめる)。 */
  readonly projectionRuns = signal(0);

  constructor() {
    // 注意: 0x03(AsAd)は購読しない。Waveform ビューが表示中だけ購読する(§10.3)。
    effect(() => {
      this.clock.tick();
      this.shownUvw.set(untracked(this.ws.uvwByPlane));
    });

    effect(() => {
      const open = this.eventTabOpen();
      const uvw = this.shownUvw();
      const geometry = this.ws.stripGeometry();
      const threshold = this.threshold();
      if (!open) return;
      untracked(() => {
        if (geometry === null || isStripGeometryUnavailable(geometry)) {
          this.projection.set(null);
          return;
        }
        if (projectionBlocker(uvw, geometry) !== null) {
          this.projection.set(null);
          return;
        }
        this.projection.set(
          projectEvent(uvw, geometry, { threshold, radius: PROJECTION_RADIUS_MM }),
        );
        this.projectionRuns.update((count) => count + 1);
      });
    });
  }

  onTabChange(index: number): void {
    this.eventTabOpen.set(index === EVENT_TAB_INDEX);
  }

  /** 閾値欄の確定。不正値(空・負・非数)は受け付けず、欄を今の値へ戻す。 */
  onThreshold(event: Event): void {
    const input = event.target as HTMLInputElement;
    const value = input.value.trim() === '' ? Number.NaN : Number(input.value);
    if (!isValidThreshold(value)) {
      input.value = String(this.threshold());
      return;
    }
    this.threshold.set(value);
    saveThreshold(value);
  }

  /** R9: イベント ID は常時表示。 */
  readonly eventLabel = computed(() => {
    const message = this.shownEvent();
    if (!message) return 'event —';
    return `run ${message.header.runNumber} · event #${message.header.eventNumber}`;
  });

  readonly incomplete = computed(() => this.shownEvent()?.header.incomplete ?? false);
  readonly hasEvent = computed(() => this.shownEvent() !== null);

  /** XY / XZ / YZ に渡すメッセージ(計算できていなければ 3 枚とも `null`)。 */
  readonly projectionMessages = computed<readonly (ProjectionMessage | null)[]>(() => {
    const result = this.projection();
    if (!result) return [null, null, null];
    return [result.xy, result.xz, result.yz].map((h) => ({ kind: 'projection' as const, ...h }));
  });

  /**
   * 投影の状態を 1 行で(silent 禁止): 未着・unavailable・計算できない理由・0 点・capped・incomplete を
   * それぞれ文で出す。
   */
  readonly projectionStatus = computed(() => {
    const geometry = this.ws.stripGeometry();
    if (geometry === null) {
      return 'strip geometry not received yet (monitor sends it once, right after meta) — no projection';
    }
    if (isStripGeometryUnavailable(geometry)) {
      return `strip geometry unavailable: ${geometry.unavailable}`;
    }
    const blocker = projectionBlocker(this.shownUvw(), geometry);
    if (blocker !== null) return `no projection: ${blocker}`;
    const result = this.projection();
    if (!result) return 'projection pending';

    const threshold = this.threshold();
    const parts: string[] = [
      result.points === 0
        ? `0 points (no U/V/W coincidence in any time bucket)`
        : `${result.points} points (U/V/W coincidences)`,
      `threshold ${threshold} ADC`,
      `radius ${this.radiusMm} mm`,
    ];
    if (result.points === 0 && !result.capped) parts.push('try a lower threshold');
    if (result.capped) {
      parts.push(
        `CAPPED after ${MAX_CROSS_EVALUATIONS.toLocaleString('en-US')} crossPoint evaluations ` +
          '(partial result) — raise the threshold',
      );
    }
    if (result.outOfRange > 0) parts.push(`${result.outOfRange} points outside the plot range`);
    if (result.incomplete) parts.push('INCOMPLETE (fragments missing — charges are short)');
    return parts.join(' · ');
  });

  histoMessage(id: number) {
    return this.histos().get(id) ?? null;
  }

  uvwMessage(index: number) {
    return this.uvw()[index] ?? null;
  }
}
