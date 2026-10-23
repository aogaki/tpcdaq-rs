# 175 — status の `events_built`(UI の EVENTS)が run 開始でリセットされない

**Status: COMPLETED**(2026-10-23 起票・同日完了 Opus。ユーザー「あの EVENTS はランを開始してもリセットされません」「できればランごとにリセットしてほしい」。
スポット Fable が確定、SPEC v1.50 は主対話が反映済み、実装は Sonnet implementer)

## Fable 裁定(2026-10-23、要約)
- 変更する(反対理由なし)。`events_built` / `events_incomplete` / `late_fragments` / **`acquisition_restarts`** を **run ごと**(close 行と同じ `now − at_open`)。
  run 外 = 直近に閉じた run の値を保持(閉じた時点の第 2 スナップショットは作らない — run 外はカウンタが動かない)、起動後 run 無し = 0。
- 不変: `frames_per_cobo`(もとから run 開始で memset)、`pending_events` / `sources_behind`(瞬間値)、**`bytes_written`(Recorder の通算のまま、SPEC に明記)**、終了 JSON / close 行。
- 消費側のリスク無し: monitor `StatusPayload` は写すだけ、UI は生値表示、controller の run_stop.counters は root-sink に REP が無く今日も null(将来 REP を足すなら run ごとこそ欲しい値)。ワイヤのキー集合・順序・型は不変。
  アンダーフロー無し(Counts は単調増加、at_open は Opened 時に取得)。歩き戻し(§1.3)も Opened で取り直し。
- SPEC v1.50: §5.3 の `events_built` 定義・`acquisition_restarts` の文言・改訂履歴・冒頭の版数(主対話が反映済み)。§9.2 は不変。

## 3. 発注(implementer = Sonnet)
- 触ってよいファイル: `tools/root_sink/rs_core.hpp`、`tools/root_sink/root_sink.cxx`、`tools/root_sink/monitor_pub.hpp`(コメントのみ)、`tools/root_sink/test_rs_core.cpp`。SPEC・他は触らない。
1. `rs_core.hpp`: 純関数 `inline RunCounts run_delta(const RunCounts& now, const RunCounts& at_open)`(全 7 フィールドの要素ごとの差)。`format_run_closed` をこれで書き直すのは任意(既存テストが守る)。
2. `root_sink.cxx` `update_status_material`: `const RunCounts d = rootsink::run_delta(run_counts(c), c.run_at_open);` から
   `events_built = d.events_complete + d.events_incomplete`、`events_incomplete`、`late_fragments`、`acquisition_restarts` を取る。`pending_events` / `sources_behind` / `frames_per_cobo` は従来どおり `c` から。
   既存コメント「idle のときの run は直近クローズした run」の続きに「カウンタも同じ範囲(TODO/175、SPEC v1.50)」。
3. `monitor_pub.hpp` `Status` のコメント(events_built / events_incomplete / late_fragments / acquisition_restarts)を「run ごと(v1.50)」に。
- 受け入れ試験(`tools/root_sink/test_rs_core.cpp`):
  - **T1 `test_run_delta_is_per_run`**: 既存 `test_run_closed_line_reports_per_run_counts` と同じ数列で、run 12 中 `events_complete=139 / events_incomplete=3 / late_fragments=2`(events_built 相当 142)、
    run 13 中 `8595 / 7 / 5`(8602)、通算 8734 が出ないことを CHECK。
  - **T2 `test_run_delta_before_first_run_is_zero`**: 既定構築の now と at_open で全フィールド 0。
  - **T3 `test_run_delta_restarts_per_run`**: 既存 110 の数列(通算 restarts 1 / at_open 後に +1)で `acquisition_restarts == 1`、`events_before_restart == 19`、`restart_duplicates == 19`。
  - T4(format_run_closed を書き直した場合): 既存の close 行テスト 3 本が不変で green。
  - **test_monitor_pub / test_conformance(run_conformance.sh)は無改修で green**(ワイヤ不変の証拠、件数を報告)。root_sink 本体もビルドが通ること。
  - ビルドは `make -j`(memory: C++ の make は必ず -j)、ROOT が要るものは `export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX27.sdk`。
- 実機確認(主対話、反映後): 次の run 開始直後に UI の EVENTS が ≈ 0 から数え始めること、stop 直前の status `events_built` が close 行の `events_complete + events_incomplete` と一致すること(run 番号と両値を結果節に)。
- 報告に「実行できなかった操作」節(無ければ「なし」)。

## 0. 事実
- UI のステータスバーの EVENTS = monitor の WS status の `events_built`(`ui/src/app/status-bar/status-bar.ts:39`)= root_sink の 1 Hz status PUB(`tools/root_sink/monitor_pub.hpp:54`)。
- 実装は **root_sink 起動からの累計**: 2026-10-23 11:2x に run 190(開始 ≈ 200 s、14 フレーム)で EVENTS 3,934 = 今朝の run(パルサー 187–189 で 3,606 + 物理 181–186・190)の合計。
- SPEC §5.3(docs/SPEC_ja.md 837 行付近)は「`events_built` = emit 済みイベント総数(complete + incomplete)」で、run 内か起動からかを書いていない。
- 一方 root_sink の close 行は run ごと(run 180: `events_complete=5701`)、ログブックの run_stop.counters.events_built は root-sink の REP が無いと null(§9 の v1.10 注記)。→ 画面と記録で意味がずれる。

## 1. 修正案(確定は Fable)
- root_sink が **run 開始で status のカウンタ(events_built / events_incomplete / late_fragments)を 0 に戻す**(run 外は直前の run の値を保持するか 0 にするかも決める)。
- SPEC §5.3 の diff 案: 「`events_built` = **現在の run(run 外では直前の run)で** emit 済みのイベント総数(complete + incomplete)。`events_incomplete` / `late_fragments` も同じ範囲」。
- 影響の確認: monitor / UI は値をそのまま表示するだけ。controller の run_stop.counters は GetStatus 由来(run 終了時点)なので、run 内の値になるのはむしろ望ましい。root_sink の C++ テスト(test_monitor_pub)に run 境界でのリセットを足す。

## 1b. 方針(2026-10-23)
- **ユーザー: 「できればランごとにリセットしてほしいですね」** → run ごとにする方向。
- 実装の手がかり: root_sink は close 行のために run 開始時のスナップショット `c.run_at_open`(`rs_core.hpp` の `RunCounts`)を既に持ち、close 行は(now − at_open)= run ごと。
  status だけが通算(`root_sink.cxx` の `update_status_material`)→ 同じ差を status にも使えばよい。
- SPEC の確定はスポット Fable に依頼中(意味・§5.3 の文案・`acquisition_restarts` も run ごとにするか・受け入れ試験・発注先)。

## 2. それまでの運用
- UI の EVENTS は「root_sink 起動(daq_up)からの累計」と読む。run ごとの数は close 行 / ログブックで。

## 結果(実装まで。2026-10-23、Mac、実装 Sonnet implementer → 主対話 Opus が照合)
- 変更(4 ファイル +83 / −8): `rs_core.hpp` に `run_delta(now, at_open)`(全 7 フィールドの差)、`root_sink.cxx` `update_status_material` で events_built / events_incomplete / late_fragments / acquisition_restarts を差から(pending / sources_behind / frames_per_cobo は従来どおり)、`monitor_pub.hpp` のコメント、`test_rs_core.cpp` に T1–T3。`format_run_closed` は書き直していない(T4 なし)。
- red: T1–T3 を先に書いた段階で `make -j test_rs_core` が `use of undeclared identifier 'run_delta'`(4 errors)。
- green: **test_rs_core 98 passed / 0 failed**(T1 `test_run_delta_is_per_run`: run 12 = 139 / 3 / 2(142)、run 13 = 8595 / 7 / 5(8602)、通算 8734 が出ない。T2 `test_run_delta_before_first_run_is_zero`: 全 0。T3 `test_run_delta_restarts_per_run`: restarts 1 / before 19 / duplicates 19)。
  **test_monitor_pub 92 / 0(無改修)**、`run_conformance.sh`: test_conformance 49 / 0(無改修)、tpc_wire 68、eb_core 391、geo 426、monitor_hist 202、ped_table 27。root_sink 本体 `make -j root_sink` 成功(警告なし)。
- 主対話の照合: `git diff --stat -- tools/root_sink/` が上の 4 ファイルのみ、T1–T3 の関数名がファイルに存在、test_rs_core / test_monitor_pub を再実行して同じ件数、diff を目視(差の取り方・範囲が裁定どおり)。
- 実行できなかった操作: なし。
- **残り**: ELITPC 反映(bundle → `~/tpcdaq-rs` ff → root_sink を `make -j` → daq_down / daq_up、run の合間にユーザー合図)→ 実機確認(run 開始直後に EVENTS ≈ 0、stop 直前の status events_built = close 行の complete + incomplete)→ COMPLETED → archive。

## 結果(実機、2026-10-23 11:50–11:53、ELITPC、ユーザー「止めました。全部やっちゃってください」)
- 反映(RUNBOOK §7.1): コミット `93baac0` を push、bundle `d73740d..93baac0` で `~/tpcdaq-rs` を ff(`git merge --ff-only <bundle>` は不可、`git pull --ff-only <bundle> main` で可)、
  `cargo build --release`(変更なし 0.46 s)、root_sink `make TPCDAQ_TPCRECO_DIR=/opt/tpcdaq/assets/TPCReco-HIGS2026_online -j` + `make … -j test`
  (tpc_wire 68 / rs_core 98 / eb_core 391 / geo 426 / monitor_hist 202 / monitor_pub 92 / ped_table 27、すべて 0 failed)、`tools/deploy/daq_down.sh` → `daq_up.sh`。
  反映後: `/api/status` 200・`/` 200、target/release 7 プロセス + getEccServer + root_sink、ログに WARN / ERROR なし。
- 確認(パルサー 20 s × 2、設定は保存 → `elinp__d9742fda` に復元、status は monitor の WS を Python 標準ライブラリの小さなクライアントで直接読む):
  | | 開始 3 s 後 | パルスの終わり | 閉じた後(idle) | close 行 complete + incomplete |
  |---|---|---|---|---|
  | 起動直後(run なし) | — | — | run 0・events_built 0 | — |
  | run 191 | 57 | 416 | **470** | **470** |
  | run 192 | **61**(前の run の 470 を引き継がない) | 423 | **463** | **463** |
  → **run 開始で 0 から数え、閉じた後は直近の run の値を保持し、close 行と完全一致**。events_incomplete / late_fragments 0。
  monitor の WS status に `acquisition_restarts` は無い(SPEC §5.3: monitor は今は読み飛ばす)。
- 付記: パルサー用テンプレートで configure すると、`asad-pulser-start` を送る前から発生器が動いている(開始 3 s で 57 事象)。174 のパルサー解析には影響なし(パルスの数は解析に使っていない)。
- スキップ: なし。実行できなかった操作: なし(git merge の書式違いは pull で代替)。
