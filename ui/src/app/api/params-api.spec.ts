/**
 * パラメータ REST クライアント(SPEC v1.33 §3.3/§8.1、078 レーン B 結果節が正)。
 *
 * 確かめること:
 * ① `GET /api/params` — 404 は `null`、200 はそのまま返す、それ以外(503/500)は
 *    `ParamsApiError` を **サーバの `error` 文言のまま** 投げる。
 * ② `PUT /api/params` — body が `{token, params}`、400/401/409/503 それぞれの文言を保つ。
 * ③ `GET /api/params/render` — `template` の有無で URL が変わる、書かない(GET のみ)。
 * ④ 応答が届かない(fetch が reject)場合も status 0 + URL つきの文言で投げる(silent 禁止)。
 */
import { type ParamsInfo, type ParamsJson, ParamsApiError, ParamsClient } from './params-api';

const PARAMS_JSON: ParamsJson = {
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
  exceptions: [],
  note: 'test',
};

const PARAMS_INFO: ParamsInfo = {
  id: 'mini-fixture__797d5dc2',
  sha256: '797d5dc245553d3aaae0c7172993e6a311de94e7d48f211855ecc6c762112ac5',
  exceptions: 0,
  params: PARAMS_JSON,
  files: {
    configure: '/tmp/ecc/configure-mini-fixture__797d5dc2.xcfg',
    prepare: '/tmp/ecc/prepare-mini-fixture__797d5dc2.xcfg',
  },
};

interface Call {
  url: string;
  init: RequestInit | undefined;
}

/** 応答を並べておく fetch(`run-control-api.spec.ts` と同じ形)。 */
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

describe('ParamsClient.getParams — GET /api/params', () => {
  it('200 はそのまま返す', async () => {
    const { fetchFn, calls } = fakeFetch([{ status: 200, payload: PARAMS_INFO }]);
    const client = new ParamsClient(() => '/api', fetchFn);

    const result = await client.getParams();

    expect(result).toEqual(PARAMS_INFO);
    expect(calls[0].url).toBe('/api/params');
    expect(calls[0].init?.method).toBe('GET');
  });

  it('404 は null(まだパラメータが無い、というだけで失敗ではない)', async () => {
    const { fetchFn } = fakeFetch([{ status: 404, payload: { error: 'no params configured' } }]);
    const client = new ParamsClient(() => '/api', fetchFn);

    expect(await client.getParams()).toBeNull();
  });

  it('503(ecc_config_dir 未設定)はサーバの文言のまま投げる', async () => {
    const { fetchFn } = fakeFetch([
      { status: 503, payload: { error: 'ecc_config_dir is not configured' } },
    ]);
    const client = new ParamsClient(() => '/api', fetchFn);

    await expect(client.getParams()).rejects.toMatchObject({
      status: 503,
      message: 'ecc_config_dir is not configured',
    });
  });

  it('500(サーバ側の不整合)も投げる。エラーが JSON でなければ fallback 文言', async () => {
    const { fetchFn } = fakeFetch([
      { status: 500, statusText: 'Internal Server Error', payload: null },
    ]);
    const client = new ParamsClient(() => '/api', fetchFn);

    await expect(client.getParams()).rejects.toThrow(
      'GET /api/params → HTTP 500 Internal Server Error',
    );
  });

  it('fetch が届かない(ネットワーク断)は status 0 + URL つきで言う', async () => {
    const client = new ParamsClient(
      () => 'http://127.0.0.1:8080/api',
      () => Promise.reject(new Error('Failed to fetch')),
    );

    await expect(client.getParams()).rejects.toMatchObject({
      status: 0,
      message: 'GET http://127.0.0.1:8080/api/params did not get through: Failed to fetch',
    });
  });
});

describe('ParamsClient.putParams — PUT /api/params {token, params}', () => {
  it('成功で GET と同じ形を返す。body は {token, params}', async () => {
    const { fetchFn, calls } = fakeFetch([{ status: 200, payload: PARAMS_INFO }]);
    const client = new ParamsClient(() => '/api', fetchFn);

    const result = await client.putParams('tok-abc', PARAMS_JSON);

    expect(result).toEqual(PARAMS_INFO);
    expect(calls[0].url).toBe('/api/params');
    expect(calls[0].init?.method).toBe('PUT');
    expect(calls[0].init?.headers).toEqual({ 'content-type': 'application/json' });
    expect(JSON.parse(String(calls[0].init?.body))).toEqual({
      token: 'tok-abc',
      params: PARAMS_JSON,
    });
  });

  it('400(スキーマ違反)は ParamsError の文言をそのまま', async () => {
    const { fetchFn } = fakeFetch([
      {
        status: 400,
        payload: {
          error: 'ノブ gain の値 7pC は許された値ではない(許容: 10pC, 120fC, 1pC, 240fC)',
        },
      },
    ]);
    const client = new ParamsClient(() => '/api', fetchFn);

    await expect(client.putParams('tok-abc', PARAMS_JSON)).rejects.toMatchObject({
      status: 400,
      message: 'ノブ gain の値 7pC は許された値ではない(許容: 10pC, 120fC, 1pC, 240fC)',
    });
  });

  it('401(token 不正)', async () => {
    const { fetchFn } = fakeFetch([{ status: 401, payload: { error: 'stale or unknown token' } }]);
    const client = new ParamsClient(() => '/api', fetchFn);

    await expect(client.putParams('bad', PARAMS_JSON)).rejects.toMatchObject({
      status: 401,
      message: 'stale or unknown token',
    });
  });

  it('409(Running 中)', async () => {
    const { fetchFn } = fakeFetch([
      { status: 409, payload: { error: 'controller is Running — not Idle' } },
    ]);
    const client = new ParamsClient(() => '/api', fetchFn);

    await expect(client.putParams('tok-abc', PARAMS_JSON)).rejects.toMatchObject({
      status: 409,
      message: 'controller is Running — not Idle',
    });
  });

  it('503(ecc_config_dir 未設定)', async () => {
    const { fetchFn } = fakeFetch([
      { status: 503, payload: { error: 'ecc_config_dir is not configured' } },
    ]);
    const client = new ParamsClient(() => '/api', fetchFn);

    await expect(client.putParams('tok-abc', PARAMS_JSON)).rejects.toMatchObject({
      status: 503,
      message: 'ecc_config_dir is not configured',
    });
  });

  it('投げるのは ParamsApiError のインスタンス', async () => {
    const { fetchFn } = fakeFetch([{ status: 400, payload: { error: 'bad' } }]);
    const client = new ParamsClient(() => '/api', fetchFn);

    await expect(client.putParams('tok-abc', PARAMS_JSON)).rejects.toBeInstanceOf(ParamsApiError);
  });
});

describe('ParamsClient.renderParams — GET /api/params/render(dry run)', () => {
  it('template 省略時は current.json の template を使う(クエリ無し)', async () => {
    const { fetchFn, calls } = fakeFetch([
      { status: 200, payload: { id: 'x__1', exceptions: 0, configure_xcfg: '<Setup/>' } },
    ]);
    const client = new ParamsClient(() => '/api', fetchFn);

    const result = await client.renderParams();

    expect(calls[0].url).toBe('/api/params/render');
    expect(calls[0].init?.method).toBe('GET');
    expect(result).toEqual({ id: 'x__1', exceptions: 0, configure_xcfg: '<Setup/>' });
  });

  it('template を指定するとクエリに乗る(URL エンコード込み)', async () => {
    const { fetchFn, calls } = fakeFetch([
      { status: 200, payload: { id: 'x__1', exceptions: 0, configure_xcfg: '<Setup/>' } },
    ]);
    const client = new ParamsClient(() => '/api', fetchFn);

    await client.renderParams('configure physics.xcfg');

    expect(calls[0].url).toBe('/api/params/render?template=configure%20physics.xcfg');
  });

  it('400(dry run の入力が原因)はそのまま投げる', async () => {
    const { fetchFn } = fakeFetch([{ status: 400, payload: { error: 'template が空' } }]);
    const client = new ParamsClient(() => '/api', fetchFn);

    await expect(client.renderParams()).rejects.toMatchObject({
      status: 400,
      message: 'template が空',
    });
  });

  it('本文が JSON でない失敗は status とテキストで語る(fallback)', async () => {
    const fetchFn = (): Promise<Response> =>
      Promise.resolve({
        ok: false,
        status: 404,
        statusText: 'Not Found',
        json: () => Promise.reject(new Error('not json')),
      } as unknown as Response);
    const client = new ParamsClient(() => '/api', fetchFn);

    await expect(client.renderParams()).rejects.toMatchObject({
      status: 404,
      message: 'GET /api/params/render → HTTP 404 Not Found',
    });
  });
});
