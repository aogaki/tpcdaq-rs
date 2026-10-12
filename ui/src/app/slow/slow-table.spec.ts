/**
 * TODO/120 — Latest 表の「機器 × ch を 1 行」変換(純関数)と、その描画。
 */
import { TestBed } from '@angular/core/testing';

import type { SlowSample } from '../ws/json';
import { WsClientService } from '../ws/ws-client';
import { buildSlowTable, formatStatusBits } from './slow-table';
import { SlowView } from './slow-view';

const NOW = 100_000;

function sample(
  device: string,
  node: number | null,
  param: string,
  value: number,
  unit: string,
  extra: Partial<SlowSample> = {},
): SlowSample {
  return { device, node, param, value, unit, ts_ms: NOW - 500, alarm: false, ...extra };
}

/** hv ch0–7(ch0 は status 33 = ON + UNV で alarm)+ lv ch1–2(outp 1 / 0)+ bdalarm / bdilk。 */
function fixtureSamples(): SlowSample[] {
  const out: SlowSample[] = [];
  for (let ch = 0; ch < 8; ch += 1) {
    const status = ch === 0 ? 33 : 1;
    out.push(sample('hv', ch, 'vmon', 100 + ch + 0.25, 'V'));
    out.push(sample('hv', ch, 'imon', 0.5 + ch, 'µA'));
    out.push(sample('hv', ch, 'status', status, 'bitmask', { alarm: ch === 0 }));
    out.push(sample('hv', ch, 'vset', 200 + ch, 'V'));
  }
  out.push(sample('lv', 1, 'volt', 5.5, 'V'));
  out.push(sample('lv', 1, 'curr', 0.125, 'A'));
  out.push(sample('lv', 1, 'outp', 1, 'on'));
  out.push(sample('lv', 2, 'volt', 3.3, 'V'));
  out.push(sample('lv', 2, 'curr', 0, 'A'));
  out.push(sample('lv', 2, 'outp', 0, 'on'));
  out.push(sample('hv', null, 'bdalarm', 0, 'bitmask'));
  out.push(sample('hv', null, 'bdilk', 1, 'flag'));
  return out;
}

describe('formatStatusBits — DT1415ET STATUS ビット', () => {
  it('33 = bit0 + bit5 → "ON UNV"、1 → "ON"、0 → "OFF"', () => {
    expect(formatStatusBits(33)).toBe('ON UNV');
    expect(formatStatusBits(1)).toBe('ON');
    expect(formatStatusBits(0)).toBe('OFF');
  });
  it('名前の無いビットも捨てない(bit 15 → B15)', () => {
    expect(formatStatusBits(1 + 32768)).toBe('ON B15');
  });
});

describe('buildSlowTable', () => {
  const table = buildSlowTable(fixtureSamples(), NOW);

  it('hv 8 ch + lv 2 ch = 10 行、device → ch の昇順', () => {
    expect(table.channels).toHaveLength(10);
    expect(table.channels.map((row) => `${row.device}/${row.ch}`)).toEqual([
      'hv/0',
      'hv/1',
      'hv/2',
      'hv/3',
      'hv/4',
      'hv/5',
      'hv/6',
      'hv/7',
      'lv/1',
      'lv/2',
    ]);
  });

  it('hv ch0 の Status は "ON UNV" で alarm、ON', () => {
    const row = table.channels[0];
    expect(row.status).toBe('ON UNV');
    expect(row.alarm).toBe(true);
    expect(row.onOff).toBe('ON');
    expect(table.channels[1].alarm).toBe(false);
  });

  it('hv の値は単位つき、Vset 200(ch0)・Imon 0.5 µA', () => {
    const row = table.channels[0];
    expect(row.vset).toEqual({ text: '200', unit: 'V' });
    expect(row.imon).toEqual({ text: '0.500', unit: 'µA' });
  });

  it('lv の Vmon は volt、Imon は curr(単位 A)から来る。status は —', () => {
    const lv1 = table.channels[8];
    expect(lv1.vmon).toEqual({ text: '5.500', unit: 'V' });
    expect(lv1.imon).toEqual({ text: '0.125', unit: 'A' });
    expect(lv1.status).toBe('—');
  });

  it('lv ch2 は outp 0 → OFF、ch1 は ON', () => {
    expect(table.channels[9].onOff).toBe('OFF');
    expect(table.channels[8].onOff).toBe('ON');
  });

  it('Iset / Vset(lv)が無い入力では —', () => {
    expect(table.channels[0].iset).toEqual({ text: '—', unit: '' });
    expect(table.channels[8].vset).toEqual({ text: '—', unit: '' });
  });

  it('Iset / Vset(lv)があれば拾う', () => {
    const withSet = buildSlowTable(
      [...fixtureSamples(), sample('hv', 0, 'iset', 20, 'µA'), sample('lv', 1, 'vset', 5, 'V')],
      NOW,
    );
    expect(withSet.channels[0].iset).toEqual({ text: '20', unit: 'µA' });
    expect(withSet.channels[8].vset).toEqual({ text: '5', unit: 'V' });
  });

  it('bdalarm / bdilk は Other に出る(消えない)', () => {
    expect(table.other.map((row) => row.param)).toEqual(['bdalarm', 'bdilk']);
  });

  it('列に写らない param(ガス等)は Other、行の age は最古、5 s 超で stale', () => {
    const t = buildSlowTable(
      [
        sample('gas', 3, 'fmeasure', 130, 'ml/min'),
        sample('hv', 0, 'vmon', 1, 'V', { ts_ms: NOW - 9000 }),
        sample('hv', 0, 'imon', 1, 'µA'),
      ],
      NOW,
    );
    expect(t.channels).toHaveLength(1);
    expect(t.other.map((row) => row.device)).toEqual(['gas']);
    expect(t.channels[0].ageMs).toBe(9000);
    expect(t.channels[0].stale).toBe(true);
  });

  it('status が bitmask でなければ数値のまま', () => {
    const t = buildSlowTable([sample('x', 1, 'status', 7, 'code')], NOW);
    expect(t.channels[0].status).toBe('7');
  });

  it('outp も status も無ければ ON/OFF は null', () => {
    const t = buildSlowTable([sample('x', 1, 'vmon', 1, 'V')], NOW);
    expect(t.channels[0].onOff).toBeNull();
  });
});

describe('SlowView — Latest 表の描画', () => {
  it('ch 表 10 行 + Other 表 2 行、ch0 に ALARM バッジ', () => {
    // jsdom に ResizeObserver が無い(Trend のチャート用。ここでは使わない)。
    vi.stubGlobal(
      'ResizeObserver',
      class {
        observe(): void {}
        unobserve(): void {}
        disconnect(): void {}
      },
    );
    TestBed.configureTestingModule({});
    const fixture = TestBed.createComponent(SlowView);
    const ws = TestBed.inject(WsClientService);
    const now = Date.now();
    ws.status.set({
      type: 'status',
      slow: {
        kind: 'slow',
        ts_ms: now,
        samples: fixtureSamples().map((s) => ({ ...s, ts_ms: now })),
        counters: { poll_ok: 1, poll_err: 0, parse_err: 0, reconnects: 0, identity_mismatch: 0 },
      },
    } as never);
    fixture.detectChanges();
    const el: HTMLElement = fixture.nativeElement;
    expect(el.querySelectorAll('table.channels tbody tr')).toHaveLength(10);
    expect(el.querySelectorAll('table.other tbody tr')).toHaveLength(2);
    expect(el.querySelector('table.channels tbody tr')?.textContent).toContain('ON UNV');
    expect(el.querySelectorAll('table.channels .badge.alarm')).toHaveLength(1);
    fixture.destroy();
  });
});
