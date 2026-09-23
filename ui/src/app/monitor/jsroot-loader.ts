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
 * colz のパレット(029 必須処置 A → 086 で「反転前提のカスタムパレット」に変更 →
 * 086 追補2 で配色を自前のダーク用ランプに差し替え)。
 *
 * 標準パレット番号をそのまま `settings.Palette` に渡すと、DARK_MODE の
 * `filter: invert(100%)` を通った**あと**の配色になる。旧設定の
 * **56 = kInvertedDarkBodyRadiator** は反転前が低 = ほぼ白・高 = 黒なので、反転後は
 * 低 = ほぼ黒・高 = 白になり、パルサー run で ADC 4095 に飽和した ch と、ただ波高が
 * 高いだけの通常 ch の頂上が両方とも白〜淡色に潰れて区別できなかった
 * (077 目視結果 第2弾で確認)。
 *
 * 既製パレットを反転前提で登録する方式(086 第一弾)も、ゼロ付近が一面の鮮やかな色に
 * なりダークテーマで眩しいと判明した(086 追補2 目視、`reference/_spike/demo/cdp_shot.py`)。
 * **JSROOT の colz は最下位レベルに約 5 % 位置の色を使う**ので先頭 15 % を黒〜暗紫にしてある。
 *
 * そこで既製パレットへの依存をやめ、`TPCDAQ_DARK_STOPS`(9 stop、位置 0–1)を
 * `rampPalette` で 255 色に線形補間し、`invertColors`(255-x)で反転してから
 * `setColorPalette(CUSTOM_PALETTE_ID, …)` で標準番号と衝突しない id に登録する。
 * SVG の `invert(100%)` を通った**結果**がランプそのものになる: 低 = 黒(暗い)→ 暗紫 →
 * 藍 → 青 → シアン → 緑 → 黄緑 → 黄 → 高 = 橙(飽和が見える、白にも一面の鮮やかな色にも
 * 潰れない)。
 *
 * 1D(`hist`)はパレットを使わないので**この変更は 1D の見た目に影響しない**
 * (028 で良好と判断された 1D の反転表示はそのまま)。
 */
type RgbStop = readonly [number, readonly [number, number, number]];

/** ダーク用ランプの 9 stop(位置 0–1、`rampPalette` に渡す前の生の配色)。086 追補2。 */
export const TPCDAQ_DARK_STOPS: ReadonlyArray<RgbStop> = [
  [0.0, [0, 0, 0]], // 黒
  [0.125, [30, 10, 60]], // 暗紫
  [0.25, [60, 20, 140]], // 藍
  [0.375, [30, 80, 200]], // 青
  [0.5, [0, 160, 200]], // シアン
  [0.625, [40, 200, 120]], // 緑
  [0.75, [180, 220, 60]], // 黄緑
  [0.875, [255, 210, 0]], // 黄
  [1.0, [255, 120, 0]], // 橙
];

/**
 * `stops`(位置 0–1 の昇順、先頭が 0・末尾が 1)を線形補間して `n` 色の
 * `rgb(r,g,b)` 配列を作る(086 追補2)。位置は `i / (n - 1)` で等間隔に振る。
 */
export function rampPalette(stops: ReadonlyArray<RgbStop>, n = 255): string[] {
  const colors: string[] = [];
  for (let i = 0; i < n; i++) {
    const t = n === 1 ? 0 : i / (n - 1);
    colors.push(rampColorAt(stops, t));
  }
  return colors;
}

function rampColorAt(stops: ReadonlyArray<RgbStop>, t: number): string {
  let lo = stops[0];
  let hi = stops[stops.length - 1];
  for (let i = 0; i < stops.length - 1; i++) {
    if (t >= stops[i][0] && t <= stops[i + 1][0]) {
      lo = stops[i];
      hi = stops[i + 1];
      break;
    }
  }
  const span = hi[0] - lo[0];
  const frac = span === 0 ? 0 : (t - lo[0]) / span;
  const [r0, g0, b0] = lo[1];
  const [r1, g1, b1] = hi[1];
  const r = Math.round(r0 + (r1 - r0) * frac);
  const g = Math.round(g0 + (g1 - g0) * frac);
  const b = Math.round(b0 + (b1 - b0) * frac);
  return `rgb(${r},${g},${b})`;
}

/** 反転済みダークランプを登録する先の id。標準パレットの範囲(51–113)と衝突しない値。 */
const CUSTOM_PALETTE_ID = 1057;

const RGB_RE = /^rgb\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*\)$/i;
const HEX_RE = /^#([0-9a-fA-F]{6})$/;

/**
 * `rgb(r,g,b)` / `#rrggbb` の色文字列を RGB 反転する(255-x、DARK_MODE の
 * `filter: invert(100%)` を打ち消すため)。086。
 *
 * `rampPalette()` は常に `rgb(r,g,b)` を返すが、想定外の形式(named color 等)が
 * 来ても**例外にせず、そのまま通す** — CLAUDE.md の「silent failure を作らない」に
 * 従い `console.warn` で可視化した上で通す。
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
    const ramp = rampPalette(TPCDAQ_DARK_STOPS);
    colors.setColorPalette(CUSTOM_PALETTE_ID, invertColors(ramp));
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
