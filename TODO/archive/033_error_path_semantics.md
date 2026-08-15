# 033 — 異常系セマンティクスの裁定(SPEC §1.3 fatal 経路 / run_stop.reason)+ 異常系 E2E

**Status: COMPLETED**(2026-08-15 implementer/Opus — 結果は末尾の「結果」節)
**Status(起票時): READY(裁定完了 2026-08-14 / Fable。**①ユーザー承認 = 済み** — 033-①②③は
**SPEC v1.12 に適用済み**(§1.3 停止シーケンスの受信静止検出 `eos_quiesce_ms` /
§1.3 異常中止の機序訂正 + 終端条項 / §9.2 の `forced_eos`・`eos_closed`)。
残る着手条件は ②[034](034_consecutive_run_ops.md) の完了のみ — `src/controller.rs` を
034 が編集中のため。034 が入り次第 implementer/Opus へ発注)**
**起票**: 2026-08-14(030 の E2E-F を分離。実装からのエスカレーション → 発注側裏取り → 本裁定)
**仕様**: SPEC §1.3(v1.6 の異常中止経路 + 停止シーケンス)/ §6.2-5(root-sink fatal)/
§9.2(run_stop)/ §12-11
**関連**: [archive/030_p3_e2e.md](archive/030_p3_e2e.md) 裁定② /
[archive/016_controller.md](archive/016_controller.md):50-52(レビュー注記)/
docs/reviews/P2_review_2026-08-13.md:43-48(R3 = SPEC v1.6 の出典)/
**[032](032_receiver_stale_link.md) 調査結果 Q1/Q2(2026-08-14 — 本裁定の前提を変えた実 GET の事実)**

> 行番号はすべて **2026-08-14 時点、034 実装前**(034 が `src/controller.rs` の run **開始**経路と
> `tests/p3_e2e.rs` を並行編集中。本裁定が読んだのは stop / 異常系経路で、034 と重ならない)。

---

## 前提となる 032 の確定事実(裁定中に共有 — 本裁定に織り込み済み)

- **実 GET は `ecc stop` でデータリンクを close しない**(`DaqCtrlNodeI.cpp:419-435` daqStop =
  割り込み停止 + flush のみ。close は breakup か次 configure の resetDataSender だけ)。
- したがって**実機の run 停止では receiver に EOF は届かない**。SPEC §1.3 停止シーケンス 1
  「ecc stop(CoBo が送信停止 → TCP close)」は事実誤り(032 改訂案①が修正)。
- 帰結: **強制 EOS(receiver `Stop`)が実機の正規停止経路**であり、「EOF を 5 秒待つ」段は
  実機では**毎回まるごと空振りする**(run 停止のたびに `eos_timeout` = 既定 5 s を待つ)。
- 「到達可能だが EOS を出さない receiver」は**実機の通常状態**(異常ではない)。

---

## 裁定サマリ

1. **論点 1**: 実装側の主張は**真**。「decoder Reset → 同一 run 内の seq ギャップ → root-sink
   fatal」は現実装に**決定的な再現経路が無い**。SPEC §1.3 v1.6 の記述は**機序が誤り**
   (P2 レビュー R3 の合成ミス)。裁定 = **(a) 改訂**: 処方(run クローズ前に Reset しない)は
   **残し**、機序を「EOS バリア喪失 → **次 run 冒頭の run-number-mismatch fatal(exit 6)として
   遅発**」に訂正 + EOS が流れ切らないときの終端条項を追加。032 の事実とは独立に成立する。
2. **論点 2**: **専用フィールド追加** — `run_stop` に `forced_eos: bool` / `eos_closed: bool`。
   reason への合成はしない。**032 の事実による意味の更新**: `forced_eos=true` は実機では
   **常態**(異常の印ではない)。異常の印は **`eos_closed=false`** ただ一つ。§9.2 改訂案は
   この意味論で書く(末尾)。
3. **論点 2′(032 事実を受けた追加裁定 — 停止シーケンスの見直し)**: 毎停止 5 秒の空振り待ちは
   **受信静止(quiesce)検出で置換**する — 「自然 EOS 完了」or「全 receiver の `bytes` が
   `eos_quiesce_ms`(新設、既定 500 ms)不変」の早い方で強制 EOS へ進む。`eos_timeout` は
   ハード上限として維持。ロスレス(ecc stop 後の在飛データの飲み切り)を保ったまま停止を
   秒未満にする。§1.3 改訂案は 032 改訂案①の**上に**書く(末尾)。
4. **論点 3**: E2E は **in-process スポーン継続**(receiver REP ポートの設定可能化は**不採用**)。
   SIGSTOP は不要。シナリオ 3 本を新規 `tests/p3_error_paths.rs` に:
   **F0 = 実機正規停止経路(リンク保持 + 強制 EOS)の初照合**(032 の事実により「異常系」から
   「実機の正常系」に格上げ)/ F1 = eos-timeout / F2 = 遅発 fatal(論点 1 の機械照合)。
5. **併発見(production 所見)**: `eos_complete` の代理観測が**最後のホップ = decoder 自身の
   EOS 送出を見ていない**。root-sink 停滞 + PUSH HWM 満杯の条件でのみ「EOS 未達なのに
   reason=normal」になり得る。小修正(decoder `eos_out` + 判定 3 点目)を実装に同梱(下記 C)。

---

## 論点 1 — 確定した答え(全項ソース追跡済み。推測は明示)

### 1-1. abandon の全経路と「その後スレッドが生きるか」(実証)

`RunDecoder::batch_abandoned`(src/decoder.rs:404-415)は確かに `out_seq += 1` する
(「下流にはギャップとして見せる」意図)。しかし **root-sink の `SeqCheck` が Gap を検出できる
のは、abandon の後に同一 run で「より大きい seq のバッチが実際に届いたとき」だけ**
(tools/root_sink/rs_core.hpp:246-258 — `last + 1` 以外で、`<= last` は Regression、超えは Gap。
届かなければ何も起きない)。よって問いは「abandon の後に同じソケットで送出が続くか」に帰着する。
abandon の発生経路は 4 つ(src/decoder.rs:605-637 `send_lossless` + 643-681 `send_pending`):

| 経路 | 発生箇所 | その後スレッドは | 同一 run 内 Gap になるか |
|---|---|---|---|
| **Reset override**(EAGAIN 中に abandon フラグ) | send_lossless:615-625 | `do_reset`(decoder.rs:1106-1115)は abandon 設定**→直後に** stop 信号。スレッドはループ先頭(801-805)の `try_recv` で**必ず抜ける**。抜け際の最終 `send_pending`(923)は pending が空で何もしない | **ならない**(末尾切断 = prefix) |
| ETERM(コンテキスト破棄) | send_lossless:627-630 | 次周回の `zmq::poll` が ETERM → break(835) | ならない |
| **一般 send エラー**(EINTR 級) | send_lossless:631-634 | **生き続ける** — 次バッチが成功すれば seq が飛ぶ | **なり得る**(1-2) |
| **符号化失敗**(`to_msgpack` Err) | send_pending:659-668 | **生き続ける** | なり得る(実質到達不能 — 1-2) |

**結論(実証)**: 実装側の主張どおり、**Reset 経路では abandon されるのは末尾のバッチであり、
スレッドは直後に抜ける。root-sink から見えるのは途中で途切れた prefix であって Gap ではなく、
`SeqCheck` は発火しない**。`eos_abandoned`(decoder.rs:427-435)も `rearm` 後は送出せず抜ける
ため、同一 run 内の Regression も作らない。

### 1-2. 「Gap → exit 3」が実在する残余経路(実証 + 到達性は推測)

- **一般 send エラー経路**: 候補は実質 **EINTR**(sndtimeo=1000 ms のブロッキング send が
  シグナルで中断。各 bin は `tokio::signal::ctrl_c()` でハンドラを入れる — src/bin/decoder.rs:52)。
  **到達性は低い(推測)**: signal-hook 系は SA_RESTART で入るため send の EINTR 返しは
  プラットフォーム依存の稀事象、かつ SIGINT 後はプロセスごと畳まれる。
- **符号化失敗経路**: 固定構造体の msgpack 直列化 Err はアロケーション障害級(推測だが強い)。
- **ns 級レース(理論のみ)**: `do_reset` の `abandon.store(true)`(1108)と `signal_stop`(1110)
  の**間**に、EAGAIN 起床(最大 1 Hz)→ abandon 観測 → `batch_abandoned`(warn 込み µs 級)→
  ループ先頭 `try_recv` までが**すべて**割り込むと次の 1 バッチが送られ Gap になる。
  決定的再現は不可能。

**よって 030 E2E-F の発注(この fatal の E2E 再現)は原理的に満たせなかった**(裁定②は正当)。
Gap/Regression → exit 3 自体は単体テスト(test_rs_core、厳格 0 起点 = root_sink.cxx:1124)が
既に担っており、E2E での再現価値もない。

### 1-3. decoder Reset の実害の正体(実証 — 改訂の核)

Reset を run クローズ前に打ったときの被害は seq ギャップではなく **EOS バリアの喪失**:

1. decoder スレッドは自分の EOS を送らずに(または `eos_abandoned` して)消える。
2. root-sink は run を**開いたまま**待ち続ける(RunState は EOS でしか閉じない。ビルド中
   イベントは idle tick で incomplete 排出されるが run は閉じない — root_sink.cxx:1128-1139)。
   `run_inprogress_*` が残り、monitor.root も書かれない。
3. **次の run の最初の Data(新 run_number)で run-number-mismatch fatal、exit 6**
   (root_sink.cxx:664-680 — seq 検査 702-710 より**前**に判定)。同一 run 番号の再投入時のみ
   SeqCheck の **Regression → exit 3**(新 decoder は seq 0 から。rs_core.hpp:256)。
4. つまり fatal は**起きる**が「同一 run 内の Gap」ではなく**次 run 境界の遅発 fatal**、
   exit コードも通常 **6**。

さらに現実装は eos-timeout で `eos_closed=false` のままでも停止手順 4 で**全コンポーネントへ
Stop→Reset を必ず打つ**(controller.rs:621-642)。v1.6 の処方「run クローズ前に Reset しない」は
EOS が流れ切らない場合には**実装上守られていない**(終端条項が SPEC に無く、畳むしかない —
これ自体は妥当)。改訂で終端条項(畳んでよい。ただし**次 run 前に root-sink を再起動**)を
明文化する。

### 1-4. なぜ SPEC はそう書いたか(出典追跡)

- **docs/reviews/P2_review_2026-08-13.md:43-48(R3)が起源**:「decoder の Reset は送出を打ち切り
  (可視カウント — 009 の設計どおり)、その結果 root-sink は seq ギャップで exit 3(fatal 死)
  する」。**2 つの単体仕様(009 の out_seq 前進 + 008 の SeqCheck)を系として合成する際に、
  スレッドが abandon 直後に終了する = 後続送出が無い、を見ていない**。実行で確認した記録は無い。
- SPEC v1.6(2026-08-13)が R3 の解消としてこの機序を §1.3 に採録(SPEC_ja.md:170-176)。
- 016 レビュー注記(archive/016_controller.md:50-52)も同前提を引き継ぎ、さらに
  「reason="error:eos-timeout" で可視」とした点も不正確(典型ケースは Abort が優先 — 論点 2)。

### 1-5. 裁定

**(a) SPEC の記述を実態に合わせて改訂**(文面は末尾)。(b) は不要 — root-sink は in-band で
検出可能なものは既に全部検出しており、「EOS の来ない prefix 切断」は in-band では「遅い decoder」
と区別不能。検出点は必然的に次 run 境界(= 現に exit 6 で検出される)か controller の時間監視
(= `eos_closed=false`)であり、root-sink へのアイドルタイムアウト fatal 追加はデータ駆動設計
(§1.3)に反し誤発火源になる。(c) 全削除も不採用 — 処方と遅発 fatal は実在する。

---

## 論点 2 — 裁定: `forced_eos` / `eos_closed` を run_stop に追加(reason 合成はしない)

### 事実確認(実証、2026-08-14 時点・034 実装前)

- `stop_run_blocking`(controller.rs:1643-1662)は最初に `collect_status` を取り、**Error または
  不達が 1 つでもあれば `StopMode::Abort("component-error:…")`**。
- reason 決定(controller.rs:644-652)は **Abort > error:eos-timeout > abort:ecc-stop-failed >
  normal**。「receiver が死んで EOS が出ない」ケースは `abort:component-error:receiver0` で確定し、
  第二段 `wait_for_eos` の失敗(= eos-timeout の事実)は reason に現れない。
- `forced_eos` / `eos_closed` は `StopReport` に**既にある**(controller.rs:392-394)が、HTTP 応答
  (1701-1708)と **audit の params**(1683-1694)止まりで、**run_stop レコードに載らない**
  (logbook.rs:146-156 / controller.rs:1674-1682)。notes も audit の error 欄のみ。
- §9.2(SPEC_ja.md:713)の run_stop は reason を「"normal" / "error:..."」としか書いておらず、
  実装 + §1.3 v1.6 の `"abort:..."` が未記載(既存乖離)。

### 裁定(032 の事実で意味論を更新)

- **run_stop は唯一の run 台帳**(§9.2)。audit 行と突き合わせないと停止の顛末が読めないのは
  記録品質の欠陥 → **`forced_eos: bool` + `eos_closed: bool` を追加**(転記 2 行 + スキーマ。
  追加のみ = 後方互換)。
- **意味論(032 反映)**: `forced_eos` は「EOS を receiver `Stop` で注入した」の記録であり、
  **実機 TCP flow では毎停止 true が正常**(stop はリンクを close しない → EOF は来ない)。
  false になるのは EOF 由来の自然 EOS(リプレイ・breakup 先行)のみ。**異常の判定材料は
  `eos_closed=false` ただ一つ**で、そのとき reason は `error:eos-timeout`(Normal 停止)か
  `abort:...`(起因が別にある)になる。ログブックだけで「どう閉じたか / 閉じ損ねたか」が読める。
- **reason への合成(`abort:...+eos-timeout` 等)は不採用** — パース規約が壊れ、「停止をどの
  モードで始めたか」と「EOS がどうなったか」という直交する 2 事実を 1 フィールドに畳むことに
  なる。優先順位は現状維持。notes の run_stop 転記も不採用(audit の error 欄で足りる)。

### 論点 2′ — 停止シーケンスの見直し(032 事実の帰結。裁定範囲としてコーディネータ承認済み)

毎停止 5 秒の空振り(実機では EOF が構造的に来ない)は放置しない。ただし**即・強制 EOS も
不採用** — ecc stop の flush 後、在飛データ(TCP 途上 + receiver 未読分)を drain し切る前に
receiver を Stop すると尻尾を落とす(絶対ルール違反)。裁定は**受信静止(quiesce)検出**:

- 第一段の待ちを「`eos_complete`(自然 EOS)**または** 全 receiver の `bytes`
  (GetStatus metrics に既在 — receiver.rs:225)が `eos_quiesce_ms` のあいだ不変、の早い方」に
  置換。receiver 不達は「静止扱い」(drain できない相手を待っても無意味 — 第二段の Stop 不達で
  可視になる)。
- `eos_timeout` は**ハード上限として存置**(quiesce が効かない病理でも従来と同じ時間で打ち切る)。
- 既定 `eos_quiesce_ms = 500`(`[controller]` の新キー、省略可)。LAN の在飛は ms 級であり
  10 倍マージン。**eos_timeout 既定の引き下げ(代替案 ii)は不採用**(盲目の短縮は flush と
  競合し得る)。**現状維持(案 iii)も不採用**(031 の 24 h back-to-back soak が停止毎に 5 秒
  燃やすのは受け入れない)。

---

## 論点 3 — E2E の設計(030 E2E-F の代替。032 の事実で再構成)

### 前提の裁定

- **receiver REP ポート設定可能化は不採用**。固定 47110+k(config.rs:46 / receiver.rs:105)は
  SPEC §3.2 の意図的な固定表。テストは 030 実証済みの **in-process スポーン**
  (`command_listen: "tcp://127.0.0.1:0"` — tests/p3_e2e.rs:709/728-751)で全ポート動的にできる。
- **SIGSTOP は使わない**: SIGSTOP した receiver は REP ごと固まり「到達可能だが EOS を出せない」
  の再現にならない(030 の発見どおり)。controller の観測面では REQ タイムアウトも接続拒否も
  同じ `Err` → unreachable(controller.rs:758-776)なので、**in-process task の abort
  (= SIGKILL 相当)で観測等価**。
- **「EOS を出さない到達可能 receiver」= 実機の常態**(032 事実)。「データリンクを閉じない
  クライアント」(raw TcpStream で .graw を書いて保持)は**実機 CoBo の忠実な模擬**であり、
  これを受けて強制 EOS で閉じる経路(F0)は異常系ではなく**実機の正規停止の初照合**。
  ※zCoBo 組込みビルドの close 挙動の実機確定は 032 の P5 現地項目(そちらが正)。
- `error:eos-timeout` へ正直に到達する組み方: mode 決定(collect_status)の**後**に receiver
  task を abort する。mode=Normal のまま、`eos_complete` は decoder/graw-writer しか見ない
  (controller.rs:691-732、receiver は 698 で skip)ため第一段は満たされず、強制 EOS の Stop は
  不達、第二段もタイムアウト → `error:eos-timeout`。タイミングは race ではない(下記 F1)。

### シナリオ(新規 `tests/p3_error_paths.rs`。env gate は 030 と同じ)

- **F0 — 実機正規停止経路(リンク保持 → 強制 EOS)**: 独立トポロジー。リンク保持クライアントで
  mini 実 graw 全量送信(EOF しない)→ run/stop → 照合: `reason="normal"` / `ok=true` /
  **`forced_eos=true` / `eos_closed=true`(新フィールド)** / `run{N}.root` finalize
  (entries=108)+ monitor.root 存在 + 全ロスレスカウンタ 0(**尻尾を落としていない**ことの
  実証 — 論点 2′ の quiesce が入っても保存系無傷であることの受け入れ)。停止所要は
  assert しない(E の有無で変わる)が**結果節に実測を記録**(E 実装後の期待値: 1 s 未満)。
- **F1 — eos-timeout(論点 2 の照合)**: F2 と同一トポロジーの run。リンク保持のまま run/stop を
  発行し、mode 確定後(POST 受理 +0.5〜1 s)に receiver task を abort → 照合:
  `reason="error:eos-timeout"` / `ok=false` / `forced_eos=true` / `eos_closed=false` /
  audit error 欄に「EOS did not propagate within N ms」+ Stop 不達 note / root-sink は run を
  開いたまま(`run_inprogress_*` 残存、monitor.root 無し)/ Rust コンポーネント全回収。
  **タイミング規律**: テストは `eos_timeout=4 s`、E 実装後は加えて `eos_quiesce_ms=2000` を
  指定する(kill 窓 = collect_status 完了(ms 級)〜 quiesce 下限 2 s → マージン 1 s 超で決定的。
  E 不採用でも第一段 = 盲目 4 s なので同じ kill 時刻で成立 — **照合値は E の有無に依存しない**)。
- **F2 — 遅発 fatal(論点 1 裁定の機械照合)**: F1 の直後、同じ root_sink プロセスへテストが
  decoder ワイヤ形式の Batch(source_id=100 / run_number=旧+1 / seq=0 / well-formed payload —
  `tpcdaq::msg` で符号化)を 1 通 PUSH → 照合: **root_sink が exit 6** / stderr に
  `FATAL run-number-mismatch` / 終了 JSON の `fatal="run-number-mismatch"` と **run 1 カウンタの
  保全** / `run_inprogress_*` が finalize されずに残る。※「次 run の decoder が送る最初の 1 通」と
  バイト等価であり、controller 経由の連続 run 起動(034 の領分)に依存しない = KISS。

### 何を検証しないか(明示)

- **同一 run 内の Gap → exit 3 の実経路**(EINTR 級 / 符号化失敗): E2E では作れず、作る価値も
  ない(SeqCheck 単体テストが担保 — 1-2 の到達性評価)。
- **SIGSTOP 特有の half-open REP**: controller transport ではタイムアウトと拒否が同一エラー経路。
- **root-sink 停滞による false-normal**(サマリ 5 = eos_out の穴): PUSH HWM(1000 msg)が埋まる
  規模が要り、mini 実 graw では EOS がキューに乗って自然回復する(= 正しい挙動)ため E2E 不可能。
  **C の単体テストで固定し、負荷実走での観測は [031](031_load_harness.md) に申し送る**。
- **実機 zCoBo の stop 後リンク保持の実挙動**: 032 の P5 現地確認項目(E2E は模擬まで)。
- **連続 run の運用シーケンス**(ecc reset 段 / Arm リトライ / 採番): 034 の領分。

---

## 実装発注書(裁定承認後に発注)

**発注先想定**: implementer/**Opus**(多プロセス E2E + 停止経路の production 修正)。
**着手条件**: ①ユーザーが本裁定を承認(§9.2 追加 + §1.3 改訂 — 032 改訂案①と同時に v1.12 として
承認するのが望ましい)②034 COMPLETED(controller.rs / p3_e2e.rs の並行編集回避)。
**032 実装との関係**: 独立(032 = receiver.rs、本件 = controller stop 経路。quiesce は既存の
`bytes` メトリクスを使い、032 の新メトリクスに依存しない)。順不同で発注可。

### やること

- **A. run_stop に `forced_eos` / `eos_closed` を追加**(§9.2 改訂案どおり):
  `logbook.rs` `LogbookRecord::RunStop` に bool 2 個 → `controller.rs` `stop_run_blocking` で
  `StopReport` から転記。golden(logbook.rs 既存 + 029 の UI フィクスチャ転記元)更新。
  **追加のみ・既存フィールドの改名/削除禁止**。
- **B. なし(root-sink 側は無変更)** — 裁定 1-5。tools/ への変更禁止。
- **C. eos_complete の最終ホップ観測**: `decoder.rs` に `eos_out` カウンタ(`eos_sent` で加算、
  `metrics_json` に追加 — `eos_abandoned` と対)→ `controller.rs` `eos_complete` の decoder 判定を
  `eos_in >= expected && eos_out >= 1` に。016 の MockTransport ハーネス(controller.rs:2087-)を
  拡張して単体で固定。※`RunDecoder` は Start 毎に新規生成(decoder.rs:1071-)なので run 跨ぎの
  累積問題は無い。
- **D. `tests/p3_error_paths.rs` 新規**: F0/F1/F2。030 の流儀(env 不足は欠けた env 名を stderr に
  出して skip / 全ポート動的 / 早期死検出)。ハーネスは p3_e2e.rs から**必要最小限を複製してよい**
  (p3_e2e.rs の refactor 禁止 — 共通化は 031 の発注時に判断)。dev でも可(計測系ではない)。
- **E. 停止第一段の quiesce 置換**(論点 2′): `wait_for_eos` の第一段を「eos_complete または
  全 receiver `bytes` が `eos_quiesce_ms` 不変(不達 receiver は静止扱い)」に。
  `config.rs` `[controller]` に `eos_quiesce_ms`(省略可、既定 500)。`eos_timeout` はハード上限で
  存置。MockTransport に bytes 進行の模擬を足して単体で固定(自然 EOS 先行 / 静止検出 /
  ハード上限 / 不達静止扱い の 4 分岐)。**ユーザーが論点 2′ を否認した場合は E を落とし、
  ほか A/C/D はそのまま成立する**(F0/F1 の照合値は E 非依存に設計済み)。

### テスト / 受け入れ

- F0/F1/F2 green(env: `TPCDAQ_ROOT_SINK_BIN` / `TPCDAQ_ECC_BRIDGE_BIN` / `TPCDAQ_FAKE_ECC_BIN` /
  `TPCDAQ_REAL_GRAW` / `TPCDAQ_REAL_GEOMETRY_MINI`)。F0 は新フィールド + ロスレスカウンタ 0、
  F1 は新フィールドの値まで、F2 は exit 6 + JSON fatal + run 1 カウンタ保全まで機械照合。
- A/C/E の単体テスト(golden 更新 + mock 拡張)。
- ゲート: `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test` 全 green、
  `make -C tools/root_sink test`(-j)+ `run_conformance.sh` **無変更で** green(C++ に diff が
  無いことの確認を兼ねる)。
- 結果節: F0/F1/F2 の実測値(reason / 新フィールド / exit code / 残存ファイル / 停止所要)+
  A/C/E の diff 要約。

### ファイル所有権

- 変更可: `src/logbook.rs` / `src/controller.rs`(**stop 経路・eos_complete・wait_for_eos のみ** —
  034 が入れた start 経路に触らない)/ `src/decoder.rs`(カウンタ追加のみ)/
  `src/config.rs`(`eos_quiesce_ms` 1 キーのみ)/ `tests/p3_error_paths.rs`(新規)/ 本 md。
- 禁止: `tools/` 全部 / `src/receiver.rs`(032 の所有)/ `tests/p3_e2e.rs` /
  `docs/SPEC_ja.md`(本文はユーザー承認後に別途 — 下記文面をそのまま使う)。

---

## SPEC 改訂案(v1.12 案 — 本文編集はユーザー承認事項。**032 改訂案①〜④と同梱の想定**で、
①の適用を前提に書く。項番の最終調整はユーザー承認時)

### 033-①: §1.3 run 停止シーケンス 2 の待ち方(032 改訂案①-2 をさらに置換 — 論点 2′)

> 2. receiver: EOF が届いた場合(breakup 先行・リプレイ等)は `EndOfStream` を下流全リンクへ。
>    **実機の通常経路では ecc stop 後に EOF は届かない**(032 調査)ため、controller は
>    「自然 EOS の完了」または「全 receiver の受信静止(受信バイト数が `eos_quiesce_ms`
>    (既定 500 ms、設定可)のあいだ不変。不達の receiver は静止とみなす)」の早い方まで待ち、
>    receiver への `Stop` コマンドで **EOS を注入する(これが正規経路)**。`eos_timeout`
>    (既定 5 s、設定可)は両段のハード上限。静止検出は ecc stop の flush 済み在飛データを
>    飲み切ってから畳むための待ちであり、ロスレス規約の一部である(v1.12)。

### 033-②: §1.3 異常中止の段落(現 SPEC_ja.md:170-176 を差し替え — 論点 1)

> - **異常中止(abort)の正規経路(v1.6、v1.12 で機序を実装実態に訂正)**: 中止も必ず
>   「EOS を流して閉じる」: ecc `stop`(不達でも続行)→ receiver へ `Stop`(強制 EOS)→
>   EOS がチェーンを流れて root-sink が run をクローズ(incomplete は可視カウント)→
>   その後に各コンポーネントの `Stop`/`Reset`。
>   **run がクローズする前に decoder を Reset しない**。理由(v1.12 訂正): Reset の送出打ち切りが
>   作るのは「同一 run 内の seq ギャップ」ではない(打ち切られるのは末尾バッチで、以後の送出
>   なしにスレッドが終了するため §6.2-5 の Gap 検出は発火しない)。実害は **EOS バリアの喪失**で
>   あり、root-sink は run を開いたまま残り、**次の run の最初の Data で run_number 食い違いの
>   fatal(exit 6、§6.2-5)として遅発する**。`Reset` は run クローズ後の Error 復旧専用。
>   例外: root-sink 自体が死んでいる場合のみ上流の Reset は無条件に可(下流不在への abandon は
>   可視カウントされ無害)。
> - **EOS が強制でも流れ切らなかったとき(v1.12 追加 — 終端条項)**: 強制 EOS 後も `eos_timeout`
>   内に伝播を観測できなければ、controller はそれ以上待たずにコンポーネントを畳んでよい
>   (`run_stop` に `ok: false` と `eos_closed: false` を記録 — §9.2)。このとき root-sink の run は
>   開いたままである。**次の run を開始する前に root-sink を再起動(または fatal 死を回収)する
>   こと** — さもなくば次 run の最初の Data が上記の遅発 fatal を踏む(これは正しい検出であり、
>   抑止しない)。run_stop は `ok: false, reason: "abort:..."` または `"error:eos-timeout"`。

### 033-③: §9.2 run_stop 行(現 SPEC_ja.md:713 の該当セルに追記 — 論点 2)

> `reason`(**"normal" / "error:eos-timeout" / "abort:<原因>"** — abort は停止開始時点の起因。
> EOS の顛末は次の 2 フィールドが持ち、reason には合成しない)、
> **`forced_eos: bool`**(EOS を receiver `Stop` で注入したか。**実機 TCP flow では通常 true**
> (§1.3 v1.12 — stop はデータリンクを close しない)。EOF 由来の自然 EOS のみ false)、
> **`eos_closed: bool`**(EOS がチェーンを流れ切ったことを観測できたか。**false が唯一の
> 異常の印**であり、reason が abort でも eos-timeout の事実はここで読める)(v1.12 追加)

### 改訂履歴への追記案(032 改訂案①〜④と統合して 1 エントリ)

> / v1.12(2026-08-XX)①§1.3 停止シーケンスの事実修正(TODO/032 — 実 GET は stop でリンクを
> close しない。強制 EOS を正規経路化)+ 第一段を受信静止検出に置換(TODO/033 論点 2′、
> `eos_quiesce_ms` 新設)②§1.3 異常中止の機序を実装実態に訂正(TODO/033 — Reset は同一 run 内
> seq ギャップを作らず、実害は EOS バリア喪失 → 次 run 冒頭の exit 6 遅発 fatal。終端条項を追加)
> ③§9.2 run_stop に `forced_eos` / `eos_closed` を追加、reason の "abort:..." を明文化
> ④§1.4-6 receiver 単一リンク規約 + §12-13 + §13-7 追記(TODO/032 改訂案②〜④)。

---

## 備考

- 030 は §12-7〜11 をクローズ済み。本ユニットが閉じるのは「**異常時の意味論が SPEC と実装で
  一致していること**」= 016 レビュー注記の宿題。F0 は 032 の事実により**実機の正規停止経路の
  初照合**へ格上げ(030 E2E-C は EOF 由来の自然 EOS だった — 実機とは経路が違ったことになる)。
- eos_out の穴(サマリ 5)の負荷実走観測は 031 に申し送り(soak 中の root-sink 停滞模擬で
  HWM 満杯条件を作れるか、031 起票時に検討)。
- 034 の採番巻き戻し(論点 C)は `run_start` 記録前 = データ未流入時に限るため、1-3 の
  Regression(exit 3)経路とは交差しない — 034 レビュー時にこの前提を確認すること。
- 032 の S4(stop 後 breakup せず同一リンクで次 run)が将来採用された場合、F0 の「リンク保持 →
  Stop で close」意味論に影響し得る — その時は 032 裁定の再訪と併せて本 E2E も見直す。

---

## 追記(2026-08-15 Fable — 041 統合デモの実測を受けて)

- **041 で「A 未実装」が実 run で確認された**: `forced_eos`/`eos_closed` は現状 audit と
  REST 応答にのみ載り、`run_stop` レコードに無い(SPEC §9.2 不履行の実測確認)。
  実装時の照合材料 = `reference/_spike/demo/out/logbook_*_saved.jsonl`(正常 run 3 本 +
  CoBo 突然死 1 本の実物)。
- **SPEC v1.14 §9.2 の注記を A の実装・文言に織り込むこと**: 実機 TCP flow では
  `forced_eos:true` が常態のため、**`forced_eos:false` は「stop 前にリンクが死んだ」強い印**
  (041 D-2: CoBo SIGKILL = OS の正常 FIN → 自然 EOF → reason:"normal" で閉じ、他に痕跡なし)。
  「唯一の異常の印は eos_closed:false」という v1.12 期の文言は v1.14 で改訂済み。

---

## 結果(2026-08-15 implementer/Opus → 発注側(Fable)レビュー PASS)

- **ゲート全 green**: `cargo test` **430 passed / 0 failed / 1 ignored**(033 新規 13 本 +
  同時進行 043 分を含む)/ fmt・clippy(--all-targets 含む)警告ゼロ /
  root_sink 全スイート green(68/71/175/426/202/92 + conformance 49)/ C++ 無変更。
- **A**: run_stop に `forced_eos`/`eos_closed`(v1.14 の意味論コメント込み。欠落 =
  「記録なし」であって false ではない、を serde default + テストで固定)。**041 の保存
  logbook(CoBo 突然死 run の audit 値)をオラクルとしてそのまま採用**し、REST↔台帳の
  同値を機械照合。
- **C**: decoder `eos_out`(eos_abandoned と対)+ controller の 3 点判定
  (eos_in ≥ expected / eos_out ≥ 1 / files_open == 0)。欠落は note + 完了と読まない。
- **D**: `tests/p3_error_paths.rs`(新規 1,180 行)— F0 実機正規停止(リンク保持 CoBo 役 +
  強制 EOS、108 entries)/ F1 eos-timeout(run_inprogress 残存)/ F2 遅発 fatal
  (**exit 6 + run-number-mismatch + run 1 カウンタ保全**を実測固定)。
- **E**: 静止検出で **run/stop 5.607 s → 1.271〜1.534 s**(対照測定 4 回)。秒未満に
  届かない残り ~0.8 s は EOS 伝播 + ファイル close の実仕事。
  **併発見・修正**: 不達 receiver への GetStatus が poll 毎に `command_timeout`(5 s)を
  燃やし停止が分単位化する穴(F1 実測 55 s → 「一度不達なら以後静止扱い」で 11.2 s。
  poll 回数を単体テストで固定)。
- **逸脱の裁定(発注側)**: ①F1+F2 を 1 関数に連結 = **受理**(発注書自身が「同じ
  root_sink プロセスへ」と指定)②p3_e2e.rs 等への 1 行追従 = **受理**(機械的必然。
  禁止理由 = 034 並行編集は消滅済み)③`eos_quiesce_ms = 0` = **拒否すべきと裁定**
  (0 は「即・強制 EOS」= 本裁定が不採用とした挙動。`eos_timeout_s = 0` 拒否の前例に整合)。
  validation 1 行 + テストは**小粒フォローアップへ**(CURRENT.md 記載)
  ④UI 表示は P4 裁量(未接触)= 妥当 ⑤`status_timeout` が停止経路で未使用な件は
  **044 レビューの検討候補に登録**。
- 実行環境・日付: macOS Darwin 25.5.0(arm64)、2026-08-15。

**Status: COMPLETED**
