/**
 * JSROOT のパネル 1 枚(SPEC §11)。
 *
 * 更新戦略は 028 発注書 6 の第一候補どおり **`redraw()` = `updateObject` + `redrawPad`**。
 * jsroot 7.11 の `THistPainter.updateObject` は `zoomChangedInteractive()` を見て
 * `checkPadRange()` を飛ばすので、ユーザーのズームは 1 Hz 更新を跨いでも保たれる。
 *
 * 再描画の刻みは `DisplayClock`(freeze / interval)。**データの購読は止めない**。
 */
import {
  Component,
  ElementRef,
  OnDestroy,
  computed,
  effect,
  inject,
  input,
  signal,
  untracked,
  viewChild,
} from '@angular/core';

import { DisplayClock } from '../display/display-clock';
import { observeResize } from '../display/resize-observer';
import { loadJsroot } from './jsroot-loader';
import { type PanelMessage, type PanelSpec, buildPanelObject, panelDrawOption } from './root-histo';

@Component({
  selector: 'app-jsroot-panel',
  template: `
    <section class="panel">
      <header>
        <span class="name">{{ spec().name }}</span>
        <span class="shape">{{ shape() }}</span>
        <span class="spacer"></span>
        @if (skipped() > 0) {
          <span
            class="skipped"
            title="Ticks that arrived before the previous draw finished (display-only decimation)"
          >
            skipped {{ skipped() }}
          </span>
        }
        <button type="button" class="log" [class.on]="log()" (click)="toggleLog()">log</button>
      </header>
      <div class="plot" #plot></div>
      @if (issue(); as text) {
        <p class="issue">{{ text }}</p>
      }
    </section>
  `,
  styles: `
    .panel {
      display: flex;
      flex-direction: column;
      min-width: 0;
      border: 1px solid var(--mat-sys-outline-variant);
      border-radius: 6px;
      background: var(--mat-sys-surface-container-low);
      overflow: hidden;
    }

    header {
      display: flex;
      align-items: center;
      gap: 0.5rem;
      padding: 0.25rem 0.5rem;
      border-bottom: 1px solid var(--mat-sys-outline-variant);
      font: var(--mat-sys-body-small);
    }

    .name {
      font-weight: 600;
    }

    .shape,
    .skipped {
      color: var(--mat-sys-on-surface-variant);
      font-size: 0.75rem;
    }

    .skipped {
      color: #ffb74d;
    }

    .spacer {
      flex: 1 1 auto;
    }

    .log {
      border: 1px solid var(--mat-sys-outline-variant);
      border-radius: 4px;
      background: transparent;
      color: var(--mat-sys-on-surface-variant);
      font: inherit;
      font-size: 0.7rem;
      padding: 0.05rem 0.4rem;
      cursor: pointer;
    }

    .log.on {
      background: var(--mat-sys-primary);
      color: var(--mat-sys-on-primary);
      border-color: var(--mat-sys-primary);
    }

    .plot {
      flex: 1 1 auto;
      min-height: 0;
      height: var(--panel-plot-height, 15rem);
    }

    .issue {
      margin: 0;
      padding: 0.25rem 0.5rem;
      color: #ef9a9a;
      font-size: 0.75rem;
    }
  `,
})
export class JsrootPanel implements OnDestroy {
  readonly spec = input.required<PanelSpec>();
  readonly message = input<PanelMessage | null>(null);

  readonly log = signal(false);
  /** 描けなかった理由(silent 禁止)。 */
  readonly issue = signal<string | null>('waiting for data');
  /** 前の描画中に来た刻みの数(表示だけの間引き。silent 禁止)。 */
  readonly skipped = signal(0);

  /** ビン数・面のサイズが**メッセージ由来**であることを画面でも見せる。 */
  readonly shape = computed(() => {
    const message = this.message();
    if (!message) return '';
    switch (message.kind) {
      case 'histo1d':
        return `${message.nbins} bins · x [${message.xmin}, ${message.xmax}]`;
      case 'histo2d':
        return `${message.nx} × ${message.ny}`;
      case 'uvw':
        return `${message.nStrips} strips × ${message.nBuckets} buckets`;
    }
  });

  private readonly plot = viewChild<ElementRef<HTMLElement>>('plot');
  private readonly clock = inject(DisplayClock);
  private busy = false;

  constructor() {
    effect(() => {
      // freeze 中は tick が進まない = 再描画しない(SPEC §5.4)。
      this.clock.tick();
      // log 切替は刻みを待たずに反映したいので追跡する。
      const log = this.log();
      void this.render(untracked(this.message), log);
    });

    // パネルの大きさが変わったら描き直す(JSROOT の SVG は固定サイズで作られる)。
    observeResize(
      () => this.plot()?.nativeElement,
      () => void this.render(untracked(this.message), untracked(this.log)),
    );
  }

  ngOnDestroy(): void {
    const host = this.plot()?.nativeElement;
    if (host) void loadJsroot().then((jsroot) => jsroot.cleanup(host));
  }

  toggleLog(): void {
    this.log.update((value) => !value);
  }

  private async render(message: PanelMessage | null, log: boolean): Promise<void> {
    const host = this.plot()?.nativeElement;
    if (!host) return;
    if (!message) {
      this.issue.set('waiting for data');
      return;
    }
    if (this.busy) {
      // 落としてよい(モニタ系)が、落としたことは見せる。
      this.skipped.update((value) => value + 1);
      return;
    }

    this.busy = true;
    try {
      const jsroot = await loadJsroot();
      const object = buildPanelObject(jsroot.createHistogram, message, this.spec());
      await jsroot.redraw(host, object, panelDrawOption(message, log));
      this.issue.set(null);
    } catch (error) {
      this.issue.set(`draw failed: ${String(error)}`);
    } finally {
      this.busy = false;
    }
  }
}
