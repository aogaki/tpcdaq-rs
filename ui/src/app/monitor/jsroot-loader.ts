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
 * colz のパレット(029 必須処置 A → 086 で 56 から 57 へ変更)。
 *
 * **56 = kInvertedDarkBodyRadiator**(旧設定)は `base/colors.mjs` の rgb 表で
 * stop0 ≈ rgb(242,243,230)(低 = ほぼ白)・stop8 = rgb(0,0,0)(高 = 黒)。
 * 上の `invert(100%)` を通すと低 = ほぼ黒、高 = 白の明度単調なランプになり、
 * 028 の要件(低い値は画面で暗く見える)は満たしていた。しかし**最上位が白**なので、
 * パルサー run で ADC 4095 に飽和した ch(白線)と、ただ波高が高いだけの通常 ch の頂上が
 * どちらも白〜淡色に潰れて区別できない(077 目視結果 第2弾で確認、086 発注)。
 *
 * **57 = kBird**(ROOT の既定パレット)に変更。rgb 表(9 stop)は
 * stop0 ≈ rgb(53,42,135)(濃紺)→ stop4 ≈ rgb(46,183,164)(緑)→
 * stop8 ≈ rgb(249,251,14)(黄)。`invert(100%)` を通すと stop0 は rgb(202,213,120)
 * (淡黄緑)、**stop8(最上位 = 飽和)は rgb(6,4,241) の濃い青**になり、白には潰れない
 * (上記 rgb 表から `255 - x` で手計算)。これで 086 の要件(飽和 ch が識別できる)を満たす。
 *
 * トレードオフ: 反転後の低域が淡黄緑(rgb(202,213,120)、明度が高い)になるため、
 * 028 が 57 を避けた理由(低い値が画面で一番明るく見える)が一部再発する。086 はこの
 * トレードオフを許容し 57 を第一候補に採った(飽和の可視化 > 低域の明度単調性)。
 * **この判断は rgb 表からの数値計算のみに基づく** — demo スタックがユーザーの
 * ポートと衝突するため今回の変更では目視確認を行っていない(086 結果節を参照)。
 * 実機画面での見え方が悪ければ 086 発注書の代替案どおり 55(rainbow 系)に差し替える。
 *
 * 1D(`hist`)はパレットを使わないので**この変更は 1D の見た目に影響しない**
 * (028 で良好と判断された 1D の反転表示はそのまま)。
 */
const PALETTE = 57;

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
