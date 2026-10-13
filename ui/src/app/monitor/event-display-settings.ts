/**
 * Event Display の投影閾値(TODO/125)。ブラウザごとに localStorage へ記憶する。
 *
 * localStorage は private window / サイトデータ拒否 / プレビューで throw または空になりうるので、
 * 読み書きは全部 try/catch。**使えなくても既定値で動く**(閾値欄は動き、ただ記憶されないだけ)。
 */
import { DEFAULT_HIT_THRESHOLD } from './xy-projection';

export const THRESHOLD_STORAGE_KEY = 'tpcdaq.eventDisplay.threshold';

/** 閾値として使える値(有限で 0 以上)。 */
export function isValidThreshold(value: number): boolean {
  return Number.isFinite(value) && value >= 0;
}

/** 保存済みの閾値。無い・壊れている・storage が使えない → 既定値。 */
export function loadThreshold(): number {
  try {
    const raw = localStorage.getItem(THRESHOLD_STORAGE_KEY);
    if (raw === null || raw.trim() === '') return DEFAULT_HIT_THRESHOLD;
    const value = Number(raw);
    return isValidThreshold(value) ? value : DEFAULT_HIT_THRESHOLD;
  } catch {
    return DEFAULT_HIT_THRESHOLD;
  }
}

/** 閾値を記憶する。失敗しても黙って続ける(記憶は便宜であって必須ではない)。 */
export function saveThreshold(value: number): void {
  try {
    localStorage.setItem(THRESHOLD_STORAGE_KEY, String(value));
  } catch {
    // 使えないブラウザ設定でも画面は動く。
  }
}
