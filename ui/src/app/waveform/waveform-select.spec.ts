/**
 * R13 / SPEC §10.2 `0x03 Waveforms` — 波形ビューの選択と**クライアント側間引き**。
 *
 * 間引きは「落としてよいが silent にしない」側(CLAUDE.md の絶対ルール)。
 * 落とした系列数・サンプル数を戻り値に載せ、画面に出せるようにする。
 */
import type { WaveformsMessage } from '../ws/wire';
import { selectWaveforms, updateWaveformCache, waveformKey } from './waveform-select';

/**
 * 非対称フィクスチャ: nAget=2, nCh=3, nBuckets=4。
 * `adc[(aget*nCh+ch)*nBuckets+bucket] = aget*100 + ch*10 + bucket`
 * (027 の §10.4 適合フィクスチャと同じ作り方 = 添字の取り違えが必ず値に出る)。
 */
function fixture(): WaveformsMessage {
  const nAget = 2;
  const nCh = 3;
  const nBuckets = 4;
  const adc = new Uint16Array(nAget * nCh * nBuckets);
  for (let aget = 0; aget < nAget; aget++) {
    for (let ch = 0; ch < nCh; ch++) {
      for (let bucket = 0; bucket < nBuckets; bucket++) {
        adc[(aget * nCh + ch) * nBuckets + bucket] = aget * 100 + ch * 10 + bucket;
      }
    }
  }
  return {
    kind: 'waveforms',
    header: { msgType: 0x03, incomplete: false, runNumber: 7, eventNumber: 43 },
    cobo: 0,
    asad: 1,
    nAget,
    nCh,
    nBuckets,
    adc,
  };
}

describe('(cobo,asad) の選択 — 最新 1 通しか来ないので view 側で面毎に覚える', () => {
  it('キーは cobo/asad の組(表示順が安定するように 0 詰めしない素の数)', () => {
    expect(waveformKey(0, 1)).toBe('cobo0/asad1');
    expect(waveformKey(1, 3)).toBe('cobo1/asad3');
  });

  it('同じ (cobo,asad) は最新で置き換わり、別の組は増える', () => {
    const first = fixture();
    const second: WaveformsMessage = {
      ...first,
      header: { ...first.header, eventNumber: 44 },
    };
    const other: WaveformsMessage = { ...first, asad: 2 };

    let cache = updateWaveformCache(new Map(), first);
    expect([...cache.keys()]).toEqual(['cobo0/asad1']);

    cache = updateWaveformCache(cache, second);
    expect([...cache.keys()]).toEqual(['cobo0/asad1']);
    expect(cache.get('cobo0/asad1')?.header.eventNumber).toBe(44);

    cache = updateWaveformCache(cache, other);
    expect([...cache.keys()].sort()).toEqual(['cobo0/asad1', 'cobo0/asad2']);
  });
});

describe('AGET / ch の絞り込み(nAget・nCh はメッセージ由来 = 焼き込み禁止)', () => {
  it('選んだ (aget,ch) だけを aget-major の正しい位置から取り出す', () => {
    const plot = selectWaveforms(fixture(), {
      agets: [1],
      channels: [0, 2],
      maxSeries: 10,
      maxPoints: 4,
    });
    expect(plot.series.map((s) => [s.aget, s.ch])).toEqual([
      [1, 0],
      [1, 2],
    ]);
    // 手計算: aget=1,ch=0 → 100+0+bucket = 100,101,102,103
    expect(plot.series[0].points).toEqual([
      [0, 100],
      [1, 101],
      [2, 102],
      [3, 103],
    ]);
    // 手計算: aget=1,ch=2 → 100+20+bucket = 120,121,122,123
    expect(plot.series[1].points).toEqual([
      [0, 120],
      [1, 121],
      [2, 122],
      [3, 123],
    ]);
    expect(plot.stride).toBe(1);
    expect(plot.droppedSeries).toBe(0);
    expect(plot.droppedSamplesPerSeries).toBe(0);
    expect(plot.outOfRange).toBe(0);
  });

  it('範囲外の aget / ch は例外にせず数える(silent 禁止)', () => {
    const plot = selectWaveforms(fixture(), {
      agets: [0, 5],
      channels: [0, 9],
      maxSeries: 10,
      maxPoints: 4,
    });
    // 要求 4 組のうち有効は (0,0) だけ。手計算: 4 - 1 = 3 が範囲外。
    expect(plot.series.map((s) => [s.aget, s.ch])).toEqual([[0, 0]]);
    expect(plot.requestedSeries).toBe(1);
    expect(plot.outOfRange).toBe(3);
  });
});

describe('クライアント側間引き(上限・stride・件数の申告)', () => {
  it('系列数の上限を超えた分は描かず、落とした数を申告する', () => {
    const plot = selectWaveforms(fixture(), {
      agets: [0, 1],
      channels: [0, 1, 2],
      maxSeries: 2,
      maxPoints: 4,
    });
    // 手計算: 2 AGET × 3 ch = 6 系列要求、上限 2 → 4 系列を落とす。
    expect(plot.requestedSeries).toBe(6);
    expect(plot.series.length).toBe(2);
    expect(plot.droppedSeries).toBe(4);
    // 先頭から順(aget 昇順 → ch 昇順)。決定的であること。
    expect(plot.series.map((s) => [s.aget, s.ch])).toEqual([
      [0, 0],
      [0, 1],
    ]);
  });

  it('サンプルは stride で間引き、落とした点数を申告する', () => {
    const plot = selectWaveforms(fixture(), {
      agets: [0],
      channels: [1],
      maxSeries: 10,
      maxPoints: 2,
    });
    // 手計算: stride = ceil(4 / 2) = 2 → bucket 0, 2 の 2 点。落とした点 = 4 - 2 = 2。
    expect(plot.stride).toBe(2);
    expect(plot.droppedSamplesPerSeries).toBe(2);
    // 手計算: aget=0,ch=1 → 10+bucket。bucket 0 → 10、bucket 2 → 12。
    expect(plot.series[0].points).toEqual([
      [0, 10],
      [2, 12],
    ]);
  });

  it('maxPoints が nBuckets 以上なら間引かない(stride=1)', () => {
    const plot = selectWaveforms(fixture(), {
      agets: [0],
      channels: [0],
      maxSeries: 10,
      maxPoints: 4096,
    });
    expect(plot.stride).toBe(1);
    expect(plot.droppedSamplesPerSeries).toBe(0);
    expect(plot.series[0].points.length).toBe(4);
  });

  it('nBuckets はメッセージ由来でそのまま返す(x 軸を焼き込まないため)', () => {
    const plot = selectWaveforms(fixture(), {
      agets: [0],
      channels: [0],
      maxSeries: 10,
      maxPoints: 4,
    });
    expect(plot.nBuckets).toBe(4);
  });

  it('何も選ばれていなければ空の結果(例外を投げない)', () => {
    const plot = selectWaveforms(fixture(), {
      agets: [],
      channels: [0],
      maxSeries: 10,
      maxPoints: 4,
    });
    expect(plot.series).toEqual([]);
    expect(plot.requestedSeries).toBe(0);
    expect(plot.droppedSeries).toBe(0);
    expect(plot.outOfRange).toBe(0);
  });
});
