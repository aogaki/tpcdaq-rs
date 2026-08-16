/**
 * **操作系** REST(SPEC §8.1)のクライアント。閲覧系(`controller-api.ts`)とは分離する
 * —— 認証(token)と監査の有無が違うので、同じサービスに混ぜない(050-A)。
 *
 * ここが受け持つのは 12 本(`run/run-actions.ts` の表と 1:1):
 *
 * - `POST /api/control/acquire {operator, passphrase}` → `{token, preempted}`(横取りは常に可)
 * - `POST /api/control/release {token}` → `{released: true}`
 * - `POST /api/run/start {token, comment?}` / `stop {token}` / `next {token, next_run}`
 * - `POST /api/ecc/{describe|prepare|configure|start|stop|breakup|reset} {token}`
 *
 * # 掟
 *
 * - **エラーは加工しない**。controller の失敗応答は一律 `{"error": "..."}` なので、その文字列を
 *   **そのまま**返す(041 D-1 の「明瞭なエラー」を殺さない)。届かなかった時だけは応答が無いので
 *   こちらで文言を作る(silent にしないため)。
 * - **HTTP 200 でも `ok: false` は失敗**。`run/stop` は異常終了でも 200 + `{ok:false, reason,
 *   notes}` を返し、`ecc/*` は実 ECC の Ignored/Denied を 200 + `{ok:false, error}` で返す。
 *   これを成功として出すと嘘になる(CLAUDE.md「silent failure を作らない」)。
 * - token は**メモリ + sessionStorage**(タブを閉じるまで。リロードで消えない)。
 * - `fetch` を直に使う(`controller-api.ts` と同じ流儀。`provideHttpClient` を増やさない = KISS)。
 *   **投げない** —— 失敗も `ActionResult` として返し、呼び手が必ず画面に出せるようにする。
 */
import { Injectable, inject, signal } from '@angular/core';

import { describeError } from '../logbook/logbook-feed';
import { RUN_ACTIONS, type RunAction } from '../run/run-actions';
import { EndpointsService } from '../ws/endpoints.service';
import { apiUrl } from './controller-api';

/** sessionStorage のキー(タブ単位。localStorage にはしない = 別タブに token を配らない)。 */
export const TOKEN_STORAGE_KEY = 'tpcdaq.control.token';

export type FetchLike = (url: string, init?: RequestInit) => Promise<Response>;

/** sessionStorage の必要な部分だけ(テストは差し替える)。 */
export interface TokenStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

/** 画面の入力欄(token 以外)。 */
export interface ActionInput {
  readonly operator: string;
  readonly passphrase: string;
  /** run/start の任意コメント(空なら body に載せない)。 */
  readonly comment: string;
  /** run/next の入力(**生の文字列**。正整数かはここで検査する)。 */
  readonly nextRun: string;
}

export const EMPTY_INPUT: ActionInput = { operator: '', passphrase: '', comment: '', nextRun: '' };

/** 組み立てた要求(`path` は `apiBase` からの相対 —— `apiUrl()` で URL になる)。 */
export type RequestPlan =
  | { readonly ok: true; readonly path: string; readonly body: Record<string, unknown> }
  | { readonly ok: false; readonly error: string };

export interface ActionResult {
  readonly ok: boolean;
  /** HTTP ステータス。**0 = 応答そのものが無い**(controller に届かなかった)。 */
  readonly status: number;
  /** 応答の `error` を**そのまま**(無ければ null)。 */
  readonly error: string | null;
  /** 応答の `notes`(`run/stop`)。 */
  readonly notes: readonly string[];
  /** 成功時に見せる 1 行(`token` は伏せる)。 */
  readonly summary: string;
  /** 応答本文(呼び手が `run` 番号などを見たいとき用)。 */
  readonly body: Record<string, unknown> | null;
}

/** 表の `endpoint`(`/api/...` の絶対パス)→ `apiBase` からの相対パス。 */
export function actionPath(endpoint: string): string {
  return endpoint.replace(/^\/api\//, '');
}

/** 正整数(u32 の範囲)だけを通す。`0` / 負 / 小数 / 空 / 文字は null。 */
export function parsePositiveInt(text: string): number | null {
  const trimmed = text.trim();
  if (!/^\d+$/.test(trimmed)) return null;
  const value = Number(trimmed);
  if (value <= 0 || value > 4294967295) return null;
  return value;
}

/**
 * 表 1 行 + 入力 → 送る要求。**送れないときは送らずに理由を返す**
 * (token 無し / next_run が正整数でない。controller も同じ判定で 401/400 を返すが、
 * 画面で分かる誤りをわざわざ往復させない)。
 */
export function buildActionRequest(
  action: RunAction,
  input: ActionInput,
  token: string | null,
): RequestPlan {
  const path = actionPath(action.endpoint);

  if (action.endpoint === '/api/control/acquire') {
    return { ok: true, path, body: { operator: input.operator, passphrase: input.passphrase } };
  }

  if (token === null || token === '') {
    return { ok: false, error: 'No control token (press Acquire first).' };
  }

  if (action.endpoint === '/api/run/start') {
    const comment = input.comment.trim();
    return { ok: true, path, body: comment === '' ? { token } : { token, comment } };
  }

  if (action.endpoint === '/api/run/next') {
    const nextRun = parsePositiveInt(input.nextRun);
    if (nextRun === null) {
      return { ok: false, error: `next_run must be a positive integer (got "${input.nextRun}").` };
    }
    return { ok: true, path, body: { token, next_run: nextRun } };
  }

  return { ok: true, path, body: { token } };
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** 成功の 1 行。`token` は出さない(画面のログに credential を残さない)。 */
function summarize(body: Record<string, unknown> | null): string {
  if (body === null) return '';
  const shown = Object.fromEntries(
    Object.entries(body).filter(([key]) => key !== 'token' && key !== 'notes'),
  );
  const text = JSON.stringify(shown);
  return text === '{}' ? '' : text;
}

/**
 * 応答 1 通 → 画面に出せる形。**HTTP 200 でも `ok: false` は失敗**(run/stop の異常終了、
 * 実 ECC の Ignored/Denied)。`error` は加工しない。
 */
export function interpretResponse(status: number, payload: unknown): ActionResult {
  const body = isObject(payload) ? payload : null;
  const httpOk = status >= 200 && status < 300;
  const bodyOk = body === null || body['ok'] !== false;
  const rawError = body === null ? null : body['error'];
  const error = typeof rawError === 'string' && rawError !== '' ? rawError : null;
  const notes =
    body !== null && Array.isArray(body['notes'])
      ? (body['notes'] as unknown[]).map((note) => (typeof note === 'string' ? note : String(note)))
      : [];
  return { ok: httpOk && bodyOk, status, error, notes, summary: summarize(body), body };
}

/**
 * Angular 非依存の本体(テストは fetch と storage を差し替えてここを叩く)。
 * `ws/endpoints.ts` + `EndpointsService` と同じ「純ロジック + 薄い DI 包み」の流儀。
 */
export class RunControlClient {
  /** 保持中の token(メモリ)。UI は有無だけを見る。 */
  readonly token = signal<string | null>(null);
  /** 直近の acquire で横取りした相手(§8.1: 横取りは常に可・監査ログに残る)。 */
  readonly preempted = signal<string | null>(null);

  constructor(
    private readonly apiBase: () => string,
    private readonly fetchFn: FetchLike,
    private readonly storage: TokenStorage | null,
  ) {
    const saved = storage?.getItem(TOKEN_STORAGE_KEY) ?? null;
    if (saved !== null && saved !== '') this.token.set(saved);
  }

  /** 表の 1 行を実行する。**投げない**。 */
  async execute(action: RunAction, input: ActionInput): Promise<ActionResult> {
    const plan = buildActionRequest(action, input, this.token());
    if (!plan.ok) {
      return { ok: false, status: 0, error: plan.error, notes: [], summary: '', body: null };
    }

    const result = await this.send(plan.path, plan.body);

    if (action.endpoint === '/api/control/acquire' && result.ok) {
      const token = result.body?.['token'];
      if (typeof token !== 'string' || token === '') {
        // 200 なのに token が無い = 何が起きたか分からない。成功として飲み込まない。
        return { ...result, ok: false, error: 'The acquire response carries no token' };
      }
      this.setToken(token);
      const preempted = result.body?.['preempted'];
      this.preempted.set(typeof preempted === 'string' ? preempted : null);
    }

    if (action.endpoint === '/api/control/release' && result.ok) {
      this.setToken(null);
      this.preempted.set(null);
    }

    return result;
  }

  /** token を捨てる(release 成功時。UI から明示的に忘れたいときにも使える)。 */
  setToken(token: string | null): void {
    this.token.set(token);
    if (this.storage === null) return;
    if (token === null) this.storage.removeItem(TOKEN_STORAGE_KEY);
    else this.storage.setItem(TOKEN_STORAGE_KEY, token);
  }

  private async send(path: string, body: Record<string, unknown>): Promise<ActionResult> {
    const url = apiUrl(this.apiBase(), path);
    let response: Response;
    try {
      response = await this.fetchFn(url, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(body),
      });
    } catch (error) {
      return {
        ok: false,
        status: 0,
        error: `POST ${url} did not get through: ${describeError(error)}`,
        notes: [],
        summary: '',
        body: null,
      };
    }

    let payload: unknown = null;
    try {
      payload = await response.json();
    } catch {
      payload = null; // 本文が JSON でない(プロキシの 404 HTML 等)。status だけで語る。
    }
    return interpretResponse(response.status, payload);
  }
}

/** sessionStorage が使えない環境(private mode 等)では null を返す。 */
export function sessionTokenStorage(): TokenStorage | null {
  try {
    return globalThis.sessionStorage ?? null;
  } catch {
    return null;
  }
}

@Injectable({ providedIn: 'root' })
export class RunControlApiService extends RunControlClient {
  constructor() {
    const endpoints = inject(EndpointsService);
    super(
      () => endpoints.apiBase(),
      (url, init) => fetch(url, init),
      sessionTokenStorage(),
    );
  }
}

/** 表と実装のずれを見つけるためのヘルパ(テスト専用ではないが使うのはテスト)。 */
export function actionByEndpoint(endpoint: string): RunAction {
  const action = RUN_ACTIONS.find((candidate) => candidate.endpoint === endpoint);
  if (action === undefined) throw new Error(`endpoint not in the action table: ${endpoint}`);
  return action;
}
