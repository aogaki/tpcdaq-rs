/**
 * TODO/125 — monitor-view の Event display タブ(投影 3 枚・状態表示)。
 * 127(レーン B): 計算は monitor(0x04 Track)へ。UI は点をビンに詰めるだけ。
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
import type { MetaMessage, StripGeometryMessage, StripGeometryUnavailable } from '../ws/json';
import type { HistoMessage } from '../ws/ws-client';
import type { TrackMessage, UvwMessage } from '../ws/wire';
import { WsClientService } from '../ws/ws-client';
import { JsrootPanel } from './jsroot-panel';
import { MonitorView } from './monitor-view';
import type { PanelMessage, PanelSpec } from './root-histo';

@Component({
  selector: 'app-jsroot-panel',
  template: `<i
    class="stub"
    [attr.data-name]="spec().name"
    [attr.data-kind]="message()?.kind"
    [attr.data-hot]="hot()"
  ></i>`,
})
class StubPanel {
  readonly spec = input.required<PanelSpec>();
  readonly message = input<PanelMessage | null>(null);

  /** 投影のうち値が 0 でないビンの `index:値`(iy*nx+ix)。 */
  hot(): string | null {
    const m = this.message();
    if (m?.kind !== 'projection') return null;
    const out: string[] = [];
    Array.from(m.values).forEach((v, i) => {
      if (v !== 0) out.push(`${i}:${v}`);
    });
    return out.join(',');
  }
}

interface FakeWs {
  histos: ReturnType<typeof signal<ReadonlyMap<number, HistoMessage>>>;
  uvwByPlane: ReturnType<typeof signal<readonly (UvwMessage | null)[]>>;
  track: ReturnType<typeof signal<TrackMessage | null>>;
  meta: ReturnType<typeof signal<MetaMessage | null>>;
  stripGeometry: ReturnType<typeof signal<StripGeometryMessage | StripGeometryUnavailable | null>>;
  setWaveforms: ReturnType<typeof vi.fn>;
}

function fakeWs(): FakeWs {
  return {
    histos: signal<ReadonlyMap<number, HistoMessage>>(new Map()),
    uvwByPlane: signal<readonly (UvwMessage | null)[]>([null, null, null]),
    track: signal<TrackMessage | null>(null),
    meta: signal<MetaMessage | null>(null),
    stripGeometry: signal<StripGeometryMessage | StripGeometryUnavailable | null>(null),
    setWaveforms: vi.fn(),
  };
}

const N_BUCKETS = 10;
/** nx = trunc(20/2 - 1) = 9、ny = trunc(18/3 - 1) = 5(xy-projection.spec と同じ手計算)。 */
const GEOMETRY: StripGeometryMessage = {
  type: 'stripGeometry',
  padPitch: 3,
  stripPitch: 2,
  rangeXY: [-10, 10, -9, 9],
  planes: {
    U: { unit: [1, 0], pitchUnit: [0, 1], segments: [] },
    V: { unit: [1, 0], pitchUnit: [0, 1], segments: [] },
    W: { unit: [1, 0], pitchUnit: [0, 1], segments: [] },
  },
};
const META = { type: 'meta', nBuckets: N_BUCKETS } as unknown as MetaMessage;

/** 3 面 1 イベント(中身は使わない — ラベルと `≠ UVW` の比較用)。 */
function uvwEvent(eventNumber: number): UvwMessage[] {
  return [0, 1, 2].map((plane) => ({
    kind: 'uvw' as const,
    header: { msgType: 2, incomplete: false, runNumber: 7, eventNumber },
    plane,
    nStrips: 1,
    nBuckets: N_BUCKETS,
    adc: new Uint16Array(N_BUCKETS),
  }));
}

/** 0x04: 直線 1 本(φ = 15.4°、xy 長さ 83.0 mm)、点は非対称な 2 点。 */
function trackMsg(
  eventNumber: number,
  extra: Partial<Omit<TrackMessage, 'kind' | 'header'>> & { incomplete?: boolean } = {},
): TrackMessage {
  const { incomplete = false, ...rest } = extra;
  return {
    kind: 'track',
    header: { msgType: 4, incomplete, runNumber: 7, eventNumber },
    ok: true,
    reason: '',
    hits: [72, 40, 34],
    unplaced: 0,
    tracks: [
      {
        phiDeg: 15.38,
        dir: [0.96, 0.27, 0.1],
        start: [0, 0, 0],
        end: [1, 1, 1],
        startF: [0, 0, 0],
        endF: [1, 1, 1],
        length: 90,
        lengthF: 91,
        xyLength: 83.04,
        charge: 1000,
      },
    ],
    // 点 [x, y, t, q, view, track]。(5, 4, 7): ix=6, iy=3, iz=7。(-9, -8, 0): ix=0, iy=0, iz=0。
    points: [
      [5, 4, 7, 2.5, 0, 0],
      [-9, -8, 0, 4, 1, 0],
    ],
    unknownKeys: [],
    ...rest,
  };
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

  it('ヒント文: 表示専用・monitor が計算・macros と同じアルゴリズム・Z は time bucket・頭尻は決めない', async () => {
    build();
    await openEventTab();
    const text = (root().textContent ?? '').replace(/\s+/g, ' ');
    expect(text).toContain('Display only.');
    expect(text).toContain('The projection is computed by monitor');
    expect(text).toContain('same algorithm as macros/analyzeUVW.C + makeTracks.C');
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

  const hot = (name: string): string =>
    root().querySelector<HTMLElement>(`.stub[data-name="${name}"]`)?.dataset['hot'] ?? '';

  it('stripGeometry より先に 0x04 が届いても壊れず「未着」を出す。あとから届けば詰める', async () => {
    build();
    ws.meta.set(META);
    ws.track.set(trackMsg(1));
    await openEventTab();
    await tick();
    expect(status()).toContain('strip geometry not received yet');
    expect(fixture.componentInstance.projectionRuns()).toBe(0);
    ws.stripGeometry.set(GEOMETRY);
    await settle();
    expect(status()).toMatch(/^straight track: /);
    expect(fixture.componentInstance.projectionRuns()).toBe(1);
  });

  it('unavailable は理由をそのまま出す', async () => {
    build();
    ws.stripGeometry.set({ type: 'stripGeometry', unavailable: 'no ANGLES header in the .dat' });
    ws.meta.set(META);
    ws.track.set(trackMsg(1));
    await openEventTab();
    await tick();
    expect(status()).toBe('strip geometry unavailable: no ANGLES header in the .dat');
  });

  it('0x04 がまだ無い → "waiting for track (0x04)"', async () => {
    build();
    ws.stripGeometry.set(GEOMETRY);
    ws.meta.set(META);
    ws.uvwByPlane.set(uvwEvent(1));
    await openEventTab();
    await tick();
    expect(status()).toBe('waiting for track (0x04)');
    expect(fixture.componentInstance.projectionRuns()).toBe(0);
  });

  it('ビンが退化した stripGeometry → "no projection: degenerate …"', async () => {
    build();
    ws.stripGeometry.set({ ...GEOMETRY, rangeXY: [0, 1, 0, 1] });
    ws.meta.set(META);
    ws.track.set(trackMsg(1));
    await openEventTab();
    await tick();
    expect(status()).toContain('no projection: degenerate projection bins');
  });

  it('ok: 状態表示の文が 125 と同じ。既知の点が正しいビンに入る(XY/XZ/YZ、nBuckets は meta から)', async () => {
    build();
    ws.stripGeometry.set(GEOMETRY);
    ws.meta.set(META);
    ws.uvwByPlane.set(uvwEvent(1));
    ws.track.set(trackMsg(1));
    await openEventTab();
    await tick();
    expect(status()).toBe(
      'straight track: φ = 15.4°, xy length 83.0 mm (2–98 % charge), strip hits U/V/W = 72/40/34',
    );
    // xy-projection.spec の手計算: (5,4,7) → XY[3*9+6=33]、XZ[7*9+6=69]、YZ[7*5+3=38];
    // (-9,-8,0) → index 0(q = 4)。
    expect(hot('ProjectionXY')).toBe('0:4,33:2.5');
    expect(hot('ProjectionXZ')).toBe('0:4,69:2.5');
    expect(hot('ProjectionYZ')).toBe('0:4,38:2.5');
    expect(root().querySelector('[data-testid="track-mismatch"]')).toBeNull();
  });

  it('ok でない 0x04 → "no track: <reason>" と空のヒスト', async () => {
    build();
    ws.stripGeometry.set(GEOMETRY);
    ws.meta.set(META);
    ws.track.set(
      trackMsg(3, {
        ok: false,
        reason: 'W has 2 strip hits after the halo cut (needs 4)',
        hits: [10, 9, 2],
        tracks: [],
        points: [],
      }),
    );
    await openEventTab();
    await tick();
    expect(status()).toBe('no track: W has 2 strip hits after the halo cut (needs 4)');
    expect(hot('ProjectionXY')).toBe('');
  });

  it('0x04 の event# が表示中の 0x02 と違う → "≠ UVW" を出すが点は描く', async () => {
    build();
    ws.stripGeometry.set(GEOMETRY);
    ws.meta.set(META);
    ws.uvwByPlane.set(uvwEvent(5));
    ws.track.set(trackMsg(6));
    await openEventTab();
    await tick();
    expect(root().querySelector('[data-testid="track-mismatch"]')?.textContent).toContain('≠ UVW');
    expect(hot('ProjectionXY')).toBe('0:4,33:2.5');
    expect(status()).toMatch(/^straight track: /);
  });

  it('shownTrack は shownUvw と同じ刻みで更新される(ラベルと絵が揃う)', async () => {
    build();
    ws.stripGeometry.set(GEOMETRY);
    ws.meta.set(META);
    ws.uvwByPlane.set(uvwEvent(1));
    ws.track.set(trackMsg(1));
    await openEventTab();
    await tick();
    const runs = fixture.componentInstance.projectionRuns();
    expect(runs).toBe(1);

    await tick(); // 刻みだけ進む → 入力が同じなら再計算しない
    expect(fixture.componentInstance.projectionRuns()).toBe(runs);

    ws.uvwByPlane.set(uvwEvent(2));
    ws.track.set(trackMsg(2, { points: [[5, 4, 7, 9, 0, 0]] }));
    await settle();
    expect(fixture.componentInstance.projectionRuns()).toBe(runs); // 刻みを待つ
    expect(root().textContent).toContain('event #1');
    expect(hot('ProjectionXY')).toBe('0:4,33:2.5');
    await tick();
    expect(fixture.componentInstance.projectionRuns()).toBe(runs + 1);
    expect(root().textContent).toContain('event #2');
    expect(hot('ProjectionXY')).toBe('33:9');
    expect(root().querySelector('[data-testid="track-mismatch"]')).toBeNull();
  });

  it('タブが閉じている(Histograms)間は詰めない。開くと 1 回詰める', async () => {
    build();
    ws.stripGeometry.set(GEOMETRY);
    ws.meta.set(META);
    ws.track.set(trackMsg(1));
    await tick();
    ws.track.set(trackMsg(2));
    await tick();
    expect(fixture.componentInstance.projectionRuns()).toBe(0);
    await openEventTab();
    expect(fixture.componentInstance.projectionRuns()).toBe(1);
  });

  it('INCOMPLETE の 0x04 は状態表示にも INCOMPLETE を出す', async () => {
    build();
    ws.stripGeometry.set(GEOMETRY);
    ws.meta.set(META);
    ws.track.set(trackMsg(1, { incomplete: true }));
    await openEventTab();
    await tick();
    expect(status()).toMatch(/^straight track: /);
    expect(status()).toContain('INCOMPLETE (fragments missing — charges are short)');
  });

  it('置けなかったヒット(unplaced)と範囲外の点は数を文で出す(silent にしない)', async () => {
    build();
    ws.stripGeometry.set(GEOMETRY);
    ws.meta.set(META);
    ws.track.set(
      trackMsg(1, {
        unplaced: 3,
        points: [
          [5, 4, 7, 2.5, 0, 0],
          [50, 0, 3, 1, 1, 0], // x が範囲外
        ],
      }),
    );
    await openEventTab();
    await tick();
    expect(status()).toContain('1 points outside the plot range');
    expect(status()).toContain('3 strip hits not placed on the line (view nearly parallel to the track)');
  });
});
