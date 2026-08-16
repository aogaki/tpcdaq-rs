/**
 * SPEC §9.2 — ログブックの 5 レコード型 + 前方互換(未知 type)の整形。
 *
 * フィクスチャは **Rust 側の golden 文字列をそのまま転記**したもの
 * (`src/logbook.rs` の `run_start_matches_the_spec_example_verbatim` 他 5 本。
 * `LogbookLine` が `record` を `#[serde(flatten)]` しているので、ワイヤは
 * `{ts, seq, type, actor, …}` のフラットな 1 オブジェクトになる)。
 * これで「Rust が実際に書く形」と TS の読み手が同じ表を見ていることが担保される。
 */
import { type LogbookEntry, formatInt, logbookKind, parseLogbookEntry } from './logbook-record';

/** src/logbook.rs `run_start_matches_the_spec_example_verbatim`(= SPEC §9.2 の例)。 */
const RUN_START = JSON.parse(
  '{"ts":"2026-08-12T15:04:05.123+03:00","seq":1042,"type":"run_start","actor":"controller","run":57,"config_id":"default","geometry":{"path":"config/geometry_mini_eTPC.dat","sha256":"ab12"},"cobos":[{"id":0,"listen":"0.0.0.0:46005"}],"operator":"aogaki","comment":"gas test","expected_fragments":[[0,0]]}',
) as unknown;

/** src/logbook.rs `run_stop_matches_a_hand_assembled_oracle`(025 の Option<u64> 化後)。 */
const RUN_STOP = JSON.parse(
  '{"ts":"2026-08-12T18:00:00.000+03:00","seq":7,"type":"run_stop","actor":"controller","run":58,"duration_s":12.5,"ok":true,"reason":"normal","counters":{"events_built":null,"events_incomplete":null,"late_fragments":null,"frames":{"0":3852,"1":3849},"overflow_frames":3,"malformed":4},"files":[{"path":"run58/CoBo0_AsAd0_ts_0000.graw","bytes":30108672},{"path":"run58/run58.root","bytes":1234567}]}',
) as unknown;

/** src/logbook.rs `audit_matches_a_hand_assembled_oracle`(ok=false + error あり)。 */
const AUDIT = JSON.parse(
  '{"ts":"2026-08-12T18:00:01.000+03:00","seq":8,"type":"audit","actor":"controller","action":"run/start","params":{"run_number":58},"operator":"aogaki","ok":false,"error":"listen-before-start violated"}',
) as unknown;

/** src/logbook.rs `comment_matches_the_spec_example_verbatim`(= SPEC §9.2 の例)。 */
const COMMENT = JSON.parse(
  '{"ts":"2026-08-12T16:10:00.001+03:00","seq":1043,"type":"comment","actor":"ui","author":"aogaki","text":"beam tuned, 4 Hz"}',
) as unknown;

/** src/logbook.rs `psu_matches_a_hand_assembled_oracle`。 */
const PSU = JSON.parse(
  '{"ts":"2026-08-12T18:00:02.000+03:00","seq":9,"type":"psu","actor":"psu","device":"CAEN0","channel":3,"event":"TRIP","values":{"vmon":350.2,"imon":0.008,"vset":350.0}}',
) as unknown;

function parsed(raw: unknown): LogbookEntry {
  const entry = parseLogbookEntry(raw);
  if (!entry) throw new Error('期待に反して取り込めなかった');
  return entry;
}

/** 詳細ブロックを label で引く(順序に依存しない照合のため)。 */
function detail(entry: LogbookEntry, label: string): readonly string[] {
  const found = entry.details.find((d) => d.label === label);
  if (!found) throw new Error(`detail "${label}" が無い: ${entry.details.map((d) => d.label)}`);
  return found.lines;
}

function field(entry: LogbookEntry, label: string): string {
  const found = entry.fields.find((f) => f.label === label);
  if (!found) throw new Error(`field "${label}" が無い: ${entry.fields.map((f) => f.label)}`);
  return found.value;
}

describe('共通フィールド(ts / seq / type / actor)', () => {
  it('ts はローカル時刻のまま日付と時刻に割る(ブラウザの TZ で読み替えない)', () => {
    const entry = parsed(COMMENT);
    expect(entry.date).toBe('2026-08-12');
    expect(entry.time).toBe('16:10:00.001');
    // 生の ts(オフセット込み)は保持する — どのタイムゾーンで書かれたかを失わない。
    expect(entry.ts).toBe('2026-08-12T16:10:00.001+03:00');
  });

  it('seq と actor をそのまま持つ', () => {
    const entry = parsed(COMMENT);
    expect(entry.seq).toBe(1043);
    expect(entry.actor).toBe('ui');
  });

  it('ts が想定の形でなければ丸ごと時刻欄に出す(捨てない)', () => {
    const entry = parsed({ ts: 'nope', seq: 1, type: 'comment', actor: 'ui', text: 'x' });
    expect(entry.date).toBe('');
    expect(entry.time).toBe('nope');
  });

  it('actor が無い行も落とさず「—」で出す', () => {
    const entry = parsed({ ts: '2026-08-12T00:00:00.000+00:00', seq: 5, type: 'comment' });
    expect(entry.actor).toBe('—');
  });
});

describe('run_start(SPEC §9.2)', () => {
  it('run 番号を見出しにし、operator / config / geometry を並べる', () => {
    const entry = parsed(RUN_START);
    expect(entry.kind).toBe('run_start');
    expect(entry.title).toBe('run 57 started');
    expect(entry.severity).toBe('normal');
    expect(field(entry, 'operator')).toBe('aogaki');
    expect(field(entry, 'config')).toBe('default');
    expect(field(entry, 'geometry')).toBe('config/geometry_mini_eTPC.dat');
  });

  it('run コメントは本文として出す', () => {
    expect(parsed(RUN_START).body).toBe('gas test');
  });

  it('cobos / expected_fragments / sha256 は畳んで出す(件数は見出しに)', () => {
    const entry = parsed(RUN_START);
    expect(detail(entry, 'geometry sha256')).toEqual(['ab12']);
    expect(detail(entry, 'CoBo (1)')).toEqual(['CoBo 0 → 0.0.0.0:46005']);
    expect(detail(entry, 'expected fragments (1)')).toEqual(['cobo 0 / asad 0']);
  });
});

describe('run_start の config_ids(SPEC §9.2 v1.13、TODO/042・051)', () => {
  it('なし(3 相同値。RUN_START フィクスチャそのもの)は従来どおり config 一本', () => {
    const entry = parsed(RUN_START);
    expect(field(entry, 'config')).toBe('default');
    expect(entry.fields.find((f) => f.label.startsWith('config ('))).toBeUndefined();
  });

  // 出典: src/logbook.rs `run_start_carries_config_ids_when_the_phases_differ`
  // (= sample_run_start_per_phase。3 値は非対称: describe/prepare/configure が全部違う)。
  it('値あり(3 相非同値)は config 一本の代わりに 3 相を出す', () => {
    const raw = JSON.parse(
      '{"ts":"2026-08-12T15:04:05.123+03:00","seq":1042,"type":"run_start","actor":"controller","run":57,"config_id":"pulser","config_ids":{"describe":"zCobo-ZC706","prepare":"prep-xcfg","configure":"pulser"},"geometry":{"path":"config/geometry_mini_eTPC.dat","sha256":"ab12"},"cobos":[{"id":0,"listen":"0.0.0.0:46005"}],"operator":"aogaki","comment":"gas test","expected_fragments":[[0,0]]}',
    ) as unknown;
    const entry = parsed(raw);
    expect(entry.fields.find((f) => f.label === 'config')).toBeUndefined();
    expect(field(entry, 'config (describe)')).toBe('zCobo-ZC706');
    expect(field(entry, 'config (prepare)')).toBe('prep-xcfg');
    expect(field(entry, 'config (configure)')).toBe('pulser');
  });
});

describe('run_stop(SPEC §9.2 — ok=false を目立たせる / counters の null は 0 ではない)', () => {
  it('ok=true は通常表示', () => {
    const entry = parsed(RUN_STOP);
    expect(entry.kind).toBe('run_stop');
    expect(entry.title).toBe('run 58 stopped (ok)');
    expect(entry.severity).toBe('normal');
    expect(field(entry, 'reason')).toBe('normal');
    expect(field(entry, 'duration')).toBe('12.5 s');
  });

  it('ok=false は severity error + 見出しで異常と分かる', () => {
    const raw = {
      ...(RUN_STOP as Record<string, unknown>),
      ok: false,
      reason: 'error: eos timeout',
    };
    const entry = parsed(raw);
    expect(entry.title).toBe('run 58 stopped (fault)');
    expect(entry.severity).toBe('error');
    expect(field(entry, 'reason')).toBe('error: eos timeout');
  });

  it('counters の null は「取得不能」と書く(0 と混同しない — SPEC v1.10 §9.2)', () => {
    // 出典: src/logbook.rs の golden。events_built/incomplete/late_fragments が null、
    // overflow_frames=3 / malformed=4、frames は cobo 0=3852 / 1=3849。
    expect(detail(parsed(RUN_STOP), 'counters')).toEqual([
      'events_built = unavailable (null)',
      'events_incomplete = unavailable (null)',
      'late_fragments = unavailable (null)',
      'frames cobo 0 = 3,852',
      'frames cobo 1 = 3,849',
      'overflow_frames = 3',
      'malformed = 4',
    ]);
  });

  it('files は件数つきで畳む', () => {
    expect(detail(parsed(RUN_STOP), 'files (2)')).toEqual([
      'run58/CoBo0_AsAd0_ts_0000.graw — 30,108,672 B',
      'run58/run58.root — 1,234,567 B',
    ]);
  });

  it('旧行(フィールド欠落 = RUN_STOP フィクスチャそのもの)は無印 — 記録なし ≠ false', () => {
    // RUN_STOP は forced_eos/eos_closed を持たない(v1.12 より前の形)。033-A の意味論どおり
    // 「記録が無い」を「観測値 false」に化けさせてはいけない。
    const entry = parsed(RUN_STOP);
    expect(entry.fields.map((f) => f.label)).toEqual(['reason', 'duration']);
    expect(entry.severity).toBe('normal');
  });
});

describe('run_stop の forced_eos / eos_closed(SPEC §9.2 v1.12/v1.14、TODO/033・051)', () => {
  /** src/logbook.rs `run_stop_matches_a_hand_assembled_oracle`(033-A 後の golden。両方 true)。 */
  const BOTH_TRUE = JSON.parse(
    '{"ts":"2026-08-12T18:00:00.000+03:00","seq":7,"type":"run_stop","actor":"controller","run":58,"duration_s":12.5,"ok":true,"reason":"normal","forced_eos":true,"eos_closed":true,"counters":{"events_built":null,"events_incomplete":null,"late_fragments":null,"frames":{"0":3852,"1":3849},"overflow_frames":3,"malformed":4},"files":[{"path":"run58/CoBo0_AsAd0_ts_0000.graw","bytes":30108672},{"path":"run58/run58.root","bytes":1234567}]}',
  ) as unknown;

  it('両方 true(常態)は無印(強調も警告も出さない)', () => {
    const entry = parsed(BOTH_TRUE);
    expect(entry.fields.map((f) => f.label)).toEqual(['reason', 'duration']);
    expect(entry.severity).toBe('normal');
  });

  // 出典: src/logbook.rs `run_stop_keeps_forced_eos_and_eos_closed_independent`
  // (= reference/_spike/demo/out/logbook_D2_saved.jsonl の実測。041 D-2、run 中に vcobo を
  // SIGKILL。OS の正常 FIN で自然 EOF になるので reason は "normal" のまま —— それでも
  // forced_eos:false は「stop 前にリンクが死んだ」ことの唯一の痕跡)。
  it('forced_eos:false(eos_closed は true)は警告として出す。severity は attention', () => {
    const raw = {
      ...(BOTH_TRUE as Record<string, unknown>),
      run: 1,
      duration_s: 27.040875875,
      ok: true,
      reason: 'normal',
      forced_eos: false,
      eos_closed: true,
    };
    const entry = parsed(raw);
    expect(entry.severity).toBe('attention');
    expect(field(entry, 'forced_eos')).toBe('false — the link may have died before stop');
    expect(entry.fields.find((f) => f.label === 'eos_closed')).toBeUndefined();
  });

  it('eos_closed:false は明確な異常として severity を error に引き上げる', () => {
    const raw = {
      ...(BOTH_TRUE as Record<string, unknown>),
      ok: false,
      reason: 'error:eos-timeout',
      forced_eos: true,
      eos_closed: false,
    };
    const entry = parsed(raw);
    expect(entry.severity).toBe('error');
    expect(field(entry, 'eos_closed')).toBe(
      'false — EOS did not travel through the whole chain (fault)',
    );
    expect(entry.fields.find((f) => f.label === 'forced_eos')).toBeUndefined();
  });

  it('eos_closed:false と forced_eos:false が両方揃っても error が勝つ(格下げしない)', () => {
    const raw = {
      ...(BOTH_TRUE as Record<string, unknown>),
      ok: false,
      reason: 'error:eos-timeout',
      forced_eos: false,
      eos_closed: false,
    };
    const entry = parsed(raw);
    expect(entry.severity).toBe('error');
    expect(field(entry, 'eos_closed')).toContain('fault');
    expect(field(entry, 'forced_eos')).toContain('link may have died');
  });
});

describe('audit(SPEC §9.2)', () => {
  it('action を見出しにし、失敗は error として出す', () => {
    const entry = parsed(AUDIT);
    expect(entry.kind).toBe('audit');
    expect(entry.title).toBe('audit — run/start');
    expect(entry.severity).toBe('error');
    expect(field(entry, 'operator')).toBe('aogaki');
    expect(field(entry, 'result')).toBe('failed');
    expect(entry.body).toBe('listen-before-start violated');
  });

  it('成功した監査は通常表示(本文なし)', () => {
    const raw = { ...(AUDIT as Record<string, unknown>), ok: true, error: null };
    const entry = parsed(raw);
    expect(entry.severity).toBe('normal');
    expect(field(entry, 'result')).toBe('ok');
    expect(entry.body).toBeNull();
  });

  it('params は畳んで生の JSON で出す(形は呼び手ごとに違う自由記述)', () => {
    expect(detail(parsed(AUDIT), 'params')).toEqual(['{"run_number":58}']);
  });
});

describe('comment(R11)', () => {
  it('author を見出し、text を本文にする', () => {
    const entry = parsed(COMMENT);
    expect(entry.kind).toBe('comment');
    expect(entry.title).toBe('comment — aogaki');
    expect(entry.body).toBe('beam tuned, 4 Hz');
    expect(entry.severity).toBe('normal');
  });
});

describe('psu(SPEC §9.2、P6 で詳細化)', () => {
  it('device / channel / event を見出しにし、値を並べる', () => {
    const entry = parsed(PSU);
    expect(entry.kind).toBe('psu');
    expect(entry.title).toBe('PSU CAEN0 ch3 — TRIP');
    expect(field(entry, 'vmon')).toBe('350.2');
    expect(field(entry, 'imon')).toBe('0.008');
    expect(field(entry, 'vset')).toBe('350');
  });

  it('TRIP は注意色(HV が落ちた記録を平文に埋もれさせない)', () => {
    expect(parsed(PSU).severity).toBe('attention');
    const on = { ...(PSU as Record<string, unknown>), event: 'ON' };
    expect(parsed(on).severity).toBe('normal');
  });
});

describe('前方互換 — 未知の type(P6 の psu 詳細化に備える)', () => {
  it('捨てずに生 JSON で出し、kind は unknown にする', () => {
    const raw = {
      ts: '2026-08-12T18:00:03.000+03:00',
      seq: 10,
      type: 'psu_v2',
      actor: 'psu',
      device: 'CAEN0',
      ramp: { up: 10, down: 5 },
    };
    const entry = parsed(raw);
    expect(entry.kind).toBe('unknown');
    expect(entry.type).toBe('psu_v2');
    expect(entry.severity).toBe('attention');
    expect(entry.title).toBe('unknown record type: psu_v2');
    expect(detail(entry, 'raw JSON')).toEqual([JSON.stringify(raw)]);
    expect(entry.seq).toBe(10);
  });

  it('type が無い行も unknown 扱いで残す', () => {
    const entry = parsed({ ts: '2026-08-12T18:00:04.000+03:00', seq: 11, actor: 'x' });
    expect(entry.kind).toBe('unknown');
    expect(entry.title).toBe('unknown record type: (no type)');
  });
});

describe('取り込めない行', () => {
  it('オブジェクトでない / seq が数でない行は null(呼び手が数えて表示する)', () => {
    expect(parseLogbookEntry(null)).toBeNull();
    expect(parseLogbookEntry('a line')).toBeNull();
    expect(parseLogbookEntry([1, 2])).toBeNull();
    expect(parseLogbookEntry({ ts: 'x', type: 'comment' })).toBeNull();
    expect(parseLogbookEntry({ seq: 'seven', type: 'comment' })).toBeNull();
  });
});

describe('補助', () => {
  it('logbookKind は 5 型だけを既知にする', () => {
    expect(logbookKind('run_start')).toBe('run_start');
    expect(logbookKind('psu')).toBe('psu');
    expect(logbookKind('psu_v2')).toBe('unknown');
  });

  it('formatInt はロケールに依存せず 3 桁区切り(テストが環境で揺れない)', () => {
    expect(formatInt(0)).toBe('0');
    expect(formatInt(3852)).toBe('3,852');
    expect(formatInt(30108672)).toBe('30,108,672');
  });
});
