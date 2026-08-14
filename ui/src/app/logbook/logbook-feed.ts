/**
 * `GET /api/logbook?since_seq=N` → `{records: [...], tail_corrupt: bool}` の追従(純ロジック)。
 *
 * ログブックは WS には載っていないので **REST のポーリング**で追う(既定 5 s)。
 * 取得済みの最大 `seq` を次の `since_seq` に使う。
 *
 * この層が守ること(CLAUDE.md「silent failure を作らない」):
 * - 取得失敗・応答の形違いは `lastError` に残して画面に出す(黙って空にしない)。
 * - `tail_corrupt` は素通しで持ち上げる(JSONL の書き込み中断の痕跡、SPEC §9.1)。
 * - 未知 type / 取り込めなかった行は**数える**。
 */

import { type LogbookEntry, parseLogbookEntry } from './logbook-record';

/** 追従のポーリング間隔(029 発注書: 既定 5 s)。 */
export const LOGBOOK_POLL_MS = 5000;

export interface LogbookFeedState {
  /** `seq` 昇順。表示は新しい順にするので、並べ替えは呼び手が行う。 */
  readonly entries: readonly LogbookEntry[];
  /** 次の `since_seq`(= 取得済みの最大 seq)。 */
  readonly sinceSeq: number;
  readonly tailCorrupt: boolean;
  /** 未知 type の累計(前方互換の見張り)。 */
  readonly unknownTypes: number;
  /** 取り込めなかった行の累計。 */
  readonly malformed: number;
  /** 直近の失敗(成功で消える)。 */
  readonly lastError: string | null;
}

export const EMPTY_LOGBOOK_FEED: LogbookFeedState = {
  entries: [],
  sinceSeq: 0,
  tailCorrupt: false,
  unknownTypes: 0,
  malformed: 0,
  lastError: null,
};

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** 例外・拒否理由を 1 行の文言にする。 */
export function describeError(error: unknown): string {
  if (error instanceof Error) return error.message;
  return String(error);
}

/** 応答 1 回分を畳み込む。**投げない**。 */
export function applyLogbookResponse(state: LogbookFeedState, raw: unknown): LogbookFeedState {
  if (!isObject(raw) || !Array.isArray(raw['records'])) {
    return {
      ...state,
      lastError: '応答に records 配列がありません(controller の返事が想定と違います)',
    };
  }

  const merged = new Map<number, LogbookEntry>();
  for (const entry of state.entries) merged.set(entry.seq, entry);

  let unknownTypes = state.unknownTypes;
  let malformed = state.malformed;
  let sinceSeq = state.sinceSeq;

  for (const record of raw['records'] as unknown[]) {
    const entry = parseLogbookEntry(record);
    if (!entry) {
      malformed += 1;
      continue;
    }
    if (!merged.has(entry.seq)) {
      merged.set(entry.seq, entry);
      if (entry.kind === 'unknown') unknownTypes += 1;
    }
    // 取得済みの最大 seq は下がらない(古い行だけが返っても巻き戻らない)。
    if (entry.seq > sinceSeq) sinceSeq = entry.seq;
  }

  const entries = [...merged.values()].sort((a, b) => a.seq - b.seq);

  return {
    entries,
    sinceSeq,
    tailCorrupt: raw['tail_corrupt'] === true,
    unknownTypes,
    malformed,
    lastError: null,
  };
}

/** 取得そのものに失敗したとき(controller が落ちている等)。取り込み済みは消さない。 */
export function applyLogbookError(state: LogbookFeedState, error: unknown): LogbookFeedState {
  return { ...state, lastError: describeError(error) };
}
