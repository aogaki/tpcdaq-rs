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
 * colz のパレット(029 必須処置 A → 086 で「反転前提のカスタムパレット」に変更)。
 *
 * 標準パレット番号をそのまま `settings.Palette` に渡すと、DARK_MODE の
 * `filter: invert(100%)` を通った**あと**の配色になる。旧設定の
 * **56 = kInvertedDarkBodyRadiator** は反転前が低 = ほぼ白・高 = 黒なので、反転後は
 * 低 = ほぼ黒・高 = 白になり、パルサー run で ADC 4095 に飽和した ch と、ただ波高が
 * 高いだけの通常 ch の頂上が両方とも白〜淡色に潰れて区別できなかった
 * (077 目視結果 第2弾で確認)。
 *
 * 57(kBird)を直接指定する案も試したが、kBird は反転前が低 = 濃紺・高 = 黄なので、
 * 反転後は**低が淡黄緑(明るい)・最上位が濃い青**になり、028 が避けた「低い値が画面で
 * 一番明るく見える」状態に逆戻りする(086 レビューで確認)。**標準パレット番号 51–113 を
 * 全数確認したが、反転後に「低域が暗く、最上位が白でない飽和色」になる既製パレットは無い**
 * (kBird の反転をさらに反転して元の kBird 配色に戻すような標準パレットは存在しないため)。
 *
 * そこで**反転を前提にしたカスタムパレットを自分で登録する**: `getColorPalette(57)` で
 * kBird の生の配色(255 色)を取り、`invertColors`(255-x)で反転してから
 * `setColorPalette(CUSTOM_PALETTE_ID, …)` で標準番号と衝突しない id に登録する。
 * SVG の `invert(100%)` を通った**結果**が本物の kBird になる: 低 = 濃紺(暗い)→ 緑 →
 * 高 = 黄(飽和が見える、白には潰れない)。028 の要件(低い値は画面で暗く見える)と
 * 086 の要件(飽和 ch が識別できる)を両立する。
 *
 * 1D(`hist`)はパレットを使わないので**この変更は 1D の見た目に影響しない**
 * (028 で良好と判断された 1D の反転表示はそのまま)。
 */
const KBIRD_PALETTE_ID = 57;

/** 反転済み kBird を登録する先の id。標準パレットの範囲(51–113)と衝突しない値。 */
const CUSTOM_PALETTE_ID = 1057;

const RGB_RE = /^rgb\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*\)$/i;
const HEX_RE = /^#([0-9a-fA-F]{6})$/;

/**
 * `rgb(r,g,b)` / `#rrggbb` の色文字列を RGB 反転する(255-x、DARK_MODE の
 * `filter: invert(100%)` を打ち消すため)。086。
 *
 * `getColorPalette()` が返す配色は上記 2 形式のどちらか(`base/colors.mjs` 参照)だが、
 * 想定外の形式(named color 等)が来ても**例外にせず、そのまま通す** — CLAUDE.md の
 * 「silent failure を作らない」に従い `console.warn` で可視化した上で通す。
 */
export function invertColors(colors: string[]): string[] {
  return colors.map((c) => {
    const rgb = RGB_RE.exec(c);
    if (rgb) {
      const [, r, g, b] = rgb;
      return `rgb(${255 - Number(r)},${255 - Number(g)},${255 - Number(b)})`;
    }
    const hex = HEX_RE.exec(c);
    if (hex) {
      const inv = 0xffffff ^ Number.parseInt(hex[1], 16);
      return `#${inv.toString(16).padStart(6, '0')}`;
    }
    console.warn(`jsroot-loader: invertColors: 未知の色形式をそのまま通す: "${c}"`);
    return c;
  });
}

let pending: Promise<JsrootApi> | null = null;

export function loadJsroot(): Promise<JsrootApi> {
  pending ??= Promise.all([
    import('jsroot/core'),
    import('jsroot/draw'),
    import('jsroot/colors'),
  ]).then(([core, draw, colors]) => {
    core.settings.DarkMode = DARK_MODE;
    const kbird = colors.getColorPalette(KBIRD_PALETTE_ID);
    colors.setColorPalette(CUSTOM_PALETTE_ID, invertColors(kbird.palette));
    core.settings.Palette = CUSTOM_PALETTE_ID;
    return {
      createHistogram: core.createHistogram,
      draw: draw.draw,
      redraw: draw.redraw,
      cleanup: draw.cleanup,
    };
  });
  return pending;
}
