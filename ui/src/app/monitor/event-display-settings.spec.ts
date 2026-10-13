/**
 * TODO/125 レーン B 項目 3/5 — 閾値欄の記憶(localStorage)。使えなくても既定値で動く。
 */
import { vi } from 'vitest';

import {
  THRESHOLD_STORAGE_KEY,
  isValidThreshold,
  loadThreshold,
  saveThreshold,
} from './event-display-settings';
import { memoryStorage } from './xy-test-fixtures';
import { DEFAULT_HIT_THRESHOLD } from './xy-projection';

describe('閾値の記憶(localStorage キー tpcdaq.eventDisplay.threshold)', () => {
  let storage: Storage;

  beforeEach(() => {
    storage = memoryStorage();
    vi.stubGlobal('localStorage', storage);
  });
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it('キー名と既定値(50 ADC)', () => {
    expect(THRESHOLD_STORAGE_KEY).toBe('tpcdaq.eventDisplay.threshold');
    expect(loadThreshold()).toBe(50);
    expect(DEFAULT_HIT_THRESHOLD).toBe(50);
  });

  it('保存した値を読み戻す(小数も)', () => {
    saveThreshold(120);
    expect(storage.getItem(THRESHOLD_STORAGE_KEY)).toBe('120');
    expect(loadThreshold()).toBe(120);
    saveThreshold(37.5);
    expect(loadThreshold()).toBe(37.5);
  });

  it('壊れた保存値(非数・負・空)は既定値に戻す', () => {
    for (const raw of ['abc', '-5', '', '  ', 'NaN', 'Infinity']) {
      storage.setItem(THRESHOLD_STORAGE_KEY, raw);
      expect(loadThreshold(), raw).toBe(DEFAULT_HIT_THRESHOLD);
    }
  });

  it('getItem が throw しても既定値で動く', () => {
    vi.spyOn(storage, 'getItem').mockImplementation(() => {
      throw new Error('SecurityError: storage blocked');
    });
    expect(loadThreshold()).toBe(DEFAULT_HIT_THRESHOLD);
  });

  it('setItem が throw しても例外を出さない', () => {
    vi.spyOn(storage, 'setItem').mockImplementation(() => {
      throw new Error('QuotaExceededError');
    });
    expect(() => saveThreshold(80)).not.toThrow();
  });

  it('localStorage 自体が無い(undefined)環境でも既定値で動く', () => {
    vi.stubGlobal('localStorage', undefined);
    expect(loadThreshold()).toBe(DEFAULT_HIT_THRESHOLD);
    expect(() => saveThreshold(80)).not.toThrow();
  });

  it('使える値は有限で 0 以上', () => {
    expect(isValidThreshold(0)).toBe(true);
    expect(isValidThreshold(50)).toBe(true);
    expect(isValidThreshold(-1)).toBe(false);
    expect(isValidThreshold(Number.NaN)).toBe(false);
    expect(isValidThreshold(Number.POSITIVE_INFINITY)).toBe(false);
  });
});
