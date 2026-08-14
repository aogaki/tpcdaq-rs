/**
 * R13 / SPEC §10.2 `0x03 Waveforms` — 波形ビューの選択と**クライアント側間引き**。
 *
 * Angular に依存しない純ロジック(vitest から直接 import する)。
 *
 * - 生 ADC・FPN 込み・減算なし(R13)を**そのまま**返す。加工しない。
 * - `nAget` / `nCh` / `nBuckets` はメッセージ由来(焼き込み禁止 = プロジェクト不変条件)。
 * - 間引きは「落としてよいが silent にしない」側。**落とした数を戻り値に載せる**。
 */

import type { WaveformsMessage } from '../ws/wire';

/** ECharts の `data: [[x, y], …]` にそのまま渡せる `[bucket, adc]` の組。 */
export type WaveformPoint = [number, number];

export interface WaveformSeries {
  readonly aget: number;
  readonly ch: number;
  /** stride 間引き後の点。x は**間引く前の bucket 番号**(横軸がずれないように)。 */
  readonly points: WaveformPoint[];
}

export interface WaveformSelection {
  /** 表示する AGET(メッセージの nAget 未満のものだけ有効)。 */
  readonly agets: readonly number[];
  /** 表示する raw ch(メッセージの nCh 未満のものだけ有効)。 */
  readonly channels: readonly number[];
  /** 描画する系列数の上限。 */
  readonly maxSeries: number;
  /** 1 系列あたりの点数の上限(超えたら stride で間引く)。 */
  readonly maxPoints: number;
}

export interface WaveformPlot {
  readonly series: readonly WaveformSeries[];
  /** メッセージ由来の bucket 数(横軸の上限)。 */
  readonly nBuckets: number;
  /** サンプルの間引き幅(1 なら間引いていない)。 */
  readonly stride: number;
  /** 有効(範囲内)だった要求系列数。 */
  readonly requestedSeries: number;
  /** 上限を超えて**描かなかった**系列数。 */
  readonly droppedSeries: number;
  /** 1 系列あたり**捨てた**サンプル点数。 */
  readonly droppedSamplesPerSeries: number;
  /** メッセージの nAget/nCh の外を指していた (aget,ch) の組の数。 */
  readonly outOfRange: number;
}

/** (cobo,asad) のキー。 */
export function waveformKey(cobo: number, asad: number): string {
  return `cobo${cobo}/asad${asad}`;
}

/**
 * (cobo,asad) 毎の最新を覚える。
 *
 * `WsClientService.waveforms()` は **最新 1 通**しか持たない(027 の設計)。
 * ELITPC は 1 イベントで 4 AsAd 分が続けて届くので、(cobo,asad) を選ばせるには
 * view 側で面毎に覚えるしかない。ここはその純ロジック。
 */
export function updateWaveformCache(
  cache: ReadonlyMap<string, WaveformsMessage>,
  message: WaveformsMessage,
): Map<string, WaveformsMessage> {
  const next = new Map(cache);
  next.set(waveformKey(message.cobo, message.asad), message);
  return next;
}

/** 選択 → 間引き。例外は投げない(壊れた選択も数えて返す)。 */
export function selectWaveforms(
  message: WaveformsMessage,
  selection: WaveformSelection,
): WaveformPlot {
  const { nAget, nCh, nBuckets } = message;

  const wanted: { aget: number; ch: number }[] = [];
  let outOfRange = 0;
  for (const aget of selection.agets) {
    for (const ch of selection.channels) {
      if (aget >= 0 && aget < nAget && ch >= 0 && ch < nCh) wanted.push({ aget, ch });
      else outOfRange += 1;
    }
  }

  const maxSeries = Math.max(0, Math.floor(selection.maxSeries));
  const drawn = wanted.slice(0, maxSeries);
  const droppedSeries = wanted.length - drawn.length;

  const maxPoints = Math.max(1, Math.floor(selection.maxPoints));
  const stride = Math.max(1, Math.ceil(nBuckets / maxPoints));

  const series = drawn.map(({ aget, ch }) => {
    const base = (aget * nCh + ch) * nBuckets;
    const points: WaveformPoint[] = [];
    for (let bucket = 0; bucket < nBuckets; bucket += stride) {
      points.push([bucket, message.adc[base + bucket]]);
    }
    return { aget, ch, points };
  });

  const keptSamples = series.length === 0 ? Math.ceil(nBuckets / stride) : series[0].points.length;

  return {
    series,
    nBuckets,
    stride,
    requestedSeries: wanted.length,
    droppedSeries,
    droppedSamplesPerSeries: nBuckets - keptSamples,
    outOfRange,
  };
}
