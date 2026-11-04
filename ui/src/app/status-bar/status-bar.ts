import { Component, type OnDestroy, computed, inject, signal } from '@angular/core';
import { MatIconModule } from '@angular/material/icon';
import { MatTooltipModule } from '@angular/material/tooltip';

import { alarmLabels, countAlarms } from '../slow/slow-history';
import { ControllerApiService } from '../api/controller-api';
import {
  type StatusView,
  drainingLabel,
  groupDigits,
  overloadedComponents,
  parseControllerStatus,
} from '../api/controller-status';
import { ParamsApiService } from '../api/params-api';
import { saturationPercent } from '../ws/json';
import { WsClientService } from '../ws/ws-client';

/** `params <id> · exceptions N` の poll 間隔(078 §11「作るもの」4)。 */
export const STATUS_BAR_PARAMS_POLL_MS = 15000;

/** controller `GET /api/status` の poll 間隔(STOPPING / ROOT DROPPED / OVERLOAD 用、TODO/187)。 */
export const STATUS_BAR_CONTROLLER_POLL_MS = 5000;

/**
 * 全ページ共通の status バー(SPEC §10.3 の `status` + monitor の 3 キー + §5.2 の飽和率)。
 *
 * **未知と 0 を混同させない**: 未接続・status 未着は `—` を出す(0 と書かない)。
 * 落としたもの(decodeErrors など)は常に見えるところに置く(silent 禁止)。
 */
@Component({
  selector: 'app-status-bar',
  imports: [MatIconModule, MatTooltipModule],
  templateUrl: './status-bar.html',
  styleUrl: './status-bar.scss',
})
export class StatusBar implements OnDestroy {
  private readonly ws = inject(WsClientService);
  private readonly paramsApi = inject(ParamsApiService);
  private readonly controllerApi = inject(ControllerApiService);

  readonly health = this.ws.health;
  readonly link = this.ws.link;
  readonly counters = this.ws.counters;
  readonly lastIssue = this.ws.lastIssue;

  private readonly status = this.ws.status;

  /** DAQ の状態。status が来ていなければ「未知」= `—`。 */
  readonly state = computed(() => this.status()?.state ?? null);
  readonly run = computed(() => this.status()?.run ?? null);
  readonly eventsBuilt = computed(() => this.status()?.events_built ?? null);
  readonly monitorGaps = computed(() => this.status()?.monitorGaps ?? null);
  readonly wsDropped = computed(() => this.status()?.wsDropped ?? null);
  // ZS 復元カウンタ(キー名との対応): zsRestored = uvwZsFragmentsRestored、
  // zsNoTable = uvwZsFragmentsNoTable、zsTableMissing = uvwTableMissingChannels。
  // キーが無い(152 より前の monitor)ときは null = 画面では「—」。
  readonly zsRestored = computed(() => this.status()?.uvwZsFragmentsRestored ?? null);
  readonly zsNoTable = computed(() => this.status()?.uvwZsFragmentsNoTable ?? null);
  readonly zsTableMissing = computed(() => this.status()?.uvwTableMissingChannels ?? null);
  readonly clients = computed(() => this.status()?.clients ?? null);

  /** SPEC §5.2: `saturated / counted * 100`。`counted == 0` は `—`。 */
  readonly saturation = computed(() => {
    const saturation = this.status()?.saturation;
    return (['U', 'V', 'W'] as const).map((plane) => ({
      plane,
      percent: saturationPercent(saturation?.[plane]),
    }));
  });

  /** 健全性バッジの文言(未接続と status 途絶を区別する)。 */
  readonly healthLabel = computed(() => {
    switch (this.health()) {
      case 'offline':
        return this.link() === 'connecting' ? 'connecting' : 'offline';
      case 'waiting':
        return 'waiting for status';
      case 'stale':
        return 'stale';
      case 'fresh':
        return 'live';
    }
  });

  readonly healthTooltip = computed(() => {
    switch (this.health()) {
      case 'offline':
        return `Not connected to the monitor WS (${this.ws.url()})`;
      case 'waiting':
        return 'Connected. Waiting for the first status';
      case 'stale':
        return 'WS is connected but no status for over 3 s (root-sink may have stopped)';
      case 'fresh':
        return 'Receiving status';
    }
  });

  /** 数値または `—`。 */
  format(value: number | null): string {
    return value === null ? '—' : value.toLocaleString();
  }

  formatPercent(value: number | null): string {
    return value === null ? '—' : `${value.toFixed(1)} %`;
  }

  reconnect(): void {
    this.ws.reconnectNow();
  }

  /** slowctl(TODO/079 §2.8): `slow` が無ければ `null`(= 画面は `—`、0 と混同させない)。 */
  readonly slowAlarmCount = computed(() => {
    const slow = this.status()?.slow;
    return slow ? countAlarms(slow.samples) : null;
  });

  readonly slowAlarmTooltip = computed(() => {
    const slow = this.status()?.slow;
    if (!slow)
      return 'No slow-control data (monitor `slow_sub` not configured or slowctl not running)';
    const labels = alarmLabels(slow.samples);
    return labels.length === 0 ? 'No active slow-control alarms' : labels.join(', ');
  });
  /**
   * `params <id> · exceptions N`(SPEC v1.33 §3.3、TODO/078 §11)。`GET /api/params` を
   * 15 s 毎に poll する REST(WS の `status` には乗らない)。404(まだ無い)/ 503
   * (`ecc_config_dir` 未設定)/ その他の失敗はどれも `null` = 画面では `—`
   * (0 と混同しない —— CLAUDE.md「未知と 0 を混同させない」)。
   */
  readonly paramsId = signal<string | null>(null);
  readonly paramsExceptions = signal<number | null>(null);

  private paramsTimer: ReturnType<typeof setInterval> | null = null;

  private controllerTimer: ReturnType<typeof setInterval> | null = null;

  /**
   * controller の `GET /api/status`(TODO/187)。取れていないときは null = 下の chip は何も
   * 出さず、`ctl` が `—` になる(「異常なし」と取り違えない)。
   */
  readonly controller = signal<StatusView | null>(null);

  /** ③ `STOPPING — draining · ROOT backlog N · waited S s`。Stopping でなければ null。 */
  readonly stoppingText = computed(() => {
    const view = this.controller();
    if (view?.phase !== 'Stopping') return null;
    return drainingLabel(
      view.stopping ?? { decoderBacklog: null, rootSinkBacklog: null, waitedS: null },
    );
  });

  /** ① 全 receiver の `decoder_link_dropped_frames` の合計。0(または未取得)なら null。 */
  readonly rootDropped = computed(() => {
    const total = this.controller()?.rootDroppedFrames ?? 0;
    return total > 0 ? groupDigits(total) : null;
  });

  /** ② Error の receiver / graw-writer の名前。無ければ空。 */
  readonly overloaded = computed(() => overloadedComponents(this.controller()?.components ?? []));

  /** ④ controller が Idle なのに root_sink が run を書いている(WS status の state)。 */
  readonly rootWritingRun = computed(() => {
    const status = this.status();
    if (this.controller()?.phase !== 'Idle' || status?.state !== 'running') return null;
    return status.run;
  });

  constructor() {
    void this.pollParams();
    this.paramsTimer = setInterval(() => void this.pollParams(), STATUS_BAR_PARAMS_POLL_MS);
    void this.pollController();
    this.controllerTimer = setInterval(
      () => void this.pollController(),
      STATUS_BAR_CONTROLLER_POLL_MS,
    );
  }

  ngOnDestroy(): void {
    if (this.paramsTimer !== null) clearInterval(this.paramsTimer);
    this.paramsTimer = null;
    if (this.controllerTimer !== null) clearInterval(this.controllerTimer);
    this.controllerTimer = null;
  }

  private async pollController(): Promise<void> {
    try {
      this.controller.set(parseControllerStatus(await this.controllerApi.getStatus()));
    } catch {
      this.controller.set(null);
    }
  }

  private async pollParams(): Promise<void> {
    try {
      const info = await this.paramsApi.getParams();
      this.paramsId.set(info?.id ?? null);
      this.paramsExceptions.set(info?.exceptions ?? null);
    } catch {
      this.paramsId.set(null);
      this.paramsExceptions.set(null);
    }
  }
}
