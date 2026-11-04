/**
 * TODO/187: controller の `GET /api/status` を読んで出す 4 つ
 * (STOPPING draining / ROOT DROPPED / OVERLOAD / ROOT writing)。
 * 値はすべて手で決めた非対称値(合計・取り違えが見える)。
 */
import { type ComponentFixture, TestBed } from '@angular/core/testing';
import { signal } from '@angular/core';

import { ControllerApiService } from '../api/controller-api';
import { ParamsApiService } from '../api/params-api';
import { WsClientService } from '../ws/ws-client';
import { StatusBar } from './status-bar';

function fakeWs(statusValue: unknown = null): Partial<WsClientService> {
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
    status: signal(statusValue) as unknown as WsClientService['status'],
    url: (() => 'ws://localhost:9000') as unknown as WsClientService['url'],
    reconnectNow: () => {},
  };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 20; i++) {
    await Promise.resolve();
  }
}

function component(
  name: string,
  state: string,
  metrics: Record<string, unknown> | null = null,
): Record<string, unknown> {
  return {
    name,
    endpoint: `tcp://127.0.0.1/${name}`,
    reachable: true,
    state,
    run_number: null,
    metrics,
    error: null,
  };
}

function controllerStatus(
  overrides: Record<string, unknown> = {},
  components: Record<string, unknown>[] = [],
): Record<string, unknown> {
  return {
    phase: 'Running',
    run: null,
    operator: null,
    components,
    ecc: { ok: true, state: 'Idle', error: '', ecc_error: 'NO_ERR' },
    log_posts: { accepted: 0, rejected: 0 },
    notes: [],
    ...overrides,
  };
}

describe('StatusBar — TODO/187 controller status chips', () => {
  let fixture: ComponentFixture<StatusBar>;

  async function build(
    controller: Record<string, unknown>,
    ws: unknown = null,
  ): Promise<HTMLElement> {
    TestBed.resetTestingModule();
    const params = { getParams: () => Promise.resolve(null) };
    const ctl = { getStatus: () => Promise.resolve(controller) };
    TestBed.configureTestingModule({
      providers: [
        { provide: ParamsApiService, useValue: params as unknown as ParamsApiService },
        { provide: ControllerApiService, useValue: ctl as unknown as ControllerApiService },
        { provide: WsClientService, useValue: fakeWs(ws) as WsClientService },
      ],
    });
    fixture = TestBed.createComponent(StatusBar);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  }

  afterEach(() => fixture?.destroy());

  it('status-bar shows STOPPING draining with the root_sink backlog while phase is Stopping', async () => {
    const root = await build(
      controllerStatus({
        phase: 'Stopping',
        stopping: {
          decoder_backlog: 33,
          root_sink_backlog: 4821,
          root_sink: { state: 'running', events_built: 15478, fragments: 61912 },
          waited_s: 125.7,
        },
      }),
    );
    const chip = root.querySelector('.stopping');
    expect(chip?.textContent).toContain('STOPPING — draining');
    expect(chip?.textContent).toContain('ROOT backlog 4,821');
    // waited は整数秒に丸める(125.7 → 126)。root_sink_backlog が取れている間は decoder の 33 は出さない。
    expect(chip?.textContent).toContain('waited 126 s');
    expect(chip?.textContent).not.toContain('33');
  });

  it('status-bar falls back to the decoder backlog, then to a bare draining, when root_sink_backlog is null', async () => {
    let root = await build(
      controllerStatus({
        phase: 'Stopping',
        stopping: { decoder_backlog: 33, root_sink_backlog: null, root_sink: null, waited_s: 4 },
      }),
    );
    expect(root.querySelector('.stopping')?.textContent).toContain('decoder backlog 33');
    expect(root.querySelector('.stopping')?.textContent).not.toContain('ROOT backlog');
    fixture.destroy();

    root = await build(
      controllerStatus({
        phase: 'Stopping',
        stopping: {
          decoder_backlog: null,
          root_sink_backlog: null,
          root_sink: null,
          waited_s: null,
        },
      }),
    );
    const text = root.querySelector('.stopping')?.textContent ?? '';
    expect(text).toContain('STOPPING — draining');
    expect(text).not.toMatch(/\d/);
    fixture.destroy();

    root = await build(controllerStatus({ phase: 'Running' }));
    expect(root.querySelector('.stopping')).toBeNull();
  });

  it('status-bar shows ROOT DROPPED with the summed decoder_link_dropped_frames', async () => {
    const root = await build(
      controllerStatus({}, [
        component('graw-writer', 'Running'),
        component('decoder', 'Running', { decoder_link_dropped_frames: 999 }),
        component('receiver0', 'Running', { decoder_link_dropped_frames: 6251 }),
        component('receiver1', 'Running', { decoder_link_dropped_frames: 17 }),
      ]),
    );
    const chip = root.querySelector('.root-dropped');
    // 6251 + 17 = 6268(receiver だけを足す。decoder の 999 は混ぜない)。
    expect(chip?.textContent).toContain('ROOT DROPPED 6,268');
    expect(chip?.classList.contains('bad')).toBe(true);
    fixture.destroy();

    const zero = await build(
      controllerStatus({}, [
        component('receiver0', 'Running', { decoder_link_dropped_frames: 0 }),
        component('receiver1', 'Running', {}),
      ]),
    );
    expect(zero.querySelector('.root-dropped')).toBeNull();
  });

  it('status-bar shows OVERLOAD when a receiver or graw-writer is in Error', async () => {
    let root = await build(
      controllerStatus({}, [
        component('graw-writer', 'Running'),
        component('receiver0', 'Running'),
        component('receiver1', 'Error'),
      ]),
    );
    expect(root.querySelector('.overload')?.textContent).toContain('OVERLOAD');
    fixture.destroy();

    root = await build(
      controllerStatus({}, [component('graw-writer', 'Error'), component('receiver0', 'Running')]),
    );
    expect(root.querySelector('.overload')?.textContent).toContain('OVERLOAD');
    fixture.destroy();

    // decoder の Error は OVERLOAD ではない(保存系の溢れではない)。
    root = await build(
      controllerStatus({}, [component('decoder', 'Error'), component('receiver0', 'Running')]),
    );
    expect(root.querySelector('.overload')).toBeNull();
  });

  it('status-bar shows ROOT writing when controller is Idle and root_sink still runs', async () => {
    let root = await build(controllerStatus({ phase: 'Idle' }), { state: 'running', run: 214 });
    expect(root.querySelector('.root-writing')?.textContent).toContain('ROOT writing');
    expect(root.querySelector('.root-writing')?.textContent).toContain('214');
    fixture.destroy();

    // 走行中(Running)の root_sink running は普通の姿。
    root = await build(controllerStatus({ phase: 'Running' }), { state: 'running', run: 214 });
    expect(root.querySelector('.root-writing')).toBeNull();
    fixture.destroy();

    root = await build(controllerStatus({ phase: 'Idle' }), { state: 'idle', run: 214 });
    expect(root.querySelector('.root-writing')).toBeNull();
  });
});
