/**
 * SPEC §9.2 — JSONL ログブックの 1 行を画面用に整形する**純ロジック**(Angular 非依存)。
 *
 * ワイヤは `{ts, seq, type, actor, …型ごとのフィールド}` のフラットな 1 オブジェクト
 * (Rust 側 `LogbookLine` が `record` を `#[serde(flatten)]` している)。
 *
 * 方針:
 * - **型ごとに見せ方を変える**(`run_stop` の ok=false は目立たせ、counters / files / params は畳む)。
 * - **未知の type は捨てない**。生 JSON を出して `kind: 'unknown'` にし、呼び手が数える
 *   (前方互換 — P6 の psu 詳細化に備える。CLAUDE.md「silent failure を作らない」)。
 * - 数値の欠損(`null`)は **「取得不能」と書く**。`0`(= 実測ゼロ)と混同しない(SPEC v1.10 §9.2)。
 * - 例外を投げない。読めない行は `null` を返し、呼び手が数える。
 */

export type LogbookKind = 'run_start' | 'run_stop' | 'audit' | 'comment' | 'psu' | 'unknown';
export type LogbookSeverity = 'normal' | 'attention' | 'error';

/** 見出しの隣に常時出す小さな項目。 */
export interface LogbookField {
  readonly label: string;
  readonly value: string;
}

/** 既定で畳んでおく塊(counters / files / params / 生 JSON)。 */
export interface LogbookDetail {
  readonly label: string;
  readonly lines: readonly string[];
}

export interface LogbookEntry {
  readonly seq: number;
  /** 生の ts(オフセット込み。どのタイムゾーンで書かれたかを失わない)。 */
  readonly ts: string;
  readonly date: string;
  readonly time: string;
  readonly kind: LogbookKind;
  /** 生の `type` 文字列(未知でもそのまま出す)。 */
  readonly type: string;
  readonly actor: string;
  readonly title: string;
  readonly severity: LogbookSeverity;
  /** 自由記述の本文(comment の text、run_start の comment、audit の error)。 */
  readonly body: string | null;
  readonly fields: readonly LogbookField[];
  readonly details: readonly LogbookDetail[];
}

const KNOWN_KINDS: readonly LogbookKind[] = ['run_start', 'run_stop', 'audit', 'comment', 'psu'];

/** SPEC §9.2 の 5 型だけを既知にする。それ以外は `unknown`。 */
export function logbookKind(type: unknown): LogbookKind {
  const found = KNOWN_KINDS.find((kind) => kind === type);
  return found ?? 'unknown';
}

/**
 * 3 桁区切り。`toLocaleString()` は実行環境のロケールで揺れるので使わない
 * (テストが環境依存になるのを避ける)。
 */
export function formatInt(value: number): string {
  if (!Number.isFinite(value)) return String(value);
  const negative = value < 0;
  const digits = Math.abs(Math.trunc(value)).toString();
  let out = '';
  for (let i = 0; i < digits.length; i++) {
    if (i > 0 && (digits.length - i) % 3 === 0) out += ',';
    out += digits[i];
  }
  return negative ? `-${out}` : out;
}

// ---------------------------------------------------------------------
// 値の取り出し(壊れた行でも投げない)
// ---------------------------------------------------------------------

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

function obj(raw: Raw, key: string): Raw | null {
  const value = raw[key];
  return isObject(value) ? value : null;
}

function arr(raw: Raw, key: string): unknown[] {
  const value = raw[key];
  return Array.isArray(value) ? value : [];
}

/** 数値なら 3 桁区切り、`null` なら「取得不能」(0 と混同しない — SPEC v1.10 §9.2)。 */
function counterValue(value: unknown): string {
  if (typeof value === 'number' && Number.isFinite(value)) return formatInt(value);
  return 'unavailable (null)';
}

/** `350.2` → `"350.2"` / `350.0` → `"350"`(余計な桁を足さない)。 */
function numberText(value: unknown): string {
  return typeof value === 'number' && Number.isFinite(value) ? String(value) : '—';
}

/** RFC3339(ミリ秒・オフセット付き、SPEC §9.1)を日付と時刻に割る。形が違えば時刻欄に丸ごと出す。 */
function splitTs(ts: string): { date: string; time: string } {
  const match = /^(\d{4}-\d{2}-\d{2})T(\d{2}:\d{2}:\d{2}(?:\.\d+)?)/.exec(ts);
  return match ? { date: match[1], time: match[2] } : { date: '', time: ts };
}

// ---------------------------------------------------------------------
// 型ごとの整形
// ---------------------------------------------------------------------

interface Shaped {
  readonly title: string;
  readonly severity: LogbookSeverity;
  readonly body: string | null;
  readonly fields: LogbookField[];
  readonly details: LogbookDetail[];
}

function runLabel(raw: Raw): string {
  const run = num(raw, 'run');
  return run === null ? '—' : String(run);
}

function shapeRunStart(raw: Raw): Shaped {
  const geometry = obj(raw, 'geometry');
  const cobos = arr(raw, 'cobos');
  const fragments = arr(raw, 'expected_fragments');
  const comment = str(raw, 'comment');
  // v1.13(TODO/042): 3 相(describe/prepare/configure)が非同値のときだけ `config_ids` が
  // 載る。同値なら省略(nullable 規律)なので、無いときは従来どおり `config_id` 一本で出す。
  const configIds = obj(raw, 'config_ids');

  const configFields: LogbookField[] = configIds
    ? [
        { label: 'config (describe)', value: str(configIds, 'describe') ?? '—' },
        { label: 'config (prepare)', value: str(configIds, 'prepare') ?? '—' },
        { label: 'config (configure)', value: str(configIds, 'configure') ?? '—' },
      ]
    : [{ label: 'config', value: str(raw, 'config_id') ?? '—' }];

  const details: LogbookDetail[] = [];
  const sha = geometry ? str(geometry, 'sha256') : null;
  if (sha) details.push({ label: 'geometry sha256', lines: [sha] });
  if (cobos.length > 0) {
    details.push({
      label: `CoBo (${cobos.length})`,
      lines: cobos.map((entry) =>
        isObject(entry) ? `CoBo ${numberText(entry['id'])} → ${str(entry, 'listen') ?? '—'}` : '—',
      ),
    });
  }
  if (fragments.length > 0) {
    details.push({
      label: `expected fragments (${fragments.length})`,
      lines: fragments.map((pair) =>
        Array.isArray(pair) ? `cobo ${numberText(pair[0])} / asad ${numberText(pair[1])}` : '—',
      ),
    });
  }

  return {
    title: `run ${runLabel(raw)} started`,
    severity: 'normal',
    body: comment && comment.length > 0 ? comment : null,
    fields: [
      { label: 'operator', value: str(raw, 'operator') ?? '—' },
      ...configFields,
      { label: 'geometry', value: geometry ? (str(geometry, 'path') ?? '—') : '—' },
    ],
    details,
  };
}

function shapeRunStop(raw: Raw): Shaped {
  const ok = raw['ok'] !== false;
  const counters = obj(raw, 'counters');
  const files = arr(raw, 'files');
  // v1.12/v1.14(TODO/033, 051): 欠落(旧行)は無印。値があるときだけ、それぞれの意味で出す —
  // `eos_closed:false` = 明確な異常(severity を上げる)/ `forced_eos:false` = 警告
  // (v1.14 注記: 実機 TCP flow では forced_eos:true が常態なので、false は
  // 「stop 前にリンクが死んだ」ことの強い印)。
  const eosClosed = boolFlag(raw, 'eos_closed');
  const forcedEos = boolFlag(raw, 'forced_eos');

  const details: LogbookDetail[] = [];
  if (counters) {
    const lines: string[] = [
      `events_built = ${counterValue(counters['events_built'])}`,
      `events_incomplete = ${counterValue(counters['events_incomplete'])}`,
      `late_fragments = ${counterValue(counters['late_fragments'])}`,
    ];
    const frames = obj(counters, 'frames');
    if (frames) {
      for (const cobo of Object.keys(frames).sort(byCoboId)) {
        lines.push(`frames cobo ${cobo} = ${counterValue(frames[cobo])}`);
      }
    }
    lines.push(`overflow_frames = ${counterValue(counters['overflow_frames'])}`);
    lines.push(`malformed = ${counterValue(counters['malformed'])}`);
    details.push({ label: 'counters', lines });
  }
  if (files.length > 0) {
    details.push({
      label: `files (${files.length})`,
      lines: files.map((file) =>
        isObject(file) ? `${str(file, 'path') ?? '—'} — ${counterValue(file['bytes'])} B` : '—',
      ),
    });
  }

  const duration = num(raw, 'duration_s');
  const fields: LogbookField[] = [
    { label: 'reason', value: str(raw, 'reason') ?? '—' },
    { label: 'duration', value: duration === null ? '—' : `${duration} s` },
  ];
  // eos_closed:false = 明確な異常。severity を 'error' へ引き上げる(ok=false の場合と同じ扱い)。
  if (eosClosed === false) {
    fields.push({
      label: 'eos_closed',
      value: 'false — EOS did not travel through the whole chain (fault)',
    });
  }
  // forced_eos:false = 警告。すでに error なら格上げしない(error が上位)。
  if (forcedEos === false) {
    fields.push({
      label: 'forced_eos',
      value: 'false — the link may have died before stop',
    });
  }

  let severity: LogbookSeverity = ok ? 'normal' : 'error';
  if (eosClosed === false) severity = 'error';
  else if (forcedEos === false && severity === 'normal') severity = 'attention';

  return {
    title: `run ${runLabel(raw)} stopped (${ok ? 'ok' : 'fault'})`,
    severity,
    body: null,
    fields,
    details,
  };
}

/**
 * 三値の bool フラグ(欠落は「記録なし」であって `false` ではない — 033-A の意味論)。
 * v1.12 より前の行は `forced_eos`/`eos_closed` を持たない。
 */
function boolFlag(raw: Raw, key: string): boolean | null {
  const value = raw[key];
  return typeof value === 'boolean' ? value : null;
}

/** cobo id は数として並べる(文字列キーの辞書順で "10" が "2" より前に来ないように)。 */
function byCoboId(a: string, b: string): number {
  const na = Number(a);
  const nb = Number(b);
  if (Number.isFinite(na) && Number.isFinite(nb)) return na - nb;
  return a < b ? -1 : a > b ? 1 : 0;
}

function shapeAudit(raw: Raw): Shaped {
  const ok = raw['ok'] !== false;
  const error = str(raw, 'error');
  const details: LogbookDetail[] = [];
  if (raw['params'] !== undefined && raw['params'] !== null) {
    details.push({ label: 'params', lines: [JSON.stringify(raw['params'])] });
  }
  return {
    title: `audit — ${str(raw, 'action') ?? '(no action)'}`,
    severity: ok ? 'normal' : 'error',
    body: ok ? null : error,
    fields: [
      { label: 'operator', value: str(raw, 'operator') ?? '—' },
      { label: 'result', value: ok ? 'ok' : 'failed' },
    ],
    details,
  };
}

function shapeComment(raw: Raw): Shaped {
  return {
    title: `comment — ${str(raw, 'author') ?? '(no author)'}`,
    severity: 'normal',
    body: str(raw, 'text'),
    fields: [],
    details: [],
  };
}

function shapePsu(raw: Raw): Shaped {
  const values = obj(raw, 'values');
  const event = str(raw, 'event') ?? '—';
  const channel = num(raw, 'channel');
  return {
    // TRIP は HV が落ちた記録。平文に埋もれさせない(P6 で詳細化)。
    title: `PSU ${str(raw, 'device') ?? '—'} ch${channel === null ? '—' : channel} — ${event}`,
    severity: event === 'TRIP' ? 'attention' : 'normal',
    body: null,
    fields: [
      { label: 'vmon', value: numberText(values?.['vmon']) },
      { label: 'imon', value: numberText(values?.['imon']) },
      { label: 'vset', value: numberText(values?.['vset']) },
    ],
    details: [],
  };
}

function shapeUnknown(raw: Raw, type: string | null): Shaped {
  return {
    title: `unknown record type: ${type ?? '(no type)'}`,
    severity: 'attention',
    body: null,
    fields: [],
    details: [{ label: 'raw JSON', lines: [JSON.stringify(raw)] }],
  };
}

// ---------------------------------------------------------------------
// 入口
// ---------------------------------------------------------------------

/**
 * 1 行を整形する。**投げない**。オブジェクトでない / `seq` が数でない行は `null`
 * (seq が無いと追従にも並べ替えにも使えないため。呼び手が数えて画面に出す)。
 */
export function parseLogbookEntry(raw: unknown): LogbookEntry | null {
  if (!isObject(raw)) return null;
  const seq = num(raw, 'seq');
  if (seq === null) return null;

  const type = str(raw, 'type');
  const kind = logbookKind(type);
  const ts = str(raw, 'ts') ?? '';
  const { date, time } = splitTs(ts);

  let shaped: Shaped;
  switch (kind) {
    case 'run_start':
      shaped = shapeRunStart(raw);
      break;
    case 'run_stop':
      shaped = shapeRunStop(raw);
      break;
    case 'audit':
      shaped = shapeAudit(raw);
      break;
    case 'comment':
      shaped = shapeComment(raw);
      break;
    case 'psu':
      shaped = shapePsu(raw);
      break;
    case 'unknown':
      shaped = shapeUnknown(raw, type);
      break;
  }

  return {
    seq,
    ts,
    date,
    time,
    kind,
    type: type ?? '(no type)',
    actor: str(raw, 'actor') ?? '—',
    title: shaped.title,
    severity: shaped.severity,
    body: shaped.body,
    fields: shaped.fields,
    details: shaped.details,
  };
}
