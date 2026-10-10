/**
 * パラメータ JSON REST(SPEC v1.33 §3.3 / §8.1、TODO/078 レーン B の結果節が正)のクライアント。
 *
 * - `GET /api/params` — 閲覧系(認証なし)。無ければ 404(= `null` で返す)。
 * - `PUT /api/params {token, params}` — 操作系。400(スキーマ違反)/ 401(token)/
 *   409(Running 中)/ 503(`ecc_config_dir` 未設定)は **サーバの `error` 文言をそのまま**
 *   `ParamsApiError` として投げる(041 D-1 の「明瞭なエラー」を殺さない)。
 * - `GET /api/params/render?template=<name>` — dry run(閲覧系)。**書かない**。
 * - `GET /api/params/templates` / `GET /api/params/defaults?template=<name>` — テンプレート
 *   xcfg の一覧と、そこから抽出した初期値(閲覧系、TODO/111 §5.0 の契約が正)。
 *
 * `controller-api.ts` / `run-control-api.ts` と同じ流儀: `fetch` を直に使い、`FetchLike` で
 * 差し替える(`provideHttpClient` を増やさない = KISS)。
 */
import { Injectable, inject } from '@angular/core';

import { describeError } from '../logbook/logbook-feed';
import { EndpointsService } from '../ws/endpoints.service';
import { apiUrl } from './controller-api';

export type FetchLike = (url: string, init?: RequestInit) => Promise<Response>;

/** 例外の宛先(`src/params/schema.rs::At` の写し。接頭辞のみ、空も不可 — サーバが検証する)。 */
export interface ExceptionAt {
  readonly asad?: number;
  readonly aget?: number;
  readonly ch?: number;
}

/** 例外 1 件(`src/params/schema.rs::Exception` の写し)。 */
export interface ExceptionRow {
  readonly at: ExceptionAt;
  readonly knob: string;
  readonly value: unknown;
  readonly note?: string;
}

/** パラメータ JSON 本体(`tpcdaq-params/1`、`src/params/schema.rs::Params` の写し)。 */
export interface ParamsJson {
  readonly schema: string;
  readonly template: string;
  readonly knobs: Record<string, unknown>;
  readonly exceptions: readonly ExceptionRow[];
  readonly note?: string;
}

export interface ParamsFiles {
  readonly configure: string;
  readonly prepare: string;
}

/** `GET`/`PUT /api/params` の応答(078 結果節 §10 の JSON 例が正)。 */
export interface ParamsInfo {
  readonly id: string;
  readonly sha256: string;
  readonly exceptions: number;
  readonly params: ParamsJson;
  readonly files: ParamsFiles;
}

/** `GET /api/params/render` の応答(dry run、書かない)。 */
export interface ParamsRenderResult {
  readonly id: string;
  readonly exceptions: number;
  readonly configure_xcfg: string;
}

/** `defaults` の `unsupported` 1 件。`found` は xcfg のテキストそのまま。 */
export interface ParamsUnsupported {
  readonly knob: string;
  readonly found: string;
  readonly why: string;
}

/**
 * `GET /api/params/defaults` の応答(TODO/111 §5.0)。
 *
 * - `knobs` = テンプレートから読めて型・範囲に収まった葉だけ(`tpcdaq-params/1` の `knobs` と同形)。
 * - `unset` = テンプレートに要素が無い葉(`.` 区切り)。黙って既定値を入れない。
 * - `unsupported` = 要素はあるが型・範囲外 / 合成ノブが写らないもの。
 */
export interface ParamsDefaults {
  readonly template: string;
  readonly knobs: Record<string, unknown>;
  readonly exceptions: readonly ExceptionRow[];
  readonly unset: readonly string[];
  readonly unsupported: readonly ParamsUnsupported[];
}

/** HTTP エラーをそのまま持ち回る(サーバの `error` 文言を加工しない)。 */
export class ParamsApiError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message);
    this.name = 'ParamsApiError';
  }
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** 応答の `error` フィールドを**そのまま**(無ければ fallback)。 */
function errorMessage(payload: unknown, fallback: string): string {
  if (isObject(payload) && typeof payload['error'] === 'string' && payload['error'] !== '') {
    return payload['error'];
  }
  return fallback;
}

async function readJson(response: Response): Promise<unknown> {
  try {
    return await response.json();
  } catch {
    return null; // 本文が JSON でない(プロキシの 404 HTML 等)。status だけで語る。
  }
}

/**
 * Angular 非依存の本体(テストは `fetch` を差し替えてここを叩く。`run-control-api.ts` の
 * `RunControlClient` と同じ形)。
 */
export class ParamsClient {
  constructor(
    private readonly apiBase: () => string,
    private readonly fetchFn: FetchLike,
  ) {}

  /** `GET /api/params`。無ければ `null`(404)。それ以外の失敗は `ParamsApiError` を投げる。 */
  async getParams(): Promise<ParamsInfo | null> {
    const url = apiUrl(this.apiBase(), 'params');
    const response = await this.send(url, 'GET');
    if (response.status === 404) return null;
    return this.parseOrThrow<ParamsInfo>(url, 'GET', response);
  }

  /** `PUT /api/params {token, params}`。成功で `GET` と同じ形。失敗は文言つきで投げる。 */
  async putParams(token: string, params: ParamsJson): Promise<ParamsInfo> {
    const url = apiUrl(this.apiBase(), 'params');
    const response = await this.send(url, 'PUT', { token, params });
    return this.parseOrThrow<ParamsInfo>(url, 'PUT', response);
  }

  /** `GET /api/params/render?template=<name>`(省略時 = current.json の template)。**書かない**。 */
  async renderParams(template?: string): Promise<ParamsRenderResult> {
    const path =
      template === undefined || template === ''
        ? 'params/render'
        : `params/render?template=${encodeURIComponent(template)}`;
    const url = apiUrl(this.apiBase(), path);
    const response = await this.send(url, 'GET');
    return this.parseOrThrow<ParamsRenderResult>(url, 'GET', response);
  }

  /** `GET /api/params/templates` — `ecc_config_dir` 直下の `configure-*.xcfg` のファイル名(辞書順)。 */
  async getTemplates(): Promise<string[]> {
    const url = apiUrl(this.apiBase(), 'params/templates');
    const response = await this.send(url, 'GET');
    const payload = await this.parseOrThrow<{ templates?: unknown }>(url, 'GET', response);
    const list = payload?.templates;
    if (!Array.isArray(list) || !list.every((name) => typeof name === 'string')) {
      // 200 なのに契約の形でない(古いサーバ等)。空配列にして黙らせない。
      throw new ParamsApiError(
        response.status,
        `GET ${url} returned an unexpected body (expected {"templates": [string…]})`,
      );
    }
    return list as string[];
  }

  /** `GET /api/params/defaults?template=<name>` — テンプレート xcfg から抽出した初期値。**書かない**。 */
  async getDefaults(template: string): Promise<ParamsDefaults> {
    const url = apiUrl(this.apiBase(), `params/defaults?template=${encodeURIComponent(template)}`);
    const response = await this.send(url, 'GET');
    return this.parseOrThrow<ParamsDefaults>(url, 'GET', response);
  }

  private async send(
    url: string,
    method: string,
    body?: Record<string, unknown>,
  ): Promise<Response> {
    try {
      return await this.fetchFn(url, {
        method,
        headers: body === undefined ? undefined : { 'content-type': 'application/json' },
        body: body === undefined ? undefined : JSON.stringify(body),
      });
    } catch (error) {
      throw new ParamsApiError(0, `${method} ${url} did not get through: ${describeError(error)}`);
    }
  }

  private async parseOrThrow<T>(url: string, method: string, response: Response): Promise<T> {
    const payload = await readJson(response);
    if (!response.ok) {
      throw new ParamsApiError(
        response.status,
        errorMessage(payload, `${method} ${url} → HTTP ${response.status} ${response.statusText}`),
      );
    }
    return payload as T;
  }
}

@Injectable({ providedIn: 'root' })
export class ParamsApiService extends ParamsClient {
  constructor() {
    const endpoints = inject(EndpointsService);
    super(
      () => endpoints.apiBase(),
      (url, init) => fetch(url, init),
    );
  }
}
