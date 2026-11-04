/**
 * TODO/187 ③: Stop は分単位になりうる(204)。応答でなく `/api/status` の phase を見て
 * Stopping を出し、Idle に戻ったら最後の run_stop(ログブック)の ok / reason を出す。
 */
import { type ComponentFixture, TestBed } from '@angular/core/testing';

import { ControllerApiService } from '../api/controller-api';
import { RunControlApiService, RunControlClient } from '../api/run-control-api';
import { RunView } from './run-view';

const TOKEN = 'tok-abcdef0123456789';

const ACTIVE_RUN = {
  run: 214,
  operator: 'aogaki',
  comment: 'demo',
  started_ts: '2026-11-04T15:00:00.000+01:00',
  elapsed_s: 300,
};

function statusJson(
  phase: string,
  run: Record<string, unknown> | null,
  stopping?: Record<string, unknown>,
): Record<string, unknown> {
  return {
    phase,
    run,
    operator: run === null ? null : 'aogaki',
    components: [],
    ecc: { ok: true, state: 'Idle', error: '', ecc_error: 'NO_ERR' },
    log_posts: { accepted: 0, rejected: 0 },
    notes: [],
    ...(stopping ? { stopping } : {}),
  };
}

/** 手計算の値: 残り 4,821 / decoder 33 / 待ち 125.7 s(整数秒に丸めて 126)。 */
const STOPPING = {
  decoder_backlog: 33,
  root_sink_backlog: 4821,
  root_sink: { state: 'running', events_built: 15478, fragments: 61912 },
  waited_s: 125.7,
};

const RUN_STOP_OK = {
  ts: '2026-11-04T15:12:00.000+01:00',
  seq: 91,
  type: 'run_stop',
  actor: 'controller',
  run: 214,
  duration_s: 700,
  ok: true,
  reason: 'normal',
  counters: { root_complete: false },
  files: [],
};

async function flush(): Promise<void> {
  for (let i = 0; i < 30; i++) {
    await Promise.resolve();
  }
}

describe('RunView — Stop は応答でなく phase を見る(TODO/187 ③)', () => {
  let fixture: ComponentFixture<RunView>;
  let status: Record<string, unknown>;
  let logbook: unknown[];
  let stopFetch: (url: string) => Promise<Response>;

  function text(): string {
    return (fixture.nativeElement as HTMLElement).textContent ?? '';
  }

  function button(label: string): HTMLButtonElement {
    const found = [...fixture.nativeElement.querySelectorAll('button')].find(
      (element) => (element as HTMLButtonElement).textContent?.trim() === label,
    );
    if (!found) throw new Error(`ボタンが無い: ${label}`);
    return found as HTMLButtonElement;
  }

  /** Stop run → Execute。応答は待たない(待つ前提のテストは分単位の応答を作れない)。 */
  async function pressStop(): Promise<void> {
    button('Stop run').click();
    await flush();
    fixture.detectChanges();
    button('Execute').click();
    await flush();
    fixture.detectChanges();
  }

  beforeEach(async () => {
    const dialog = HTMLDialogElement.prototype as unknown as Record<string, unknown>;
    if (typeof dialog['showModal'] !== 'function') dialog['showModal'] = function noop() {};
    if (typeof dialog['close'] !== 'function') dialog['close'] = function noop() {};

    status = statusJson('Running', ACTIVE_RUN);
    logbook = [];
    stopFetch = () => new Promise<Response>(() => {});

    const control = new RunControlClient(
      () => '/api',
      (url) => stopFetch(url),
      null,
    );
    control.setToken(TOKEN);
    const viewApi = {
      getStatus: () => Promise.resolve(status),
      getLogbook: () => Promise.resolve({ records: logbook, tail_corrupt: false }),
    };
    TestBed.configureTestingModule({
      providers: [
        { provide: ControllerApiService, useValue: viewApi as unknown as ControllerApiService },
        { provide: RunControlApiService, useValue: control as RunControlApiService },
      ],
    });
    fixture = TestBed.createComponent(RunView);
    fixture.detectChanges();
    await flush();
    fixture.detectChanges();
  });

  afterEach(() => fixture.destroy());

  it('run view keeps showing Stopping from /api/status even when the stop request has not answered yet', async () => {
    // 応答が永遠に返らない stop(分単位の応答のモック)。
    status = statusJson('Stopping', ACTIVE_RUN, STOPPING);
    await pressStop();

    const chip = (fixture.nativeElement as HTMLElement).querySelector('.stopping');
    expect(chip?.textContent).toContain('STOPPING — draining');
    expect(chip?.textContent).toContain('ROOT backlog 4,821');
    expect(chip?.textContent).toContain('waited 126 s');

    // 応答がまだ無いまま phase が Idle に戻った → ログブックの最後の run_stop を出す。
    status = statusJson('Idle', null);
    logbook = [{ ...RUN_STOP_OK, seq: 90, run: 213, ok: false, reason: 'error:old' }, RUN_STOP_OK];
    await fixture.componentInstance.refresh();
    await flush();
    fixture.detectChanges();

    expect((fixture.nativeElement as HTMLElement).querySelector('.stopping')).toBeNull();
    // 最後(seq 最大)の run_stop = run 214, ok, normal。古い run 213 を拾わない。
    expect(text()).toContain('run_stop: run 214 ok');
    expect(text()).toContain('reason normal');
    expect(text()).not.toContain('error:old');
  });

  it('run view shows a failed run_stop (ok false, reason) once phase returns to Idle', async () => {
    status = statusJson('Stopping', ACTIVE_RUN, STOPPING);
    await pressStop();

    status = statusJson('Idle', null);
    logbook = [{ ...RUN_STOP_OK, ok: false, reason: 'error:eos-timeout' }];
    await fixture.componentInstance.refresh();
    await flush();
    fixture.detectChanges();

    expect(text()).toContain('run_stop: run 214 FAILED');
    expect(text()).toContain('reason error:eos-timeout');
  });

  it('run view does not show an error when the stop request fails on the wire while phase is Stopping', async () => {
    stopFetch = () => Promise.reject(new TypeError('Failed to fetch (proxy timeout)'));
    status = statusJson('Stopping', ACTIVE_RUN, STOPPING);
    await pressStop();

    const root = fixture.nativeElement as HTMLElement;
    expect(text()).not.toContain('did not get through');
    expect(text()).not.toContain('Failed to fetch');
    expect(root.querySelector('.banner.error')).toBeNull();
    expect(root.querySelector('.stopping')?.textContent).toContain('STOPPING — draining');
  });

  it('run view still shows a wire failure when the controller is not Stopping', async () => {
    stopFetch = () => Promise.reject(new TypeError('Failed to fetch'));
    // phase は Running のまま = 本当に届いていない。
    await pressStop();

    expect(text()).toContain('did not get through');
  });
});
