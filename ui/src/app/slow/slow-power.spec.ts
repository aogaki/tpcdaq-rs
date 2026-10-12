/**
 * TODO/124 レーン C — ON/OFF ボタンの規則・ダイアログ・結果表示(純関数)。
 */
import type { ActionResult } from '../api/run-control-api';
import {
  NO_TOKEN_REASON,
  buildDialog,
  buttonLabel,
  disabledReason,
  interpretOutcome,
} from './slow-power';
import type { ChannelRow } from './slow-table';

function row(over: Partial<ChannelRow>): ChannelRow {
  return {
    key: 'hv/3',
    device: 'hv',
    ch: 3,
    vmon: { text: '1', unit: 'V' },
    vset: { text: '1', unit: 'V' },
    imon: { text: '1', unit: 'µA' },
    iset: { text: '1', unit: 'µA' },
    status: 'ON UNV',
    onOff: 'ON',
    power: 'hv',
    statusMask: 33,
    ageMs: 0,
    stale: false,
    alarm: false,
    ...over,
  };
}
const lv = (ch: number, onOff: 'ON' | 'OFF'): ChannelRow =>
  row({ key: `lv/${ch}`, device: 'lv', ch, onOff, power: 'lv', statusMask: null, status: '—' });

const ctx = (phase: string | null, hasToken = true, busy = false) => ({ hasToken, phase, busy });

function res(
  status: number,
  body: Record<string, unknown> | null,
  error: string | null = null,
): ActionResult {
  return { ok: status === 200, status, error, notes: [], summary: '', body };
}

describe('disabledReason', () => {
  it('token が無ければ無効 + 取得を促す', () => {
    expect(disabledReason(row({}), ctx('Idle', false))).toBe(NO_TOKEN_REASON);
  });
  it('HV OFF は Running でも押せる、HV ON は Idle 以外で理由つき無効', () => {
    expect(disabledReason(row({ onOff: 'ON' }), ctx('Running'))).toBeNull();
    expect(disabledReason(row({ onOff: 'OFF' }), ctx('Running'))).toContain('Idle');
    expect(disabledReason(row({ onOff: 'OFF' }), ctx('Starting'))).toContain('Starting');
    expect(disabledReason(row({ onOff: 'OFF' }), ctx('Idle'))).toBeNull();
  });
  it('LV ON は常時可、LV OFF は Idle のみ', () => {
    expect(disabledReason(lv(2, 'OFF'), ctx('Running'))).toBeNull();
    expect(disabledReason(lv(1, 'ON'), ctx('Running'))).toContain('stop the run first');
    expect(disabledReason(lv(1, 'ON'), ctx('Idle'))).toBeNull();
  });
  it('相が不明のとき Idle 必須の操作は無効(通す嘘を作らない)', () => {
    expect(disabledReason(lv(1, 'ON'), ctx(null))).toContain('unknown');
  });
  it('送信中は無効', () => {
    expect(disabledReason(row({}), ctx('Idle', true, true))).toContain('in progress');
  });
});

describe('buttonLabel', () => {
  it('ON の行は Turn OFF、OFF の行は Turn ON', () => {
    expect(buttonLabel(row({ onOff: 'ON' }))).toBe('Turn OFF');
    expect(buttonLabel(lv(2, 'OFF'))).toBe('Turn ON');
  });
});

describe('buildDialog', () => {
  it('HV OFF @ Running: タイトル・今の状態・データが終わる注意', () => {
    const d = buildDialog(row({}), [], 'Running');
    expect(d.title).toBe('Turn OFF hv ch 3?');
    expect(d.state).toBe('Now: ON UNV');
    expect(d.warnings.join(' ')).toContain('data of this run ends');
  });
  it('HV OFF @ Idle: 注意は出さない', () => {
    expect(buildDialog(row({}), [], 'Idle').warnings).toEqual([]);
  });
  it('LV: 両 ch の before を出す', () => {
    const rows = [lv(1, 'ON'), lv(2, 'OFF'), row({})];
    const d = buildDialog(lv(2, 'OFF'), rows, 'Idle');
    expect(d.title).toBe('Turn ON lv ch 2?');
    expect(d.lvBefore).toEqual(['lv ch 1: ON', 'lv ch 2: OFF']);
  });
});

describe('interpretOutcome', () => {
  it('HV 成功: before → after のフラグ', () => {
    const out = interpretOutcome(
      row({}),
      false,
      res(200, {
        ok: true,
        before: { status: 33, flags: ['ON', 'UNV'] },
        after: { status: 0, flags: [] },
      }),
    );
    expect(out).toEqual({ ok: true, text: 'hv ch 3 OFF: ON UNV → OFF', notes: [] });
  });
  it('LV 成功: ほかの ch が変わったら明示する', () => {
    const out = interpretOutcome(
      lv(2, 'OFF'),
      true,
      res(200, {
        ok: true,
        before: { outp: { '1': 0, '2': 0 } },
        after: { outp: { '1': 1, '2': 1 } },
      }),
    );
    expect(out.text).toBe('lv ch 2 ON: ch 1 OFF → ON, ch 2 OFF → ON');
    expect(out.notes).toEqual(['lv ch 1 also changed: OFF → ON']);
  });
  it('LV 成功: ほかの ch が変わらなければ notes 空', () => {
    const out = interpretOutcome(
      lv(2, 'OFF'),
      true,
      res(200, {
        ok: true,
        before: { outp: { '1': 1, '2': 0 } },
        after: { outp: { '1': 1, '2': 1 } },
      }),
    );
    expect(out.notes).toEqual([]);
  });
  it('409 / 502 はサーバの文言をそのまま', () => {
    expect(interpretOutcome(row({}), true, res(409, null, 'HV ON only at Idle')).text).toBe(
      'hv ch 3 ON failed (HTTP 409): HV ON only at Idle',
    );
    expect(
      interpretOutcome(row({}), true, res(502, null, 'front switch is OFF (DIS)')).text,
    ).toContain('front switch is OFF (DIS)');
  });
  it('504 は「state unknown — check the table」を必ず含む', () => {
    const withText = interpretOutcome(
      row({}),
      false,
      res(504, null, 'timeout — state unknown — check the table'),
    );
    expect(withText.text).toContain('state unknown — check the table');
    const without = interpretOutcome(row({}), false, res(504, null, null));
    expect(without.text).toContain('state unknown — check the table');
  });
  it('届かなかった(status 0)は not sent', () => {
    expect(interpretOutcome(row({}), false, res(0, null, 'No control token')).text).toContain(
      'not sent',
    );
  });
});
