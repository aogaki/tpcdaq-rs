/**
 * SPEC §8.1 の run 制御操作の**完成形レイアウト**の表(純データ)。
 *
 * # ユーザー決定(2026-08-13、変更不可)
 *
 * **ボタン類は完成形レイアウトを置き、全部 disabled**。モック関数・仮バックエンドを作らない。
 * したがってこのモジュールは「どんなボタンがどこに並ぶか」だけを持ち、**REST を呼ぶコードは
 * 1 行も無い**(P4 で配線する)。`GET /api/status` とログブックだけは閲覧系なので呼んでよい
 * —— そちらは `api/controller-api.ts`。
 *
 * # 有効化
 *
 * P4 では `RUN_CONTROL_ENABLED` を `true` にし、`run-view` の `submit()` に REST 呼び出しを
 * 足すだけで済むようにしてある(**切替点はこの 1 定数**)。
 */

export const RUN_CONTROL_ENABLED = false;

export interface RunAction {
  /** ボタンの文言。 */
  readonly label: string;
  /** SPEC §8.1 のエンドポイント(**表示専用** — ここから fetch は呼ばない)。 */
  readonly endpoint: string;
  /** 必要なパラメタ(P4 の配線先を画面に明示する)。 */
  readonly body: string;
  /** 破壊的操作 = 確認ダイアログを出す対象(SPEC §11)。 */
  readonly destructive: boolean;
  /** 補足(何が起きるか)。 */
  readonly note: string;
}

export interface RunActionGroup {
  readonly id: 'control' | 'run' | 'ecc';
  readonly label: string;
  readonly note: string;
  readonly actions: readonly RunAction[];
}

/** ECC の段階操作(R6: GET controller と同じ操作感で並べる)。すべて `{token}`。 */
const ECC_STEPS: readonly { name: string; note: string; destructive: boolean }[] = [
  { name: 'describe', note: 'ハードウェア記述を送る', destructive: false },
  { name: 'prepare', note: 'AsAd / AGET を準備する', destructive: false },
  { name: 'configure', note: 'DataLinkSet を渡して設定する', destructive: false },
  { name: 'start', note: 'ECC 側の取得を開始する', destructive: false },
  { name: 'stop', note: 'ECC 側の取得を止める', destructive: true },
  { name: 'breakup', note: 'データリンクを解体する', destructive: true },
  { name: 'reset', note: 'ECC を初期状態へ戻す', destructive: true },
];

export const RUN_ACTION_GROUPS: readonly RunActionGroup[] = [
  {
    id: 'control',
    label: '操作権',
    note: '操作権は常に 1 クライアント。取得は常に横取り可で、横取りは監査ログに残る(SPEC §8.1)。',
    actions: [
      {
        label: 'Acquire',
        endpoint: '/api/control/acquire',
        body: '{operator, passphrase}',
        destructive: false,
        note: '成功すると token が返る(以後の状態変更系に付ける)',
      },
      {
        label: 'Release',
        endpoint: '/api/control/release',
        body: '{token}',
        destructive: false,
        note: '操作権を手放す',
      },
    ],
  },
  {
    id: 'run',
    label: 'Run',
    note: 'SPEC §1.3 の run シーケンス。run 番号は controller が採番する(手動設定は next)。',
    actions: [
      {
        label: 'Start run',
        endpoint: '/api/run/start',
        body: '{token, comment?}',
        destructive: false,
        note: 'Configure → Arm → ECC configure/start → Start(listen-before-start)',
      },
      {
        label: 'Stop run',
        endpoint: '/api/run/stop',
        body: '{token}',
        destructive: true,
        note: 'ECC stop → EndOfStream 伝播 → ファイル確定 → run_stop を記録',
      },
      {
        label: 'Set next run',
        endpoint: '/api/run/next',
        body: '{token, next_run}',
        destructive: false,
        note: '正整数のみ。run 実行中は拒否。次の start から有効',
      },
    ],
  },
  {
    id: 'ecc',
    label: 'ECC 段階操作',
    note: 'R6: GET controller と同じ操作感。通常の run では start/stop が自動で回すので、ここは復旧用。',
    actions: ECC_STEPS.map((step) => ({
      label: `ECC ${step.name}`,
      endpoint: `/api/ecc/${step.name}`,
      body: '{token}',
      destructive: step.destructive,
      note: step.note,
    })),
  },
];

/** 群をまたいだ通しの一覧(テストと「全 disabled」の確認用)。 */
export const RUN_ACTIONS: readonly RunAction[] = RUN_ACTION_GROUPS.flatMap(
  (group) => group.actions,
);

/**
 * ボタンを押せるか。**既定はフラグ由来で常に disabled**。
 * `enabled` を引数に出してあるのは、テストが「フラグ 1 つで切り替わる」ことを
 * 確かめられるようにするため(実画面は既定値を使う)。
 */
export function isRunActionDisabled(_action: RunAction, enabled = RUN_CONTROL_ENABLED): boolean {
  return !enabled;
}
