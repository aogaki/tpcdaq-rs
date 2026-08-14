/**
 * SPEC §5.2 / §10.2 — WS のヒストメッセージ → JSROOT の TH1D/TH2D オブジェクト。
 *
 * **jsroot を import しない純ロジック**。`createHistogram` は引数で受け取るので
 * ①遅延ロードした jsroot をそのまま渡せる ②テストも本物の `createHistogram` を渡せる。
 *
 * ## ROOT の fArray レイアウト(ここが唯一の落とし穴)
 *
 * ROOT のヒストは under/overflow ビンを配列に含む:
 * - TH1: `fNcells = nbins + 2`、ビン i(0 始まり)は `fArray[i+1]`
 * - TH2: `fNcells = (nx+2)*(ny+2)`、ビン (ix,iy)(0 始まり)は `fArray[(iy+1)*(nx+2)+(ix+1)]`
 *
 * ワイヤ側(§10.2)は 2D が **iy 外側 row-major**(`idx = iy*nx + ix`)なので、
 * 行と列を取り違えると「転置した絵」が出る。`root-histo.spec.ts` が全ビン検査する。
 *
 * ch 数・ビン数・面の本数は**すべてメッセージ由来**(プロジェクト不変条件)。
 */

import type { Histo1dMessage, Histo2dMessage, UvwMessage } from '../ws/wire';

/** JSROOT のヒストオブジェクトのうち、ここで触る分だけ。 */
export interface RootAxis {
  fNbins: number;
  fXmin: number;
  fXmax: number;
  fTitle: string;
}

/** 添字で書ける配列(実体は Float64Array)。 */
export interface RootBinArray {
  [index: number]: number;
  readonly length: number;
}

export interface RootHisto {
  fName: string;
  fTitle: string;
  fXaxis: RootAxis;
  fYaxis: RootAxis;
  fZaxis: RootAxis;
  fNcells: number;
  fArray: RootBinArray;
  fEntries: number;
}

/** `jsroot/core` の `createHistogram` と同じ形(テストからは本物を渡す)。 */
export type HistFactory = (typename: string, nbinsx: number, nbinsy?: number) => RootHisto;

/** パネル 1 枚の見出し(名前・軸タイトルは呼び手が決める)。 */
export interface PanelSpec {
  readonly name: string;
  readonly title: string;
  readonly xTitle: string;
  readonly yTitle: string;
}

/** 1 枚のパネルに描けるメッセージ(9 ヒスト + イベント表示の Uvw)。 */
export type PanelMessage = Histo1dMessage | Histo2dMessage | UvwMessage;

/** SPEC §10.2: plane 0=U, 1=V, 2=W。 */
export const PLANE_NAMES = ['U', 'V', 'W'] as const;
export type PlaneName = (typeof PLANE_NAMES)[number];

/** モニタビューの 3×3 グリッドの 1 行(= §5.2 の 1 種類 × U/V/W)。 */
export interface HistoRow {
  /** §5.2 の名前の頭(`StripTime` → `StripTimeU`)。 */
  readonly key: string;
  /** 画面の行見出し。 */
  readonly label: string;
  /** U / V / W の順の id(§5.2 の表)。 */
  readonly ids: readonly [number, number, number];
  readonly is2d: boolean;
  readonly xTitle: string;
  readonly yTitle: string;
}

/**
 * SPEC §5.2 の 9 枚。行 = 種類、列 = 面。
 * ビン数・軸範囲は**ここに書かない**(メッセージから来る)。
 */
export const HISTO_ROWS: readonly HistoRow[] = [
  {
    key: 'StripTime',
    label: 'StripTime — Σ ADC (strip × time bucket)',
    ids: [1, 2, 3],
    is2d: true,
    xTitle: 'strip',
    yTitle: 'time bucket',
  },
  {
    key: 'Charge',
    label: 'Charge — 全ストリップの波高',
    ids: [4, 5, 6],
    is2d: false,
    xTitle: 'pulse height (raw ADC)',
    yTitle: 'entries',
  },
  {
    key: 'ChargeMax',
    label: 'ChargeMax — イベント最大波高',
    ids: [7, 8, 9],
    is2d: false,
    xTitle: 'pulse height (raw ADC)',
    yTitle: 'entries',
  },
];

/** 行 × 列 → §5.2 の名前(`StripTimeU` 等)。 */
export function histoPanelSpec(row: HistoRow, planeIndex: number): PanelSpec {
  const plane = PLANE_NAMES[planeIndex] ?? '?';
  const name = `${row.key}${plane}`;
  return { name, title: name, xTitle: row.xTitle, yTitle: row.yTitle };
}

/** イベント表示(0x02 Uvw)の 1 面。 */
export function uvwPanelSpec(planeIndex: number): PanelSpec {
  const plane = PLANE_NAMES[planeIndex] ?? '?';
  return {
    name: `Event${plane}`,
    title: `Event${plane}`,
    xTitle: 'strip',
    yTitle: 'time bucket',
  };
}

/** メッセージの種類で分けて JSROOT オブジェクトを組み立てる。 */
export function buildPanelObject(
  create: HistFactory,
  message: PanelMessage,
  spec: PanelSpec,
): RootHisto {
  switch (message.kind) {
    case 'histo1d':
      return buildHisto1d(create, message, spec);
    case 'histo2d':
      return buildHisto2d(create, message, spec);
    case 'uvw':
      return buildUvw(create, message, spec);
  }
}

/**
 * 描画オプション。2D は §11 決定どおり常に `colz`。
 * log は 1D が y、2D が z(JSROOT の DrawOptions は `;` を区切りとして無視する)。
 *
 * **2D は `nostat`**(029 必須処置 B = ユーザー裁定 2026-08-14、変更不可):
 * StripTime{U,V,W} とイベント表示の目的は**各ストリップの時間変化を一枚絵にすること**で、
 * 統計量としては意味を持たない(ついでに、ワイヤはビン値しか運ばないので stats box の
 * `Entries` は実際には Σbins = ΣADC になり、桁が全く違う数字が「Entries」と書かれてしまう)。
 * 1D(Charge / ChargeMax)は波高分布なので stats box を**残す** —— こちらは 1 エントリ
 * 1 カウントなので `fEntries = Σbins` が正しい値になる。
 */
export function panelDrawOption(message: PanelMessage, log: boolean): string {
  if (message.kind === 'histo1d') return log ? 'hist;logy' : 'hist';
  return log ? 'colz;logz;nostat' : 'colz;nostat';
}

function applySpec(histo: RootHisto, spec: PanelSpec): void {
  histo.fName = spec.name;
  histo.fTitle = spec.title;
  histo.fXaxis.fTitle = spec.xTitle;
  histo.fYaxis.fTitle = spec.yTitle;
}

/** `0x10 Histo1d` → TH1D。軸範囲はメッセージの xmin/xmax(§5.2 の 0–4096 固定はサーバ側の値)。 */
function buildHisto1d(create: HistFactory, message: Histo1dMessage, spec: PanelSpec): RootHisto {
  const histo = create('TH1D', message.nbins);
  applySpec(histo, spec);
  histo.fXaxis.fXmin = message.xmin;
  histo.fXaxis.fXmax = message.xmax;

  let sum = 0;
  for (let i = 0; i < message.nbins; i++) {
    const value = message.bins[i];
    histo.fArray[i + 1] = value;
    sum += value;
  }
  // 統計ボックスの Entries に「積算した重みの合計」を出す(表示専用。正値は monitor.root 側)。
  histo.fEntries = sum;
  return histo;
}

/** `0x11 Histo2d` → TH2D。ワイヤは iy 外側 row-major。 */
function buildHisto2d(create: HistFactory, message: Histo2dMessage, spec: PanelSpec): RootHisto {
  const { nx, ny } = message;
  const histo = create('TH2D', nx, ny);
  applySpec(histo, spec);
  histo.fXaxis.fXmin = message.xmin;
  histo.fXaxis.fXmax = message.xmax;
  histo.fYaxis.fXmin = message.ymin;
  histo.fYaxis.fXmax = message.ymax;

  const stride = nx + 2;
  let sum = 0;
  for (let iy = 0; iy < ny; iy++) {
    const src = iy * nx;
    const dst = (iy + 1) * stride + 1;
    for (let ix = 0; ix < nx; ix++) {
      const value = message.bins[src + ix];
      histo.fArray[dst + ix] = value;
      sum += value;
    }
  }
  histo.fEntries = sum;
  return histo;
}

/**
 * `0x02 Uvw` → TH2D(イベント表示 = R9。ヒストと同じ描画系に揃える — 028 発注書 4)。
 * 軸は x = strip 1..nStrips、y = bucket 0..nBuckets。どちらもメッセージ由来。
 */
function buildUvw(create: HistFactory, message: UvwMessage, spec: PanelSpec): RootHisto {
  const nx = message.nStrips;
  const ny = message.nBuckets;
  const histo = create('TH2D', nx, ny);
  applySpec(histo, spec);
  histo.fXaxis.fXmin = 1;
  histo.fXaxis.fXmax = nx + 1;
  histo.fYaxis.fXmin = 0;
  histo.fYaxis.fXmax = ny;

  const stride = nx + 2;
  let sum = 0;
  // ワイヤは strip-major(`idx=(strip-1)*nBuckets+bucket`)、ROOT は bucket が外側。
  for (let ix = 0; ix < nx; ix++) {
    const src = ix * ny;
    for (let iy = 0; iy < ny; iy++) {
      const value = message.adc[src + iy];
      histo.fArray[(iy + 1) * stride + (ix + 1)] = value;
      sum += value;
    }
  }
  histo.fEntries = sum;
  return histo;
}
