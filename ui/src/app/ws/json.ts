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

/** slowctl → monitor PUB の 1 サンプル(SPEC §2.3 v1.31 `samples[]`)。 */
export interface SlowSample {
  readonly device: string;
  /** FLOW-BUS ノード番号など。無いデバイス(Arduino 等)は `null`(§9.2 の node 無し)。 */
  readonly node: number | null;
  readonly param: string;
  readonly value: number;
  /** 機器が返した単位文字列そのまま(コードで決め打ちしない)。 */
  readonly unit: string;
  readonly ts_ms: number;
  /** 機器側 Alarm info ≠ 0、または `[lo, hi]` 逸脱。 */
  readonly alarm: boolean;
}

/** slowctl → monitor PUB の `counters`(SPEC §2.3 v1.31、Rust `SlowCounters` そのまま)。 */
export interface SlowCounters {
  readonly poll_ok: number;
  readonly poll_err: number;
  readonly parse_err: number;
  readonly reconnects: number;
  readonly identity_mismatch: number;
  /**
   * TODO/122 の InfluxDB writer(点 = Sample 1 個)。**122 より前の slowctl はキー自体が
   * 無い = `undefined`**(画面では「—」)。`[slowctl.influx]` が無い slowctl は 0 を出す。
   */
  readonly influx_points_written?: number;
  readonly influx_points_dropped?: number;
  readonly influx_write_errors?: number;
  /** 最後に失敗した POST の内容。`null` = 一度も失敗していない、`undefined` = 古い slowctl。 */
  readonly influx_last_error?: string | null;
}

/** slowctl → monitor PUB ペイロード(SPEC §2.3 v1.31、`status` の `slow` に同梱される)。 */
export interface SlowPayload {
  readonly kind: string;
  readonly ts_ms: number;
  readonly samples: readonly SlowSample[];
  readonly counters: SlowCounters;
}

/** SPEC §10.3 `status` = §5.3 の 11 キー + monitor の 3 キー + `slow`(v1.31、任意)。 */
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
  /** ZS フラグメントをペデスタル表で復元した数(152〜)。152 より前の monitor はキー無し = `undefined`。 */
  readonly uvwZsFragmentsRestored?: number;
  /** ZS フラグメントが来たが表が無く復元できなかった数(152〜)。0 でなければ問題。 */
  readonly uvwZsFragmentsNoTable?: number;
  /** 表に無い ch に当たった数(152〜)。0 でなければ問題。 */
  readonly uvwTableMissingChannels?: number;
  /** slowctl の最新ペイロード(v1.31)。`[monitor] slow_sub` 未設定なら**キー自体が無い**
   * (省略、`undefined`)—— 従来の status と同一に見える。 */
  readonly slow?: SlowPayload;
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

/** `stripGeometry` の線分 1 本: `[strip(1 始まり), section, x0, y0, length]`(mm)。 */
export type SegmentRow = readonly [
  strip: number,
  section: number,
  x0: number,
  y0: number,
  length: number,
];

/** `stripGeometry` の 1 面(U/V/W)。 */
export interface StripPlaneGeometry {
  /** strip 方向の単位ベクトル。 */
  readonly unit: readonly [number, number];
  /** pitch 方向の単位ベクトル(strip 番号が増える向き)。strip 方向とは六角配置でしか直交しない。 */
  readonly pitchUnit: readonly [number, number];
  readonly segments: readonly SegmentRow[];
}

/**
 * SPEC §10.3 `stripGeometry`(接続時 1 回、`meta` の次。TODO/125)。単位は mm。
 * 線分の有効範囲は (x0,y0) から `unit` 方向に `[-0.5*padPitch, length]`。
 */
export interface StripGeometryMessage {
  readonly type: 'stripGeometry';
  readonly padPitch: number;
  readonly stripPitch: number;
  /** `[xmin, xmax, ymin, ymax]` */
  readonly rangeXY: readonly [number, number, number, number];
  readonly planes: {
    readonly U: StripPlaneGeometry;
    readonly V: StripPlaneGeometry;
    readonly W: StripPlaneGeometry;
  };
}

/** ジオメトリが作れなかったときの形(理由は UI がそのまま表示する)。 */
export interface StripGeometryUnavailable {
  readonly type: 'stripGeometry';
  readonly unavailable: string;
}

export function isStripGeometryUnavailable(
  message: StripGeometryMessage | StripGeometryUnavailable,
): message is StripGeometryUnavailable {
  return 'unavailable' in message;
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
  | {
      readonly kind: 'stripGeometry';
      readonly geometry: StripGeometryMessage | StripGeometryUnavailable;
      readonly unknownKeys: readonly string[];
    }
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

function booleanAt(object: Record<string, unknown>, key: string): boolean | null {
  const value = object[key];
  return typeof value === 'boolean' ? value : null;
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

/** `samples[]` の 1 要素(SPEC §2.3)。壊れていれば `null`(呼び手が status 全体を bad-field にする)。 */
function slowSampleAt(value: unknown): SlowSample | null {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return null;
  const object = value as Record<string, unknown>;
  const device = stringAt(object, 'device');
  const param = stringAt(object, 'param');
  const sampleValue = numberAt(object, 'value');
  const unit = stringAt(object, 'unit');
  const tsMs = numberAt(object, 'ts_ms');
  const alarm = booleanAt(object, 'alarm');
  if (
    device === null ||
    param === null ||
    sampleValue === null ||
    unit === null ||
    tsMs === null ||
    alarm === null
  ) {
    return null;
  }

  const nodeRaw = object['node'];
  let node: number | null;
  if (nodeRaw === null) node = null;
  else if (typeof nodeRaw === 'number' && Number.isFinite(nodeRaw)) node = nodeRaw;
  else return null;

  return { device, node, param, value: sampleValue, unit, ts_ms: tsMs, alarm };
}

/** 型違いの印(キーが無い = `undefined` と区別する)。 */
const BAD = Symbol('bad');

/** 任意の数値キー: 無ければ `undefined`、あって数値でなければ `BAD`。 */
function optionalNumberAt(
  object: Record<string, unknown>,
  key: string,
): number | undefined | typeof BAD {
  if (object[key] === undefined) return undefined;
  return numberAt(object, key) ?? BAD;
}

/** 任意の「文字列か null」キー: 無ければ `undefined`、型違いは `BAD`。 */
function optionalNullableStringAt(
  object: Record<string, unknown>,
  key: string,
): string | null | undefined | typeof BAD {
  const value = object[key];
  if (value === undefined || value === null || typeof value === 'string') return value;
  return BAD;
}

/**
 * `counters`(SPEC §2.3、Rust `SlowCounters`)。5 キーは必須、TODO/122 の `influx_*` 4 キーは
 * 任意(122 より前の slowctl には無い)。**あるのに型が違えば** `null`(呼び手が bad-field)。
 */
function slowCountersAt(value: unknown): SlowCounters | null {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return null;
  const object = value as Record<string, unknown>;
  const pollOk = numberAt(object, 'poll_ok');
  const pollErr = numberAt(object, 'poll_err');
  const parseErr = numberAt(object, 'parse_err');
  const reconnects = numberAt(object, 'reconnects');
  const identityMismatch = numberAt(object, 'identity_mismatch');
  const influxWritten = optionalNumberAt(object, 'influx_points_written');
  const influxDropped = optionalNumberAt(object, 'influx_points_dropped');
  const influxErrors = optionalNumberAt(object, 'influx_write_errors');
  const influxLastError = optionalNullableStringAt(object, 'influx_last_error');
  if (
    pollOk === null ||
    pollErr === null ||
    parseErr === null ||
    reconnects === null ||
    identityMismatch === null ||
    influxWritten === BAD ||
    influxDropped === BAD ||
    influxErrors === BAD ||
    influxLastError === BAD
  ) {
    return null;
  }
  return {
    poll_ok: pollOk,
    poll_err: pollErr,
    parse_err: parseErr,
    reconnects,
    identity_mismatch: identityMismatch,
    influx_points_written: influxWritten,
    influx_points_dropped: influxDropped,
    influx_write_errors: influxErrors,
    influx_last_error: influxLastError,
  };
}

/**
 * `status.slow`(SPEC §10.3 v1.31)。**キーが無ければ `value: undefined`**(従来と同一の
 * status = 省略、`[monitor] slow_sub` 未設定)。キーはあるが壊れていれば `bad: true` にして
 * 呼び手が status 全体を bad-field にする(silent に無視しない)。
 */
function slowAt(object: Record<string, unknown>): { value: SlowPayload | undefined; bad: boolean } {
  const raw = object['slow'];
  if (raw === undefined) return { value: undefined, bad: false };
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) {
    return { value: undefined, bad: true };
  }
  const object2 = raw as Record<string, unknown>;
  const kind = stringAt(object2, 'kind');
  const tsMs = numberAt(object2, 'ts_ms');
  const samplesRaw = object2['samples'];
  const counters = slowCountersAt(object2['counters']);
  if (kind === null || tsMs === null || !Array.isArray(samplesRaw) || counters === null) {
    return { value: undefined, bad: true };
  }

  const samples: SlowSample[] = [];
  for (const item of samplesRaw) {
    const sample = slowSampleAt(item);
    if (sample === null) return { value: undefined, bad: true };
    samples.push(sample);
  }

  return { value: { kind, ts_ms: tsMs, samples, counters }, bad: false };
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
  'uvwZsFragmentsRestored',
  'uvwZsFragmentsNoTable',
  'uvwTableMissingChannels',
  'slow',
] as const;

const RUN_KEYS = ['type', 'state', 'run', 'ts'] as const;

const STRIP_GEOMETRY_KEYS = [
  'type',
  'padPitch',
  'stripPitch',
  'rangeXY',
  'planes',
  'unavailable',
] as const;

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
    case 'stripGeometry':
      return parseStripGeometry(object);
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

  const zsRestored = optionalNumberAt(object, 'uvwZsFragmentsRestored');
  const zsNoTable = optionalNumberAt(object, 'uvwZsFragmentsNoTable');
  const zsMissing = optionalNumberAt(object, 'uvwTableMissingChannels');
  if (zsRestored === BAD) return fail('bad-field', 'status.uvwZsFragmentsRestored');
  if (zsNoTable === BAD) return fail('bad-field', 'status.uvwZsFragmentsNoTable');
  if (zsMissing === BAD) return fail('bad-field', 'status.uvwTableMissingChannels');

  const slow = slowAt(object);
  if (slow.bad) return fail('bad-field', 'status.slow');

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
      ...(zsRestored !== undefined ? { uvwZsFragmentsRestored: zsRestored } : {}),
      ...(zsNoTable !== undefined ? { uvwZsFragmentsNoTable: zsNoTable } : {}),
      ...(zsMissing !== undefined ? { uvwTableMissingChannels: zsMissing } : {}),
      ...(slow.value !== undefined ? { slow: slow.value } : {}),
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

/** 数値ちょうど 2 個の配列。 */
function pairAt(object: Record<string, unknown>, key: string): [number, number] | null {
  const values = numberArrayAt(object, key);
  return values !== null && values.length === 2 ? [values[0], values[1]] : null;
}

function segmentRowAt(value: unknown): SegmentRow | null {
  if (!Array.isArray(value) || value.length !== 5) return null;
  for (const item of value) {
    if (typeof item !== 'number' || !Number.isFinite(item)) return null;
  }
  return [value[0], value[1], value[2], value[3], value[4]];
}

/** 失敗したら `detail` に場所を入れた文字列を返す。 */
function stripPlaneAt(object: Record<string, unknown>, name: string): StripPlaneGeometry | string {
  const plane = objectAt(object, name);
  if (plane === null) return `stripGeometry.planes.${name}`;
  const unit = pairAt(plane, 'unit');
  if (unit === null) return `stripGeometry.planes.${name}.unit`;
  const pitchUnit = pairAt(plane, 'pitchUnit');
  if (pitchUnit === null) return `stripGeometry.planes.${name}.pitchUnit`;
  const rows = plane['segments'];
  if (!Array.isArray(rows)) return `stripGeometry.planes.${name}.segments`;
  const segments: SegmentRow[] = [];
  for (const row of rows) {
    const parsed = segmentRowAt(row);
    if (parsed === null) return `stripGeometry.planes.${name}.segments[${segments.length}]`;
    segments.push(parsed);
  }
  return { unit, pitchUnit, segments };
}

function parseStripGeometry(object: Record<string, unknown>): JsonParseResult {
  const unknownKeys = unknownKeysOf(object, STRIP_GEOMETRY_KEYS);

  // `unavailable` 形(ヘッダ不足・strip 行 0 本など)。理由はそのまま表示する。
  if (object['unavailable'] !== undefined) {
    const reason = stringAt(object, 'unavailable');
    if (reason === null) return fail('bad-field', 'stripGeometry.unavailable');
    return {
      kind: 'stripGeometry',
      geometry: { type: 'stripGeometry', unavailable: reason },
      unknownKeys,
    };
  }

  const padPitch = numberAt(object, 'padPitch');
  const stripPitch = numberAt(object, 'stripPitch');
  const range = numberArrayAt(object, 'rangeXY');
  const planesRaw = objectAt(object, 'planes');
  if (padPitch === null) return fail('bad-field', 'stripGeometry.padPitch');
  if (stripPitch === null) return fail('bad-field', 'stripGeometry.stripPitch');
  if (range === null || range.length !== 4) return fail('bad-field', 'stripGeometry.rangeXY');
  if (planesRaw === null) return fail('bad-field', 'stripGeometry.planes');

  const u = stripPlaneAt(planesRaw, 'U');
  if (typeof u === 'string') return fail('bad-field', u);
  const v = stripPlaneAt(planesRaw, 'V');
  if (typeof v === 'string') return fail('bad-field', v);
  const w = stripPlaneAt(planesRaw, 'W');
  if (typeof w === 'string') return fail('bad-field', w);

  return {
    kind: 'stripGeometry',
    geometry: {
      type: 'stripGeometry',
      padPitch,
      stripPitch,
      rangeXY: [range[0], range[1], range[2], range[3]],
      planes: { U: u, V: v, W: w },
    },
    unknownKeys,
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
