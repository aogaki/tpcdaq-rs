/**
 * モニタビュー(SPEC §5.2 の 9 ヒスト + R9 のイベント表示 + 077 の AsAd パネル)。
 *
 * - 3×3 グリッド: 行 = StripTime / Charge / ChargeMax、列 = U / V / W(028 発注書 2)。
 * - イベント表示 = `0x02 Uvw` の 3 面を **JSROOT の TH2** で(発注書 4)。
 *   イベント ID(run / eventNumber)と incomplete フラグは**タブに関係なく常時表示**。
 * - イベント表示の下に 4 枚目として AsAd 電子回路座標(`0x03 Waveforms`)を並べる(077)。
 *   表示中だけ `ws.setWaveforms(true)`(離脱で false — 波形ビューと同じ帯域制御 §10.3)。
 * - freeze / interval はクライアント側だけ(§5.4)。WS の購読は止めない。
 *
 * ビン数・strip 数・軸範囲・AsAd 枚数は**一切書かない**。全部メッセージから来る。
 */
import { Component, OnDestroy, computed, effect, inject, signal, untracked } from '@angular/core';
import { MatTabsModule } from '@angular/material/tabs';

import { DisplayClock } from '../display/display-clock';
import { DisplayControls } from '../display/display-controls';
import { updateWaveformCache, waveformKey } from '../waveform/waveform-select';
import type { UvwMessage, WaveformsMessage } from '../ws/wire';
import { WsClientService } from '../ws/ws-client';
import { JsrootPanel } from './jsroot-panel';
import {
  HISTO_ROWS,
  PLANE_NAMES,
  asadEventMismatch,
  asadPanelSpec,
  histoPanelSpec,
  uvwPanelSpec,
} from './root-histo';

@Component({
  selector: 'app-monitor-view',
  imports: [MatTabsModule, DisplayControls, JsrootPanel],
  templateUrl: './monitor-view.html',
  styleUrl: './monitor-view.scss',
})
export class MonitorView implements OnDestroy {
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

  /**
   * 077: (cobo,asad) 毎の最新(`ws.waveforms()` は最新 1 通しか持たないため —
   * `waveform-select.ts` の `updateWaveformCache` を波形ビューと同じ流儀で再利用)。
   */
  private readonly waveformCache = signal<ReadonlyMap<string, WaveformsMessage>>(new Map());
  /** イベント表示と同じく、ラベル(event# / ≠ UVW)は freeze の刻みでだけ進める。 */
  private readonly shownAsad = signal<ReadonlyMap<string, WaveformsMessage>>(new Map());

  constructor() {
    // 077 発注書 1: monitor タブを表示中だけ 0x03 を購読する(離脱で false — §10.3)。
    this.ws.setWaveforms(true);

    effect(() => {
      const message = this.ws.waveforms();
      if (!message) return;
      this.waveformCache.set(updateWaveformCache(untracked(this.waveformCache), message));
    });

    effect(() => {
      this.clock.tick();
      this.shownEvent.set(untracked(this.latestUvw));
      this.shownAsad.set(untracked(this.waveformCache));
    });
  }

  ngOnDestroy(): void {
    // 離脱したら購読を戻す(帯域制御 — §10.3)。
    this.ws.setWaveforms(false);
  }

  /** R9: イベント ID は常時表示。 */
  readonly eventLabel = computed(() => {
    const message = this.shownEvent();
    if (!message) return 'event —';
    return `run ${message.header.runNumber} · event #${message.header.eventNumber}`;
  });

  readonly incomplete = computed(() => this.shownEvent()?.header.incomplete ?? false);
  readonly hasEvent = computed(() => this.shownEvent() !== null);

  /**
   * 077: 現在わかっている (cobo,asad) の一覧(mini = 1 枚、ELITPC = 4 枚 — 枚数は
   * メッセージ由来、焼き込まない)。ラベルは freeze の刻みで進む `shownAsad` から作る。
   */
  readonly asadPanels = computed(() => {
    const uvwEventNumber = this.shownEvent()?.header.eventNumber ?? null;
    return [...this.shownAsad().values()]
      .sort((a, b) => a.cobo - b.cobo || a.asad - b.asad)
      .map((message) => ({
        key: waveformKey(message.cobo, message.asad),
        spec: asadPanelSpec(message.cobo, message.asad),
        eventLabel: `event #${message.header.eventNumber}`,
        mismatch: asadEventMismatch(message.header.eventNumber, uvwEventNumber),
      }));
  });

  histoMessage(id: number) {
    return this.histos().get(id) ?? null;
  }

  uvwMessage(index: number) {
    return this.uvw()[index] ?? null;
  }

  /** JsrootPanel には常に最新を渡す(再描画そのものは JsrootPanel 内の clock tick が司る)。 */
  asadMessage(key: string) {
    return this.waveformCache().get(key) ?? null;
  }
}
