# 085 — HiVolta DT1415ET / R&S HMP2020 読み出しコーデック(080 C-3 の前倒し、監視のみ)

**Status: COMPLETED**(2026-09-22 起票 Fable → 同日 Sonnet レーンで実装完了)

## 0. 結論(1 分で)
- 2 機器の**要求文字列の生成と応答の解析**を純関数 + `Read + Write` ジェネリックのリンクとして作る(079 Phase 1 の ProPar と同じ形)。
  `Driver` trait への組み込みは 079 Phase 2 が trait を固めた後の小チケット(本票では触らない)。
- **設定系コマンドは型として存在しない**(SPEC §11 安全境界): DT1415ET の `$CMD:SET,...`、HMP の `VOLT` / `CURR` / `OUTP ON` 等を生成する関数を作らない。

## 1. 一次資料(reference/ は gitignore。worktree に無いので本体の絶対パスで読む。PDF は `pdftotext -layout` で)
- `/Users/aogaki/Workspace/tpcdaq-rs/reference/HiVolta/CAEN_DT1415ET_UserManual_rev16.pdf` — Command Format 節:
  要求 `$CMD:MON,CH:X,PAR:YYY<CR><LF>`、応答 `#CMD:OK,VAL:*<CR><LF>`(単 ch)/ `#CMD:OK,VAL:*;*;*;*`(全 ch。**全 ch 指定の CH 値はマニュアルで確定**して
  結果節に書く)、エラー `#CMD:ERR` / `#VAL:ERR` / `#LOC:ERR` ほか(**全種をマニュアルの表から写す**)。ボード `$CMD:MON,PAR:BDNAME|BDNCH|BDFREL|BDSNUM|BDILK|BDILKM|BDCTR|BDALARM`。
  CH の `STATUS`(XXXXX)と `BDALARM` のビット定義表。トランスポート: TCP :1470 と USB VCP 9600 8N1(同じ ASCII)。**同時 1 クライアント**。
- `/Users/aogaki/Workspace/tpcdaq-rs/reference/HMP2020/HAMEG_HMPseries_SCPI_Programmers_Manual_EN.pdf` — `*IDN?`、`INST:NSEL {1|2}` / `INST:NSEL?`、
  `MEAS:VOLT?` / `MEAS:CURR?`、`OUTP?`、`SYST:ERR?`。終端は LF または CR LF、応答終端 LF。HMP2020 は ch 1–2 のみ。USB CDC(/dev/ttyACM*)。
- `/Users/aogaki/Workspace/tpcdaq-rs/reference/HiVolta/AsAd_PowerSupply_and_Monitoring_1-1.pdf` — AsAd 電源(HMP)の用途。
- 079 §2.1(by-id、open 時の identity 照合: HiVolta は `BDSNUM`、HMP は `*IDN?`)。

## 2. 作るもの
- `src/slowctl/hivolta.rs`:
  `enum ChPar { Vset, Vmon, Iset, Imon, Status, Trip, Pdwn, Rup, Rdwn, ImRange, ... }`(**読み出し用のみ**)、`enum BdPar { Name, Nch, Frel, Snum, Ilk, IlkMode, Ctr, Alarm }`、
  `fn mon_ch(ch: u8, par: ChPar) -> String`、`fn mon_bd(par: BdPar) -> String`、`fn parse_reply(line: &str) -> Result<Vec<String>, HvError>`、
  `HvError { Cmd, Val, Loc, Ch, Par, Malformed(String) }`(マニュアルの全エラー種)、`struct ChStatus(u16)` + ビット述語(`on()`, `ramp_up()`, `ovc()`, `ovv()`, `unv()`, `trip()`, …)、
  `fn parse_status(&str) -> Result<ChStatus>`、`struct BdAlarm(u16)` 同様。`HvLink<S: Read + Write>`: 1 要求 1 応答、行終端 CR LF、`Read` のタイムアウトは呼び手のソケット設定。
- `src/slowctl/hmp.rs`:
  `fn idn() -> &'static str`、`fn nsel(ch: u8) -> String`(1..=2 以外は `Err`)、`fn meas_volt() / meas_curr() / outp_query() / syst_err()`、
  `fn parse_idn(&str) -> Result<Idn { vendor, model, serial, fw }>`、`fn parse_f64(&str)`、`fn parse_bool01(&str)`、`fn parse_syst_err(&str) -> (i32, String)`。
  `ScpiLink<S: Read + Write>`(終端 LF、応答 1 行)。
- `src/slowctl/fake_hv.rs` / `fake_hmp.rs`(cfg(test) + feature `slowctl-fake`): マニュアル実例を返す応答器。生の `$CMD:SET,...` 文字列を流すと `#LOC:ERR` を返す
  (SET は型が無いので**生文字列**で負性テスト)。
- `src/slowctl/mod.rs` に `pub mod hivolta; pub mod hmp;` を足すだけ(既存の型は触らない — 079 Phase 2 と並走中)。

## 3. テスト(ゴールデン = マニュアルの実例を一字も変えない。ページ番号を結果節に書く)
- 要求文字列の完全一致(ch 単体 / 全 ch / ボード)。応答: `#CMD:OK,VAL:...`(単 / 複数 / 小数 / 文字列値 `YES`・`LOCAL`)、全エラー種、
  壊れ行(終端無し・`#` 無し・`VAL:` 無し・値の個数不一致)。
- STATUS / BDALARM: マニュアルの表の各ビットを 1 つずつ立てて述語を検証(表の全行)。
- HMP: `*IDN?` の応答実例、`MEAS:VOLT?` の数値形式、`OUTP?` の `0/1`、`SYST:ERR?` の `0,"No error"`、`INST:NSEL 3` は生成段階で `Err`。
- Link: fake を挟んで 1 往復、空応答(タイムアウト)、CR LF / LF 両終端。

## 4. 受け入れ
`cargo fmt && cargo clippy --tests -- -D warnings && cargo test` green(614 + 増分)。production に `.unwrap()` 無し。
**SET 系の生成関数が存在しない**ことを結果節に明記。実機受け入れ(現地: GUI 表示と 1 分間一致)は未実施と明記。
着手前に worktree で `git merge --ff-only main`(ff できなければ固有コミットが無いことを確認して `git reset --hard main`)。

## 5. 参照
079 §2.1 / §2.5、SPEC §11(v1.29 安全境界)、§3.1(v1.31 `[slowctl]`)、080 C-3、PROPOSAL §R12(HiVolta TCP 同時 1 クライアント)。

## 結果(2026-09-22、Sonnet レーン)

### 実行コマンドとテスト数

```
cargo merge --ff-only main   # 809eb29 → c797792 へ fast-forward、固有コミット無し
cargo test                    # 着手前ベースライン: 614 passed / 0 failed
# 実装後
cargo test slowctl            # lib unittests: 79 passed / 0 failed(Phase 1 の 36 + 本票の 43)
cargo fmt                     # 差分無し(hivolta.rs/hmp.rs/fake_hv.rs/fake_hmp.rs 追加後に実行、整形済み)
cargo clippy --all-targets -- -D warnings          # warning 0
cargo clippy --all-targets --features slowctl-fake -- -D warnings   # warning 0
cargo test                    # 完了後: 657 passed / 0 failed / 0 ignored(新規 +43)
```

新規テスト数の内訳(`grep -c '#\[test\]'`): `hivolta.rs` 22、`hmp.rs` 15、`fake_hv.rs` 3、`fake_hmp.rs` 3 = **43 件追加**。
614 → 657(+43)、赤 0。

新規テスト名(代表、全 43 件のうち主要なもの):
- `hivolta`: `mon_ch_single_channel_matches_manual_template` / `mon_ch_all_channels_uses_numch_as_ch_value` /
  `mon_bd_matches_manual_template` / `parse_reply_single_value` / `parse_reply_all_channels_semicolon_list` /
  `parse_reply_string_value_yes` / `parse_reply_string_value_local` / `parse_reply_bare_ok_has_no_values` /
  `all_error_strings_are_recognized` / `missing_crlf_terminator_is_malformed` / `missing_hash_prefix_is_malformed` /
  `missing_val_prefix_is_malformed` / `empty_value_entry_in_list_is_malformed` /
  `status_bits_match_the_manual_table_row_by_row`(15 ビット全行)/ `status_malformed_value_is_err` /
  `bdalarm_bits_match_the_manual_table_row_by_row`(4 ビット全行)/ `bdalarm_full_mask_from_manual_sets_all_four_bits` /
  `link_exchange_round_trips_through_fake` / `link_exchange_times_out_on_silent_device`
- `hmp`: `idn_matches_manual` / `nsel_matches_manual_example` / `nsel_channel_3_is_err_for_hmp2020` /
  `meas_and_outp_and_syst_err_short_forms` / `parse_idn_matches_manual_example` / `parse_f64_matches_manual_examples` /
  `parse_bool01_matches_manual` / `parse_syst_err_matches_manual_examples` / `link_exchange_round_trips_with_lf_terminator` /
  `link_exchange_round_trips_with_crlf_terminator` / `link_exchange_times_out_on_silent_device`
- `fake_hv`: `registered_response_is_echoed_back` / `raw_set_command_is_rejected_with_loc_err_even_when_unregistered` /
  `silent_device_yields_would_block`
- `fake_hmp`: `registered_response_is_echoed_back` / `unregistered_setter_style_command_gets_no_reply` /
  `silent_device_yields_would_block`

### ゴールデンの出典ページ(`pdftotext -layout` で抽出、scratchpad に保存・リポには入れていない)

- `reference/HiVolta/CAEN_DT1415ET_UserManual_rev16.pdf`(rev.16, 2024-11-12 発行、NPO 00104/16:D1415.MUTx/16):
  - p.22: Command Format(要求/応答書式)、`NUMCH = 8`、`CH = NUMCH command related to all channels (group)`、
    エラー表(`#CMD:ERR` / `#CH:ERR` / `#PAR:ERR` / `#VAL:ERR` / `#LOC:ERR`)、応答 OK 表(`#CMD:OK` /
    `#CMD:OK,VAL:*` / `#CMD:OK,VAL:*;*;*;*`)、チャンネル MON パラメータ表(VSET…IMRES)。
  - p.23: チャンネル MON パラメータ表続き(ISRES…ZCADJ)、"Meaning of STATUS bits"(0–14 の全 15 ビット)。
  - p.24: モジュール MON パラメータ表(BDNAME…BDCFRD0..4)、"Meaning of Board Alarm value"(`0x22C0` マスク、
    bit13/9/7/6 の 4 ビット定義)。
  - p.22 の PDF テキスト抽出は数値付きの実応答例(ワイヤ値そのもの)を含まない(コマンド文字列テンプレートと
    エラー・応答フォーマットの記述のみ)。したがって①要求文字列の golden はマニュアルの `$CMD:MON,CH:X,PAR:YYY`
    テンプレート(X/YYY を実値に置換)と一致させ、②応答値の golden(`503.20` 等)はテンプレート
    `#CMD:OK,VAL:*` に整合する代表値として合成(マニュアルに逐語の数値例が無いことを本注記で明記)。
- `reference/HMP2020/HAMEG_HMPseries_SCPI_Programmers_Manual_EN.pdf`:
  - p.9: §2.1.1 "Long and short form"(短縮形=大文字、完全形=全体。どちらか一方のみ送信可)、
    実例 "MEASure:CURRent? is equivalent to MEAS:CURR?"。
  - p.18: `*IDN?` の応答実例 `HAMEG,HMP4040,055310003,HW50020001/SW2.41`(逐語ゴールデン。型番は
    HMP4040 だがフィールド構造は HMP シリーズ共通 — SCPI manual は HMP2020/2030/4030/4040 共通)。
  - p.20: `SYSTem:ERRor[:NEXT]?` 応答表 — `0, "No error"` / `-100, "Command error"` / `-102, "Syntax error"` /
    `-350, "Queue overflow"`(逐語ゴールデン)。
  - p.22: `INSTrument:NSELect {1|2|3|4}` 実例 `INST:NSEL 1` / `INST:NSEL?, Response: 1`(逐語ゴールデン)、
    NOTICE「HMP2020: :NSELect {3} and :NSELect {4} are not available」(**HMP2020 は ch 1–2 のみをマニュアルで確定**)。
  - p.26: `OUTPut[:STATe]?` 応答値 `1`(ON)/ `0`(OFF)。
  - p.31: `MEASure[:SCALar]:CURRent[:DC]?` 応答例 `e.g. 1.0000`、`MEASure[:SCALar][:VOLTage][:DC]?` 応答例
    `e.g. 1.000`(逐語ゴールデン)。
  - §1.1 NOTICE(pdftotext 行 87 付近、章番号無しの導入部): 「The end character must be set to linefeed (LF) or
    carriage return + linefeed (CR-LF).」— 要求側の終端は LF/CRLF どちらでも可(このコーデックは LF のみ送信、
    KISS)。応答側は `ScpiLink::exchange` が LF/CRLF 両方を剥がす(テストで確認済み)。

### マニュアルで確定した事項

- **DT1415ET の「全 ch 指定」の CH 値 = `NUMCH` = 8**(p.22 "NUMCH = 8" と「CH = NUMCH command related to all
  channels (group)」)。`hivolta::ALL_CH` として定数化(`NUM_CH` と同値)。
- **HMP2020 の有効チャンネルは 1..=2 のみ**(p.22 NOTICE)。`hmp::nsel(3)` / `hmp::nsel(4)` / `hmp::nsel(0)` は
  すべて `Err(HmpError::ChannelOutOfRange)`。
- DT1415ET のエラー種は 5 つ全て実装: `#CMD:ERR` / `#CH:ERR` / `#PAR:ERR` / `#VAL:ERR` / `#LOC:ERR`(p.22)。
- DT1415ET の STATUS ビット定義は p.23 の 15 ビット全行を実装・テスト(`on/ramp_up/ramp_down/ovc/ovv/unv/trip/
  ovp/twn/ovt/kill/intlk/isdis/fail/lock`)。BDALARM は p.24 のマスク `0x22C0` が定義する 4 ビット全行を実装・
  テスト(`trip/ovp/ovt/int_comm_fail`)。
- HMP のコマンド終端は LF または CR LF(p.9 §1.1 NOTICE、逐語引用は上表)。

### 解釈判断(設計分岐ではなく、マニュアルの読み方の注記 — コード内 doc コメントにも記載)

- **BDALARM の読み値の進数**: p.24 の "Meaning of Board Alarm value" 表は値を `0x22C0`(16 進)で提示するが、
  ワイヤ上の実際の読み値が 10 進 ASCII か 16 進 ASCII かは明記が無い。STATUS(p.23、「value read in decimal
  Format」と明記)と同じ `( XXXXX )` プレースホルダ表記を BDALARM(`$CMD:MON,PAR:BDALARM Read out Board Alarm
  status value ( XXXXX )`)も使っていることから、本実装は BDALARM も 10 進 ASCII として `parse_status` と同じ
  復号(`parse_alarm`)を採用した(`hivolta.rs` の `BdAlarm` doc コメント参照)。これは API 形状や trait 分割の
  判断ではなく数値表記の読み取りであり、エスカレーションの対象になる「設計分岐」ではないと判断したが、
  念のためここに明記する。
- **応答の値個数不一致の扱い**: 発注書 §3 の「値の個数不一致」は `parse_reply` が呼び出し側の期待個数を知らない
  (シグネチャに request が無い)ため、期待個数との比較はできない。代わりに `VAL:` のセミコロン区切りに空要素
  が混ざる場合(例 `VAL:0;;0`)を壊れたフレームとして `Malformed` にする実装で解釈した(`empty_value_entry_
  in_list_is_malformed` テスト)。期待個数の突き合わせは呼び出し側(将来の `Driver` 実装)の責務として残した。

### 発注書からの逸脱

- `ChPar` は発注書の例示(`Vset, Vmon, Iset, Imon, Status, Trip, Pdwn, Rup, Rdwn, ImRange, ...`)を「…」の指示
  どおりマニュアル p.22–23 の表**全 37 パラメータ**に展開した(機械的な転記であり抽象化の追加ではない —
  KISS「退屈なコード」の範囲内と判断)。
- それ以外は発注書のシグネチャ・型名・ファイル配置に一致(`mon_ch`/`mon_bd`/`parse_reply`/`HvError`/`ChStatus`/
  `parse_status`/`BdAlarm`/`HvLink<S: Read+Write>`、`idn`/`nsel`/`meas_volt`/`meas_curr`/`outp_query`/`syst_err`/
  `parse_idn`/`Idn{vendor,model,serial,fw}`/`parse_f64`/`parse_bool01`/`parse_syst_err`/`ScpiLink<S: Read+Write>`)。

### SET 系生成関数が存在しないことの確認

- `src/slowctl/hivolta.rs` に `$CMD:SET,...` を生成する関数は無い(`grep -n "fn.*set" hivolta.rs` は `Iset`/
  `Vset` などパラメータ token 名のみヒットし、いずれも `mon_ch`(MONITOR 読み出し)専用。production コードに
  `.unwrap()` 無し)。
- `src/slowctl/hmp.rs` に `VOLT`/`CURR`/`OUTP ON` を生成する関数は無い(`nsel()` は instrument 選択のみで、
  読み出しチェーンの一部として発注書 §2 が明示的に許容している。production コードに `.unwrap()` 無し)。
- `fake_hv.rs` は生の `$CMD:SET,...` 文字列を(登録の有無に関わらず)常に `#LOC:ERR` で拒否することを
  `raw_set_command_is_rejected_with_loc_err_even_when_unregistered` で確認。`fake_hmp.rs` は生の `VOLT 5` の
  ような未登録行に応答しない(タイムアウト)ことを `unregistered_setter_style_command_gets_no_reply` で確認。

### 未実施

- **実機受け入れ**(現地: CAEN 純正 GUI / HMP 前面パネル表示と 1 分間一致)は未実施。本票の範囲は純関数
  コーデック + fake 経由のリンクのみで、TCP/USB シリアルへの実配線・`Driver` trait 実装・component 組み込みは
  別チケット(079 Phase 2 系列、080 C-3)。
- `slowctl-fake` feature を有効にした `cargo clippy --all-targets --features slowctl-fake -- -D warnings` も
  warning 0 で確認済み(上表)。

### 実行環境

- 2026-09-22、worktree `.claude/worktrees/agent-aa3fd837ab134dcd2`(main を ff-only merge 済み、`c797792` 以降)。
- Rust ツールチェーン・OS はリポジトリ既定(Darwin 25.6.0、cargo/clippy/fmt はリポ CI 相当の設定を使用)。
