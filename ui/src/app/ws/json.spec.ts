/**
 * SPEC §10.3 — JSON テキストメッセージの TS 側テスト。
 *
 * 一番の関心事は **casing**: status 本体は §5.3 のフィールドをそのまま出すので
 * snake_case、monitor が足す 3 つだけ camelCase(`monitorGaps` / `clients` /
 * `wsDropped`)。ここを取り違えると「0 が表示され続ける」silent failure になる。
 */
import { DEFAULT_STREAMS, encodeSubscribe, parseJsonMessage, saturationPercent } from './json';

/** monitor(src/monitor.rs `status_json`)が出す形の status を組む。 */
function statusText(overrides: Record<string, unknown> = {}): string {
  return JSON.stringify({
    // serde が StatusPayload をそのまま出すので `kind` も載る(`type` は monitor が足す)。
    kind: 'status',
    type: 'status',
    run: 7,
    state: 'running',
    events_built: 108,
    events_incomplete: 2,
    late_fragments: 3,
    pending_events: 4,
    frames_per_cobo: { '0': 5, '1': 6 },
    bytes_written: 30108684,
    saturation: {
      U: { saturated: 1, counted: 4 },
      V: { saturated: 0, counted: 0 },
      W: { saturated: 3, counted: 6 },
    },
    publish_drops: 107,
    monitorGaps: 9,
    clients: 2,
    wsDropped: 11,
    ...overrides,
  });
}

describe('SPEC §10.3 status', () => {
  it('本体は snake_case、monitor の 3 つだけ camelCase', () => {
    const parsed = parseJsonMessage(statusText());

    if (parsed.kind !== 'status') throw new Error(`expected status, got ${parsed.kind}`);
    const s = parsed.status;
    expect(s.run).toBe(7);
    expect(s.state).toBe('running');
    expect(s.events_built).toBe(108);
    expect(s.events_incomplete).toBe(2);
    expect(s.late_fragments).toBe(3);
    expect(s.pending_events).toBe(4);
    expect(s.bytes_written).toBe(30108684);
    expect(s.publish_drops).toBe(107);
    expect(s.frames_per_cobo).toEqual({ '0': 5, '1': 6 });
    expect(s.saturation['W']).toEqual({ saturated: 3, counted: 6 });
    // monitor が足す 3 つ(camelCase)
    expect(s.monitorGaps).toBe(9);
    expect(s.clients).toBe(2);
    expect(s.wsDropped).toBe(11);
    expect(parsed.unknownKeys).toEqual([]);
  });

  it('camelCase で来た本体キーは受け付けない(casing の取り違えを検出する)', () => {
    const text = JSON.stringify({ type: 'status', run: 7, state: 'running', eventsBuilt: 108 });
    const parsed = parseJsonMessage(text);
    expect(parsed.kind).toBe('error');
    if (parsed.kind !== 'error') return;
    expect(parsed.reason).toBe('bad-field');
    expect(parsed.detail).toContain('events_built');
  });

  it('知らないキーは落とさず数える(前方互換)', () => {
    const parsed = parseJsonMessage(statusText({ future_counter: 1, anotherThing: 'x' }));
    if (parsed.kind !== 'status') throw new Error('expected status');
    expect(parsed.status.events_built).toBe(108); // 既知フィールドは無事
    expect([...parsed.unknownKeys].sort()).toEqual(['anotherThing', 'future_counter']);
  });
});

describe('SPEC §5.2 飽和率 = saturated / counted * 100', () => {
  it('counted > 0 なら %、counted == 0 は null(= 画面では「—」)', () => {
    // 手計算: 1/4 = 25 %、3/6 = 50 %
    expect(saturationPercent({ saturated: 1, counted: 4 })).toBe(25);
    expect(saturationPercent({ saturated: 3, counted: 6 })).toBe(50);
    expect(saturationPercent({ saturated: 0, counted: 0 })).toBeNull();
    expect(saturationPercent(undefined)).toBeNull();
  });
});

describe('SPEC §10.3 meta', () => {
  const metaText = JSON.stringify({
    type: 'meta',
    nBuckets: 512,
    planes: { U: 72, V: 92, W: 92 }, // mini eTPC(SPEC §5.2)
    geometry: 'geometry_mini_reduced.dat',
    anglesDeg: [90, 30, 150],
    detector: 'mini_eTPC',
    cobos: [0],
    run: 7,
  });

  it('meta は全部 camelCase(nBuckets / anglesDeg)', () => {
    const parsed = parseJsonMessage(metaText);
    if (parsed.kind !== 'meta') throw new Error('expected meta');
    expect(parsed.meta.nBuckets).toBe(512);
    expect(parsed.meta.planes).toEqual({ U: 72, V: 92, W: 92 });
    expect(parsed.meta.geometry).toBe('geometry_mini_reduced.dat');
    expect(parsed.meta.anglesDeg).toEqual([90, 30, 150]);
    expect(parsed.meta.detector).toBe('mini_eTPC');
    expect(parsed.meta.cobos).toEqual([0]);
    expect(parsed.meta.run).toBe(7);
  });

  it('anglesDeg は null になりうる(HeaderScalars に無いジオメトリ)', () => {
    const parsed = parseJsonMessage(JSON.stringify({ ...JSON.parse(metaText), anglesDeg: null }));
    if (parsed.kind !== 'meta') throw new Error('expected meta');
    expect(parsed.meta.anglesDeg).toBeNull();
  });
});

describe('SPEC §10.3 run', () => {
  it('state / run / ts を読む', () => {
    const parsed = parseJsonMessage(
      JSON.stringify({
        type: 'run',
        state: 'running',
        run: 8,
        ts: '2026-08-14T10:00:00.123+09:00',
      }),
    );
    if (parsed.kind !== 'run') throw new Error('expected run');
    expect(parsed.run.state).toBe('running');
    expect(parsed.run.run).toBe(8);
    expect(parsed.run.ts).toBe('2026-08-14T10:00:00.123+09:00');
  });
});

describe('未知 type / 壊れた JSON', () => {
  it('知らない type は落とさず数える(前方互換)', () => {
    const parsed = parseJsonMessage(JSON.stringify({ type: 'psu', volts: 1 }));
    expect(parsed.kind).toBe('unknown-type');
    if (parsed.kind !== 'unknown-type') return;
    expect(parsed.type).toBe('psu');
  });

  it('JSON でない / オブジェクトでない / type が無い → エラー値(例外は投げない)', () => {
    expect(parseJsonMessage('{not json').kind).toBe('error');
    expect(parseJsonMessage('[1,2,3]').kind).toBe('error');
    expect(parseJsonMessage('{"run":1}').kind).toBe('error');
  });
});

describe('SPEC §10.3 subscribe(C→S)', () => {
  it('既定は waveforms だけ OFF', () => {
    expect(DEFAULT_STREAMS).toEqual({ uvw: true, waveforms: false, histos: true, status: true });
  });

  it('ON のストリーム名だけを SPEC の順で並べる', () => {
    expect(encodeSubscribe(DEFAULT_STREAMS)).toBe(
      JSON.stringify({ streams: ['uvw', 'histos', 'status'] }),
    );
    expect(encodeSubscribe({ ...DEFAULT_STREAMS, waveforms: true })).toBe(
      JSON.stringify({ streams: ['uvw', 'waveforms', 'histos', 'status'] }),
    );
    // 全 OFF も表現できる(monitor 側は「配列に無いものは OFF」)。
    expect(encodeSubscribe({ uvw: false, waveforms: false, histos: false, status: false })).toBe(
      JSON.stringify({ streams: [] }),
    );
  });
});
