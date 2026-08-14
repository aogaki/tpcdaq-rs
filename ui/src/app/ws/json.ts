/**
 * SPEC §10.3 — WS の JSON テキストメッセージ(S→C の `meta` / `status` / `run`、
 * C→S の `subscribe`)。Angular に依存しない純 TS。
 *
 * **casing は SPEC の文言どおり**: `status` の本体は §5.3 の status をそのまま出すので
 * snake_case、monitor が足す 3 つだけ camelCase。`meta` / `run` は camelCase。
 * ここを勝手に正規化すると、Rust 側とのズレが「常に 0」という見え方で隠れる。
 *
 * 未知キー・未知 type は**捨てずに数える**(前方互換 + silent 禁止)。
 */

/** 面毎の飽和カウンタ(SPEC §5.3 の `saturation`)。 */
export interface Saturation {
  readonly saturated: number;
  readonly counted: number;
}

/** SPEC §10.3 `status` = §5.3 の 11 キー + monitor の 3 キー。 */
export interface StatusMessage {
  readonly type: 'status';
  readonly run: number;
  readonly state: string;
  readonly events_built: number;
  readonly events_incomplete: number;
  readonly late_fragments: number;
  readonly pending_events: number;
  readonly frames_per_cobo: Readonly<Record<string, number>>;
  readonly bytes_written: number;
  readonly saturation: Readonly<Record<string, Saturation>>;
  readonly publish_drops: number;
  /** monitor が足す(camelCase): root-sink → monitor の取りこぼし数。 */
  readonly monitorGaps: number;
  /** monitor が足す(camelCase): WS 接続クライアント数。 */
  readonly clients: number;
  /** monitor が足す(camelCase): live キューの drop-oldest 数。 */
  readonly wsDropped: number;
}

/** SPEC §10.3 `meta`(接続時・run 変化時)。 */
export interface MetaMessage {
  readonly type: 'meta';
  readonly nBuckets: number;
  readonly planes: { readonly U: number; readonly V: number; readonly W: number };
  readonly geometry: string;
  readonly anglesDeg: readonly number[] | null;
  readonly detector: string;
  readonly cobos: readonly number[];
  readonly run: number;
}

/** SPEC §10.3 `run`(state 遷移時)。 */
export interface RunMessage {
  readonly type: 'run';
  readonly state: string;
  readonly run: number;
  readonly ts: string;
}

export type JsonParseErrorReason = 'not-json' | 'not-object' | 'missing-type' | 'bad-field';

export type JsonParseResult =
  | { readonly kind: 'meta'; readonly meta: MetaMessage; readonly unknownKeys: readonly string[] }
  | {
      readonly kind: 'status';
      readonly status: StatusMessage;
      readonly unknownKeys: readonly string[];
    }
  | { readonly kind: 'run'; readonly run: RunMessage; readonly unknownKeys: readonly string[] }
  | { readonly kind: 'unknown-type'; readonly type: string }
  | { readonly kind: 'error'; readonly reason: JsonParseErrorReason; readonly detail: string };

function fail(reason: JsonParseErrorReason, detail: string): JsonParseResult {
  return { kind: 'error', reason, detail };
}

/** 既知キー以外を集める(捨てないで数えるため)。 */
function unknownKeysOf(object: Record<string, unknown>, known: readonly string[]): string[] {
  return Object.keys(object).filter((key) => !known.includes(key));
}

/** 型が違えば `null`(呼び手が bad-field にする)。 */
function numberAt(object: Record<string, unknown>, key: string): number | null {
  const value = object[key];
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

function stringAt(object: Record<string, unknown>, key: string): string | null {
  const value = object[key];
  return typeof value === 'string' ? value : null;
}

function objectAt(object: Record<string, unknown>, key: string): Record<string, unknown> | null {
  const value = object[key];
  return typeof value === 'object' && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

/** `{cobo: u64}` のような数値マップ。1 つでも数値でなければ `null`。 */
function numberMapAt(object: Record<string, unknown>, key: string): Record<string, number> | null {
  const raw = objectAt(object, key);
  if (raw === null) return null;
  const out: Record<string, number> = {};
  for (const [name, value] of Object.entries(raw)) {
    if (typeof value !== 'number' || !Number.isFinite(value)) return null;
    out[name] = value;
  }
  return out;
}

function numberArrayAt(object: Record<string, unknown>, key: string): number[] | null {
  const value = object[key];
  if (!Array.isArray(value)) return null;
  const out: number[] = [];
  for (const item of value) {
    if (typeof item !== 'number' || !Number.isFinite(item)) return null;
    out.push(item);
  }
  return out;
}

/** `saturation: {"U": {saturated, counted}, ...}`。 */
function saturationAt(object: Record<string, unknown>): Record<string, Saturation> | null {
  const raw = objectAt(object, 'saturation');
  if (raw === null) return null;
  const out: Record<string, Saturation> = {};
  for (const [plane, value] of Object.entries(raw)) {
    if (typeof value !== 'object' || value === null) return null;
    const entry = value as Record<string, unknown>;
    const saturated = numberAt(entry, 'saturated');
    const counted = numberAt(entry, 'counted');
    if (saturated === null || counted === null) return null;
    out[plane] = { saturated, counted };
  }
  return out;
}

const META_KEYS = [
  'type',
  'nBuckets',
  'planes',
  'geometry',
  'anglesDeg',
  'detector',
  'cobos',
  'run',
] as const;

// `kind` は monitor が §5.3 の StatusPayload をそのまま直列化するために載る
// (`type` は monitor が足したもの)。既知キー扱いにして雑音にしない。
const STATUS_KEYS = [
  'type',
  'kind',
  'run',
  'state',
  'events_built',
  'events_incomplete',
  'late_fragments',
  'pending_events',
  'frames_per_cobo',
  'bytes_written',
  'saturation',
  'publish_drops',
  'monitorGaps',
  'clients',
  'wsDropped',
] as const;

const RUN_KEYS = ['type', 'state', 'run', 'ts'] as const;

/**
 * WS のテキストフレーム 1 通を解釈する。**例外は投げない**。
 */
export function parseJsonMessage(text: string): JsonParseResult {
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch (error) {
    return fail('not-json', String(error));
  }
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) {
    return fail('not-object', `top level is ${Array.isArray(raw) ? 'an array' : typeof raw}`);
  }
  const object = raw as Record<string, unknown>;
  const type = stringAt(object, 'type');
  if (type === null) return fail('missing-type', 'no string `type` key');

  switch (type) {
    case 'meta':
      return parseMeta(object);
    case 'status':
      return parseStatus(object);
    case 'run':
      return parseRun(object);
    default:
      return { kind: 'unknown-type', type };
  }
}

function parseMeta(object: Record<string, unknown>): JsonParseResult {
  const nBuckets = numberAt(object, 'nBuckets');
  const planesRaw = objectAt(object, 'planes');
  const geometry = stringAt(object, 'geometry');
  const detector = stringAt(object, 'detector');
  const cobos = numberArrayAt(object, 'cobos');
  const run = numberAt(object, 'run');
  if (nBuckets === null) return fail('bad-field', 'meta.nBuckets');
  if (planesRaw === null) return fail('bad-field', 'meta.planes');
  if (geometry === null) return fail('bad-field', 'meta.geometry');
  if (detector === null) return fail('bad-field', 'meta.detector');
  if (cobos === null) return fail('bad-field', 'meta.cobos');
  if (run === null) return fail('bad-field', 'meta.run');

  const u = numberAt(planesRaw, 'U');
  const v = numberAt(planesRaw, 'V');
  const w = numberAt(planesRaw, 'W');
  if (u === null || v === null || w === null) return fail('bad-field', 'meta.planes.{U,V,W}');

  // `anglesDeg` は null になりうる(HeaderScalars に無いジオメトリ)。
  const anglesValue = object['anglesDeg'];
  let anglesDeg: number[] | null = null;
  if (anglesValue !== null && anglesValue !== undefined) {
    anglesDeg = numberArrayAt(object, 'anglesDeg');
    if (anglesDeg === null) return fail('bad-field', 'meta.anglesDeg');
  }

  return {
    kind: 'meta',
    meta: {
      type: 'meta',
      nBuckets,
      planes: { U: u, V: v, W: w },
      geometry,
      anglesDeg,
      detector,
      cobos,
      run,
    },
    unknownKeys: unknownKeysOf(object, META_KEYS),
  };
}

function parseStatus(object: Record<string, unknown>): JsonParseResult {
  const run = numberAt(object, 'run');
  const state = stringAt(object, 'state');
  const eventsBuilt = numberAt(object, 'events_built');
  const eventsIncomplete = numberAt(object, 'events_incomplete');
  const lateFragments = numberAt(object, 'late_fragments');
  const pendingEvents = numberAt(object, 'pending_events');
  const framesPerCobo = numberMapAt(object, 'frames_per_cobo');
  const bytesWritten = numberAt(object, 'bytes_written');
  const saturation = saturationAt(object);
  const publishDrops = numberAt(object, 'publish_drops');
  const monitorGaps = numberAt(object, 'monitorGaps');
  const clients = numberAt(object, 'clients');
  const wsDropped = numberAt(object, 'wsDropped');

  if (run === null) return fail('bad-field', 'status.run');
  if (state === null) return fail('bad-field', 'status.state');
  if (eventsBuilt === null) return fail('bad-field', 'status.events_built');
  if (eventsIncomplete === null) return fail('bad-field', 'status.events_incomplete');
  if (lateFragments === null) return fail('bad-field', 'status.late_fragments');
  if (pendingEvents === null) return fail('bad-field', 'status.pending_events');
  if (framesPerCobo === null) return fail('bad-field', 'status.frames_per_cobo');
  if (bytesWritten === null) return fail('bad-field', 'status.bytes_written');
  if (saturation === null) return fail('bad-field', 'status.saturation');
  if (publishDrops === null) return fail('bad-field', 'status.publish_drops');
  if (monitorGaps === null) return fail('bad-field', 'status.monitorGaps');
  if (clients === null) return fail('bad-field', 'status.clients');
  if (wsDropped === null) return fail('bad-field', 'status.wsDropped');

  return {
    kind: 'status',
    status: {
      type: 'status',
      run,
      state,
      events_built: eventsBuilt,
      events_incomplete: eventsIncomplete,
      late_fragments: lateFragments,
      pending_events: pendingEvents,
      frames_per_cobo: framesPerCobo,
      bytes_written: bytesWritten,
      saturation,
      publish_drops: publishDrops,
      monitorGaps,
      clients,
      wsDropped,
    },
    unknownKeys: unknownKeysOf(object, STATUS_KEYS),
  };
}

function parseRun(object: Record<string, unknown>): JsonParseResult {
  const state = stringAt(object, 'state');
  const run = numberAt(object, 'run');
  const ts = stringAt(object, 'ts');
  if (state === null) return fail('bad-field', 'run.state');
  if (run === null) return fail('bad-field', 'run.run');
  if (ts === null) return fail('bad-field', 'run.ts');
  return {
    kind: 'run',
    run: { type: 'run', state, run, ts },
    unknownKeys: unknownKeysOf(object, RUN_KEYS),
  };
}

/**
 * SPEC §5.2 の飽和率 = `saturated / counted * 100`。
 *
 * `counted == 0` は「まだ数えていない」であって 0 % ではないので `null` を返し、
 * 画面は `—` を出す(未知と 0 を混同させない)。
 */
export function saturationPercent(saturation: Saturation | undefined | null): number | null {
  if (!saturation || saturation.counted === 0) return null;
  return (saturation.saturated / saturation.counted) * 100;
}

/** SPEC §10.3 `subscribe` の購読集合。 */
export interface StreamSet {
  readonly uvw: boolean;
  readonly waveforms: boolean;
  readonly histos: boolean;
  readonly status: boolean;
}

/** SPEC §10.3: 既定は waveforms **以外** ON(帯域制御)。 */
export const DEFAULT_STREAMS: StreamSet = {
  uvw: true,
  waveforms: false,
  histos: true,
  status: true,
};

/** SPEC §10.3 の並び。monitor 側は「配列に無いものは OFF」と読む。 */
const STREAM_ORDER = ['uvw', 'waveforms', 'histos', 'status'] as const;

/** `{"streams": [...]}` を作る(C→S)。 */
export function encodeSubscribe(streams: StreamSet): string {
  return JSON.stringify({ streams: STREAM_ORDER.filter((name) => streams[name]) });
}
