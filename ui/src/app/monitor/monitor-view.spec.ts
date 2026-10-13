/**
 * TODO/125 レーン B 項目 3/5 — monitor-view の Event display タブ(投影 3 枚・閾値欄・状態表示)。
 *
 * WS は最小の fake(シグナルだけ)。JsrootPanel は描画せず名前と受け取ったメッセージだけ見える
 * スタブに差し替える(jsroot の描画は別のテストの担当)。
 */
import { ChangeDetectorRef, Component, input, signal } from '@angular/core';
import { type ComponentFixture, TestBed } from '@angular/core/testing';
import { MATERIAL_ANIMATIONS } from '@angular/material/core';
import { MatTabGroup } from '@angular/material/tabs';
import { By } from '@angular/platform-browser';
import { vi } from 'vitest';

import { DisplayClock } from '../display/display-clock';
import type { StripGeometryMessage, StripGeometryUnavailable } from '../ws/json';
import type { HistoMessage } from '../ws/ws-client';
import type { UvwMessage } from '../ws/wire';
import { WsClientService } from '../ws/ws-client';
import { THRESHOLD_STORAGE_KEY } from './event-display-settings';
import { JsrootPanel } from './jsroot-panel';
import { MonitorView } from './monitor-view';
import type { PanelMessage, PanelSpec } from './root-histo';
import { HEX, addTrack, emptyUvw, geometryOf, memoryStorage, setHit } from './xy-test-fixtures';

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

interface FakeWs {
  histos: ReturnType<typeof signal<ReadonlyMap<number, HistoMessage>>>;
  uvwByPlane: ReturnType<typeof signal<readonly (UvwMessage | null)[]>>;
  stripGeometry: ReturnType<typeof signal<StripGeometryMessage | StripGeometryUnavailable | null>>;
  setWaveforms: ReturnType<typeof vi.fn>;
}

function fakeWs(): FakeWs {
  return {
    histos: signal<ReadonlyMap<number, HistoMessage>>(new Map()),
    uvwByPlane: signal<readonly (UvwMessage | null)[]>([null, null, null]),
    stripGeometry: signal<StripGeometryMessage | StripGeometryUnavailable | null>(null),
    setWaveforms: vi.fn(),
  };
}

const GEOMETRY = geometryOf(HEX);

/** 第 1 象限の直線トラックを持つ 1 イベント。 */
function trackEvent(eventNumber = 1, incomplete = false): UvwMessage[] {
  const uvw = emptyUvw(HEX, 300, eventNumber, incomplete);
  addTrack(uvw, GEOMETRY, HEX, (b) => [20 + 0.4 * (b - 100), 5 + 0.25 * (b - 100)], 100, 200);
  return uvw;
}

describe('MonitorView — Event display の投影', () => {
  let ws: FakeWs;
  let fixture: ComponentFixture<MonitorView>;
  let clock: DisplayClock;

  function build(): void {
    ws = fakeWs();
    TestBed.configureTestingModule({
      providers: [
        { provide: WsClientService, useValue: ws as unknown as WsClientService },
        // jsdom では transitionend が来ないので、タブの切替が完了するよう無効化する。
        { provide: MATERIAL_ANIMATIONS, useValue: { animationsDisabled: true } },
      ],
    });
    TestBed.overrideComponent(MonitorView, {
      remove: { imports: [JsrootPanel] },
      add: { imports: [StubPanel] },
    });
    clock = TestBed.inject(DisplayClock);
    fixture = TestBed.createComponent(MonitorView);
    fixture.detectChanges();
  }

  async function settle(): Promise<void> {
    for (let i = 0; i < 3; i++) {
      fixture.detectChanges();
      await new Promise((resolve) => setTimeout(resolve, 0));
      await fixture.whenStable();
    }
  }

  async function openEventTab(): Promise<void> {
    const element = fixture.debugElement.query(By.directive(MatTabGroup));
    (element.componentInstance as MatTabGroup).selectedIndex = 1;
    // zoneless: 親ビューが dirty でないと MatTabGroup の ngAfterContentChecked が走らない。
    element.injector.get(ChangeDetectorRef).markForCheck();
    await settle();
  }

  async function tick(): Promise<void> {
    clock.tick.update((value) => value + 1);
    await settle();
  }

  const root = (): HTMLElement => fixture.nativeElement as HTMLElement;
  const status = (): string =>
    root().querySelector('[data-testid="projection-status"]')?.textContent?.trim() ?? '';
  const thresholdInput = (): HTMLInputElement =>
    root().querySelector<HTMLInputElement>('.threshold input')!;
  const panelNames = (): string[] =>
    [...root().querySelectorAll<HTMLElement>('.stub')].map((el) => el.dataset['name'] ?? '');

  function commitThreshold(value: string): void {
    const input = thresholdInput();
    input.value = value;
    input.dispatchEvent(new Event('change'));
  }

  let storage: Storage;
  beforeEach(() => {
    storage = memoryStorage();
    vi.stubGlobal('localStorage', storage);
  });
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it('monitor タブは 0x03 を購読しない(setWaveforms を呼ばない)・AsAd パネルが無い', async () => {
    build();
    await openEventTab();
    expect(ws.setWaveforms).not.toHaveBeenCalled();
    expect(panelNames().some((name) => name.startsWith('Asad'))).toBe(false);
    fixture.destroy();
    expect(ws.setWaveforms).not.toHaveBeenCalled();
  });

  it('2 段目に XY / XZ / YZ の 3 枚、1 段目は U/V/W のまま', async () => {
    build();
    await openEventTab();
    expect(panelNames()).toEqual([
      'EventU',
      'EventV',
      'EventW',
      'ProjectionXY',
      'ProjectionXZ',
      'ProjectionYZ',
    ]);
  });

  it('ヒント文: 表示専用・三面一致・Z は time bucket', async () => {
    build();
    await openEventTab();
    const text = root().textContent ?? '';
    expect(text).toContain('Display only');
    expect(text).toContain('MatchCrossPoint');
    expect(text).toContain('radius 2 mm');
    expect(text).toContain('time bucket');
  });

  it('閾値欄の既定値は 50(ラベルは hit threshold (ADC above baseline))', async () => {
    build();
    await openEventTab();
    expect(root().textContent).toContain('hit threshold (ADC above baseline)');
    expect(thresholdInput().value).toBe('50');
    expect(fixture.componentInstance.threshold()).toBe(50);
  });

  it('stripGeometry より先に 0x02 が届いても壊れず「未着」を出す', async () => {
    build();
    ws.uvwByPlane.set(trackEvent());
    await openEventTab();
    await tick();
    expect(status()).toContain('strip geometry not received yet');
    expect(fixture.componentInstance.projectionRuns()).toBe(0);
    // あとからジオメトリが届けば計算される
    ws.stripGeometry.set(GEOMETRY);
    await settle();
    expect(status()).toMatch(/^\d+ points \(U\/V\/W coincidences\)/);
    expect(fixture.componentInstance.projectionRuns()).toBe(1);
  });

  it('unavailable は理由をそのまま出す', async () => {
    build();
    ws.stripGeometry.set({ type: 'stripGeometry', unavailable: 'no ANGLES header in the .dat' });
    ws.uvwByPlane.set(trackEvent());
    await openEventTab();
    await tick();
    expect(status()).toBe('strip geometry unavailable: no ANGLES header in the .dat');
  });

  it('通常: 「N points (U/V/W coincidences) · threshold T ADC · radius 2 mm」と 3 枚のメッセージ', async () => {
    build();
    ws.stripGeometry.set(GEOMETRY);
    ws.uvwByPlane.set(trackEvent());
    await openEventTab();
    await tick();
    expect(status()).toMatch(
      /^[1-9]\d* points \(U\/V\/W coincidences\) · threshold 50 ADC · radius 2 mm$/,
    );
    const kinds = [...root().querySelectorAll<HTMLElement>('.stub')]
      .filter((el) => el.dataset['name']?.startsWith('Projection'))
      .map((el) => el.dataset['kind']);
    expect(kinds).toEqual(['projection', 'projection', 'projection']);
  });

  it('同じ (event, 閾値) なら再計算しない。新しい 0x02 は次の刻みで反映される', async () => {
    build();
    ws.stripGeometry.set(GEOMETRY);
    ws.uvwByPlane.set(trackEvent(1));
    await openEventTab();
    await tick();
    const runs = fixture.componentInstance.projectionRuns();
    expect(runs).toBe(1);

    await tick(); // 刻みだけ進む
    await settle();
    expect(fixture.componentInstance.projectionRuns()).toBe(runs);

    ws.uvwByPlane.set(trackEvent(2)); // 新イベントは刻みを待つ(画面のラベルと絵を揃える)
    await settle();
    expect(fixture.componentInstance.projectionRuns()).toBe(runs);
    expect(root().textContent).toContain('event #1');
    await tick();
    expect(fixture.componentInstance.projectionRuns()).toBe(runs + 1);
    expect(root().textContent).toContain('event #2');
  });

  it('タブが閉じている(Histograms)間は計算しない。開くと 1 回計算する', async () => {
    build();
    ws.stripGeometry.set(GEOMETRY);
    ws.uvwByPlane.set(trackEvent(1));
    await tick();
    ws.uvwByPlane.set(trackEvent(2));
    await tick();
    expect(fixture.componentInstance.projectionRuns()).toBe(0);
    await openEventTab();
    expect(fixture.componentInstance.projectionRuns()).toBe(1);
  });

  it('閾値を変えると再計算し、localStorage に記憶する。不正値は受け付けず欄を戻す', async () => {
    build();
    ws.stripGeometry.set(GEOMETRY);
    ws.uvwByPlane.set(trackEvent());
    await openEventTab();
    await tick();
    const runs = fixture.componentInstance.projectionRuns();

    commitThreshold('120');
    await settle();
    expect(fixture.componentInstance.threshold()).toBe(120);
    expect(storage.getItem(THRESHOLD_STORAGE_KEY)).toBe('120');
    expect(fixture.componentInstance.projectionRuns()).toBe(runs + 1);
    expect(status()).toContain('threshold 120 ADC');

    commitThreshold('-3');
    await settle();
    expect(fixture.componentInstance.threshold()).toBe(120);
    expect(thresholdInput().value).toBe('120');
    commitThreshold('');
    await settle();
    expect(thresholdInput().value).toBe('120');
    expect(fixture.componentInstance.projectionRuns()).toBe(runs + 1);
  });

  it('保存済みの閾値で起動する(次のセッションで復元)', async () => {
    storage.setItem(THRESHOLD_STORAGE_KEY, '230');
    build();
    await openEventTab();
    expect(thresholdInput().value).toBe('230');
  });

  it('localStorage が throw しても既定値で動き、閾値欄も使える', async () => {
    vi.spyOn(storage, 'getItem').mockImplementation(() => {
      throw new Error('SecurityError');
    });
    vi.spyOn(storage, 'setItem').mockImplementation(() => {
      throw new Error('QuotaExceededError');
    });
    build();
    ws.stripGeometry.set(GEOMETRY);
    ws.uvwByPlane.set(trackEvent());
    await openEventTab();
    await tick();
    expect(thresholdInput().value).toBe('50');
    expect(status()).toContain('threshold 50 ADC');
    commitThreshold('90');
    await settle();
    expect(status()).toContain('threshold 90 ADC');
  });

  it('0 点: 文で出す(閾値を下げる案内つき)', async () => {
    build();
    ws.stripGeometry.set(GEOMETRY);
    const uvw = emptyUvw(HEX, 300, 3);
    for (const message of uvw) setHit(message, 101, 120, 130); // baseline 100 + 30 < 閾値 50
    ws.uvwByPlane.set(uvw);
    await openEventTab();
    await tick();
    expect(status()).toContain('0 points');
    expect(status()).toContain('threshold 50 ADC');
    expect(status()).toContain('try a lower threshold');
  });

  it('INCOMPLETE のイベントは投影の状態表示にも INCOMPLETE を出す', async () => {
    build();
    ws.stripGeometry.set(GEOMETRY);
    ws.uvwByPlane.set(trackEvent(1, true));
    await openEventTab();
    await tick();
    expect(status()).toContain('INCOMPLETE');
    expect(status()).toMatch(/points \(U\/V\/W coincidences\)/);
  });

  it('3 面が揃っていない / 別イベント → 計算しない理由を文で出す', async () => {
    build();
    ws.stripGeometry.set(GEOMETRY);
    const uvw = trackEvent(1);
    ws.uvwByPlane.set([uvw[0], uvw[1], null]);
    await openEventTab();
    await tick();
    expect(status()).toContain('no projection: waiting for W');
    expect(fixture.componentInstance.projectionRuns()).toBe(0);

    const other = trackEvent(2);
    ws.uvwByPlane.set([uvw[0], uvw[1], other[2]]);
    await tick();
    expect(status()).toContain('different events');
  });

  it('評価回数が上限(2×10⁶)を超えると CAPPED を文で出す(「閾値を上げて」)', async () => {
    build();
    ws.stripGeometry.set(GEOMETRY);
    const uvw = emptyUvw(HEX, 100, 4);
    for (const message of uvw) {
      for (let strip = 1; strip <= HEX.n; strip++) {
        for (let b = 25; b < 100; b++) setHit(message, strip, b, 1000);
      }
    }
    ws.uvwByPlane.set(uvw);
    await openEventTab();
    await tick();
    expect(status()).toContain('CAPPED after 2,000,000 crossPoint evaluations');
    expect(status()).toContain('raise the threshold');
  });
});
