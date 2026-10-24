/**
 * `KNOB_FIELDS` はサーバ(`src/params/schema.rs::KNOBS`)の**写し**でしかない。
 * ここでは `tests/fixtures/params/params_roundtrip.json`(078 レーン A の往復フィクスチャ、
 * サーバ側テストが同じファイルをオラクル照合済み)と突き合わせて、写し損ないを検知する
 * (078 §11「サーバ側の表が変わったら赤になる = 二重管理の検知」)。
 */
import { readFileSync } from 'node:fs';

import {
  KNOB_FIELDS,
  KNOB_GROUPS,
  PEAKING_TIME_NS,
  flattenKnobs,
  knobField,
  nestKnobs,
} from './knob-spec';

// リポ直下の合成フィクスチャ(コミット済み、実データではない)。`ui/` から見た相対パス。
const FIXTURE_PATH = '../tests/fixtures/params/params_roundtrip.json';

function loadFixtureKnobs(): Record<string, unknown> {
  const raw = JSON.parse(readFileSync(FIXTURE_PATH, 'utf-8')) as { knobs: Record<string, unknown> };
  return raw.knobs;
}

describe('KNOB_FIELDS ↔ params_roundtrip.json(過不足なし)', () => {
  it('13 グループ・16 葉(11 単純 + zeroSuppression 2 + pulser 3)', () => {
    expect(KNOB_GROUPS).toHaveLength(13);
    expect(KNOB_FIELDS).toHaveLength(16);
  });

  it('KNOB_FIELDS のキー集合 = フィクスチャの knobs を平らにしたキー集合(過不足なし)', () => {
    const fixtureKeys = [...flattenKnobs(loadFixtureKnobs()).keys()].sort();
    const tableKeys = KNOB_FIELDS.map((field) => field.key).sort();
    expect(tableKeys).toEqual(fixtureKeys);
  });

  it('全ノブがフィクスチャに実在する値を持つ(未定義が無い)', () => {
    const flat = flattenKnobs(loadFixtureKnobs());
    for (const field of KNOB_FIELDS) {
      expect(flat.has(field.key), `missing: ${field.key}`).toBe(true);
    }
  });
});

describe('flattenKnobs / nestKnobs — 往復', () => {
  it('フィクスチャの knobs を平らにしてまたネストすると元と一致する', () => {
    const knobs = loadFixtureKnobs();
    const flat = flattenKnobs(knobs);
    expect(nestKnobs(flat)).toEqual(knobs);
  });

  it('flattenKnobs は KNOB_FIELDS に無いキーを無視する(未知キー検出はサーバの仕事)', () => {
    const flat = flattenKnobs({ ...loadFixtureKnobs(), extraUnknownKnob: 1 });
    expect(flat.has('extraUnknownKnob')).toBe(false);
  });

  it('nestKnobs は複合ノブ(zeroSuppression/pulser)を正しく組み立てる', () => {
    const flat = new Map<string, unknown>([
      ['triggerDelay', 1748],
      ['zeroSuppression.enabled', true],
      ['zeroSuppression.threshold', 42],
      ['pulser.enabled', false],
      ['pulser.voltage_mV', -500],
      ['pulser.period_ms', 2000],
    ]);
    expect(nestKnobs(flat)).toEqual({
      triggerDelay: 1748,
      zeroSuppression: { enabled: true, threshold: 42 },
      pulser: { enabled: false, voltage_mV: -500, period_ms: 2000 },
    });
  });
});

describe('個別ノブの型・範囲(docs/PARAMS_KNOBS_ja.md §7 の写し)', () => {
  it('peakingTime_ns の選択肢 = AGET 仕様の 16 値', () => {
    expect(knobField('peakingTime_ns')?.choices).toEqual(PEAKING_TIME_NS);
    expect(PEAKING_TIME_NS).toHaveLength(16);
  });

  it('gain / triggerMode / triggerInhibition / readout は enum(choices を持つ)', () => {
    expect(knobField('gain')?.choices).toEqual(['10pC', '120fC', '1pC', '240fC']);
    expect(knobField('triggerMode')?.choices).toEqual([
      'external',
      'noTrigger',
      'onRequest',
      'onSlowControl',
      'periodically',
    ]);
    expect(knobField('triggerInhibition')?.choices).toEqual([
      'inhibit_channel',
      'inhibit_trigger',
      'none',
    ]);
    expect(knobField('readout')?.choices).toEqual(['full', 'partial']);
  });

  it('lsbThreshold [0,15] / globalThreshold [0,7] / zeroSuppression.threshold [0,4095]', () => {
    expect(knobField('lsbThreshold')).toMatchObject({ kind: 'int', min: 0, max: 15 });
    expect(knobField('globalThreshold')).toMatchObject({ kind: 'int', min: 0, max: 7 });
    expect(knobField('zeroSuppression.threshold')).toMatchObject({
      kind: 'int',
      min: 0,
      max: 4095,
    });
  });

  it('triggerPeriod_ms は int [1, 21474] ms、Module.triggerPeriod、triggerMode の直後(TODO/179)', () => {
    expect(knobField('triggerPeriod_ms')).toMatchObject({
      kind: 'int',
      scopeDepth: 0,
      unit: 'ms',
      min: 1,
      max: 21474,
      xcfgElements: ['triggerPeriod'],
    });
    const keys = KNOB_FIELDS.map((field) => field.key);
    expect(keys.indexOf('triggerPeriod_ms')).toBe(keys.indexOf('triggerMode') + 1);
  });

  it('zeroSuppression.enabled / pulser.enabled / fpnRead は bool', () => {
    expect(knobField('zeroSuppression.enabled')?.kind).toBe('bool');
    expect(knobField('pulser.enabled')?.kind).toBe('bool');
    expect(knobField('fpnRead')?.kind).toBe('bool');
  });

  it('readout は isAllChannelRead だけ、fpnRead は isFPNRead(111 §5.4 で分けた)', () => {
    expect(knobField('readout')?.xcfgElements).toEqual(['isAllChannelRead']);
    expect(knobField('fpnRead')).toMatchObject({
      label: 'Read FPN channels',
      scopeDepth: 2,
      xcfgElements: ['isFPNRead'],
    });
    // フォームの並びは readout の直後。
    const keys = KNOB_FIELDS.map((field) => field.key);
    expect(keys.indexOf('fpnRead')).toBe(keys.indexOf('readout') + 1);
  });

  it('未知のキーは undefined', () => {
    expect(knobField('doesNotExist')).toBeUndefined();
  });

  it('スコープ深さは CoBo=0 / AGET=2 / channel=3 のみ(AsAd=1 のノブは無い — §7)', () => {
    const depths = new Set(KNOB_FIELDS.map((field) => field.scopeDepth));
    expect([...depths].sort()).toEqual([0, 2, 3]);
  });
});
