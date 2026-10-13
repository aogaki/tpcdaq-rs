/**
 * TODO/125 レーン B 項目 4/5 — AsAd 電子回路座標パネル(077)は Waveform ビューに出る。
 * `≠ UVW` の表示も保つ。WS は最小の fake、JsrootPanel は名前だけ見えるスタブ。
 */
import { Component, input, signal } from '@angular/core';
import { type ComponentFixture, TestBed } from '@angular/core/testing';
import { vi } from 'vitest';

import { DisplayClock } from '../display/display-clock';
import { JsrootPanel } from '../monitor/jsroot-panel';
import type { PanelMessage, PanelSpec } from '../monitor/root-histo';
import type { UvwMessage, WaveformsMessage } from '../ws/wire';
import { WsClientService } from '../ws/ws-client';
import { WaveformView } from './waveform-view';

@Component({
  selector: 'app-jsroot-panel',
  template: `<i
    class="stub"
    [attr.data-name]="spec().name"
    [attr.data-kind]="message()?.kind"
  ></i>`,
})
class StubPanel {
  readonly spec = input.required<PanelSpec>();
  readonly message = input<PanelMessage | null>(null);
}

function waveforms(cobo: number, asad: number, eventNumber: number): WaveformsMessage {
  return {
    kind: 'waveforms',
    header: { msgType: 3, incomplete: false, runNumber: 7, eventNumber },
    cobo,
    asad,
    nAget: 2,
    nCh: 3,
    nBuckets: 5,
    adc: new Uint16Array(2 * 3 * 5).fill(100),
  };
}

function uvw(eventNumber: number): UvwMessage {
  return {
    kind: 'uvw',
    header: { msgType: 2, incomplete: false, runNumber: 7, eventNumber },
    plane: 0,
    nStrips: 2,
    nBuckets: 5,
    adc: new Uint16Array(10),
  };
}

describe('WaveformView — AsAd 電子回路座標パネル(077、TODO/125 で monitor タブから移設)', () => {
  let fixture: ComponentFixture<WaveformView>;
  let clock: DisplayClock;
  const ws = {
    waveforms: signal<WaveformsMessage | null>(null),
    uvwByPlane: signal<readonly (UvwMessage | null)[]>([null, null, null]),
    setWaveforms: vi.fn(),
  };

  async function settle(): Promise<void> {
    for (let i = 0; i < 3; i++) {
      fixture.detectChanges();
      await new Promise((resolve) => setTimeout(resolve, 0));
      await fixture.whenStable();
    }
  }

  beforeEach(() => {
    // jsdom に ResizeObserver は無い(パネルの大きさ監視は本テストの関心外)。
    vi.stubGlobal(
      'ResizeObserver',
      class {
        observe(): void {}
        unobserve(): void {}
        disconnect(): void {}
      },
    );
    ws.waveforms.set(null);
    ws.uvwByPlane.set([null, null, null]);
    ws.setWaveforms.mockClear();
    TestBed.configureTestingModule({
      providers: [{ provide: WsClientService, useValue: ws as unknown as WsClientService }],
    });
    TestBed.overrideComponent(WaveformView, {
      remove: { imports: [JsrootPanel] },
      add: { imports: [StubPanel] },
    });
    clock = TestBed.inject(DisplayClock);
    fixture = TestBed.createComponent(WaveformView);
    fixture.detectChanges();
  });

  afterEach(() => vi.unstubAllGlobals());

  const root = (): HTMLElement => fixture.nativeElement as HTMLElement;
  const panelNames = (): string[] =>
    [...root().querySelectorAll<HTMLElement>('.stub')].map((el) => el.dataset['name'] ?? '');

  async function deliver(message: WaveformsMessage): Promise<void> {
    ws.waveforms.set(message);
    await settle();
  }

  async function tick(): Promise<void> {
    clock.tick.update((value) => value + 1);
    await settle();
  }

  it('Waveform ビューは 0x03 を購読する(setWaveforms(true))', () => {
    expect(ws.setWaveforms).toHaveBeenCalledWith(true);
  });

  it('届く前は待機文、(cobo,asad) ごとに AsAd パネルが出る(枚数はメッセージ由来)', async () => {
    expect(root().textContent).toContain('waiting for AsAd waveforms (0x03)');
    expect(panelNames()).toEqual([]);

    await deliver(waveforms(0, 1, 5));
    await deliver(waveforms(0, 0, 5));
    await tick();
    expect(panelNames()).toEqual(['AsadCobo0Asad0', 'AsadCobo0Asad1']); // cobo, asad 昇順
    expect(root().textContent).not.toContain('waiting for AsAd waveforms');
    expect(root().textContent).toContain('AsAd raw electronics view');
  });

  it('UVW の event# と食い違う AsAd だけに ≠ UVW を出す', async () => {
    ws.uvwByPlane.set([uvw(5), uvw(5), uvw(5)]);
    await deliver(waveforms(0, 0, 5));
    await deliver(waveforms(0, 1, 4)); // 0x03 が drop-oldest で 1 つ遅れた
    await tick();
    const labels = [...root().querySelectorAll<HTMLElement>('.asad-label')].map((el) =>
      (el.textContent ?? '').replace(/\s+/g, ' ').trim(),
    );
    expect(labels).toEqual(['event #5', 'event #4 ≠ UVW']);
  });

  it('UVW がまだ無いときは不一致と判定しない', async () => {
    await deliver(waveforms(0, 0, 9));
    await tick();
    expect(root().textContent).not.toContain('≠ UVW');
  });
});
