# 110 — 前の取得を止めずにスタックを落とすと、次の run の頭に古いフレームが混ざり、root-sink がほぼ全イベントを late で落とす

**Status: COMPLETED(2026-10-13、結果は末尾)** — 旧: READY(2026-10-09 起票 Opus。109 の workers-8 試験で発見。graw は無傷。設計判断あり → Fable キュー)

## 事実(ELITPC、2026-10-09 12:05–12:13)

- きっかけ(Opus のミス): workers-8 試験の列 `~/w8_seq.sh` は前回の列から作ったが、その前の手直しで「α run を run/stop する」行が消えていた。
  → **α run 28 が走ったまま `daq_down.sh` でスタックごと停止**(12:12:29)。ECC(getEccServer)が stop を出さずに SIGTERM で落ちたので、
  **zCoBo は外部トリガでの取得を続けたまま**。
- 直後の run 29(`elinp-per10`、periodic 10 ms、workers 8)の graw: **先頭 19 フレームが run 28 の続き**(eventIdx 2038–2056、CoBo タイムスタンプ
  406.36–409.02 s = run 28 開始からの時刻、frameType 1)。frame #19 で **eventIdx 2056 → 0、時刻 409.016 → 0.016 s** と逆戻りし、以降 2,406 フレームが periodic。
- root-sink: `run 29 closed (batches=2402 events_complete=368 events_incomplete=0 late_fragments=2057)` = **新しい run の大半を late として捨てた**
  (graw は 2,425 フレーム全部ある)。receiver の overflow 0、malformed 0。
- run 28: graw は eventIdx 0–2037 まで(2 本)。ROOT は part 0 が `run0028.root` で確定、part 1 と raw は `run_inprogress_*` のまま(EOS が来ていない)。
  logbook に run_stop なし → 12:2x に controller の comment で経緯を記録(`/api/logbook/comment`)。

## 問題
- 運用ミスに限らず、**ELITPC の停電・クラッシュ**(zCoBo は動いたまま)の後の最初の run で同じことが起きる。graw は無傷でも ROOT(PEventTPC / raw)が大量欠損する。
- root-sink の late 判定は「すでに出したイベントより古い eventIdx」を捨てる設計(eb_core)。run の途中で eventIdx が 0 に戻ることを想定していない。

## 直し方の候補(設計判断 → Fable)
1. run/start の前に ECC を確実に stop / breakup / reset してから describe(controller の walk-back を「前回の状態が不明なとき」にも必ず通す)。
   zCoBo に溜まった古いフレームが新しい接続に流れ込むかは要確認(今回は流れ込んだ)。
2. receiver / decoder で「run 開始直後、eventIdx が 0 に戻る前のフレーム」を前の取得の残りとして別扱い(数えて INFO、graw には残す)。
3. root-sink の event builder が eventIdx の大きな逆戻りを「取得のやり直し」として扱う(late にしない)。
- いずれも silent にしない(CLAUDE.md)。まず合成フィクスチャ(古い eventIdx の塊 → 0 から)で red テストを書く。

## Fable 裁定(2026-10-13、スポット Fable — 読み取りのみ。一次資料 = reference/20190315_patched)

### 機序(一次資料で追えたこと、確度 高)
- **リングバッファ**: zCoBo の getHwServer では、FW が DDR のリング(AsAd 毎 32 MiB)にフレームを書く。それを割り込みタスクが TCP で送る。
  - 送れないと `MemRead::flushData` が例外を catch し、readPtr は進まない。
  - FW は空きが `aFullLevel`(16 MiB)を切ると書くのをやめる。これはレジスタのコメントによるもので、FW の実挙動としての確度は中。
- **ECC が死んでも hw node は走り続ける**(`isRunning_=true`)。新しく起動した ECC から次の run を建てると、次の順で進む:
  - `prepare` が FW の書きポインタと readPtr **レジスタ**を memStart に戻す。
  - ただし hw node 側のソフトコピー `readPtr` は古い位置 X のまま。
  - `configure` の末尾の `daqConnect` → `createDataSender` が新しい TCP を即 connect する。
  - 次の `flushData` で「writePtr < readPtr」をラップとみなし、**[X, memEnd) をそのまま送る**。これが run の頭の古いフレーム。
- `start` の `daqStart()` は `isRunning_` で no-op になる(topology frame も出ない)。以後は `runInit` → `resetTime` により、eventIdx 0 / time 0 から新しいフレームになる。
  - 現用の xcfg 5 本は、どれも `useExternalTimeReset=false`。
- **新しく起動した ECC から hw node へ `daqStop` が届く遷移は無い**(dhsm の未定義遷移は無音)。
  - hw node が正常化するのは次の正規の `stop` のとき。112 の「正規 stop → start なら混ざらない」と整合する。
- 推定(確度 中〜低):
  - 古いフレームの本数は「[X, memEnd) に収まった分」で偶然に決まる(今回は 19 本 = 10.6 MB)。
  - さらに前のラップの内容(境界で千切れたフレーム = malformed)が混ざることもあり得る。
  - 予測: run 29 の decoder `topology_frames` は 0。

### 採る案: ③(root-sink の epoch)を主に、②(decoder のカウンタ)を台帳のために併用。graw は残す。① は却下
- **① を却下した理由**: ECC 経由では止められない。getHwServer へ直接 Ice で `daqStop` を打つのは第 2 の制御経路になり、KISS に反する。
- **graw**: 残す。ロスレスであり、実機の DataRouter も同じバイト列を書く(オフライン完全一致の契約)。本数はカウンタで見せる。
- **判定規則(両層で同じ定義)= 取得の再開(acquisition restart)**: 次の 3 条件がそろったとき。
  - `event_idx == 0` のフレームである。
  - 同じ run で、それより大きい eventIdx を見た後に来た。
  - `event_time` が、その run(epoch)で見た最小値より小さい。
  - 067 の「後退幅 1 の隣接混在」は emit 前(pending 中)なので発火しない。
  - decoder は `max_event_idx > 8` でゲートする。
- **root-sink**: restart のときに次の 5 つを行う。
  - `pending` を flush する。
  - `emitted_upto` を消す。
  - epoch を +1 する。
  - `acquisition_restarts` を数える。
  - run 毎に 1 回 stderr に出す。
  
  あわせて、すでに書いた古い epoch のイベントは残す。**recorder の `last_event_id_`(duplicate 判定)も epoch でリセット**する(`BuiltEvent.epoch`)。
- **silent にしない**:
  - root-sink は close 行の `restarts=N events_before_restart=K`、counts JSON、status PUB に出す。
  - decoder は `acquisition_restarts` / `frames_before_restart` を metrics → `/api/status.counters` → **`run_stop.counters`**(台帳に残る唯一の経路)→ UI の logbook の counters 行へ出す。

### 発注(レーン分け — ユーザーの答え(下の問い)を待って着手)
- 共通:
  - worktree 隔離。着手前に `git merge --ff-only main`。
  - 設計分岐に出会ったら実装せず報告する。報告には「実行できなかった操作」の節を必須とする。
  - ゲート: `cargo fmt && cargo clippy --tests -- -D warnings && cargo test`、`cd tools/root_sink && make -j test`(可能なら `make -j test-root`)。
- **レーン A — root-sink(C++、Sonnet)**:
  - `eb_core.hpp`: run 毎に `Epoch{hw, time_floor, epoch}` を持たせる(`emitted_upto_` を置き換え)。late 判定の前に restart 判定を入れる。
    - restart の flush の返し方(`take_restart_flush()` か、feed の戻りを struct にするか)は implementer が選んで報告する。
  - `root_recorder.hpp` の `admit`: epoch が変わったら `have_last_event_id_ = false`。
  - `raw_tree.hpp`: 変更不要であることを確認して報告する。
  - `root_sink.cxx`: restart の flush を `absorb` に通す。あわせて、run 毎 1 回の stderr、`Counts.acquisition_restarts` / `events_before_restart`、`format_run_closed` の追記(`rs_core.hpp` の純関数)、counts JSON、`monitor_pub.hpp` の Status に `acquisition_restarts`(`pending_events` の次のキー)。
  - テスト(`test_eb_core.cpp` ほか):
    - `test_an_event_idx_zero_with_an_earlier_clock_after_emission_starts_a_new_epoch`
    - `test_a_restart_flushes_the_pending_events_of_the_old_epoch`
    - `test_a_late_fragment_of_event_zero_is_still_late_not_a_restart`
    - `test_adjacent_mixing_at_the_head_is_not_a_restart`
    - `test_two_restarts_in_one_run_are_both_counted`
    - `test_run_closed_line_reports_restarts_per_run`
    - (可能なら)`test_recorder_accepts_a_lower_event_id_after_an_epoch_change`
  - 受け入れ: 合成列 (run 29、idx 2038..2056、time 406e6..409e6 相当) → (idx 0..、time 16e3..) で、次がそろうこと。
    - `late_fragments=0`、`restarts=1`、`events_before_restart=19`
    - entries_written = 19 + N
- **レーン B — decoder + controller + logbook 表示(Rust / UI、Sonnet)**:
  - `src/decoder.rs` `RunDecoder`: ソース毎に `max_event_idx` と `time_floor` を持たせる。カウンタは次の 2 つ。
    - `acquisition_restarts`
    - `frames_before_restart`(最初の restart でラッチする)
    
    フレームは落とさず、Error にもしない。`info!` を出し、`rearm` で初期化、`metrics_json` に 2 キーを足す。
  - `src/controller/status.rs` の `collect_status` と `src/logbook.rs` の `Counters` に、nullable の 2 項目を足す(`#[serde(default)]`)。`/api/status.counters` と `run_stop.counters` に載せる。
  - `ui/src/app/logbook/logbook-record.ts` の counters 行に 2 項目を足す(spec 1 本)。
  - テスト:
    - `a_stale_head_followed_by_event_idx_zero_counts_an_acquisition_restart`
    - `adjacent_mixing_at_the_head_of_a_run_is_not_a_restart`
    - `a_restart_is_counted_per_source_and_reported_once_per_run_start`
    - `restart_counters_reset_on_rearm`
    - `run_stop_counters_carry_the_decoder_restart_counters`
  - 受け入れ: 合成列で `acquisition_restarts=1 / frames_before_restart=19` が `run_stop.counters` に載る。
- **レーン C(任意・後回し)**: monitor の StatusPayload と UI の status バーに `restarts N`(N > 0 のときだけ)。
- **実機確認(マージ後、ユーザーの許可を取ってから)**: ELITPC で α run 中に `daq_down.sh`(stop なし)→ `daq_up.sh` → run/start。期待値:
  - graw の先頭に古いフレームがある。
  - close 行に `restarts=1 late_fragments=0` が出る。
  - `run_stop.counters` に同じ値が載る。
  - `topology_frames=0`。
  - 次の正規の stop → start では混入しない。

### SPEC diff 案(v1.46 に同梱 → CURRENT の Fable キュー)
- §1.3: 「ECC 不在で zCoBo が取得を続けた後の最初の run」の 1 項。
  - 機序と、controller では防げないこと。
  - 扱いは §6.3 と §9.2 で行うこと。
  - graw は残すこと。
- §6.3: 「取得の再開(acquisition restart)」の 1 項。
- §5.3: `acquisition_restarts`。
- §7: 前の取得の残りフレームも保全する 1 文。
- §9.2: `run_stop.counters` に 2 項目。
- decoder の節: ゲート `max_event_idx > 8`。

### ユーザーに問う点
- Q1: 再開後の run.root で eventId が重複する(古い K 件と新しい同番号)。両方残すか、新しい同番号を捨てるか。
- Q2: レーン C(status バー)を今やるか。
- Q3: getHwServer へ直接 `daqStop` を打つ経路は今回は不採用でよいか。
- Q4: ELITPC で実機再現をやってよいか。

## ユーザー裁定(2026-10-13)と発注の修正(主対話、Opus)
- **Q1 = 新しい同番号を捨てる**(eventId は run の中で一回きり — TPCReco の eventIdMap、`root_recorder.hpp` の `admit` と同じ意味論)。
  - **注意(主対話がコードで確認)**: recorder の重複判定は「最後に書いた番号以下」の**高水位**(`root_recorder.hpp:918`)。そのままでは新しい取得の 0..2056 が全部落ち、いまの 2,057 件の欠損と変わらない。
  - したがって**捨てるのは「前の epoch で emit した番号と同じ番号」の K 件だけ**にする。
  - また raw ツリーは recorder の `admit` を通らない別経路(`root_sink.cxx` の raw スレッド)。そのため**この判定はイベントビルダ(`eb_core.hpp`)で 1 か所**で行い、PEventTPC・raw ツリー・monitor の中身をそろえる。
- **レーン A の修正(上の発注より優先)**:
  - `eb_core.hpp`: restart のとき、その時点までにその run で emit した event_idx を `retired_ids_`(昇順の `std::vector<uint32_t>`。epoch 内の emit は昇順なので push_back で済む。restart で併合して sort)に移す。
  - 新しい epoch で emit しようとするイベントの idx が `retired_ids_` にあれば、**emit せずに `restart_duplicate_events` を +1**(complete / incomplete とも)。run 毎に初回だけ stderr に出す。
  - `root_recorder.hpp` の `admit`: epoch が変わったら `have_last_event_id_ = false`(上の発注どおり。これが無いと新しい epoch の 0.. が高水位で落ちる)。
  - `raw_tree.hpp`: 変更不要(eb で除いた後のストリームを書く)。
  - close 行・counts JSON に `restart_duplicates=K` を足す。
  - テスト追加: `test_new_epoch_events_with_ids_already_emitted_are_dropped_and_counted`。
  - 受け入れの修正: 合成列(古い 2038..2056 → 新しい 0..N、N > 2056)で次がそろうこと。
    - `late_fragments=0`、`restarts=1`、`events_before_restart=19`、`restart_duplicates=19`
    - PEventTPC と raw ツリーの entries がどちらも `19 + (N + 1) − 19`
    - eventId の重複 0
- Q2 = レーン C は後回し(既定)。Q3 = getHwServer へ直接 `daqStop` は不採用(既定)。Q4 = **マージ後に ELITPC で実機再現してよい**(日時はそのとき確認)。
- SPEC §6.3 の文案に 1 文を足す: 「新しい epoch で、前の epoch で emit した番号と同じイベントは書かずに数える(`restart_duplicates`)」。

## 結果(2026-10-13、レーン A / B マージ済み — 実機再現は未実施)

### レーン B(decoder + controller + logbook UI、Sonnet `cd82b97` → merge `ab82b42`)
- `src/decoder.rs` の `RunDecoder`:
  - ソース毎に `max_event_idx` と `time_floor` を持つ(`HashMap` はソース数ぶんだけ)。
  - 判定は `event_idx == 0 && max > 8 && event_time < floor`。
  - 発火すると次のように扱う:
    - そのソースを新 epoch にする。
    - `acquisition_restarts` を +1 する。
    - `frames_before_restart` は最初の 1 回だけラッチする(run 全体の data frame 数)。
    - `info!` は run 毎に 1 回。
  - フレームは落とさず、Error にもしない。`rearm` で初期化する。
- `src/logbook.rs` の `Counters` に、nullable の 2 項目を足した(`#[serde(default)]`)。
  - `src/controller/status.rs` で、decoder の metrics を合算して `run_stop.counters` に載せる。
  - `/api/status` は `components[].metrics` 経由で見える。`/api/status.counters` という項目は元から無い — 逸脱 1、承認。
- UI: logbook の counters 行に 2 項目を足した(3 桁区切り)。
- テスト:
  - `decoder::tests::a_stale_head_followed_by_event_idx_zero_counts_an_acquisition_restart`(19 本 + idx 0 → restarts 1 / before 19、21 fragments、Error なし)
  - `adjacent_mixing_at_the_head_of_a_run_is_not_a_restart`
  - `a_restart_is_counted_per_source_and_reported_once_per_run_start`
  - `restart_counters_reset_on_rearm`
  - `sequencer::tests_stop::run_stop_counters_carry_the_decoder_restart_counters`
  - UI spec 1 本
- red: decoder の 4 本は assert で落ちた。controller の 1 本はコンパイルエラーだった。

### レーン A(root-sink、Sonnet `a2b7607` + レビュー差し戻し 2 回 `f0594ce` / `5a7c3ed` → merge `2fb7efb`)
- `eb_core.hpp`:
  - run 毎の `RunState` に epoch、高水位、`max_seen`、`time_floor`、emit 済み番号の**区間リスト**(`IdRanges`)を持たせた。
  - **restart の判定** = `event_idx == 0 && (この epoch に complete になったイベントがある || emit 済み) && max_seen > 8 && event_time < time_floor`。
  - restart のときに行うこと:
    - 旧 epoch の pending を `take_restart_flush()` で先に押し出す。
    - 番号を `retired` へ併合する。
    - epoch を +1 する。
  - 新 epoch で `retired` にある番号のイベントは emit せず、`restart_duplicate_events` に数える(ユーザー裁定 Q1)。
  - `close_run` で区間リストを解放する。
- `root_recorder.hpp`: epoch が変わったら eventId の高水位をリセットする。
- `raw_tree.hpp`: 変更なし(eb で除いた後を書く)。
- `rs_core.hpp`: close 行に、常に `restarts=N events_before_restart=K restart_duplicates=D` を出す。
- `root_sink.cxx`: counts JSON に 3 キーを足した。run 毎 1 回の stderr。status PUB に `acquisition_restarts`(12 キー目)。
- **レビューで直したこと**(主対話):
  1. 判定に「complete か emit 済み」を必須にした。Warsaw ELITPC は 2 枚の zCoBo なので、板の時刻のずれで遅れて届く event 0 を誤判定しないため。遅れた板のフラグメントはまだどのイベントにも来ていないので、complete のイベントは無い。
  2. 「emit 済み」だけでは、古い頭と新しい idx 0 が**同じバッチ**に入ったとき見落とす(root_sink はバッチの feed の後で 1 回だけ poll する)。一次資料の機序ではこれが一番ありそうな形なので、complete も含めた。
  3. emit 済み番号を 1 イベント 4 B で run を閉じても保持していた → 常駐で RSS が伸びるので、区間リストにして close で解放するよう直した。
- テスト(`test_eb_core.cpp` ほか):
  - `test_an_event_idx_zero_with_an_earlier_clock_after_emission_starts_a_new_epoch`
  - `test_a_restart_flushes_the_pending_events_of_the_old_epoch`
  - `test_a_late_fragment_of_event_zero_is_still_late_not_a_restart`
  - `test_adjacent_mixing_at_the_head_is_not_a_restart`
  - `test_two_restarts_in_one_run_are_both_counted`
  - `test_new_epoch_events_with_ids_already_emitted_are_dropped_and_counted`
  - `test_synthetic_stale_head_acceptance`
  - `test_a_late_event_zero_fragment_from_another_board_with_an_earlier_clock_is_not_a_restart`
  - `test_emitted_ids_are_kept_as_ranges_and_released_on_close`
  - `test_a_stale_head_and_the_new_event_zero_in_the_same_batch_is_a_restart`
  - `test_a_stale_head_with_four_asads_in_the_same_batch_is_a_restart`
  - `test_rs_core::test_run_closed_line_reports_restarts_per_run`
  - `test_recorder::test_recorder_accepts_a_lower_event_id_after_an_epoch_change`
  - `test_raw_tree::test_raw_tree_writes_the_epoch_filtered_stream_as_is`
- red の実測:
  - recorder の epoch リセットを無効化すると dups 2038 / entries 23 で FAIL。
  - 判定を `have_emitted` だけに戻すと 26 件 FAIL(同じバッチの 2 本、flush、受け入れ)。
  - `have_emitted` を外すと、板のずれのテストが FAIL。
- **受け入れ値(合成列)**: 古い 2038..2056(19 件)→ 新しい 0..2060 で、次のとおり。
  - `late_fragments=0`、`restarts=1`、`events_before_restart=19`、`restart_duplicates=19`
  - PEventTPC と raw ツリーの entries はどちらも 2061、`duplicate_event_ids=0`

### ゲート(main `2fb7efb`、Mac、2026-10-13、ROOT 6.40.04 + SDKROOT=MacOSX27.sdk)
- `make -j test`: tpc_wire 68 / rs_core 79 / eb_core 272 / geo 426 / monitor_hist 202 / monitor_pub 92、全 passed、failed 0。conformance は golden 未指定で SKIP(従来どおり)。
- `make -j test-root`: recorder 174 / pevent 99 / raw_tree 91、全 passed、failed 0(実機 PEventTPC の比較は `TPCDAQ_REAL_PEVENT` 未設定で SKIP — 従来どおり)。
- root_sink 本体のビルド: 警告・エラーなし。
- cargo: `fmt --check` OK / clippy 警告 0 / `cargo test` **1078 passed / 0 failed / 1 ignored**(既存のベンチ)。
- 照合(主対話): 各レーンの `git show --stat` と、発注書のテスト名が diff にあることを確認。両レーンとも「実行できなかった操作」はなし(worktree 保護で複合コマンドを分割したのみ)。

### 実機再現(ELITPC、2026-10-13 12:49–12:53 EEST、ユーザー許可「マージ後にやってよい」。run 71 はユーザーが停止)
- 反映:
  - main `7326d9b` を bundle → ELITPC で ff → `cargo build --release` → root_sink を `make TPCDAQ_TPCRECO_DIR=/opt/tpcdaq/assets/TPCReco-HIGS2026_online -j`。gcc 15 で警告 0、C++ テストは ELITPC 上でも全 passed。
  - UI を差し替えた(旧版 `/opt/tpcdaq/ui.prev110`)。
  - 注意: 最初の daq_down / up は古い `~/deploy_test/` 版を使ってしまった(slowctl を止めも起こしもしない版)。slowctl は前のプロセスのまま動き続けたので実害なし。再現試験はリポ版 `tools/deploy/` で行った。
- 手順(`~/repro110.sh a|b|c`、操作者 `claude`、params `elinp__dcdaf9ca` = 25 MHz / triggerDelay 1500、α 線源・外部トリガ):
  - **A = run 72**: 60 s 走らせ、**stop せずに** `daq_down.sh`(12:50:14)。
  - **B = run 73**: 20 s 後に `daq_up.sh` → run/start → 60 s → 正規 stop。
  - **C = run 74**: 正規の stop → start → 30 s → stop。
- 結果:

| 項目 | run 72(A) | run 73(B) | run 74(C) |
|---|---|---|---|
| graw | 259 frames(idx 0–258、t 0.14–60.2 s)、ROOT は `run_inprogress_*` のまま(EOS が来ない — 110 の原事故と同じ) | **389 frames、先頭 91 本が run 72 の続き**(idx 259–349、t 60.8–79.6 s)、frame #91 で idx 349 → 0、t 79.6 → 0.28 s。後退は 1 回、frameType 1 のみ | 混入なし |
| root_sink close 行 | — | `events_complete=350 events_incomplete=0 late_fragments=0 restarts=1 events_before_restart=91 restart_duplicates=39`(= 91 + 新 298 − 重複 39) | `late_fragments=0 restarts=0 events_before_restart=0 restart_duplicates=0` |
| decoder | — | run 中の metrics: `acquisition_restarts=1 / frames_before_restart=91 / topology_frames=0`(**Fable の予測どおり daqStart は no-op で topology frame なし**)、`malformed=0` | — |
| `run_stop.counters` | (run_stop なし) | **`acquisition_restarts=0 / frames_before_restart=0` ← 不具合** | — |
| run/start の応答 | 132 の notes が実機で出た(`config_id taken from params elinp__dcdaf9ca …`) | 同 | 同 |

- **見つけた不具合**:
  - decoder の `rearm()`(自分の EOS 送出後)が再開カウンタを 0 に戻していた。controller が run_stop の counters を集めるのは EOS 伝播の後なので、0 になる。
  - RunDecoder は Start ごとに作り直されるので、malformed と同じく `rearm` で戻さないのが正しい。
  - レーン B の `restart_counters_reset_on_rearm` が誤った仕様を固定していた → 修正を Sonnet に発注(2026-10-13)。
- 比較: 110 の原事故(run 29)では 2,057 件を late で捨てていた。今回、root-sink の欠損は同番号の 39 件だけで、数えて見える(ユーザー裁定 Q1)。
- ログブック: run 72 に run_stop が無い経緯を comment で記録した(author `claude`)。

### 不具合の修正と再確認(2026-10-13 12:59–13:01 EEST)
- 修正: Sonnet `fc3bfb5` → merge `22e9284`。`src/decoder.rs` の `rearm()` で再開カウンタを戻さない(malformed と同じ run カウンタ)。判定用の状態は従来どおり消す。
  - テストは `restart_counters_reset_on_rearm`(誤った仕様)を `decoder::tests::restart_counters_survive_the_own_eos_until_the_next_start` に置き換えた。
  - 赤: 修正前は `left: Some(0), right: Some(1)`。
- ゲート(main `22e9284`、Mac): fmt OK / clippy 警告 0 / cargo **1078 passed / 0 failed / 1 ignored**。
- ELITPC に反映(bundle → ff → `cargo build --release`)し、A / B を各 30 s で再実行した(`SEC=30 ~/repro110.sh a` / `b`):
  - **run 75**: stop せずに落とした(12:59:16)。ROOT は `run_inprogress_*`、graw は無傷。
  - **run 76**: root_sink `events_complete=136 late_fragments=0 restarts=1 events_before_restart=17 restart_duplicates=0`(新しい run が古い頭の番号まで届かなかったので重複 0)。
    **logbook `run_stop.counters` = `acquisition_restarts=1 / frames_before_restart=17`**(root-sink と一致)。reason normal。
- ログブック: run 75 / 72 の経緯を comment で記録した。
- ELITPC は Idle で終えた(新しい α run は始めていない — ユーザーが UI から)。

### 反映ミス(2026-10-13 午後、ユーザー「web UI にアクセスできなくなった」で発覚)
- 12:46 の UI 差し替えで、`ui/dist/ui` ごと `/opt/tpcdaq/ui` に置いてしまった。正しくは `ui/dist/ui/browser` の**中身**(daq.toml の `ui_dir` の注記どおり、127 まではそうしていた)。
  - その結果 `index.html` が `/opt/tpcdaq/ui/browser/` の下になり、`GET /` が 404。API と WS は無事。
- 直し方: `mv /opt/tpcdaq/ui /opt/tpcdaq/ui.bad110 && cp -a /opt/tpcdaq/ui.bad110/browser /opt/tpcdaq/ui`。静的ファイルなので再起動は不要。
- 確認: `GET /` 200(9,407 B)、`/slow`(深いリンク)200、`main-*.js` 200。
- 教訓: 反映の後は `curl -o /dev/null -w "%{http_code}" http://127.0.0.1:8080/` で UI の 200 まで確かめる(今回は API だけ見ていた)。

### 受け入れ
- 合成列(両レーンのテスト)と実機(run 73 / 76)の両方で、次を確認した:
  - 前の取得の残りフレームは graw にそのまま残る。
  - root-sink は late で捨てず、epoch として扱う。
  - 新しい同番号だけを数えて捨てる(Q1)。
  - 本数は close 行・counts JSON・status PUB・`run_stop.counters` で見える。
- 正規の stop → start では混入しない(run 74)。
- 残り: SPEC の文案(§1.3 / §5.3 / §6.3 / §7 / §9.2、decoder の節)は CURRENT の Fable キューで v1.46 として確定する。レーン C(status バーの `restarts N`)は後回し(ユーザー既定)。

**Status: COMPLETED(2026-10-13)**
