/**
 * モニタビュー(SPEC §5.2 の 9 ヒスト + R9 のイベント表示)。
 *
 * - 3×3 グリッド: 行 = StripTime / Charge / ChargeMax、列 = U / V / W(028 発注書 2)。
 * - イベント表示 = `0x02 Uvw` の 3 面を **JSROOT の TH2** で(発注書 4)。
 *   イベント ID(run / eventNumber)と incomplete フラグは**タブに関係なく常時表示**。
 * - freeze / interval はクライアント側だけ(§5.4)。WS の購読は止めない。
 *
 * ビン数・strip 数・軸範囲は**一切書かない**。全部メッセージから来る。
 */
import { Component, computed, effect, inject, signal, untracked } from '@angular/core';
import { MatTabsModule } from '@angular/material/tabs';

import { DisplayClock } from '../display/display-clock';
import { DisplayControls } from '../display/display-controls';
import type { UvwMessage } from '../ws/wire';
import { WsClientService } from '../ws/ws-client';
import { JsrootPanel } from './jsroot-panel';
import { HISTO_ROWS, PLANE_NAMES, histoPanelSpec, uvwPanelSpec } from './root-histo';

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

  /** イベント表示の 3 面。 */
  readonly planes = PLANE_NAMES.map((plane, index) => ({
    plane,
    index,
    spec: uvwPanelSpec(index),
  }));

  private readonly histos = this.ws.histos;
  readonly uvw = this.ws.uvwByPlane;

  /** 3 面のうち最新のもの(3 面は同じイベントから作られる)。 */
  private readonly latestUvw = computed<UvwMessage | null>(() => {
    let latest: UvwMessage | null = null;
    for (const message of this.uvw()) {
      if (!message) continue;
      if (!latest || message.header.eventNumber >= latest.header.eventNumber) latest = message;
    }
    return latest;
  });

  /**
   * 画面に**今出ているイベント**。freeze / interval と同じ刻みで更新するので、
   * ラベルと絵が食い違わない(status バーは別物で、freeze 中も動き続ける)。
   */
  private readonly shownEvent = signal<UvwMessage | null>(null);
  private readonly clock = inject(DisplayClock);

  constructor() {
    effect(() => {
      this.clock.tick();
      this.shownEvent.set(untracked(this.latestUvw));
    });
  }

  /** R9: イベント ID は常時表示。 */
  readonly eventLabel = computed(() => {
    const message = this.shownEvent();
    if (!message) return 'event —';
    return `run ${message.header.runNumber} · event #${message.header.eventNumber}`;
  });

  readonly incomplete = computed(() => this.shownEvent()?.header.incomplete ?? false);
  readonly hasEvent = computed(() => this.shownEvent() !== null);

  histoMessage(id: number) {
    return this.histos().get(id) ?? null;
  }

  uvwMessage(index: number) {
    return this.uvw()[index] ?? null;
  }
}
