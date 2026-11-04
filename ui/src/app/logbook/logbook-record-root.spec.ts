/**
 * TODO/187 ⑤: run_stop の `counters.root_complete === false` に「ROOT incomplete」の印。
 * フィクスチャは logbook-record.spec.ts の RUN_STOP(Rust 側 golden)から作る。
 */
import { type LogbookEntry, parseLogbookEntry } from './logbook-record';

const RUN_STOP = JSON.parse(
  '{"ts":"2026-08-12T18:00:00.000+03:00","seq":7,"type":"run_stop","actor":"controller","run":58,"duration_s":12.5,"ok":true,"reason":"normal","counters":{"events_built":null,"events_incomplete":null,"late_fragments":null,"frames":{"0":3852,"1":3849},"overflow_frames":3,"malformed":4},"files":[]}',
) as Record<string, unknown>;

function parsed(raw: unknown): LogbookEntry {
  const entry = parseLogbookEntry(raw);
  if (!entry) throw new Error('期待に反して取り込めなかった');
  return entry;
}

function stopWith(rootComplete: unknown, extra: Record<string, unknown> = {}): unknown {
  const counters = { ...(RUN_STOP['counters'] as Record<string, unknown>) };
  if (rootComplete !== undefined) counters['root_complete'] = rootComplete;
  return { ...RUN_STOP, counters, ...extra };
}

function fieldValue(entry: LogbookEntry, label: string): string | undefined {
  return entry.fields.find((f) => f.label === label)?.value;
}

describe('TODO/187 — ROOT incomplete の印', () => {
  it('logbook record marks run_stop rows with root_complete false', () => {
    const entry = parsed(stopWith(false));
    expect(fieldValue(entry, 'ROOT')).toContain('incomplete');
    // 印だけ。graw は完全なので run 全体の severity は格上げしない。
    expect(entry.severity).toBe('normal');
    // true / null / キー無し(古い run)は印なし。
    for (const value of [true, null, undefined]) {
      expect(fieldValue(parsed(stopWith(value)), 'ROOT'), String(value)).toBeUndefined();
    }
  });

  it('reason が normal 以外でも reason は今までどおり出て、印と独立に並ぶ', () => {
    const entry = parsed(stopWith(false, { reason: 'error:eos-timeout', ok: false }));
    expect(fieldValue(entry, 'reason')).toBe('error:eos-timeout');
    expect(fieldValue(entry, 'ROOT')).toContain('incomplete');
    expect(entry.severity).toBe('error');
  });
});
