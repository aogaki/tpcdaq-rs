/**
 * SPEC §8.1 の run 制御操作の表(純データ)+ ボタンの有効/無効の規則(純ロジック)。
 *
 * # 経緯
 *
 * 029(ユーザー決定 2026-08-13)では**完成形レイアウトを置いて全部 disabled**にした
 * (モック関数・仮バックエンドを作らない)。**050 で実配線**したので `RUN_CONTROL_ENABLED`
 * は `true`。表そのもの(endpoint / body / destructive)は 029 のまま**無変更** ——
 * controller の実装(`src/controller.rs` のハンドラ)と一致していることは
 * `api/run-control-api.spec.ts` が機械照合する。
 *
 * このモジュールは今も **REST を呼ばない**(呼ぶのは `api/run-control-api.ts`)。
 */

/** 050 で実配線済み。**切替点はこの 1 定数**(false に戻せば操作は全 disabled に戻る)。 */
export const RUN_CONTROL_ENABLED = true;

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
  { name: 'describe', note: 'Send the hardware description', destructive: false },
  { name: 'prepare', note: 'Prepare AsAd / AGET', destructive: false },
  { name: 'configure', note: 'Hand over the DataLinkSet and configure', destructive: false },
  { name: 'start', note: 'Start acquisition on the ECC side', destructive: false },
  { name: 'stop', note: 'Stop acquisition on the ECC side', destructive: true },
  { name: 'breakup', note: 'Tear down the data links', destructive: true },
  { name: 'reset', note: 'Bring ECC back to its initial state', destructive: true },
];

export const RUN_ACTION_GROUPS: readonly RunActionGroup[] = [
  {
    id: 'control',
    label: 'Control',
    note: 'The control token is held by exactly one client. Acquire can always take it over, and every takeover is kept in the audit log (SPEC §8.1).',
    actions: [
      {
        label: 'Acquire',
        endpoint: '/api/control/acquire',
        body: '{operator, passphrase}',
        destructive: false,
        note: 'Returns a token on success (attach it to every state-changing call)',
      },
      {
        label: 'Release',
        endpoint: '/api/control/release',
        body: '{token}',
        destructive: false,
        note: 'Release the control token',
      },
    ],
  },
  {
    id: 'run',
    label: 'Run',
    note: 'The run sequence of SPEC §1.3. Run numbers are assigned by the controller (set one by hand with next).',
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
        note: 'ECC stop → EndOfStream propagation → files finalized → run_stop recorded',
      },
      {
        label: 'Set next run',
        endpoint: '/api/run/next',
        body: '{token, next_run}',
        destructive: false,
        note: 'Positive integers only. Rejected while a run is in progress. Takes effect at the next start',
      },
    ],
  },
  {
    id: 'ecc',
    label: 'ECC step operations',
    note: 'R6: the same feel as the GET controller. A normal run drives these automatically from start/stop, so this group is for recovery.',
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

/** ボタンの有効/無効を決めるのに要る画面の状態(050-C)。 */
export interface RunControlUiState {
  /** 配線が有効か(既定 = `RUN_CONTROL_ENABLED`)。 */
  readonly enabled: boolean;
  /** 操作権の token を持っているか。 */
  readonly hasToken: boolean;
  /** `GET /api/status` の `run` が非 null(= run 実行中)。 */
  readonly runActive: boolean;
  /** 送信中の要求があるか(二重送信防止。run/start は ≈7 s かかる)。 */
  readonly busy: boolean;
}

/** 何も持っていない・何も起きていない状態(テストと初期表示の起点)。 */
export const INITIAL_UI_STATE: RunControlUiState = {
  enabled: RUN_CONTROL_ENABLED,
  hasToken: false,
  runActive: false,
  busy: false,
};

/**
 * ボタンを押せるか。**規則は最小限**(050-C):
 *
 * 1. 配線が無効 → 全 disabled(029 の出荷形に戻る)。
 * 2. 送信中 → 全 disabled(二重送信と操作の交差を防ぐ)。
 * 3. Acquire は常に押せる(token を得る唯一の入口。横取りは仕様どおり常に可 — §8.1)。
 * 4. token 無し → 他は全 disabled。
 * 5. run 実行中 → `run/start` と `run/next` を disabled(stop は有効)。
 *
 * **ECC 段階操作は ECC の state で先回りガードしない**(KISS)。実 ECC は不正な順序に
 * Ignored/Denied を返す(036/049 で確認済み)ので、結果のエラー表示で十分。
 */
export function isRunActionDisabled(action: RunAction, state: RunControlUiState): boolean {
  if (!state.enabled) return true;
  if (state.busy) return true;
  if (action.endpoint === '/api/control/acquire') return false;
  if (!state.hasToken) return true;
  if (state.runActive) {
    return action.endpoint === '/api/run/start' || action.endpoint === '/api/run/next';
  }
  return false;
}
