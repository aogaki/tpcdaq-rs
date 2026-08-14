/**
 * SPEC §5.4 — 表示間隔と freeze は**クライアント側だけ**。
 *
 * freeze は **表示だけ**止める。WS の購読も DAQ も積算も保存も一切止めない
 * (status バーは動き続ける)。run Stop と混同させないのは呼び手(`DisplayControls`)の仕事。
 *
 * 描画側は `tick()` を読む effect を書けばよい: freeze 中は `tick` が進まないので
 * 再描画されない。飛ばした表示更新の回数は `suppressed()` で数えて画面に出す
 * (落としてよいが silent にしない — CLAUDE.md 絶対ルール)。
 */
import { Injectable, effect, signal } from '@angular/core';

/** 表示更新間隔の選択肢(028 発注書 5)。 */
export const DISPLAY_INTERVALS_MS = [500, 1000, 2000, 5000] as const;

@Injectable({ providedIn: 'root' })
export class DisplayClock {
  readonly frozen = signal(false);
  readonly intervalMs = signal<number>(1000);

  /** 表示更新の刻み。描画 effect はこれを読む。 */
  readonly tick = signal(0);
  /** freeze 中に飛ばした表示更新の回数(unfreeze でリセット)。 */
  readonly suppressed = signal(0);

  constructor() {
    effect((onCleanup) => {
      const period = this.intervalMs();
      const timer = setInterval(() => {
        // frozen はここ(タイマのコールバック = 追跡外)で読む。
        // effect の本体で読むとタイマが張り直されてしまう。
        if (this.frozen()) this.suppressed.update((value) => value + 1);
        else this.tick.update((value) => value + 1);
      }, period);
      onCleanup(() => clearInterval(timer));
    });
  }

  toggleFreeze(): void {
    const next = !this.frozen();
    this.frozen.set(next);
    if (!next) {
      this.suppressed.set(0);
      // 解除した瞬間に最新へ追いつく(次の刻みを待たせない)。
      this.tick.update((value) => value + 1);
    }
  }

  setIntervalMs(ms: number): void {
    this.intervalMs.set(ms);
  }
}
