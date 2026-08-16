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
  stripBaseline,
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

  it('2D は 2 枚とも 縦 = strip / 横 = time bucket(058 のユーザー裁定)', () => {
    const stripTime = HISTO_ROWS[0];
    expect(stripTime.xTitle).toBe('time bucket');
    expect(stripTime.yTitle).toBe('strip');
    expect(uvwPanelSpec(0).xTitle).toBe('time bucket');
    expect(uvwPanelSpec(0).yTitle).toBe('strip');
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

describe('0x11 Histo2d → TH2D(058: 縦 = strip / 横 = time bucket に転置)', () => {
  // 非対称: nx=3(strip), ny=2(bucket)。ワイヤは iy 外側 row-major(`idx = iy*nx + ix`)。
  // 値を 10*ix + iy にしておくと、行列の取り違えも転置忘れも必ず露見する。
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

  it('ROOT の x = time bucket(ワイヤの ny)、y = strip(ワイヤの nx)', () => {
    const histo = buildPanelObject(createHistogram, message, SPEC);
    expect(histo.fXaxis.fNbins).toBe(2); // bucket 数
    expect(histo.fYaxis.fNbins).toBe(3); // strip 数
    // 軸範囲もワイヤの x↔y を入れ替えて渡す(値はメッセージ由来のまま)。
    expect(histo.fXaxis.fXmin).toBe(0);
    expect(histo.fXaxis.fXmax).toBe(2);
    expect(histo.fYaxis.fXmin).toBe(1);
    expect(histo.fYaxis.fXmax).toBe(4);
  });

  it('bins[iy*nx+ix] が ROOT の fArray[(ix+1)*(ny+2)+(iy+1)] に入る(転置)', () => {
    const histo = buildPanelObject(createHistogram, message, SPEC);
    // 手計算: fNcells = (2+2)*(3+2) = 20(x が bucket 2 本、y が strip 3 本)
    expect(histo.fNcells).toBe(20);
    expect(histo.fArray.length).toBe(20);

    // 手計算の対応表(ix = strip、iy = bucket。ROOT のビン番号は 1 始まり、
    // 行の刻み = nbucket + 2 = 4):
    //   (ix=0,iy=0) val 0  → fArray[1*4+1] = fArray[5]
    //   (ix=0,iy=1) val 1  → fArray[1*4+2] = fArray[6]
    //   (ix=1,iy=0) val 10 → fArray[2*4+1] = fArray[9]
    //   (ix=1,iy=1) val 11 → fArray[2*4+2] = fArray[10]
    //   (ix=2,iy=0) val 20 → fArray[3*4+1] = fArray[13]
    //   (ix=2,iy=1) val 21 → fArray[3*4+2] = fArray[14]
    expect(histo.fArray[5]).toBe(0);
    expect(histo.fArray[6]).toBe(1);
    expect(histo.fArray[9]).toBe(10);
    expect(histo.fArray[10]).toBe(11);
    expect(histo.fArray[13]).toBe(20);
    expect(histo.fArray[14]).toBe(21);

    // 触っていないセル(under/overflow の行と列)は 0 のまま。
    for (const cell of [0, 1, 2, 3, 4, 7, 8, 11, 12, 15, 16, 17, 18, 19]) {
      expect(histo.fArray[cell]).toBe(0);
    }
  });

  it('StripTime にはベースライン減算をかけない(Σ ADC の堆積マップのまま)', () => {
    const histo = buildPanelObject(createHistogram, message, SPEC);
    // 手計算: 0+10+20+1+11+21 = 63(引き算が混ざれば合わない)
    expect(histo.fEntries).toBe(63);
  });
});

describe('0x02 Uvw → TH2D(058: 転置 + strip 毎のベースライン減算)', () => {
  // 非対称: nStrips=3, nBuckets=2。strip-major(`idx=(strip-1)*nBuckets+bucket`)。
  // ベースラインは cell が 25 本無いので**ある分だけ**(= 2 セルの平均)。
  //   strip1: (100+140)/2 = 120 → cell0 = -20 / cell1 = +20
  //   strip2: (200+210)/2 = 205 → cell0 =  -5 / cell1 =  +5
  //   strip3: (300+302)/2 = 301 → cell0 =  -1 / cell1 =  +1
  // 6 セルの値が全部違うので、行・列・転置のどれを取り違えても露見する。
  const message: UvwMessage = {
    kind: 'uvw',
    header: { msgType: 0x02, incomplete: true, runNumber: 7, eventNumber: 42 },
    plane: 1,
    nStrips: 3,
    nBuckets: 2,
    adc: new Uint16Array([100, 140, 200, 210, 300, 302]),
  };

  it('x = time bucket 0..nBuckets、y = strip 1..N(どちらもメッセージ由来)', () => {
    const histo = buildPanelObject(createHistogram, message, uvwPanelSpec(1));
    expect(histo.fXaxis.fNbins).toBe(2);
    expect(histo.fXaxis.fXmin).toBe(0);
    expect(histo.fXaxis.fXmax).toBe(2); // 手計算: nBuckets = 2
    expect(histo.fYaxis.fNbins).toBe(3);
    expect(histo.fYaxis.fXmin).toBe(1);
    expect(histo.fYaxis.fXmax).toBe(4); // 手計算: nStrips + 1 = 4
    expect(histo.fName).toBe('EventV'); // plane 1 = V
    expect(histo.fXaxis.fTitle).toBe('time bucket');
    expect(histo.fYaxis.fTitle).toBe('strip');
  });

  it('adc[(strip-1)*nBuckets+bucket] - baseline が fArray[strip*(nBuckets+2)+bucket+1] に入る', () => {
    const histo = buildPanelObject(createHistogram, message, uvwPanelSpec(1));
    // 手計算: fNcells = (2+2)*(3+2) = 20、行の刻み = nBuckets + 2 = 4
    expect(histo.fNcells).toBe(20);
    //   strip1,bucket0 = -20 → fArray[1*4+1] = fArray[5]
    //   strip1,bucket1 = +20 → fArray[1*4+2] = fArray[6]
    //   strip2,bucket0 =  -5 → fArray[2*4+1] = fArray[9]
    //   strip2,bucket1 =  +5 → fArray[2*4+2] = fArray[10]
    //   strip3,bucket0 =  -1 → fArray[3*4+1] = fArray[13]
    //   strip3,bucket1 =  +1 → fArray[3*4+2] = fArray[14]
    expect(histo.fArray[5]).toBe(-20);
    expect(histo.fArray[6]).toBe(20);
    expect(histo.fArray[9]).toBe(-5);
    expect(histo.fArray[10]).toBe(5);
    expect(histo.fArray[13]).toBe(-1);
    expect(histo.fArray[14]).toBe(1);
  });

  it('負値は 0 に切り上げない(Uint16 のワイヤ値でも符号が残る)', () => {
    const histo = buildPanelObject(createHistogram, message, uvwPanelSpec(1));
    const negatives = [...Array(histo.fNcells).keys()].filter((i) => histo.fArray[i] < 0);
    // 手計算: 3 strip × cell0 の 3 セルが負(-20 / -5 / -1)。
    expect(negatives).toEqual([5, 9, 13]);
  });
});

describe('Event Display のベースライン = 先頭 25 cell(0..24)の平均(058)', () => {
  /**
   * strip1 = 先頭 25 cell(0..23 が 4、24 が 104)→ 合計 200 → **baseline = 8**。
   * cell 25 以降は窓の外なので baseline に効かない(窓が 26 本なら 8 にならない)。
   * strip2 は全セル 0 = 読み出しの無いストリップ → baseline 0 で何も引かれない。
   */
  const nBuckets = 30;
  const adc = new Uint16Array(2 * nBuckets);
  for (let b = 0; b < 24; b++) adc[b] = 4;
  adc[24] = 104;
  adc[25] = 1000;
  adc[26] = 0;
  adc[27] = 3;
  adc[28] = 8;
  adc[29] = 12;

  const message: UvwMessage = {
    kind: 'uvw',
    header: { msgType: 0x02, incomplete: false, runNumber: 7, eventNumber: 1 },
    plane: 0,
    nStrips: 2,
    nBuckets,
    adc,
  };

  it('先頭 25 cell だけがベースラインに入る(手計算: 200 / 25 = 8)', () => {
    expect(stripBaseline(adc, 0, nBuckets)).toBe(8);
    // 窓は 25 で頭打ち —— グリッドが 26 cell でも cell 25(1000)は入らない
    // (入れば (200+1000)/26 = 46.15… になる = 窓の境界のテスト)。
    expect(stripBaseline(adc, 0, 26)).toBe(8);
    // 逆に 25 cell 無いグリッドではある分だけ(cell 0..23 の 4 が 24 本 → 4)。
    expect(stripBaseline(adc, 0, 24)).toBe(4);
  });

  it('各データ点から baseline を引く。負値も 0 も素通し', () => {
    const histo = buildPanelObject(createHistogram, message, uvwPanelSpec(0));
    const stride = nBuckets + 2; // 32
    const cell = (strip: number, bucket: number) => histo.fArray[strip * stride + bucket + 1];
    expect(cell(1, 0)).toBe(-4); // 4 - 8
    expect(cell(1, 23)).toBe(-4);
    expect(cell(1, 24)).toBe(96); // 104 - 8
    expect(cell(1, 25)).toBe(992); // 1000 - 8(窓の外なので baseline は動かない)
    expect(cell(1, 26)).toBe(-8); // 0 - 8
    expect(cell(1, 28)).toBe(0); // 8 - 8 = ちょうど 0
    expect(cell(1, 29)).toBe(4); // 12 - 8
  });

  it('読み出しの無いストリップは 0 のまま(未読が「引きすぎ」に見えない)', () => {
    const histo = buildPanelObject(createHistogram, message, uvwPanelSpec(0));
    expect(stripBaseline(adc, nBuckets, nBuckets)).toBe(0);
    const stride = nBuckets + 2;
    for (let b = 0; b < nBuckets; b++) {
      expect(histo.fArray[2 * stride + b + 1]).toBe(0);
    }
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
