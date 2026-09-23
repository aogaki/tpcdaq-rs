/**
 * status-bar の `slow` バッジ(TODO/079 §2.8-4)。
 *
 * 既存の 4 本柱(health/state/run/... の各 `item`)は他レーンとの衝突を避けるため
 * 触っていない(発注書: status-bar.html は末尾に `<span class="item">` を 1 個足すだけ)。
 * ここでは新しく足した `slowAlarmCount` / `slowAlarmTooltip` の計算だけを見る。
 */
import { type ComponentFixture, TestBed } from '@angular/core/testing';

import { WsClientService } from '../ws/ws-client';
import { StatusBar } from './status-bar';

/** monitor の `status_json`(SPEC §10.3)の最小形 + `slow`(v1.31、任意)。 */
function statusWith(slow?: Record<string, unknown>): Record<string, unknown> {
  return {
    type: 'status',
    run: 7,
    state: 'running',
    events_built: 0,
    events_incomplete: 0,
    late_fragments: 0,
    pending_events: 0,
    frames_per_cobo: {},
    bytes_written: 0,
    saturation: {
      U: { saturated: 0, counted: 0 },
      V: { saturated: 0, counted: 0 },
      W: { saturated: 0, counted: 0 },
    },
    publish_drops: 0,
    monitorGaps: 0,
    clients: 0,
    wsDropped: 0,
    ...(slow ? { slow } : {}),
  };
}

describe('StatusBar — slow バッジ', () => {
  let fixture: ComponentFixture<StatusBar>;
  let ws: WsClientService;

  beforeEach(() => {
    TestBed.configureTestingModule({});
    fixture = TestBed.createComponent(StatusBar);
    ws = TestBed.inject(WsClientService);
    fixture.detectChanges();
  });

  afterEach(() => fixture.destroy());

  it('status が来ていない、または slow キーが無ければ「—」', () => {
    expect(fixture.componentInstance.slowAlarmCount()).toBeNull();

    ws.status.set(statusWith() as never);
    fixture.detectChanges();
    expect(fixture.componentInstance.slowAlarmCount()).toBeNull();
    expect(fixture.nativeElement.textContent).toContain('—');
  });

  it('alarm=true のサンプル数を数える(非対称: 2 件 alarm、1 件正常)', () => {
    ws.status.set(
      statusWith({
        kind: 'slow',
        ts_ms: 1,
        samples: [
          {
            device: 'gas',
            node: 3,
            param: 'fmeasure',
            value: 130,
            unit: 'mbar',
            ts_ms: 1,
            alarm: true,
          },
          {
            device: 'env',
            node: null,
            param: 't_c',
            value: 24,
            unit: 'degC',
            ts_ms: 1,
            alarm: true,
          },
          {
            device: 'gas',
            node: 5,
            param: 'fmeasure',
            value: 100,
            unit: 'mbar',
            ts_ms: 1,
            alarm: false,
          },
        ],
        counters: { poll_ok: 3, poll_err: 0, parse_err: 0, reconnects: 0, identity_mismatch: 0 },
      }) as never,
    );
    fixture.detectChanges();

    expect(fixture.componentInstance.slowAlarmCount()).toBe(2);
    expect(fixture.componentInstance.slowAlarmTooltip()).toBe('gas/3/fmeasure, env/t_c');
    expect(fixture.nativeElement.textContent).toContain('slow');
    expect(fixture.nativeElement.textContent).toContain('2');
  });

  it('slow はあるが alarm が 0 件なら 0(通常色、tooltip は「no active alarms」)', () => {
    ws.status.set(
      statusWith({
        kind: 'slow',
        ts_ms: 1,
        samples: [
          {
            device: 'gas',
            node: 3,
            param: 'fmeasure',
            value: 100,
            unit: 'mbar',
            ts_ms: 1,
            alarm: false,
          },
        ],
        counters: { poll_ok: 1, poll_err: 0, parse_err: 0, reconnects: 0, identity_mismatch: 0 },
      }) as never,
    );
    fixture.detectChanges();

    expect(fixture.componentInstance.slowAlarmCount()).toBe(0);
    expect(fixture.componentInstance.slowAlarmTooltip()).toBe('No active slow-control alarms');
  });
});
