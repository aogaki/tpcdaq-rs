/**
 * TODO/124 レーン C — Slow control 表の ON/OFF ボタン → 確認ダイアログ → POST → 結果表示。
 * `fetch` だけ差し替え、`RunControlClient` は本物(請求形も確かめる)。
 */
import { type ComponentFixture, TestBed } from '@angular/core/testing';

import { ControllerApiService } from '../api/controller-api';
import { RunControlApiService, RunControlClient } from '../api/run-control-api';
import type { SlowSample } from '../ws/json';
import { WsClientService } from '../ws/ws-client';
import { SlowView } from './slow-view';

const TOKEN = 'tok-abcdef0123456789';

function s(device: string, node: number, param: string, value: number, unit: string): SlowSample {
  return { device, node, param, value, unit, ts_ms: Date.now(), alarm: false };
}

const SAMPLES: SlowSample[] = [
  s('hv', 3, 'vmon', 100, 'V'),
  s('hv', 3, 'status', 33, 'bitmask'),
  s('lv', 1, 'outp', 1, 'on'),
  s('lv', 2, 'outp', 0, 'on'),
];

describe('SlowView — HV / LV ON/OFF', () => {
  let fixture: ComponentFixture<SlowView>;
  let posts: { url: string; body: unknown }[];
  let phase: string;
  let powerReply: { status: number; payload: unknown };

  function el(): HTMLElement {
    return fixture.nativeElement as HTMLElement;
  }
  function rowButton(device: string, ch: number): HTMLButtonElement {
    const rows = [...el().querySelectorAll('table.channels tbody tr')];
    const row = rows.find((r) => {
      const cells = r.querySelectorAll('td');
      return cells[0].textContent?.trim() === device && cells[1].textContent?.trim() === `${ch}`;
    });
    return row?.querySelector('button.power') as HTMLButtonElement;
  }
  async function settle(): Promise<void> {
    await fixture.whenStable();
    fixture.detectChanges();
  }
  function dialogButton(label: string): HTMLButtonElement {
    const found = [...el().querySelectorAll('dialog button')].find(
      (b) => b.textContent?.trim() === label,
    );
    return found as HTMLButtonElement;
  }

  async function mount(withToken: boolean): Promise<void> {
    vi.stubGlobal(
      'ResizeObserver',
      class {
        observe(): void {}
        unobserve(): void {}
        disconnect(): void {}
      },
    );
    const dialog = HTMLDialogElement.prototype as unknown as Record<string, unknown>;
    if (typeof dialog['showModal'] !== 'function') dialog['showModal'] = function noop() {};
    if (typeof dialog['close'] !== 'function') dialog['close'] = function noop() {};

    const fetchFn = (url: string, init?: RequestInit): Promise<Response> => {
      posts.push({ url, body: JSON.parse(String(init?.body ?? 'null')) });
      return Promise.resolve({
        status: powerReply.status,
        json: () => Promise.resolve(powerReply.payload),
      } as unknown as Response);
    };
    const control = new RunControlClient(() => '/api', fetchFn, null);
    if (withToken) control.setToken(TOKEN);
    const api = { getStatus: () => Promise.resolve({ phase, run: null, components: [] }) };
    TestBed.configureTestingModule({
      providers: [
        { provide: ControllerApiService, useValue: api as unknown as ControllerApiService },
        { provide: RunControlApiService, useValue: control as RunControlApiService },
      ],
    });
    fixture = TestBed.createComponent(SlowView);
    TestBed.inject(WsClientService).status.set({
      type: 'status',
      slow: {
        kind: 'slow',
        ts_ms: Date.now(),
        samples: SAMPLES,
        counters: { poll_ok: 1, poll_err: 0, parse_err: 0, reconnects: 0, identity_mismatch: 0 },
      },
    } as never);
    fixture.detectChanges();
    await settle();
  }

  beforeEach(() => {
    posts = [];
    phase = 'Idle';
    powerReply = { status: 200, payload: { ok: true } };
  });
  afterEach(() => fixture.destroy());

  it('token が無ければ全ボタン無効で、理由がツールチップに出る', async () => {
    await mount(false);
    const b = rowButton('hv', 3);
    expect(b.disabled).toBe(true);
    expect(b.title).toBe('acquire control in Run control first');
  });

  it('Running 中: HV OFF は押せる / LV OFF は無効(理由つき)/ LV ON は押せる', async () => {
    phase = 'Running';
    await mount(true);
    expect(rowButton('hv', 3).disabled).toBe(false);
    expect(rowButton('lv', 1).disabled).toBe(true);
    expect(rowButton('lv', 1).title).toContain('stop the run first');
    expect(rowButton('lv', 2).disabled).toBe(false);
  });

  it('HV OFF @ Running: ダイアログに注意が出て、Confirm するまで送らない', async () => {
    phase = 'Running';
    await mount(true);
    rowButton('hv', 3).click();
    await settle();
    const text = el().querySelector('dialog')?.textContent ?? '';
    expect(text).toContain('Turn OFF hv ch 3?');
    expect(text).toContain('Now: ON UNV');
    expect(text).toContain('data of this run ends');
    expect(posts).toEqual([]);
  });

  it('Cancel は送らない', async () => {
    await mount(true);
    rowButton('hv', 3).click();
    await settle();
    dialogButton('Cancel').click();
    await settle();
    expect(posts).toEqual([]);
  });

  it('LV ON: ダイアログに両 ch の before、Confirm で POST、ほかの ch の変化を明示', async () => {
    powerReply = {
      status: 200,
      payload: {
        ok: true,
        before: { outp: { '1': 0, '2': 0 } },
        after: { outp: { '1': 1, '2': 1 } },
      },
    };
    await mount(true);
    rowButton('lv', 2).click();
    await settle();
    const text = el().querySelector('dialog')?.textContent ?? '';
    expect(text).toContain('Turn ON lv ch 2?');
    expect(text).toContain('lv ch 1: ON');
    expect(text).toContain('lv ch 2: OFF');

    dialogButton('Confirm').click();
    await settle();

    expect(posts).toEqual([
      { url: '/api/slow/power', body: { token: TOKEN, device: 'lv', ch: 2, on: true } },
    ]);
    const shown = el().textContent ?? '';
    expect(shown).toContain('lv ch 2 ON: ch 1 OFF → ON, ch 2 OFF → ON');
    expect(shown).toContain('lv ch 1 also changed: OFF → ON');
  });

  it('504: サーバの文言「state unknown — check the table」をそのまま出す', async () => {
    powerReply = { status: 504, payload: { error: 'timeout — state unknown — check the table' } };
    await mount(true);
    rowButton('hv', 3).click();
    await settle();
    dialogButton('Confirm').click();
    await settle();
    expect(el().querySelector('.power-result.fail')?.textContent).toContain(
      'timeout — state unknown — check the table',
    );
  });
});
