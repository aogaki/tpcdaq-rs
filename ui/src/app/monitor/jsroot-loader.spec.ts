/**
 * 086 レビュー — 「反転前提のカスタムパレット」の核になる純関数 `invertColors` の単体テスト。
 *
 * DARK_MODE(`filter: invert(100%)`)を打ち消すため、kBird の生の配色(`rgb(r,g,b)` / `#rrggbb`)
 * を 255-x で反転してからカスタムパレットとして登録する(jsroot-loader.ts 参照)。
 * ここでは jsroot を読み込まず、文字列変換だけを検証する(KISS — 実際の反転後パレットが
 * 本物の kBird に一致することは 086 の結果節に手計算の出典を記録)。
 */
import { vi } from 'vitest';

import { invertColors } from './jsroot-loader';

describe('086 invertColors — DARK_MODE の invert(100%) を打ち消す反転', () => {
  it('rgb(r,g,b) は各成分を 255-x する(出典: colors.mjs case 57 stop0 ≈ rgb(53,42,135)、255-53=202/255-42=213/255-135=120)', () => {
    expect(invertColors(['rgb(53,42,135)'])).toEqual(['rgb(202,213,120)']);
  });

  it('#rrggbb は 255-x をバイトごとに計算し小文字 hex を保つ(出典: stop8 ≈ rgb(249,251,14) = #f9fb0e、255-249=6/255-251=4/255-14=241 → #0604f1)', () => {
    expect(invertColors(['#f9fb0e'])).toEqual(['#0604f1']);
  });

  it('未知の色形式は例外にせずそのまま通し、console.warn で可視化する(silent failure 禁止)', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      expect(invertColors(['papayawhip'])).toEqual(['papayawhip']);
      expect(warn).toHaveBeenCalledTimes(1);
    } finally {
      warn.mockRestore();
    }
  });
});
