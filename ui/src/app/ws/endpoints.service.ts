import { Injectable, signal } from '@angular/core';

import { DEFAULT_API_BASE, type FetchLike, loadEndpoints } from './endpoints';

/**
 * 解決済みの接続先を 1 か所で持つ(WS は 027 が、REST は 029 が使う)。
 *
 * 決定規則そのものは Angular 非依存の `./endpoints` にある(テストはそちら)。
 * ここは「起動時に 1 回だけ解決して signal に置く」だけの薄い入れ物。
 */
@Injectable({ providedIn: 'root' })
export class EndpointsService {
  readonly wsUrl = signal<string>('');
  readonly apiBase = signal<string>(DEFAULT_API_BASE);
  /** 既定か `ui-config.json` か(画面と起動ログに出す)。 */
  readonly source = signal<string>('defaults');

  async load(location: Location, fetchFn: FetchLike): Promise<string> {
    const resolved = await loadEndpoints(location, fetchFn);
    this.wsUrl.set(resolved.endpoints.wsUrl);
    this.apiBase.set(resolved.endpoints.apiBase);
    this.source.set(resolved.source);
    return resolved.endpoints.wsUrl;
  }
}
