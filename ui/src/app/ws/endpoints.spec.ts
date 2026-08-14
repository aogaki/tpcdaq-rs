/**
 * 接続先の決定規則(TODO/027 の確定設計 + SPEC §3.2)。
 *
 * controller REST = 8080 / monitor WS = 9000 は**別プロセス・別ポート**。本番では
 * controller が UI を配信するので、UI から見て REST は同一オリジン・WS は別ポート。
 */
import { vi } from 'vitest';
import {
  UI_CONFIG_URL,
  WS_PATH,
  MONITOR_WS_PORT,
  defaultEndpoints,
  loadEndpoints,
} from './endpoints';

/** `fetch` の最小形(テストから差し替えるのはこれだけ)。 */
function fetcher(status: number, body: string) {
  return vi.fn(async () => ({ ok: status >= 200 && status < 300, status, text: async () => body }));
}

describe('既定の接続先', () => {
  it('WS は同一ホストの :9000/ws、REST は same-origin の /api', () => {
    expect(MONITOR_WS_PORT).toBe(9000); // SPEC §3.2
    expect(WS_PATH).toBe('/ws'); // 026 申し送り: パスは /ws 固定
    expect(defaultEndpoints({ protocol: 'http:', hostname: 'daq-pc' })).toEqual({
      wsUrl: 'ws://daq-pc:9000/ws',
      apiBase: '/api',
    });
  });

  it('ページが https なら wss', () => {
    expect(defaultEndpoints({ protocol: 'https:', hostname: 'daq-pc' }).wsUrl).toBe(
      'wss://daq-pc:9000/ws',
    );
  });
});

describe('ui-config.json による上書き', () => {
  const location = { protocol: 'http:', hostname: 'daq-pc' };

  it('両方指定すれば両方使う', async () => {
    const fetchFn = fetcher(
      200,
      JSON.stringify({ wsUrl: 'ws://other:9100/ws', apiBase: 'http://other:8080/api' }),
    );
    const resolved = await loadEndpoints(location, fetchFn);
    expect(fetchFn).toHaveBeenCalledWith(UI_CONFIG_URL);
    expect(resolved.endpoints).toEqual({
      wsUrl: 'ws://other:9100/ws',
      apiBase: 'http://other:8080/api',
    });
    expect(resolved.source).toBe('ui-config.json');
  });

  it('片方だけなら残りは既定のまま', async () => {
    const resolved = await loadEndpoints(location, fetcher(200, '{"wsUrl":"ws://other:9100/ws"}'));
    expect(resolved.endpoints).toEqual({ wsUrl: 'ws://other:9100/ws', apiBase: '/api' });
  });

  it('404 は既定へフォールバックし console.info を 1 回出す(silent 禁止)', async () => {
    const info = vi.spyOn(console, 'info').mockImplementation(() => undefined);
    const resolved = await loadEndpoints(location, fetcher(404, 'Not Found'));
    expect(resolved.endpoints).toEqual({ wsUrl: 'ws://daq-pc:9000/ws', apiBase: '/api' });
    expect(resolved.source).toBe('defaults');
    expect(info).toHaveBeenCalledTimes(1);
    info.mockRestore();
  });

  it('パース失敗も既定へフォールバックし console.info を出す', async () => {
    const info = vi.spyOn(console, 'info').mockImplementation(() => undefined);
    const resolved = await loadEndpoints(location, fetcher(200, '{ broken'));
    expect(resolved.endpoints.wsUrl).toBe('ws://daq-pc:9000/ws');
    expect(resolved.source).toBe('defaults');
    expect(info).toHaveBeenCalledTimes(1);
    info.mockRestore();
  });

  it('fetch 自体が失敗しても投げずに既定へ落ちる', async () => {
    const info = vi.spyOn(console, 'info').mockImplementation(() => undefined);
    const boom = vi.fn(async () => {
      throw new Error('network down');
    });
    const resolved = await loadEndpoints(location, boom);
    expect(resolved.endpoints.apiBase).toBe('/api');
    expect(info).toHaveBeenCalledTimes(1);
    info.mockRestore();
  });

  it('値が文字列でなければ無視して既定を使う(型は黙って信じない)', async () => {
    const info = vi.spyOn(console, 'info').mockImplementation(() => undefined);
    const resolved = await loadEndpoints(location, fetcher(200, '{"wsUrl":9000,"apiBase":null}'));
    expect(resolved.endpoints).toEqual({ wsUrl: 'ws://daq-pc:9000/ws', apiBase: '/api' });
    expect(info).toHaveBeenCalledTimes(1);
    info.mockRestore();
  });
});
