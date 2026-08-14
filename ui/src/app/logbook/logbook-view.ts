/**
 * ログブック(R11 / SPEC §9)。
 *
 * - `GET /api/logbook?since_seq=N` を **5 s ポーリング**して追従(WS には載っていない)。
 * - `POST /api/logbook/comment {author, text}` で追記(**token 不要** — SPEC v1.10 §8.1)。
 * - 追従・整形の中身は純ロジック(`logbook-feed.ts` / `logbook-record.ts`)。
 *   ここは「叩く・並べる・入力を受ける」だけの薄い層。
 *
 * **silent failure を作らない**: ポーリング失敗 / `tail_corrupt` / 未知 type / 取り込めなかった
 * 行は、数と最終試行時刻つきで画面の上端に出す。
 */
import { Component, type OnDestroy, computed, inject, signal } from '@angular/core';

import { ControllerApiService } from '../api/controller-api';
import {
  EMPTY_LOGBOOK_FEED,
  LOGBOOK_POLL_MS,
  applyLogbookError,
  applyLogbookResponse,
  describeError,
} from './logbook-feed';

/** ローカル時刻の `HH:MM:SS`(ロケールで揺れない形)。 */
function clockText(): string {
  return new Date().toTimeString().slice(0, 8);
}

@Component({
  selector: 'app-logbook-view',
  templateUrl: './logbook-view.html',
  styleUrl: './logbook-view.scss',
})
export class LogbookView implements OnDestroy {
  private readonly api = inject(ControllerApiService);

  readonly pollSeconds = LOGBOOK_POLL_MS / 1000;

  private readonly feed = signal(EMPTY_LOGBOOK_FEED);

  /** 新しい順(記録は seq 昇順で持ち、表示だけ反転する)。 */
  readonly entries = computed(() => [...this.feed().entries].reverse());
  readonly sinceSeq = computed(() => this.feed().sinceSeq);
  readonly tailCorrupt = computed(() => this.feed().tailCorrupt);
  readonly unknownTypes = computed(() => this.feed().unknownTypes);
  readonly malformed = computed(() => this.feed().malformed);
  readonly lastError = computed(() => this.feed().lastError);

  /** 最後に**成功した**取得と、最後に**試した**時刻(区別する)。 */
  readonly lastOkAt = signal<string | null>(null);
  readonly lastTryAt = signal<string | null>(null);

  readonly author = signal('');
  readonly text = signal('');
  readonly posting = signal(false);
  readonly postError = signal<string | null>(null);
  readonly postedAt = signal<string | null>(null);

  readonly canPost = computed(
    () => !this.posting() && this.author().trim().length > 0 && this.text().trim().length > 0,
  );

  private timer: ReturnType<typeof setInterval> | null = null;

  constructor() {
    void this.refresh();
    this.timer = setInterval(() => void this.refresh(), LOGBOOK_POLL_MS);
  }

  ngOnDestroy(): void {
    if (this.timer !== null) clearInterval(this.timer);
    this.timer = null;
  }

  setAuthor(event: Event): void {
    this.author.set((event.target as HTMLInputElement).value);
  }

  setText(event: Event): void {
    this.text.set((event.target as HTMLTextAreaElement).value);
  }

  async refresh(): Promise<void> {
    this.lastTryAt.set(clockText());
    try {
      const raw = await this.api.getLogbook(this.feed().sinceSeq);
      this.feed.update((state) => applyLogbookResponse(state, raw));
      if (this.feed().lastError === null) this.lastOkAt.set(clockText());
    } catch (error) {
      this.feed.update((state) => applyLogbookError(state, error));
    }
  }

  async post(): Promise<void> {
    if (!this.canPost()) return;
    this.posting.set(true);
    this.postError.set(null);
    try {
      await this.api.postComment(this.author().trim(), this.text().trim());
      this.text.set('');
      this.postedAt.set(clockText());
      // 追記した行をすぐ見せる(次のポーリングを待たない)。
      await this.refresh();
    } catch (error) {
      this.postError.set(describeError(error));
    } finally {
      this.posting.set(false);
    }
  }
}
