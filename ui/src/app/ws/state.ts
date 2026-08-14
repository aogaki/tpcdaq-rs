/**
 * WS リンクの健全性判定(純ロジック)。時計と乱数は引数で受け取るので、
 * テストが決定的に書ける。
 */

/** TODO/027 の確定設計: `status` が 3 秒途絶したら stale。 */
export const STATUS_STALE_MS = 3000;

/** 再接続バックオフの基準(1 回目)。 */
export const RECONNECT_BASE_MS = 500;
/** 再接続バックオフの上限(TODO/027: 5–10 s)。 */
export const RECONNECT_MAX_MS = 8000;

/** WS ソケットの状態。 */
export type LinkState = 'disconnected' | 'connecting' | 'connected';

/**
 * status の健全性。
 *
 * - `offline` — WS が繋がっていない(monitor が落ちている / 経路が切れている)。
 * - `waiting` — 繋がったばかりで最初の status をまだ待っている。
 * - `fresh` — 直近 3 秒以内に status が来た。
 * - `stale` — 繋がっているのに status が 3 秒以上来ない(= root-sink 側が止まっている)。
 */
export type StatusHealth = 'offline' | 'waiting' | 'fresh' | 'stale';

export interface StatusHealthInput {
  readonly link: LinkState;
  /** WS が open した時刻(ms)。未接続なら null。 */
  readonly connectedAtMs: number | null;
  /** 最後に status を受けた時刻(ms)。未着なら null。 */
  readonly lastStatusAtMs: number | null;
  readonly nowMs: number;
  readonly staleAfterMs?: number;
}

export function statusHealth(input: StatusHealthInput): StatusHealth {
  const staleAfter = input.staleAfterMs ?? STATUS_STALE_MS;
  if (input.link !== 'connected') return 'offline';

  // status が一度も来ていないときは接続時刻から測る(monitor 単体起動 =
  // root-sink 無しだと status は永遠に来ないので、やがて stale になるのが正)。
  const since = input.lastStatusAtMs ?? input.connectedAtMs;
  if (since === null) return 'waiting';
  if (input.nowMs - since >= staleAfter) return 'stale';
  return input.lastStatusAtMs === null ? 'waiting' : 'fresh';
}

/**
 * 指数バックオフ + ジッタ。`attempt` は 1 始まり。
 *
 * ジッタは基準の 50–100 %(複数タブが同時に落ちたときに再接続が同期しないように)。
 */
export function reconnectDelayMs(attempt: number, random: () => number = Math.random): number {
  const steps = Math.max(0, Math.floor(attempt) - 1);
  const base = Math.min(RECONNECT_BASE_MS * 2 ** steps, RECONNECT_MAX_MS);
  return Math.round(base * (0.5 + 0.5 * random()));
}
