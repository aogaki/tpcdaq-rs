/**
 * 086 レビュー — 「反転前提のカスタムパレット」の核になる純関数 `invertColors` の単体テスト。
 * 086 追補2 — 自前のダーク用ランプを作る純関数 `rampPalette` の単体テスト。
 *
 * DARK_MODE(`filter: invert(100%)`)を打ち消すため、ダーク用ランプの生の配色
 * (`rgb(r,g,b)` / `#rrggbb`)を 255-x で反転してからカスタムパレットとして登録する
 * (jsroot-loader.ts 参照)。ここでは jsroot を読み込まず、純関数だけを検証する(KISS)。
 */
import { vi } from 'vitest';

import { invertColors, rampPalette, TPCDAQ_DARK_STOPS } from './jsroot-loader';

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

describe('086 追補2 rampPalette — 9 stop の線形補間ランプ', () => {
  it('n=255 で 255 色を返す(位置は i / 254 で等間隔)', () => {
    expect(rampPalette(TPCDAQ_DARK_STOPS)).toHaveLength(255);
  });

  it('先頭は stop0 の端点そのもの(位置 0.000 = 黒 rgb(0,0,0))', () => {
    expect(rampPalette(TPCDAQ_DARK_STOPS)[0]).toBe('rgb(0,0,0)');
  });

  it('末尾は stop8 の端点そのもの(位置 1.000 = 橙 rgb(255,120,0))', () => {
    const colors = rampPalette(TPCDAQ_DARK_STOPS);
    expect(colors[colors.length - 1]).toBe('rgb(255,120,0)');
  });

  it('位置 0.0625(黒と暗紫の中間)を手計算で照合する(出典: n=17 のとき index 1 の位置は 1/16=0.0625。stop0 黒 rgb(0,0,0)@0.000 と stop1 暗紫 rgb(30,10,60)@0.125 のちょうど中間なので frac=0.5 → r=(0+30)/2=15、g=(0+10)/2=5、b=(0+60)/2=30)', () => {
    expect(rampPalette(TPCDAQ_DARK_STOPS, 17)[1]).toBe('rgb(15,5,30)');
  });
});
