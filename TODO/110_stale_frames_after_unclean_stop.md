# 110 — 前の取得を止めずにスタックを落とすと、次の run の頭に古いフレームが混ざり、root-sink がほぼ全イベントを late で落とす

**Status: READY**(2026-10-09 起票 Opus。109 の workers-8 試験で発見。graw は無傷。設計判断あり → Fable キュー)

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
