/**
 * 接続先の決定規則(TODO/027 の確定設計、SPEC §3.2)。
 *
 * - **WS 既定** = `ws://{ページの hostname}:9000/ws`(ページが https なら `wss://`)。
 *   パスは `/ws` 固定(026 申し送り)。
 * - **REST 既定** = same-origin の `/api/...`(本番は controller が UI を配信する)。
 * - **上書き** = same-origin の `ui-config.json`(運用配置物。リポにはコミットしない)。
 *   404 / パース失敗 / 型違いは既定へフォールバックし `console.info` を 1 回出す。
 */

/** SPEC §3.2: monitor WS。 */
export const MONITOR_WS_PORT = 9000;
/** 026 申し送り: WS のパスは `/ws` に固定。 */
export const WS_PATH = '/ws';
/** SPEC §8.1 の REST は same-origin の `/api/...`。 */
export const DEFAULT_API_BASE = '/api';
/** 上書きファイル(same-origin)。 */
export const UI_CONFIG_URL = 'ui-config.json';

export interface Endpoints {
  readonly wsUrl: string;
  readonly apiBase: string;
}

/** `window.location` のうち、決定規則が使う分だけ。 */
export interface PageLocation {
  readonly protocol: string;
  readonly hostname: string;
}

/** `fetch` の最小形(テストから差し替えられるように狭く取る)。 */
export type FetchLike = (url: string) => Promise<{
  readonly ok: boolean;
  readonly status: number;
  text(): Promise<string>;
}>;

export interface ResolvedEndpoints {
  readonly endpoints: Endpoints;
  /** どこから来た値か(起動ログと画面に出す)。 */
  readonly source: 'defaults' | typeof UI_CONFIG_URL;
}

/** ページの URL だけから決まる既定。 */
export function defaultEndpoints(location: PageLocation): Endpoints {
  const scheme = location.protocol === 'https:' ? 'wss:' : 'ws:';
  return {
    wsUrl: `${scheme}//${location.hostname}:${MONITOR_WS_PORT}${WS_PATH}`,
    apiBase: DEFAULT_API_BASE,
  };
}

function stringField(object: Record<string, unknown>, key: string): string | null {
  const value = object[key];
  return typeof value === 'string' && value.length > 0 ? value : null;
}

/**
 * `ui-config.json` を読んで既定に重ねる。**投げない**。
 *
 * 既定へ落ちたときは理由つきで `console.info` を 1 回出す(silent failure 禁止)。
 */
export async function loadEndpoints(
  location: PageLocation,
  fetchFn: FetchLike,
): Promise<ResolvedEndpoints> {
  const defaults = defaultEndpoints(location);

  let text: string;
  try {
    const response = await fetchFn(UI_CONFIG_URL);
    if (!response.ok) {
      return fallback(defaults, `${UI_CONFIG_URL} → HTTP ${response.status}`);
    }
    text = await response.text();
  } catch (error) {
    return fallback(defaults, `${UI_CONFIG_URL} → ${String(error)}`);
  }

  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch (error) {
    return fallback(defaults, `${UI_CONFIG_URL} is not JSON: ${String(error)}`);
  }
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) {
    return fallback(defaults, `${UI_CONFIG_URL} is not an object`);
  }

  const object = raw as Record<string, unknown>;
  const wsUrl = stringField(object, 'wsUrl');
  const apiBase = stringField(object, 'apiBase');
  if (wsUrl === null && apiBase === null) {
    return fallback(defaults, `${UI_CONFIG_URL} has no usable wsUrl/apiBase`);
  }

  return {
    endpoints: { wsUrl: wsUrl ?? defaults.wsUrl, apiBase: apiBase ?? defaults.apiBase },
    source: UI_CONFIG_URL,
  };
}

function fallback(defaults: Endpoints, why: string): ResolvedEndpoints {
  console.info(`tpcdaq: using the default endpoints (${why})`, defaults);
  return { endpoints: defaults, source: 'defaults' };
}
