# 109 — 高レートで root_sink が追いつかず、daq_down の 10 s SIGKILL で ROOT が未確定のまま残る

**Status: READY**(2026-10-09 起票 Opus。107 の periodic トリガ試験で発見。graw は無傷)

## 事実(ELITPC、2026-10-09 11:21–11:25、`elinp` のトリガだけ `periodically` にした試験 — 107)

| run | triggerPeriod | 受理(graw) | イベント間隔 | root_sink |
|---|---|---|---|---|
| 15 | 1000 ms | 30 | 1002.931 ms 一定 | 正常 close |
| 16 | 200 ms | 150 | 202.931 ms 一定 | 正常 close |
| 17 | 50 ms | 577 | 52.931 ms 一定 | `events_complete=577`、raw / PEventTPC とも 577 |
| 18 | 10 ms | **2,368**(77.3 Hz、graw 2 本、eventIdx 連番) | 12.931 ms 一定 | **close されず**: `run_inprogress_*.root` 520 MB / `run_raw_inprogress_*.root` 110 MB のまま |

- zCoBo の周期トリガは「周期 + 2.931 ms」で一定 = **1 イベントの読み出しに 2.93 ms**(上限 ≈ 340 Hz)。線上と graw は全レートで欠損なし。
- run 18 の root_sink ログは `recording run 18 part 0` で止まり、`closed` も最終カウンタの JSON 行も無い = **SIGKILL された**。
  経緯: 77 Hz × 全出力(≈ 1.3 MB/event ≈ 100 MB/s、`--recorder-workers` 既定 1)で root_sink が遅れて内部に滞留 → run/stop は
  `eos_closed:true`(controller は root-sink を直接観測できない — decoder / graw-writer の GetStatus で代理、SPEC §1.3)→ 直後の
  `tools/deploy/daq_down.sh` が root_sink に SIGINT → **10 s 待って SIGKILL**(滞留を書き切る前)。
- 影響: run 18 の PEventTPC / raw ツリーの未書き出し分。graw(正本)は全数あるので raw は graw2root.C、PEventTPC は graw_replay → root_sink で再生成できる。
  **CLAUDE.md 絶対ルール「保存系は落とさない」に対し、運用スクリプト(daq_down の 10 s)が穴になっている**。
- α 線源の本番(≈ 4.5 Hz)はこの限界の遥か下なので、通常運転では起きない。

## 直すこと(案)
1. `daq_down.sh`: root_sink だけは SIGINT 後に **書き切るまで待つ**(進捗を 10 s ごとに表示、上限は長め — 例 600 s、超えたら SIGKILL せず警告して残す)。
2. `daq_up.sh`: 高レート向けに root_sink の `--recorder-workers 4`(064: mini 100 events/s を実証)を env / TOML で選べるように。
   ファイル名が `run{N}_w{k}.root` になる(TPCReco 互換は 064 で確認済み)点を運用マニュアルに書く。
3. run/stop 後に root-sink の close を待てる手段(root-sink の status PUB の `runs` / `pending_events` を controller が見る等)は SPEC 事項 → Fable キュー。
