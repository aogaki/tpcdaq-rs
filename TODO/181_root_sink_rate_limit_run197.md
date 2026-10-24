# 181 — run 197: 周期 10 ms・全 ch(76 Hz)で root_sink が追いつかず、receiver の内部キューが溢れた

**Status: DRAFT — 方針はユーザー裁定済み、Fable レビュー済み(2026-10-24)→ CLAUDE.md / SPEC v1.53 の文言をユーザー確認 → 184–188 起票**(2026-10-24 起票 Opus。ユーザー「止められないです」「いいえ、止まってません」「現実に信号は来てます」)

## 0. 何が起きたか(run 197、2026-10-24 17:31–17:41)
- params `elinp__a6cb1018`: 物理(`configure-elinp.xcfg`、12.5 MHz・delay 2500・502 ns・120 fC・**full**)+ `triggerMode periodically`・**`triggerPeriod_ms 10`**(179 の新ノブで初の run)、comment「pedestal」。
- 取得のレート = 44,995 / 591 s ≈ **76 Hz**(= 周期 10 ms + 読み出し 2.9 ms。receiver の `frames` は溢れた分を含む受信数。**当初「110 Hz」と書いたのは溢れを二重に足した誤り** — 182 の計測で 10 ms → 78 Hz を確認、2026-10-24 訂正)。
- **root_sink の書き出しは ≈ 17.6 Hz**(PEventTPC 1 GB 分割 1,936 entries ≈ 110 s ごと、raw ツリー同時、`compression=101 imt=4 workers=1`)。
- 背圧が decoder → receiver に逆流(「blocking on backpressure (lossless)」の繰り返し)→ 17:35:11 に receiver の内部キューが満杯「dropping frames and reporting Error (SPEC §1.4-3)」→ 17:40:49 Error(`overflow_frames=19855`)。
- ユーザーの Stop(17:41:02): `reason abort:component-error:receiver0`、`eos_closed false`、receiver「illegal transition Error -> Configured」→ Reset で Idle。decoder `batches_abandoned 1`、receiver `messages_abandoned 2`(graw-writer への EOS を含む)。
- **失われたフレーム = 19,855**(receiver の分岐の手前なので graw にも無い)。graw = 7,008,629,760 B = 278,784 B × **25,140** = 44,995 − 19,855(完全一致)。
- Stop の後も root_sink は溜まった分を書き続け、UI のステータスバーは STATE running / RUN 197、Monitor に事象が流れ続けた → ユーザーには「止まっていない」と見えた。ECC は Ready、receiver / decoder / graw-writer は Idle、zCoBo との TCP 接続なし。

## 0b. 復旧(2026-10-24 17:58、ユーザー了解「進めてください」)
- Stop の EOS が root_sink に届かず(receiver Error → Reset で abandon)、root_sink は 17:56 に溜まった分を書き終えた後も run 197 を開いたまま(UI の STATE running)。
  この状態で次の run を始めると、~~そのデータが run 197 のファイルに入る~~ **root_sink が fatal(exit 6 `run-number-mismatch`、`root_sink.cxx:808–823`、SPEC §6.2-5 v1.10)で落ち、以降の run は ROOT 無しになる**(2026-10-24 Fable レビューで訂正 — 当初は `RunState::on_data` だけを見て「混ざる」と誤認)。
- `daq_down.sh` → root_sink は SIGTERM で吸い切って inprogress のまま閉じた(`stopped mid-run — kept … NOT finalized`、最終 JSON: fragments 24,042、events_complete 24,042、incomplete 0、late 0、pending 0、publish_drops 13,150)。
- 手で最終名に: `run_inprogress_1792853590.root` → `run0197_0012.root`(831 entries)、`run_raw_inprogress_1792852270.root` → `run0197_raw.root`(24,042 entries)。`mv -n`(既存の上書きなし)。
- `daq_up.sh` → status 200・`/` 200・Idle・8 プロセス + root_sink。ログブックにコメント(author claude)。
- **graw 25,140 vs ROOT 24,042(差 1,098)**: raw ツリーの eventId = 0..43,896、不連続 3 か所(= キューの溢れの塊)。→ **末尾の ≈ 1,098 事象(eventId 43,897 以降)が graw には在り ROOT に無い** =
  Stop / Reset の時点で receiver → decoder → root_sink の途中(ZMQ のバッファ・decoder の入力)にあったもの。数えられたのは receiver `messages_abandoned 2`・decoder `batches_abandoned 1`(fragments 1)だけ →
  **約 1,097 フレームが数えられずに消えた**(派生物の ROOT 側。graw には在るので graw から作り直せる)。

## 0c. 追加で分かったこと(2026-10-24 夜、181 を詰める前の確認)
- **溢れてから Error になるまで 5.5 分かかった**: 最初の溢れ 17:35:11、Error 17:40:49。receiver は溢れを drain タスクで数えるが、状態を Error にするのは**コマンドを受けたとき**(`receiver.rs` の `handle()` 冒頭の `latch_overload()`)。
  controller は run 中にコンポーネントの状態を見に行かない(背景の監視なし)ので、ユーザーが Run control 画面を開いて `/api/status` が GetStatus を送った 17:40:49 まで誰も気づかなかった。
- **SPEC §1.4-3「controller は run を異常停止して JSONL に記録する」は実装されていない** — 異常停止の判定は Stop を押したときだけ(`rest.rs::stop_run_blocking` の「中止判定」)。run 197 は 5.5 分間、溢れ(= graw も含めた喪失)を続けた。
- 182 / 183 で速さは改善(PEventTPC なしで 169 Hz に完全追従、上限 ≈ 220 Hz)。ただし構造の問題は速さと無関係に残る(ディスクの一時的な詰まり・解析の負荷などで ROOT 側が遅れれば同じことが起きる)。

## 1. 論点(未決 — Fable キュー候補)
1. root_sink の持続レート(全 ch・raw ツリーあり)≈ 18 Hz は最低目標 100 Hz(CLAUDE.md)に遠い → **182 で計測: workers 1 = 16.6 Hz、4 = ≈ 66 Hz(比例)、raw ツリーの費用はほぼ 0、重いのは PEventTPC**。150 の最大レート試験は短い run(2,333–7,674 事象)でキューが吸収していただけ。workers > 1・raw ツリーの圧縮・PEventTPC の書き方のどれが効くかを測る。
2. 周期トリガのノブ(179)で誰でも高レートを出せるようになった → UI に「全 ch の持続レートの目安」を出すか、周期の下限を設けるか。
3. Stop の後に root_sink の残りを書いている間、UI がそれと分かる表示(「draining run 197: 残り N」)。今は Running と区別がつかない。
4. ~~周期 10 ms で 110 Hz になった理由~~ → 誤算だった(実際は 76 Hz、上の訂正)。
5. **Stop / Reset のときに途中にあるフレームが数えられずに消える**(上の 1,097)= silent failure 禁止(CLAUDE.md)に反する。どこで消えたか(receiver の送信キュー / ZMQ HWM / decoder の入力)を特定し、数えるか吸い切る。
7. **溢れの検知と対応**: Error がコマンド待ちで遅れる + controller に監視が無い(§1.4-3 の異常停止が未実装)。
6. EOS が失われたとき root_sink の run を閉じる正規の手段が無い(今回は daq_down / up + 手で rename)。次の run/start で root_sink が古い run を開いたままなら拒否する、または強制 EOS を root_sink に直接送る経路。

## 2. 当面の運用
- 周期トリガで全 ch を長く取るときは周期 ≥ 60 ms(≲ 16 Hz)にする。ペデスタルは数千事象なら 10 ms でも 1 分以内に止めれば溢れない(キュー 1,000 + 1,000 メッセージ)。

## 3. ユーザー裁定(2026-10-24 夜)
1. **A**: graw は絶対に落とさない。ROOT の経路(receiver → decoder → root_sink)は、追いつかなければ**数えて落としてよい**。run は止めない。その run の ROOT は「欠けあり、graw から作り直す」と分かるようにする。
   = CLAUDE.md の絶対ルール「保存系は落とさない」の**保存系は graw-writer だけ**にし、root-sink は「graw から作り直せる派生物」に格下げ(ユーザー了承)。
2. Stop の終了処理は「下流が進んでいる限り待つ」+ リンクごとの送受の数を run_stop に書き、差 = 消えた数を必ず記録。画面は取得停止の直後から「書き出し中(残り N)」。
3. EOS を失った run は「**未完であることが分かればよい**」。
4. controller が run 中も数秒おきに状態を見て、溢れたらすぐ画面に赤く出す。

## 4. 設計の下書き(Opus、Fable レビュー前)
### 4-1. receiver: リンクごとのキュー(裁定 1)
- 今: drain タスク → 有界 sync_channel 1 本 → 送信スレッド 1 本 → PUSH ×2(graw-writer、decoder)。decoder が詰まると送信スレッドごと止まり、graw も止まる。
- 新: drain タスク → **リンクごとに有界キューと送信スレッド**(graw 用 1 本、decoder 用 1 本)。フレームは両方のキューに入れる(コピーか `Arc<[u8]>` — 実装の判断)。
  - **graw のキューが満杯** = 本当の過負荷(ディスクが入力より遅い)→ 今の §1.4-3 どおり `overflow_frames` + Error + **controller が異常停止**(4-4)。
  - **decoder のキューが満杯** → そのフレームは decoder リンクにだけ送らず **`decoder_link_dropped_frames`** を数える(初回 warn、以後カウンタ)。**Error にしない、run は続く**。graw には入る。
  - EOS は両方のキューで間引かない(満杯なら空くまで待つ — 今の規則のまま)。
  - 既定のキュー長は今と同じ(目標レート 2 秒分以上)。
- 「ROOT が欠けた」の印: run_stop.counters の `decoder_link_dropped_frames > 0`、status、UI(4-5)。root_sink 側は eventId の抜けとして現れる。作り直しは graw から(graw_replay で流し直す / `macros/graw2root.C`)。
### 4-2. 停止: 進んでいる限り待つ + 送受の数(裁定 2)
- `Sequencer::stop_run` の EOS 待ち(`wait_for_eos`、`eos_timeout` 固定 5 s)を、**decoder / graw-writer の受信・送出カウンタが動いている間は待ち続ける**に(`eos_quiesce` と同じ「静止したら判定」の作り)。上限 `eos_drain_cap`(既定 10 分、設定可)。上限で打ち切ったら今どおり Reset、ただし 4-3 の数で喪失が必ず見える。
- 待っている間 controller の phase は **Stopping**(既存)。`/api/status` に decoder の残り(受信 − 送出)を出す。
- run_stop.counters に**リンクごとの送受**: receiver `frames_sent_graw` / `frames_sent_decoder`、graw-writer `frames_in`、decoder `frames_in` / `frames_out`。controller が差を `in_transit_lost`(リンク別)として書く。
- decoder → root_sink のリンクは controller から見えない(root_sink に REP が無い)→ **EOS に送り手の送出フレーム数を載せる**(§2.2 の EOS ペイロードに `frames_sent`)。root_sink は run を閉じるとき受信数と比べ、差を close 行と status に `lost_in_transit` として出す。graw-writer も receiver の EOS の `frames_sent` と照合(二重の確認)。
### 4-3. EOS を失った run(裁定 3)
- root_sink: run が開いているときに**別の run 番号のデータ**が来たら、古い run を **inprogress の名前のまま閉じて**(= 未完、SPEC §6.5 の既存の印)、close 行に `closed without EOS (incomplete)`、カウンタ `runs_closed_without_eos`、そして新しい run を開く。今の「数えて同じファイルに混ぜる」(`RunState::on_data` の Continued + mismatch)をやめる。
- raw ツリーも同じ(inprogress のまま)。
### 4-4. 監視(裁定 4)
- controller に背景の監視タスク: Running / Stopping の間、**2 s ごと**に全コンポーネントへ GetStatus(receiver の `latch_overload` もこれで 2 s 以内に効く)。
  - receiver が Error(graw の過負荷)→ **自動で異常停止**(`abort:overload`、§1.4-3 の実装)。
  - `decoder_link_dropped_frames` が増えた → 止めない、`/api/status` に警告(4-5)。
- 監視の GetStatus と run/start・stop の制御コマンドが同じ REQ ソケットでぶつからないこと(transport の排他)— 実装の注意。
### 4-5. UI
- ステータスバー: `decoder_link_dropped_frames > 0` で赤い「ROOT DROPPED N(graw は完全、ROOT は graw から作り直す)」、receiver Error で赤い「OVERLOAD — run aborted」。
- phase Stopping の間「STOPPING — draining (decoder 残り N)」、root_sink が run を開いたまま(monitor の status)で controller が Idle なら「ROOT 書き出し中 run N」。
### 4-6. SPEC / CLAUDE.md(v1.53 案)
- CLAUDE.md 絶対ルール: 「保存系(graw-writer)は絶対に落とさない。root-sink は graw から作り直せる派生物 — 追いつかなければ数えて落とし、run に印を付ける」。
- SPEC §1.4(過負荷)、§1.3(停止の待ち方)、§2.2(EOS の `frames_sent`)、§2.3 / §9.2(カウンタ)、§6(root_sink の run の切り替え)、§11(UI)。
### 4-7. 分け方(発注書 = 1 フェーズ先まで)
- **184** receiver のリンク別キュー + decoder リンクの数えて落とす(Rust、並行性 → **Opus implementer**)。
- **185** controller の監視 + 自動異常停止 + 停止の待ち方 + 送受の数(Rust、Opus)。
- **186** EOS の `frames_sent`(Rust の送り手 + root_sink / graw-writer の照合)+ root_sink の run 切り替え(C++、Sonnet)。
- **187** UI の表示(Sonnet)。
- 受け入れ: 合成の詰まり(root_sink を遅くする試験用の `--throttle-ms` は既にある)で graw の完全性・カウンタの一致、実機で run 197 の再現(PEventTPC on + 周期 10 ms)→ graw 欠けなし。

## 5. Fable レビュー(2026-10-24、スポット Fable、読み取りのみ)と改訂後の設計
### 5-0. 前提の訂正(主対話がコードで確認済み)
- 誤認 A: run 中の別 run_number は root_sink が **fatal(exit 6)**(混ざらない)。§4-3 は exit 6 の**置き換え**として正しい。SPEC §6.2-5 と §1.3 終端条項(「次の run の前に root-sink を再起動」)も改訂。
- 誤認 B: `ZmqTransport::command` は呼び出し毎に REQ を作って捨てる(`transport.rs:47`)— ソケットの衝突は無い。本当の競合は ①REP は逐次(decoder `Reset` の join ≤ 2.1 s 中は GetStatus が status_timeout 1 s で一時不達に見える)②`run.phase` の Running → Stopping を監視と `POST run/stop` が取り合う。
- **見落とし(致命)**: receiver は Error で `Stop` を拒否(`command.rs` の遷移表に `(Error, Configured)` が無い)、強制 EOS は receiver `Stop` でしか出せない → **graw 過負荷 → Error → 自動異常停止の道は EOS を出せず run 197 と同じ結末**。→ 184 で「Error 中の `Stop` は `do_stop` を実行して Error に留まる」(3 コンポーネント共通 `handle()` の 1 規則、SPEC §1.3 に 1 行)。
### 5-1. 項目別の判定
- **§4-1 → 方式を差し替え(代案 B)**: 送信スレッド 1 本・キュー 1 本のまま、**decoder リンクの Data / Heartbeat だけ `ZMQ_DONTWAIT`**。EAGAIN(HWM 満杯 / 相手不在)ならそのバッチを decoder へは送らず `decoder_link_dropped_frames`(+ `_batches`、`_heartbeats`)、初回 warn。graw リンクは従来どおりブロック(ロスレス)、**EOS は両リンクともブロック**。
  - **落としたバッチでは decoder リンクの `sequence_number` を進めない**(進めると decoder が seq gap → Error になり「数えて落とす」が「decoder を壊す」に化ける)。本物の喪失(符号化失敗等)は従来どおり進める。
  - 緩衝 = decoder PUSH HWM 1000 バッチ + decoder 側 → 76 Hz で ≈ 13 s 分(§1.4-2 の 2 秒分以上)。
  - 恒等式: `receiver.frames − overflow_frames = graw のフレーム数`、`… − decoder_link_dropped_frames = decoder.frames_in`(receiver の `frames` は溢れ分を含む)。
  - 副作用: Start 直後に decoder PUSH の connect 前の先頭バッチが EAGAIN で落ちうる → e2e で確認(実機は ecc start まで数秒あり実害なし)。
  - 元案(2 キュー 2 スレッド)は per-frame 確保 2 倍・カウンタのリンク別化・Reset で 2 join・EOS の直列待ちが要る → 不採用。
- **§4-2 → 修正して承認**: 第二段の待ち = 「`eos_complete` **または** 観測カウンタが `eos_timeout`(既定 5 s、既存のノブ)の間 1 つも動かない」で打ち切り。**新ノブ `eos_drain_cap` は不要**。観測 = receiver `batches`・graw-writer `asad[].frames` 合計・decoder `frames_in` / `batches_out`。**Heartbeat 系は入れない**(永久待ちの罠)。
  - **Abort でも第一段(静止検出)を飛ばさない**(今の「中止は即・強制 EOS」は receiver の drain を即座に畳み、カーネル受信バッファの未読分が数えられずに消える = graw を落とす経路)。Abort は reason のラベルだけ。
  - `POST run/stop` の応答は分単位になりうる → UI は応答を待たず `/api/status` の `phase=Stopping` と **`stopping: {decoder_backlog, waited_s}`** で「書き出し中」を出す。
  - run_stop.counters に(全部 nullable): `decoder_link_dropped_frames`、`graw_frames`、`decoder_frames_in`、`decoder_fragments_out`、`decoder_abandoned`、差 `lost_receiver_to_graw` / `lost_receiver_to_decoder`、**`root_complete: bool`**(= overflow 0 ∧ dropped 0 ∧ eos_closed ∧ decoder_abandoned 0)= 「ROOT は graw から作り直せ」の印 1 個。
  - **EOS に `frames_sent`(§2.2 のワイヤ変更)は不採用**: decoder → root_sink は順序保証 + SeqCheck 厳格(gap = fatal)なので EOS が届いた run では途中欠落は既に不可能、失われるのは Reset 時の在庫だけでそのとき EOS は届かない(= 要るときに無い)。receiver → graw-writer は両端の GetStatus で controller が引き算できる。代わりに root_sink の close 行と status に run 内 `fragments=`。
- **§4-3 → 承認(前提を訂正)**: 別 run_number の Data → EventBuilder の pending を flush → 旧 run を **inprogress のまま**閉じる(PEventTPC / raw とも `finalize=false`、monitor.root は書かない = 未完の印)→ close 行 `closed without EOS (incomplete)`、`runs_closed_without_eos` +1 → 新 run を Opened と同じ手順で開く(SeqCheck・ヒスト・`run_at_open` を reset)。**別 run_number の EOS**(0 事象 run の後)も旧 run を未完で閉じ、その EOS は `stale_eos`。`RunState::on_data` に `Reopened` を足す。
- **§4-4 → 修正して承認**: 2 s ごとの GetStatus は `status_timeout` で。**phase == Running のときだけ**自動異常停止(Stopping 中は観測のみ)。**引き金は receiver / graw-writer の Error だけ**(保存系)— decoder の Error では止めない(画面を赤く、停止時に `root_complete:false`)。不達は引き金にしない。Running → Stopping は `controller.run` の mutex 下で CAS(負けた側は 409)。停止本体は `stop_run_blocking` を共用、operator `watchdog`、reason `abort:overload:<component>`。`latch_overload` の癖(コマンド時にしか Error にならない)は監視で実害が消えるので変えない。
- **§4-5 UI → 承認**。**§4-6 → 修正して承認**(§2.2 は変えない)。
### 5-2. 文言の案(v1.53 — ユーザー確認待ち)
- CLAUDE.md 絶対ルールの段落、SPEC §1.4 の全文差し替え: Fable の案を本チケットの付録 A / B に写す(主対話がユーザーに提示)。
- 併せて: §1.3(中止も第一段を通す、第二段 = EOS 完了 or カウンタ静止、Error 中の Stop、終端条項の置換、監視タスク)、§2.3 表(decoder リンクは間引かれうる)、§6.2-5・§6.5 表・§6.3 close 行の `fragments=`、§5.3 status、§9.2 の追加キーと `root_complete`、§11 の表示 4 件、§12 受け入れ「ROOT 側を止めても graw は完全」。
### 5-3. チケットとモデル
| # | 内容 | モデル | 依存 |
|---|---|---|---|
| 184 | receiver: decoder リンク DONTWAIT + 計数 + seq 不前進、**Error 中の `Stop` を実行**(3 コンポーネント共通 `handle()`) | Opus | なし |
| 185 | controller 停止: Abort でも静止検出、第二段を進捗待ちに、run_stop の追加キー + `root_complete`、`/api/status.stopping` | Opus | 184 のカウンタ名(発注書で固定すれば並列可) |
| 188 | controller 監視 + 自動異常停止(保存系 Error のみ、Running のみ、CAS、operator watchdog) | Opus | 184、185 |
| 186 | root_sink: 別 run_number で旧 run を未完で閉じて新 run を開く、close 行 / status に `fragments` | Sonnet | なし(184 と並列) |
| 187 | UI 4 表示 + ログブック行の `root_complete` | Sonnet | 184–186 |
- 受け入れテスト(Fable 案、各発注書に写す):
  - 184 ①decoder 不在 / SIGSTOP で N バッチ → graw 全受領・dropped = N のフレーム数・receiver Running ②再開後の decoder `seq_gaps == 0` ③EOS は decoder 満杯でも届く ④graw-writer を止めると従来どおり overflow + Error ⑤Error で `Stop` → success・EOS が両リンク・Error のまま・`Reset` で Idle ⑥正常 start で先頭バッチが落ちない ⑦heartbeat 落としも計数。
  - 185 ①カウンタが動く間は 5 s を超えて待ち、止まって `eos_timeout` で打ち切り ②Abort でも静止検出を通る ③run_stop の新キー・恒等式(欠けなし 0 / 欠けで正)④`root_complete` 4 ケース ⑤`eos_complete` は不変。
  - 188 ①receiver Error → 2 回目の poll までに `abort:overload:receiver0`(audit operator watchdog)②decoder Error では止めない ③1 回の不達では止めない ④`POST run/stop` と同時でも停止は 1 回 ⑤Stopping / Idle / Starting 中は引き金にならない。
  - 186 ①run N 開放中に N+1 の Data → N は inprogress・monitor.root 無し・`runs_closed_without_eos == 1`、N+1 は EOS で finalize ②別 run_number の EOS → 旧 run 未完・`stale_eos` +1 ③既存 exit 6 テストの置換 ④`fragments` が decoder `fragments_out` と一致。
  - 187 Angular spec で 4 表示と `root_complete:false` の行。
  - 実機(184–188 後): run 197 の再現(`pevent=true`・周期 10 ms・5 分以上)→ graw = receiver.frames − overflow、dropped > 0、run 継続、Stop で `eos_closed:true`・`root_complete:false`・root_sink 正常 close。graw-writer を SIGSTOP → ≤ 2 s で Error → 自動 abort → EOS → graw close・`forced_eos:true`。
- 注意: decoder リンクの緩衝はバッチ数建て(最大 8 MiB × 1000)→ 落とし始める閾値なので RUNBOOK に 1 行。181 §1-2(周期の下限 / 目安)は独立、Fable キューに残す。

## 付録 A — CLAUDE.md 絶対ルールの差し替え案(Fable、ユーザー確認待ち)
```
## Absolute Rule — データ保全(tpcdaq 版、v1.53 改訂 2026-10-24)

**生データ(graw)は絶対に落とさない。派生物とモニタは数えて落としてよい。この区別を混ぜない。**
- 保存系 = **graw-writer だけ**: ロスレス。PUSH/PULL の背圧で守る。意図的ドロップ禁止。receiver 内部キューの溢れ(= graw の喪失)は Error + controller の自動異常停止。
- 派生系(decoder → root-sink): graw から作り直せる(graw_replay / graw2root)。追いつかなければ **receiver が decoder リンクでだけ数えて落とす**(`decoder_link_dropped_frames`)。run は止めない。run_stop に `root_complete:false` の印。**ROOT 側の遅れで graw を巻き添えにしない**(run 197 の教訓)。
- モニタ系(monitor / UI 配信): PUB/SUB + 間引き。落としてよいが **silent にしない**(ドロップはカウントして可視化)。
- receiver は drain のみ。下流の遅延がソケットへ逆圧してはならない(never-stop)。
- 停止は「下流が進んでいる限り待つ」。消えた数はリンクごとの送受の差として run_stop に必ず残す(数えずに消える経路を作らない)。
- 「保存と可視化の分離」(C++ 版ハードニング C3 の教訓)をコンポーネント境界として維持する。
```

## 付録 B — SPEC §1.4 の全文差し替え案(Fable、ユーザー確認待ち)
```
### 1.4 過負荷時の縮退規約(v1.53 — 保存系 = graw-writer のみ。TODO/181 run 197)

「graw は絶対に落とさない」「receiver はソケットへ逆圧しない」は、下流が恒常的に入力より遅い
状況では同時には満たせない。その場合の規約を仕様として固定する。**ロスレスなのは receiver →
graw-writer のリンクだけ**で、decoder → root-sink の経路は graw から作り直せる派生物として
「数えて落とす」側に置く(ユーザー裁定 2026-10-24)。

1. receiver の受信タスクは常にソケットを drain する(never-stop)。送信タスクとは有界キューで分離。
2. graw リンク(receiver → graw-writer)は有限 HWM(既定 1000 メッセージ)+ 受け側有界内部キューの
   背圧で守る。一時的な停滞(ディスクスパイク等)はバッファが吸収する(既定サイズは目標レート
   **2 秒分**以上)。
3. receiver 内部キューが満杯に達したら、それは**graw の喪失 = システム過負荷 = Error 状態**。以降の
   フレームは `overflow_frames` としてカウントし(silent 禁止)、コンポーネントは Error を報告、
   controller は **run 中の定期監視(既定 2 s)でこれを検知して run を異常停止する**(`abort:overload:
   <component>`、§1.3 の中止経路 = EOS を流して閉じる。Error 中の receiver も `Stop` を受けて EOS を
   出す)。「静かに間引いて run を続ける」ことはしない。graw-writer の Error も同じ扱い。
4. **decoder リンク(receiver → decoder)は送り手側で数えて落としてよい**: receiver は decoder リンクの
   Data / Heartbeat を待たずに送り(`ZMQ_DONTWAIT`)、HWM 満杯・相手不在ならそのバッチを decoder へは
   送らず `decoder_link_dropped_frames`(+ batches / heartbeats)に数える(初回 warn、以後カウンタ)。
   **落としたバッチでは sequence_number を進めない**(decoder の seq 検査は「意図しない喪失」の検出に
   残す)。graw リンクはこの間もブロックせず進む。run は止めない。EndOfStream はこの規則から除外
   (§2.2 — 空くまで待つ)。decoder が Error でも run は止めない(§1.4-3 の自動異常停止は保存系のみ)。
5. 欠けた ROOT の印は run_stop の `root_complete:false`(= `overflow_frames == 0 ∧ decoder_link_dropped_
   frames == 0 ∧ eos_closed ∧ decoder の abandoned == 0`、§9.2)。復元は graw から(graw_replay / graw2root)。
6. モニタ系(root-sink → monitor の PUB)は常時間引き可。ドロップは sequence_number ギャップとして
   受信側で検出し、UI に累積数を表示する。
7. **契約違反検出時の応答の役割非対称(v1.10、R-P2-6)**: REP を持つコンポーネント(receiver / decoder /
   graw-writer)は **Error 状態を報告して drain は継続**し、止める判断は controller が行う(§1.3)。
   REP を持たない root-sink はプロトコル違反(seq gap・malformed)では**即 fatal 終了**が唯一の可視な
   失敗表明。ただし **run 開放中の別 run_number は fatal にせず、古い run を inprogress のまま閉じて
   (= EOS 未達・未完の印、`runs_closed_without_eos`)新しい run を開く**(§6.2-5 v1.53)。
8. **receiver のデータリンクは同時に 1 本(先勝ち、v1.12)**: (現行 6 項の本文をそのまま)
```
