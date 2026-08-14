/**
 * staleness 判定と再接続バックオフ(TODO/027 の確定設計)。
 *
 * 026 申し送り: monitor は独自タイマを持たないので、root-sink が止まると
 * `status` の途絶として現れる。「WS が繋がっていない」と「繋がっているが status が
 * 来ない」は原因が別なので、画面でも別物として出す。
 */
import {
  RECONNECT_BASE_MS,
  RECONNECT_MAX_MS,
  STATUS_STALE_MS,
  reconnectDelayMs,
  statusHealth,
} from './state';

describe('staleness(status が 3 秒途絶 = stale)', () => {
  it('しきい値は 3 秒', () => {
    expect(STATUS_STALE_MS).toBe(3000);
  });

  it('WS 未接続は offline(stale ではない — 原因が違う)', () => {
    const base = { connectedAtMs: null, lastStatusAtMs: null, nowMs: 10_000 };
    expect(statusHealth({ ...base, link: 'disconnected' })).toBe('offline');
    expect(statusHealth({ ...base, link: 'connecting' })).toBe('offline');
  });

  it('接続直後で status 未着なら waiting、3 秒経てば stale', () => {
    // 接続 t=1000。status は一度も来ていない。
    const base = { link: 'connected' as const, connectedAtMs: 1000, lastStatusAtMs: null };
    expect(statusHealth({ ...base, nowMs: 1000 + 2999 })).toBe('waiting');
    expect(statusHealth({ ...base, nowMs: 1000 + 3000 })).toBe('stale');
  });

  it('status が来ていれば最後の 1 通からの経過で判定する', () => {
    const base = { link: 'connected' as const, connectedAtMs: 1000, lastStatusAtMs: 5000 };
    expect(statusHealth({ ...base, nowMs: 5000 })).toBe('fresh');
    expect(statusHealth({ ...base, nowMs: 5000 + 2999 })).toBe('fresh');
    expect(statusHealth({ ...base, nowMs: 5000 + 3000 })).toBe('stale');
  });
});

describe('再接続バックオフ(指数 + ジッタ、上限あり)', () => {
  it('基準は倍々で伸び、上限で頭打ち', () => {
    // random=1 でジッタ上端 = 基準そのもの。500 → 1000 → 2000 → 4000 → 8000 → 8000
    const full = (attempt: number) => reconnectDelayMs(attempt, () => 1);
    expect(full(1)).toBe(RECONNECT_BASE_MS);
    expect(full(2)).toBe(1000);
    expect(full(3)).toBe(2000);
    expect(full(4)).toBe(4000);
    expect(full(5)).toBe(RECONNECT_MAX_MS);
    expect(full(20)).toBe(RECONNECT_MAX_MS);
    expect(RECONNECT_MAX_MS).toBe(8000); // 上限 5–10 s の範囲内
  });

  it('ジッタは基準の 50–100 %(同時再接続の同期を崩す)', () => {
    expect(reconnectDelayMs(3, () => 0)).toBe(1000); // 2000 の 50 %
    expect(reconnectDelayMs(3, () => 0.5)).toBe(1500);
    expect(reconnectDelayMs(3, () => 1)).toBe(2000);
  });

  it('attempt が 0 や負でも下限を割らない', () => {
    expect(reconnectDelayMs(0, () => 1)).toBe(RECONNECT_BASE_MS);
    expect(reconnectDelayMs(-3, () => 1)).toBe(RECONNECT_BASE_MS);
  });
});
