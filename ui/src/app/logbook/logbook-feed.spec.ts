/**
 * SPEC §8.1 `GET /api/logbook?since_seq=N` → `{records: [...], tail_corrupt: bool}` の
 * 追従ロジック(純粋関数)。
 *
 * 守るべきこと:
 * - 取得済みの最大 `seq` を次の `since_seq` に使う。**重複しない・巻き戻らない**。
 * - `tail_corrupt` は必ず見える(JSONL の書き込み中断の痕跡)。
 * - **失敗を silent にしない**(直近エラーを状態に残す)。落とした行も数える。
 */
import {
  EMPTY_LOGBOOK_FEED,
  LOGBOOK_POLL_MS,
  applyLogbookError,
  applyLogbookResponse,
} from './logbook-feed';

/** `{ts, seq, type, actor, author, text}` のフラットな 1 行(§9.2)。 */
function comment(seq: number, text: string): Record<string, unknown> {
  return {
    ts: `2026-08-12T16:10:0${seq}.000+03:00`,
    seq,
    type: 'comment',
    actor: 'ui',
    author: 'aogaki',
    text,
  };
}

function response(records: unknown[], tailCorrupt = false): unknown {
  return { records, tail_corrupt: tailCorrupt };
}

function seqs(state: { entries: readonly { seq: number }[] }): number[] {
  return state.entries.map((e) => e.seq);
}

describe('ポーリング間隔', () => {
  it('既定 5 s(発注書)', () => {
    expect(LOGBOOK_POLL_MS).toBe(5000);
  });
});

describe('since_seq 追従', () => {
  it('最初の応答を取り込み、次の since_seq は最大 seq になる', () => {
    const state = applyLogbookResponse(
      EMPTY_LOGBOOK_FEED,
      response([comment(1, 'a'), comment(2, 'b')]),
    );
    expect(seqs(state)).toEqual([1, 2]);
    expect(state.sinceSeq).toBe(2);
    expect(state.lastError).toBeNull();
  });

  it('続きだけが来たら末尾に足す', () => {
    let state = applyLogbookResponse(
      EMPTY_LOGBOOK_FEED,
      response([comment(1, 'a'), comment(2, 'b')]),
    );
    state = applyLogbookResponse(state, response([comment(3, 'c')]));
    expect(seqs(state)).toEqual([1, 2, 3]);
    expect(state.sinceSeq).toBe(3);
  });

  it('同じ seq が再送されても重複しない', () => {
    let state = applyLogbookResponse(
      EMPTY_LOGBOOK_FEED,
      response([comment(1, 'a'), comment(2, 'b')]),
    );
    state = applyLogbookResponse(state, response([comment(2, 'b'), comment(3, 'c')]));
    expect(seqs(state)).toEqual([1, 2, 3]);
    expect(state.sinceSeq).toBe(3);
  });

  it('古い seq だけが返っても巻き戻らない', () => {
    let state = applyLogbookResponse(
      EMPTY_LOGBOOK_FEED,
      response([comment(1, 'a'), comment(3, 'c')]),
    );
    expect(state.sinceSeq).toBe(3);
    state = applyLogbookResponse(state, response([comment(1, 'a')]));
    expect(seqs(state)).toEqual([1, 3]);
    expect(state.sinceSeq).toBe(3);
  });

  it('順不同で来ても seq 昇順に並べる', () => {
    const state = applyLogbookResponse(
      EMPTY_LOGBOOK_FEED,
      response([comment(3, 'c'), comment(1, 'a'), comment(2, 'b')]),
    );
    expect(seqs(state)).toEqual([1, 2, 3]);
  });

  it('空の応答は何も変えない(エラーでもない)', () => {
    let state = applyLogbookResponse(EMPTY_LOGBOOK_FEED, response([comment(1, 'a')]));
    state = applyLogbookResponse(state, response([]));
    expect(seqs(state)).toEqual([1]);
    expect(state.sinceSeq).toBe(1);
    expect(state.lastError).toBeNull();
  });
});

describe('tail_corrupt(SPEC §9.1 — 末尾行の破損は必ず見せる)', () => {
  it('true で立ち、false の応答で降りる', () => {
    let state = applyLogbookResponse(EMPTY_LOGBOOK_FEED, response([comment(1, 'a')], true));
    expect(state.tailCorrupt).toBe(true);
    state = applyLogbookResponse(state, response([comment(2, 'b')], false));
    expect(state.tailCorrupt).toBe(false);
  });
});

describe('落としたものを数える(silent failure 禁止)', () => {
  it('未知 type は取り込んだうえで数える', () => {
    const state = applyLogbookResponse(
      EMPTY_LOGBOOK_FEED,
      response([comment(1, 'a'), { ts: 'x', seq: 2, type: 'psu_v2', actor: 'psu' }]),
    );
    expect(seqs(state)).toEqual([1, 2]);
    expect(state.unknownTypes).toBe(1);
  });

  it('取り込めない行は数え、累積する', () => {
    let state = applyLogbookResponse(EMPTY_LOGBOOK_FEED, response([comment(1, 'a'), 'garbage']));
    expect(seqs(state)).toEqual([1]);
    expect(state.malformed).toBe(1);
    state = applyLogbookResponse(state, response([42]));
    expect(state.malformed).toBe(2);
  });
});

describe('取得失敗(赤い嘘を出さない)', () => {
  it('例外は直近エラーに残り、取り込み済みの記録は消えない', () => {
    let state = applyLogbookResponse(EMPTY_LOGBOOK_FEED, response([comment(1, 'a')]));
    state = applyLogbookError(state, new Error('Failed to fetch'));
    expect(state.lastError).toBe('Failed to fetch');
    expect(seqs(state)).toEqual([1]);
    expect(state.sinceSeq).toBe(1);
  });

  it('応答の形が違うのも失敗として出す(黙って空にしない)', () => {
    let state = applyLogbookResponse(EMPTY_LOGBOOK_FEED, response([comment(1, 'a')]));
    state = applyLogbookResponse(state, { nope: true });
    expect(state.lastError).toContain('records');
    expect(seqs(state)).toEqual([1]);
  });

  it('次の成功でエラー表示は消える', () => {
    let state = applyLogbookError(EMPTY_LOGBOOK_FEED, new Error('boom'));
    expect(state.lastError).toBe('boom');
    state = applyLogbookResponse(state, response([comment(1, 'a')]));
    expect(state.lastError).toBeNull();
  });
});
