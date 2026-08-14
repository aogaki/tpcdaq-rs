/**
 * jsroot 7.11 の型定義(028)。
 *
 * 配布物の `types.d.ts` は `declare module 'jsroot'` 等の**空宣言だけ**(全部 any)で、
 * しかもサブパス(`jsroot/core` / `jsroot/draw`)には `exports` の `types` 条件が無いので
 * TS からは見えない。ここで **028 が実際に使う分だけ** 型を付ける
 * (`import 'jsroot'`(main.mjs)は geom/three/io/tree/gui を全部引くので使わない —
 * サブパスだけを動的 import して遅延チャンクを小さく保つ)。
 */

declare module 'jsroot/core' {
  /** ROOT の TH1D/TH2D 相当を作る(under/overflow 込みの fArray も確保される)。 */
  export function createHistogram(
    typename: string,
    nbinsx: number,
    nbinsy?: number,
    nbinsz?: number,
  ): import('./app/monitor/root-histo').RootHisto;

  /** JSROOT の大域設定。028 は DarkMode、029 は colz のパレット番号(必須処置 A)。 */
  export const settings: { DarkMode: boolean; Palette: number };

  /** ROOT の gStyle 相当。028 で触るのは統計ボックスの既定だけ。 */
  export const gStyle: { fOptStat: number };
}

declare module 'jsroot/draw' {
  /** 初回描画。返り値は painter(以後の更新に使う)。 */
  export function draw(dom: HTMLElement | string, obj: unknown, opt?: string): Promise<unknown>;

  /**
   * 既存の描画を **updateObject + redrawPad** で更新する(未描画なら draw に落ちる)。
   * JSROOT 側で `zoomChangedInteractive()` を見ているので、ユーザーのズームは保たれる。
   */
  export function redraw(dom: HTMLElement | string, obj: unknown, opt?: string): Promise<unknown>;

  /** 描画を消す(コンポーネント破棄時)。 */
  export function cleanup(dom?: HTMLElement | string): void;
}
