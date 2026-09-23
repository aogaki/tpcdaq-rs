/**
 * TODO/079 §2.8 — Slow control ビューの純ロジック(Angular に依存しない)。
 *
 * `slow-view.ts` は ECharts の遅延ロードと DOM しか扱わない(`waveform-select.ts` /
 * `root-histo.ts` と同じ切り分け)。ここでテストするのは:
 * - Trend のリングバッファ(上限・順序・系列分離)
 * - Latest 表の安定ソート(device → node → param)と age/stale 判定
 * - status バーの alarm 集計
 * - Trend 系列選択の上限(最大 4、silent に落とさない)
 * - 表示用フォーマッタ
 */
import type { SlowSample } from '../ws/json';
import {
  MAX_TREND_SERIES,
  SLOW_HISTORY_LIMIT,
  STALE_AGE_MS,
  alarmLabels,
  appendSlowSamples,
  capSeriesSelection,
  countAlarms,
  formatAgeMs,
  formatSlowValue,
  slowSeriesKey,
  sortedSlowRows,
} from './slow-history';

/** SPEC §9.2 の `slow_latest` キーと同じ組み立て(`"<device>/<node>/<param>"`、node 無しは `"-"`)。 */
describe('slowSeriesKey — SPEC §9.2 slow_latest キーと同じ組み立て', () => {
  it('node ありは番号、node 無しは "-"', () => {
    expect(slowSeriesKey('gas', 3, 'fmeasure')).toBe('gas/3/fmeasure');
    expect(slowSeriesKey('env', null, 't_c')).toBe('env/-/t_c');
  });
});

function sample(overrides: Partial<SlowSample> = {}): SlowSample {
  return {
    device: 'gas',
    node: 3,
    param: 'fmeasure',
    value: 100.5,
    unit: 'mbar',
    ts_ms: 1_700_000_000_000,
    alarm: false,
    ...overrides,
  };
}

describe('appendSlowSamples — Trend のリングバッファ(1 系列 1,800 点 = 1 Hz × 30 min)', () => {
  it('異なる (device,node,param) は別系列として増える', () => {
    const history = appendSlowSamples(new Map(), [
      sample({ device: 'gas', node: 3, param: 'fmeasure', ts_ms: 1, value: 100 }),
      sample({ device: 'env', node: null, param: 't_c', ts_ms: 1, value: 24 }),
    ]);
    expect([...history.keys()].sort()).toEqual(['env/-/t_c', 'gas/3/fmeasure']);
    expect(history.get('gas/3/fmeasure')).toEqual([{ ts_ms: 1, value: 100 }]);
    expect(history.get('env/-/t_c')).toEqual([{ ts_ms: 1, value: 24 }]);
  });

  it('同じ系列は末尾に足される(順序を保つ)', () => {
    let history = appendSlowSamples(new Map(), [sample({ ts_ms: 1, value: 10 })]);
    history = appendSlowSamples(history, [sample({ ts_ms: 2, value: 20 })]);
    expect(history.get('gas/3/fmeasure')).toEqual([
      { ts_ms: 1, value: 10 },
      { ts_ms: 2, value: 20 },
    ]);
  });

  it(`上限 ${SLOW_HISTORY_LIMIT} 点(SLOW_HISTORY_LIMIT)を超えたら最古から捨てる`, () => {
    // 手計算: ts_ms 1..1801(1801 点)を 1 本ずつ足す。上限 1800 なので
    // 先頭(ts_ms=1)だけが落ち、残るのは ts_ms 2..1801(長さ 1800、先頭が 2、末尾が 1801)。
    let history: ReturnType<typeof appendSlowSamples> = new Map();
    for (let ts = 1; ts <= SLOW_HISTORY_LIMIT + 1; ts++) {
      history = appendSlowSamples(history, [sample({ ts_ms: ts, value: ts })]);
    }
    const points = history.get('gas/3/fmeasure') ?? [];
    expect(points.length).toBe(SLOW_HISTORY_LIMIT);
    expect(points[0]).toEqual({ ts_ms: 2, value: 2 });
    expect(points[points.length - 1]).toEqual({
      ts_ms: SLOW_HISTORY_LIMIT + 1,
      value: SLOW_HISTORY_LIMIT + 1,
    });
  });

  it('空の samples は何もしない(参照を保つ)', () => {
    const history = appendSlowSamples(new Map(), [sample({ ts_ms: 1 })]);
    expect(appendSlowSamples(history, [])).toBe(history);
  });
});

describe('sortedSlowRows — Latest 表(device → node → param の順で安定ソート)', () => {
  it('非対称フィクスチャ: device 違い→node 違い(null は先頭)→param 違いの 3 段で並ぶ', () => {
    // 手計算の並び: "env" < "gas"(device で確定)。gas の中は node null(表示 "-")が最小、
    // 次に 3、次に 5。node 3 の中では "alarm_info" < "fmeasure"(param で確定)。
    const samples: SlowSample[] = [
      sample({ device: 'gas', node: 5, param: 'fmeasure', value: 1 }),
      sample({ device: 'env', node: null, param: 't_c', value: 2 }),
      sample({ device: 'gas', node: 3, param: 'fmeasure', value: 3 }),
      sample({ device: 'gas', node: null, param: 'valve', value: 4 }),
      sample({ device: 'gas', node: 3, param: 'alarm_info', value: 5 }),
    ];
    const rows = sortedSlowRows(samples, 0);
    expect(rows.map((row) => row.key)).toEqual([
      'env/-/t_c',
      'gas/-/valve',
      'gas/3/alarm_info',
      'gas/3/fmeasure',
      'gas/5/fmeasure',
    ]);
  });

  it(`age = now − ts_ms、${STALE_AGE_MS} ms(STALE_AGE_MS)超で stale`, () => {
    // 手計算: now=10_000。ts_ms=4_000 → age=6_000(> 5_000 → stale)。
    // ts_ms=9_000 → age=1_000(≤ 5_000 → not stale)。
    const rows = sortedSlowRows(
      [
        sample({ device: 'a', node: 1, ts_ms: 4_000 }),
        sample({ device: 'b', node: 1, ts_ms: 9_000 }),
      ],
      10_000,
    );
    const a = rows.find((row) => row.device === 'a');
    const b = rows.find((row) => row.device === 'b');
    expect(a?.ageMs).toBe(6_000);
    expect(a?.stale).toBe(true);
    expect(b?.ageMs).toBe(1_000);
    expect(b?.stale).toBe(false);
  });

  it('now < ts_ms(時計ずれ)でも age は負にしない(0 に丸める)', () => {
    const rows = sortedSlowRows([sample({ ts_ms: 5_000 })], 1_000);
    expect(rows[0].ageMs).toBe(0);
    expect(rows[0].stale).toBe(false);
  });

  it('alarm フラグをそのまま運ぶ', () => {
    const rows = sortedSlowRows([sample({ alarm: true })], 0);
    expect(rows[0].alarm).toBe(true);
  });
});

describe('countAlarms / alarmLabels — status バーの slow バッジ(発注書 §2.8-4)', () => {
  // 非対称: alarm=true が 2 件(node あり/なし混在)、alarm=false が 1 件。
  const samples: SlowSample[] = [
    sample({ device: 'gas', node: 3, param: 'fmeasure', alarm: true }),
    sample({ device: 'env', node: null, param: 't_c', alarm: true }),
    sample({ device: 'gas', node: 5, param: 'fmeasure', alarm: false }),
  ];

  it('alarm=true の件数だけ数える', () => {
    expect(countAlarms(samples)).toBe(2);
    expect(countAlarms([])).toBe(0);
  });

  it('alarm=true の device/param 一覧(node があれば添える)', () => {
    expect(alarmLabels(samples)).toEqual(['gas/3/fmeasure', 'env/t_c']);
  });
});

describe('capSeriesSelection — Trend は最大 4 系列(超えた分は silent に描かない)', () => {
  it(`${MAX_TREND_SERIES} 本(MAX_TREND_SERIES)以下ならそのまま通す`, () => {
    const result = capSeriesSelection(['a', 'b', 'c']);
    expect(result).toEqual({ kept: ['a', 'b', 'c'], droppedCount: 0 });
  });

  it('5 本要求したら先頭 4 本だけ残し、落とした本数を返す', () => {
    const result = capSeriesSelection(['a', 'b', 'c', 'd', 'e']);
    expect(result.kept).toEqual(['a', 'b', 'c', 'd']);
    expect(result.droppedCount).toBe(1);
  });
});

describe('formatSlowValue / formatAgeMs — 表示専用フォーマッタ(単位換算はしない)', () => {
  it('整数はそのまま、小数は 3 桁に丸める', () => {
    expect(formatSlowValue(100)).toBe('100');
    // 手計算: 100.056789 は小数第 4 位が 7(≥5)なので第 3 位を切り上げ → 100.057
    expect(formatSlowValue(100.056789)).toBe('100.057');
  });

  it('age はミリ秒を秒(小数 1 桁)にする', () => {
    // 手計算: 1234 ms / 1000 = 1.234 → 小数 1 桁で 1.2、6500 ms / 1000 = 6.5
    expect(formatAgeMs(1234)).toBe('1.2 s');
    expect(formatAgeMs(6500)).toBe('6.5 s');
  });
});
