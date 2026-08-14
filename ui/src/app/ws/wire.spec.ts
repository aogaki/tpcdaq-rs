/**
 * SPEC §10.1 / §10.2 — WS バイナリワイヤの TS 側独立レイアウトテスト(§10.4-4)。
 *
 * ここでの期待値は **Rust 実装のバイト列を写したものではなく**、SPEC の表から
 * 手で組み立てたもの。テスト側は DataView で 1 バイトずつ書き、本番デコーダが
 * それを復元できることを見る(= 二つの独立実装が同じ表を読んでいることの検査)。
 */
import {
  FLAG_INCOMPLETE,
  MSG_HISTO1D,
  MSG_HISTO2D,
  MSG_UVW,
  MSG_WAVEFORMS,
  WS_HEADER_LEN,
  WS_VERSION,
  decodeBinary,
  isDecodeError,
} from './wire';

interface Frame {
  readonly buffer: ArrayBuffer;
  readonly bytes: Uint8Array;
  readonly view: DataView;
}

/** `size` バイトの枠に SPEC §10.1 の 13 B ヘッダを手で書く(全フィールド LE)。 */
function frame(
  size: number,
  msgType: number,
  flags: number,
  runNumber: number,
  eventNumber: number,
): Frame {
  const buffer = new ArrayBuffer(size);
  const bytes = new Uint8Array(buffer);
  const view = new DataView(buffer);
  bytes[0] = 0x54; // 'T'
  bytes[1] = 0x50; // 'P'
  bytes[2] = msgType;
  bytes[3] = WS_VERSION;
  bytes[4] = flags;
  view.setUint32(5, runNumber, true);
  view.setUint32(9, eventNumber, true);
  return { buffer, bytes, view };
}

describe('SPEC §10.1 13 B ヘッダ', () => {
  it('ヘッダ長は 13 B(magic 2 + type 1 + version 1 + flags 1 + run 4 + event 4)', () => {
    expect(WS_HEADER_LEN).toBe(13);
    expect(2 + 1 + 1 + 1 + 4 + 4).toBe(WS_HEADER_LEN);
  });

  it('run / event は off 5 / off 9 の u32 LE として読まれる', () => {
    // Uvw の最小形(nStrips=1, nBuckets=1)でヘッダのオフセットだけを見る。
    const f = frame(WS_HEADER_LEN + 5 + 2, MSG_UVW, 0, 0x11223344, 0x55667788);
    f.view.setUint8(13, 2); // plane = W
    f.view.setUint16(14, 1, true); // nStrips
    f.view.setUint16(16, 1, true); // nBuckets
    f.view.setUint16(18, 4095, true); // ADC(飽和天井)

    const message = decodeBinary(f.buffer);

    expect(isDecodeError(message)).toBe(false);
    if (isDecodeError(message)) return;
    expect(message.header.msgType).toBe(MSG_UVW);
    expect(message.header.runNumber).toBe(0x11223344);
    expect(message.header.eventNumber).toBe(0x55667788);
    expect(message.header.incomplete).toBe(false);
  });

  it('flags bit0 が incomplete(他のビットは無視する)', () => {
    const f = frame(WS_HEADER_LEN + 5 + 2, MSG_UVW, FLAG_INCOMPLETE | 0x80, 1, 2);
    f.view.setUint16(14, 1, true);
    f.view.setUint16(16, 1, true);

    const message = decodeBinary(f.buffer);
    expect(isDecodeError(message)).toBe(false);
    if (isDecodeError(message)) return;
    expect(message.header.incomplete).toBe(true);
  });
});

describe('SPEC §10.2 0x02 Uvw', () => {
  it('plane / nStrips / nBuckets を off 13,14,16 から読み、ADC は strip-major', () => {
    // 非対称データ: 2 strip × 3 bucket。値 = strip*10 + bucket(手計算)。
    const nStrips = 2;
    const nBuckets = 3;
    const f = frame(WS_HEADER_LEN + 5 + nStrips * nBuckets * 2, MSG_UVW, 0, 7, 42);
    f.view.setUint8(13, 1); // plane = V
    f.view.setUint16(14, nStrips, true);
    f.view.setUint16(16, nBuckets, true);
    // idx = (strip-1)*nBuckets + bucket
    const expected = [10, 11, 12, 20, 21, 22];
    expected.forEach((value, idx) => f.view.setUint16(18 + idx * 2, value, true));

    const message = decodeBinary(f.buffer);

    expect(isDecodeError(message)).toBe(false);
    if (message.kind !== 'uvw') throw new Error('expected uvw');
    expect(message.plane).toBe(1);
    expect(message.nStrips).toBe(nStrips);
    expect(message.nBuckets).toBe(nBuckets);
    expect(Array.from(message.adc)).toEqual(expected);
    // strip 2 の bucket 1 = idx (2-1)*3 + 1 = 4 → 21
    expect(message.adc[(2 - 1) * nBuckets + 1]).toBe(21);
  });
});

describe('SPEC §10.2 0x03 Waveforms', () => {
  it('cobo/asad/nAget/nCh は off 13..16、nBuckets は off 17、ADC は aget-major', () => {
    // 2 AGET × 3 ch × 2 bucket。値 = aget*100 + ch*10 + bucket(手計算)。
    const nAget = 2;
    const nCh = 3;
    const nBuckets = 2;
    const total = nAget * nCh * nBuckets;
    const f = frame(WS_HEADER_LEN + 6 + total * 2, MSG_WAVEFORMS, FLAG_INCOMPLETE, 7, 43);
    f.view.setUint8(13, 0); // cobo
    f.view.setUint8(14, 1); // asad
    f.view.setUint8(15, nAget);
    f.view.setUint8(16, nCh);
    f.view.setUint16(17, nBuckets, true);
    const expected = [0, 1, 10, 11, 20, 21, 100, 101, 110, 111, 120, 121];
    expected.forEach((value, idx) => f.view.setUint16(19 + idx * 2, value, true));

    const message = decodeBinary(f.buffer);

    if (message.kind !== 'waveforms') throw new Error('expected waveforms');
    expect(message.header.incomplete).toBe(true);
    expect(message.cobo).toBe(0);
    expect(message.asad).toBe(1);
    expect(message.nAget).toBe(nAget);
    expect(message.nCh).toBe(nCh);
    expect(message.nBuckets).toBe(nBuckets);
    expect(Array.from(message.adc)).toEqual(expected);
    // aget 1 / ch 2 / bucket 0 = idx ((1*3)+2)*2 + 0 = 10 → 120
    expect(message.adc[(1 * nCh + 2) * nBuckets + 0]).toBe(120);
  });
});

describe('SPEC §10.2 0x10 Histo1d', () => {
  it('id(u16)/ nbins(u32)/ xmin,xmax(f32)を off 13,15,19,23 から読む', () => {
    const nbins = 4;
    const f = frame(WS_HEADER_LEN + 2 + 4 + 4 + 4 + nbins * 4, MSG_HISTO1D, 0, 7, 0);
    f.view.setUint16(13, 4, true); // id = 4 → ChargeU(SPEC §5.2)
    f.view.setUint32(15, nbins, true);
    f.view.setFloat32(19, 0, true);
    f.view.setFloat32(23, 4096, true); // §5.2: x レンジ 0–4096 固定
    // 2 進で厳密に表せる値だけ使う(f32 往復で値が変わらない)。
    const expected = [0, 1.5, 2.25, 3.75];
    expected.forEach((value, idx) => f.view.setFloat32(27 + idx * 4, value, true));

    const message = decodeBinary(f.buffer);

    if (message.kind !== 'histo1d') throw new Error('expected histo1d');
    expect(message.id).toBe(4);
    expect(message.nbins).toBe(nbins);
    expect(message.xmin).toBe(0);
    expect(message.xmax).toBe(4096);
    expect(message.header.eventNumber).toBe(0); // ヒストは 0(SPEC §10.1)
    expect(Array.from(message.bins)).toEqual(expected);
  });
});

describe('SPEC §10.2 0x11 Histo2d', () => {
  it('id/nx/ny + 4 つの f32 軸を読み、ビンは iy 外側 row-major', () => {
    const nx = 3;
    const ny = 2;
    const f = frame(WS_HEADER_LEN + 2 + 2 + 2 + 16 + nx * ny * 4, MSG_HISTO2D, 0, 7, 0);
    f.view.setUint16(13, 1, true); // id = 1 → StripTimeU
    f.view.setUint16(15, nx, true);
    f.view.setUint16(17, ny, true);
    f.view.setFloat32(19, 1, true); // xmin(strip は 1 始まり)
    f.view.setFloat32(23, 4, true); // xmax = nx + 1
    f.view.setFloat32(27, 0, true); // ymin
    f.view.setFloat32(31, 2, true); // ymax = ny
    // iy 外側: [ (ix0,iy0), (ix1,iy0), (ix2,iy0), (ix0,iy1), ... ]
    const expected = [11, 21, 31, 12, 22, 32];
    expected.forEach((value, idx) => f.view.setFloat32(35 + idx * 4, value, true));

    const message = decodeBinary(f.buffer);

    if (message.kind !== 'histo2d') throw new Error('expected histo2d');
    expect(message.id).toBe(1);
    expect(message.nx).toBe(nx);
    expect(message.ny).toBe(ny);
    expect(message.xmin).toBe(1);
    expect(message.xmax).toBe(4);
    expect(message.ymin).toBe(0);
    expect(message.ymax).toBe(2);
    expect(Array.from(message.bins)).toEqual(expected);
    // (ix=2, iy=1) = idx iy*nx + ix = 1*3 + 2 = 5 → 32
    expect(message.bins[1 * nx + 2]).toBe(32);
  });
});

describe('異常入力は例外を投げずエラー値を返す(silent 禁止 = 呼び手が数える)', () => {
  /** 正常な Uvw(1 strip × 1 bucket)を作る。壊すのは呼び手側。 */
  function goodUvw(size = WS_HEADER_LEN + 5 + 2): Frame {
    const f = frame(size, MSG_UVW, 0, 7, 42);
    f.view.setUint8(13, 0);
    f.view.setUint16(14, 1, true);
    f.view.setUint16(16, 1, true);
    f.view.setUint16(18, 123, true);
    return f;
  }

  it('13 B に満たない → short-header', () => {
    const message = decodeBinary(new ArrayBuffer(12));
    expect(isDecodeError(message)).toBe(true);
    if (!isDecodeError(message)) return;
    expect(message.reason).toBe('short-header');
  });

  it('magic が TP でない → bad-magic', () => {
    const f = goodUvw();
    f.bytes[1] = 0x51; // 'Q'
    const message = decodeBinary(f.buffer);
    if (!isDecodeError(message)) throw new Error('expected an error value');
    expect(message.reason).toBe('bad-magic');
  });

  it('version が 2 でない → bad-version', () => {
    const f = goodUvw();
    f.bytes[3] = 1;
    const message = decodeBinary(f.buffer);
    if (!isDecodeError(message)) throw new Error('expected an error value');
    expect(message.reason).toBe('bad-version');
  });

  it('知らない msgType → unknown-type(旧 0x01 Event は廃止 = 未知)', () => {
    const f = goodUvw();
    f.bytes[2] = 0x01;
    const message = decodeBinary(f.buffer);
    if (!isDecodeError(message)) throw new Error('expected an error value');
    expect(message.reason).toBe('unknown-type');
    expect(message.detail).toContain('0x01');
  });

  it('宣言より本体が短い → truncated', () => {
    const f = goodUvw();
    // nStrips=9 と宣言する(本体は 1 ビン分しかない)。
    f.view.setUint16(14, 9, true);
    const message = decodeBinary(f.buffer);
    if (!isDecodeError(message)) throw new Error('expected an error value');
    expect(message.reason).toBe('truncated');
  });

  it('宣言より本体が長い → length-mismatch(黙って切り捨てない)', () => {
    const f = goodUvw(WS_HEADER_LEN + 5 + 2 + 2); // 末尾に 2 B 余分
    const message = decodeBinary(f.buffer);
    if (!isDecodeError(message)) throw new Error('expected an error value');
    expect(message.reason).toBe('length-mismatch');
  });

  it('固定フィールドの途中で切れている → truncated', () => {
    const f = goodUvw();
    const short = f.buffer.slice(0, WS_HEADER_LEN + 3);
    const message = decodeBinary(short);
    if (!isDecodeError(message)) throw new Error('expected an error value');
    expect(message.reason).toBe('truncated');
  });
});
