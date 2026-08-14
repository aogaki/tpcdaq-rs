import { Routes } from '@angular/router';

/** サイドナビの見出しはここから作る(ルート表と 1 か所で対応させる)。 */
export interface NavItem {
  readonly path: string;
  readonly label: string;
  readonly icon: string;
}

export const NAV_ITEMS: readonly NavItem[] = [
  { path: 'monitor', label: 'Monitor', icon: 'insights' },
  { path: 'waveform', label: 'Waveform', icon: 'show_chart' },
  { path: 'logbook', label: 'Logbook', icon: 'history_edu' },
  { path: 'run', label: 'Run control', icon: 'play_circle' },
  { path: 'power', label: 'Power', icon: 'bolt' },
];

export const routes: Routes = [
  { path: '', redirectTo: 'monitor', pathMatch: 'full' },
  // 028: JSROOT / ECharts を初期バンドルに入れないため**遅延ルート**にする
  // (それぞれのビューがさらに動的 import で描画ライブラリを引く)。
  {
    path: 'monitor',
    loadComponent: () => import('./monitor/monitor-view').then((m) => m.MonitorView),
  },
  {
    path: 'waveform',
    loadComponent: () => import('./waveform/waveform-view').then((m) => m.WaveformView),
  },
  // 029: 残る 3 ビュー。どれも遅延ルート(シェルの初期チャンクを太らせない)。
  {
    path: 'logbook',
    loadComponent: () => import('./logbook/logbook-view').then((m) => m.LogbookView),
  },
  {
    path: 'run',
    loadComponent: () => import('./run/run-view').then((m) => m.RunView),
  },
  {
    path: 'power',
    loadComponent: () => import('./power/power-view').then((m) => m.PowerView),
  },
  { path: '**', redirectTo: 'monitor' },
];
