/**
 * `src/params/schema.rs` の `KNOBS`(サーバ側の唯一の定義)の**写し**
 * (`docs/PARAMS_KNOBS_ja.md` §7)。
 *
 * これは焼き込みではなく写しであることをここに明記する: サーバのスキーマが変われば
 * `knob-spec.spec.ts` が `tests/fixtures/params/params_roundtrip.json` との突き合わせで
 * 赤くなる(= 二重管理の検知)。**最終検証は常にサーバ**(400 の `ParamsError` 文言を
 * そのまま画面に見せる。ここは表示用のヒント(型・範囲・単位)にすぎない)。
 *
 * UI はノブの**スコープを見せない**(078 §0)。`scopeDepth` はここに残すが、
 * フォーム(`params-view`)には描画しない — Preview のハイライト等の内部用途だけに使う。
 */

export type KnobKind = 'int' | 'real' | 'bool' | 'enum';

/** CoBo=0 / AsAd=1 / AGET=2 / channel=3(`src/params/paths.rs::Scope::depth`)。 */
export type ScopeDepth = 0 | 1 | 2 | 3;

export interface KnobField {
  /** JSON の葉のキー(`.` 区切り)。`exceptions[].knob` と同じ表記。 */
  readonly key: string;
  readonly label: string;
  readonly kind: KnobKind;
  readonly scopeDepth: ScopeDepth;
  readonly unit?: string;
  readonly min?: number;
  readonly max?: number;
  /** enum の許容値(`int`/`real`/`str` のいずれか)。 */
  readonly choices?: readonly (string | number)[];
  /**
   * この葉が書き込む xcfg 要素名(末尾セグメント、PARAMS_KNOBS_ja.md §7「xcfg」列)。
   * Preview xcfg で「管理ノブの行」をハイライトする用途だけに使う表示ヒント
   * (レンダラの意味論には使わない — それはサーバの仕事)。
   */
  readonly xcfgElements: readonly string[];
}

export interface KnobGroup {
  /** JSON の `knobs` のトップレベルキー。単純ノブは `fields` が 1 要素。 */
  readonly name: string;
  readonly label: string;
  readonly fields: readonly KnobField[];
}

/** AGET 仕様の 16 値(`src/params/schema.rs::PEAKING_TIME_NS` と同じ、xcfg `range` 属性の写し)。 */
export const PEAKING_TIME_NS: readonly number[] = [
  70, 117, 232, 280, 334, 383, 502, 541, 568, 632, 721, 760, 831, 870, 976, 1014,
];

/** 13 ノブ(PARAMS_KNOBS_ja.md §7)。葉は 16(`zeroSuppression` 2 + `pulser` 3 + 単純 11)。 */
export const KNOB_GROUPS: readonly KnobGroup[] = [
  {
    name: 'triggerDelay',
    label: 'Trigger delay',
    fields: [
      {
        key: 'triggerDelay',
        label: 'Trigger delay',
        kind: 'int',
        scopeDepth: 0,
        unit: '× 10 ns',
        min: 0,
        xcfgElements: ['triggerDelay'],
      },
    ],
  },
  {
    name: 'writingClockFrequency_MHz',
    label: 'Writing clock frequency',
    fields: [
      {
        key: 'writingClockFrequency_MHz',
        label: 'Writing clock frequency',
        kind: 'enum',
        scopeDepth: 0,
        unit: 'MHz',
        choices: [12.5, 25, 50, 100],
        xcfgElements: ['writingClockFrequency'],
      },
    ],
  },
  {
    name: 'triggerMode',
    label: 'Trigger mode',
    fields: [
      {
        key: 'triggerMode',
        label: 'Trigger mode',
        kind: 'enum',
        scopeDepth: 0,
        choices: ['external', 'noTrigger', 'onRequest', 'onSlowControl', 'periodically'],
        xcfgElements: ['triggerMode'],
      },
    ],
  },
  {
    name: 'triggerPeriod_ms',
    label: 'Trigger period (periodically only)',
    fields: [
      {
        key: 'triggerPeriod_ms',
        label: 'Trigger period (periodically only)',
        kind: 'int',
        scopeDepth: 0,
        unit: 'ms',
        // 上限 21474 ms: GET は 100000 * ms を int で計算する(CoBoNode::setTriggerPeriod)。TODO/179。
        min: 1,
        max: 21474,
        xcfgElements: ['triggerPeriod'],
      },
    ],
  },
  {
    name: 'globalThreshold',
    label: 'Global threshold',
    fields: [
      {
        key: 'globalThreshold',
        label: 'Global threshold',
        kind: 'int',
        scopeDepth: 2,
        min: 0,
        max: 7,
        xcfgElements: ['GlobalThresholdValue'],
      },
    ],
  },
  {
    name: 'peakingTime_ns',
    label: 'Peaking time',
    fields: [
      {
        key: 'peakingTime_ns',
        label: 'Peaking time',
        kind: 'enum',
        scopeDepth: 2,
        unit: 'ns',
        choices: PEAKING_TIME_NS,
        xcfgElements: ['peackingTime'], // GET 側の綴りのまま(078 レーン A 結果節 §4-1)
      },
    ],
  },
  {
    name: 'readout',
    label: 'Readout',
    fields: [
      {
        key: 'readout',
        label: 'Readout',
        kind: 'enum',
        scopeDepth: 2,
        choices: ['full', 'partial'],
        // full/partial は isAllChannelRead だけ(111 §5.4 で isFPNRead を fpnRead に分けた。
        // `enableWriteHittedregister` / `channel.Reading` は非管理)。
        xcfgElements: ['isAllChannelRead'],
      },
    ],
  },
  {
    name: 'fpnRead',
    label: 'Read FPN channels',
    fields: [
      {
        key: 'fpnRead',
        label: 'Read FPN channels',
        kind: 'bool',
        scopeDepth: 2,
        xcfgElements: ['isFPNRead'],
      },
    ],
  },
  {
    name: 'lsbThreshold',
    label: 'LSB threshold',
    fields: [
      {
        key: 'lsbThreshold',
        label: 'LSB threshold',
        kind: 'int',
        scopeDepth: 3,
        min: 0,
        max: 15,
        xcfgElements: ['LSBThresholdValue'],
      },
    ],
  },
  {
    name: 'gain',
    label: 'Gain',
    fields: [
      {
        key: 'gain',
        label: 'Gain',
        kind: 'enum',
        scopeDepth: 3,
        choices: ['10pC', '120fC', '1pC', '240fC'],
        xcfgElements: ['Gain'],
      },
    ],
  },
  {
    name: 'triggerInhibition',
    label: 'Trigger inhibition',
    fields: [
      {
        key: 'triggerInhibition',
        label: 'Trigger inhibition',
        kind: 'enum',
        scopeDepth: 3,
        choices: ['inhibit_channel', 'inhibit_trigger', 'none'],
        xcfgElements: ['TriggerInhibition'],
      },
    ],
  },
  {
    name: 'zeroSuppression',
    label: 'Zero suppression',
    fields: [
      {
        key: 'zeroSuppression.enabled',
        label: 'Zero suppression enabled',
        kind: 'bool',
        scopeDepth: 0,
        xcfgElements: ['enableZeroSuppression'],
      },
      {
        key: 'zeroSuppression.threshold',
        label: 'Zero suppression threshold',
        kind: 'int',
        scopeDepth: 3,
        min: 0,
        max: 4095,
        xcfgElements: ['zeroSuppressionThreshold'],
      },
    ],
  },
  {
    name: 'pulser',
    label: 'Pulser',
    fields: [
      {
        key: 'pulser.enabled',
        label: 'Pulser enabled',
        kind: 'bool',
        scopeDepth: 0,
        xcfgElements: ['periodicModeEnabled'],
      },
      {
        key: 'pulser.voltage_mV',
        label: 'Pulser voltage',
        kind: 'real',
        scopeDepth: 0,
        unit: 'mV',
        min: -1000,
        max: 1000,
        xcfgElements: ['amplitudeStart'],
      },
      {
        key: 'pulser.period_ms',
        label: 'Pulser period',
        kind: 'int',
        scopeDepth: 0,
        unit: 'ms',
        min: 0,
        xcfgElements: ['period_ms'],
      },
    ],
  },
];

/** 16 の葉(表示・往復用のフラット版)。 */
export const KNOB_FIELDS: readonly KnobField[] = KNOB_GROUPS.flatMap((group) => group.fields);

/** 全 [`KnobField.xcfgElements`] を集めたもの(Preview xcfg のハイライト判定に使う)。 */
export const MANAGED_XCFG_ELEMENTS: readonly string[] = KNOB_FIELDS.flatMap(
  (field) => field.xcfgElements,
);

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function lookupPath(source: Record<string, unknown>, path: string): unknown {
  let value: unknown = source;
  for (const part of path.split('.')) {
    if (!isPlainObject(value)) return undefined;
    value = value[part];
  }
  return value;
}

/**
 * ネストした `knobs` オブジェクト(サーバ / フィクスチャの実際の形、`zeroSuppression: {enabled,
 * threshold}` のように入れ子)→ 葉のキー(`.` 区切り)→ 値のフラットな `Map`。
 * `KNOB_FIELDS` に無いキーは無視する(未知キー検出はサーバの仕事 — ここは表示用)。
 */
export function flattenKnobs(knobs: Record<string, unknown>): Map<string, unknown> {
  const out = new Map<string, unknown>();
  for (const field of KNOB_FIELDS) {
    const value = lookupPath(knobs, field.key);
    if (value !== undefined) out.set(field.key, value);
  }
  return out;
}

/** フラットな `Map`(`.` 区切りキー→値)→ ネストした `knobs` オブジェクト(PUT の body 用)。 */
export function nestKnobs(flat: ReadonlyMap<string, unknown>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [key, value] of flat) {
    const parts = key.split('.');
    let node = out;
    for (let i = 0; i < parts.length - 1; i++) {
      const part = parts[i];
      const child = node[part];
      if (!isPlainObject(child)) node[part] = {};
      node = node[part] as Record<string, unknown>;
    }
    node[parts[parts.length - 1]] = value;
  }
  return out;
}

/** 名前(JSON の葉のキー)から [`KnobField`] を引く。 */
export function knobField(key: string): KnobField | undefined {
  return KNOB_FIELDS.find((field) => field.key === key);
}
