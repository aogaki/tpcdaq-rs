/**
 * JSROOT の**遅延ロード**(SPEC §11 / 028 発注書 1)。
 *
 * - ラッパライブラリは入れない(KISS)。素の動的 `import()` 1 か所だけ。
 * - `jsroot`(main.mjs)は geom / three / io / tree / gui を全部引くので**使わない**。
 *   必要なサブパス(`jsroot/core` と `jsroot/draw`)だけを引く。
 * - 解決済み Promise をモジュールに持つので、パネルが何枚あっても取得は 1 回。
 */

import type { HistFactory } from './root-histo';

export interface JsrootApi {
  readonly createHistogram: HistFactory;
  readonly draw: (dom: HTMLElement, obj: unknown, opt?: string) => Promise<unknown>;
  readonly redraw: (dom: HTMLElement, obj: unknown, opt?: string) => Promise<unknown>;
  readonly cleanup: (dom?: HTMLElement) => void;
}

/**
 * JSROOT のダークモード。実体は canvas SVG への `filter: invert(100%)`
 * (jsroot 7.11 `TPadPainter.changeDarkMode`)なので、白背景のパッドが
 * ページのダーク面に馴染む代わりに **colz パレットの色も反転する**。
 * §11 確認項目②の所見は結果節に記録。
 */
const DARK_MODE = true;

/**
 * colz のパレット(029 必須処置 A)。
 *
 * 既定の **57 = kBird** は低域が暗い青なので、上の `invert(100%)` を通すと
 * **低い値が画面で一番明るいクリーム色**になる。オンラインモニタを「明るい = 多い」と
 * 読む目には逆に見え、実験中の誤判断に直結する(028 レビューの裁定)。
 *
 * **56 = kInvertedDarkBodyRadiator は 53 = kDarkBodyRadiator の色を逆順に並べたもの**
 * (jsroot の `base/colors.mjs` の rgb 表で確認 — 56 の 9 stop は 53 の 9 stop の逆順)。
 * つまり 56 は「低 = ほぼ白、高 = 黒」なので、反転後は **低 = ほぼ黒、高 = 白** の
 * 明度単調なランプになる。反転後に kBird そのものになる標準パレットは存在しない
 * (kBird を色ごとに反転した表は ROOT/JSROOT の 51–113 に無い)ので、
 * 「明度が単調で、低い値が暗い」という要件を満たす標準パレットを選んだ。
 *
 * 1D(`hist`)はパレットを使わないので**この変更は 1D の見た目に影響しない**
 * (028 で良好と判断された 1D の反転表示はそのまま)。
 */
const PALETTE = 56;

let pending: Promise<JsrootApi> | null = null;

export function loadJsroot(): Promise<JsrootApi> {
  pending ??= Promise.all([import('jsroot/core'), import('jsroot/draw')]).then(([core, draw]) => {
    core.settings.DarkMode = DARK_MODE;
    core.settings.Palette = PALETTE;
    return {
      createHistogram: core.createHistogram,
      draw: draw.draw,
      redraw: draw.redraw,
      cleanup: draw.cleanup,
    };
  });
  return pending;
}
