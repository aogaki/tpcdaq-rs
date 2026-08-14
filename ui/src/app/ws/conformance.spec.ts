/**
 * SPEC §10.4 — クロス言語適合性テスト(TS 側 = §10.4-2/3)。
 *
 * Rust の `ws_proto_sample`(本番エンコーダ)が書いた `u32 LE 長さ + ペイロード` の
 * 連結ストリームを、**本番デコーダ**(`./wire`)で分解・復元して既知値と突き合わせる。
 *
 * 期待値は `tpcdaq::monitor::ws_sample_messages()`(src/monitor.rs)の**ロジックを読んで
 * 独立に起こしたもの**(バイト列は写していない)。生成規則は次のとおり:
 *
 * - Uvw       : run=7 / event=42 / plane=V / 3 strip × 4 bucket、値 = strip*10 + bucket
 * - Waveforms : run=7 / event=43 / incomplete / cobo=0 / asad=1 / 2 AGET × 3 ch × 2 bucket、
 *               値 = aget*100 + ch*10 + bucket
 * - Histo1d   : run=7 / id=4(ChargeU)/ 4 ビン / x=[0,4096](§5.2 の固定レンジ)/
 *               値 = [0, 1.5, 2.25, 3.75]
 * - Histo2d   : run=7 / id=1(StripTimeU)/ nx=3 × ny=2 / x=[1,4] y=[0,2] /
 *               PUB 順(ix 外側)の [11,12,21,22,31,32] が **iy 外側へ転置**されて届く
 *
 * フィクスチャは毎回再生成する(コミットしない)。走らせ方は `ui/run_ws_conformance.sh`。
 */
import { readFileSync } from 'node:fs';

import { decodeBinary, isDecodeError, type WsBinaryMessage } from './wire';

const samplePath = process.env['TPCDAQ_WS_SAMPLE'] ?? '';

/** float の突き合わせ許容差(SPEC §10.4-2)。 */
const EPS = 1e-5;

/** `u32 LE 長さ + ペイロード` の連結を 1 通ずつに割る。 */
function splitFrames(bytes: Uint8Array): ArrayBuffer[] {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const frames: ArrayBuffer[] = [];
  let at = 0;
  while (at < bytes.byteLength) {
    if (at + 4 > bytes.byteLength) throw new Error(`dangling length prefix at ${at}`);
    const length = view.getUint32(at, true);
    at += 4;
    if (at + length > bytes.byteLength) {
      throw new Error(`frame at ${at} claims ${length} B but only ${bytes.byteLength - at} B left`);
    }
    // ここで 1 回コピーする(呼び手には常に独立した ArrayBuffer を渡す)。
    const frame = new ArrayBuffer(length);
    new Uint8Array(frame).set(bytes.subarray(at, at + length));
    frames.push(frame);
    at += length;
  }
  return frames;
}

function expectClose(actual: number, expected: number, what: string): void {
  expect(Math.abs(actual - expected), `${what}: ${actual} vs ${expected}`).toBeLessThanOrEqual(EPS);
}

describe.skipIf(samplePath === '')('SPEC §10.4 適合(Rust 生成 → TS 本番デコーダ)', () => {
  let messages: WsBinaryMessage[] = [];

  beforeAll(() => {
    const bytes = readFileSync(samplePath);
    messages = splitFrames(new Uint8Array(bytes)).map(decodeBinary);
  });

  it('4 通すべてがデコードできる(0x02 / 0x03 / 0x10 / 0x11)', () => {
    expect(messages).toHaveLength(4);
    const broken = messages.filter(isDecodeError);
    expect(broken, JSON.stringify(broken)).toEqual([]);
    expect(messages.map((m) => m.kind)).toEqual(['uvw', 'waveforms', 'histo1d', 'histo2d']);
  });

  it('0x02 Uvw = V 面 3×4、値は strip*10 + bucket', () => {
    const message = messages[0];
    if (message.kind !== 'uvw') throw new Error('expected uvw');
    expect(message.header.runNumber).toBe(7);
    expect(message.header.eventNumber).toBe(42);
    expect(message.header.incomplete).toBe(false);
    expect(message.plane).toBe(1);
    expect(message.nStrips).toBe(3);
    expect(message.nBuckets).toBe(4);

    for (let strip = 1; strip <= 3; strip++) {
      for (let bucket = 0; bucket < 4; bucket++) {
        expect(message.adc[(strip - 1) * 4 + bucket]).toBe(strip * 10 + bucket);
      }
    }
  });

  it('0x03 Waveforms = incomplete / cobo0 asad1 / aget-major', () => {
    const message = messages[1];
    if (message.kind !== 'waveforms') throw new Error('expected waveforms');
    expect(message.header.runNumber).toBe(7);
    expect(message.header.eventNumber).toBe(43);
    expect(message.header.incomplete).toBe(true); // flags bit0
    expect(message.cobo).toBe(0);
    expect(message.asad).toBe(1);
    expect(message.nAget).toBe(2);
    expect(message.nCh).toBe(3);
    expect(message.nBuckets).toBe(2);

    for (let aget = 0; aget < 2; aget++) {
      for (let ch = 0; ch < 3; ch++) {
        for (let bucket = 0; bucket < 2; bucket++) {
          const idx = (aget * 3 + ch) * 2 + bucket;
          expect(message.adc[idx]).toBe(aget * 100 + ch * 10 + bucket);
        }
      }
    }
  });

  it('0x10 Histo1d = id 4 / 4 ビン / x レンジ [0,4096]', () => {
    const message = messages[2];
    if (message.kind !== 'histo1d') throw new Error('expected histo1d');
    expect(message.header.runNumber).toBe(7);
    expect(message.header.eventNumber).toBe(0); // ヒストは 0
    expect(message.id).toBe(4);
    expect(message.nbins).toBe(4);
    expectClose(message.xmin, 0, 'xmin');
    expectClose(message.xmax, 4096, 'xmax');

    const expected = [0, 1.5, 2.25, 3.75];
    expect(message.bins).toHaveLength(expected.length);
    expected.forEach((value, idx) => expectClose(message.bins[idx], value, `bin ${idx}`));
  });

  it('0x11 Histo2d = id 1 / 3×2 / PUB の ix 外側が iy 外側へ転置されている', () => {
    const message = messages[3];
    if (message.kind !== 'histo2d') throw new Error('expected histo2d');
    expect(message.header.runNumber).toBe(7);
    expect(message.header.eventNumber).toBe(0);
    expect(message.id).toBe(1);
    expect(message.nx).toBe(3);
    expect(message.ny).toBe(2);
    expectClose(message.xmin, 1, 'xmin');
    expectClose(message.xmax, 4, 'xmax');
    expectClose(message.ymin, 0, 'ymin');
    expectClose(message.ymax, 2, 'ymax');

    // Rust 側の入力(§5.3 の PUB 順 = ix 外側): [11,12, 21,22, 31,32]
    const pubOrder = [11, 12, 21, 22, 31, 32];
    // §10.2 のワイヤ順 = iy 外側 row-major。idx = iy*nx + ix ← pubOrder[ix*ny + iy]
    expect(message.bins).toHaveLength(6);
    for (let iy = 0; iy < 2; iy++) {
      for (let ix = 0; ix < 3; ix++) {
        expectClose(message.bins[iy * 3 + ix], pubOrder[ix * 2 + iy], `bin (${ix},${iy})`);
      }
    }
  });
});
