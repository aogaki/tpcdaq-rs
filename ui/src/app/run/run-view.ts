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
import { type StatusView, drainingLabel, parseControllerStatus } from '../api/controller-status';
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

/** Stop を追っている間(押してから Idle に戻るまで)の status の poll 間隔(TODO/187)。 */
export const STOP_WATCH_MS = 1000;

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

  /** TODO/187 ③: phase が Stopping の間の「STOPPING — draining · …」。 */
  readonly stoppingText = computed(() => {
    const view = this.status();
    if (view?.phase !== 'Stopping') return null;
    return drainingLabel(
      view.stopping ?? { decoderBacklog: null, rootSinkBacklog: null, waitedS: null },
    );
  });

  /**
   * Stop を押してから Idle に戻るまで true。HTTP の応答(分単位になりうる)でなく phase を
   * 信じて Stopping → Idle を追う。Idle に戻ったら最後の run_stop(ログブック)を `stopOutcome` に出す。
   */
  private awaitingStop = false;
  private sawStopping = false;
  /** Stop を押した時点で run が走っていたか(Stopping を見逃して Idle に戻っていた場合の判定用)。 */
  private runWasActive = false;
  private stopWatchTimer: ReturnType<typeof setInterval> | null = null;
  readonly stopOutcome = signal<{ ok: boolean; text: string; at: string } | null>(null);

  readonly components = computed(() => this.status()?.components ?? []);
  readonly notes = computed(() => this.status()?.notes ?? []);

  /**
   * `Off` 表示の注記(SPEC §8.2 v1.14 / TODO/043 発見・051): 実 ECC の unPrepare 失敗は
   * dhsm が halt して、我々の map では `Off` に見える。実機では getEccServer の
   * 再起動が要ることがある —— ツールチップで伝える(状態そのものは変えない)。
   */
  readonly eccOffHint = 'ECC stopped or halted — on real hardware getEccServer may need a restart';

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
    this.endStopWatch();
  }

  disabled(action: RunAction): boolean {
    return isRunActionDisabled(action, this.uiState());
  }

  /** 送信中のボタンだけ文言を変える(押せないことが見えるように)。 */
  buttonLabel(action: RunAction): string {
    return this.busy() === action.endpoint ? 'sending…' : action.label;
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
        this.statusError.set(
          'The response is not in the expected shape (check what the controller returned)',
        );
        return;
      }
      this.status.set(view);
      this.statusError.set(null);
      this.lastOkAt.set(clockText());
      await this.followStop(view);
    } catch (error) {
      this.statusError.set(describeError(error));
    }
  }

  /** Stop を追っている間: Stopping を見たあと Idle に戻ったら、最後の run_stop を出す。 */
  private async followStop(view: StatusView): Promise<void> {
    if (!this.awaitingStop) return;
    if (view.phase === 'Stopping') this.sawStopping = true;
    else if (view.phase === 'Idle' && this.sawStopping) await this.finishStop();
  }

  private beginStopWatch(): void {
    this.awaitingStop = true;
    this.sawStopping = false;
    this.runWasActive = this.status()?.run != null;
    this.stopOutcome.set(null);
    if (this.stopWatchTimer === null) {
      this.stopWatchTimer = setInterval(() => void this.refresh(), STOP_WATCH_MS);
    }
  }

  private endStopWatch(): void {
    this.awaitingStop = false;
    if (this.stopWatchTimer !== null) clearInterval(this.stopWatchTimer);
    this.stopWatchTimer = null;
  }

  /** Idle に戻った: ログブックの最後(seq 最大)の run_stop の ok / reason を出す。 */
  private async finishStop(): Promise<void> {
    this.endStopWatch();
    const at = clockText();
    try {
      const raw = (await this.api.getLogbook(0)) as { records?: unknown };
      const records = Array.isArray(raw?.records) ? (raw.records as Record<string, unknown>[]) : [];
      const stops = records.filter((r) => r?.['type'] === 'run_stop');
      const last = stops.reduce<Record<string, unknown> | null>(
        (best, r) => (best === null || Number(r['seq']) > Number(best['seq']) ? r : best),
        null,
      );
      if (last === null) {
        this.stopOutcome.set({
          ok: false,
          at,
          text: 'The controller is Idle again, but no run_stop record was found in the logbook',
        });
        return;
      }
      const ok = last['ok'] !== false;
      const counters = last['counters'] as Record<string, unknown> | null | undefined;
      const rootIncomplete = counters?.['root_complete'] === false ? ' · ROOT incomplete' : '';
      this.stopOutcome.set({
        ok,
        at,
        text: `run_stop: run ${String(last['run'])} ${ok ? 'ok' : 'FAILED'} · reason ${String(last['reason'] ?? '—')}${rootIncomplete}`,
      });
    } catch (error) {
      this.stopOutcome.set({
        ok: false,
        at,
        text: `The controller is Idle again, but the run_stop record could not be read: ${describeError(error)}`,
      });
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
    const isStop = action.endpoint === '/api/run/stop';
    try {
      const pending = this.control.execute(action, {
        operator: this.operator(),
        passphrase: this.passphrase(),
        comment: this.comment(),
        nextRun: this.nextRun(),
      });
      if (isStop) {
        // TODO/187 ③: 応答は分単位になりうる。待たずに phase を見に行く。
        this.beginStopWatch();
        void this.refresh();
      }
      const result = await pending;
      if (isStop) {
        if (result.status === 0) {
          // 応答が無い(プロキシ / ブラウザのタイムアウト等)。phase を見て、止まりつつあるなら
          // エラーにしない。止まっていないなら今までどおり失敗として出す。
          await this.refresh();
          const phase = this.status()?.phase;
          const stillStopping = phase === 'Stopping';
          const doneUnseen = phase === 'Idle' && this.runWasActive && !this.sawStopping;
          if (doneUnseen) await this.finishStop();
          if (stillStopping || doneUnseen) return;
        }
        this.endStopWatch();
      }
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
