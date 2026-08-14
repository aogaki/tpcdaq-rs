/**
 * SPEC §5.2 / §10.2 — WS のヒストメッセージ → JSROOT オブジェクトの組み立て。
 *
 * **本物の `createHistogram`(jsroot/core)を渡してテストする**。ROOT の
 * fArray レイアウト(under/overflow 込みの `(nx+2)*(ny+2)`)は JSROOT の描画側が
 * 前提にしているものなので、自前のスタブで代用すると「テストは通るが描けない」
 * が起きる。ここが「テストが見ているものと画面が見ているものが同じ」の担保。
 */
import { createHistogram } from 'jsroot/core';

import type { Histo1dMessage, Histo2dMessage, UvwMessage, WsHeader } from '../ws/wire';
import {
  HISTO_ROWS,
  PLANE_NAMES,
  buildPanelObject,
  histoPanelSpec,
  panelDrawOption,
  uvwPanelSpec,
} from './root-histo';

const HEADER: WsHeader = { msgType: 0x10, incomplete: false, runNumber: 7, eventNumber: 0 };

const SPEC = { name: 'test', title: 'title', xTitle: 'x', yTitle: 'y' };

describe('§5.2 の 9 枚テーブル(id と名前は仕様どおり・ch 数は焼き込まない)', () => {
  it('行 = StripTime / Charge / ChargeMax、列 = U/V/W で id 1–9 を覆う', () => {
    expect(HISTO_ROWS.map((row) => row.key)).toEqual(['StripTime', 'Charge', 'ChargeMax']);
    expect(HISTO_ROWS.flatMap((row) => [...row.ids])).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9]);
    expect(HISTO_ROWS.map((row) => row.is2d)).toEqual([true, false, false]);
    expect(PLANE_NAMES).toEqual(['U', 'V', 'W']);
  });

  it('名前は §5.2 の表と一致する(StripTimeU … ChargeMaxW)', () => {
    const names = HISTO_ROWS.flatMap((row) =>
      PLANE_NAMES.map((_plane, index) => histoPanelSpec(row, index).name),
    );
    expect(names).toEqual([
      'StripTimeU',
      'StripTimeV',
      'StripTimeW',
      'ChargeU',
      'ChargeV',
      'ChargeW',
      'ChargeMaxU',
      'ChargeMaxV',
      'ChargeMaxW',
    ]);
  });
});

describe('0x10 Histo1d → TH1D', () => {
  // 非対称データ: 4 ビン、値は全部違う。
  const message: Histo1dMessage = {
    kind: 'histo1d',
    header: HEADER,
    id: 4,
    nbins: 4,
    xmin: 0,
    xmax: 4096,
    bins: new Float32Array([0, 1.5, 2.25, 3.75]),
  };

  it('ビン数と軸範囲はメッセージ由来(0–4096 を焼き込まない)', () => {
    const histo = buildPanelObject(createHistogram, message, SPEC);
    expect(histo.fXaxis.fNbins).toBe(4);
    expect(histo.fXaxis.fXmin).toBe(0);
    expect(histo.fXaxis.fXmax).toBe(4096);

    // 軸範囲が違うメッセージを渡せばそのまま反映される = 定数ではない。
    const shifted = buildPanelObject(createHistogram, { ...message, xmin: -10, xmax: 20 }, SPEC);
    expect(shifted.fXaxis.fXmin).toBe(-10);
    expect(shifted.fXaxis.fXmax).toBe(20);
  });

  it('ROOT の fArray は under/overflow 込み: bins[i] → fArray[i+1]', () => {
    const histo = buildPanelObject(createHistogram, message, SPEC);
    // TH1 の fNcells = nbins + 2(手計算: 4 + 2 = 6)。
    expect(histo.fNcells).toBe(6);
    expect(histo.fArray.length).toBe(6);
    expect(histo.fArray[0]).toBe(0); // underflow は触らない
    expect(histo.fArray[1]).toBeCloseTo(0, 6);
    expect(histo.fArray[2]).toBeCloseTo(1.5, 6);
    expect(histo.fArray[3]).toBeCloseTo(2.25, 6);
    expect(histo.fArray[4]).toBeCloseTo(3.75, 6);
    expect(histo.fArray[5]).toBe(0); // overflow は触らない
    // 手計算: 0 + 1.5 + 2.25 + 3.75 = 7.5
    expect(histo.fEntries).toBeCloseTo(7.5, 6);
  });

  it('名前と軸タイトルは呼び手が決める', () => {
    const histo = buildPanelObject(createHistogram, message, {
      name: 'ChargeU',
      title: 'ChargeU (run 7)',
      xTitle: 'pulse height',
      yTitle: 'entries',
    });
    expect(histo.fName).toBe('ChargeU');
    expect(histo.fTitle).toBe('ChargeU (run 7)');
    expect(histo.fXaxis.fTitle).toBe('pulse height');
    expect(histo.fYaxis.fTitle).toBe('entries');
  });
});

describe('0x11 Histo2d → TH2D(iy 外側 row-major → ROOT の行列位置)', () => {
  // 非対称: nx=3, ny=2。ワイヤは iy 外側 row-major(`idx = iy*nx + ix`)なので
  // 値を 10*ix + iy にしておくと、行列位置の取り違えが必ず露見する。
  //   idx: 0    1    2    3    4    5
  //   iy :  0    0    0    1    1    1
  //   ix :  0    1    2    0    1    2
  //   val:  0   10   20    1   11   21
  const message: Histo2dMessage = {
    kind: 'histo2d',
    header: HEADER,
    id: 1,
    nx: 3,
    ny: 2,
    xmin: 1,
    xmax: 4,
    ymin: 0,
    ymax: 2,
    bins: new Float32Array([0, 10, 20, 1, 11, 21]),
  };

  it('nx / ny / 軸範囲はメッセージ由来', () => {
    const histo = buildPanelObject(createHistogram, message, SPEC);
    expect(histo.fXaxis.fNbins).toBe(3);
    expect(histo.fYaxis.fNbins).toBe(2);
    expect(histo.fXaxis.fXmin).toBe(1);
    expect(histo.fXaxis.fXmax).toBe(4);
    expect(histo.fYaxis.fXmin).toBe(0);
    expect(histo.fYaxis.fXmax).toBe(2);
  });

  it('bins[iy*nx+ix] が ROOT の fArray[(iy+1)*(nx+2)+(ix+1)] に入る', () => {
    const histo = buildPanelObject(createHistogram, message, SPEC);
    // 手計算: fNcells = (3+2)*(2+2) = 20
    expect(histo.fNcells).toBe(20);
    expect(histo.fArray.length).toBe(20);

    // 手計算の対応表(ix,iy は 0 始まり、ROOT のビン番号は 1 始まり):
    //   (ix=0,iy=0) val 0  → fArray[1*5+1] = fArray[6]
    //   (ix=1,iy=0) val 10 → fArray[1*5+2] = fArray[7]
    //   (ix=2,iy=0) val 20 → fArray[1*5+3] = fArray[8]
    //   (ix=0,iy=1) val 1  → fArray[2*5+1] = fArray[11]
    //   (ix=1,iy=1) val 11 → fArray[2*5+2] = fArray[12]
    //   (ix=2,iy=1) val 21 → fArray[2*5+3] = fArray[13]
    expect(histo.fArray[6]).toBe(0);
    expect(histo.fArray[7]).toBe(10);
    expect(histo.fArray[8]).toBe(20);
    expect(histo.fArray[11]).toBe(1);
    expect(histo.fArray[12]).toBe(11);
    expect(histo.fArray[13]).toBe(21);

    // 触っていないセル(under/overflow の行と列)は 0 のまま。
    for (const cell of [0, 1, 2, 3, 4, 5, 9, 10, 14, 15, 16, 17, 18, 19]) {
      expect(histo.fArray[cell]).toBe(0);
    }
  });
});

describe('0x02 Uvw → TH2D(strip-major。軸はメッセージの nStrips/nBuckets 由来)', () => {
  // 非対称: nStrips=3, nBuckets=2。strip-major(`idx=(strip-1)*nBuckets+bucket`)。
  //   idx : 0        1        2        3        4        5
  //   strip: 1        1        2        2        3        3
  //   bucket:0        1        0        1        0        1
  //   adc  :100      101      200      201      300      301
  const message: UvwMessage = {
    kind: 'uvw',
    header: { msgType: 0x02, incomplete: true, runNumber: 7, eventNumber: 42 },
    plane: 1,
    nStrips: 3,
    nBuckets: 2,
    adc: new Uint16Array([100, 101, 200, 201, 300, 301]),
  };

  it('x = strip 1..N、y = bucket 0..nBuckets(どちらもメッセージ由来)', () => {
    const histo = buildPanelObject(createHistogram, message, uvwPanelSpec(1));
    expect(histo.fXaxis.fNbins).toBe(3);
    expect(histo.fXaxis.fXmin).toBe(1);
    expect(histo.fXaxis.fXmax).toBe(4); // 手計算: nStrips + 1 = 4
    expect(histo.fYaxis.fNbins).toBe(2);
    expect(histo.fYaxis.fXmin).toBe(0);
    expect(histo.fYaxis.fXmax).toBe(2); // 手計算: nBuckets = 2
    expect(histo.fName).toBe('EventV'); // plane 1 = V
  });

  it('adc[(strip-1)*nBuckets+bucket] が fArray[(bucket+1)*(nStrips+2)+strip] に入る', () => {
    const histo = buildPanelObject(createHistogram, message, uvwPanelSpec(1));
    // 手計算: fNcells = (3+2)*(2+2) = 20
    expect(histo.fNcells).toBe(20);
    //   strip1,bucket0 = 100 → fArray[1*5+1] = fArray[6]
    //   strip2,bucket0 = 200 → fArray[1*5+2] = fArray[7]
    //   strip3,bucket0 = 300 → fArray[1*5+3] = fArray[8]
    //   strip1,bucket1 = 101 → fArray[2*5+1] = fArray[11]
    //   strip2,bucket1 = 201 → fArray[2*5+2] = fArray[12]
    //   strip3,bucket1 = 301 → fArray[2*5+3] = fArray[13]
    expect(histo.fArray[6]).toBe(100);
    expect(histo.fArray[7]).toBe(200);
    expect(histo.fArray[8]).toBe(300);
    expect(histo.fArray[11]).toBe(101);
    expect(histo.fArray[12]).toBe(201);
    expect(histo.fArray[13]).toBe(301);
  });
});

describe('描画オプション(log 切替)', () => {
  const histo1d: Histo1dMessage = {
    kind: 'histo1d',
    header: HEADER,
    id: 4,
    nbins: 2,
    xmin: 0,
    xmax: 4096,
    bins: new Float32Array([1, 2]),
  };
  const histo2d: Histo2dMessage = {
    kind: 'histo2d',
    header: HEADER,
    id: 1,
    nx: 1,
    ny: 1,
    xmin: 1,
    xmax: 2,
    ymin: 0,
    ymax: 1,
    bins: new Float32Array([1]),
  };

  const uvw: UvwMessage = {
    kind: 'uvw',
    header: { ...HEADER, msgType: 0x02 },
    plane: 0,
    nStrips: 1,
    nBuckets: 1,
    adc: new Uint16Array([1]),
  };

  it('1D は y、2D は z を log にする。2D は常に colz', () => {
    expect(panelDrawOption(histo1d, false)).toBe('hist');
    expect(panelDrawOption(histo1d, true)).toBe('hist;logy');
    expect(panelDrawOption(histo2d, false)).toBe('colz;nostat');
    expect(panelDrawOption(histo2d, true)).toBe('colz;logz;nostat');
  });

  /**
   * 029 必須処置 B(ユーザー裁定 2026-08-14、変更不可): **2D は stats box を出さない**。
   * 2D(StripTime / イベント表示)の目的は各ストリップの時間変化を一枚絵にすることで、
   * 統計量としては意味を持たない。1D(Charge / ChargeMax)は波高分布なので残す。
   */
  it('2D(StripTime とイベント表示)は stats box を出さない、1D は出す', () => {
    expect(panelDrawOption(histo2d, false)).toContain('nostat');
    expect(panelDrawOption(histo2d, true)).toContain('nostat');
    expect(panelDrawOption(uvw, false)).toBe('colz;nostat');
    expect(panelDrawOption(uvw, true)).toBe('colz;logz;nostat');
    expect(panelDrawOption(histo1d, false)).not.toContain('nostat');
    expect(panelDrawOption(histo1d, true)).not.toContain('nostat');
  });
});
