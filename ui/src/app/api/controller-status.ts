/**
 * SPEC §8.1 `GET /api/status` の応答 → 画面用の形(純ロジック、Angular 非依存)。
 *
 * `GET /api/status` は**閲覧系なので認証なし**。UI から呼んでよい唯一の controller 状態取得。
 *
 * この層の仕事は「**取れていないものを取れたことにしない**」こと(CLAUDE.md):
 * - ecc-bridge が居ない開発機では `ecc.state = "Unknown"` が返る。これは**「不明」**であって
 *   「停止中」でも「異常」でもない。色も文言も別にする。
 * - REP に応答しないコンポーネントは `state: null` で来る。`Idle` に丸めず「不明」と書く。
 */

export type EccTone = 'ok' | 'unknown' | 'error';

export interface EccView {
  /** 生の状態文字列(`Off/Idle/Described/Prepared/Ready/Running/Paused/Unknown`、SPEC §8.2)。 */
  readonly state: string;
  /** 画面の文言(`Unknown` → 「不明」)。 */
  readonly label: string;
  readonly tone: EccTone;
  readonly error: string | null;
  /**
   * GET の error フラグ(SPEC §8.2 v1.14、TODO/043/051)。**`NO_ERR`(=正常)と欠落は
   * 出さない** —— 見せる価値があるのは異常フラグが立っているときだけ。
   */
  readonly eccError: string | null;
}

export interface ComponentView {
  readonly name: string;
  readonly endpoint: string;
  readonly reachable: boolean;
  /** 届かなければ「不明」。 */
  readonly state: string;
  readonly runNumber: number | null;
  readonly error: string | null;
}

export interface ActiveRunView {
  readonly run: number;
  readonly operator: string;
  readonly comment: string;
  readonly startedTs: string;
  readonly elapsedS: number;
}

export interface LogPostsView {
  readonly accepted: number;
  readonly rejected: number;
}

export interface StatusView {
  readonly phase: string;
  /** 実行中でなければ `null`(0 番の run を捏造しない)。 */
  readonly run: ActiveRunView | null;
  readonly operator: string | null;
  readonly components: readonly ComponentView[];
  readonly ecc: EccView;
  readonly logPosts: LogPostsView | null;
  readonly notes: readonly string[];
}

/** 状態が取れていないことを表す表示文字列。 */
export const UNKNOWN_LABEL = 'unknown';

type Raw = Record<string, unknown>;

function isObject(value: unknown): value is Raw {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function str(raw: Raw, key: string): string | null {
  const value = raw[key];
  return typeof value === 'string' ? value : null;
}

function num(raw: Raw, key: string): number | null {
  const value = raw[key];
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

/**
 * `Unknown` は**不明**(ECC に届いていない)。届いているのに `ok=false` はエラー。
 * この 2 つを同じ赤で出すと、開発機の「ecc-bridge を上げていないだけ」が
 * 「ECC が壊れている」に見える(= 赤い嘘)。
 */
export function eccTone(ok: boolean, state: string): EccTone {
  if (state === 'Unknown') return 'unknown';
  return ok ? 'ok' : 'error';
}

/**
 * GET の error フラグ(SPEC §8.2 v1.14)を「表示に値するときだけ」返す。
 * `NO_ERR`(正常)・空文字・欠落(古い ecc-bridge)は `null`(= 出さない)。
 */
function eccErrorFlag(raw: Raw): string | null {
  const value = str(raw, 'ecc_error');
  if (value === null || value === '' || value === 'NO_ERR') return null;
  return value;
}

function parseEcc(raw: unknown): EccView {
  const source = isObject(raw) ? raw : {};
  const state = str(source, 'state') ?? 'Unknown';
  return {
    state,
    label: state === 'Unknown' ? UNKNOWN_LABEL : state,
    tone: eccTone(source['ok'] === true, state),
    error: str(source, 'error'),
    // `state: "Unknown"`(ブリッジ不達)のとき controller が注入する `ecc_error: "Unknown"`
    // は GET が申告した実フラグではなく不達センチネル(SPEC §8.2 v1.14)。既存の不達表示
    // (`不明` ラベル + tone `unknown`)に一本化し、ここでは二重に出さない。
    eccError: state === 'Unknown' ? null : eccErrorFlag(source),
  };
}

function parseComponent(raw: unknown): ComponentView | null {
  if (!isObject(raw)) return null;
  return {
    name: str(raw, 'name') ?? '—',
    endpoint: str(raw, 'endpoint') ?? '—',
    reachable: raw['reachable'] === true,
    state: str(raw, 'state') ?? UNKNOWN_LABEL,
    runNumber: num(raw, 'run_number'),
    error: str(raw, 'error'),
  };
}

function parseRun(raw: unknown): ActiveRunView | null {
  if (!isObject(raw)) return null;
  const run = num(raw, 'run');
  if (run === null) return null;
  return {
    run,
    operator: str(raw, 'operator') ?? '—',
    comment: str(raw, 'comment') ?? '',
    startedTs: str(raw, 'started_ts') ?? '',
    elapsedS: num(raw, 'elapsed_s') ?? 0,
  };
}

function parseLogPosts(raw: unknown): LogPostsView | null {
  if (!isObject(raw)) return null;
  const accepted = num(raw, 'accepted');
  const rejected = num(raw, 'rejected');
  if (accepted === null || rejected === null) return null;
  return { accepted, rejected };
}

/** 応答 1 通を画面用に畳む。**投げない**。オブジェクトでなければ `null`(= 取得できていない)。 */
export function parseControllerStatus(raw: unknown): StatusView | null {
  if (!isObject(raw)) return null;
  const components = Array.isArray(raw['components'])
    ? (raw['components'] as unknown[])
        .map(parseComponent)
        .filter((component): component is ComponentView => component !== null)
    : [];
  const notes = Array.isArray(raw['notes'])
    ? (raw['notes'] as unknown[]).map((note) => (typeof note === 'string' ? note : String(note)))
    : [];

  return {
    phase: str(raw, 'phase') ?? UNKNOWN_LABEL,
    run: parseRun(raw['run']),
    operator: str(raw, 'operator'),
    components,
    ecc: parseEcc(raw['ecc']),
    logPosts: parseLogPosts(raw['log_posts']),
    notes,
  };
}
