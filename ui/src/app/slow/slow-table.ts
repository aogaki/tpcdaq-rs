/**
 * TODO/120 — Latest 表を「機器 × ch を 1 行」に組み替える純ロジック(Angular 非依存)。
 *
 * 入力は slowctl の最新サンプル集合(wire は不変)。出力は
 * - `channels`: node が数値で、列に写る param を持つ (device, node) ごとの 1 行
 * - `other`: 列に写らないサンプル(node 無し・ガス・環境・未知の param)。**消さない**
 *   (silent 禁止)。従来形式の行で返す。
 *
 * 安全境界(SPEC §11): 表示の計算だけ。設定・作動の関数は無い。
 */
import type { SlowSample } from '../ws/json';
import { STALE_AGE_MS, type SlowRow, formatSlowValue, sortedSlowRows } from './slow-history';

/** 値セル。値が無いとき text は `—`、unit は空。 */
export interface ValueCell {
  readonly text: string;
  readonly unit: string;
}

/** ch 表の 1 行。 */
export interface ChannelRow {
  readonly key: string;
  readonly device: string;
  readonly ch: number;
  readonly vmon: ValueCell;
  readonly vset: ValueCell;
  readonly imon: ValueCell;
  readonly iset: ValueCell;
  /** 例 `ON UNV`。status を持たない機器は `—`。 */
  readonly status: string;
  readonly onOff: 'ON' | 'OFF' | null;
  readonly ageMs: number;
  readonly stale: boolean;
  readonly alarm: boolean;
}

export interface SlowTable {
  readonly channels: readonly ChannelRow[];
  readonly other: readonly SlowRow[];
}

/** DT1415ET の STATUS ビット名(`src/slowctl/hivolta.rs` の `ChStatus` と同じ)。 */
const STATUS_BITS: readonly string[] = [
  'ON',
  'UP',
  'DOWN',
  'OVC',
  'OVV',
  'UNV',
  'TRIP',
  'OVP',
  'TWN',
  'OVT',
  'KILL',
  'INTLK',
  'DIS',
  'FAIL',
  'LOCK',
];

const DASH = '—';
const EMPTY_CELL: ValueCell = { text: DASH, unit: '' };

type Column = 'vmon' | 'vset' | 'imon' | 'iset' | 'status' | 'outp';

/** param → 列。ここに無い param は Other へ回る。 */
const COLUMN_OF: Readonly<Record<string, Column>> = {
  vmon: 'vmon',
  volt: 'vmon',
  vset: 'vset',
  imon: 'imon',
  curr: 'imon',
  iset: 'iset',
  status: 'status',
  outp: 'outp',
};

/** status ビットマスク → 文字。0 は `OFF`。名前の無い上位ビットは `B<n>` で見せる(黙って捨てない)。 */
export function formatStatusBits(mask: number): string {
  if (mask === 0) return 'OFF';
  const names: string[] = [];
  for (let bit = 0; bit < 32; bit += 1) {
    if (Math.floor(mask / 2 ** bit) % 2 === 1) names.push(STATUS_BITS[bit] ?? `B${bit}`);
  }
  return names.join(' ');
}

export function buildSlowTable(samples: readonly SlowSample[], nowMs: number): SlowTable {
  const groups = new Map<string, { device: string; ch: number; cols: Map<Column, SlowSample> }>();
  const otherSamples: SlowSample[] = [];

  for (const sample of samples) {
    const column = COLUMN_OF[sample.param];
    if (sample.node === null || column === undefined) {
      otherSamples.push(sample);
      continue;
    }
    const key = `${sample.device}/${sample.node}`;
    let group = groups.get(key);
    if (!group) {
      group = { device: sample.device, ch: sample.node, cols: new Map() };
      groups.set(key, group);
    }
    group.cols.set(column, sample);
  }

  const channels = [...groups.entries()].map(([key, group]): ChannelRow => {
    const members = [...group.cols.values()];
    const ageMs = Math.max(...members.map((sample) => Math.max(0, nowMs - sample.ts_ms)));
    const status = group.cols.get('status');
    const outp = group.cols.get('outp');
    let onOff: 'ON' | 'OFF' | null = null;
    if (outp) onOff = outp.value !== 0 ? 'ON' : 'OFF';
    else if (status) onOff = Math.floor(status.value) % 2 === 1 ? 'ON' : 'OFF';
    return {
      key,
      device: group.device,
      ch: group.ch,
      vmon: cell(group.cols.get('vmon')),
      vset: cell(group.cols.get('vset')),
      imon: cell(group.cols.get('imon')),
      iset: cell(group.cols.get('iset')),
      status: status ? statusText(status) : DASH,
      onOff,
      ageMs,
      stale: ageMs > STALE_AGE_MS,
      alarm: members.some((sample) => sample.alarm),
    };
  });
  channels.sort((a, b) => {
    if (a.device !== b.device) return a.device < b.device ? -1 : 1;
    return a.ch - b.ch;
  });

  return { channels, other: sortedSlowRows(otherSamples, nowMs) };
}

function cell(sample: SlowSample | undefined): ValueCell {
  return sample ? { text: formatSlowValue(sample.value), unit: sample.unit } : EMPTY_CELL;
}

function statusText(sample: SlowSample): string {
  return sample.unit === 'bitmask' ? formatStatusBits(sample.value) : formatSlowValue(sample.value);
}
