/**
 * Run 制御(SPEC §8.1 / R6)。**050 で実配線**した。
 *
 * # 経緯
 *
 * 029(ユーザー決定 2026-08-13)では完成形レイアウトを置いて全 disabled にした
 * (モック関数・仮バックエンドを作らない)。エンドポイントが実在し実走まで確認できた(041)ので、
 * 050 で `RUN_CONTROL_ENABLED = true` にし、`proceed()` から実際に POST する。
 *
 * # このコンポーネントの分担
 *
 * - 閲覧系の `GET /api/status` は `ControllerApiService`(認証なし)。取れないときは
 *   **「取れていない」と書く** —— controller が動いていない開発機で赤い嘘を出さない。
 * - 操作系は `RunControlApiService`(token 必須・監査あり)。**失敗は応答の `error` を
 *   そのまま出す**。`notes`(run/stop)も全部出す。
 * - 破壊的操作(SPEC §11)は確認ダイアログを挟む。
 */
import { Component, type OnDestroy, computed, inject, signal, viewChild } from '@angular/core';

import { ControllerApiService } from '../api/controller-api';
import { type StatusView, parseControllerStatus } from '../api/controller-status';
import { type ActionResult, RunControlApiService } from '../api/run-control-api';
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

/** 直近 1 回の操作の結果(画面上端に出す)。 */
export interface LastAction extends ActionResult {
  readonly label: string;
  readonly endpoint: string;
  readonly at: string;
}

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
  private readonly control = inject(RunControlApiService);

  readonly groups = RUN_ACTION_GROUPS;
  readonly enabled = RUN_CONTROL_ENABLED;
  readonly actionCount = RUN_ACTIONS.length;
  readonly pollSeconds = STATUS_POLL_MS / 1000;

  readonly status = signal<StatusView | null>(null);
  readonly statusError = signal<string | null>(null);
  readonly lastOkAt = signal<string | null>(null);
  readonly lastTryAt = signal<string | null>(null);

  /** 入力欄。 */
  readonly operator = signal('');
  readonly passphrase = signal('');
  readonly comment = signal('');
  readonly nextRun = signal('');

  /** 送信中の endpoint(1 度に 1 本だけ。run/start は ≈7 s かかる — 041 実測)。 */
  readonly busy = signal<string | null>(null);
  readonly lastAction = signal<LastAction | null>(null);

  readonly components = computed(() => this.status()?.components ?? []);
  readonly notes = computed(() => this.status()?.notes ?? []);

  /**
   * `Off` 表示の注記(SPEC §8.2 v1.14 / TODO/043 発見・051): 実 ECC の unPrepare 失敗は
   * dhsm が halt して、我々の map では `Off` に見える。実機では getEccServer の
   * 再起動が要ることがある —— ツールチップで伝える(状態そのものは変えない)。
   */
  readonly eccOffHint = 'ECC 停止または halt — 実機では getEccServer の再起動が必要な場合あり';

  /** 操作権(token はメモリ + sessionStorage。画面には頭だけ出す)。 */
  readonly hasToken = computed(() => this.control.token() !== null);
  readonly tokenHint = computed(() => {
    const token = this.control.token();
    return token === null ? null : `${token.slice(0, 6)}…`;
  });
  readonly preempted = this.control.preempted;

  /** ボタンの有効/無効を決める状態(規則は `run-actions.ts`)。 */
  readonly uiState = computed(() => ({
    enabled: RUN_CONTROL_ENABLED,
    hasToken: this.hasToken(),
    runActive: this.status()?.run != null,
    busy: this.busy() !== null,
  }));

  /** 破壊的操作の確認ダイアログ(SPEC §11)。 */
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
    return isRunActionDisabled(action, this.uiState());
  }

  /** 送信中のボタンだけ文言を変える(押せないことが見えるように)。 */
  buttonLabel(action: RunAction): string {
    return this.busy() === action.endpoint ? '送信中…' : action.label;
  }

  setOperator(event: Event): void {
    this.operator.set((event.target as HTMLInputElement).value);
  }

  setPassphrase(event: Event): void {
    this.passphrase.set((event.target as HTMLInputElement).value);
  }

  setComment(event: Event): void {
    this.comment.set((event.target as HTMLInputElement).value);
  }

  setNextRun(event: Event): void {
    this.nextRun.set((event.target as HTMLInputElement).value);
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

  /** 破壊的操作は先に確認ダイアログを出す(SPEC §11)。それ以外は即送信。 */
  activate(action: RunAction): void {
    if (action.destructive) {
      this.pending.set(action);
      this.confirmDialog()?.nativeElement.showModal();
      return;
    }
    void this.proceed(action);
  }

  confirm(): void {
    const action = this.pending();
    this.closeConfirm();
    if (action) void this.proceed(action);
  }

  closeConfirm(): void {
    this.confirmDialog()?.nativeElement.close();
    this.pending.set(null);
  }

  /**
   * **050 の配線点**。`POST {action.endpoint}`(body は表のとおり)を投げ、結果を必ず画面に出す。
   * 成功・失敗どちらでも直後に status を取り直す(押した効果を待たせない)。
   */
  private async proceed(action: RunAction): Promise<void> {
    if (this.busy() !== null) return; // 二重送信防止(disabled の取りこぼし対策)
    this.busy.set(action.endpoint);
    this.lastAction.set(null);
    try {
      const result = await this.control.execute(action, {
        operator: this.operator(),
        passphrase: this.passphrase(),
        comment: this.comment(),
        nextRun: this.nextRun(),
      });
      this.lastAction.set({
        ...result,
        label: action.label,
        endpoint: action.endpoint,
        at: clockText(),
      });
      // 取れた token を画面に残さない(credential を DOM に置きっぱなしにしない)。
      if (action.endpoint === '/api/control/acquire' && result.ok) this.passphrase.set('');
    } finally {
      this.busy.set(null);
    }
    await this.refresh();
  }
}
