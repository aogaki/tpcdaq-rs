/**
 * SPEC §8.1 `GET /api/status`(閲覧系 = 認証なし)の整形。
 *
 * フィクスチャは **実際に走っている controller の応答**から取った形
 * (`src/controller.rs` の `get_status`。ecc-bridge を上げていない開発機では
 * `ecc.state = "Unknown"` になるのが正 —— これを画面で「不明」と出せることが受け入れ条件)。
 *
 * **取れていないものを 0 や「正常」と偽らない**のがこのモジュールの仕事。
 */
import { eccTone, parseControllerStatus } from './controller-status';

/** ecc-bridge なしで実測した応答(components の metrics は長いので 1 台分だけ残した)。 */
const LIVE = {
  phase: 'Idle',
  run: null,
  operator: null,
  components: [
    {
      name: 'graw-writer',
      endpoint: 'tcp://127.0.0.1:47100',
      reachable: true,
      state: 'Running',
      run_number: 1,
      metrics: { files_open: 2 },
      error: null,
    },
    {
      name: 'receiver0',
      endpoint: 'tcp://127.0.0.1:47110',
      reachable: false,
      state: null,
      run_number: null,
      metrics: null,
      error: 'no reply within 1000 ms',
    },
  ],
  ecc: {
    ok: false,
    state: 'Unknown',
    error:
      'ecc-bridge tcp://127.0.0.1:47200: no reply within 1000 ms: Resource temporarily unavailable',
  },
  log_posts: { accepted: 0, rejected: 0 },
  notes: [],
} as unknown;

describe('全体', () => {
  it('phase / operator / log_posts / notes をそのまま出す', () => {
    const view = parseControllerStatus(LIVE);
    expect(view).not.toBeNull();
    expect(view?.phase).toBe('Idle');
    expect(view?.operator).toBeNull();
    expect(view?.logPosts).toEqual({ accepted: 0, rejected: 0 });
    expect(view?.notes).toEqual([]);
  });

  it('run が null なら「実行中の run なし」を表す null(0 番の run を捏造しない)', () => {
    expect(parseControllerStatus(LIVE)?.run).toBeNull();
  });

  it('run 実行中は run 番号 / operator / 経過秒を持つ', () => {
    const raw = {
      ...(LIVE as Record<string, unknown>),
      phase: 'Running',
      run: {
        run: 58,
        operator: 'aogaki',
        comment: 'gas test',
        started_ts: '2026-08-14T15:53:12.332',
        elapsed_s: 12.5,
      },
      operator: 'aogaki',
    };
    const view = parseControllerStatus(raw);
    expect(view?.run).toEqual({
      run: 58,
      operator: 'aogaki',
      comment: 'gas test',
      startedTs: '2026-08-14T15:53:12.332',
      elapsedS: 12.5,
    });
    expect(view?.operator).toBe('aogaki');
  });
});

describe('components(未知と 0 を混同しない)', () => {
  it('届いた state をそのまま、届かなかった台は「不明」+ 理由つき', () => {
    const components = parseControllerStatus(LIVE)?.components ?? [];
    expect(components).toHaveLength(2);
    expect(components[0]).toEqual({
      name: 'graw-writer',
      endpoint: 'tcp://127.0.0.1:47100',
      reachable: true,
      state: 'Running',
      runNumber: 1,
      error: null,
    });
    expect(components[1]).toEqual({
      name: 'receiver0',
      endpoint: 'tcp://127.0.0.1:47110',
      reachable: false,
      state: '不明',
      runNumber: null,
      error: 'no reply within 1000 ms',
    });
  });
});

describe('ecc(ecc-bridge 無しの開発機で赤い嘘を出さない)', () => {
  it('state "Unknown" は「不明」と出し、理由も添える', () => {
    const ecc = parseControllerStatus(LIVE)?.ecc;
    expect(ecc?.state).toBe('Unknown');
    expect(ecc?.label).toBe('不明');
    expect(ecc?.tone).toBe('unknown');
    expect(ecc?.error).toContain('no reply within 1000 ms');
  });

  it('繋がっていれば ECC の状態をそのまま出す', () => {
    const raw = {
      ...(LIVE as Record<string, unknown>),
      ecc: { ok: true, state: 'Ready', error: null },
    };
    const ecc = parseControllerStatus(raw)?.ecc;
    expect(ecc?.label).toBe('Ready');
    expect(ecc?.tone).toBe('ok');
    expect(ecc?.error).toBeNull();
  });

  it('繋がっているのに ok=false はエラー色(不明とは別物)', () => {
    expect(eccTone(false, 'Idle')).toBe('error');
    expect(eccTone(false, 'Unknown')).toBe('unknown');
    expect(eccTone(true, 'Running')).toBe('ok');
  });

  // 043: controller が `ecc.ecc_error`(GET の error フラグ)を素通しで載せるようになった。
  // **この UI はまだ表示しない**(表示は P4)—— ここで見るのは「知らないキーが増えても
  // 既存の見え方が 1 つも変わらない」ことだけ。パーサが未知キーを無視する作りである証拠。
  it('知らないキー ecc_error が増えても既存の表示は変わらない(表示は P4)', () => {
    const raw = {
      ...(LIVE as Record<string, unknown>),
      ecc: { ok: true, state: 'Idle', error: '', ecc_error: 'WHEN_DESCRIBE' },
    };
    const ecc = parseControllerStatus(raw)?.ecc;
    expect(ecc).toEqual({ state: 'Idle', label: 'Idle', tone: 'ok', error: '' });
  });
});

describe('壊れた応答', () => {
  it('オブジェクトでなければ null(呼び手が「取得できていない」を出す)', () => {
    expect(parseControllerStatus(null)).toBeNull();
    expect(parseControllerStatus('nope')).toBeNull();
    expect(parseControllerStatus([1])).toBeNull();
  });

  it('欄が欠けていても落ちない(未知は「不明」/ 空)', () => {
    const view = parseControllerStatus({});
    expect(view?.phase).toBe('不明');
    expect(view?.components).toEqual([]);
    expect(view?.ecc.label).toBe('不明');
    expect(view?.logPosts).toBeNull();
  });
});
