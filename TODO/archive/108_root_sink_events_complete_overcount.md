# 108 — root_sink の `events_complete` が書き出し件数より多い(高レートで +3.4 %)

**Status: COMPLETED**(2026-10-09 起票 Opus。2026-10-11 原因判明(Fable)→ Sonnet が修正 → 同日完了)

## 事実(ELITPC、run 13 = α 線源 ≈ 4.8 Hz、2026-10-09 09:51–10:20、107)

- root_sink ログ: `run 13 closed (batches=8534 events_complete=8888 events_incomplete=0 late_fragments=0)`。
- 実際に書かれた件数: PEventTPC 5 分割 1859 + 1855 + 1865 + 1865 + 1151 = **8,595**、`run0013_raw.root` **8,595**(eventId 0–8594)。
- 線上 = graw 5 本の全フレーム走査: **8,595 フレーム**、eventIdx 0–8594 連番、欠番 0・重複 0、frameType 1 のみ。
- → **データは全数そろっている**。`events_complete` だけが **+293(+3.4 %)** 多い。
- 低レートでは一致: run 2(0.035 Hz、1,697 件)は `events_complete=1697` = 書き出し 1,697。run 8(5 Hz パルサー、154 件)は close 行未確認。
- batches 8,534 < フレーム 8,595(61 バッチが 2 フレーム以上を運んだ)。差 293 とは一致しない。

## 調べること
- `events_complete` を増やす箇所(eb_core / rs_core)を全部洗い、1 イベントを 2 回数え得る経路(1 バッチ複数フラグメント、part のロールオーバー、raw / PEventTPC の 2 系統、モニタの event publish)を特定。
- 既存の合成フィクスチャで「1 バッチに複数フレーム」+「~5 Hz」を再現する red テストを先に書く(TDD)。CLAUDE.md「silent failure を作らない」— カウンタは書き出し件数と一致すべき。

## 影響
UI / status / logbook の `events_complete` を信じて「全部書けた」と判断すると数が合わない。データ(graw・ROOT)は無傷。

## 原因(2026-10-11、112 の実機 10 周で判明 — Fable)
`run N closed (batches=… events_complete=…)` の **`events_complete` はプロセス通算**(run 41 = 49、run 42 = 98、run 43 = 137 = 98 + 39、… run 50 = 408)。`batches` も通算。
run 13 の 8,888 = 8,595(run 13)+ 293(同じプロセスで先に取った run 12 の 139 + run 8 の 154)。書き出し件数(`finalized … entries`)は per-run で正しい。
→ 直し方: close 行は**その run の件数**を出す(run 開始時の通算値との差、または per-run カウンタ)。通算は最終 JSON 行にだけ残す。`late_fragments` も同様に per-run にするか確認。
テスト: `tools/root_sink` の既存テスト(recorder / eb_core)に「2 run 連続で close 行の件数が各 run の件数」を 1 本。

## 結果(2026-10-11)
- 修正(Sonnet `893c97b` → merge): `tools/root_sink/rs_core.hpp` に純関数 `format_run_closed(run, now, at_open)` と `RunCounts`、`root_sink.cxx` は run open 時の通算を `run_at_open` に保存し close 行は差分を出す。
  最終 JSON 行(通算)は不変。`batches` / `events_incomplete` / `late_fragments` も per-run。
- テスト: `test_rs_core` に `test_run_closed_line_reports_per_run_counts`(run 12 = 139 → run 13 = 8,595 の実例、旧実装なら 8,734 が出る)。
  `make -j test`(Mac、Fable 再実行): tpc_wire 68 / rs_core **74** / eb_core 175 / geo 426 / monitor_hist 202 / monitor_pub 92、failed 0。Sonnet 側で `make test-root` 170 / 99 / 88 も green。
- ELITPC への反映は次のスタック入れ替え時(今の α run 51 は止めない)。
- 未対応(別件、軽微): `late_fragments == 0` の初回ログ判定は通算のまま。
