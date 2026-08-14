import { Component, computed, inject } from '@angular/core';
import { MatButtonModule } from '@angular/material/button';
import { MatIconModule } from '@angular/material/icon';
import { MatListModule } from '@angular/material/list';
import { MatSidenavModule } from '@angular/material/sidenav';
import { MatToolbarModule } from '@angular/material/toolbar';
import { MatTooltipModule } from '@angular/material/tooltip';
import { RouterLink, RouterLinkActive, RouterOutlet } from '@angular/router';

import { NAV_ITEMS } from './app.routes';
import { StatusBar } from './status-bar/status-bar';
import { EndpointsService } from './ws/endpoints.service';
import { WsClientService } from './ws/ws-client';

/**
 * アプリシェル(SPEC §11): Material の toolbar + サイドナビ + 5 ルート + status バー。
 * 意匠の作り込みは 029。ここは骨格と WS 配線だけ。
 */
@Component({
  selector: 'app-root',
  imports: [
    RouterOutlet,
    RouterLink,
    RouterLinkActive,
    MatToolbarModule,
    MatSidenavModule,
    MatListModule,
    MatIconModule,
    MatButtonModule,
    MatTooltipModule,
    StatusBar,
  ],
  templateUrl: './app.html',
  styleUrl: './app.scss',
})
export class App {
  private readonly ws = inject(WsClientService);
  private readonly endpoints = inject(EndpointsService);

  readonly nav = NAV_ITEMS;
  readonly meta = this.ws.meta;
  readonly wsUrl = this.endpoints.wsUrl;
  readonly endpointSource = this.endpoints.source;

  /** toolbar の副題(検出器名 + ジオメトリ。meta 未着なら接続先を出す)。 */
  readonly subtitle = computed(() => {
    const meta = this.meta();
    if (!meta) return this.wsUrl() || 'connecting…';
    return `${meta.detector} · ${meta.geometry} · ${meta.nBuckets} buckets`;
  });

  constructor() {
    // 起動時に接続先を決めて WS を開く(以後は WsClientService が繋ぎ直す)。
    void this.start();
  }

  private async start(): Promise<void> {
    const wsUrl = await this.endpoints.load(window.location, (url) => fetch(url));
    this.ws.connect(wsUrl);
  }
}
