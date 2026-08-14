/**
 * SPEC §10.1 / §10.2 — WS バイナリメッセージの**本番デコーダ**。
 *
 * Angular に依存しない純 TS。vitest から直接 import できる(§10.4-2 の適合テストは
 * このファイルを import する = 「テストが見ているものと画面が見ているものが同じ」)。
 *
 * ## ゼロコピーについて(027 の判断)
 *
 * 13 B ヘッダのせいで本体の先頭オフセットは型のサイズに揃わない:
 *
 * | 型 | 本体オフセット | u16/f32 境界 |
 * |---|---|---|
 * | 0x02 Uvw       | 13 + 5 = 18 | 2 の倍数 ✔ |
 * | 0x03 Waveforms | 13 + 6 = 19 | 奇数 ✘ |
 * | 0x10 Histo1d   | 13 + 14 = 27 | 4 の倍数でない ✘ |
 * | 0x11 Histo2d   | 13 + 22 = 35 | 4 の倍数でない ✘ |
 *
 * つまり `new Uint16Array(buf, 19, n)` は 4 型中 3 型で RangeError になる。
 * 型ごとに「揃っていればビュー・揃っていなければコピー」と分岐すると、返す配列が
 * 「元バッファのエイリアス」か「独立したコピー」かが型によって変わり、呼び手
 * (028 の描画)が踏む落とし穴になる。**常に `ArrayBuffer.slice` で 1 回だけ
 * bulk copy し、必ず独立配列を返す**(= 呼び手から見て常に同じ意味)。
 * slice の返り値は新しい ArrayBuffer なので境界は常に満たされる。
 */

/** SPEC §10.1 の共通ヘッダ長。 */
export const WS_HEADER_LEN = 13;
/** マジック `'T'`(off 0)。 */
export const WS_MAGIC_T = 0x54;
/** マジック `'P'`(off 1)。 */
export const WS_MAGIC_P = 0x50;
/** SPEC §10.1: 型再定義のため 2。 */
export const WS_VERSION = 2;
/** off 4 の bit0 = incomplete event。 */
export const FLAG_INCOMPLETE = 0x01;

/** SPEC §10.2 の msgType。 */
export const MSG_UVW = 0x02;
export const MSG_WAVEFORMS = 0x03;
export const MSG_HISTO1D = 0x10;
export const MSG_HISTO2D = 0x11;

/** SPEC §10.1 の 13 B ヘッダ。 */
export interface WsHeader {
  readonly msgType: number;
  /** flags bit0。ヒスト系は常に false。 */
  readonly incomplete: boolean;
  readonly runNumber: number;
  /** ヒスト系は 0(SPEC §10.1)。 */
  readonly eventNumber: number;
}

/** `0x02 Uvw` — 面毎 `nStrips × nBuckets` の生 ADC グリッド。 */
export interface UvwMessage {
  readonly kind: 'uvw';
  readonly header: WsHeader;
  /** 0=U, 1=V, 2=W。 */
  readonly plane: number;
  readonly nStrips: number;
  readonly nBuckets: number;
  /** strip-major: `idx = (strip-1)*nBuckets + bucket`。 */
  readonly adc: Uint16Array;
}

/** `0x03 Waveforms` — (cobo,asad) 毎の dense 波形(FPN 込み・減算なし)。 */
export interface WaveformsMessage {
  readonly kind: 'waveforms';
  readonly header: WsHeader;
  readonly cobo: number;
  readonly asad: number;
  readonly nAget: number;
  readonly nCh: number;
  readonly nBuckets: number;
  /** aget-major: `idx = (aget*nCh + ch)*nBuckets + bucket`(raw ch 順)。 */
  readonly adc: Uint16Array;
}

/** `0x10 Histo1d`。 */
export interface Histo1dMessage {
  readonly kind: 'histo1d';
  readonly header: WsHeader;
  /** SPEC §5.2 の id(4–9 が 1D)。 */
  readonly id: number;
  readonly nbins: number;
  readonly xmin: number;
  readonly xmax: number;
  readonly bins: Float32Array;
}

/** `0x11 Histo2d`。 */
export interface Histo2dMessage {
  readonly kind: 'histo2d';
  readonly header: WsHeader;
  /** SPEC §5.2 の id(1–3 が 2D)。 */
  readonly id: number;
  readonly nx: number;
  readonly ny: number;
  readonly xmin: number;
  readonly xmax: number;
  readonly ymin: number;
  readonly ymax: number;
  /** **iy 外側 row-major**: `idx = iy*nx + ix`(monitor 側で転置済み)。 */
  readonly bins: Float32Array;
}

/** デコード不能の理由。UI は理由別に数えて status バーに出す(silent 禁止)。 */
export type DecodeErrorReason =
  'short-header' | 'bad-magic' | 'bad-version' | 'unknown-type' | 'truncated' | 'length-mismatch';

/** **例外は投げない**。呼び手が数えられるようにエラーを値で返す。 */
export interface DecodeError {
  readonly kind: 'error';
  readonly reason: DecodeErrorReason;
  readonly detail: string;
}

export type WsBinaryMessage =
  UvwMessage | WaveformsMessage | Histo1dMessage | Histo2dMessage | DecodeError;

export function isDecodeError(message: WsBinaryMessage): message is DecodeError {
  return message.kind === 'error';
}

/**
 * ワイヤは全フィールド LE。typed array は**実行環境のエンディアン**で読むので、
 * bulk copy 経路は LE のときだけ使い、BE 環境では DataView で 1 要素ずつ読む。
 * (現実の対象は x86_64 / arm64 = LE。BE でも黙って壊れないことを担保するだけ。)
 */
const PLATFORM_IS_LITTLE_ENDIAN = (() => {
  const probe = new DataView(new ArrayBuffer(2));
  probe.setUint16(0, 0x0102, true);
  return new Uint16Array(probe.buffer)[0] === 0x0102;
})();

let loggedBigEndian = false;

function noteBigEndian(): void {
  if (loggedBigEndian) return;
  loggedBigEndian = true;
  // silent にしない: 遅い経路に落ちたことを 1 回だけ知らせる。
  console.info('tpcdaq: big-endian platform — WS binary decoding uses the slow DataView path');
}

function readU16Array(buffer: ArrayBuffer, offset: number, count: number): Uint16Array {
  if (PLATFORM_IS_LITTLE_ENDIAN) {
    return new Uint16Array(buffer.slice(offset, offset + count * 2));
  }
  noteBigEndian();
  const view = new DataView(buffer);
  const out = new Uint16Array(count);
  for (let i = 0; i < count; i++) out[i] = view.getUint16(offset + i * 2, true);
  return out;
}

function readF32Array(buffer: ArrayBuffer, offset: number, count: number): Float32Array {
  if (PLATFORM_IS_LITTLE_ENDIAN) {
    return new Float32Array(buffer.slice(offset, offset + count * 4));
  }
  noteBigEndian();
  const view = new DataView(buffer);
  const out = new Float32Array(count);
  for (let i = 0; i < count; i++) out[i] = view.getFloat32(offset + i * 4, true);
  return out;
}

function fail(reason: DecodeErrorReason, detail: string): DecodeError {
  return { kind: 'error', reason, detail };
}

/**
 * 本体の実長と宣言長を突き合わせる。合っていれば `null`。
 *
 * 「短い」と「長い」を別の理由にするのは、前者が途中切断・後者が型の取り違えや
 * 版ずれを示すため(status バーで別々に見えたほうが原因に早く着く)。
 */
function checkLength(actual: number, declared: number, what: string): DecodeError | null {
  if (actual < declared) return fail('truncated', `${what}: ${actual} B < ${declared} B`);
  if (actual > declared) return fail('length-mismatch', `${what}: ${actual} B > ${declared} B`);
  return null;
}

/**
 * WS の 1 メッセージ(= ArrayBuffer 1 個)をデコードする。
 *
 * **例外を投げない。** 壊れた入力は [`DecodeError`] を返すので、呼び手が数える。
 */
export function decodeBinary(buffer: ArrayBuffer): WsBinaryMessage {
  const total = buffer.byteLength;
  if (total < WS_HEADER_LEN) {
    return fail('short-header', `${total} B < ${WS_HEADER_LEN} B`);
  }

  const view = new DataView(buffer);
  if (view.getUint8(0) !== WS_MAGIC_T || view.getUint8(1) !== WS_MAGIC_P) {
    return fail('bad-magic', `0x${view.getUint8(0).toString(16)}${view.getUint8(1).toString(16)}`);
  }

  const msgType = view.getUint8(2);
  const version = view.getUint8(3);
  if (version !== WS_VERSION) {
    return fail('bad-version', `version=${version} (expected ${WS_VERSION})`);
  }

  const header: WsHeader = {
    msgType,
    incomplete: (view.getUint8(4) & FLAG_INCOMPLETE) !== 0,
    runNumber: view.getUint32(5, true),
    eventNumber: view.getUint32(9, true),
  };

  switch (msgType) {
    case MSG_UVW:
      return decodeUvw(buffer, view, header, total);
    case MSG_WAVEFORMS:
      return decodeWaveforms(buffer, view, header, total);
    case MSG_HISTO1D:
      return decodeHisto1d(buffer, view, header, total);
    case MSG_HISTO2D:
      return decodeHisto2d(buffer, view, header, total);
    default:
      return fail('unknown-type', `msgType=0x${msgType.toString(16).padStart(2, '0')}`);
  }
}

/** `u8 plane`, `u16 nStrips`, `u16 nBuckets`, `u16 ADC × nStrips*nBuckets`。 */
function decodeUvw(
  buffer: ArrayBuffer,
  view: DataView,
  header: WsHeader,
  total: number,
): WsBinaryMessage {
  const fixed = WS_HEADER_LEN + 5;
  if (total < fixed) return fail('truncated', `uvw fixed fields: ${total} B < ${fixed} B`);

  const plane = view.getUint8(13);
  const nStrips = view.getUint16(14, true);
  const nBuckets = view.getUint16(16, true);
  const count = nStrips * nBuckets;
  const bad = checkLength(total, fixed + count * 2, 'uvw body');
  if (bad) return bad;

  return {
    kind: 'uvw',
    header,
    plane,
    nStrips,
    nBuckets,
    adc: readU16Array(buffer, fixed, count),
  };
}

/** `u8 cobo`, `u8 asad`, `u8 nAget`, `u8 nCh`, `u16 nBuckets`, `u16 ADC × nAget*nCh*nBuckets`。 */
function decodeWaveforms(
  buffer: ArrayBuffer,
  view: DataView,
  header: WsHeader,
  total: number,
): WsBinaryMessage {
  const fixed = WS_HEADER_LEN + 6;
  if (total < fixed) return fail('truncated', `waveforms fixed fields: ${total} B < ${fixed} B`);

  const cobo = view.getUint8(13);
  const asad = view.getUint8(14);
  const nAget = view.getUint8(15);
  const nCh = view.getUint8(16);
  const nBuckets = view.getUint16(17, true);
  const count = nAget * nCh * nBuckets;
  const bad = checkLength(total, fixed + count * 2, 'waveforms body');
  if (bad) return bad;

  return {
    kind: 'waveforms',
    header,
    cobo,
    asad,
    nAget,
    nCh,
    nBuckets,
    adc: readU16Array(buffer, fixed, count),
  };
}

/** `u16 id`, `u32 nbins`, `f32 xmin`, `f32 xmax`, `f32 × nbins`。 */
function decodeHisto1d(
  buffer: ArrayBuffer,
  view: DataView,
  header: WsHeader,
  total: number,
): WsBinaryMessage {
  const fixed = WS_HEADER_LEN + 14;
  if (total < fixed) return fail('truncated', `histo1d fixed fields: ${total} B < ${fixed} B`);

  const id = view.getUint16(13, true);
  const nbins = view.getUint32(15, true);
  const xmin = view.getFloat32(19, true);
  const xmax = view.getFloat32(23, true);
  const bad = checkLength(total, fixed + nbins * 4, 'histo1d bins');
  if (bad) return bad;

  return {
    kind: 'histo1d',
    header,
    id,
    nbins,
    xmin,
    xmax,
    bins: readF32Array(buffer, fixed, nbins),
  };
}

/** `u16 id`, `u16 nx`, `u16 ny`, `f32 xmin,xmax,ymin,ymax`, `f32 × nx*ny`。 */
function decodeHisto2d(
  buffer: ArrayBuffer,
  view: DataView,
  header: WsHeader,
  total: number,
): WsBinaryMessage {
  const fixed = WS_HEADER_LEN + 22;
  if (total < fixed) return fail('truncated', `histo2d fixed fields: ${total} B < ${fixed} B`);

  const id = view.getUint16(13, true);
  const nx = view.getUint16(15, true);
  const ny = view.getUint16(17, true);
  const xmin = view.getFloat32(19, true);
  const xmax = view.getFloat32(23, true);
  const ymin = view.getFloat32(27, true);
  const ymax = view.getFloat32(31, true);
  const count = nx * ny;
  const bad = checkLength(total, fixed + count * 4, 'histo2d bins');
  if (bad) return bad;

  return {
    kind: 'histo2d',
    header,
    id,
    nx,
    ny,
    xmin,
    xmax,
    ymin,
    ymax,
    bins: readF32Array(buffer, fixed, count),
  };
}
