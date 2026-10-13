/**
 * TODO/125 — monitor-view の Event display タブ(投影 3 枚・状態表示)。
 * 改訂 2(レーン D): 投影は macros/analyzeUVW.C + makeTracks.C の移植(直線 1 本)、閾値欄は撤去。
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
import { JsrootPanel } from './jsroot-panel';
import { MonitorView } from './monitor-view';
import type { PanelMessage, PanelSpec } from './root-histo';
import { type TrackOk, trackEvent } from './track-fit';
import {
  MINI_ANGLES,
  type Track,
  addHalo,
  addTrack,
  emptyEvent,
  layoutOf,
  syntheticGeometry,
} from './track-test-fixtures';

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

const LAYOUT = layoutOf(MINI_ANGLES);
const GEOMETRY = syntheticGeometry(LAYOUT);
/** 第 1 象限の非対称な直線: (10, 4) mm → (90, 26) mm、bucket 150 → 250(φ = 15.38°、xy 83 mm)。 */
const TRACK: Track = { a: [10, 4, 150], b: [90, 26, 250] };

/** 直線トラックを 1 本持つ 1 イベント。 */
function lineEvent(eventNumber = 1, incomplete = false): UvwMessage[] {
  const uvw = emptyEvent(LAYOUT, 512, eventNumber, incomplete);
  addTrack(uvw, LAYOUT, TRACK);
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
  const panelNames = (): string[] =>
    [...root().querySelectorAll<HTMLElement>('.stub')].map((el) => el.dataset['name'] ?? '');

  afterEach(() => {
    vi.restoreAllMocks();
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

  it('ヒント文: 表示専用・macros と同じアルゴリズム・Z は time bucket・頭尻は決めない', async () => {
    build();
    await openEventTab();
    const text = (root().textContent ?? '').replace(/\s+/g, ' ');
    expect(text).toContain('Display only.');
    expect(text).toContain('Same algorithm as macros/analyzeUVW.C + makeTracks.C');
    expect(text).toContain('one hit per strip (peak ±5 cells, halo cut q ≥ 2000 ADC)');
    expect(text).toContain('one straight track per event');
    expect(text).toContain('Z is the time bucket');
    expect(text).toContain('Head/tail not determined');
  });

  it('閾値欄は無い(定数は macros と同じに固定)', async () => {
    build();
    await openEventTab();
    expect(root().querySelector('input')).toBeNull();
    expect(root().textContent).not.toContain('hit threshold');
  });

  it('stripGeometry より先に 0x02 が届いても壊れず「未着」を出す', async () => {
    build();
    ws.uvwByPlane.set(lineEvent());
    await openEventTab();
    await tick();
    expect(status()).toContain('strip geometry not received yet');
    expect(fixture.componentInstance.projectionRuns()).toBe(0);
    // あとからジオメトリが届けば計算される
    ws.stripGeometry.set(GEOMETRY);
    await settle();
    expect(status()).toMatch(/^straight track: /);
    expect(fixture.componentInstance.projectionRuns()).toBe(1);
  });

  it('unavailable は理由をそのまま出す', async () => {
    build();
    ws.stripGeometry.set({ type: 'stripGeometry', unavailable: 'no ANGLES header in the .dat' });
    ws.uvwByPlane.set(lineEvent());
    await openEventTab();
    await tick();
    expect(status()).toBe('strip geometry unavailable: no ANGLES header in the .dat');
  });

  it('ok: 「straight track: φ = …°, xy length … mm (2–98 % charge), strip hits U/V/W = a/b/c」と 3 枚', async () => {
    build();
    ws.stripGeometry.set(GEOMETRY);
    ws.uvwByPlane.set(lineEvent());
    await openEventTab();
    await tick();
    const expected = trackEvent(lineEvent(), GEOMETRY)!;
    const track = expected.track as TrackOk;
    const [u, v, w] = expected.selected.map((hits) => hits.length);
    expect(status()).toBe(
      `straight track: φ = ${track.phiDeg.toFixed(1)}°, ` +
        `xy length ${track.xyLength.toFixed(1)} mm (2–98 % charge), strip hits U/V/W = ${u}/${v}/${w}`,
    );
    // 真の直線: φ = atan2(22, 80) = 15.4°、xy の長さ √(80² + 22²) = 83.0 mm(2–98 % で 96 % ≈ 79.7)
    expect(status()).toMatch(/^straight track: φ = 15\.\d°, xy length [78]\d\.\d mm/);
    const kinds = [...root().querySelectorAll<HTMLElement>('.stub')]
      .filter((el) => el.dataset['name']?.startsWith('Projection'))
      .map((el) => el.dataset['kind']);
    expect(kinds).toEqual(['projection', 'projection', 'projection']);
  });

  it('トラックが無いときは理由を文で出す(halo だけのイベント)', async () => {
    build();
    ws.stripGeometry.set(GEOMETRY);
    const uvw = emptyEvent(LAYOUT, 512, 3);
    addHalo(uvw, 200);
    ws.uvwByPlane.set(uvw);
    await openEventTab();
    await tick();
    expect(status()).toBe(
      'no track: U has 0 strip hits after the halo cut (needs 4); ' +
        'V has 0 strip hits after the halo cut (needs 4); ' +
        'W has 0 strip hits after the halo cut (needs 4)',
    );
  });

  it('同じイベントなら再計算しない。新しい 0x02 は次の刻みで反映される', async () => {
    build();
    ws.stripGeometry.set(GEOMETRY);
    ws.uvwByPlane.set(lineEvent(1));
    await openEventTab();
    await tick();
    const runs = fixture.componentInstance.projectionRuns();
    expect(runs).toBe(1);

    await tick(); // 刻みだけ進む
    await settle();
    expect(fixture.componentInstance.projectionRuns()).toBe(runs);

    ws.uvwByPlane.set(lineEvent(2)); // 新イベントは刻みを待つ(画面のラベルと絵を揃える)
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
    ws.uvwByPlane.set(lineEvent(1));
    await tick();
    ws.uvwByPlane.set(lineEvent(2));
    await tick();
    expect(fixture.componentInstance.projectionRuns()).toBe(0);
    await openEventTab();
    expect(fixture.componentInstance.projectionRuns()).toBe(1);
  });

  it('INCOMPLETE のイベントは投影の状態表示にも INCOMPLETE を出す', async () => {
    build();
    ws.stripGeometry.set(GEOMETRY);
    ws.uvwByPlane.set(lineEvent(1, true));
    await openEventTab();
    await tick();
    expect(status()).toMatch(/^straight track: /);
    expect(status()).toContain('INCOMPLETE (fragments missing — charges are short)');
  });

  it('stripGeometry に線分の無い strip のヒットは数を文で出す(silent にしない)', async () => {
    build();
    // V strip 90(s = 33 mm)はトラックが横切る。その線分だけ抜いたジオメトリ。
    const trimmed: StripGeometryMessage = {
      ...GEOMETRY,
      planes: {
        ...GEOMETRY.planes,
        V: {
          ...GEOMETRY.planes.V,
          segments: GEOMETRY.planes.V.segments.filter(([strip]) => strip !== 90),
        },
      },
    };
    ws.stripGeometry.set(trimmed);
    ws.uvwByPlane.set(lineEvent());
    await openEventTab();
    await tick();
    expect(status()).toMatch(/^straight track: /);
    expect(status()).toContain('strips with a hit but no segment in stripGeometry (skipped): 1');
  });

  it('3 面が揃っていない / 別イベント → 計算しない理由を文で出す', async () => {
    build();
    ws.stripGeometry.set(GEOMETRY);
    const uvw = lineEvent(1);
    ws.uvwByPlane.set([uvw[0], uvw[1], null]);
    await openEventTab();
    await tick();
    expect(status()).toBe('no projection: waiting for W (0x02)');
    expect(fixture.componentInstance.projectionRuns()).toBe(0);

    const other = lineEvent(2);
    ws.uvwByPlane.set([uvw[0], uvw[1], other[2]]);
    await tick();
    expect(status()).toContain('different events');
  });
});
