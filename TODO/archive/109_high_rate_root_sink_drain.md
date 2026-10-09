# 109 — 高レートで root_sink が追いつかず、daq_down の 10 s SIGKILL で ROOT が未確定のまま残る

**Status: COMPLETED**(2026-10-09 起票・実装 Opus。107 の periodic トリガ試験で発見。graw は無傷。ユーザー指示「1, 2 を実装」「並列にした後の最大レート試験は重要」)

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

## 結果(2026-10-09、ELITPC = Ubuntu 26.04.1 / ROOT 6.40.04 / ZFS `data/tpcdaq`)

### 実装(`77a7b2b`、`d4ae58c`)
- `tools/deploy/daq_down.sh`: root_sink だけは**書き切るまで待つ**(10 s ごとに「still writing … N s」、上限 `ROOT_SINK_DRAIN_S` 既定 600 s、
  超えたら WARNING を出して**残す** = pid ファイルを残し exit 1、末尾の leftover 掃除からも除外)。**root_sink には SIGTERM**(下記の罠)。
- `tools/deploy/daq_up.sh`: `ROOT_SINK_RECORDER_WORKERS`(daq.env、既定 1、正の整数でなければ起動前に FAIL)> 1 で `--recorder-workers N`。
  起動時に `root_sink workers: N` を表示。`daq.env.example` に 2 変数を追記。

### 偽バイナリでの試験(ELITPC、本番スタックに触れない一時ツリー、`fake.py` = 起動行を出して待つ test double)
- T1(既定): `--recorder-workers` なし ✅ / SIGINT を無視する controller は 10 s で SIGKILL ✅ / root_sink は 25 s の書き切りを待ち `root_sink stopped (25 s)`、
  SIGKILL なし(fake のログに `drained`)✅ / exit 0、pid ファイル残らず ✅(合計 36–37 s)。
- T2(workers=4、上限 5 s、root_sink が止まらない): `--recorder-workers 4` ✅ / WARNING・exit 1・プロセス生存・pid 残し ✅ / 続く daq_up は `already running` で FAIL ✅。
- T3(`ROOT_SINK_RECORDER_WORKERS=x`): 起動前に FAIL ✅。SIGTERM 化の後に T1–T3 を再実行して同じ結果 ✅。

### 実機で見つかった罠: root_sink は SIGINT を受け取れなくなる
- 最初の試験列で、α run 19 停止後の daq_down が root_sink の終了を 9 分待ち続けた(run 19 自体は 3,068 件で正常に確定済み)。
  `/proc/<pid>/status`: **SigIgn = 0x6(SIGINT・SIGQUIT 無視)、SigCgt に SIGTERM はあるが SIGINT は無い**。
  スクリプトから `&` で起動したプロセスは SIGINT / SIGQUIT = SIG_IGN を継ぐ(POSIX、ジョブ制御なし)。root_sink は起動時に `std::signal(SIGINT, …)` を
  入れるが、run で ROOT が初期化された後は SIG_IGN に戻っている(run 前の root_sink は SIGINT で止まっていたことと整合)。
  → **SIGTERM を送ると 0.4 s で正常終了**(最終 JSON: events_complete 3,068 = entries_written 3,068)。daq_down を SIGTERM に変更。
  旧 daq_down(SIGINT + 10 s 後 SIGKILL)は、run 後の root_sink を毎回 SIGKILL していたことになる(run 18 以外は run を閉じた後なので無害)。
  root_sink 本体の対策(ROOT 初期化後に SIGINT を入れ直す)は未実施 — 後続。

### 最大レート試験(`elinp` のトリガだけ `periodically`、30 s ずつ、`~/maxrate.sh`、run 20–25、11:47–11:57)

| run | 周期 | workers | zCoBo 受理(graw) | graw 間隔 | 停止後の書き切り | raw / PEventTPC 件数 | 受信 overflow |
|---|---|---|---|---|---|---|---|
| 20 | 10 ms | **1** | 77.3 Hz、2,400 | 12.931 ms 一定 | **102 s** | 2,400 / 2,400 | 0 |
| 21 | 10 ms | 4 | 77.3 Hz、2,470 | 12.931 ms 一定 | 6 s | 2,470 / 2,470(w0–w3) | 0 |
| 22 | 5 ms | 4 | 126.1 Hz、3,964 | 7.931 ms 一定 | 27 s | 3,964 / 3,964 | 0 |
| 23 | 3 ms | 4 | 168.6 Hz、5,350 | 5.931–6.158 ms | 51 s | 5,350 / 5,350 | 0 |
| 24 | 2 ms | 4 | 181.7 Hz、5,628 | 4.931–153.9 ms | 59 s | 5,628 / 5,628 | 0 |
| 25 | 1 ms | 4 | **182.5 Hz**、5,656 | 3.931–286.1 ms | 60 s | 5,656 / 5,656 | 0 |

- **全レートでロスなし**: graw は全 run 連番・欠番 0、raw ツリーと PEventTPC の件数 = graw、`events_complete` = 書き出し件数(108 の過大は再現せず)、
  receiver の overflow / abandoned 0。
- **トリガ上限 ≈ 182 Hz = 1 GbE で頭打ち**(557,312 B × 182.5 Hz = 101.7 MB/s ≈ 814 Mbit/s)。周期 2 ms 以下では TCP の背圧で zCoBo が止まり、
  間隔が不規則(最大 286 ms)になる。zCoBo の読み出し自体は 2.93 ms(≈ 340 Hz 相当)。
- **root_sink の持続処理量(全出力: PEventTPC + raw + monitor)**: workers 1 ≈ **18 events/s**(2,400 / 132 s)、workers 4 ≈ **63–70 events/s**
  (例 3,964 / 57 s、CPU ≈ 300 %)。それを超える分はメモリに溜めて(RSS 最大 3.1 GB / 30 s @182 Hz)、停止後に書き切る。
- α 線源の本番(≈ 4.5 Hz)は workers 1 で十分 → 試験後 `ROOT_SINK_RECORDER_WORKERS=1` に戻して α run 26 を再開(11:57:24)。

### 残り(後続)
- **持続的な高レート**(> 70 events/s を数分以上)で root_sink の滞留がどこで頭打ち・背圧になるか(`--queue` 1000 を超えて RSS が伸びた — recorder 側のキュー?)、
  そのとき受信と graw が無傷かは未試験。
- root_sink 本体: ROOT 初期化後に SIGINT ハンドラを入れ直す。
- run/stop が root-sink の close を待てるようにする(controller は root-sink を直接観測できない、SPEC §1.3)→ Fable キュー。

## 追記: ZFS は効いているか(2026-10-09 12:00–12:06、ユーザー質問「ZFS アレイの遅さも関係?」)
- 素の書き込み速度(乱数 4 GiB を tmpfs から `dd bs=4M conv=fdatasync`): **ZFS 328 / 226 MB/s**、**NVMe(ext4)809 MB/s**。
  182 Hz 時に要るのは graw ≈ 100 MB/s + ROOT ≈ 40 MB/s ≈ 140 MB/s → ZFS で足りるが余裕は 2 倍程度。
- A/B(periodic 5 ms = 126 Hz、workers 4、30 s、α run 26 を 12:02:56 停止 → run 27 = ZFS、`/srv/tpcdaq/speedtest` の run 1 = NVMe → α run 28 を 12:05:43 再開):

  | 出力先 | イベント | 停止後の書き切り | 1 イベントあたり(run + 書き切り)| I/O 待ち(PSI io some avg10) |
  |---|---|---|---|---|
  | ZFS | 4,257 | 32 s | 14.6 ms(≈ 69 events/s) | 0.9–10 % |
  | NVMe | 3,997 | 25 s | 13.8 ms(≈ 73 events/s) | 0.3–0.9 % |

  両方ともロスなし(NVMe: graw 3,997 連番 = raw 3,997 = w0–w3 計 3,997)。**root_sink の 4 本の recorder スレッドは両方で 99.9 % に張り付き**
  (`top -H`)= **律速は CPU(PEventTPC の詰め替え + 圧縮)**。ZFS の寄与は ≈ 5–6 %。もっと速くするなら workers を増やす(ELITPC は 20 スレッド)。
- 試験用の `/srv/tpcdaq/speedtest`(4.6 GB)は確認後に削除。
