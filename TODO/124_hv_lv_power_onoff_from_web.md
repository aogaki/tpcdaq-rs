# 124 — HV / LV の ch ごとの on/off を web から(SPEC §11 安全境界の改訂)

**Status: IN PROGRESS**(2026-10-12 起票 Opus。ユーザー「on/off を今から実装できますか?」+ 方針 3 点の回答。設計の確定は Fable)

## ユーザー裁定(2026-10-12、AskUserQuestion の回答。10-09 の方針「web からは on/off と読み出しだけ、上限・ランプは機器側」の具体化)
1. **LV(HMP2020 = AsAd 電源)も web に載せる。run が Running の間の LV OFF は拒否**(先に Stop run)。
2. **HV(DT1415ET)の OFF はいつでも可、ON は run 外(Idle)だけ**(ランプアップ中は偽の事象が出るため)。
3. **操作権は run 制御と同じ操作権(token)+ 押すたびに確認ダイアログ。全操作をログブックに記録**。
- 変えないこと: 電圧値・電流上限・ランプ・トリップ・アラームリセット・ガス・バルブは web に載せない(機器側)。

## 機器のコマンド(マニュアルで確認済み)
- DT1415ET(User Manual rev16 p.25 の SET 表): `$CMD:SET,CH:X,PAR:ON` / `$CMD:SET,CH:X,PAR:OFF`。応答は `#CMD:OK`。
  LOCAL モードでは `#LOC:ERR`(「Command SET with module in LOCAL mode」)。
  REMOTE でも前面の ch スイッチが OFF なら STATUS に DIS(bit 12)が立ち、ON にならない(p.14)= 現場の人による許可。実機は今 REMOTE。
- HMP2020(SCPI マニュアル p.26): `INST:NSEL n` の後、`OUTP ON`(選んだ ch と出力を ON)/ `OUTP OFF`(選んだ ch だけ OFF、出力ボタンは残る)。
  読み戻しは `OUTP?`(1 / 0)と `SYST:ERR?`。

## 設計(契約。3 レーンはこれに従う)
### 1. slowctl: コマンドの受け口(レーン A、Opus)
- 新しい REP ソケット `[slowctl] cmd_bind`(既定 `tcp://127.0.0.1:47007`、localhost のみ)。
- 要求(JSON): `{"op":"power","device":"hv","ch":3,"on":false}`。
- 応答:
  - 成功: `{"ok":true,"device":"hv","ch":3,"on":false,"before":{…},"after":{…}}`
  - 失敗: `{"ok":false,"error":"…","before":{…}|null}`
  - before / after の中身: HV は `{"status":33,"flags":["ON","UNV"]}`、LV は `{"outp":1}`。
- **実行は、その機器の device_loop スレッドで行う**(ポートの持ち主を 1 つに保つ)。
  - 機器ごとに有界のコマンドキュー(深さ 4)を設け、device_loop は poll の合間の sleep をキューの `recv_timeout` に置き換える(コマンドが来たらすぐ起きる)。
  - 応答は oneshot で REP に返す。上限は 5 s。超えたら `{"ok":false,"error":"timeout"}`。
- slowctl 側の検証:
  - device が存在し、type が `hivolta` / `hmp2020` であること。
  - ch の範囲: hivolta 0–7、hmp 1–2。
  - 機器が識別済みであること(未接続ならエラー)。
  - op が既知であること。
  - **run 状態の規則は controller 側で判断する**(slowctl は run を知らない)。
- Driver trait に `fn set_power(&mut self, node: u8, on: bool) -> Result<PowerOutcome, DriverError>` を足す(既定は Unsupported)。
  - **hivolta**:
    - コーデック `hivolta.rs` に `pub fn set_power(ch: u8, on: bool) -> String` を足す(`$CMD:SET,CH:{ch},PAR:ON|OFF\r\n`)。**これがコーデック唯一の SET 生成関数**。モジュール doc の安全境界の記述を更新する。
    - 実行前に STATUS を読む(before)。実行後に STATUS を読み直す(after)。
    - `#LOC:ERR` は「module is in LOCAL mode」のエラーにする。ON の後に DIS が立っていれば「front switch is OFF (DIS)」のエラーにする。
  - **hmp2020**:
    - `hmp.rs` に `pub fn outp_set(on: bool) -> &'static str`(`OUTP ON` / `OUTP OFF`)を足す。
    - 手順: `INST:NSEL n` → `INST:NSEL?` で確認 → `OUTP?`(before)→ `OUTP ON|OFF` → `OUTP?`(after)→ `SYST:ERR?` が 0 であること。
  - HiVolta のシリアルはオンデマンドで開く方式(119)なので、set_power もその中で開いて閉じる。
- slowctl の log(info)に、毎回 device / ch / on / before / after / 結果を出す。

### 2. controller: REST(レーン B、Sonnet)
- `POST /api/slow/power`。本文は `{"token":"…","device":"hv","ch":3,"on":false}`。
  - token が無効なら 401(run 制御と同じ判定)。
  - `[controller] slow_cmd`(既定 `tcp://127.0.0.1:47007`)が使えなければ 503。
  - device の type は daq.toml の `[[slowctl.device]]` から引く。hivolta / hmp2020 以外は 400。
  - **規則**:
    - hivolta の `on=true` は相が `Idle` のときだけ。それ以外は 409「HV ON is allowed only while no run is active」。
    - hmp2020 の `on=false` は相が `Idle` のときだけ。それ以外は 409「LV OFF is refused while a run is active — stop the run first」。
    - hivolta の OFF と hmp2020 の ON はいつでも可。
  - slowctl へ REQ を送る(タイムアウト 10 s、linger 0)。
    - 成功なら 200 と slowctl の応答をそのまま返す。
    - slowctl が `ok:false` なら 502 で error をそのまま返す。
    - 不達・タイムアウトなら 504。
  - **ログブック**: 成否にかかわらず audit `slow/power` を書く(operator、device、ch、on、ok、before、after、error、相)。
- 実装は run 制御の token / 相のロックの扱いに合わせる。規則の判定とその後の転送の間に run/start が割り込んでも、HV ON の検査と送信は同じロック区間で行う(KISS で足りる範囲で)。

### 3. UI(レーン C、Sonnet)
- Slow control の表の ON/OFF 列に、操作できる行(hv = status ビットマスクを持つ、lv = outp を持つ)だけボタン「Turn ON」/「Turn OFF」を出す。
- 操作権が無ければ無効にし、ツールチップで「acquire control in Run control first」と出す。
- 規則で禁止される操作(HV の ON、LV の OFF)は、相が Idle でなければ無効にし、理由をツールチップに出す。
- 押すとページ内の確認ダイアログ(`window.confirm` は使わない)を出す。文言は「Turn OFF hv ch 3?」+ 今の状態 + 規則の注意。
  確定したら POST し、結果(before → after)かサーバの文言をそのまま表示する。
- token は Run control で取ったものを共有する(既存の仕組みを使う)。

## テスト
- A:
  - コーデック: 生成文字列の完全一致。SET は power だけであることを型で保証する(他の SET 関数が無い)。
  - fake HV / HMP: before → after、`#LOC:ERR`、DIS。
  - device_loop: poll とコマンドが交互に来ても直列に実行され、コマンドは poll 周期を待たずに実行されること。
  - REP の要求 / 応答、不正入力(device 無し・ch 範囲外・未知 op)。
- B: 401 / 400 / 409(HV ON @ Running、LV OFF @ Running)/ 200 / 502 / 504、audit が書かれること(偽 slowctl を REP で立てる)。
- C: ボタンの出し分け、無効化と理由、確認ダイアログ、POST の本文、結果の表示。

## 実機の受け入れ(ユーザー立ち会い、ユーザーが選んだ ch で)
- LV ch2(今は OFF・未使用)で ON → OFF。
- HV は、ユーザーが指定した 1 ch で OFF → ON(ランプは機器の設定のまま)。
- 前面スイッチが OFF の ch で ON が拒否されること(DIS)。可能なら確かめる。
- run 中に HV ON / LV OFF が 409 になること。ログブックの audit を確認する。

## SPEC / CLAUDE.md(Fable 確定)
- §11 安全境界 v1.42: 「web が持てる物理的作動は **HV / LV の ch ごとの on/off だけ**(ユーザー裁定 2026-10-12)」を足す。規則・操作権・確認・audit・機器側の許可(REMOTE と前面スイッチ)を明記する。電圧値・上限・ランプ・ガス・バルブは引き続き載せない。
- §3.1 `cmd_bind` / `slow_cmd`、§3.2 に 47007、§8.1 に `POST /api/slow/power`、§9.2 に audit `slow/power`。
- CLAUDE.md の「やらないこと」の該当行を同じ内容に更新する(ユーザーの明示の依頼に基づく)。

## Fable 設計確定(2026-10-12、スポット。上の設計に優先する)
判定: **骨格は承認、以下 7 点を反映して着手**。レーン: **A = Opus、B = Opus**(power_inflight と spawn_blocking の復旧が残る)、**C = Sonnet**。

### A(slowctl)の修正
1. **REP は専用の std スレッド `slowctl-cmd`**(`main_loop` には混ぜない)。`rcvtimeo = DRAIN_TIMEOUT` で `stop` を見ながら回す。
   - 流れ: recv → 静的検証(device 名・type が hivolta / hmp2020・ch 範囲・op)→ 機器の `SyncSender<PowerCommand>` に `try_send` → `reply_rx.recv_timeout(5 s)` → REP に返す。
   - REQ / REP なので**どの経路でも必ず 1 通返す**。send が失敗(EFSM)したらソケットを作り直す。
2. **キューの深さは 1**(`sync_channel(1)`)。満杯なら即 `"device busy"`。
3. **コマンドは device_loop が受ける**。セッション中の sleep に加え、**backoff の sleep でも drain する**。
   - セッション中なら実行する。backoff 中なら即 `{"ok":false,"error":"device not connected"}`。
   - `sleep_until_stop` の `thread::sleep` を `cmd_rx.recv_timeout(DRAIN_TIMEOUT.min(残り))` に置き換える。
4. **コマンドを実行したら、残りの sleep を捨ててすぐ poll する**(表が古いまま残らないように)。
5. **5 s を超えて遅れて届いた結果**は、返し先が無くても `warn!` で device / ch / on / after を出す。
6. **HV**:
   - `OnDemandHiVolta::set_power` の流れは open → `HiVoltaDriver`(nch を引き継ぐ)→ before の STATUS(`CH:X` 単発)→ SET → after の STATUS。スコープの終わりで閉じる。
   - **before に DIS(isdis)が立っていれば SET を送らない**: `"front switch is OFF (DIS)"`。
   - after の検証: ON の要求なら `on()` が真であること(RUP が同時でも可)。OFF の要求なら `!on() || ramp_down()`。満たさなければ `"channel did not switch (status=…)"`。
   - before / after には `status_ignore` を**適用しない**(生の STATUS を返す)。`flags` は ChStatus の既存のビット名を全部並べる。
7. **LV は両 ch を返す**: `{"outp":{"1":1,"2":0}}`(`INST:NSEL 1` → `OUTP?`、`INST:NSEL 2` → `OUTP?`。それぞれ `INST:NSEL?` で確認する)。`OUTP ON` はマスタも ON にするので、ほかの ch が変わったことを隠さない。
8. **`cmd_bind` は opt-in**(省略 = REP を開かない)。
9. `fake_hv` は **`PAR:ON|OFF` だけを受理して内部の status を書き換える**。ほかの SET は従来どおり `#LOC:ERR`。

### B(controller)の修正
1. **`slow_cmd` は opt-in**(省略 = `POST /api/slow/power` は 503)。`ControllerParams` に `slow_cmd: Option<String>` と `slow_devices: Vec<(name, type)>`(`Config.slowctl.device` から)を足す。
2. **「同じロック区間で検査と送信」は却下**(std Mutex を REQ の 10 s のあいだ握れない)。**run/start と同じ手口 = 短いロック + 占有の印**にする。
   - `RunState.power_inflight: Option<PowerInflight{device,ch,on,operator}>` を足す。
   - `slow/power` の流れ(ロック内で順に判定):
     1. power_inflight が Some なら 409 `"another power command is in flight"`。
     2. 規則: HV ON と LV OFF は `phase == Idle` のときだけ。Starting / Stopping / Running はすべて 409。
     3. 通ったら印を立ててロックを放し、`spawn_blocking` で REQ を送る。完了したらロック内で None に戻す。panic からの復旧も run/start と同じ形にする。
   - **run/start**: Idle の検査と同じロック内で、`power_inflight.is_some()` なら 409 `"slow-control power command in flight — retry in a few seconds"`。run/stop は塞がない。
3. 504 / timeout の文言には **「state unknown — check the table」** を入れる。audit には `error:"timeout"` と before を残す。
4. **audit は 401 / 400 / 409 / 503 / 504 を含む全経路で 1 行**。

### C(UI)の修正
- 確認ダイアログには、LV では両 ch の before を出す。結果でほかの ch が変わったら、そのことを明示する。
- Running 中の HV OFF では「ramp-down により run のデータが終わる」という注意を出す。

### テスト・実機の追加
- A:
  - backoff 中のコマンドは即 `device not connected`。
  - timeout の後に遅れて届いた結果を warn する。
  - コマンドの後はすぐ poll する。
  - HMP は両 ch を読み戻す。
  - HV は before の DIS で拒否し、after を検証する。`#LOC:ERR` も扱う。
- B:
  - power_inflight 中の run/start は 409。power の二重投入も 409。
  - HV ON は Starting / Stopping でも 409。
  - `slow_cmd` を省略すると 503。
  - 全経路で audit が 1 行。
- 実機: 既存の 4 項目に加えて、**「LV ch2 を ON にしたとき ch1 の outp が変わらない」**を確かめる。**HV OFF @ Running は実機ではやらない**(データを失う)。B の 200 テストで代わりにする。

### 確定文
SPEC §11 / §3.1 / §3.2 / §8.1 / §9.2 と CLAUDE.md の文案は Fable 作で、Opus が反映済み(v1.42、コミットは本票の起票後)。

## ついで(2026-10-12、ユーザー「ついでに HV のポーリングも 1 秒間隔にしてください」)
- ELITPC の `/opt/tpcdaq/daq.toml` の hv を `poll_s = 5.0` → **1.0** にした(バックアップは `daq.toml.bak-2026-10-12-hv1s`)。slowctl だけを入れ替えた(12:51:51Z)。読むときだけ開く方式はそのまま。
- run 70 の実行中に、60 s ずつトリガレートを測った: **5 s 周期で 4.92 Hz → 1 s 周期で 4.73 Hz**(統計誤差 ≈ ±0.3 Hz の範囲で変化なし。新しいケーブルなので偽トリガは出ない)。
- マニュアル(日英)の「HV 5 秒」を「1 秒」に直した。

## 進捗(2026-10-12)
- 実装:
  - レーン C(Sonnet `91dfc8d` → merge `0fece9a`)
  - レーン B(Opus `6bf3a5f` → merge `692a7ff`)
  - レーン A(Opus `0e9b301` + `ad3de3e`(期限切れは実行しない / LV も after を検証)+ `51e5b10`(SET 後に 100 ms × 最大 10 回読み直す)→ merge `70cffee` / `ed05974`)
  - 各レーンの報告は `git show --stat` とテスト名で照合してから受け入れた(CLAUDE.md の新ルール)。
- ゲート(main `ed05974`、Mac): fmt OK / clippy 0(feature 有無とも)/ **cargo test 992 passed / 0 failed / 1 ignored**(既存)/ UI **349 passed / 5 skipped**、build OK。
- ELITPC への反映(13:10Z):
  - `/opt/tpcdaq/daq.toml` に `cmd_bind` と `slow_cmd` を追加(バックアップは `.bak-2026-10-12-power`)。
  - daq_down / up と UI の差し替え。`power on/off REP bound (… localhost only)`。
- 機器を動かさない確認(Opus、curl):
  - 不正な token → 401。
  - hv ch 8 → 400。
  - 未登録の device → 400。
  - 3 件とも audit に残った(operator、phase Idle)。
- 実機(ユーザーが UI で操作):
  - **HV ch 0–7 の OFF**(16:13–16:14 EEST):
    - 全 ch が RDWN 10 V/s(InfluxDB の vmon の直線当てはめで **−9.55〜−9.62 V/s**)で ≈ 0 V まで下がった。
    - ただし応答は 8 件とも 502 `channel did not switch (status=1)` の**偽陰性**だった。SET 直後の STATUS に DOWN ビットがまだ立っていなかったため。
    - `51e5b10`(読み直し)で修正し、13:23Z に slowctl だけ入れ替えた。ログブックには「実際は切り替わった」旨の comment を残した。
  - **LV ch2 の ON → OFF**(16:17、空き ch): 2 件とも ok。ch1 は before / after とも 1 のままで、**ch2 だけが切り替わった**。
  - **LV ch1 の OFF**(16:23:34、操作者 Soiciro = ユーザーの UI 操作): slowctl を入れ替えている最中に controller が REQ を送り、新しい slowctl の REP(13:23:35.226Z に bind)が受けて実行した(ok、ch1 1 → 0)。
    controller の 10 s 窓の中なので、仕様どおりの動作。**AsAd の電源は今 OFF**(ユーザーに意図の確認を依頼した)。
- **残り**:
  - HV の ON(ランプアップ 2 V/s、ch 7 で ≈ 5 分)の実機確認。修正後の判定で ok になること。
  - 前面スイッチ OFF での DIS 拒否(任意)。
  - run 中に HV ON / LV OFF が 409 になることの実機確認(B のテストで代わりにしている)。
  - SPEC の結果節と archive。
- マニュアル:
  - 最新イベント API を英語タブの付録に取り込み、claude.ai へのリンクを無くした。
  - PDF は日英とも `reference/_manual/ELITPC_DAQ_Manual_{ja,en}_2026-10-12.pdf`(11 / 14 ページ)。

## 実機確認(ユーザー、2026-10-13 申告)と残る不具合
- **全 ch で ON / OFF が動くことを確認済み**(ユーザー)。HV ON のランプアップも含む。
- **不具合: ランプアップ中に OFF にできない**。ボタンを押すと「もう一度 ON にするか」を聞かれる。
  - コード上は、UI のボタンの向き(`ui/src/app/slow/slow-power.ts` の `targetOn` = `row.onOff !== 'ON'`)と、サーバの成功判定(`hivolta_driver.rs` の `switched`)が、どちらも STATUS の bit 0(ON)を見ている。
    ON の SET が成功判定を通った以上、直後の bit 0 は 1 だったはず。
  - 仮説:
    - ① ランプアップ中に DT1415 が bit 0 を落として RUP(bit 1)だけを立てる。
    - ② その間 HV の読み出しが失敗し、古い(OFF の)サンプルのまま。UI は `stale` でも向きを決めている。
  - 確かめ方: ランプアップ中の STATUS の時系列(InfluxDB)と、ログブックの audit `slow/power` の after。ELITPC につながり次第。
  - 直し方の候補(原因が分かってから): ON / RUP / RDW のどれかが立っていれば「Turn OFF」にする(HV の OFF はいつでも可なので安全側)。stale の行はボタンを無効にする。
