# 108 — root_sink の `events_complete` が書き出し件数より多い(高レートで +3.4 %)

**Status: READY**(2026-10-09 起票 Opus。データ損失なし = 表示 / カウンタの不具合。原因調査から)

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
