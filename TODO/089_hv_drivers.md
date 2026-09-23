# 089 — HiVolta DT1415ET / HMP2020 を slowctl `Driver` に載せる(085 コーデックの配線、監視のみ)

**Status: READY**(2026-09-23 起票 Fable。Sonnet 1 日。実機不要 — fake で縛る)

## 0. 前提
- 085 済: `src/slowctl/hivolta.rs`(`mon_ch` / `mon_bd` / `parse_reply` / `parse_status` / `parse_alarm` / `HvLink<T: Read+Write>::exchange`)、
  `src/slowctl/hmp.rs`(`idn` / `nsel` / `meas_volt` / `meas_curr` / `outp_query` / `syst_err` / `parse_idn` / `parse_f64` / `parse_bool01` / `parse_syst_err` /
  `ScpiLink<T>::exchange`)、fake は `fake_hv.rs` / `fake_hmp.rs`。
- 079 Phase 2/3 済: `Driver` trait(`ident` / `poll`)、`Sample { device, node, param, value, unit, ts_ms, alarm }`、`Ident`、`DriverError`、
  `component.rs::open_driver`(`"hivolta"` / `"hmp2020"` は今 "not implemented yet" で明示的に失敗)、`SlowDeviceConfig { port, addr, expect_serial, … }`、
  `bronkhorst.rs::open_serial` / `arduino.rs` の流儀。
- 安全境界(SPEC §11): 書き込み系は作らない。085 の型に SET は無い。
- 着手前に worktree で `git merge --ff-only main`(`79df4b2` 以降)。発注書は本体の絶対パスで読んでよい。

## 1. 作るもの
1. `src/slowctl/hivolta_driver.rs`: `HiVoltaDriver<T: Read + Write>` が `Driver` を実装。
   - `ident`: `BDNAME` / `BDNCH` / `BDFREL` / `BDSNUM` / `BDCTR` / `BDILKM` を読み `Ident { node: None, fields }`。`expect_serial` が空でなければ `BDSNUM` と照合
     (不一致 = `DriverError::Identity`)。`BDNCH` を保持して poll のチャンネル数にする(**8 を焼き込まない**)。
   - `poll`: ch 0..nch について `VMON`[`V`] / `IMON`[`µA`] / `STATUS`[`bitmask`、生値を f64、`alarm = ovc | ovv | unv | trip | ovp | ovt | kill | intlk | fail`]、
     加えて `VSET`[`V`](読むだけ)。ボード: `BDALARM`[`bitmask`、node None、alarm = 非 0]、`BDILK`(`YES` = 1 / `NO` = 0、unit `flag`)。
     `#LOC:ERR` 等のエラー応答は `DriverError::Protocol`(parse_err に数えられる)、無応答は `Timeout`。**全 ch 一括 `CH:8`(NUMCH)は使わない**(nch 可変のため個別 ch)。
   - 1 poll = (5 × nch + 2) 往復。1 Hz で 42 往復は 9600 baud でも間に合うか計算して結果節に(間に合わなければ VSET を 10 poll に 1 回にする等、報告)。
2. `src/slowctl/hmp_driver.rs`: `HmpDriver<T: Read + Write>`(HMP2020、ch 1–2 = `nsel` の Err で自然に縛る)。
   - `ident`: `*IDN?` → `Ident { fields: {vendor, model, serial, fw} }`、`expect_serial` は serial と照合。
   - `poll`: ch 1..=2 で `INST:NSEL n` → `MEAS:VOLT?`[`V`] / `MEAS:CURR?`[`A`] / `OUTP?`[`on`、0/1]。最後に `SYST:ERR?` を 1 回、非 0 なら `Protocol`(文言付き)。
     node = ch。alarm は付けない(HMP に TRIP 概念が無い。閾値は TOML の `alarms` に任せる)。
3. `component.rs::open_driver`: `"hivolta"` = `addr`(TCP、`std::net::TcpStream::connect_timeout` 3 s、read/write timeout 2 s)を優先、無ければ `port`
   (serial 9600 8N1、`open_serial` と同じ by-id 警告)。`"hmp2020"` = `port`(serial、HO720 USB CDC なので baud は名目 9600)。
   どちらも `expect_serial` を渡す。既存の「not implemented yet」分岐は消える。
4. fake は 085 の `FakeHv` / `FakeHmp` を使う(新しい偽物を作らない)。必要なら応答登録の補助関数を fake 側に足す。

## 2. テスト(TDD、赤→緑)
- hivolta_driver: ident(BDSNUM 照合 OK / NG)、poll(nch = 2 の fake で 12 サンプル + BDALARM + BDILK、STATUS ビット → alarm、`#LOC:ERR` → Protocol、無応答 → Timeout)。
- hmp_driver: ident(`*IDN?` 実例)、poll(2 ch × 3 + SYST:ERR 0)、SYST:ERR 非 0 → Protocol、ch 3 は生成段階で Err。
- component: `open_driver("hivolta")` が `addr` 優先で TCP を試みること(接続拒否のエラーになるところまで)、`"hmp2020"` が `port` 経路(arduino のテストの流儀)。
- `cargo fmt && cargo clippy --tests -- -D warnings && cargo test` green(現 812)、production `.unwrap()` 無し。

## 3. 受け入れ・逸脱
結果節に: 単位文字列の一覧、1 poll の往復数と 1 Hz 成立の見積もり、テスト数 before → after、承認が要る逸脱。実機受け入れ(CAEN GUI / HMP 前面表示と
1 分間一致)は未実施と明記。SPEC は変更不要(§3.1 の type 一覧に既出)。設計判断が残る分岐は実装せず報告。
