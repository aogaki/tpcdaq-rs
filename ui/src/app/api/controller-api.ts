/**
 * controller REST(SPEC §8.1)のうち **UI が呼んでよい 3 本だけ**を持つ薄いクライアント。
 *
 * - `GET /api/status` — 閲覧系(認証なし)
 * - `GET /api/logbook?since_seq=N` — 閲覧系(認証なし)
 * - `POST /api/logbook/comment {author, text}` — **token 不要**(SPEC v1.10 §8.1 の明文化された
 *   例外。DAQ 状態を変えない記録系で、R11「シフト全員が書けるログブック」を優先する)
 *
 * **状態変更系(acquire / release / run / ecc)はここに書かない**(029 のユーザー決定:
 * モック・仮バックエンドを作らない。P4 で配線する)。
 *
 * `fetch` を直に使う(027 の `ui-config.json` 読み込みと同じ流儀。`provideHttpClient` を
 * 増やさない = KISS)。**投げる**ので、呼び手が捕まえて画面に出す(silent 禁止)。
 */

import { Injectable, inject } from '@angular/core';

import { EndpointsService } from '../ws/endpoints.service';

/** `apiBase`(既定 `/api`、`ui-config.json` で絶対 URL に差し替え可)+ パス。 */
export function apiUrl(base: string, path: string): string {
  return `${base.replace(/\/+$/, '')}/${path}`;
}

@Injectable({ providedIn: 'root' })
export class ControllerApiService {
  private readonly endpoints = inject(EndpointsService);

  /** `GET /api/logbook?since_seq=N` → `{records, tail_corrupt}`(生のまま返す)。 */
  getLogbook(sinceSeq: number): Promise<unknown> {
    return this.getJson(`logbook?since_seq=${sinceSeq}`);
  }

  /** `GET /api/status` → SPEC §8.1 の状態一式(生のまま返す)。 */
  getStatus(): Promise<unknown> {
    return this.getJson('status');
  }

  /** `POST /api/logbook/comment {author, text}` → `{appended: true}`。token は要らない。 */
  async postComment(author: string, text: string): Promise<unknown> {
    const url = apiUrl(this.endpoints.apiBase(), 'logbook/comment');
    const response = await fetch(url, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ author, text }),
    });
    if (!response.ok) {
      throw new Error(`POST ${url} → HTTP ${response.status} ${response.statusText}`);
    }
    return response.json();
  }

  private async getJson(path: string): Promise<unknown> {
    const url = apiUrl(this.endpoints.apiBase(), path);
    const response = await fetch(url);
    if (!response.ok) {
      throw new Error(`GET ${url} → HTTP ${response.status} ${response.statusText}`);
    }
    return response.json();
  }
}
