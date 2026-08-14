/**
 * Run 制御(SPEC §8.1 / R6)。
 *
 * # ユーザー決定(2026-08-13、変更不可)
 *
 * **完成形レイアウトを置き、操作ボタンは全部 disabled**。モック関数も仮バックエンドも作らない。
 * したがってこのコンポーネントは **状態変更系の REST を 1 本も呼ばない**。
 * 有効化は `run-actions.ts` の `RUN_CONTROL_ENABLED` を `true` にするだけ(切替点はその 1 定数)。
 *
 * 呼んでよいのは閲覧系の `GET /api/status` だけ(認証なし)。取れないときは
 * **「取れていない」と書く** —— controller が動いていない開発機で赤い嘘を出さない。
 */
import { Component, type OnDestroy, computed, inject, signal, viewChild } from '@angular/core';

import { ControllerApiService } from '../api/controller-api';
import { type StatusView, parseControllerStatus } from '../api/controller-status';
import { describeError } from '../logbook/logbook-feed';
import {
  RUN_ACTIONS,
  RUN_ACTION_GROUPS,
  RUN_CONTROL_ENABLED,
  type RunAction,
  isRunActionDisabled,
} from './run-actions';

/** status のポーリング間隔(ログブックと同じ刻み)。 */
export const STATUS_POLL_MS = 5000;

function clockText(): string {
  return new Date().toTimeString().slice(0, 8);
}

@Component({
  selector: 'app-run-view',
  templateUrl: './run-view.html',
  styleUrl: './run-view.scss',
})
export class RunView implements OnDestroy {
  private readonly api = inject(ControllerApiService);

  readonly groups = RUN_ACTION_GROUPS;
  readonly enabled = RUN_CONTROL_ENABLED;
  readonly actionCount = RUN_ACTIONS.length;
  readonly pollSeconds = STATUS_POLL_MS / 1000;

  readonly status = signal<StatusView | null>(null);
  readonly statusError = signal<string | null>(null);
  readonly lastOkAt = signal<string | null>(null);
  readonly lastTryAt = signal<string | null>(null);

  /** 操作権の入力欄(**送らない** — P4 で配線する)。 */
  readonly operator = signal('');
  readonly passphrase = signal('');

  readonly components = computed(() => this.status()?.components ?? []);
  readonly notes = computed(() => this.status()?.notes ?? []);

  /** 押されたが配線されていないことの表示(silent failure 禁止)。 */
  readonly notWired = signal<string | null>(null);

  /** 破壊的操作の確認ダイアログ(ガワ。押せないので開くのは P4 以降)。 */
  readonly pending = signal<RunAction | null>(null);
  private readonly confirmDialog = viewChild<{ nativeElement: HTMLDialogElement }>('confirmDialog');

  private timer: ReturnType<typeof setInterval> | null = null;

  constructor() {
    void this.refresh();
    this.timer = setInterval(() => void this.refresh(), STATUS_POLL_MS);
  }

  ngOnDestroy(): void {
    if (this.timer !== null) clearInterval(this.timer);
    this.timer = null;
  }

  disabled(action: RunAction): boolean {
    return isRunActionDisabled(action);
  }

  setOperator(event: Event): void {
    this.operator.set((event.target as HTMLInputElement).value);
  }

  setPassphrase(event: Event): void {
    this.passphrase.set((event.target as HTMLInputElement).value);
  }

  async refresh(): Promise<void> {
    this.lastTryAt.set(clockText());
    try {
      const raw = await this.api.getStatus();
      const view = parseControllerStatus(raw);
      if (!view) {
        this.statusError.set('応答が想定の形ではありません(controller の返事を確認してください)');
        return;
      }
      this.status.set(view);
      this.statusError.set(null);
      this.lastOkAt.set(clockText());
    } catch (error) {
      this.statusError.set(describeError(error));
    }
  }

  /**
   * **P4 の唯一の配線点**。029 では REST を呼ばない(ユーザー決定: モック禁止)。
   * 破壊的操作は先に確認ダイアログを出す(SPEC §11)。
   */
  activate(action: RunAction): void {
    if (action.destructive) {
      this.pending.set(action);
      this.confirmDialog()?.nativeElement.showModal();
      return;
    }
    this.proceed(action);
  }

  confirm(): void {
    const action = this.pending();
    this.closeConfirm();
    if (action) this.proceed(action);
  }

  closeConfirm(): void {
    this.confirmDialog()?.nativeElement.close();
    this.pending.set(null);
  }

  /**
   * ここに `POST {action.endpoint}` が入る(P4)。029 は**何も送らない**。
   * 押されたのに黙って無反応、にはしない(silent failure 禁止)。
   */
  private proceed(action: RunAction): void {
    this.notWired.set(
      `${action.label}: まだ何も送っていません。P4 で POST ${action.endpoint} ${action.body} を配線します。`,
    );
  }
}
