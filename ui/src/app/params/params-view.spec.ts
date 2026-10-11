/**
 * Parameters ビューの配線(078 §11)。
 *
 * `fetch` だけを差し替えて `ParamsClient` / `RunControlClient` は本物を使う(`run-view.spec.ts`
 * と同じ流儀 —— 請求形も一緒に確かめられる)。WS の `status()` は `WsClientService` を
 * 部分差し替えして与える(078 §11: Running 判定は monitor WS の `state`)。
 */
import { type ComponentFixture, TestBed } from '@angular/core/testing';
import { signal } from '@angular/core';
import { readFileSync } from 'node:fs';

import { ControllerApiService } from '../api/controller-api';
import { RunControlApiService, RunControlClient } from '../api/run-control-api';
import { ParamsApiService, ParamsClient } from '../api/params-api';
import { WsClientService } from '../ws/ws-client';
import { ParamsView } from './params-view';

// リポ直下の合成フィクスチャ(`knob-spec.spec.ts` と同じファイル)。
const FIXTURE_PATH = '../tests/fixtures/params/params_roundtrip.json';

const TOKEN = 'tok-abcdef0123456789';

const SAVED_PARAMS = {
  schema: 'tpcdaq-params/1',
  template: 'configure-mini-fixture.xcfg',
  knobs: {
    triggerDelay: 1748,
    writingClockFrequency_MHz: 25.0,
    triggerMode: 'external',
    globalThreshold: 1,
    lsbThreshold: 15,
    gain: '120fC',
    triggerInhibition: 'none',
    peakingTime_ns: 232,
    readout: 'full',
    fpnRead: true,
    zeroSuppression: { enabled: false, threshold: 600 },
    pulser: { enabled: false, voltage_mV: -1000.0, period_ms: 1000 },
  },
  exceptions: [] as unknown[],
  note: 'template_mini.xcfg の現行値そのまま',
};

const SAVED_INFO = {
  id: 'mini-fixture__797d5dc2',
  sha256: '797d5dc245553d3aaae0c7172993e6a311de94e7d48f211855ecc6c762112ac5',
  exceptions: 0,
  params: SAVED_PARAMS,
  files: {
    configure: '/tmp/ecc/configure-mini-fixture__797d5dc2.xcfg',
    prepare: '/tmp/ecc/prepare-mini-fixture__797d5dc2.xcfg',
  },
};

/** elinp 相当のテンプレートから抽出した応答(111 §5.0 の例。mini = 100 MHz / triggerDelay 320)。 */
const ELINP_DEFAULTS = {
  template: 'configure-elinp.xcfg',
  knobs: {
    triggerDelay: 320,
    writingClockFrequency_MHz: 100.0,
    triggerMode: 'external',
    globalThreshold: 1,
    lsbThreshold: 15,
    gain: '120fC',
    triggerInhibition: 'none',
    peakingTime_ns: 232,
    readout: 'partial',
    fpnRead: true, // mini の現用 = isAllChannelRead=false + isFPNRead=true(111 §5.4)
    zeroSuppression: { enabled: false, threshold: 600 },
    pulser: { enabled: false, voltage_mV: -1000.0, period_ms: 1000 },
  } as Record<string, unknown>,
  exceptions: [] as unknown[],
  unset: [] as string[],
  unsupported: [] as { knob: string; found: string; why: string }[],
};

const TEMPLATES = ['configure-elinp.xcfg', 'configure-pulser-vm.xcfg'];

interface Call {
  url: string;
  init: RequestInit | undefined;
}

function fakeFetch(replies: { status: number; statusText?: string; payload: unknown }[]) {
  const calls: Call[] = [];
  const fetchFn = (url: string, init?: RequestInit): Promise<Response> => {
    calls.push({ url, init });
    const reply = replies[calls.length - 1] ?? { status: 500, payload: { error: '応答が尽きた' } };
    return Promise.resolve({
      ok: reply.status >= 200 && reply.status < 300,
      status: reply.status,
      statusText: reply.statusText ?? '',
      json: () => Promise.resolve(reply.payload),
    } as unknown as Response);
  };
  return { fetchFn, calls };
}

describe('ParamsView', () => {
  let fixture: ComponentFixture<ParamsView>;
  let getReplies: { status: number; payload: unknown }[];
  let putReplies: { status: number; payload: unknown }[];
  let defaultsReplies: { status: number; payload: unknown }[];
  let templatesReply: { status: number; payload: unknown };
  /** `POST /api/run/{stop,start}` への応答キュー。`gate` があればそれが解けるまで応答を保留する。 */
  let postReplies: { status: number; payload: unknown; gate?: Promise<void> }[];
  /** `GET /api/status` への応答キュー(尽きたら最後の 1 通を繰り返す)。 */
  let statusReplies: unknown[];
  let statusCalls: number;
  let wsState: ReturnType<typeof signal<{ state: string } | null>>;

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

  function input(labelText: string): HTMLInputElement {
    const label = [...fixture.nativeElement.querySelectorAll('label')].find((element) =>
      (element as HTMLElement).textContent?.trim().startsWith(labelText),
    );
    if (!label) throw new Error(`ラベルが無い: ${labelText}`);
    const found = (label as HTMLElement).querySelector('input, select');
    if (!found) throw new Error(`入力欄が無い: ${labelText}`);
    return found as HTMLInputElement;
  }

  /** テンプレート選択の `<select>`(一覧が読めたとき)。 */
  function picker(): HTMLSelectElement {
    return fixture.nativeElement.querySelector('select.template-picker') as HTMLSelectElement;
  }

  function choose(name: string): void {
    const select = picker();
    select.value = name;
    select.dispatchEvent(new Event('change'));
    fixture.detectChanges();
  }

  /**
   * `fixture.whenStable()` だけでは足りない: `getParams`/`putParams`/`renderParams` は
   * `fetch → response.json()` と 2 段以上 await が連なるので(zoneless なので raw な
   * Promise チェーンは自動追跡されない)、マイクロタスクを十分な回数消化してから
   * 描画し直す。
   */
  async function settle(): Promise<void> {
    await fixture.whenStable();
    for (let i = 0; i < 20; i++) {
      await Promise.resolve();
    }
    fixture.detectChanges();
  }

  /** GET が先に走るので、順番は [GET, ...PUT/render のうち呼んだ分]。 */
  function setup(
    get: { status: number; payload: unknown }[],
    templates: { status: number; payload: unknown } = {
      status: 200,
      payload: { templates: TEMPLATES },
    },
  ): {
    calls: Call[];
    control: RunControlClient;
  } {
    getReplies = get;
    putReplies = [];
    defaultsReplies = [];
    postReplies = [];
    statusReplies = [{ phase: 'Idle' }];
    statusCalls = 0;
    templatesReply = templates;
    const callSeq: Call[] = [];
    const respond = (reply: { status: number; payload: unknown }): Promise<Response> =>
      Promise.resolve({
        ok: reply.status >= 200 && reply.status < 300,
        status: reply.status,
        statusText: '',
        json: () => Promise.resolve(reply.payload),
      } as unknown as Response);
    const fetchFn = (url: string, init?: RequestInit): Promise<Response> => {
      callSeq.push({ url, init });
      const method = init?.method ?? 'GET';
      if (url === '/api/params/templates') return respond(templatesReply);
      if (url.startsWith('/api/params/defaults')) {
        return respond(
          defaultsReplies.shift() ?? {
            status: 500,
            payload: { error: 'no defaults reply queued' },
          },
        );
      }
      if (method === 'POST') {
        const reply = postReplies.shift() ?? {
          status: 500,
          payload: { error: 'no POST reply queued' },
        };
        return (reply.gate ?? Promise.resolve()).then(() => respond(reply));
      }
      if (method === 'PUT') {
        const reply = putReplies.shift() ?? {
          status: 500,
          payload: { error: 'no PUT reply queued' },
        };
        return Promise.resolve({
          ok: reply.status >= 200 && reply.status < 300,
          status: reply.status,
          statusText: '',
          json: () => Promise.resolve(reply.payload),
        } as unknown as Response);
      }
      const reply = getReplies.shift() ?? {
        status: 500,
        payload: { error: 'no GET reply queued' },
      };
      return Promise.resolve({
        ok: reply.status >= 200 && reply.status < 300,
        status: reply.status,
        statusText: '',
        json: () => Promise.resolve(reply.payload),
      } as unknown as Response);
    };

    const api = new ParamsClient(() => '/api', fetchFn);
    const control = new RunControlClient(() => '/api', fetchFn, null);
    control.setToken(TOKEN);
    wsState = signal<{ state: string } | null>({ state: 'idle' });

    TestBed.configureTestingModule({
      providers: [
        { provide: ParamsApiService, useValue: api as unknown as ParamsApiService },
        { provide: RunControlApiService, useValue: control as unknown as RunControlApiService },
        { provide: WsClientService, useValue: { status: wsState } as unknown as WsClientService },
        {
          provide: ControllerApiService,
          useValue: {
            getStatus: () => {
              statusCalls += 1;
              const reply = statusReplies.length > 1 ? statusReplies.shift() : statusReplies[0];
              return Promise.resolve(reply);
            },
          } as unknown as ControllerApiService,
        },
      ],
    });

    fixture = TestBed.createComponent(ParamsView);
    // Idle 待ちは 200 ms 間隔・30 s 上限が本番値。試験は待たずに、上限は 3 回に縮める。
    fixture.componentInstance.sleep = () => Promise.resolve();
    fixture.componentInstance.idleMaxPolls = 3;
    fixture.detectChanges();
    return { calls: callSeq, control };
  }

  afterEach(() => fixture?.destroy());

  it('404: "No parameters yet" とテンプレート一覧を出し、選んで読むとテンプレートの値でフォームが埋まる', async () => {
    const { calls } = setup([{ status: 404, payload: { error: 'no params configured' } }]);
    await settle();

    expect(text()).toContain('No parameters yet');
    expect(calls.some((c) => c.url === '/api/params/templates')).toBe(true);
    const options = [...picker().querySelectorAll('option')].map((o) => o.value);
    expect(options).toEqual(['', ...TEMPLATES]);
    expect(button('Load from template').disabled).toBe(true);

    defaultsReplies = [{ status: 200, payload: ELINP_DEFAULTS }];
    choose('configure-elinp.xcfg');
    expect(button('Load from template').disabled).toBe(false);
    button('Load from template').click();
    await settle();

    const defaultsCall = calls.find((c) => c.url.startsWith('/api/params/defaults'));
    expect(defaultsCall?.url).toBe('/api/params/defaults?template=configure-elinp.xcfg');
    expect(text()).not.toContain('No parameters yet');
    expect(text()).toContain('Knobs');
    // 旧 DEFAULT_KNOBS(25 MHz / 1748)ではなく、テンプレート由来の 100 MHz / 320。
    expect(input('Trigger delay').value).toBe('320');
    expect(input('Writing clock frequency').value).toBe('100');
    expect(input('Readout').value).toBe('partial');
    expect(input('template').value).toBe('configure-elinp.xcfg');
    expect(text()).toContain('not saved yet');
    expect(text()).not.toContain('unset in template');
  });

  it('defaults の knobs が params_roundtrip.json の knobs なら、フィクスチャの値でフォームが埋まる', async () => {
    const raw = JSON.parse(readFileSync(FIXTURE_PATH, 'utf-8')) as {
      knobs: Record<string, unknown>;
    };
    setup([{ status: 404, payload: { error: 'no params configured' } }]);
    await settle();
    defaultsReplies = [{ status: 200, payload: { ...ELINP_DEFAULTS, knobs: raw.knobs } }];
    choose('configure-elinp.xcfg');
    button('Load from template').click();
    await settle();

    expect(input('Trigger delay').value).toBe('1748');
    expect(input('Writing clock frequency').value).toBe('25');
    expect(input('Trigger mode').value).toBe('external');
    expect(input('Peaking time').value).toBe('232');
    expect(input('Readout').value).toBe('full');
    expect(input('Pulser voltage').value).toBe('-1000');
    expect(input('Pulser period').value).toBe('1000');
    expect(text()).not.toContain('unset in template');
    expect(button('Save').disabled).toBe(false);
  });

  it('テンプレート一覧が読めない(503)ときはテキスト入力に落とし、理由を画面に出す', async () => {
    const { calls } = setup([{ status: 404, payload: { error: 'no params configured' } }], {
      status: 503,
      payload: { error: 'ecc_config_dir is not configured' },
    });
    await settle();

    expect(picker()).toBeNull();
    expect(text()).toContain('ecc_config_dir is not configured');
    const typed = fixture.nativeElement.querySelector('input.template-picker') as HTMLInputElement;
    typed.value = 'configure-elinp.xcfg';
    typed.dispatchEvent(new Event('input'));
    fixture.detectChanges();

    defaultsReplies = [{ status: 200, payload: ELINP_DEFAULTS }];
    button('Load from template').click();
    await settle();

    expect(calls.some((c) => c.url === '/api/params/defaults?template=configure-elinp.xcfg')).toBe(
      true,
    );
    expect(input('Trigger delay').value).toBe('320');
  });

  it('defaults の失敗(404)はサーバの文言をそのまま出し、フォームは作らない', async () => {
    setup([{ status: 404, payload: { error: 'no params configured' } }]);
    await settle();
    defaultsReplies = [
      { status: 404, payload: { error: 'template が無い: configure-elinp.xcfg' } },
    ];
    choose('configure-elinp.xcfg');
    button('Load from template').click();
    await settle();

    expect(text()).toContain('template が無い: configure-elinp.xcfg');
    expect(text()).toContain('No parameters yet');
    expect(text()).not.toContain('Knobs');
  });

  it('defaults の exceptions が Exceptions 表に入る', async () => {
    setup([{ status: 404, payload: { error: 'no params configured' } }]);
    await settle();
    defaultsReplies = [
      {
        status: 200,
        payload: {
          ...ELINP_DEFAULTS,
          exceptions: [{ at: { asad: 1, aget: 2, ch: 5 }, knob: 'lsbThreshold', value: 3 }],
        },
      },
    ];
    choose('configure-elinp.xcfg');
    button('Load from template').click();
    await settle();

    expect(text()).toContain('Exceptions 1');
    const cells = [...fixture.nativeElement.querySelectorAll('table.exceptions tbody input')].map(
      (e) => (e as HTMLInputElement).value,
    );
    // asad / aget / ch / value / note(knob は select)。
    expect(cells.slice(0, 3)).toEqual(['1', '2', '5']);
    expect(cells[3]).toBe('3');
  });

  it('unset の葉は空欄 + 赤バッジ。埋まるまで Save 無効で、理由に葉の名前が出る', async () => {
    setup([{ status: 404, payload: { error: 'no params configured' } }]);
    await settle();
    const knobs = structuredClone(ELINP_DEFAULTS.knobs) as { pulser: Record<string, unknown> };
    delete knobs.pulser['period_ms'];
    defaultsReplies = [
      { status: 200, payload: { ...ELINP_DEFAULTS, knobs, unset: ['pulser.period_ms'] } },
    ];
    choose('configure-elinp.xcfg');
    button('Load from template').click();
    await settle();

    expect(input('Pulser period').value).toBe('');
    const badges = fixture.nativeElement.querySelectorAll('.badge-unset');
    expect(badges).toHaveLength(1);
    expect((badges[0] as HTMLElement).textContent).toContain('unset in template');
    expect(button('Save').disabled).toBe(true);
    expect(text()).toContain('pulser.period_ms');

    // 手で値を入れれば解消(token はある・Idle)。
    const period = input('Pulser period');
    period.value = '500';
    period.dispatchEvent(new Event('change'));
    fixture.detectChanges();
    expect(fixture.nativeElement.querySelectorAll('.badge-unset')).toHaveLength(0);
    expect(button('Save').disabled).toBe(false);
  });

  it('unsupported は警告一覧("knob: found — why")に出し、該当欄は空欄、Save 無効', async () => {
    setup([{ status: 404, payload: { error: 'no params configured' } }]);
    await settle();
    const knobs = { ...ELINP_DEFAULTS.knobs };
    delete knobs['triggerMode'];
    defaultsReplies = [
      {
        status: 200,
        payload: {
          ...ELINP_DEFAULTS,
          knobs,
          unsupported: [
            { knob: 'triggerMode', found: 'onMultiplicity', why: 'not in {external, …}' },
          ],
        },
      },
    ];
    choose('configure-elinp.xcfg');
    button('Load from template').click();
    await settle();

    const warnings = fixture.nativeElement.querySelector('.unsupported-list') as HTMLElement;
    expect(warnings.textContent).toContain('triggerMode: onMultiplicity — not in {external, …}');
    // 空欄(onMultiplicity をそのまま入れない)。
    expect(input('Trigger mode').value).toBe('');
    expect(button('Save').disabled).toBe(true);
    expect(text()).toContain('triggerMode');
  });

  it('読み込み済み画面にも Load from template がある。下書きが無変更なら確認なしで読み直す', async () => {
    const { calls } = setup([{ status: 200, payload: SAVED_INFO }]);
    await settle();
    expect(input('Trigger delay').value).toBe('1748');

    defaultsReplies = [{ status: 200, payload: ELINP_DEFAULTS }];
    choose('configure-elinp.xcfg');
    button('Load from template').click();
    await settle();

    expect(calls.some((c) => c.url === '/api/params/defaults?template=configure-elinp.xcfg')).toBe(
      true,
    );
    expect(text()).not.toContain('will be discarded');
    expect(input('Trigger delay').value).toBe('320');
    // 保存済み id は残る(読み直しは下書きだけ。サーバは触らない)。
    expect(text()).toContain('mini-fixture__797d5dc2');
  });

  it('下書きに変更があるときは確認文言を出し、Cancel で下書きを守り、確認で読み直す', async () => {
    const { calls } = setup([{ status: 200, payload: SAVED_INFO }]);
    await settle();
    const delay = input('Trigger delay');
    delay.value = '999';
    delay.dispatchEvent(new Event('change'));
    fixture.detectChanges();

    defaultsReplies = [{ status: 200, payload: ELINP_DEFAULTS }];
    choose('configure-elinp.xcfg');
    button('Load from template').click();
    await settle();

    expect(text()).toContain('will be discarded');
    expect(calls.some((c) => c.url.startsWith('/api/params/defaults'))).toBe(false);

    button('Cancel').click();
    await settle();
    expect(text()).not.toContain('will be discarded');
    expect(input('Trigger delay').value).toBe('999');

    button('Load from template').click();
    await settle();
    button('Discard edits and reload').click();
    await settle();

    expect(input('Trigger delay').value).toBe('320');
    expect(text()).not.toContain('will be discarded');
  });

  it('200: サーバの params でフォームが埋まり、id・exceptions バッジが出る', async () => {
    setup([{ status: 200, payload: SAVED_INFO }]);
    await settle();

    expect(text()).toContain('mini-fixture__797d5dc2');
    expect(text()).toContain('exceptions 0');
    expect(input('Trigger delay').value).toBe('1748');
    expect(input('Gain').value).toBe('120fC');
  });

  it('503(ecc_config_dir 未設定)はロードエラーとして出す(No parameters yet にしない)', async () => {
    setup([{ status: 503, payload: { error: 'ecc_config_dir is not configured' } }]);
    await settle();

    expect(text()).toContain('ecc_config_dir is not configured');
    expect(text()).not.toContain('No parameters yet');
  });

  it('例外の追加・削除でバッジの件数が動く。Reset to uniform で全消し', async () => {
    setup([{ status: 200, payload: SAVED_INFO }]);
    await settle();

    expect(text()).toContain('Exceptions 0');

    button('Add exception').click();
    fixture.detectChanges();
    button('Add exception').click();
    fixture.detectChanges();
    expect(text()).toContain('Exceptions 2');

    const removeButtons = [...fixture.nativeElement.querySelectorAll('button')].filter(
      (b) => (b as HTMLButtonElement).textContent?.trim() === 'Remove',
    );
    (removeButtons[0] as HTMLButtonElement).click();
    fixture.detectChanges();
    expect(text()).toContain('Exceptions 1');

    button('Reset to uniform').click();
    fixture.detectChanges();
    expect(text()).toContain('Exceptions 0');
  });

  it('token が無ければ Save は無効("Acquire control in Run control first")', async () => {
    const { control } = setup([{ status: 200, payload: SAVED_INFO }]);
    await settle();
    control.setToken(null);
    fixture.detectChanges();

    expect(button('Save').disabled).toBe(true);
    expect(text()).toContain('Acquire control in Run control first');
  });

  it('Running 中(WS status.state==="running")は Save 無効・理由を表示', async () => {
    setup([{ status: 200, payload: SAVED_INFO }]);
    await settle();
    wsState.set({ state: 'running' });
    fixture.detectChanges();

    expect(button('Save').disabled).toBe(true);
    expect(text()).toContain('A run is active');
  });

  it('Save 成功で id・exceptions・files を表示する', async () => {
    const { calls } = setup([{ status: 200, payload: SAVED_INFO }]);
    await settle();
    putReplies = [{ status: 200, payload: SAVED_INFO }];

    button('Save').click();
    await settle();

    const putCall = calls.find((c) => c.init?.method === 'PUT');
    expect(putCall).toBeDefined();
    const body = JSON.parse(String(putCall?.init?.body));
    expect(body.token).toBe(TOKEN);
    expect(body.params.schema).toBe('tpcdaq-params/1');
    expect(text()).toContain('Saved');
    expect(text()).toContain('mini-fixture__797d5dc2');
    expect(text()).toContain('/tmp/ecc/configure-mini-fixture__797d5dc2.xcfg');
  });

  it('Save 失敗(400)はサーバの文言をそのまま出す', async () => {
    setup([{ status: 200, payload: SAVED_INFO }]);
    await settle();
    putReplies = [{ status: 400, payload: { error: 'ノブ gain の値 7pC は許された値ではない' } }];

    button('Save').click();
    await settle();

    expect(text()).toContain('ノブ gain の値 7pC は許された値ではない');
  });

  it('Preview は保存済み(info あり)のときだけ押せ、管理ノブの行をハイライトする', async () => {
    const { calls } = setup([{ status: 200, payload: SAVED_INFO }]);
    await settle();

    expect(button('Preview xcfg').disabled).toBe(false);

    const xcfgText = [
      '<Setup id="Conditions">',
      '  <triggerDelay unit="10ns">1748</triggerDelay>',
      '  <someUnrelatedThing>true</someUnrelatedThing>',
      '</Setup>',
    ].join('\n');
    getReplies.push({
      status: 200,
      payload: { id: 'mini-fixture__797d5dc2', exceptions: 0, configure_xcfg: xcfgText },
    });

    button('Preview xcfg').click();
    await settle();

    const renderCall = calls.find((c) => c.url.startsWith('/api/params/render'));
    expect(renderCall).toBeDefined();

    const lines = [...fixture.nativeElement.querySelectorAll('.xcfg-preview .line')];
    const highlighted = lines.filter((line) => (line as HTMLElement).classList.contains('hl'));
    expect(highlighted).toHaveLength(1);
    expect((highlighted[0] as HTMLElement).textContent).toContain('triggerDelay');
  });

  it('Preview は info が無い(まだ保存していない)間は無効', async () => {
    setup([{ status: 404, payload: { error: 'no params configured' } }]);
    await settle();
    defaultsReplies = [{ status: 200, payload: ELINP_DEFAULTS }];
    choose('configure-elinp.xcfg');
    button('Load from template').click();
    await settle();

    expect(button('Preview xcfg').disabled).toBe(true);
    expect(text()).toContain('Save once first');
  });

  describe('Apply & restart (TODO/112 §3)', () => {
    const STOP_OK = { status: 200, payload: { ok: true, run: 7 } };
    const START_OK = { status: 200, payload: { run: 8, phase: 'Running' } };

    function posts(calls: Call[]): string[] {
      return calls.filter((c) => c.init?.method === 'POST').map((c) => c.url);
    }

    /** 状態を変える呼び出し(POST / PUT)の順序だけを抜き出す。 */
    function sequence(calls: Call[]): string[] {
      return calls
        .filter((c) => c.init?.method === 'POST' || c.init?.method === 'PUT')
        .map((c) => `${c.init?.method} ${c.url}`);
    }

    it('Running: Stop → Idle 待ち → PUT → Start の順に 3 段。comment は note 欄、Run N+1 を表示', async () => {
      const { calls } = setup([{ status: 200, payload: SAVED_INFO }]);
      await settle();
      wsState.set({ state: 'running', run: 7 } as { state: string });
      postReplies = [STOP_OK, START_OK];
      putReplies = [{ status: 200, payload: SAVED_INFO }];
      statusReplies = [{ phase: 'Stopping' }, { phase: 'Idle' }];
      fixture.detectChanges();

      expect(button('Apply & restart').disabled).toBe(false);
      button('Apply & restart').click();
      await settle();

      expect(sequence(calls)).toEqual([
        'POST /api/run/stop',
        'PUT /api/params',
        'POST /api/run/start',
      ]);
      expect(statusCalls).toBe(2); // Stopping → Idle の 2 回目で抜ける。
      const stopBody = JSON.parse(String(calls.find((c) => c.url === '/api/run/stop')?.init?.body));
      expect(stopBody).toEqual({ token: TOKEN });
      const startBody = JSON.parse(
        String(calls.find((c) => c.url === '/api/run/start')?.init?.body),
      );
      // SAVED_PARAMS.note が Parameters の note 欄に入っている。
      expect(startBody).toEqual({ token: TOKEN, comment: 'template_mini.xcfg の現行値そのまま' });
      expect(text()).toContain('1/3 Stopping run 7');
      expect(text()).toContain('2/3 Saving parameters');
      expect(text()).toContain('3/3 Starting run');
      expect(text()).toContain('Run 8 started with mini-fixture__797d5dc2');
      expect(button('Apply & restart').disabled).toBe(false);
    });

    it('note 欄が空なら comment は "apply: <params id>"', async () => {
      const { calls } = setup([{ status: 200, payload: SAVED_INFO }]);
      await settle();
      fixture.componentInstance.noteText.set('');
      postReplies = [START_OK];
      putReplies = [{ status: 200, payload: SAVED_INFO }];

      button('Apply & restart').click();
      await settle();

      const startBody = JSON.parse(
        String(calls.find((c) => c.url === '/api/run/start')?.init?.body),
      );
      expect(startBody).toEqual({ token: TOKEN, comment: 'apply: mini-fixture__797d5dc2' });
    });

    it('Idle: stop を飛ばして PUT → Start の 2 段(status も見ない)', async () => {
      const { calls } = setup([{ status: 200, payload: SAVED_INFO }]);
      await settle();
      postReplies = [START_OK];
      putReplies = [{ status: 200, payload: SAVED_INFO }];

      button('Apply & restart').click();
      await settle();

      expect(sequence(calls)).toEqual(['PUT /api/params', 'POST /api/run/start']);
      expect(statusCalls).toBe(0);
      expect(text()).not.toContain('Stopping run');
      expect(text()).toContain('1/2 Saving parameters');
      expect(text()).toContain('2/2 Starting run');
      expect(text()).toContain('Run 8 started with mini-fixture__797d5dc2');
    });

    it('stop 失敗(HTTP 409)はサーバの文言を出し、以降の段を実行しない', async () => {
      const { calls } = setup([{ status: 200, payload: SAVED_INFO }]);
      await settle();
      wsState.set({ state: 'running' });
      postReplies = [{ status: 409, payload: { error: 'run は Stopping 中' } }];

      button('Apply & restart').click();
      await settle();

      expect(sequence(calls)).toEqual(['POST /api/run/stop']);
      expect(statusCalls).toBe(0);
      expect(text()).toContain('run は Stopping 中');
      expect(text()).not.toContain('started with');
    });

    it('stop が 200 でも ok:false(異常終了)なら失敗として止める', async () => {
      const { calls } = setup([{ status: 200, payload: SAVED_INFO }]);
      await settle();
      wsState.set({ state: 'running' });
      postReplies = [{ status: 200, payload: { ok: false, reason: 'forced EOS', notes: ['n1'] } }];

      button('Apply & restart').click();
      await settle();

      expect(sequence(calls)).toEqual(['POST /api/run/stop']);
      expect(text()).toContain('Stop failed');
      expect(text()).toContain('n1');
    });

    it('Idle に 30 s 届かない: "Stop did not reach Idle in 30 s" を出し、PUT / start しない', async () => {
      const { calls } = setup([{ status: 200, payload: SAVED_INFO }]);
      await settle();
      wsState.set({ state: 'running' });
      postReplies = [STOP_OK];
      statusReplies = [{ phase: 'Stopping' }];

      button('Apply & restart').click();
      await settle();

      expect(statusCalls).toBe(3); // idleMaxPolls = 3
      expect(sequence(calls)).toEqual(['POST /api/run/stop']);
      expect(text()).toContain('Stop did not reach Idle in 30 s');
      expect(button('Apply & restart').disabled).toBe(false);
    });

    it('Idle 待ち中の status 取得失敗は再試行する(2 回失敗 → 3 回目で Idle → 続行)', async () => {
      const { calls } = setup([{ status: 200, payload: SAVED_INFO }]);
      await settle();
      wsState.set({ state: 'running' });
      postReplies = [STOP_OK, START_OK];
      putReplies = [{ status: 200, payload: SAVED_INFO }];
      const outcomes: (() => Promise<unknown>)[] = [
        () => Promise.reject(new Error('GET /api/status → HTTP 500')),
        () => Promise.resolve('not an object'), // 想定外の形
        () => Promise.resolve({ phase: 'Idle' }),
      ];
      let polls = 0;
      TestBed.inject(ControllerApiService).getStatus = () => outcomes[polls++]();

      button('Apply & restart').click();
      await settle();

      expect(polls).toBe(3);
      expect(sequence(calls)).toEqual([
        'POST /api/run/stop',
        'PUT /api/params',
        'POST /api/run/start',
      ]);
      expect(text()).toContain('Run 8 started with');
    });

    it('status がずっと取れない: 上限で "Stop did not reach Idle in 30 s (last status error: …)" で止まる', async () => {
      const { calls } = setup([{ status: 200, payload: SAVED_INFO }]);
      await settle();
      wsState.set({ state: 'running' });
      postReplies = [STOP_OK];
      let polls = 0;
      TestBed.inject(ControllerApiService).getStatus = () => {
        polls += 1;
        return Promise.reject(new Error('GET /api/status → HTTP 500'));
      };

      button('Apply & restart').click();
      await settle();

      expect(polls).toBe(3); // idleMaxPolls = 3
      expect(sequence(calls)).toEqual(['POST /api/run/stop']);
      expect(text()).toContain(
        'Stop did not reach Idle in 30 s (last status error: GET /api/status → HTTP 500)',
      );
    });

    it('PUT 400 では start せず、run は止まったままと明示する', async () => {
      const { calls } = setup([{ status: 200, payload: SAVED_INFO }]);
      await settle();
      wsState.set({ state: 'running' });
      postReplies = [STOP_OK];
      putReplies = [{ status: 400, payload: { error: 'ノブ gain の値 7pC は許された値ではない' } }];

      button('Apply & restart').click();
      await settle();

      expect(sequence(calls)).toEqual(['POST /api/run/stop', 'PUT /api/params']);
      expect(posts(calls)).not.toContain('/api/run/start');
      expect(text()).toContain('ノブ gain の値 7pC は許された値ではない');
      expect(text()).toContain('The run is stopped and was not restarted');
    });

    it('start 401 は文言そのままに "run は止まったまま" を明示する(パラメータは保存済み)', async () => {
      setup([{ status: 200, payload: SAVED_INFO }]);
      await settle();
      wsState.set({ state: 'running' });
      postReplies = [
        STOP_OK,
        { status: 401, payload: { error: 'token が違う(別の operator が取得)' } },
      ];
      putReplies = [{ status: 200, payload: SAVED_INFO }];

      button('Apply & restart').click();
      await settle();

      expect(text()).toContain('token が違う(別の operator が取得)');
      expect(text()).toContain('The run is still stopped');
      expect(text()).toContain('mini-fixture__797d5dc2'); // 保存はできている
      expect(text()).not.toContain('started with');
    });

    it('進行中は全ボタン無効(Save / Apply / Preview)。終われば戻る', async () => {
      setup([{ status: 200, payload: SAVED_INFO }]);
      await settle();
      wsState.set({ state: 'running' });
      let release: () => void = () => undefined;
      const gate = new Promise<void>((resolve) => (release = resolve));
      postReplies = [{ ...STOP_OK, gate }, START_OK];
      putReplies = [{ status: 200, payload: SAVED_INFO }];

      button('Apply & restart').click();
      await settle();

      expect(button('Apply & restart').disabled).toBe(true);
      expect(button('Save').disabled).toBe(true);
      expect(button('Preview xcfg').disabled).toBe(true);
      expect(text()).toContain('1/3 Stopping run');

      wsState.set({ state: 'idle' });
      release();
      await settle();
      expect(button('Apply & restart').disabled).toBe(false);
      expect(text()).toContain('Run 8 started with');
    });

    it('token が無ければ Apply & restart は無効(理由を表示)。Save と違い Running でも有効', async () => {
      const { control } = setup([{ status: 200, payload: SAVED_INFO }]);
      await settle();
      wsState.set({ state: 'running' });
      fixture.detectChanges();
      expect(button('Apply & restart').disabled).toBe(false);
      expect(button('Save').disabled).toBe(true);

      control.setToken(null);
      fixture.detectChanges();
      expect(button('Apply & restart').disabled).toBe(true);
      expect(text()).toContain('Acquire control in Run control first');
    });
  });
});
