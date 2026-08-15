/**
 * SPEC §8.1 の操作一覧(表)と、050-C の **disabled 規則**。
 *
 * 表(endpoint / body / destructive)は 029 のまま**無変更**。変わったのは
 * 「押せるかどうか」の部分だけ —— 029 は「フラグ由来で常に disabled」、
 * 050 は「配線済み + 最小限の規則」。
 */
import {
  RUN_ACTIONS,
  RUN_ACTION_GROUPS,
  RUN_CONTROL_ENABLED,
  type RunControlUiState,
  isRunActionDisabled,
} from './run-actions';

/** 何も持っていない状態(配線は有効)。テストはここから 1 項目ずつ動かす。 */
const IDLE: RunControlUiState = {
  enabled: true,
  hasToken: false,
  runActive: false,
  busy: false,
};

const endpointsDisabledIn = (state: RunControlUiState): string[] =>
  RUN_ACTIONS.filter((action) => isRunActionDisabled(action, state)).map((a) => a.endpoint);

describe('050 — 配線の切替点は 1 定数', () => {
  it('050 で配線したので出荷値は true', () => {
    expect(RUN_CONTROL_ENABLED).toBe(true);
  });

  it('フラグを false に戻せば 029 の出荷形(全 disabled)に戻る', () => {
    const off: RunControlUiState = { ...IDLE, enabled: false, hasToken: true };
    expect(endpointsDisabledIn(off)).toEqual(RUN_ACTIONS.map((a) => a.endpoint));
  });
});

describe('050-C — disabled 規則は最小限', () => {
  it('token 無し: Acquire だけ押せる(token を得る唯一の入口)', () => {
    expect(endpointsDisabledIn(IDLE)).toEqual(
      RUN_ACTIONS.map((a) => a.endpoint).filter((e) => e !== '/api/control/acquire'),
    );
  });

  it('token 有り + run 無し: 12 個すべて押せる', () => {
    expect(endpointsDisabledIn({ ...IDLE, hasToken: true })).toEqual([]);
  });

  it('run 実行中: start 系(run/start, run/next)だけ disabled、run/stop は有効', () => {
    const running: RunControlUiState = { ...IDLE, hasToken: true, runActive: true };
    expect(endpointsDisabledIn(running)).toEqual(['/api/run/start', '/api/run/next']);
  });

  it('ECC 段階操作は run 中でも押せる(先回りガードを作らない = KISS)', () => {
    const running: RunControlUiState = { ...IDLE, hasToken: true, runActive: true };
    const ecc = RUN_ACTIONS.filter((a) => a.endpoint.startsWith('/api/ecc/'));
    expect(ecc).toHaveLength(7);
    for (const action of ecc) {
      expect(isRunActionDisabled(action, running)).toBe(false);
    }
  });

  it('送信中(run/start は ≈7 s)はすべて disabled = 二重送信できない', () => {
    const busy: RunControlUiState = { ...IDLE, hasToken: true, busy: true };
    expect(endpointsDisabledIn(busy)).toEqual(RUN_ACTIONS.map((a) => a.endpoint));
  });
});

describe('SPEC §8.1 の操作一覧(完成形レイアウト)', () => {
  it('操作権 / run / ecc の 3 群に分かれている', () => {
    expect(RUN_ACTION_GROUPS.map((g) => g.id)).toEqual(['control', 'run', 'ecc']);
  });

  it('群に並ぶ操作の合計が一覧と一致する(表示漏れを作らない)', () => {
    const fromGroups = RUN_ACTION_GROUPS.flatMap((g) => g.actions);
    expect(fromGroups).toEqual([...RUN_ACTIONS]);
  });

  it('エンドポイントは §8.1 のとおり(操作権 2 + run 3 + ecc 7 = 12)', () => {
    expect(RUN_ACTIONS.map((a) => a.endpoint)).toEqual([
      '/api/control/acquire',
      '/api/control/release',
      '/api/run/start',
      '/api/run/stop',
      '/api/run/next',
      '/api/ecc/describe',
      '/api/ecc/prepare',
      '/api/ecc/configure',
      '/api/ecc/start',
      '/api/ecc/stop',
      '/api/ecc/breakup',
      '/api/ecc/reset',
    ]);
  });

  it('必要なパラメタも表に持つ(画面に出して P4 の配線先を明示する)', () => {
    const body = (endpoint: string) =>
      RUN_ACTIONS.find((a) => a.endpoint === endpoint)?.body ?? '(無い)';
    expect(body('/api/control/acquire')).toBe('{operator, passphrase}');
    expect(body('/api/run/start')).toBe('{token, comment?}');
    expect(body('/api/run/next')).toBe('{token, next_run}');
    expect(body('/api/ecc/configure')).toBe('{token}');
  });

  it('破壊的操作は stop / breakup / reset(確認ダイアログを出す対象、SPEC §11)', () => {
    expect(RUN_ACTIONS.filter((a) => a.destructive).map((a) => a.endpoint)).toEqual([
      '/api/run/stop',
      '/api/ecc/stop',
      '/api/ecc/breakup',
      '/api/ecc/reset',
    ]);
  });
});
