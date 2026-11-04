# 204 — 下流(root_sink)が積み残しを書いている最中の停止が `error:eos-timeout` になる

**Status: COMPLETED(2026-11-04 — main `343f571`、ELITPC 反映 `aa4336a`、実機受け入れ run 217 合格)**(発注書 = 下の「Fable 裁定」節。181 の実機受け入れの不合格として扱う)(2026-11-03 起票 Opus。出どころ = 184–186 の実機の受け入れ run 214)

## 何が起きたか
- run 214(ELITPC、`pevent = true` = PEventTPC を書く遅い設定、CoBo の周期トリガ 10 ms ≈ 73 Hz、300 s、`~/test/accept184/accept197b.sh` — run 197 の再現)。
- 取得中は SPEC v1.53 どおり: 受信 23,252 = **graw 23,252**(lost_receiver_to_graw 0)、decoder リンクで**数えて落とした 6,251**(batches 6,250、heartbeats 3)、decoder の入力 15,479、受信 → decoder の差 1,522(`lost_receiver_to_decoder`)、`root_complete: false`、5 分間 run は止まらない。
- **停止**: `stop took 10.6 s`、`ok: false`、`reason: error:eos-timeout`、`eos_closed: false`、notes = "EOS did not propagate within 5000 ms of no downstream progress after the forced EOS — waited 5000 ms; the observed counters never moved"。その後 phase は Idle。root_sink は停止の後も十数分 PEventTPC を書き続けていた(15:18 停止 → 15:29 時点で _0007 まで)。

## 読み(Opus)
- 185 の停止の第二段「下流が進んでいる限り待つ」は、controller が観測する数(decoder 等)が動くことを前提にしている。root_sink が積み残しを書いている間は root_sink の受け口(rcvhwm 1,000)と内部キュー(1,000)が満杯で、decoder は送れずに止まる → controller の見る数は動かない → 5 s で `eos-timeout`。
- ELITPC では controller は root_sink の進み具合(書いた事象数)を見ていない(run_stop の `events_built` は以前から null — run 205–214 すべて)。root_sink は実際には進んでいる。
- 実害: graw は完全、DAQ はすぐ Idle、派生物は `root_complete: false` で印が付く。ただしログブックに「エラー」と残り、運転員を誤らせる。1,522 の「受信 → decoder の差」は、停止の時点で ZMQ の途中に詰まっていた分(後で root_sink が受け取るのか、捨てられるのかは未確認 — 下の確認 2)。

## 判断してほしいこと(Fable)
1. 直し方: (a) controller が root_sink の状態(events written / pending)を見て、それが動いている限り待つ(ELITPC で `events_built` が null な理由も合わせて直す)、(b) 「下流が詰まっているだけ」(decoder の送信待ち)を区別して待つ、(c) 停止は第二段を打ち切って ok: true で閉じ、派生物の未完は `root_complete: false` だけで表す(graw が完全なら run は正常、という SPEC v1.53 の精神)。
2. 確認すべきこと: root_sink が最後に run 214 をどう閉じたか(EOS が届いたか、未完で閉じたか、fragments の数)、1,522 フレームの行方。
3. 188(ウォッチドッグ / 自動の異常停止)との関係。

## 証拠の置き場
ELITPC `~/test/accept184/`(accept197b.log、status_A4_*.json = 10 s ごとの状態、run_stop_A4.json)、`/data/tpcdaq/run0214/`、`/srv/tpcdaq/logs/root_sink.log`(.prev)。

## 経過の確定(2026-11-03、ログ — 時刻は UTC、現地 = + 2 h)
- 13:14:51 receiver: `decoder link full or absent — dropping and counting (graw is unaffected, SPEC §1.4-7)`(開始から ≈ 1.5 分で root_sink の受け口・内部キュー・ZMQ が満杯)。
- 13:18:30 controller: `EOS did not propagate within 5000 ms of no downstream progress after the forced EOS`。
- 13:18:31 receiver: `send abandoned while stopping (Reset / component teardown) — this message is lost and counted … link="decoder" kind="EndOfStream"` → **EOS が詰まったリンクで捨てられた**(数えられている)。
- 13:18:32 decoder: `Fragments batch abandoned (Reset override or unsendable socket) … fragments=1`。
- root_sink: 受け取った 15,478 を全部書いた(`events_complete 15478`、incomplete 0、`eos 0`、`runs 0`)が、EOS が来ないので run を閉じずに待ち、15:30 の daq_down(受け入れのスクリプトの復元)で `stopped mid-run — kept …run_inprogress_1793712505.root (8 entries, NOT finalized)`・`…run_raw_inprogress_1793711600.root (15478 entries, NOT finalized)`。PEventTPC の part 0–7 は閉じている。
- → 停止で EOS が decoder リンクの詰まりで捨てられ、root_sink が run を閉じない。通常の運転なら次の run で 186 の「未完を閉じる」が働く。**直し方の候補に (d) を追加**: 停止の EOS は decoder リンクでも捨てずに待つ(または root_sink に別経路で届ける)— SPEC v1.53 の「受信は下流の遅れでソケットに逆圧しない」と両立させる形で。
- run 214 の `run_inprogress_1793712505.root`(4.5 MB)・`run_raw_inprogress_1793711600.root`(1.87 GB)は**ユーザーの指示で削除**(2026-11-03 15:38、graw 7 本 6.48 GB と PEventTPC part 0–7 は残す)。ログブックに comment(seq 6877、author `claude-accept184`、run 211–214 = 受け入れ試験・物理データではない)。

---

## Fable 裁定(2026-11-04、スポット Fable — 読んだもの: CLAUDE.md、SPEC §1.3 / §1.4 / §2.3 / §5.3 / §9.2 / §11 / §12、181、archive 184–186、`src/receiver.rs`・`src/decoder.rs`・`src/controller/{sequencer,rest,status,transport}.rs`・`src/monitor/{wire,sub}.rs`・`tools/root_sink/root_sink.cxx`、ELITPC の `~/test/accept184/`(accept197b.log、status_A4_5/15/29/30、run_stop_A4.json)と `/srv/tpcdaq/logs/*.prev`(読むだけ))

**結論: 181 の受け入れ(「Stop で `eos_closed:true`・`root_complete:false`・root_sink 正常 close」)は不合格。原因は 185 の第二段の観測点の選び方(decoder の進みを下流の進みの代理にした)で、receiver(184)と root_sink(186)は設計どおりに動いた。直すのは controller だけ。**

### 1. 原因(コードの行で)
停止の時系列(UTC、`controller.log.prev` / `receiver0.log.prev` / `decoder.log.prev` / `graw_writer.log.prev`):
1. 13:18:25.199 第一段 = 受信静止 600 ms で強制 EOS → receiver `Stop`(`sequencer.rs:698-706`)。receiver の drain が畳まれ、送信スレッドが EOS を graw-writer → decoder の順に送る(`receiver.rs:724-749`)。graw-writer は 13:18:25.200 に `all expected sources reached EndOfStream` = **graw 側は完結**。
2. 13:18:26.201 receiver: `downstream is full or absent — blocking on backpressure … link="decoder" kind="EndOfStream"` = `send_on`(`receiver.rs:949-1008`)が decoder リンクの HWM 満杯で待っている。**これは正しい**(SPEC §2.2 / §1.4-7: EOS は捨てない、空くまで待つ)。
3. 13:18:30.289 controller: 第二段 `wait_for_eos_while_downstream_moves`(`sequencer.rs:885-949`)が `still >= eos_timeout`(5 s)で打ち切り。`the observed counters never moved`。
4. 13:18:31.207 controller が receiver へ **`Reset`**(`sequencer.rs:721-739` の畳む段)→ `do_reset`(`receiver.rs:1216`)→ `abandon_sends`(`:1236`)→ 待っていた `send_on` が `abandon` を見て `record_abandoned`(`:967-969` → `:258-277`)→ **13:18:31.206 の WARN「send abandoned while stopping … kind="EndOfStream"」= 停止の EOS が捨てられた瞬間**(`messages_abandoned = 1`、数えてはいる)。
5. 13:18:31–32 decoder へ `Stop` → `Reset` → `send_lossless`(`decoder.rs:781-813`)が `abandon` で 1 バッチを打ち切り(`batch_abandoned`、`fragments=1`)。decoder は receiver の EOS を受けていない(`eos_in 0`)ので自分の EOS を出さない → root_sink `eos 0`・run 開いたまま。

**なぜ「観測カウンタが動かなかった」か** — 第二段の観測点(`downstream_progress`、`sequencer.rs:952-1000`)は receiver `batches`・graw-writer `asad[].frames`・decoder `frames_in` / `batches_out` の 4 つだけ。ecc stop + receiver Stop の後は、receiver は送るものが無く(EOS 待ち)、graw-writer は書き切っていて、decoder は **13:17:00 から `send_lossless` の中で止まっていた**(`decoder.log.prev`: `downstream is full — blocking on backpressure`、解放されずに Reset)。decoder → root_sink の PUSH/PULL は HWM(1000)に達すると、**libzmq は読み手が low-water mark(≈ HWM/2 = 500 通)まで吸うまで送り手を再開しない**。root_sink は ≈ 17 Hz で書いているので、decoder は**正常に 20–90 s 単位で凍る**(`decoder.log.prev` の 13:14:06→13:14:21、13:14:28→13:14:51、… ≈ 22 s 止まって ≈ 7 s 動く周期 = まさにこのヒステリシス)。status_A4_29(15:17:5x)と status_A4_30(15:18:0x)で decoder の `frames_in 15479 / batches_out 6493 / fragments_out 15478` は 10 s 間不変、その間 root_sink は part 2(15:17:15–15:19:09)を書き続けている。→ **「decoder のカウンタが 5 s 動かない」は「下流が止まった」を意味しない。本当に進んでいる場所(root_sink の `events_built` / `bytes_written`)を controller は見ていない。**

- root_sink の進みを見ていない理由 = root_sink に REP が無く(SPEC §1.3)、controller は root_sink の PUB status(§5.3、1 Hz)を購読していない。**run_stop の `events_built` / `events_incomplete` / `late_fragments` が ELITPC の全 run で null なのも同じ理由**(SPEC §9.2 v1.10「root-sink が REP を持たない間 … null」、`status.rs:176`、`logbook.rs:640`)。退行ではなく、最初から観測していない。
- **184 ③ が実機の状況を捉えなかった理由**: `tests/receiver_decoder_link.rs:434` は receiver 単体 — controller が居ないので 5 s 後の `Reset` が来ない。「receiver は EOS を待つ」(真)を確かめたが、壊したのは controller の Reset。185 の `tests_stop.rs` は MockTransport で「decoder の `batches_out` が動く間は待つ」をモデル化したが、**モックに root_sink が存在せず、「decoder は HWM ヒステリシスで凍るが root_sink は進む」という実機の形を誰も書かなかった**。欠けていたのは 184 ③ と 185 ① の間の統合(本物の root_sink を遅くして controller 経由で止める)と、「decoder の進み ≈ 下流の進み」という代理の仮定。
- 1,522(`lost_receiver_to_decoder`)の行方 = Reset の瞬間に receiver PUSH / decoder PULL の ZMQ パイプに居た分(設計どおり差として記録、root_sink には届いていない: `decoder_fragments_out 15478` = root_sink `fragments 15478`)。decoder PUSH パイプ・root_sink の rcvhwm / queue に居た ≈ 10,000 事象は Reset 後も届いた(production の linger は既定 = 無限。`zmq_helper.rs:209/220` の `set_linger(0)` はテスト内)— root_sink が 15:30 まで書き続けたのはこれ。**今回の直しで、この 1,522 も届くようになる(受け入れで `lost_receiver_to_decoder == 0` を要求する)。**

### 2. 裁定 — (a) を採る(厳密化して)。(b) は不要、(c) は却下、(d) は receiver では既に実装済みで直す場所が違う
- **(c) 却下**: ユーザー裁定(181 §3-2)「停止は下流が進んでいる限り待つ」と、停止の契約「`ok` = EOS と保存の完結」(SPEC §1.3 v1.38)に反する。打ち切ると EOS が届かず root_sink の run は次の run(186)か daq_down まで開いたまま、monitor.root は書かれず、その日の最後の run は inprogress で残る。「graw が完全なら run は正常」は `root_complete:false` が担う印であって、`ok` の意味を変える理由にならない。
- **(b) 不要**: decoder が「詰まっているだけ」か「死んでいる」かは decoder を見ても分からない(どちらも送出でブロック)。区別できるのは **root_sink が進んでいるか**だけ。(a) を入れれば (b) は要らない(KISS)。
- **(d)**: receiver の EOS 待ちは正しく動いた。捨てたのは controller の Reset。直すのは「root_sink が進んでいる間は Reset しない」= (a) の中身。別経路で EOS を root_sink に届ける案は経路を増やすだけで不採用。
- **(a) の中身(= 本チケットの実装、A–E)**:
  - **A. controller が root_sink の PUB status を購読する**(`[monitor] sub_endpoint` と同じ宛先 = root-sink PUB 47004。**新しい設定キーは作らない**)。専用 OS スレッド 1 本(`src/monitor/sub.rs` の `spawn_sub_thread` と同じ形: SUB、`subscribe("")`、RCVHWM 小、poll 100 ms、linger 0)。`kind == "status"` だけ `monitor::wire::StatusPayload` に decode(`fragments` を `Option<u64>` + `serde(default)` で追加 — 186 以降の root_sink は出している)、他の kind(hist_snapshot / built_event)は読み捨て。最新 1 通を `Arc<Mutex<Option<RootSinkStatus { received_at: Instant, payload }>>>` に置く。**Sequencer には `Transport` 越しに渡す**: trait に `fn root_sink_status(&mut self) -> Option<RootSinkStatus>` を 1 本足し、`ZmqTransport` は共有の最新値を返し、`MockTransport` は台本(poll ごとの列)を返す。Sequencer は IO を持たない、という今の切り方を崩さない。
  - **B. 第二段の終了条件と観測集合**(`wait_for_eos_while_downstream_moves`):
    1. **観測集合に root_sink の `events_built`・`bytes_written`・`fragments` を足す**(status が届いている限り)。既存の 4 点は残す。Heartbeat 系は入れない(変えない)。
    2. **EOS 完了の定義を root_sink の close まで伸ばす**: 今の 3 点(decoder `eos_in ≥ cobos` ∧ `eos_out ≥ 1` ∧ graw-writer `files_open == 0`)**に加えて、root_sink を「見ている」なら root_sink が run N を閉じたこと**(status の `run == N` ∧ `state == "idle"`)。「見ている」= その停止までに `run == N` ∧ `state == "running"` の status を 1 度でも受け取った(SUB スレッドは常駐なので run 中に見える)。見ていない(0 事象 run、root_sink 不在、テスト)なら今どおり 3 点で完了。**根拠**: SPEC §1.3 停止シーケンスの 3(root-sink が EOS で finalize)→ 4(コンポーネント Stop、run_stop)の順そのもの。decoder `eos_out` は「EOS がパイプに入った」であって「流れ切った」ではなく、root_sink が積み残しを持つ間に次の run を始めると、**新しい接続の Data が古い接続の EOS を追い越して root_sink に届き、186 が古い run を未完で閉じてしまう(EOS は stale)**という競合もこれで消える(controller が Stopping の間は `run/start` が 409 で弾かれる — 既存)。
    3. **静止の打ち切りは「root_sink の status が `eos_timeout` 届かない(= root_sink が死んだ / 居ない)か、届いているが観測集合が 1 つも動かない」ときだけ**。それ以外は分単位でも待つ(ノブは置かない — 185 と同じ)。
    4. **ロスレスの守り(185 の穴)**: graw-writer が **到達可能で `files_open > 0`** の間は、静止しても **絶対に打ち切らない**(Reset が receiver の graw リンク送出待ちを `messages_abandoned` に捨てる = graw の意図的ドロップで、CLAUDE.md の絶対ルール違反)。`eos_timeout` ごとに warn 1 行(「graw-writer still has N files open — waiting; the lossless link is never cut by a timer」)+ notes に 1 度。打ち切ってよいのは graw-writer に `GetStatus` が届かない(プロセスが居ない)ときだけ。ZFS の sync スパイク(5 s 超)で停止中に graw を失う経路を閉じる。
  - **C. run_stop と `/api/status`**: `events_built` / `events_incomplete` / `late_fragments` を **close を観測した status(`run == N`、`idle`)の値で埋める**(root_sink を見ていなければ今どおり null。v1.50 以降この 3 つは run ごとの値)。`root_complete` の式は変えない(`eos_closed` が root_sink の close を含むようになる)。`/api/status.stopping` に `root_sink: {state, events_built, fragments} | null` と **`root_sink_backlog`** = Σ receiver(`frames − overflow_frames − decoder_link_dropped_frames`)− root_sink `fragments`(どれか取れなければ null)を足す = UI の「残り N」の元(187 ③)。
  - **D. 歩き戻し(114)の第二段も同じ関数にする**(`sequencer.rs:336-345` の `wait_for_eos` を `wait_for_eos_while_downstream_moves` に)。185 の申し送り(CURRENT.md「Fable 待ち」の 185)の裁定 = **統一する**。第二段の実装は 1 本だけ持つ(KISS)。`wait_for_eos` は呼び手が無くなれば消す。
  - **E. 運転員にどう見えるか(run 214 の条件で)**: Stop を押す → ecc stop(≈ 3 s)→ 静止 0.6 s → 強制 EOS → **phase `Stopping` が root_sink の書き切りまで続く**(run 214 なら ≈ 12 分: 積み残し ≈ 10,000 事象 ÷ 17 Hz)。`/api/status.stopping = {decoder_backlog, root_sink_backlog(減っていく), root_sink: {…}, waited_s}`。root_sink が `run 214 closed` を出し monitor.root を書く → controller が Stop / Reset を打つ → Idle。`POST /api/run/stop` の応答と run_stop: **`ok: true`、`reason: "normal"`、`eos_closed: true`、`forced_eos: true`、`root_complete: false`**(`decoder_link_dropped_frames 6251 > 0`)、`events_built = 15478 + 1522 = 17000`、`lost_receiver_to_decoder 0`、`lost_receiver_to_graw 0`。ログブックの読み方 = 「run は正常に閉じた。ROOT は欠けあり(graw から作り直す)」。**Idle に戻る時刻 = root_sink の close 時刻**(= UI の「書き出し中」が終わる時刻と一致。運転員が 2 つの時刻を持たない)。Stopping の間 `run/start` は 409(既存)。**途中で諦める API は作らない**(KISS)— どうしても待てないときは root_sink を SIGTERM(inprogress のまま閉じる、既存)→ status が止まる → `eos_timeout` で打ち切り → `error:eos-timeout`(RUNBOOK に 1 行)。HTTP 側: `tools/scan/scan.py` の `urlopen(timeout=300)` と UI の run/stop 待ちは分単位で切れうる → 187 で UI は応答でなく phase を見る、scan.py は RUNBOOK 注意(範囲外)。
  - **pevent = false(ELITPC の現用)では何も変わらない**: root_sink は 250 Hz に追従(A2: 停止 5.2 s、close は 11 s 後)。B-2 により停止が root_sink の close まで数秒伸びるだけ。

### 3. 11/9 との関係
- 11/9 の高レート運転は **`pevent = false` のまま**(182: PEventTPC なしで 169 Hz 完全追従、上限 ≈ 220 Hz > 1 GbE の 182 Hz)。この条件では root_sink の積み残しは秒で消え、204 の失敗は**起きない**。起きるのは `pevent = true` か、ディスクの詰まりで ROOT 側が数分遅れたとき。起きても graw は完全・数は残る(run 214 で実証)が、ログブックに `ok:false error:eos-timeout` が残り、root_sink の run は次の run か daq_down まで開いたまま・monitor.root 無し。
- **判断: 204 は 11/9 前に入れる価値がある(Opus implementer ≈ 1 日 + ELITPC 受け入れ 20 分)が、間に合わなければ RUNBOOK の 2 行で凌げる**:
  「高レート run は `pevent = false` で取る(PEventTPC はオフラインで graw から)。run_stop が `error:eos-timeout` でも `lost_receiver_to_graw 0`・`overflow_frames 0` なら graw は完全。ROOT は欠けあり(`root_complete:false`)、root_sink の run は次の run 開始で inprogress のまま閉じる(186)。作り直しは graw から。」
- 順番: **204 → 188 → 187**(204 と 188 は同じ `src/controller/` を触るので直列。187 は UI だけなので 204 のキー名が決まれば並行可)。

### 4. 188(監視 + 自動異常停止)・187(UI)
- **188 は 11/9 前に要る** → 起票した([188](188_controller_watchdog_auto_abort.md))。理由: SPEC §1.4-3 の自動異常停止は未実装で、run 197 は溢れを 5.5 分放置した。11/9 は 50 MB/s(182 Hz)を時間単位で書く = 180 GB/h を冗長性なしの ZFS に積む — ディスクの詰まり・満杯で graw-writer が止まれば **receiver の溢れ(= graw の喪失)を誰も止めない**(`latch_overload` はコマンドを受けたときだけ Error になり、controller は run 中に見に行かない)。188 の 2 s ポーリングでこれが ≤ 2 s で表に出て止まる。204 との関係: 188 の自動停止は `stop_run_blocking` を共用するので、204 の第二段(root_sink の close まで待つ・graw-writer が開いている間は切らない)がそのまま効く。
- **187 は縮めて要る** → 起票した([187](187_ui_v153_overload_and_draining.md))。必須 = ③「STOPPING — draining(ROOT backlog N、waited S s)」と **Stop ボタンが分単位の応答を「失敗 / 固まった」と見せないこと**(応答でなく phase を見る — SPEC §11 の既定)、①「ROOT DROPPED N」、ログブック行の `root_complete:false` の印。②(OVERLOAD)は 188 の後で意味を持つ、④(controller Idle で root_sink が run を開いたまま)は 204 の後は eos-timeout で打ち切った稀な場合だけになる — どちらも 187 に入れるが優先は低い。

### 5. SPEC の文言(v1.54 案 — 確定は主対話がユーザーに確認)
- **§1.3 停止シーケンス 2 の v1.53 段落**に追記: 「**v1.54(TODO/204)**: 観測カウンタに root-sink の status(§5.3 PUB、`events_built` / `bytes_written` / `fragments`)を加える。root-sink が run N を開いたことを status で見ていれば、第二段の完了は **root-sink が run N を閉じたこと**(status `run == N` ∧ `state == "idle"`)まで。見ていなければ(0 事象 run・root-sink 不在)従来の 3 点。root-sink の status が `eos_timeout` 届かなければ死んだとみなす。**graw-writer が到達可能で `files_open > 0` の間は静止でも打ち切らない**(ロスレスのリンクを時計で切らない)。decoder の `frames_in` / `batches_out` は decoder → root-sink の HWM ヒステリシス(libzmq は読み手が HWM/2 まで吸うまで送り手を再開しない)で数十秒凍りうるので、単独では下流の進みの証拠にならない(run 214)」。
- **§1.3 歩き戻し**: 「第二段 = EOS 伝播待ち(観測点・上限 `eos_timeout` とも停止経路と同じ)」の注に「v1.54: 停止経路と同じ『下流が進む限り待つ』(185 の申し送りの裁定)」。
- **§1.3 の `Reset` の注**(「Reset の `*_abandoned` は…」の段落)に 1 文: 「停止で Reset が EOS や graw リンクの送出待ちを捨てるのは root-sink / graw-writer が死んでいるときだけ(v1.54)」。
- **§5.3** 冒頭: 「status の読み手は monitor と **controller(停止の第二段と run_stop の材料、v1.54)**」。
- **§8.1 `GET /api/status`**: `stopping: {decoder_backlog, root_sink_backlog, root_sink: {state, events_built, fragments} | null, waited_s}`。
- **§9.2 run_stop**: 「v1.10: root-sink が REP を持たない間 events_built/events_incomplete/late_fragments は null」→「**v1.54: root-sink の close を status で観測できた run ではその status の値、観測できなければ null**」。
- **§11 ③**: 「STOPPING — draining(ROOT backlog N)」と「Stop の応答を待たず phase を見る」を明記。
- **§12-14**: 「Stop で `eos_closed:true`・`root_complete:false`・root_sink 正常 close」に「**controller は root-sink の close まで `Stopping`、`ok:true` / `normal`、`lost_receiver_to_decoder 0`、`events_built` 非 null**」を足す。

### ユーザー裁定(2026-11-04、主対話が確認)
- **B-4(graw-writer が到達可能で `files_open > 0` の間は停止を時計で打ち切らない)= 入れる**。ディスクが固まったときは運転員が graw-writer を落とすまで停止が終わらない(RUNBOOK に 1 行)— ロスレス優先。
- **B-2(root_sink が run を閉じるまで Stopping、閉じてから Idle)= close まで待つ**。run 214 の条件で ≈ 12 分、その間 `run/start` は 409。pevent = false の普段の運転では数秒伸びるだけ。
- SPEC v1.54(上 §5)はこの中身で、文言はマージ時に主対話が docs/SPEC_ja.md へ。
- 順番: 204 → 188(直列、同じ `src/controller/`)、187 は 204 のキー名(`stopping.root_sink_backlog`・`stopping.root_sink`)で並行。

### 主対話の裁定(2026-11-04、implementer の着手前の問い 1–5)
1. **B-4 の範囲 = 案 β**: 守り(静止でも打ち切らない)は「graw-writer が到達可能 ∧ `files_open > 0` ∧ graw-writer の state ≠ Error ∧ **全 receiver が到達可能**」の間だけ。理由: 守りの目的は「待てば graw が閉じる見込みがある間は Reset で receiver の graw リンクの送出待ちを捨てない」。receiver が不達ならその EOS は来ず、捨てる送出待ちもプロセスごと無い。graw-writer が Error なら PULL の消費を止めていて(`graw_writer.rs:744-746`)自力では閉じない。graw-writer 自身は Stop / Reset で finalize するので受け取り済みの graw は失われない。守りを外すときは warn 1 行 + notes 1 行(どの条件で外したか)。既存の F1(`f1_eos_timeout_leaves_the_run_open_and_f2_the_next_run_hits_a_late_fatal`、停止中に receiver を殺す)は無改変で `error:eos-timeout`。残る穴(2 CoBo で 1 台が死に、同時にもう 1 台の graw リンクがディスクで詰まる二重故障)は notes に残す。ユーザー裁定(B-4 = 入れる)の趣旨の範囲内と判断 — ユーザーに報告済み。SPEC v1.54 §5 の「打ち切ってよいのは graw-writer 不達だけ」はこの 4 条件に書き換える。
2. **テストダブルを実機に寄せる**(既存テストの本体は触らない): `MockTransport` は全 receiver が Stop を受けた後の graw-writer の GetStatus で `files_open = 0`(実機は receiver が EOS を graw → decoder の順に送り、graw-writer は ≈ 1 ms で閉じる — run 214)。テスト 6 用に上書きの builder(例 `graw_files_open_until(Duration)`)を 1 本。`tests/controller_integration.rs` のフェイク graw-writer は `eos_closed = false` でも `files_open: 0`(欠けるのは decoder 側だけ、という形)。逸脱として報告。
3. **D で変わる既存テスト** `a_fully_running_chain_is_closed_with_eos_before_the_walk_back_resets`: 名前と意図(EOS 待ちが receiver Stop と decoder Stop の間にあること)を保ち、呼び出し列と時間の期待値を計算し直す。逸脱として報告(どう変わったかを前後で)。
4. **N の渡し方 = 案 i**: builder `Sequencer::with_run(run)`。`stop_run_blocking` は `active.run`、歩き戻しは `previous_run_number(&reported_runs)` が `Ok(N)` のときだけ。N が無ければ「見ていない」= 従来の 3 点。既存テストは無改変。
5. **範囲外の機械的な 1 行**: `ControllerParams` の構造体リテラルに `root_sink_pub` を足す `tests/controller_ecc.rs`(2 箇所)・`tests/controller_ecc_timeout.rs`・`tests/p3_e2e.rs` を触ってよい。値は p3 系 = `sink.pub_ep`(本番の `from_config` と同じ配線)、ecc 系 = 何も listen していないダミーの endpoint。

### 6. 発注書(implementer へ — **Opus**。並行性・trait 設計・終了条件の判断が残る)
#### 作るもの(= 上の A–E。設計の分岐は無い — 出会ったら実装せず報告)
1. `src/controller/root_sink_status.rs`(新規): SUB スレッド(`monitor/sub.rs` の形)。`RootSinkStatus { received_at: Instant, payload: StatusPayload }` の最新 1 通を `Arc<Mutex<Option<_>>>` に。`kind != "status"` は読み捨て(カウンタだけ)。停止合図で終わる(スレッドを残さない)。接続先 = `config.monitor.sub_endpoint`(`ControllerParams` に `root_sink_pub: String` を足して `from_config` で写す。新しい TOML キーは作らない)。
2. `src/monitor/wire.rs` `StatusPayload` に `#[serde(default)] pub fragments: Option<u64>`(monitor の既存テスト・WS JSON は不変であること)。
3. `src/controller/transport.rs`: `Transport` に `fn root_sink_status(&mut self) -> Option<RootSinkStatus>`。`ZmqTransport` は共有の最新値を clone して返す(`Clone` 導出はそのまま — `Arc` を持つ)。`MockTransport`(`test_support.rs`)は台本 `Vec<Option<RootSinkStatus>>` を poll ごとに消費(`counter_moves_until` と同じ流儀)。
4. `src/controller/sequencer.rs`: B-1〜B-4 を `wait_for_eos_while_downstream_moves` と `eos_complete` に。「見ている」の判定は **第二段に入る前の最新 status と第二段の間に受けた status** で(`run == N && state == "running"` を 1 度でも見たか)。D: 歩き戻しも同じ関数。
5. `src/controller/status.rs` / `rest.rs` / `logbook.rs`(必要なら): C の run_stop 3 キーと `/api/status.stopping` の追加キー。
6. `src/controller/mod.rs`: `run_controller` で SUB スレッドを起動・終了時に join。
#### テスト(TDD: 先に赤。名前は固定 — 報告でこの名前を使う)
- `src/controller/sequencer/tests_stop.rs`
  1. `a_decoder_frozen_by_a_full_pipe_does_not_end_the_stop_while_root_sink_keeps_writing` — decoder の `frames_in` / `batches_out` を凍らせ、root_sink status の `events_built` を poll ごとに増やす。`eos_timeout` を越えても Reset は撃たれず、root_sink が `idle` を返した poll の後に `Stop → Reset`。`eos_closed == true`、`reason == "normal"`、`ok`。
  2. `the_stop_waits_for_root_sink_to_close_the_run_before_resetting_the_decoder` — decoder `eos_out = 1`・graw `files_open = 0` は早く揃うが root_sink は `running` のまま → 待つ。`idle` で終わる。コマンド列の順序を assert。
  3. `a_run_root_sink_never_opened_does_not_wait_for_its_close` — status が run N の `running` を一度も示さない(0 事象 / 不在)→ 従来の 3 点で完了(既存テストと同じ時間で閉じる)。
  4. `a_root_sink_that_stops_publishing_counts_as_still_and_the_stop_times_out` — run N を開いたのを見た後、status が来なくなり decoder も凍る → `eos_timeout` で打ち切り、`error:eos-timeout`、Reset。
  5. `root_sink_counters_alone_keep_the_second_stage_waiting` — 既存 `each_observed_counter_alone_keeps_the_second_stage_waiting` の形で `events_built` / `bytes_written` / `fragments` の 3 つを個別に。
  6. `the_stop_never_resets_while_graw_writer_is_reachable_with_open_files` — 全カウンタ静止・root_sink 不在・graw `files_open = 1` が `eos_timeout` の 3 倍続く間 Reset を撃たない(notes に warn 1 行)。`files_open` が 0 になった poll で従来どおり進む。
  7. `an_unreachable_graw_writer_lets_the_stop_time_out` — graw-writer の GetStatus 不達なら 6 の守りは効かず、今どおり `eos_timeout` で打ち切る。
  8. `run_stop_counters_carry_root_sink_events_from_its_closing_status` / `root_sink_counters_are_null_when_root_sink_was_not_observed`。
- `src/controller/sequencer/tests_walk_back.rs` 9. `the_walk_back_second_stage_waits_while_downstream_moves`(D。既存の歩き戻しテストは無改変 green か、時間の前提だけ変えてよい — 逸脱として報告)。
- `src/controller/root_sink_status.rs` 単体 10. `the_reader_keeps_only_the_latest_status_and_ignores_other_kinds`(本物の ZMQ PUB をテスト内に建てて hist_snapshot と status を混ぜて流す)。
- `tests/controller_integration.rs` 11. `the_status_shows_the_root_sink_backlog_while_stopping`(`stopping.root_sink_backlog` と `stopping.root_sink`、Stopping 以外では無い)。
- **統合(184 ③ と 185 ① の間の穴を塞ぐ本命)** `tests/p3_error_paths.rs` に 12. `f2_a_stop_with_a_throttled_root_sink_waits_for_its_close_and_reports_ok` — 本物の root_sink を `--throttle-ms`(既存)で入力より遅くし、decoder リンクの落としが発火する量を流してから `POST /api/run/stop`: 応答に数十秒かかる間 `/api/status.phase == "Stopping"` と `stopping.root_sink_backlog` が減ること、応答 `ok:true` / `reason normal` / `eos_closed true` / `root_complete false` / `lost_receiver_to_decoder 0` / `events_built` 非 null、receiver `messages_abandoned 0`、decoder `batches_abandoned + eos_abandoned 0`、root_sink のログに `run N closed` と `run{N}_monitor.root`、inprogress 無し。`TPCDAQ_ROOT_SINK_BIN` ゲート(他の root_sink e2e と同じ)。
- 全体: `cargo fmt && cargo clippy --tests -- -D warnings && cargo test`、`TPCDAQ_ROOT_SINK_BIN=… cargo test --test p3_error_paths --test p3_e2e --test controller_integration`。既存の停止テスト(033 / 114 / 185)は無改変 green(時間の前提を変えたら逸脱として報告)。
#### 受け入れ(実機 ELITPC — **主対話がユーザーの OK を得て実行**。物理 run 中は触らない)
- 反映後、`~/test/accept184/accept197b.sh`(pevent = true・周期 10 ms・300 s)を再実行 → 期待: 取得中は run 214 と同じ(dropped > 0、graw = 受信、run 継続)、**停止は `phase Stopping` が数分続き `stopping.root_sink_backlog` が単調に減る**、run_stop `ok:true` / `normal` / `eos_closed:true` / `root_complete:false` / `lost_receiver_to_graw 0` / **`lost_receiver_to_decoder 0`** / `events_built` = root_sink の close 行の事象数、root_sink ログ `run N closed … fragments=` + `run{N}_monitor.root`、inprogress 残りなし、receiver / decoder の abandoned 0。終了後 pevent = false・外部トリガへ復元(スクリプトが自動)。
- 対照: 通常 run(pevent = false、10 ms、60 s = A1 相当)の停止が従来 + 数秒以内で `ok:true`(root_sink の close を待つ分だけ伸びる)。
#### 禁止
- `src/receiver.rs` / `src/decoder.rs` / `src/graw_writer.rs` / `tools/root_sink/` を触らない(直すのは controller)。新しい TOML キー・新しい REST エンドポイント・上限ノブ(`eos_drain_cap` 等)を作らない。`eos_timeout` の意味を変えない。Heartbeat 系を観測に入れない。`docs/`・`TODO/`(本チケットの `## 結果` 節以外)・`ui/` を触らない。コミットは worktree のブランチにだけ(主対話がマージ)。
#### 触ってよいファイル
`src/controller/{mod.rs, rest.rs, sequencer.rs, sequencer/tests_stop.rs, sequencer/tests_walk_back.rs, status.rs, transport.rs, test_support.rs}`、新規 `src/controller/root_sink_status.rs`、`src/monitor/wire.rs`(`fragments` の 1 行)、`src/logbook.rs`(run_stop の 3 キーの埋め方が変わるなら)、`tests/controller_integration.rs`、`tests/p3_error_paths.rs`。worktree で作業、着手時に `git merge --ff-only main`。
#### 報告(必須)
worktree のブランチとコミット、`git diff --stat`、テスト 1–12 の名前・場所・PASS / FAIL、赤の証拠、cargo のテスト数、発注書からの逸脱と理由、**「実行できなかった操作」の節(権限拒否・失敗したコマンド。無ければ「なし」)**、残プロセスが無いこと。

## 結果(2026-11-04 — 実装・テスト済み。Fable レビュー、マージ、ELITPC の受け入れは未)
- **実装**: implementer(Opus)、worktree `.claude/worktrees/agent-a3a4df6a1a2270d4b`、ブランチ `worktree-agent-a3a4df6a1a2270d4b`、コミット **`dc8d957`**(main 663b00d の上に 1 つ、未マージ・未 push)。18 ファイル + 1,959 / − 75: `src/controller/{mod.rs, rest.rs, root_sink_status.rs(新規 276), sequencer.rs, sequencer/tests_stop.rs, sequencer/tests_walk_back.rs, status.rs, test_support.rs, transport.rs}`、`src/monitor/wire.rs`、`tests/{controller_integration.rs, p3_error_paths.rs}`、機械的な 1–5 行 = `tests/{controller_ecc.rs, controller_ecc_timeout.rs, p3_e2e.rs}`(裁定 5)・`src/monitor/test_util.rs`・`tests/{monitor_ws.rs, monitor_e2e.rs}`(`StatusPayload.fragments` の構造体リテラル)。主対話が `git show --stat dc8d957` と 14 本のテスト名(各 1 件)を照合。
- **テスト**(2026-11-04、Mac Darwin 25.6): `cargo fmt && cargo clippy --tests -- -D warnings && cargo test` → 警告 0、**1,147 passed / 0 failed / 1 ignored**(既存の ignored 1 本、TODO/202 の 3 本は落ちず)。controller の単体 96 passed。env ゲート(`TPCDAQ_ROOT_SINK_BIN=tools/root_sink/root_sink`(11/3 ビルド)・`TPCDAQ_FAKE_ECC_BIN`・`TPCDAQ_REAL_GRAW`・`TPCDAQ_REAL_GEOMETRY_MINI`、`SDKROOT=MacOSX27.sdk`): controller_integration 43 / p3_e2e 5 / p3_error_paths 2 passed + **1 failed = 既存の `f1_eos_timeout_leaves_the_run_open_and_f2_the_next_run_hits_a_late_fatal` の F2 部分**(186 で root_sink の exit 6 を撤去した後の古い期待、main 663b00d でも同じく落ちる — [206](206_p3_f2_stale_exit6_expectation.md) に切り出し)。F1 部分は β どおり `error:eos-timeout`(12.2 s)。
  | # | テスト | 結果 | 赤(実装前) |
  |---|---|---|---|
  | 1 | `a_decoder_frozen_by_a_full_pipe_does_not_end_the_stop_while_root_sink_keeps_writing` | PASS | 旧実装は 5.6 s で打ち切り、`eos_closed` 偽 |
  | 2 | `the_stop_waits_for_root_sink_to_close_the_run_before_resetting_the_decoder` | PASS | slept 600 ms ≠ 10.6 s |
  | 3 | `a_run_root_sink_never_opened_does_not_wait_for_its_close` | PASS | 最初から green(回帰ガード) |
  | 4 | `a_root_sink_that_stops_publishing_counts_as_still_and_the_stop_times_out` | PASS | 5.6 s ≠ 6.6 s |
  | 5 | `root_sink_counters_alone_keep_the_second_stage_waiting` | PASS | 5.6 s ≠ 7.8 s |
  | 6 | `the_stop_never_resets_while_graw_writer_is_reachable_with_open_files` | PASS | 5.6 s ≠ 16 s |
  | 7 | `an_unreachable_graw_writer_lets_the_stop_time_out` | PASS | 最初から green(回帰ガード) |
  | 8a | `run_stop_counters_carry_root_sink_events_from_its_closing_status` | PASS | 値が None |
  | 8b | `root_sink_counters_are_null_when_root_sink_was_not_observed` | PASS | `eos_closed` true ≠ false |
  | 9 | `the_walk_back_second_stage_waits_while_downstream_moves` | PASS | 5.6 s ≠ 13.8 s |
  | 10 | `the_reader_keeps_only_the_latest_status_and_ignores_other_kinds` | PASS | スタブで first status を待って timeout |
  | 11 | `the_status_shows_the_root_sink_backlog_while_stopping` | PASS | 旧 rest.rs で 10 s timeout |
  | 12 | `f2_a_stop_with_a_throttled_root_sink_waits_for_its_close_and_reports_ok` | PASS(env ゲート) | root_sink を見ない controller で `ok:false error:eos-timeout`(run 214 の再現) |
  | 追加 | `the_lossless_guard_is_lifted_when_a_receiver_is_gone_or_graw_writer_is_in_error` | PASS | note 0 本 ≠ 1 本 |
  - テスト 12 の緑の実測: `ok:true` / `normal` / `forced_eos:true` / `eos_closed:true`、停止 28.8 s、`root_sink_backlog` 0.5 s ごとに 2,148 → 23 と単調減少、run_stop `root_complete:false`・`lost_receiver_to_decoder 0`・`lost_receiver_to_graw 0`・`overflow_frames 0`・`decoder_link_dropped_frames 100`・`events_built` 非 null、root_sink fragments 2,300(= 送った 2,400 − 落とした 100)、`run 1 closed`・`run0001_monitor.root`・inprogress なし。作り方: 合成 16 KiB フレーム、root_sink `--throttle-ms 10 --rcvhwm 1 --queue 1`、`eos_timeout` 2 s、receiver HWM 200(実 .graw では debug ビルドの decoder が遅すぎて詰まらず、96 B では TCP バッファが呑んで落としが起きない — 実測)。
- **逸脱(承認済み・申告)**: テストダブルを実機に寄せた(裁定 2、`graw_files_open_until`)、114 のテストの期待値の更新(呼び出し列 `[gw, gw, decoder]` → `[gw, decoder]`、slept 800 → 600 ms — 原因は裁定 2 のモック)、`fragments` は `#[serde(default, skip_serializing)]`(WS JSON 不変)、p3_e2e の別プロセス controller の TOML に `sub_endpoint`(+ 2 行)、B-3 は観測集合の静止だけ(status 不着の別タイマーは作らない)、B-2 は `eos_complete` 共有で第一段にも効く(リプレイで root_sink が遅いと `forced_eos:true`)、`root_sink_backlog` は別 run の status なら null・負は null + notes、`ControllerError::RootSinkThread`、SUB の RCVHWM 16。note の文言: 守り = `graw-writer still has N files open — waiting; the lossless link is never cut by a timer`、外す = `lossless guard lifted: …`。
- **実行できなかった操作**: 権限拒否なし。失敗したコマンド 3 つ(`timeout` が macOS に無い、python 置換の assert、型の食い違い)— いずれも回避・取り下げ済み。残プロセスなし。
- **未**: Fable のフェーズ境界レビュー(2026-11-04 依頼中)→ マージ(187 と一緒、ユーザーの了解)→ SPEC v1.54 の文言を docs へ → ELITPC 反映(RUNBOOK §7.1)→ 実機の受け入れ(§6: `accept197b.sh` の再実行 + 通常 run の対照)。188 は 204 のブランチの上で実装中。

## Fable のフェーズ境界レビュー(2026-11-04、`dc8d957`)
- **判定: このままマージ可**(ロスレスと停止の終わり方に欠陥なし)。Fable が worktree で `cargo test --lib controller::` → 96 passed / 0 failed。env ゲートの e2e は implementer の報告とテスト本文の読みで判断。
- 確認された点: graw のリンクを時計で切る経路は閉じている(打ち切りは静止 ≥ `eos_timeout` ∧ 守りが Off / Lifted のときだけ)。永久待ちの経路(0 事象 run、停止中の root_sink の死・再起動、decoder の死)はすべて終わる。B-3 を値の静止だけにしたのは妥当(KISS)。SUB スレッドの起動・終了・Mutex・RCVHWM 16 は問題なし。テストダブルの変更は既存テストを弱めていない。silent failure なし。F2 を別票(206)にする判断は妥当。
- **受け入れの前に入れる(同じ worktree、`dc8d957` の上に 1 コミット — 2026-11-04 発注)**: A = receiver が到達可能でも Stop を拒んだら守りを外す(永久 Hold の残り)、B = 第一段の完了判定を chain の 3 点だけにする(自然 EOS のリプレイ / 歩き戻しで `forced_eos:true` の誤記録 — ELITPC 本番の TCP flow は常に forced なので実害は無い)、C = `stopping.root_sink` に `run`(別 run なら null)、D = テスト 12 を `f3_` に改名。
- **別票** → [208](208_controller_root_sink_status_followups.md): run 番号の再利用で前回の `idle run N` を close と取り違える(0 事象 run のとき run_stop の `events_built` などが前回の値)、controller が hist_snapshot(≈ 1 MB)を 1 Hz で全部復号してから捨てる。
- SPEC v1.54 の文言の修正案(Fable): §1.3 停止 2(status の静止 = 値の静止、守りの 4 条件 + A)、§1.3 歩き戻し(+ **186 で撤去済みの exit 6 の古い記述を直す**)、§1.3 Reset の注、§5.3、§8.1(`root_sink: {run, …}`)、§9.2、§12-14、RUNBOOK の 1 行 — マージ時に主対話が docs へ。

## 受け入れ前の小直し A–D とマージ(2026-11-04)
- implementer(Opus、同じ worktree)が `dc8d957` の上に **`90af662`**(5 ファイル + 145 / − 14: `src/controller/{rest.rs, sequencer.rs, sequencer/tests_stop.rs}`、`tests/{controller_integration.rs, p3_error_paths.rs}`)。A = Stop を拒んだ receiver で守りを外す(note `lossless guard lifted: receiver0 rejected the Stop — …`)、B = `eos_complete` を `chain_eos_complete()` / `root_sink_closed()` に分け、第一段は chain の 3 点で完了・root_sink が開いていれば receiver Stop を撃たずに第二段へ(停止経路と歩き戻しの両方)、C = `stopping.root_sink` = `{run, state, events_built, fragments}`・別 run なら null、D = テスト 12 → `f3_a_stop_with_a_throttled_root_sink_waits_for_its_close_and_reports_ok`。
- 新テスト: `the_lossless_guard_is_lifted_when_a_reachable_receiver_rejects_the_stop`(赤 = slept 60 s ≠ 5.6 s)、`a_natural_eos_with_root_sink_still_writing_is_not_recorded_as_forced`(赤 = forced_eos true)— PASS。テスト 11 に `run` の照合(赤 = Null ≠ 1)。既存テスト 2 は無改変で green、逸脱なし。cargo 1,149 passed / 0 failed / 1 ignored、env ゲート controller_integration 43 / p3_e2e 5 / p3_error_paths 2 + 既知の F2(206)。テスト 12 の再実測: 停止 25.7 s、backlog 2,141 から単調減少、fragments 2,300。主対話が `git show --stat 90af662` と新テスト名を照合。
- **マージ**: main `71dd15d` に `343f571`(Merge 204)・`654f2fd`(Merge 187)。**main の上のゲート**: `cargo fmt --check` OK、`cargo clippy --tests -- -D warnings` 警告 0、`cargo test` **1,149 passed / 0 failed / 1 ignored**、UI `npm test` 413 passed / 10 skipped・`npm run build` 警告なし(2026-11-04)。SPEC v1.54・RUNBOOK v0.5 を同じ日に docs へ(Fable の文言案 + A の条件)。
- 未: ELITPC 反映 → 受け入れ(`~/test/accept184/accept204.sh` = accept197b.sh + 停止中の status の記録)。

## 実機の受け入れ(2026-11-04、ELITPC — ユーザー了解「作業してください」、run 216 はユーザーが停止済み)
- **反映**: Mac main `aa4336a` を `git bundle 663b00d..main` → ELITPC `~/tpcdaq-rs` で ff → `cargo build --release`(16.6 s)。root_sink は 663b00d から無変更(diff 0 行)なのでビルドせず。UI は Mac の `ui/dist/ui/browser`(main 654f2fd でビルド)を `/opt/tpcdaq/ui` へ(旧版 `/opt/tpcdaq/ui.prev663`)。`daq_down.sh` → `daq_up.sh`: `/api/status` 200・`/` 200・Idle・8 プロセス + root_sink・ログに WARN / ERROR なし。controller のログ `subscribed to the root-sink status PUB endpoint="tcp://127.0.0.1:47004"`。
- **受け入れ run = run 217**(`~/test/accept184/accept204.sh` = accept197b.sh + 停止中 10 s ごとの status の記録。pevent = true・周期 10 ms・300 s、終了後に pevent = false・params `elinp__8efb10d4` へ自動復元 = 確認済み):
  | | run 214(204 の前) | **run 217(204 の後)** |
  |---|---|---|
  | 停止 | 10.6 s、`ok:false`・`error:eos-timeout` | **698.6 s、`ok:true`・`normal`** |
  | eos_closed / forced_eos | false / — | **true / true**(TCP flow は常に強制 EOS = 期待どおり) |
  | 受信 = graw | 23,252 = 23,252 | **23,309 = 23,309**(`lost_receiver_to_graw` 0、overflow 0) |
  | decoder リンクで数えて落とした | 6,251 | 6,309(batches 6,303、heartbeats 3) |
  | `lost_receiver_to_decoder` | 1,522(停止で捨てられた) | **0** |
  | root_sink | EOS 来ず run を閉じず(inprogress) | **`run 217 closed (… events_complete=17000 … fragments=17000)`、monitor.root あり、inprogress 0** |
  | `events_built` | null | **17,000**(= 23,309 − 6,309、恒等式が厳密に成立) |
  | root_complete | false | false(落としたので正しい) |
  | abandoned | EOS 1 通が捨てられた | `decoder_abandoned` 0、send abandoned の行なし |
- **停止中の見え方**(`accept204_stop.log`): phase `Stopping` が 11:41:53 → 11:53:28、`stopping.root_sink_backlog` が 7,514(waited 262 s)→ 49(696 s)と **10 s ごとに単調減少(≈ 17 /s = PEventTPC の書き出し速度)**、`stopping.root_sink = {run 217, running, events_built 9,486 → 16,951}`、`decoder_backlog` 0。close の 8 s 後に Idle(`root_sink_backlog` null)。
- **controller のログ**(`controller.log.prev`): `root-sink has the run open — the stop waits for its close (SPEC §1.3 v1.54) run=217 events_built=5028` → `receivers are quiet … forcing EOS now` → `downstream is still moving after the forced EOS — waiting past eos_timeout` → `root-sink closed the run run=217 events_built=17000` → `run stopped reason="normal" forced_eos=true`。status の購読 = status_in 1,019 / ignored 8,981(hist_snapshot など)/ decode_errors 0。WARN は receiver の `decoder link full or absent — dropping and counting`(設計どおり)の 1 行だけ。
- **181 の実機受け入れ(§5-3「実機: Stop で eos_closed:true・root_complete:false・root_sink 正常 close」)もこれで合格**。
- 未(188 の受け入れと一緒に): 対照の通常 run(pevent = false)の停止が従来 + 数秒で `ok:true`。187 の目視(STOPPING — draining の表示)はユーザーが次の停止で。
