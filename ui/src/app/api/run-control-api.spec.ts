/**
 * 操作系 REST(SPEC §8.1)の**請求形の機械照合**と、応答の読み方(050-A / 050-D)。
 *
 * ここが守るもの:
 * ① 12 本の URL が `run-actions.ts` の表の endpoint と**バイト一致**する。
 * ② body が表の `body` 欄と**フィールド名まで一致**する(表と実装がずれたら赤くなる)。
 *    controller 側の受け口(`src/controller.rs` の `AcquireRequest` / `TokenRequest` /
 *    `RunStartRequest{token, comment=#[serde(default)]}` / `RunNextRequest{token, next_run}`)を
 *    2026-08-15 に読んで一致を確認済み。表はその写し。
 * ③ token が acquire 以外の全部に付く(acquire には付かない)。
 * ④ 失敗応答の `error` を**加工しない**。`notes` を落とさない。HTTP 200 + `ok:false` を
 *    成功にしない(run/stop の異常終了、実 ECC の Ignored/Denied)。
 */
import {
  RUN_ACTIONS,
  RUN_CONTROL_ENABLED,
  type RunAction,
  isRunActionDisabled,
} from '../run/run-actions';
import { apiUrl } from './controller-api';
import {
  EMPTY_INPUT,
  RunControlClient,
  TOKEN_STORAGE_KEY,
  type TokenStorage,
  actionByEndpoint,
  actionPath,
  buildActionRequest,
  interpretResponse,
  parsePositiveInt,
} from './run-control-api';

/** 全欄を埋めた入力(非対称に — 取り違えたら分かる値にする)。 */
const FULL = {
  operator: 'aogaki',
  passphrase: 'demo-passphrase',
  comment: '041 integrated demo',
  nextRun: '7',
};

const TOKEN = 'tok-abcdef0123456789';

function planOf(endpoint: string, token: string | null = TOKEN, input = FULL) {
  return buildActionRequest(actionByEndpoint(endpoint), input, token);
}

function bodyOf(endpoint: string, token: string | null = TOKEN, input = FULL) {
  const plan = planOf(endpoint, token, input);
  if (!plan.ok) throw new Error(`送れない: ${plan.error}`);
  return plan.body;
}

/** 表の `body` 欄(`{token, comment?}`)→ フィールド名。 */
function tableFields(action: RunAction): { required: string[]; optional: string[] } {
  const names = action.body
    .replace(/^\{/, '')
    .replace(/\}$/, '')
    .split(',')
    .map((name) => name.trim())
    .filter((name) => name.length > 0);
  return {
    required: names.filter((name) => !name.endsWith('?')),
    optional: names.filter((name) => name.endsWith('?')).map((name) => name.slice(0, -1)),
  };
}

describe('請求形 ① URL — 表の endpoint と一致する', () => {
  it('既定の same-origin `/api` で 12 本すべて表どおり', () => {
    const urls = RUN_ACTIONS.map((action) => apiUrl('/api', actionPath(action.endpoint)));
    expect(urls).toEqual(RUN_ACTIONS.map((action) => action.endpoint));
  });

  it('`ui-config.json` で絶対 URL にしても末尾だけが変わる', () => {
    expect(apiUrl('https://daq-pc.lan/api', actionPath('/api/run/start'))).toBe(
      'https://daq-pc.lan/api/run/start',
    );
    expect(apiUrl('https://daq-pc.lan/api/', actionPath('/api/ecc/breakup'))).toBe(
      'https://daq-pc.lan/api/ecc/breakup',
    );
  });
});

describe('請求形 ② body — 表の欄と実装をフィールド名で照合する', () => {
  it('全欄を埋めれば body の鍵 = 表の必須 + 任意(12 本)', () => {
    for (const action of RUN_ACTIONS) {
      const { required, optional } = tableFields(action);
      const keys = Object.keys(bodyOf(action.endpoint)).sort();
      expect([action.endpoint, keys]).toEqual([action.endpoint, [...required, ...optional].sort()]);
    }
  });

  it('任意欄が空なら body に載せない(必須だけ)', () => {
    for (const action of RUN_ACTIONS) {
      const { required } = tableFields(action);
      const keys = Object.keys(bodyOf(action.endpoint, TOKEN, { ...FULL, comment: '   ' })).sort();
      expect([action.endpoint, keys]).toEqual([action.endpoint, [...required].sort()]);
    }
  });

  it('acquire は token を付けない(token を得る要求だから)', () => {
    expect(bodyOf('/api/control/acquire', null)).toEqual({
      operator: 'aogaki',
      passphrase: 'demo-passphrase',
    });
  });

  it('acquire 以外の 11 本は token をそのまま付ける', () => {
    const others = RUN_ACTIONS.filter((a) => a.endpoint !== '/api/control/acquire');
    expect(others).toHaveLength(11);
    for (const action of others) {
      expect([action.endpoint, bodyOf(action.endpoint)['token']]).toEqual([action.endpoint, TOKEN]);
    }
  });

  it('run/start の comment は trim して載せる', () => {
    expect(bodyOf('/api/run/start')).toEqual({ token: TOKEN, comment: '041 integrated demo' });
    expect(bodyOf('/api/run/start', TOKEN, { ...FULL, comment: '  spaces  ' })).toEqual({
      token: TOKEN,
      comment: 'spaces',
    });
  });

  it('run/next は数値の 7 を送る(文字列の "7" ではない)', () => {
    expect(bodyOf('/api/run/next')).toEqual({ token: TOKEN, next_run: 7 });
  });

  it('ECC 7 本はどれも {token} だけ(config_id やリンクは controller が作る)', () => {
    const ecc = RUN_ACTIONS.filter((a) => a.endpoint.startsWith('/api/ecc/'));
    expect(ecc.map((a) => a.endpoint)).toEqual([
      '/api/ecc/describe',
      '/api/ecc/prepare',
      '/api/ecc/configure',
      '/api/ecc/start',
      '/api/ecc/stop',
      '/api/ecc/breakup',
      '/api/ecc/reset',
    ]);
    for (const action of ecc) {
      expect(bodyOf(action.endpoint)).toEqual({ token: TOKEN });
    }
  });
});

describe('請求形 ③ 送らない場合 — 画面で分かる誤りは往復させない', () => {
  it('token 無しなら acquire 以外は組み立てない', () => {
    for (const action of RUN_ACTIONS) {
      const plan = buildActionRequest(action, FULL, null);
      if (action.endpoint === '/api/control/acquire') {
        expect(plan.ok).toBe(true);
      } else {
        expect([action.endpoint, plan.ok]).toEqual([action.endpoint, false]);
      }
    }
  });

  it('next_run は正整数だけ', () => {
    expect(parsePositiveInt('7')).toBe(7);
    expect(parsePositiveInt('  12  ')).toBe(12);
    expect(parsePositiveInt('4294967295')).toBe(4294967295); // u32::MAX(controller の上限)
    expect(parsePositiveInt('4294967296')).toBeNull();
    expect(parsePositiveInt('0')).toBeNull();
    expect(parsePositiveInt('-3')).toBeNull();
    expect(parsePositiveInt('2.5')).toBeNull();
    expect(parsePositiveInt('')).toBeNull();
    expect(parsePositiveInt('abc')).toBeNull();
  });

  it('next_run が不正なら理由つきで止まる(入力をそのまま見せる)', () => {
    const plan = planOf('/api/run/next', TOKEN, { ...FULL, nextRun: 'zero' });
    expect(plan.ok).toBe(false);
    if (!plan.ok) expect(plan.error).toContain('"zero"');
  });
});

describe('応答の読み方 — error は加工しない / notes を落とさない', () => {
  it('403 wrong passphrase(controller の文字列そのまま)', () => {
    const result = interpretResponse(403, { error: 'wrong passphrase' });
    expect(result.ok).toBe(false);
    expect(result.error).toBe('wrong passphrase');
    expect(result.status).toBe(403);
  });

  it('401 stale or unknown token', () => {
    expect(interpretResponse(401, { error: 'stale or unknown token' }).error).toBe(
      'stale or unknown token',
    );
  });

  it('409 は phase 衝突の文言をそのまま(run 中の next_run 等)', () => {
    const text = 'controller is Running — cannot set next_run while a run is active';
    expect(interpretResponse(409, { error: text }).error).toBe(text);
  });

  it('run/start 成功は run 番号を要約に出す', () => {
    const result = interpretResponse(200, { run: 12, phase: 'Running' });
    expect(result.ok).toBe(true);
    expect(result.summary).toBe('{"run":12,"phase":"Running"}');
    expect(result.error).toBeNull();
  });

  it('run/stop の正常終了は notes を全部持つ', () => {
    const result = interpretResponse(200, {
      run: 12,
      ok: true,
      reason: 'normal',
      forced_eos: false,
      eos_closed: true,
      notes: ['graw-writer: 4 files', 'root-sink: 108 events'],
    });
    expect(result.ok).toBe(true);
    expect(result.notes).toEqual(['graw-writer: 4 files', 'root-sink: 108 events']);
  });

  it('HTTP 200 でも ok:false は失敗(run/stop の異常終了)', () => {
    const result = interpretResponse(200, {
      run: 13,
      ok: false,
      reason: 'error:eos-timeout',
      forced_eos: true,
      notes: ['decoder: EndOfStream を 5 s 待って諦めた'],
    });
    expect(result.ok).toBe(false);
    expect(result.notes).toEqual(['decoder: EndOfStream を 5 s 待って諦めた']);
    expect(result.summary).toContain('"reason":"error:eos-timeout"');
  });

  it('HTTP 200 + ok:false の ECC(実 ECC の Ignored/Denied)も失敗にする', () => {
    const result = interpretResponse(200, {
      ok: false,
      state: 'Idle',
      error: 'Ignored: the transition is not allowed in state Idle',
    });
    expect(result.ok).toBe(false);
    expect(result.error).toBe('Ignored: the transition is not allowed in state Idle');
  });

  it('本文が JSON でない失敗(プロキシの 404 等)は status だけで語る', () => {
    const result = interpretResponse(404, null);
    expect(result.ok).toBe(false);
    expect(result.error).toBeNull();
    expect(result.summary).toBe('');
  });

  it('要約に token を出さない(credential を画面に残さない)', () => {
    const result = interpretResponse(200, { token: TOKEN, preempted: null });
    expect(result.summary).toBe('{"preempted":null}');
    expect(result.summary).not.toContain(TOKEN);
  });
});

/** sessionStorage の代わり(テスト用の最小実装)。 */
function fakeStorage(
  initial: Record<string, string> = {},
): TokenStorage & { map: Map<string, string> } {
  const map = new Map(Object.entries(initial));
  return {
    map,
    getItem: (key) => map.get(key) ?? null,
    setItem: (key, value) => void map.set(key, value),
    removeItem: (key) => void map.delete(key),
  };
}

interface Call {
  url: string;
  init: RequestInit | undefined;
}

/** 応答を並べておく fetch。呼ばれた回数と中身を残す。 */
function fakeFetch(replies: { status: number; payload: unknown }[]) {
  const calls: Call[] = [];
  const fetchFn = (url: string, init?: RequestInit): Promise<Response> => {
    calls.push({ url, init });
    const reply = replies[calls.length - 1] ?? { status: 500, payload: { error: '応答が尽きた' } };
    return Promise.resolve({
      status: reply.status,
      json: () => Promise.resolve(reply.payload),
    } as unknown as Response);
  };
  return { fetchFn, calls };
}

describe('RunControlClient — token の保持と実際の送信', () => {
  it('acquire 成功で token をメモリと sessionStorage に持つ', async () => {
    const storage = fakeStorage();
    const { fetchFn, calls } = fakeFetch([
      { status: 200, payload: { token: TOKEN, preempted: 'someone-else' } },
    ]);
    const client = new RunControlClient(() => '/api', fetchFn, storage);

    const result = await client.execute(actionByEndpoint('/api/control/acquire'), FULL);

    expect(result.ok).toBe(true);
    expect(client.token()).toBe(TOKEN);
    expect(client.preempted()).toBe('someone-else');
    expect(storage.map.get(TOKEN_STORAGE_KEY)).toBe(TOKEN);
    expect(calls).toHaveLength(1);
    expect(calls[0].url).toBe('/api/control/acquire');
    expect(calls[0].init?.method).toBe('POST');
    expect(calls[0].init?.headers).toEqual({ 'content-type': 'application/json' });
    expect(calls[0].init?.body).toBe('{"operator":"aogaki","passphrase":"demo-passphrase"}');
  });

  it('リロードしても sessionStorage から token を拾う', () => {
    const storage = fakeStorage({ [TOKEN_STORAGE_KEY]: TOKEN });
    const client = new RunControlClient(() => '/api', fakeFetch([]).fetchFn, storage);
    expect(client.token()).toBe(TOKEN);
  });

  it('release 成功で token を捨てる(メモリも storage も)', async () => {
    const storage = fakeStorage({ [TOKEN_STORAGE_KEY]: TOKEN });
    const { fetchFn, calls } = fakeFetch([{ status: 200, payload: { released: true } }]);
    const client = new RunControlClient(() => '/api', fetchFn, storage);

    const result = await client.execute(actionByEndpoint('/api/control/release'), EMPTY_INPUT);

    expect(result.ok).toBe(true);
    expect(client.token()).toBeNull();
    expect(storage.map.has(TOKEN_STORAGE_KEY)).toBe(false);
    expect(calls[0].init?.body).toBe(`{"token":"${TOKEN}"}`);
  });

  it('401 の release では token を捨てない(サーバの言い分をそのまま出す)', async () => {
    const storage = fakeStorage({ [TOKEN_STORAGE_KEY]: TOKEN });
    const { fetchFn } = fakeFetch([{ status: 401, payload: { error: 'stale or unknown token' } }]);
    const client = new RunControlClient(() => '/api', fetchFn, storage);

    const result = await client.execute(actionByEndpoint('/api/control/release'), EMPTY_INPUT);

    expect(result.error).toBe('stale or unknown token');
    expect(client.token()).toBe(TOKEN);
  });

  it('200 なのに token が無い acquire は成功にしない', async () => {
    const { fetchFn } = fakeFetch([{ status: 200, payload: { preempted: null } }]);
    const client = new RunControlClient(() => '/api', fetchFn, null);

    const result = await client.execute(actionByEndpoint('/api/control/acquire'), FULL);

    expect(result.ok).toBe(false);
    expect(result.error).toBe('The acquire response carries no token');
    expect(client.token()).toBeNull();
  });

  it('token 無しで run/start を押しても 1 通も送らない', async () => {
    const { fetchFn, calls } = fakeFetch([{ status: 200, payload: { run: 1 } }]);
    const client = new RunControlClient(() => '/api', fetchFn, null);

    const result = await client.execute(actionByEndpoint('/api/run/start'), FULL);

    expect(calls).toHaveLength(0);
    expect(result.status).toBe(0);
    expect(result.error).toContain('token');
  });

  it('controller に届かないときは URL つきで言う(黙らない)', async () => {
    const client = new RunControlClient(
      () => 'http://127.0.0.1:8080/api',
      () => Promise.reject(new Error('Failed to fetch')),
      null,
    );
    client.setToken(TOKEN);

    const result = await client.execute(actionByEndpoint('/api/run/stop'), EMPTY_INPUT);

    expect(result.ok).toBe(false);
    expect(result.status).toBe(0);
    expect(result.error).toBe(
      'POST http://127.0.0.1:8080/api/run/stop did not get through: Failed to fetch',
    );
  });

  it('ecc/configure は表どおり {token} を実際に送る', async () => {
    const { fetchFn, calls } = fakeFetch([
      { status: 200, payload: { ok: true, state: 'Ready', error: '' } },
    ]);
    const client = new RunControlClient(() => '/api', fetchFn, null);
    client.setToken(TOKEN);

    const result = await client.execute(actionByEndpoint('/api/ecc/configure'), EMPTY_INPUT);

    expect(calls[0].url).toBe('/api/ecc/configure');
    expect(calls[0].init?.body).toBe(`{"token":"${TOKEN}"}`);
    expect(result.ok).toBe(true);
    expect(result.summary).toBe('{"ok":true,"state":"Ready","error":""}');
  });
});

describe('配線と disabled 規則の噛み合わせ', () => {
  it('配線が有効なので押せる操作が実際に送信まで行く', () => {
    expect(RUN_CONTROL_ENABLED).toBe(true);
    const state = { enabled: RUN_CONTROL_ENABLED, hasToken: true, runActive: false, busy: false };
    for (const action of RUN_ACTIONS) {
      expect(isRunActionDisabled(action, state)).toBe(false);
      expect(buildActionRequest(action, FULL, TOKEN).ok).toBe(true);
    }
  });
});
