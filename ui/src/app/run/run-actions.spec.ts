/**
 * SPEC §8.1 の操作一覧と **全 disabled**(ユーザー決定 2026-08-13、029 発注書)。
 *
 * ここでの受け入れは 2 つ:
 * ① 既定では**すべての操作が disabled**(モック・仮バックエンドを作らない)。
 * ② 有効化が**フラグ 1 つ**で済む(P4 の配線時に `RUN_CONTROL_ENABLED` だけを触る)。
 */
import {
  RUN_ACTIONS,
  RUN_ACTION_GROUPS,
  RUN_CONTROL_ENABLED,
  isRunActionDisabled,
} from './run-actions';

describe('ユーザー決定 — Run 制御は完成形レイアウト + 全 disabled', () => {
  it('フラグは false で出荷する', () => {
    expect(RUN_CONTROL_ENABLED).toBe(false);
  });

  it('既定ではすべての操作が disabled', () => {
    expect(RUN_ACTIONS.length).toBeGreaterThan(0);
    for (const action of RUN_ACTIONS) {
      expect(isRunActionDisabled(action)).toBe(true);
    }
  });

  it('フラグ 1 つで全部 enabled に変わる(P4 の配線点はここだけ)', () => {
    for (const action of RUN_ACTIONS) {
      expect(isRunActionDisabled(action, true)).toBe(false);
    }
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
