/**
 * `ResizeObserver` を Angular のコンポーネント寿命に合わせて張る共有ヘルパ。
 *
 * `jsroot-panel.ts` と `waveform-view.ts` にあった「field + viewChild ready で
 * lazy 生成 + ngOnDestroy で disconnect」という同一パターン(048 発注書 B)を集約する。
 * `effect()` の `onCleanup` は依存が変わった直後とコンポーネント破棄時の両方で呼ばれる
 * ので、呼び出し側は observer フィールドも `ngOnDestroy` の disconnect も書かなくてよい。
 * **injection context(コンストラクタ等)から呼ぶこと**(`effect()` の制約)。
 */
import { effect } from '@angular/core';

export function observeResize(host: () => HTMLElement | undefined, onResize: () => void): void {
  effect((onCleanup) => {
    const element = host();
    if (!element) return;
    const observer = new ResizeObserver(onResize);
    observer.observe(element);
    onCleanup(() => observer.disconnect());
  });
}
