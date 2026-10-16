# 133 — 取得の再開の判定: 4 AsAd で古い頭が全部 incomplete のときの見落とし / 多 CoBo での `frames_before_restart`

**Status: COMPLETED**(2026-10-13 起票 Opus。SPEC v1.46 確定時に Fable が挙げた実装側の疑義 2 点。**今の mini(1 AsAd、1 フレーム = 1 イベントで即 complete)では起きない** — Warsaw ELITPC(4 AsAd、2 枚の zCoBo)と多 CoBo に効く。設計判断あり → Fable。**2026-10-14 Fable レビューで「端」から「Warsaw 前の必須」に格上げ — §1b**)

## 1. 4 AsAd で再開を見落とす端(root-sink、`tools/root_sink/eb_core.hpp`)
- いまの判定(SPEC v1.46 §6.3)は `event_idx == 0 && (この epoch に complete のイベントがある || emit 済み) && max_seen > 8 && event_time < time_floor`。
- 4 AsAd で、古い頭がどのイベントも incomplete のまま、新しい idx 0 が同じバッチで来ると次のように崩れる。
  - complete も emit 済みも無いので、再開と判定しない。
  - 新しい idx 0 が pending に入り、`time_floor` を下げる。
  - 以後の新 epoch の idx 0 も発火しない。
  - 古い incomplete がタイムアウトで emit されると `emitted_upto` が古い値になり、新しい取得が late で落ちる(原事故の形)。
- どういうときに起きるか:
  - getHwServer の割り込みタスクが AsAd ごとのリングを順に流す作りだと、起こり得る。たとえば AsAd0 の古い頭 → AsAd0 の新しい idx 0 → AsAd1 の古い頭 … の順。
  - Fable 裁定の「境界で千切れたフレーム(malformed)」と重なると、さらに起こりやすい。
- 調べること: 一次資料(`reference/20190315_patched` の `MemRead::flushData` / intrTask)で、複数 AsAd のリングを流す順序を確かめる。
- 直し方の候補(Fable):
  - ① ソース(AsAd)ごとに「その AsAd で idx > 8 を見た後に idx 0 が小さい time で来た」を数え、過半(または全 AsAd)で再開とみなす。
  - ② 古い epoch の incomplete を、新 epoch の idx 0 が来た時点で「別 epoch」として隔離する。
- まず合成テスト(4 AsAd、AsAd 順に流れる古い頭 + 新しい idx 0 が同じバッチ)で red を確かめる。

## 1b. 再開を検出できた場合でも 4 AsAd では原事故が再発しうる(2026-10-14 追記、Fable)
- 送出が AsAd 毎のリング順(AsAd0 の古い頭 → AsAd0 の新 idx 0 → AsAd1 の古い頭 → …)だと、再開検出(epoch +1、高水位リセット)の**後**に AsAd1 の古い idx 2057 などが来る。
- それは新 epoch の pending に入り(`retired` は emit 済み番号だけなので弾かれない)、build timeout で incomplete のまま emit される → `emitted_upto = 2057` → **新しい 1..2056 が全部 late** = 原事故そのもの。
- つまり 110 の実装は 1 AsAd(mini)では完結しているが、4 AsAd では送出順次第で効かない。§1 の「見落とし」と合わせ、**idx ではなく時刻で世代を分ける**(旧 epoch の時刻帯 [floor_old, max_old] に入るフラグメントは旧 epoch に振る、134 の閾値と同じ尺度)のが筋。
- まず一次資料(`MemRead::flushData` / intrTask)で AsAd 毎のリングの送出順を確定し、合成テスト(4 AsAd、AsAd 順の古い頭)で red を作ってから設計。

## 2. decoder の `frames_before_restart` は全ソース合算(`src/decoder.rs`)
- 単一ソースでは root-sink の `events_before_restart` と一致する(実機 run 76: 17 = 17)。
- 多 CoBo(複数 receiver)では、「最初に再開したソースまでに全ソースから来たフレーム数」になり、単位も意味も root-sink とずれる。
- 候補: ソースごとの値にする(`run_stop.counters` に配列 or 最大値)。台帳の読み方が紛れないかで決める(Fable)。

## 参照
SPEC v1.46 §1.3 / §2.3 / §6.3 / §9.2、archive/110(実機 run 72–76)。

## 3. 一次資料の確認(2026-10-15 Fable)
`reference/20190315_patched/GetBench/src/get/daq/MemRead.cpp`: `processInterrupt()` → `sendData()` → `flushData()`(L238–263)。**割り込みごとに `circularBuffers_`(map、AsAd 0..3 の順)を回り、有効な AsAd のリングを `dataSenderPtr->flushData(buffer)` で readPtr→writePtr まで全部送ってから次の AsAd へ**(`TcpDataSender::flushData` L131–160 は `nextChunkSize_B()` が 0 になるまで送る)。送出順は **AsAd-major**。再開の瞬間(ECC を止めずにスタックを落とし、新しい ECC の configure で各リングの [X, memEnd) が残っている)は、**AsAd0 の古い残り → AsAd0 の新 idx 0.. → AsAd1 の古い残り → AsAd1 の新 idx 0.. → …** の順で届く。§1 / §1b の懸念は実機の仕様どおり起きる。

## 4. 設計(Fable 2026-10-15)— 世代はソース(AsAd)ごとに持ち、イベントの鍵を (epoch, idx) にする
現行(SPEC v1.47 §6.3)は run に 1 つの世代と 1 つの time_floor を持ち、再開検出で旧世代の pending を即 flush する。4 AsAd では、検出後に来る他 AsAd の古いフラグメントが新世代の pending に入り原事故が再発する(§1b)。直し:
1. **ソースごとの世代**: `SourceState{epoch, max_seen, time_floor, have_time}` を (run, source) で持つ。再開判定は**そのソースの**状態で行う(`idx == 0 && max_seen > 8 && have_time && event_time + 100 ms < time_floor`)。満たしたらそのソースの epoch += 1、max_seen / time_floor をリセット。
   - 「complete か emit 済みがある」条件は不要になる(§1 の見落としを塞ぐ。ソースごとの idx の跳びと時刻後退で十分。134 の時刻閾値はそのまま)。
2. **イベントの鍵 = (run, source-epoch, idx)**: フラグメントは、そのソースの現在の epoch の pending に入る。古い AsAd1 の idx 2057 は AsAd1 がまだ epoch 0 なので epoch 0 の (2057) に入り、新しい AsAd0 の idx 0 は epoch 1 の (0) に入る → 混ざらない。
3. **run の世代 = 全ソースの epoch の最小値(= 一番遅れているソース)**。emit の順序(late 判定の高水位 `emitted_upto`)は**世代ごと**に持つ。旧世代の pending は「全ソースが次の世代に進んだとき」または build timeout で emit(incomplete なら incomplete のまま、従来どおり)。新世代の emit は旧世代の pending が空になるまで待たない(順序は世代番号 → idx で単調。recorder は epoch が変わったら高水位をリセット = 現行どおり)。
4. **再開の確定と計数**: run の `acquisition_restarts` は「全ソースの epoch の最小値が上がったとき」に +1(= 4 AsAd 全部が再開して初めて 1 回と数える)。ソースごとの再開は `source_restarts[source]` に。`events_before_restart` は旧世代で emit した数(従来どおり)。`restart_duplicates` は旧世代で emit 済みの idx が新世代で来たとき(`retired_ids` は世代ごと)。
5. **死んだソース**(再開しないまま来なくなった AsAd): run の世代が上がらない → 新世代のイベントは**その世代の**順序で emit されるので実害なし。旧世代の pending は timeout で emit。status PUB に `sources_behind`(世代が最小のソース数)を出して UI で見える化。
6. **decoder(`src/decoder.rs`)**: `frames_before_restart` を**ソースごと**に(`HashMap<source_id, u64>` → `run_stop.counters.frames_before_restart` を `{source: n}` の辞書に。1 ソースなら現行と同じ数)。`acquisition_restarts` は全ソースの最小 epoch が上がった回数。
7. **SPEC**: §2.3 / §6.3 / §9.2 を v1.48 として改訂(実装の緑を見てから Fable が確定)。

### テスト(合成、root_sink `tools/root_sink/test_eb_core.cpp` 系 + decoder tests)
- 4 AsAd、AsAd-major の再開バッチ: [A0: idx 2050..2057 (t 古い), A0: idx 0..5 (t 新しい)][A1: 2050..2057 古い, 0..5 新しい]…[A3 …] を**同じ feed バッチ**で入れる → 旧世代の 2050..2057 が 4 AsAd で complete として emit(incomplete にならない)、新世代の 0..5 も complete、late 0、`acquisition_restarts` = 1、`source_restarts` = [1,1,1,1]、新世代の高水位は 5(2057 ではない)。
- 旧世代が全部 incomplete(§1 の形: A0 の古い頭が 1 フラグメントだけ)でも再開を検出し、新世代が late にならない。
- 1 AsAd(mini)では現行テストが全部そのまま通る(archive/110 / 134 の合成 + 実機 run 72–76 のオラクル値 `restarts=1 late_fragments=0 restart_duplicates=39`)。
- 死んだ AsAd(A3 が再開しない): run の世代は 0 のまま、`sources_behind` = 1、新世代の A0–A2 のイベントは A3 待ちで timeout → incomplete emit(3/4)、late 0。
- 同じ idx が旧世代で emit 済み → 新世代で来たら restart_duplicates(現行どおり)。
- decoder: 2 ソース(2 receiver)で片方だけ再開 → `frames_before_restart` が辞書で片方だけ、`acquisition_restarts` 0(最小 epoch 上がらず)。両方再開 → 1。

## 結果(2026-10-15、Opus implementer × 4 往復(本体 + 追従 + 付け替え + 中点)、Fable 照合・SPEC v1.48)
- コミット(main に cherry-pick): `476dcb9`(§4 本体)、`eb60806`(時刻帯による追従)、`ff3fd34`(再開検出時の付け替え)、`b01c2e8`(中点規則)。変更: `tools/root_sink/{eb_core.hpp, monitor_pub.hpp, root_sink.cxx, test_eb_core.cpp, test_monitor_pub.cpp}`、`src/{decoder.rs, logbook.rs, controller/status.rs, controller/mod.rs, controller/sequencer/tests_stop.rs}`、`ui/src/app/logbook/logbook-record.{ts,spec.ts}`。
- 実測(Mac、2026-10-15): `test_eb_core` 284(着手前)→ 赤 293/41(§1b の症状: 4 AsAd バッチで restarts 0、新イベントが late)→ **391 / 0**。`test_monitor_pub` 92(13 キー)。`cargo test` **1081 / 0**(clippy 警告 0、fmt OK)。root_sink `make -j` 警告 0、conformance 49、recorder 174 / pevent 99 / raw_tree 91。実機 .graw リプレイ `root_sink_intake` 11/0(108 entries)、`mini_rawtree_e2e` 1/0。UI `npx ng build` 通過、spec 31/31。
- **1 ソースのオラクル不変**: archive/110 の実機 close 行を合成で再現(run 73 `events_complete=350 events_incomplete=0 late_fragments=0 restarts=1 events_before_restart=91 restart_duplicates=39`、run 76 `…=136 … restarts=1 events_before_restart=17 restart_duplicates=0`)、新旧コードで一致。
- 実装中に implementer が見つけて Fable が裁定した穴 3 つ(SPEC §6.3 に反映): (A) 古い残りを持たない AsAd が新しい取得を世代で割る → 時刻帯による追従(上端も要る: 再開で時計が 0 から始め直すので古い残りは新世代より時刻が大きい)。(A′) その AsAd が AsAd 順で最初の再開より前に来る → 再開検出時の付け替え(pending を新世代へ)。(A″) 死んだ AsAd の長い古い残りを誤って付け替える → 中点規則 `t < t0 + (floor_old − t0)/2`。却下した案: 振幅や idx の条件(高レートで取りこぼす)。
- 判断(採用): 「全ソース」= 期待集合(未着ソースは 0)、`restart_duplicates` は両向き、decoder は receiver 単位のまま(追従・付け替え無し、SPEC §2.3 に明記)、`source_restarts` / `source_epoch_joins` はビルダの受け口のみ(JSON には出さない)、`sources_behind` は C++ status PUB まで(Rust monitor / UI への配線は後続)。
- テスト名(§4 の 6 項目 + 追補): `test_a_four_asad_restart_in_asad_major_order_keeps_old_and_new_epochs_apart`、`test_a_restart_is_detected_even_when_every_old_event_is_incomplete`、`test_single_source_oracles_of_run73_and_run76_are_unchanged`、`test_a_dead_asad_holds_the_run_epoch_and_new_events_time_out_incomplete`、`test_new_epoch_ids_already_emitted_by_the_old_epoch_are_dropped_with_four_asads`、decoder `frames_before_restart_is_per_source_and_a_restart_counts_once_every_source_restarted`、logbook `frames_before_restart_reads_both_the_old_number_and_the_per_source_map`、`test_a_fresh_asad_without_a_stale_head_follows_the_new_epoch_by_time`、`test_the_epoch_follow_band_is_inclusive_by_the_restart_gap`、`test_a_fresh_asad_that_comes_first_is_reattached_when_another_asad_restarts`、`test_the_reattach_threshold_is_exclusive`、`test_stale_frames_of_an_already_restarted_asad_are_not_reattached`、`test_a_fragment_is_not_reattached_onto_an_already_emitted_idx`、`test_a_dead_asad_with_a_longer_stale_head_is_not_reattached`、`test_the_reattach_midpoint_is_exclusive`。
- §2(decoder の全ソース合算)は辞書化で解消。

## 実機反映(2026-10-16 07:50–07:58、ユーザー了解「やってください」)
- 1 回目(07:50): 非対話シェルで `cargo` / `npx` が PATH に無く Rust と UI は未ビルド、root_sink だけ新版 → daq_down/up → 再現 A/B/C(repro110.sh): **run 143 `events_complete=184 events_incomplete=0 late_fragments=0 restarts=1 events_before_restart=19 restart_duplicates=19`**、run 144(通常の stop→start)`restarts=0`。root_sink の新版は 1 AsAd で 110 と同じ形で動く。UI は `rm` 後に cp が失敗して 404 → Mac で `npx ng build` して scp で復旧(ELITPC に node 無し)。
- 2 回目(07:55、`PATH=$HOME/.cargo/bin:$PATH`): decoder / controller を再ビルド → daq_down/up → 再現: **run 146 `events_complete=183 events_incomplete=0 late_fragments=0 restarts=1 events_before_restart=10 restart_duplicates=0`、`run_stop.counters` = `acquisition_restarts 1, frames_before_restart {"0": 10}`(辞書、SPEC v1.48 §9.2)**、run 147 `restarts=0`、`frames_before_restart {}`。UI / status とも 200。root_sink の events_before_restart(10)と decoder の frames_before_restart["0"](10)が一致(1 ソース)。
- ELITPC = main `14d58ce` 相当(Rust、root_sink、UI)。4 AsAd の経路は合成テストのみ — Warsaw で実機確認(現地)。

ELITPC への反映(RUNBOOK §7.1: bundle → build → root_sink make → daq_down/up)と 110 と同じ再開の再現(stop せずにスタックを落として run/start、1 AsAd なので close 行が 110 と同じ形になること)は、**一晩 run(134)の後、ユーザー了解を得てから**。Warsaw(4 AsAd)での実機確認は現地。

## 実行できなかった操作
implementer: heredoc の一括書き換えが worktree 隔離で拒否(Edit で代替)、`timeout` コマンド無し(付けずに実行)。主対話: なし。
