/**
 * status バーの `params <id> · exceptions N`(SPEC v1.33 §3.3、TODO/078 §11「作るもの」4)。
 *
 * `GET /api/params` を 15 s 毎に poll する。404/503/失敗はどれも `—`(0 と混同しない —
 * CLAUDE.md「未知と 0 を混同させない」)。他の status バー項目(health/state/run 等)は
 * `WsClientService` を最小の fake で満たすだけにして触らない。
 */
import { type ComponentFixture, TestBed } from '@angular/core/testing';
import { signal } from '@angular/core';

import { ParamsApiService, ParamsClient, type ParamsInfo } from '../api/params-api';
import { WsClientService } from '../ws/ws-client';
import { StatusBar, STATUS_BAR_PARAMS_POLL_MS } from './status-bar';

const PARAMS_INFO: ParamsInfo = {
  id: 'mini-fixture__797d5dc2',
  sha256: '797d5dc2455',
  exceptions: 3,
  params: {
    schema: 'tpcdaq-params/1',
    template: 'configure-mini-fixture.xcfg',
    knobs: {},
    exceptions: [],
  },
  files: { configure: '/tmp/configure.xcfg', prepare: '/tmp/prepare.xcfg' },
};

/** 実物の WS シグナルの形だけ真似た最小 fake(health バッジ等は既定でも壊れない値にする)。 */
function fakeWs(): Partial<WsClientService> {
  return {
    health: signal('offline') as unknown as WsClientService['health'],
    link: signal('idle') as unknown as WsClientService['link'],
    counters: signal({
      meta: 0,
      status: 0,
      run: 0,
      uvw: 0,
      waveforms: 0,
      histo1d: 0,
      histo2d: 0,
      decodeErrors: 0,
      jsonErrors: 0,
      unknownJsonTypes: 0,
      reconnects: 0,
    }) as unknown as WsClientService['counters'],
    lastIssue: signal(null) as unknown as WsClientService['lastIssue'],
    status: signal(null) as unknown as WsClientService['status'],
    url: (() => 'ws://localhost:9000') as unknown as WsClientService['url'],
    reconnectNow: () => {},
  };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 20; i++) {
    await Promise.resolve();
  }
}

describe('StatusBar — params <id> · exceptions N', () => {
  let fixture: ComponentFixture<StatusBar>;

  function text(): string {
    return (fixture.nativeElement as HTMLElement).textContent ?? '';
  }

  function build(getParams: () => Promise<ParamsInfo | null>): void {
    const api = new ParamsClient(
      () => '/api',
      () => Promise.reject(new Error('unused: getParams is stubbed directly below')),
    );
    // getParams だけ差し替える(URL 組み立ての回帰確認は params-api.spec.ts の役目)。
    api.getParams = getParams;

    TestBed.configureTestingModule({
      providers: [
        { provide: ParamsApiService, useValue: api as unknown as ParamsApiService },
        { provide: WsClientService, useValue: fakeWs() as WsClientService },
      ],
    });
    fixture = TestBed.createComponent(StatusBar);
    fixture.detectChanges();
  }

  afterEach(() => fixture?.destroy());

  it('取れたら id と exceptions を出す', async () => {
    build(() => Promise.resolve(PARAMS_INFO));
    await flush();
    fixture.detectChanges();

    expect(text()).toContain('mini-fixture__797d5dc2');
    expect(text()).toContain('exceptions');
    expect(text()).toContain('3');
  });

  it('404(まだ無い)は — (0 と書かない)', async () => {
    build(() => Promise.resolve(null));
    await flush();
    fixture.detectChanges();

    expect(fixture.componentInstance.paramsId()).toBeNull();
    expect(fixture.componentInstance.paramsExceptions()).toBeNull();
    expect(text()).toContain('—');
  });

  it('503/失敗も — にする(黙って 0 にしない)', async () => {
    build(() => Promise.reject(new Error('ecc_config_dir is not configured')));
    await flush();
    fixture.detectChanges();

    expect(fixture.componentInstance.paramsId()).toBeNull();
    expect(fixture.componentInstance.paramsExceptions()).toBeNull();
  });

  it(`15 s(STATUS_BAR_PARAMS_POLL_MS)ごとに再取得する`, async () => {
    vi.useFakeTimers();
    try {
      let calls = 0;
      const results: (ParamsInfo | null)[] = [null, PARAMS_INFO];
      build(() => Promise.resolve(results[Math.min(calls++, results.length - 1)]));
      await flush();
      fixture.detectChanges();
      expect(fixture.componentInstance.paramsId()).toBeNull();

      await vi.advanceTimersByTimeAsync(STATUS_BAR_PARAMS_POLL_MS);
      await flush();
      fixture.detectChanges();

      expect(calls).toBeGreaterThanOrEqual(2);
      expect(fixture.componentInstance.paramsId()).toBe('mini-fixture__797d5dc2');
    } finally {
      vi.useRealTimers();
    }
  });
});
