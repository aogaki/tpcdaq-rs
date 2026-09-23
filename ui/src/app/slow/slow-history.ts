/**
 * TODO/079 §2.8 — Slow control ビューの純ロジック(Angular にも ECharts にも依存しない)。
 *
 * `waveform-select.ts` / `root-histo.ts` と同じ切り分け: ここは IO 非依存のドメイン計算だけを
 * 持ち、`slow-view.ts` は ECharts の遅延ロードと DOM だけを扱う(Clean Architecture、
 * CLAUDE.md — ドメイン核は IO に依存しない)。
 *
 * 安全境界(SPEC §11): ここには setpoint / バルブ / HV を動かす関数は一切無い
 * (読み取り・表示の計算だけ)。
 */
import type { SlowSample } from '../ws/json';

/** Trend 1 系列あたりの保持点数(1 Hz × 30 min、発注書 §2.8-3)。 */
export const SLOW_HISTORY_LIMIT = 1800;

/** age(now − ts_ms)がこれを超えたら Latest 表で「古い」扱い(薄く出す、発注書 §2.8-3)。 */
export const STALE_AGE_MS = 5000;

/** Trend で同時に重ね描きする系列数の上限(発注書 §2.8-3)。 */
export const MAX_TREND_SERIES = 4;

/** Trend のリングバッファ 1 点。 */
export interface SlowHistoryPoint {
  readonly ts_ms: number;
  readonly value: number;
}

/** 系列キー(= [`slowSeriesKey`])→ 時系列(古い順)。 */
export type SlowHistoryMap = ReadonlyMap<string, readonly SlowHistoryPoint[]>;

/**
 * SPEC §9.2 の `slow_latest` キーと同じ組み立て(`"<device>/<node>/<param>"`、
 * node 無しは `"-"`)。controller 側の慣習に合わせておくと、ログブックの記録と
 * 画面の系列名がそのまま対応する。
 */
export function slowSeriesKey(device: string, node: number | null, param: string): string {
  return `${device}/${node === null ? '-' : node}/${param}`;
}

/**
 * status 到着ぶんのサンプルを Trend のリングバッファへ足す。
 *
 * 上限を超えた分は**最古から**捨てる(CLAUDE.md のメモリ規律。モニタ系は落としてよいが、
 * 「最新優先」の向きを間違えない = 古いほうを落とす)。呼び手(`slow-view.ts`)は同じ
 * `ts_ms` の payload を二重に足さない責務を持つ(ここは純粋に「足して切り詰める」だけ)。
 */
export function appendSlowSamples(
  history: SlowHistoryMap,
  samples: readonly SlowSample[],
): SlowHistoryMap {
  if (samples.length === 0) return history;
  const next = new Map(history);
  for (const sample of samples) {
    const key = slowSeriesKey(sample.device, sample.node, sample.param);
    const points = next.get(key) ?? [];
    const appended = [...points, { ts_ms: sample.ts_ms, value: sample.value }];
    next.set(
      key,
      appended.length > SLOW_HISTORY_LIMIT
        ? appended.slice(appended.length - SLOW_HISTORY_LIMIT)
        : appended,
    );
  }
  return next;
}

/** Latest 表の 1 行。 */
export interface SlowRow {
  readonly key: string;
  readonly device: string;
  readonly node: number | null;
  readonly param: string;
  readonly value: number;
  readonly unit: string;
  readonly ageMs: number;
  readonly stale: boolean;
  readonly alarm: boolean;
}

/**
 * Latest 表の行を作り、device → node → param の順で安定ソートする(発注書 §2.8-3)。
 * node 無し(`null`)は `-1` 扱いで数値ノードより前に来る。
 */
export function sortedSlowRows(samples: readonly SlowSample[], nowMs: number): SlowRow[] {
  const rows = samples.map((sample) => {
    const ageMs = Math.max(0, nowMs - sample.ts_ms);
    return {
      key: slowSeriesKey(sample.device, sample.node, sample.param),
      device: sample.device,
      node: sample.node,
      param: sample.param,
      value: sample.value,
      unit: sample.unit,
      ageMs,
      stale: ageMs > STALE_AGE_MS,
      alarm: sample.alarm,
    };
  });
  return rows.sort((a, b) => {
    if (a.device !== b.device) return a.device < b.device ? -1 : 1;
    const nodeA = a.node ?? -1;
    const nodeB = b.node ?? -1;
    if (nodeA !== nodeB) return nodeA - nodeB;
    if (a.param !== b.param) return a.param < b.param ? -1 : 1;
    return 0;
  });
}

/** alarm=true のサンプル数(status バーの slow バッジ、発注書 §2.8-4)。 */
export function countAlarms(samples: readonly SlowSample[]): number {
  return samples.filter((sample) => sample.alarm).length;
}

/**
 * alarm=true のサンプルの `device/param` 一覧(status バーの tooltip、発注書 §2.8-4)。
 * node があれば `device/node/param` にする。サンプルの並び順のまま(ソートしない)。
 */
export function alarmLabels(samples: readonly SlowSample[]): string[] {
  return samples
    .filter((sample) => sample.alarm)
    .map((sample) =>
      sample.node === null
        ? `${sample.device}/${sample.param}`
        : `${sample.device}/${sample.node}/${sample.param}`,
    );
}

/**
 * Trend の系列選択を {@link MAX_TREND_SERIES} 本に切り詰める。
 * 落とした本数を返す(silent 禁止 — CLAUDE.md 絶対ルール、モニタ系でも黙って捨てない)。
 */
export function capSeriesSelection(requested: readonly string[]): {
  readonly kept: readonly string[];
  readonly droppedCount: number;
} {
  const kept = requested.slice(0, MAX_TREND_SERIES);
  return { kept, droppedCount: requested.length - kept.length };
}

/** 表示用の丸め(整数はそのまま、小数は 3 桁)。単位換算はしない — 機器の値そのまま(SPEC §9.2)。 */
export function formatSlowValue(value: number): string {
  return Number.isInteger(value) ? String(value) : value.toFixed(3);
}

/** age をミリ秒 → 秒(小数 1 桁)の表示文字列にする。 */
export function formatAgeMs(ageMs: number): string {
  return `${(ageMs / 1000).toFixed(1)} s`;
}
