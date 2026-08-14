/**
 * 表示制御バー(SPEC §5.4 / 028 発注書 5)。
 *
 * **FREEZE は run Stop ではない**。DAQ は動き続けている。
 * 見分けがつくように ①文言に "display only" を必ず入れる ②色は Stop(赤)ではなく
 * シアン(情報色)③freeze 中は「DAQ は動き続けています」を常時出す
 * ④飛ばした表示更新の回数を出す(silent 禁止)。
 */
import { Component, inject } from '@angular/core';

import { DISPLAY_INTERVALS_MS, DisplayClock } from './display-clock';

@Component({
  selector: 'app-display-controls',
  template: `
    <div class="controls">
      <button
        type="button"
        class="freeze"
        [class.on]="clock.frozen()"
        (click)="clock.toggleFreeze()"
      >
        {{ clock.frozen() ? '❄ FREEZE (display only)' : 'Freeze display' }}
      </button>

      <label class="interval">
        update
        <!-- [value] を select 側に付けると option より先に評価されて先頭が選ばれてしまう
             (= 画面と実際の値がずれる)。option 側の [selected] で選ぶ。 -->
        <select (change)="onInterval($event)" aria-label="display update interval">
          @for (ms of intervals; track ms) {
            <option [value]="ms" [selected]="ms === clock.intervalMs()">{{ ms / 1000 }} s</option>
          }
        </select>
      </label>

      @if (clock.frozen()) {
        <span class="note">
          表示だけ止めています — <strong>DAQ / 保存 / 積算は動き続けています</strong>(Run Stop
          ではありません)。飛ばした表示更新 {{ clock.suppressed() }} 回。
        </span>
      }

      <ng-content />
    </div>
  `,
  styles: `
    .controls {
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      gap: 0.75rem;
      padding: 0.5rem 0.75rem;
      border-bottom: 1px solid var(--mat-sys-outline-variant);
      background: var(--mat-sys-surface-container-low);
      font: var(--mat-sys-body-small);
    }

    .freeze {
      /* Stop(破壊的操作)は赤。freeze は表示だけなので情報色のシアンで別物にする。 */
      border: 1px solid #26c6da;
      border-radius: 999px;
      background: transparent;
      color: #26c6da;
      padding: 0.3rem 0.9rem;
      font: inherit;
      font-weight: 600;
      cursor: pointer;
    }

    .freeze.on {
      background: #26c6da;
      color: #06232b;
    }

    .note {
      color: #26c6da;
    }

    .interval {
      display: inline-flex;
      align-items: center;
      gap: 0.35rem;
      color: var(--mat-sys-on-surface-variant);
    }

    select {
      background: var(--mat-sys-surface-container-high);
      color: var(--mat-sys-on-surface);
      border: 1px solid var(--mat-sys-outline-variant);
      border-radius: 4px;
      padding: 0.2rem 0.35rem;
      font: inherit;
    }
  `,
})
export class DisplayControls {
  readonly clock = inject(DisplayClock);
  readonly intervals = DISPLAY_INTERVALS_MS;

  onInterval(event: Event): void {
    const value = Number((event.target as HTMLSelectElement).value);
    if (Number.isFinite(value) && value > 0) this.clock.setIntervalMs(value);
  }
}
