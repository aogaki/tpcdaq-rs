import { Component, computed, inject } from '@angular/core';
import { MatIconModule } from '@angular/material/icon';
import { MatTooltipModule } from '@angular/material/tooltip';

import { saturationPercent } from '../ws/json';
import { WsClientService } from '../ws/ws-client';

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
export class StatusBar {
  private readonly ws = inject(WsClientService);

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
        return `monitor の WS(${this.ws.url()})に繋がっていない`;
      case 'waiting':
        return '接続済み。最初の status を待っている';
      case 'stale':
        return 'WS は繋がっているが status が 3 秒以上来ない(root-sink 側が止まっている可能性)';
      case 'fresh':
        return 'status を受信中';
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
}
