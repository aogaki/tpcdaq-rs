# 085 — HiVolta DT1415ET / R&S HMP2020 読み出しコーデック(080 C-3 の前倒し、監視のみ)

**Status: READY**(2026-09-22 起票 Fable。実機不要 — マニュアルの実例をゴールデンにする。Sonnet 1 日)

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
