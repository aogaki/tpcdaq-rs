/**
 * Run 制御画面の配線(050-B / 050-C / 050-D)。
 *
 * ここは**画面から実際に POST が出るところまで**を見る唯一のテスト:
 * ボタンを押す → 破壊的なら確認ダイアログ → 送る → 応答を出す。
 * `fetch` だけを差し替え、`RunControlClient` は本物を使う(請求形も一緒に確かめられる)。
 */
import { type ComponentFixture, TestBed } from '@angular/core/testing';

import { ControllerApiService } from '../api/controller-api';
import { RunControlApiService, RunControlClient } from '../api/run-control-api';
import { RunView } from './run-view';

interface Call {
  url: string;
  body: unknown;
}

const TOKEN = 'tok-abcdef0123456789';

/** `GET /api/status` の応答(controller の実際の形。run は既定で null)。 */
function statusJson(
  run: Record<string, unknown> | null,
  ecc: Record<string, unknown> = { ok: true, state: 'Idle', error: '', ecc_error: 'NO_ERR' },
): Record<string, unknown> {
  return {
    phase: run === null ? 'Idle' : 'Running',
    run,
    operator: run === null ? null : 'aogaki',
    components: [
      {
        name: 'receiver-cobo0',
        endpoint: 'tcp://127.0.0.1:5561',
        reachable: true,
        state: 'Idle',
        run_number: null,
        error: null,
      },
    ],
    ecc,
    log_posts: { accepted: 3, rejected: 0 },
    notes: [],
  };
}

const ACTIVE_RUN = {
  run: 12,
  operator: 'aogaki',
  comment: 'demo',
  started_ts: '2026-08-15T18:00:00.000+02:00',
  elapsed_s: 4.5,
};

describe('RunView — 押す → 送る → 出す', () => {
  let fixture: ComponentFixture<RunView>;
  let calls: Call[];
  let replies: { status: number; payload: unknown }[];
  let status: Record<string, unknown>;

  /** 表示テキストの前方一致でボタンを引く(送信中は文言が変わるので trim して見る)。 */
  function button(label: string): HTMLButtonElement {
    const found = [...fixture.nativeElement.querySelectorAll('button')].find(
      (element) => (element as HTMLButtonElement).textContent?.trim() === label,
    );
    if (!found) throw new Error(`ボタンが無い: ${label}`);
    return found as HTMLButtonElement;
  }

  function text(): string {
    return (fixture.nativeElement as HTMLElement).textContent ?? '';
  }

  async function click(label: string): Promise<void> {
    button(label).click();
    await fixture.whenStable();
    fixture.detectChanges();
  }

  beforeEach(async () => {
    calls = [];
    replies = [];
    status = statusJson(null);

    // jsdom に showModal/close が無い版でも動くようにする(表示の有無は pending() で見る)。
    const dialog = HTMLDialogElement.prototype as unknown as Record<string, unknown>;
    if (typeof dialog['showModal'] !== 'function') dialog['showModal'] = function noop() {};
    if (typeof dialog['close'] !== 'function') dialog['close'] = function noop() {};

    const fetchFn = (url: string, init?: RequestInit): Promise<Response> => {
      calls.push({ url, body: JSON.parse(String(init?.body ?? 'null')) });
      const reply = replies[calls.length - 1] ?? { status: 200, payload: { ok: true } };
      return Promise.resolve({
        status: reply.status,
        json: () => Promise.resolve(reply.payload),
      } as unknown as Response);
    };

    const control = new RunControlClient(() => '/api', fetchFn, null);
    const viewApi = { getStatus: () => Promise.resolve(status) };

    TestBed.configureTestingModule({
      providers: [
        { provide: ControllerApiService, useValue: viewApi as unknown as ControllerApiService },
        { provide: RunControlApiService, useValue: control as RunControlApiService },
      ],
    });

    fixture = TestBed.createComponent(RunView);
    fixture.detectChanges();
    await fixture.whenStable();
    fixture.detectChanges();
  });

  afterEach(() => fixture.destroy());

  it('token が無い間は Acquire だけ押せる', () => {
    expect(button('Acquire').disabled).toBe(false);
    expect(button('Release').disabled).toBe(true);
    expect(button('Start run').disabled).toBe(true);
    expect(button('ECC describe').disabled).toBe(true);
    expect(text()).toContain('操作権がありません');
  });

  it('Acquire すると token を持ち、他のボタンが押せるようになる', async () => {
    replies = [{ status: 200, payload: { token: TOKEN, preempted: null } }];

    await click('Acquire');

    expect(calls[0].url).toBe('/api/control/acquire');
    expect(calls[0].body).toEqual({ operator: '', passphrase: '' });
    expect(button('Start run').disabled).toBe(false);
    expect(button('Stop run').disabled).toBe(false);
    expect(text()).toContain('取得済み');
  });

  it('run 実行中は Start run が disabled、Stop run は有効', async () => {
    replies = [{ status: 200, payload: { token: TOKEN, preempted: null } }];
    status = statusJson(ACTIVE_RUN);

    await click('Acquire');

    expect(button('Start run').disabled).toBe(true);
    expect(button('Set next run').disabled).toBe(true);
    expect(button('Stop run').disabled).toBe(false);
    expect(button('ECC reset').disabled).toBe(false);
  });

  it('破壊的操作は確認ダイアログを挟む(押しただけでは 1 通も送らない)', async () => {
    replies = [{ status: 200, payload: { token: TOKEN, preempted: null } }];
    await click('Acquire');
    calls = [];
    replies = [{ status: 200, payload: { run: 12, ok: true, reason: 'normal', notes: [] } }];

    await click('Stop run');

    expect(calls).toHaveLength(0);
    expect(fixture.componentInstance.pending()?.endpoint).toBe('/api/run/stop');
    expect(text()).toContain('Stop run を実行しますか?');

    await click('実行する');

    expect(calls).toHaveLength(1);
    expect(calls[0].url).toBe('/api/run/stop');
    expect(calls[0].body).toEqual({ token: TOKEN });
    expect(fixture.componentInstance.pending()).toBeNull();
  });

  it('確認ダイアログで「やめる」を選べば送らない', async () => {
    replies = [{ status: 200, payload: { token: TOKEN, preempted: null } }];
    await click('Acquire');
    calls = [];

    await click('ECC reset');
    await click('やめる');

    expect(calls).toHaveLength(0);
    expect(fixture.componentInstance.pending()).toBeNull();
  });

  it('非破壊の操作は確認なしで送る', async () => {
    replies = [{ status: 200, payload: { token: TOKEN, preempted: null } }];
    await click('Acquire');
    calls = [];
    replies = [{ status: 200, payload: { ok: true, state: 'Described', error: '' } }];

    await click('ECC describe');

    expect(calls).toHaveLength(1);
    expect(calls[0].url).toBe('/api/ecc/describe');
  });

  it('失敗は controller の error 文字列をそのまま画面に出す', async () => {
    replies = [{ status: 403, payload: { error: 'wrong passphrase' } }];

    await click('Acquire');

    expect(text()).toContain('wrong passphrase');
    expect(text()).toContain('HTTP 403');
    expect(button('Start run').disabled).toBe(true); // token は取れていない
  });

  it('run/stop の notes を全部出す(HTTP 200 + ok:false も失敗として出す)', async () => {
    replies = [{ status: 200, payload: { token: TOKEN, preempted: null } }];
    await click('Acquire');
    calls = [];
    replies = [
      {
        status: 200,
        payload: {
          run: 13,
          ok: false,
          reason: 'error:eos-timeout',
          forced_eos: true,
          notes: ['decoder: EndOfStream を 5 s 待って諦めた', 'graw-writer: 4 files'],
        },
      },
    ];

    await click('Stop run');
    await click('実行する');

    expect(text()).toContain('decoder: EndOfStream を 5 s 待って諦めた');
    expect(text()).toContain('graw-writer: 4 files');
    expect(text()).toContain('error:eos-timeout');
    expect(fixture.nativeElement.querySelector('.banner.ok')).toBeNull();
  });

  it('next_run が正整数でなければ送らずに理由を出す', async () => {
    replies = [{ status: 200, payload: { token: TOKEN, preempted: null } }];
    await click('Acquire');
    calls = [];
    fixture.componentInstance.nextRun.set('0');
    fixture.detectChanges();

    await click('Set next run');

    expect(calls).toHaveLength(0);
    expect(text()).toContain('next_run は正の整数だけです');
  });

  it('送信中は全ボタンが disabled(run/start は ≈7 s かかる — 041 実測)', async () => {
    replies = [{ status: 200, payload: { token: TOKEN, preempted: null } }];
    await click('Acquire');

    // 応答を保留したままにして「送信中」を作る(run/start の 7 s の代役)。
    let answer: (result: unknown) => void = () => {};
    const held = new Promise<unknown>((resolve) => {
      answer = resolve;
    });
    const control = TestBed.inject(RunControlApiService) as unknown as {
      execute: (...args: unknown[]) => Promise<unknown>;
    };
    const original = control.execute;
    control.execute = () => held;

    button('Start run').click();
    fixture.detectChanges();

    expect(button('送信中…').disabled).toBe(true);
    expect(button('Stop run').disabled).toBe(true);
    expect(button('ECC describe').disabled).toBe(true);

    answer({ ok: true, status: 200, error: null, notes: [], summary: '{"run":14}', body: null });
    await fixture.whenStable();
    control.execute = original;
    fixture.detectChanges();

    expect(button('Start run').disabled).toBe(false);
    expect(text()).toContain('{"run":14}');
  });
});

describe('RunView — ecc_error / Off 注記(SPEC §8.2 v1.14、TODO/051)', () => {
  let fixture: ComponentFixture<RunView>;
  let status: Record<string, unknown>;

  function text(): string {
    return (fixture.nativeElement as HTMLElement).textContent ?? '';
  }

  /** ECC の値バッジ(唯一 `tone-*` クラスを持つ `.value`)。 */
  function eccBadge(): HTMLElement {
    const found = fixture.nativeElement.querySelector('[class*="tone-"]');
    if (!found) throw new Error('ECC バッジが無い');
    return found as HTMLElement;
  }

  async function refresh(): Promise<void> {
    await fixture.componentInstance.refresh();
    fixture.detectChanges();
  }

  beforeEach(async () => {
    status = statusJson(null);
    const viewApi = { getStatus: () => Promise.resolve(status) };
    TestBed.configureTestingModule({
      providers: [
        { provide: ControllerApiService, useValue: viewApi as unknown as ControllerApiService },
        {
          provide: RunControlApiService,
          useValue: new RunControlClient(
            () => '/api',
            () => Promise.reject('unused'),
            null,
          ),
        },
      ],
    });
    fixture = TestBed.createComponent(RunView);
    fixture.detectChanges();
    await fixture.whenStable();
    fixture.detectChanges();
  });

  afterEach(() => fixture.destroy());

  it('値あり(NO_ERR 以外)は "error: <flag>" を出す', async () => {
    status = statusJson(null, { ok: true, state: 'Idle', error: '', ecc_error: 'WHEN_DESCRIBE' });
    await refresh();
    expect(text()).toContain('error: WHEN_DESCRIBE');
  });

  it('なし(NO_ERR)は既定の状態(何も足さない)', () => {
    // beforeEach の既定応答がまさに ecc_error: 'NO_ERR'。
    expect(text()).not.toContain('error: NO_ERR');
    expect(text()).not.toMatch(/error: \S/);
  });

  it('Unknown(ecc-bridge 不達)は既存の不達表示と整合させる("不明" のまま、二重表示しない)', async () => {
    status = statusJson(null, {
      ok: false,
      state: 'Unknown',
      error: 'no reply',
      ecc_error: 'Unknown',
    });
    await refresh();
    expect(eccBadge().textContent).toContain('不明');
    // controller が不達時に注入する ecc_error:"Unknown" は GET の実フラグではない
    // (不達センチネル)。「不明」表示と別に "error: Unknown" を重ねて出さない。
    expect(text()).not.toContain('error: Unknown');
  });

  it('Off state はツールチップで halt 注記を出す(表示テキスト自体は変えない)', async () => {
    status = statusJson(null, { ok: true, state: 'Off', error: '', ecc_error: 'NO_ERR' });
    await refresh();
    const badge = eccBadge();
    expect(badge.textContent?.trim()).toBe('Off');
    expect(badge.title).toBe(
      'ECC 停止または halt — 実機では getEccServer の再起動が必要な場合あり',
    );
  });

  it('Off 以外は halt 注記のツールチップを出さない', () => {
    // beforeEach の既定応答は state: 'Idle'。
    expect(eccBadge().title).toBe('');
  });
});
