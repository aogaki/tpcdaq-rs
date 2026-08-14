/**
 * `WsClientService` — 接続 / 購読 / 再接続 / カウンタ。
 *
 * 本物の `WebSocket` の代わりに fake ソケットを差し込むので、DOM も実サーバも
 * 要らない純ロジックのテストになる(実サーバ相手の確認はライブスモークの担当)。
 */
import { vi } from 'vitest';
import { DEFAULT_STREAMS } from './json';
import { RECONNECT_BASE_MS } from './state';
import { MSG_HISTO2D, MSG_UVW, WS_HEADER_LEN, WS_VERSION } from './wire';
import { WsClientService, type WsSocket } from './ws-client';

class FakeSocket implements WsSocket {
  binaryType = '';
  readonly sent: string[] = [];
  closed = false;
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;

  constructor(readonly url: string) {}

  send(data: string): void {
    this.sent.push(data);
  }

  close(): void {
    this.closed = true;
  }

  /** サーバ側の出来事を模す。 */
  open(): void {
    this.onopen?.();
  }
  deliver(data: unknown): void {
    this.onmessage?.({ data });
  }
  dropByServer(): void {
    this.closed = true;
    this.onclose?.();
  }
}

/** 差し込んだ fake ソケットを覚えておくファクトリ。 */
function harness() {
  const sockets: FakeSocket[] = [];
  const service = new WsClientService();
  service.socketFactory = (url) => {
    const socket = new FakeSocket(url);
    sockets.push(socket);
    return socket;
  };
  service.random = () => 1; // ジッタ上端に固定(決定的にする)
  return { service, sockets, last: () => sockets[sockets.length - 1] };
}

function statusText(extra: Record<string, unknown> = {}): string {
  return JSON.stringify({
    type: 'status',
    run: 7,
    state: 'running',
    events_built: 108,
    events_incomplete: 0,
    late_fragments: 0,
    pending_events: 0,
    frames_per_cobo: { '0': 3852 },
    bytes_written: 1234,
    saturation: {
      U: { saturated: 1, counted: 4 },
      V: { saturated: 0, counted: 0 },
      W: { saturated: 0, counted: 2 },
    },
    publish_drops: 0,
    monitorGaps: 0,
    clients: 1,
    wsDropped: 0,
    ...extra,
  });
}

const metaText = JSON.stringify({
  type: 'meta',
  nBuckets: 512,
  planes: { U: 72, V: 92, W: 92 },
  geometry: 'geometry_mini_reduced.dat',
  anglesDeg: null,
  detector: 'mini_eTPC',
  cobos: [0],
  run: 7,
});

/** 最小の Uvw(1 strip × 1 bucket)。 */
function uvwBuffer(plane: number): ArrayBuffer {
  const buffer = new ArrayBuffer(WS_HEADER_LEN + 5 + 2);
  const view = new DataView(buffer);
  view.setUint8(0, 0x54);
  view.setUint8(1, 0x50);
  view.setUint8(2, MSG_UVW);
  view.setUint8(3, WS_VERSION);
  view.setUint32(5, 7, true);
  view.setUint32(9, 42, true);
  view.setUint8(13, plane);
  view.setUint16(14, 1, true);
  view.setUint16(16, 1, true);
  view.setUint16(18, 777, true);
  return buffer;
}

/** 最小の Histo2d(1×1)。 */
function histo2dBuffer(id: number): ArrayBuffer {
  const buffer = new ArrayBuffer(WS_HEADER_LEN + 22 + 4);
  const view = new DataView(buffer);
  view.setUint8(0, 0x54);
  view.setUint8(1, 0x50);
  view.setUint8(2, MSG_HISTO2D);
  view.setUint8(3, WS_VERSION);
  view.setUint32(5, 7, true);
  view.setUint16(13, id, true);
  view.setUint16(15, 1, true);
  view.setUint16(17, 1, true);
  view.setFloat32(35, 5.5, true);
  return buffer;
}

describe('接続と購読', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('connect でソケットを開き、binaryType を arraybuffer にして subscribe を送る', () => {
    const h = harness();
    h.service.connect('ws://daq-pc:9000/ws');

    expect(h.service.link()).toBe('connecting');
    expect(h.last().url).toBe('ws://daq-pc:9000/ws');
    expect(h.last().binaryType).toBe('arraybuffer');

    h.last().open();
    expect(h.service.link()).toBe('connected');
    // 既定は waveforms OFF(SPEC §10.3)
    expect(h.last().sent).toEqual([JSON.stringify({ streams: ['uvw', 'histos', 'status'] })]);
    expect(h.service.streams()).toEqual(DEFAULT_STREAMS);

    h.service.disconnect();
  });

  it('setWaveforms で購読を切り替え、接続中なら即 subscribe を再送する', () => {
    const h = harness();
    h.service.connect('ws://x:9000/ws');
    h.last().open();

    h.service.setWaveforms(true);
    expect(h.service.streams().waveforms).toBe(true);
    expect(h.last().sent[1]).toBe(
      JSON.stringify({ streams: ['uvw', 'waveforms', 'histos', 'status'] }),
    );

    h.service.setWaveforms(false);
    expect(h.last().sent[2]).toBe(JSON.stringify({ streams: ['uvw', 'histos', 'status'] }));

    // 同じ値を二度指示しても再送しない(帯域と雑音を増やさない)。
    h.service.setWaveforms(false);
    expect(h.last().sent).toHaveLength(3);

    h.service.disconnect();
  });

  it('未接続で setWaveforms しても投げず、次の接続時にその購読で開く', () => {
    const h = harness();
    h.service.setWaveforms(true);
    h.service.connect('ws://x:9000/ws');
    h.last().open();
    expect(h.last().sent[0]).toBe(
      JSON.stringify({ streams: ['uvw', 'waveforms', 'histos', 'status'] }),
    );
    h.service.disconnect();
  });
});

describe('受信', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('meta / status / run を保持し、型別に数える', () => {
    const h = harness();
    h.service.connect('ws://x:9000/ws');
    h.last().open();

    h.last().deliver(metaText);
    h.last().deliver(statusText());
    h.last().deliver(JSON.stringify({ type: 'run', state: 'stopped', run: 7, ts: 'now' }));

    expect(h.service.meta()?.detector).toBe('mini_eTPC');
    expect(h.service.status()?.events_built).toBe(108);
    expect(h.service.run()?.state).toBe('stopped');
    const counters = h.service.counters();
    expect(counters.meta).toBe(1);
    expect(counters.status).toBe(1);
    expect(counters.run).toBe(1);

    h.service.disconnect();
  });

  it('Uvw は面別、ヒストは id 別に最新を保つ', () => {
    const h = harness();
    h.service.connect('ws://x:9000/ws');
    h.last().open();

    h.last().deliver(uvwBuffer(1)); // V 面
    h.last().deliver(histo2dBuffer(3)); // StripTimeW

    expect(h.service.uvwByPlane()[0]).toBeNull();
    expect(h.service.uvwByPlane()[1]?.adc[0]).toBe(777);
    expect(h.service.histos().get(3)?.kind).toBe('histo2d');
    expect(h.service.counters().uvw).toBe(1);
    expect(h.service.counters().histo2d).toBe(1);

    h.service.disconnect();
  });

  it('壊れたフレーム・未知 type・未知キーは落として数える(silent 禁止)', () => {
    const h = harness();
    h.service.connect('ws://x:9000/ws');
    h.last().open();

    h.last().deliver(new ArrayBuffer(4)); // 13 B 未満
    h.last().deliver('{not json');
    h.last().deliver(JSON.stringify({ type: 'psu', volts: 1 }));
    h.last().deliver(statusText({ future_key: 1 }));

    const counters = h.service.counters();
    expect(counters.decodeErrors).toBe(1);
    expect(counters.jsonErrors).toBe(1);
    expect(counters.unknownJsonTypes).toBe(1);
    expect(counters.unknownJsonKeys).toBe(1);
    expect(h.service.lastIssue()).not.toBeNull();
    // 数えたうえで、まともなフィールドは生きている。
    expect(h.service.status()?.events_built).toBe(108);

    h.service.disconnect();
  });
});

describe('staleness', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('未接続 → offline、接続直後 → waiting、status 到着 → fresh、3 秒途絶 → stale', () => {
    const h = harness();
    expect(h.service.health()).toBe('offline');

    h.service.connect('ws://x:9000/ws');
    h.last().open();
    expect(h.service.health()).toBe('waiting');

    h.last().deliver(statusText());
    expect(h.service.health()).toBe('fresh');

    vi.advanceTimersByTime(3000);
    expect(h.service.health()).toBe('stale');

    // 再開すれば戻る。
    h.last().deliver(statusText());
    expect(h.service.health()).toBe('fresh');

    h.service.disconnect();
    expect(h.service.health()).toBe('offline');
  });

  it('status が一度も来ないまま 3 秒経てば stale(monitor 単体起動 = root-sink 無し)', () => {
    const h = harness();
    h.service.connect('ws://x:9000/ws');
    h.last().open();
    h.last().deliver(metaText); // meta は届く

    vi.advanceTimersByTime(3000);
    expect(h.service.link()).toBe('connected'); // 繋がってはいる
    expect(h.service.health()).toBe('stale'); // が status が来ない

    h.service.disconnect();
  });
});

describe('自動再接続', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('サーバに切られたらバックオフして繋ぎ直し、回数を数える', () => {
    const h = harness();
    h.service.connect('ws://x:9000/ws');
    h.last().open();
    expect(h.sockets).toHaveLength(1);

    h.last().dropByServer();
    expect(h.service.link()).toBe('disconnected');

    vi.advanceTimersByTime(RECONNECT_BASE_MS - 1);
    expect(h.sockets).toHaveLength(1); // まだ待つ
    vi.advanceTimersByTime(1);
    expect(h.sockets).toHaveLength(2); // 1 回目 = 500 ms
    expect(h.service.counters().reconnects).toBe(1);

    // 2 回目は 1000 ms(指数バックオフ)
    h.last().dropByServer();
    vi.advanceTimersByTime(999);
    expect(h.sockets).toHaveLength(2);
    vi.advanceTimersByTime(1);
    expect(h.sockets).toHaveLength(3);

    // 繋がったらバックオフはリセット
    h.last().open();
    h.last().dropByServer();
    vi.advanceTimersByTime(RECONNECT_BASE_MS);
    expect(h.sockets).toHaveLength(4);

    h.service.disconnect();
  });

  it('disconnect したら再接続しない(手動停止を尊重する)', () => {
    const h = harness();
    h.service.connect('ws://x:9000/ws');
    h.last().open();
    h.service.disconnect();

    vi.advanceTimersByTime(60_000);
    expect(h.sockets).toHaveLength(1);
    expect(h.service.link()).toBe('disconnected');
  });

  it('reconnectNow は待たずに繋ぎ直す', () => {
    const h = harness();
    h.service.connect('ws://x:9000/ws');
    h.last().open();

    h.service.reconnectNow();
    expect(h.sockets).toHaveLength(2);
    expect(h.sockets[0].closed).toBe(true);

    h.service.disconnect();
  });
});
