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

import { RunControlApiService, RunControlClient } from '../api/run-control-api';
import { ParamsApiService, ParamsClient } from '../api/params-api';
import { WsClientService } from '../ws/ws-client';
import { DEFAULT_KNOBS, ParamsView } from './params-view';

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
  function setup(get: { status: number; payload: unknown }[]): {
    calls: Call[];
    control: RunControlClient;
  } {
    getReplies = get;
    putReplies = [];
    const callSeq: Call[] = [];
    const fetchFn = (url: string, init?: RequestInit): Promise<Response> => {
      callSeq.push({ url, init });
      const method = init?.method ?? 'GET';
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
      ],
    });

    fixture = TestBed.createComponent(ParamsView);
    fixture.detectChanges();
    return { calls: callSeq, control };
  }

  afterEach(() => fixture?.destroy());

  it('404: "No parameters yet" を出し、テンプレート名を入れると既定値のフォームができる', async () => {
    setup([{ status: 404, payload: { error: 'no params configured' } }]);
    await settle();

    expect(text()).toContain('No parameters yet');
    expect(button('Load defaults from template').disabled).toBe(true);

    const templateInput = fixture.nativeElement.querySelector('.card input') as HTMLInputElement;
    templateInput.value = 'configure-mini-fixture.xcfg';
    templateInput.dispatchEvent(new Event('input'));
    fixture.detectChanges();

    expect(button('Load defaults from template').disabled).toBe(false);
    button('Load defaults from template').click();
    await settle();

    expect(text()).not.toContain('No parameters yet');
    expect(text()).toContain('Knobs');
    // 既定値(= params_roundtrip.json)が反映されている。
    expect(input('Trigger delay').value).toBe('1748');
    expect(input('Global threshold').value).toBe('1');
    expect(text()).toContain('not saved yet');
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
    const templateInput = fixture.nativeElement.querySelector('.card input') as HTMLInputElement;
    templateInput.value = 'configure-mini-fixture.xcfg';
    templateInput.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    button('Load defaults from template').click();
    await settle();

    expect(button('Preview xcfg').disabled).toBe(true);
    expect(text()).toContain('Save once first');
  });
});

describe('DEFAULT_KNOBS ↔ tests/fixtures/params/params_roundtrip.json', () => {
  it('「既定値で作る」の値は往復フィクスチャの knobs と一致する(ずれの検知)', () => {
    const raw = JSON.parse(readFileSync(FIXTURE_PATH, 'utf-8')) as { knobs: unknown };
    expect(DEFAULT_KNOBS).toEqual(raw.knobs);
  });
});
