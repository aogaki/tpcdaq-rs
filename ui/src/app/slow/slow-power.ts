/**
 * TODO/124 レーン C — HV / LV の ch ごとの ON/OFF ボタンの純ロジック(Angular 非依存)。
 *
 * SPEC §11 v1.42: web が持てる物理的作動はこれだけ。規則の**判断はサーバ(controller)**が
 * 最終に行う(409)。ここの規則は「画面で分かる誤りを往復させない」ための先回りで、
 * サーバの規則と同じ: HV ON と LV OFF は相 `Idle` のみ、HV OFF と LV ON は常時。
 *
 * 文言は英語。サーバの `error` は加工せずそのまま見せる(504 の
 * 「state unknown — check the table」もサーバ文言)。
 */
import type { ActionResult } from '../api/run-control-api';
import type { ChannelRow } from './slow-table';
import { formatStatusBits } from './slow-table';

export const NO_TOKEN_REASON = 'acquire control in Run control first';
export const UNKNOWN_STATE_HINT = 'state unknown — check the table';

export interface PowerContext {
  readonly hasToken: boolean;
  /** controller の相(`GET /api/status`)。取れていなければ null。 */
  readonly phase: string | null;
  /** 別の power 操作を送信中。 */
  readonly busy: boolean;
}

/** 押したときの目標。ON の行は OFF にし、それ以外(OFF / 不明)は ON にする。 */
export function targetOn(row: ChannelRow): boolean {
  return row.onOff !== 'ON';
}

/** ボタン文言。 */
export function buttonLabel(row: ChannelRow): string {
  return targetOn(row) ? 'Turn ON' : 'Turn OFF';
}

/** Idle でなければ拒否される操作(HV ON / LV OFF)か。 */
export function needsIdle(row: ChannelRow): boolean {
  const on = targetOn(row);
  return (row.power === 'hv' && on) || (row.power === 'lv' && !on);
}

/** 押せない理由(押せるなら null)。ツールチップに出す。 */
export function disabledReason(row: ChannelRow, ctx: PowerContext): string | null {
  if (!ctx.hasToken) return NO_TOKEN_REASON;
  if (ctx.busy) return 'another power command is in progress';
  if (needsIdle(row)) {
    const what = row.power === 'hv' ? 'HV ON' : 'LV OFF';
    const hint = row.power === 'lv' ? ' — stop the run first' : '';
    if (ctx.phase === null) return `${what} needs the run phase Idle, but the phase is unknown`;
    if (ctx.phase !== 'Idle') {
      return `${what} is allowed only when the run phase is Idle (now ${ctx.phase})${hint}`;
    }
  }
  return null;
}

/** 確認ダイアログの内容。 */
export interface PowerDialog {
  readonly title: string;
  readonly state: string;
  /** LV のみ: 両 ch の今の状態。 */
  readonly lvBefore: readonly string[];
  readonly warnings: readonly string[];
}

function stateText(row: ChannelRow): string {
  if (row.power === 'hv') {
    return `Now: ${row.statusMask === null ? row.status : formatStatusBits(row.statusMask)}`;
  }
  return `Now: ${row.onOff ?? 'unknown'}`;
}

export function buildDialog(
  row: ChannelRow,
  siblings: readonly ChannelRow[],
  phase: string | null,
): PowerDialog {
  const on = targetOn(row);
  const warnings: string[] = [];
  if (row.power === 'hv' && !on && phase === 'Running') {
    warnings.push(
      'A run is in progress: turning HV OFF ramps the voltage down and the data of this run ends there.',
    );
  }
  if (row.power === 'hv' && on) {
    warnings.push(
      'The channel ramps up with the instrument settings; the front-panel switch must be ON.',
    );
  }
  if (row.power === 'lv') {
    warnings.push(
      on
        ? 'Turning an LV channel ON also switches the master output ON; other channels may change.'
        : 'LV OFF is allowed only when the run is Idle.',
    );
  }
  const lvBefore =
    row.power === 'lv'
      ? siblings
          .filter((s) => s.power === 'lv' && s.device === row.device)
          .map((s) => `${s.device} ch ${s.ch}: ${s.onOff ?? 'unknown'}`)
      : [];
  return {
    title: `Turn ${on ? 'ON' : 'OFF'} ${row.device} ch ${row.ch}?`,
    state: stateText(row),
    lvBefore,
    warnings,
  };
}

/** 結果の表示。 */
export interface PowerOutcome {
  readonly ok: boolean;
  readonly text: string;
  /** LV で、操作した ch 以外が変わったときの明示(無ければ空)。 */
  readonly notes: readonly string[];
}

function isObj(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}

function hvText(v: unknown): string {
  if (!isObj(v)) return 'n/a';
  const flags = v['flags'];
  if (Array.isArray(flags)) return flags.length === 0 ? 'OFF' : flags.join(' ');
  return typeof v['status'] === 'number' ? formatStatusBits(v['status']) : 'n/a';
}

function lvOutp(v: unknown): Record<string, number> | null {
  if (!isObj(v) || !isObj(v['outp'])) return null;
  const out: Record<string, number> = {};
  for (const [ch, value] of Object.entries(v['outp'])) {
    if (typeof value === 'number') out[ch] = value;
  }
  return out;
}

const onOffText = (n: number): string => (n !== 0 ? 'ON' : 'OFF');

export function interpretOutcome(row: ChannelRow, on: boolean, result: ActionResult): PowerOutcome {
  const what = `${row.device} ch ${row.ch} ${on ? 'ON' : 'OFF'}`;
  if (!result.ok) {
    let error = result.error ?? `HTTP ${result.status}`;
    if (result.status === 504 && !error.includes(UNKNOWN_STATE_HINT)) {
      error = `${error} (${UNKNOWN_STATE_HINT})`;
    }
    const code = result.status === 0 ? 'not sent' : `HTTP ${result.status}`;
    return { ok: false, text: `${what} failed (${code}): ${error}`, notes: [] };
  }
  const before = result.body?.['before'];
  const after = result.body?.['after'];
  if (row.power === 'lv') {
    const b = lvOutp(before);
    const a = lvOutp(after);
    if (b === null || a === null) {
      return { ok: true, text: `${what}: done (before/after not reported)`, notes: [] };
    }
    const chs = [...new Set([...Object.keys(b), ...Object.keys(a)])].sort();
    const parts = chs.map((ch) => `ch ${ch} ${onOffText(b[ch] ?? 0)} → ${onOffText(a[ch] ?? 0)}`);
    const notes = chs
      .filter((ch) => ch !== String(row.ch) && (b[ch] ?? 0) !== (a[ch] ?? 0))
      .map(
        (ch) =>
          `${row.device} ch ${ch} also changed: ${onOffText(b[ch] ?? 0)} → ${onOffText(a[ch] ?? 0)}`,
      );
    return { ok: true, text: `${what}: ${parts.join(', ')}`, notes };
  }
  return { ok: true, text: `${what}: ${hvText(before)} → ${hvText(after)}`, notes: [] };
}
