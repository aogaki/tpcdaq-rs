# 089 — HiVolta DT1415ET / HMP2020 を slowctl `Driver` に載せる(085 コーデックの配線、監視のみ)

**Status: COMPLETED**(2026-09-23 起票 Fable → 実装 Sonnet レーン)

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

## 結果(2026-09-23、Sonnet レーン)

### 実装したファイル
- `src/slowctl/hivolta_driver.rs`(新規): `HiVoltaDriver<T: Read + Write>`(`Driver` 実装)+ `open_tcp` / `open_serial`。
- `src/slowctl/hmp_driver.rs`(新規): `HmpDriver<T: Read + Write>`(`Driver` 実装)+ `open_serial`。
- `src/slowctl/component.rs`: `open_driver` の `"hivolta"` / `"hmp2020"`(旧 "not implemented yet")を実装に配線
  + 経路確認テスト 6 本追加。
- `src/slowctl/fake_hmp.rs`: `FakeHmp::push_response`(要求行ごとの FIFO キュー)を追加(既存 `set_response` は無変更)。
  理由は下記「§1/§2 との食い違い」の HMP 節参照。
- `src/slowctl/mod.rs`: 末尾に `pub mod hivolta_driver;` / `pub mod hmp_driver;` を追加(既存の宣言は無変更)。
- `TODO/089_hv_drivers.md`: 本節・Status 更新。

`hivolta.rs` / `hmp.rs` / `fake_hv.rs`(コーデックと fake 本体)は無変更。

### 単位文字列の一覧
| device | param | unit | 備考 |
|---|---|---|---|
| hivolta | vmon | `V` | |
| hivolta | imon | `µA` | |
| hivolta | status | `bitmask` | 生の 10 進値(f64) |
| hivolta | vset | `V` | 読むだけ |
| hivolta | bdalarm | `bitmask` | node = None |
| hivolta | bdilk | `flag` | `YES`=1.0 / `NO`=0.0、node = None |
| hmp2020 | volt | `V` | node = ch(1..=2) |
| hmp2020 | curr | `A` | |
| hmp2020 | outp | `on` | `1`=ON / `0`=OFF |

### 1 poll の往復数と 1 Hz 成立の見積もり
**HiVolta(TCP 優先パス)**: Ethernet なので往復数によらず 1 Hz は問題なく成立する(桁で余裕)。

**HiVolta(`addr` が無いときの serial フォールバック、9600 8N1)**: 発注書 §1 は 1 ch あたり
`VMON`/`IMON`/`STATUS`/`VSET` の 4 項目のみを列挙しているため、実装はこの 4 項目 + ボード 2 項目
(`BDALARM`/`BDILK`)= **`4×nch+2` 往復**にした(nch=8 の DT1415ET で **34 往復**)。実プロトコル
文字列(例 `$CMD:MON,CH:0,PAR:VMON\r\n` → `#CMD:OK,VAL:1000.00\r\n`)で総バイト数を積むと
**1,509 バイト**(ch あたり 179 B × 8 + ボード 77 B)。9600 8N1 は 1 バイト 10 bit なので
伝送だけで **約 1.57 秒**——デバイス側の処理・ターンアラウンド遅延を一切含まない生の下限値
がすでに 1 Hz の予算(1.0 秒)を超える。**結論: serial フォールバックは 1 Hz では間に合わない**
(TCP 優先のときは無関係。運用は TCP 前提にするか、serial 運用時は poll_hz を下げる/
VSET 等を間引く——発注書 §1 が示唆する緩和を実装するかどうかは §4 の設計判断待ち)。

**HMP2020(serial のみ、9600 8N1)**: `INST:NSEL`×2 + `MEAS:VOLT?`/`MEAS:CURR?`/`OUTP?`×2ch +
`SYST:ERR?`×1 = **9 往復**。実文字列で概算 ~360 バイト → 伝送 **約 0.38 秒**。1 Hz に桁で収まる。

### 発注書との食い違い(§1/§2、実装前に確認できなかったため実装 + 報告で対応)
発注書 §1 は HiVolta の 1 ch あたりパラメータを `VMON`/`IMON`/`STATUS`/`VSET` の **4 項目**とだけ
明記しているが、同じ §1 の見積もり文「1 poll = (5 × nch + 2) 往復」と §2 のテスト文言
「nch = 2 の fake で 12 サンプル」は **1 ch あたり 5 項目**を前提にした数(5×2+2=12。4 項目なら
4×2+2=10)。5 個目のパラメータ名が発注書のどこにも無いため、**§1 の明記された 4 項目のみを実装し
(`4×nch+2` 往復、nch=2 で 10 サンプル)、この食い違いをそのまま報告する**(5 個目を憶測で足すのは
設計判断の代行になるため避けた)。5 個目が必要であれば追加チケットでパラメータ名を指定してほしい。

### 設計判断が必要な分岐(実装していない)
1. **HiVolta port フォールバックの by-id 警告**: 発注書 §1 は「`port`(serial 9600 8N1、
   `open_serial` と同じ by-id 警告)」と書くが、実際の by-id 警告は `config.rs::validate_slowctl`
   が `SLOWCTL_SERIAL_TYPES`(`bronkhorst_flowbus`/`arduino_env`/`hmp2020`/`fake`)に対してのみ
   発火する仕組みで、`"hivolta"` はこの一覧に無い(= `addr` 必須・`port` は任意という現行の検証)。
   `hivolta` を dual-transport(addr **または** port)として正しく検証するには
   `validate_slowctl` の「serial 系は port 必須 / それ以外は addr 必須」という二分法自体を
   変える必要があり、これは発注書が対象ファイルに挙げていない `config.rs` への設計変更になる。
   本チケットでは `component.rs::open_driver` 側のフォールバック配線(addr 優先・無ければ port)
   だけを実装し、`config.rs` の検証ロジックは**変更していない**——現状、TOML 経由では `hivolta`
   に `addr` が必須のままなので、port-only 構成は設定検証で先に弾かれる(直接 `SlowDeviceConfig`
   を組み立てる単体テストでは port フォールバックの経路そのものは確認済み)。要判断: `hivolta`
   を addr-or-port の OR 検証にするか、当面 addr 必須のままにするか。
2. **HMP2020 `INST:NSEL` の無応答疑義**: `INST:NSEL n` は SCPI のクエリ(`?` 終端)ではなく、
   実機が応答を返す保証は無い。`ScpiLink::exchange`(085、変更禁止)は「書いて 1 行読む」の対
   でしか提供されないため、他に手段が無く `exchange` 経由で送り、返信は意味を持たないものとして
   読み捨てる実装にした(fake には空行を登録)。**実機が本当に無応答なら、毎 poll 周期
   `READ_TIMEOUT_MS`(2000 ms)分ブロックして `DriverError::Timeout` になる**——これは
   `hmp_driver.rs` の doc コメントに明記した。書き込み専用の経路を `ScpiLink` に足すかどうかは
   `hmp.rs`(変更禁止指定)への変更を伴うため、実機検証の上で判断してほしい。

### テスト数(before → after)
- `cargo test` 全体: **812 → 829 passed**(0 failed)。新規 17 件:
  - `hivolta_driver::tests`(6): `ident_reads_all_six_board_fields_and_keeps_nch` /
    `ident_serial_mismatch_is_an_identity_error` / `ident_serial_match_succeeds` /
    `poll_of_two_channels_yields_four_samples_each_plus_two_board_samples` /
    `a_loc_err_reply_is_a_protocol_error` / `a_silent_device_times_out`
  - `hmp_driver::tests`(6): `ident_reads_idn_and_keeps_all_four_fields` /
    `ident_serial_mismatch_is_an_identity_error` /
    `poll_of_both_channels_yields_six_samples_and_checks_syst_err_zero` /
    `a_nonzero_syst_err_is_a_protocol_error` / `a_silent_device_times_out` /
    `only_channels_one_and_two_exist_for_the_hmp2020`
  - `component::tests`(6): `open_driver_of_hivolta_with_an_addr_prefers_tcp_and_reports_connection_refused` /
    `open_driver_of_hivolta_without_an_addr_falls_back_to_serial` /
    `open_driver_of_hivolta_without_addr_or_port_is_a_device_error` /
    `open_driver_routes_hmp2020_to_the_hmp_serial_opener` /
    `open_driver_of_hmp2020_without_a_port_is_a_device_error`
  (`fake_hmp` の `push_response` 自体はヘルパー追加のみで専用テストは足していない
  —— `hmp_driver` の poll テストが往復して検証している)。

### 実行コマンド・結果
```
$ git merge --ff-only main          # 1cead35 まで前進(既存 812 passed を確認してから着手)
$ cargo test                        # 812 passed (着手前ベースライン)
$ cargo test                        # 829 passed, 0 failed(実装後)
$ cargo fmt                         # 3 ファイルに整形差分(component.rs / hivolta_driver.rs / hmp_driver.rs)、その後 --check で差分無し
$ cargo clippy --tests -- -D warnings   # 警告 0
$ cargo clippy --all-targets -- -D warnings  # 警告 0(念のため広い方でも確認)
```
production コードに `.unwrap()` / `.expect()` 無し(`#[cfg(test)]` より前を grep して確認)。

### 実機受け入れ(未実施)
CAEN GUI(HiVolta)/ HMP2020 前面表示との 1 分間一致確認は**未実施**。本チケットは fake のみで
縛る前提(発注書 冒頭)。TCP :1470 の実機疎通、USB VCP 9600 8N1 の実機疎通(HiVolta フォールバック
・HMP2020)、`INST:NSEL` 無応答疑義の実機確認(上記「設計判断」§2)はいずれも今後の実機セッション
待ち。
