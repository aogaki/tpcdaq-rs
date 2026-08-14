# 032 — receiver: 余分な TCP 接続 1 本で DAQ が silent stall する

**Status: COMPLETED**(2026-08-14。裁定(Fable)→ SPEC v1.12 でユーザー承認・適用 →
implementer/Opus 実装 → 発注側レビュー PASS + **既存テスト 1 本の retire を発注側が実施**)

## 結果

### 実装(2 ファイル)

- `src/receiver.rs`(+258/−8): `drain_task` の内側ループを **3 腕 select(biased: stop → read →
  accept)** に。accept 腕の `reject_extra_connection()` は **`await` を一切含まない同期関数**
  なので即 `drop` = FIN、**drain は 1 ミリ秒も止まらない**(never-stop を壊さない)。
  `owes_eos` は accept ではなく**接続の最初のデータバイト**で立てる(run 開始時の初期値 `true` は
  維持 = 無接続 run の強制 EOS を保護)。カウンタ `extra_connections` / `empty_connections`、
  GetStatus に `peer` / `last_read_unix_ns`、warn は初回のみ(ラッチは Reset で降りる)。
- `tests/receiver_stale_link.rs`(新規 618 行、統合 7 本)+ `src/receiver.rs` 単体 3 本。

### テスト(2026-08-14、macOS Darwin 25.5.0)

**発注側で再実行**: `cargo fmt --check` OK / `cargo clippy --all-targets -- -D warnings` 警告ゼロ /
**`cargo test` = 397 passed / 0 failed**(032 前 388 + 新規 10 − retire 1)。

- **TDD の red(実装前)**: `1 passed / 6 failed`。
  `余分な接続が閉じられない = backlog に滞留している(silent stall): Elapsed(())` —
  **030 E2E-C で踏んだ閉塞そのものを新テストが再現**できていることの証拠。
- 固定した仕様: **T1** 流通中に別接続を張る → 相手側 `read`=0(即 close)/ `extra_connections`=1 /
  **現接続の 4 フレームはフレーム境界一致 + バイト列一致** / EOS は EOF まで 0 回 /
  **T2** 3 本とも close + カウンタ 3、現接続の `frames` 無影響 / **T3** 0 バイト接続 →
  `empty_connections`=1・**下流は EAGAIN(EOS なし)**・その後の実接続 EOF で EOS ちょうど 1 回 /
  **T4a** データ有りの EOF は従来どおり EOS / **T4b** 無接続 run の Stop → 強制 EOS ちょうど 1 回
  (SPEC §1.3 の経路不変)/ 迷い込みが `owes_eos` を降ろさないこと / **T5** `peer` と
  `last_read_unix_ns`(切断で peer は null に戻るが**最終受信時刻は残る** = stale link 判別材料)。
- **`last_read_unix_ns` の更新頻度**: `unix_nanos()` を実測 **20.3 ns/call**(10^7 回 = 203 ms)。
  read は 256 KiB 単位なので 100 Hz 目標(≈27.9 MB/s)で **~110 read/s = 2.2 µs/s**、
  1 GB/s でも 81 µs/s。**間引かず read 成功毎に更新**(複雑さに見合わない)。根拠はコード内にも記載。

### 発注側の処置 — 既存テスト 1 本の retire(**意味論の反転による**)

`tests/receiver_integration.rs::eof_without_any_data_still_delivers_end_of_stream` は
**旧意味論**(「データ 0 バイトの接続でも EOF は run 境界」)を固定しており、
**SPEC v1.12 §1.4-6 と真っ向から矛盾**する。実装側は所有権外として**修正せず報告**(掟どおり)。
発注側が判断して **retire**(テスト本体を撤去し、**撤去理由・新意味論・引き継ぎ先を書いた
コメントブロックを同じ位置に残す** — 記録主義)。

- 引き継ぎ先: `receiver_stale_link.rs::a_connection_that_carried_no_byte_does_not_close_the_run`
  (新意味論版)+ `stop_still_forces_exactly_one_end_of_stream_when_nobody_connected`
  (無接続 run の強制 EOS)。**カバレッジは失われていない**。
- 付随して判明: このテストは新実装下で**失敗ではなく無限ハング**していた(同ファイルの
  `collect_until_eos` が Heartbeat を読み捨てて回り続け、`heartbeat_ms: 1_000` の餌で
  `rcvtimeo` 10 s が永久に発火しない)。**CI が返らなくなる罠**だったので retire は必須だった。
  retire 後の `cargo test` は 397 passed で正常終了することを発注側が確認。

### レビュー(発注側 Opus)

- 逸脱 5 件すべて受理。特に **accept 腕の `reject_extra` ガード**(accept が恒常エラーを返すと
  3 腕 select が即 Ready で回り続け **drain を食う busy loop** になる — エラー時はその接続の間だけ
  accept 腕を降ろし、外側ループの `ACCEPT_BACKOFF_MS` で拾い直す)は、**never-stop を守るための
  正しい追加**。`last_read_unix_ns` を未受信時 `null` にしたのも §9.2 v1.10「null ≠ 0」の流儀に合う。
- `Mutex` poison を `unwrap_or_else(|p| p.into_inner())` で復帰(`.unwrap()` 禁止 + Metrics は
  不変条件のない数値袋なので値を捨てない)= 023 の申し送りと同じ処方。

### P5 現地で機械確認できるようになったこと(実装の副産物)

| 項目 | 手順 |
|---|---|
| §13-7 の接続数目視 | run 中に receiver GetStatus の `metrics.extra_connections`。**0 のまま = 1 リンク構成が実機で確定**。> 0 なら 2 接続構成 = 「要改修」シグナル(初回 warn に peer が出る) |
| **zCoBo の close 挙動**(032 調査で唯一「推定」で残った点) | sm-stop 後に `peer` が **non-null のまま**なら「stop では close しない」が実機で確定(`last_read_unix_ns` が止まったまま peer が生きている = 静止したリンク)。その後 breakup で `peer` が null に落ちれば EOF 到達。**netstat 不要** |
| S1 復旧リハーサル | CoBo 電源断 → 再 configure。旧接続を握ったままなら **`extra_connections` が上がり warn が peer 付きで出る**(旧実装では完全に無言) |
**起票**: 2026-08-14(030 の実装からのエスカレーション → 発注側が実コードで裏取り)
**裁定**: 2026-08-14(Fable 級調査 — reference/ 実 ECC ソース + C++ 版 + zCoBo FW 資料で決着)
**仕様**: SPEC §1.4(never-stop)/ §1.3(run シーケンス — **事実誤りの修正案あり**)/ §12・§13-7
(注: 起票時の「§7(receiver)」は誤記 — §7 は graw-writer。receiver 仕様の所在は §1.1/§1.3/§1.4)
**関連**: [archive/030_p3_e2e.md](archive/030_p3_e2e.md) 裁定① / [034](034_consecutive_run_ops.md)(申し送りあり)

## 事実(発注側が `src/receiver.rs` を読んで確認済み — 起票時のまま)

- drain タスクは `'accept: loop { accept; loop { read } }` = **1 接続ずつ**。内側の read ループは
  EOF(`Ok(0)`)まで抜けないので、**先に張られた接続が黙っていると後続の接続は accept されない**
  (std listener の既定 backlog に SYN/ACK 済みで滞留 = 相手からは「接続成功」に見える)。
- 実測(030 E2E-C): `fake_ecc` が start() で張って保持する接続が受け口を占有し、あとから繋いだ
  `graw_replay` のデータが **1 バイトも読まれない**。この間カウンタは 1 つも動かず、警告も出ず、
  GetStatus に現接続 peer / 最終受信時刻が無いので運用でも検知不能。
- CLAUDE.md「**silent failure を作らない**」/ SPEC §1.4 never-stop の精神に抵触。

---

## 調査結果 Q1 — 実機で起こり得るか(2026-08-14、ソース実証)

### 結論: 実 ECC は probe 接続を張らない。正常系では 032 の閉塞は実機で起きない

**データリンクを張るのは CoBo 自身**であり、ECC はデータポートへ TCP を一切張らない。
ECC の役割は Ice で CoBo に「あそこへ繋げ」と指示するだけ。その 1 本がそのままデータの通り道で、
receiver への接続は正常運用では常に 1 本。030 E2E-C の閉塞は fake_ecc(テストダブル)が
CoBo の身代わりに接続を張る構成の artifact(030 裁定①の判断どおり)。

### 実証チェーン(実験使用中と同一版 = reference/20190315_patched)

1. **ECC 側は Ice 指示のみ**(configure 遷移でリンク確立を指示):
   - `GetBench/src/get/rc/SystemManager.cpp:485-513` — configure 遷移で DataLinkSet XML を parse し
     CoBo 毎に `setDataLinks` + `daqConnect()`。`:521-538` — **breakup** で `daqDisconnect()`。
   - `GetBench/src/get/rc/Node.cpp:70-105` — `daqConnect` → `hwNode().connectRunProcessor(sockAddr, flowType)`。
   - `MDaq/src/mdaq/rc/HardwareNode.cpp:171-188` — `connectRunProcessor` は
     **Ice プロキシ呼び出し `daqCtrl()->connect(type, addr)` のみ**。生 TCP なし。
   - 網羅確認: GetController/src・GetBench/src/get/rc・MDaq の EccBackEnd.cpp / EccImpl.cpp に
     `TcpConnection` / `acceptConnection` の使用は**皆無**(grep 実施)。
     `GetController/src/get/EccClient.cpp:159-166` の `daqConnect` も Ice 経由。
2. **CoBo 上の servant が connect する**(VxWorks API = CoBo 実機上で走るコード):
   - `GetBench/src/get/daq/DaqCtrlNodeI.cpp:247-316` — `createDataSender`。`:256` で
     `memReader.resetDataSender()` = **旧接続をまず close**、`:274` `StdDataSender`(TCP)/
     `:282` `FdtDataSender`(FDT)を生成。
   - `MDaq/src/mdaq/daq/TcpDataSender.hpp:86-91` — コンストラクタで `TcpConnection` 生成。
   - `MDaq/src/mdaq/utl/net/TcpConnection.cpp:65-87` — **コンストラクタで `::connect()`**
     (+送り側 SO_KEEPALIVE)。= **接続確立は ECC `configure` の時点**。
3. **start / stop はリンクを触らない**:
   - `DaqCtrlNodeI.cpp:387-414` `daqStart` — `dataSender().start()` は TCP 系では no-op
     (`MDaq/src/mdaq/daq/DataSender.h:53` — フラグ切替のみ)+ `sendTopology()`。
     `:399` **「Could not establish data link.」は CoBo 上で throw**され Ice 経由で ECC へ届く
     (016/017 負性テストの文言の出典。configure 時の接続失敗は Node.cpp:91 の
     「Could not connect data sender …」)。
   - `DaqCtrlNodeI.cpp:419-435` `daqStop` — 割り込み停止 + flush + フラグのみ。**close しない**。
     close は `disconnect`(`:358-363`、breakup 時)か次の configure の resetDataSender のみ。
   - `GetBench/src/get/daq/MemRead.cpp:362-380` `sendTopology` — **FDT のときだけ** 12 B の
     frameType 7 フレームを送る。mini 実 graw の「ctrl 12 B(frameType 7 ×1)」(§12-2)と一致
     = 実運用が FDT だったことの傍証(§13-4 の TCP 疎通確認の重要性を裏づけ)。
4. **GET 純正 DataRouter も単一接続**(ECC が probe を張らないことの運用上の証明):
   - `MDaq/src/mdaq/daq/TcpDataReceiver.cpp:92-179` — `:107-108` 接続が無いときだけ acceptor 生成、
     `:137-138` **接続確立後に acceptor を破棄(listen 自体を閉じる)**、`:155-159` EOF で再 listen。
     もし ECC が probe を張る設計なら、純正 DataRouter は唯一の受け口を probe に食われて
     一切データを受けられない。実験は何年も動いている → probe は存在しない。
     (副産物: 純正では余分な接続は **ECONNREFUSED で即可視**。うちは backlog で silent — 純正より悪い。)
5. **C++ 版 tpcdaq(実運用中)も逐次単一 accept**:
   - `~/test/get/tpcdaq/src/net/tcp_receiver.cpp:66-103` — accept → EOF/エラーまで recv → close →
     accept へ戻る。`:35` listen backlog=1。多重接続は捌けない。
     → **「C++ 版が実運用できている」のは「実 ECC が接続しないから」**。切り分け決着。
6. **ELITPC 運用スクリプト**(`reference/ZC706_20181031_ELINP/scripts_2asads/README_SCRIPTS.txt`):
   `:49` dataRouter を先に起動(listen-before-start)→ `:74,116,243` sm-configure に DataLinkSet
   (単一 DataLink)→ start。ECC からの probe 手順は存在しない。

### 推定(実証と区別して明記)

- **zCoBo の組込みビルド**(ZC706 = Zynq Embedded Linux)が上記 VxWorks 版 DaqCtrlNodeI と
  同一挙動かはソース未確認(SD_image はバイナリのみ)。README_SCRIPTS.txt の運用手順・
  実 graw 内の topology フレーム・DataLinkSet 意味論から**同系 GetBench コードと推定**。
  → P5 現地確認項目に落とす(下記)。

---

## 調査結果 Q2 — 異常時に閉塞が実在するか

| # | シナリオ | 実機で起きるか | 帰結(現行実装) |
|---|---|---|---|
| S1 | **CoBo の無 FIN 消滅**(電源断 / FW ハング / リンク断)→ half-open。復旧後の再 configure で CoBo が**新接続**を張る | **起きる**(ZC706 の再起動・emergency-poweroff は運用上珍しくない — README_SCRIPTS.txt III-(j)) | drain は旧接続の read で永久待ち。新接続は backlog で SYN/ACK 済み = CoBo からは接続成功に見え、daqStart も受信ウィンドウが埋まるまで成功 → **双方無警告で run が空振り**。純正なら ECONNREFUSED で configure が即失敗する(可視)ぶん、**うちの方が悪い** |
| S2 | **迷い込み接続**(閉域網でもポートスキャン / ヘルスチェック / 設定ミスの別プロセス) | 低確率だが**あり得る** | 先に accept されると本物の CoBo リンクが backlog 滞留(S1 と同じ stall)。さらに迷い込みが**即 FIN すると `Ok(0)` → 偽 EOS → run が空で閉じる**(現行意味論の副作用) |
| S3 | ECC 二重 start / 二重 configure による二重接続 | **起きない** | SM が防ぐ(configure は PREPARED からのみ、RUNNING から不可)。二重 daqStart は `isRunning_` ガードで no-op(DaqCtrlNodeI.cpp:389) |
| S4 | stop 後 breakup せず同一リンクで次 run(実 SM では合法: READY→RUNNING→READY→RUNNING) | 運用次第 | receiver は `Stop` で現接続を**こちらから close** するため、この運用パターンとは非互換(次 run の daqStart が CoBo 側 EPIPE →「Could not establish data link.」)。閉塞ではなく**可視な失敗**になるが、034 への申し送り必須(下記) |

**閉塞の持続時間はいずれも run 境界まで**: receiver `Stop` で drain が畳まれ全接続 close、
次 run の Arm/Start で更地。恒久 deadlock ではなく「**その run が無言で空振りする**」のが実害。
問題の本質は閉塞そのものより **silent であること**(0 Hz と stale link を区別する材料がゼロ)。

**併せて発見した SPEC の事実誤り**: §1.3 run 停止シーケンス 1「ecc stop(CoBo が送信停止 →
TCP close)」— 実 GET は **stop では close しない**(上記実証 3)。EOF は breakup まで届かないため、
実機の run 停止は「EOF 5 秒待ち → 強制 EOS」ではなく**最初から強制 EOS 経路が正規**になる。
改訂案①で修正する。

### 034 への申し送り(controller の連続 run 設計との接点)

receiver の現行意味論(Stop で現接続 close / TCP EOF = run 境界)が成立する条件は
「**次の run の前に必ず ecc configure を通す**(= CoBo がリンクを張り直す — resetDataSender が
旧接続を close して新接続)」こと。034 がどの復帰経路(breakup/undo → 再 describe 系)を選んでも、
configure を再実行する経路なら receiver 側の変更は不要。configure を飛ばして start だけ再発行する
経路(実 SM では合法)を選ぶ場合は S4 の非互換が顕在化するので、その時は本チケットの裁定を再訪する。

---

## 裁定 Q3(2026-08-14)

### 採用: (a′)「先勝ち単一リンク + 余分接続の即時 close + 可視化」

案 (a) の可視化を、GET 純正 DataRouter と**同型の fail-fast**に強化した形。
ロスレス契約・背圧・EOS の基本意味論には触れない(KISS)。

1. **余分接続の即時拒絶**: 現接続保持中も accept を継続し、余分な接続は即 drop(close = FIN)+
   初回 warn(peer アドレス付き)+ `extra_connections` カウンタ(毎回加算)。
   → S1/S2 の silent stall が「イベント駆動・閾値レス」で即可視になり、かつ相手(CoBo)にも
   接続断として即見える(純正の ECONNREFUSED 相当の fail-fast)。
2. **0 バイト接続は run 境界を構成しない**: `Ok(0)` / read エラーで EOS を出すのは
   **その接続で 1 バイト以上読んだ場合のみ**。0 バイトの接続終了は `empty_connections` として
   カウント + info(S2 の偽 EOS 防止)。`owes_eos` は accept 時ではなく**最初のデータバイトで**
   立てる(迷い込みが EOS 勘定に一切影響しない)。強制 EOS(Stop)経路は不変 —
   run が閉じられなくなることはない。
3. **GetStatus の可視化**: metrics に `peer`(現接続、null 可)/ `last_read_unix_ns` /
   `extra_connections` / `empty_connections` を追加。0 Hz(トリガ無し)と stale link の切り分けは
   これで運用可能になる。

副次効果: §13-7 の P5 目視項目「zCoBo 2 枚が万一 2 接続で来る構成だったら」の**検出器**になる
(`extra_connections` > 0 が即シグナル)。

### 却下した案

- **(b) 多重接続並行 drain**: 実機に対応物がない(1 CoBo = 1 リンク。ELITPC も筐体内で束ねて
  1 本 — SPEC §13-7 v1.11)。同一 CoBo からの 2 接続はプロトコル異常であり、並行 drain は
  フレーム順序・framer 状態・EOS 意味論を複雑化する(KISS 違反)。純正 DataRouter も単一接続。
  多 CoBo は「CoBo 毎に receiver」で既に設計済み。万一 P5 で 2 接続構成が観測されたら
  §13-7 の「要改修」として**別チケット**で再訪(本裁定の extra_connections がその検出器)。
- **(c) 古い接続の追い出し(新規優先)**: ①receiver は「本物 vs 迷い込み」を判定できない —
  ポートスキャンが走行中の本物 run を殺す事故を**新設**してしまう。②追い出し時に旧接続の EOS を
  出す/出さないの判断が TCP EOF = run 境界意味論と衝突(起票時の指摘どおり)。③half-open 復旧
  ケースでも瞬断時点でデータ欠落があり run は作り直しが正道 — seamless 引き継ぎに守る価値がない。
  fail-fast + オペレータの run 再開が正直で単純。

### 見送り(記録)

- **受信側 TCP keepalive**(half-open の自然検出): OS 既定タイマ(≈2 時間)では実効性なし、
  短縮は socket2 依存 + チューニングが要る。可視化で運用上は足りる。なお GET 純正は accepted 側にも
  SO_KEEPALIVE を立てている(TcpConnection.cpp:89-93)。必要になったら別チケット。
- **アイドル閾値警告**: 0 Hz は正当状態(トリガ無し)で閾値の根拠が立たない。判別材料は
  `last_read_unix_ns` + モニタ側の仕事とする。

---

## SPEC 改訂案(**ユーザー承認事項** — 本チケットは文面のみ、SPEC 本文は未変更)

**① §1.3 run 停止シーケンス 1–2 の事実修正**(現行 v1.11 → v1.12 案):

> 1. ecc `stop`(CoBo が送信停止。**実 GET は stop ではデータリンクを close しない** — close は
>    breakup(daqDisconnect)または次の configure の再接続時。20190315_patched
>    `DaqCtrlNodeI::daqStop` / `disconnect` で確認、v1.12)
> 2. receiver: EOF が届いた場合(breakup が先行した場合など)は `EndOfStream` を下流全リンクへ。
>    **実機の通常経路では ecc stop 後に EOF は届かないため、controller の `Stop` コマンドによる
>    強制 EOS が正規経路**(EOF 待ちタイムアウト既定 5 秒、設定可 — v1.12 で「例外経路」から
>    「正規経路」に位置づけを訂正)

**② §1.4 に 6 を追加**:

> 6. **receiver のデータリンクは同時に 1 本(先勝ち、v1.12)**: 接続保持中に到着した余分な接続は
>    accept して即 close し、`extra_connections` としてカウント + 初回 warn(黙って backlog に
>    滞留させない — silent stall 禁止)。**1 バイトも運ばなかった接続の終了(EOF / エラー)は
>    run 境界(EOS)を構成しない** — `empty_connections` としてカウントする(迷い込み接続の
>    即断が偽 EOS で run を閉じるのを防ぐ。§1.3 の強制 EOS 経路は不変)。現接続 peer と
>    最終受信時刻(`last_read_unix_ns`)は GetStatus で可視。
>    根拠: 実機のデータリンクは CoBo が configure 時に張る 1 本のみで、ECC は probe を張らない
>    (TODO/032 調査)。GET 純正 DataRouter も単一接続(確立後は listen 自体を閉じる)であり、
>    本規約はそれを可視化強化した同型。

**③ §12 表に 13 を追加**:

> | 13 | receiver 余分接続 | 現接続でデータ流通中に余分な接続を張っても、現接続のフレーム列は
> バイト一致で無影響 + 余分接続は即 close + `extra_connections` 加算 + warn 1 回。
> 0 バイト接続の終了で EOS が出ない(偽 run 境界なし) |

**④ §13-7 末尾に追記**:

> P5 初日の接続数目視は receiver の `extra_connections` カウンタで機械確認できる
> (run 中 0 のままなら 1 リンク構成の実機確認完了。> 0 なら 2 接続構成 = §13-7 の「要改修」シグナル)。

---

## 発注書(実装ユニット — SPEC 改訂案の承認後に着手)

### やること(`src/receiver.rs`)

1. `drain_task` の内側 read ループの `tokio::select!` に `listener.accept()` 腕を追加
   (biased 順: stop → read → accept。read 優先なので全速受信中は拒絶が遅延し得るが、
   正しさには影響しない — 発注済み挙動として受け入れる)。accept したら即 drop + 
   `Metrics::record_extra_connection(cobo_id, peer)`(**初回だけ warn、以降カウンタのみ** —
   `record_dropped_frame` と同じ「一度だけログ」パターン。warn に peer と現接続 peer を含める)。
   注: `listener` の所有は現行どおり drain タスク。tokio の `accept()` は cancel-safe(文書確認済み)。
2. 接続毎に `conn_bytes` を数え、`Ok(0)` / read エラー時に `conn_bytes == 0` なら EOS を出さず
   `empty_connections` を加算(info ログ)して 'accept へ戻る。`owes_eos` は
   **接続の最初のデータバイトを読んだ時点で true にする**(現行の「accept で無条件 true」をやめる。
   run 開始時の初期値 true は維持 — 無接続 run の Stop 強制 EOS を守る)。
3. `Metrics` に追加: `extra_connections: AtomicU64` / `empty_connections: AtomicU64` /
   `last_read_unix_ns: AtomicU64`(read 成功毎に `unix_nanos()` で更新 — clock_gettime 1 回/read は
   ホットパス許容)/ `peer: std::sync::Mutex<Option<String>>`(書き込みは接続確立時のみ =
   ホットパス外。GetStatus 読みは低頻度)。`json()` に 4 項目追加、`reset()` で全消去、
   「一度だけ warn」ラッチも追加。
4. ログ文言は既存の流儀(「— this is counted, never silent」調)に合わせる。

### テスト(`tests/receiver_stale_link.rs` 新規 + `src/receiver.rs` 単体)

- **T1(030 の再現 = 本丸)**: 接続 A でフレーム流通中に接続 B を張る → B が即 close される
  (B 側 read が EOF/RST)+ A のフレーム列は全数・バイト一致で無影響 + `extra_connections` = 1 +
  EOS は流通中に出ていない。
- **T2**: B を 3 本張る → `extra_connections` = 3、warn は 1 回だけ(単体側で record_* の戻り値照合)。
- **T3**: 0 バイト接続(connect → 即 close)→ `empty_connections` = 1、**EOS が下流に出ない**。
  その後 A が connect → データ → EOF で EOS がちょうど 1 回。
- **T4(後方互換)**: データあり接続の EOF → EOS(既存 `receiver_integration` の意味論不変)。
  無接続 run の `Stop` → 強制 EOS 1 回(owes_eos 経路不変)。
- **T5**: GetStatus metrics に `peer` / `last_read_unix_ns` / `extra_connections` /
  `empty_connections` が載る(Start 前は null / 0)。
- 単体(`#[cfg(test)]`): 新カウンタの reset 挙動 / 一度だけ warn の戻り値(既存テストと同型)。

### 受け入れ

- `cargo fmt && cargo clippy --tests -- -D warnings && cargo test` 全 green(既存テストへの影響ゼロ)。
- T1 が「silent stall しない」ことを機械照合(カウンタ + warn + A 無影響の三点)。
- 完了時は CLAUDE.md 絶対ルールどおり `## 結果` 節(コマンド / green 数 / カウンタ実測値)→
  COMPLETED → archive。

### ファイル所有権・モデル

- **所有権: `src/receiver.rs` + `tests/receiver_stale_link.rs`(新規)の 2 ファイルのみ**。
  `src/controller.rs` / `tests/p3_e2e.rs` は **034 が編集中 — 接触禁止**。SPEC 本文も触らない
  (改訂はユーザー承認後に別途反映)。
- 想定モデル: implementer/**Opus**(select 3 腕の cancel-safety・owes_eos 遷移の工学判断が残る。
  発注書は厳密なので Sonnet 降格も可だが、receiver は保存系の入口 = ロスレスの要なので Opus 推奨)。

---

## Warsaw 前にやること / P5 現地で目視すること

**Warsaw 前(必須)**:
1. SPEC 改訂案①〜④のユーザー承認 → SPEC v1.12 反映。
2. 本発注書の実装(上記)— チケット起票時の「最低ライン (a)」を(a′)として完遂。
3. 034 との整合確認(申し送り節 — 「次 run の前に必ず ecc configure」の経路になっているか)。

**P5 現地(目視・実測)**:
1. **zCoBo 実機の接続数** = run 中 `extra_connections` が 0 のまま(§13-7 の目視の機械化)。
2. **zCoBo 組込みビルドの close 挙動**(本調査で唯一「推定」のまま残る点): sm-stop 後に
   receiver 側で ESTABLISHED が残るか(netstat / GetStatus の peer)、breakup で EOF が届くか。
   VxWorks 版ソースからの推定(stop では close しない)を実機で確定させる。
3. **flowType TCP の実疎通**(§13-4 既載)+ TCP では frameType 7 topology フレームが
   **来ない**こと(FDT 専用 — MemRead.cpp:362-380。mini 実 graw の ctrl 12 B は FDT 運用の痕跡)。
   graw-writer の ctrl/ 出力が空でも正常、とオラクルを読み替える準備をしておく。
4. **S1 復旧リハーサル**: CoBo 電源断 → 再 configure → `extra_connections` が上がって warn が出る →
   run 組み直しで回復、を実機で 1 回踏む(手順として運用メモに残す)。

## 備考

- 030 のテスト側は「fake_ecc にデータリンクを張らないモードを足す」で解決済み(harness の
  artifact)。本ユニットは production 側の可視性と実機異常系への備え。
- 本裁定で「起きない」と判定した正常系閉塞(S3)も、(a′)の拒絶 + カウンタが入れば
  「万一起きたら即見える」体制になる — 判定が外れた場合の保険を含む。
