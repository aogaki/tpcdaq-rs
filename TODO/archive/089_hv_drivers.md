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
**初回実装**:
- `src/slowctl/hivolta_driver.rs`(新規): `HiVoltaDriver<T: Read + Write>`(`Driver` 実装)+ `open_tcp` / `open_serial`。
- `src/slowctl/hmp_driver.rs`(新規): `HmpDriver<T: Read + Write>`(`Driver` 実装)+ `open_serial`。
- `src/slowctl/component.rs`: `open_driver` の `"hivolta"` / `"hmp2020"`(旧 "not implemented yet")を実装に配線
  + 経路確認テスト 5 本追加。
- `src/slowctl/fake_hmp.rs`: `FakeHmp::push_response`(要求行ごとの FIFO キュー)を追加(既存 `set_response` は無変更)。
- `src/slowctl/mod.rs`: 末尾に `pub mod hivolta_driver;` / `pub mod hmp_driver;` を追加(既存の宣言は無変更)。

**レビュー反映(追加コミット、2026-09-23)**:
- `src/config.rs`: `validate_slowctl` に `hivolta` 専用の addr-or-port OR 検証 + テスト 3 本。
- `src/slowctl/hmp.rs`: `ScpiLink::send`(書くだけ)+ `nsel_query()`(`"INST:NSEL?"`)を追加
  (085 の変更禁止はレビューで解除。既存の `exchange` / 既存テストは無変更)+ テスト 3 本。
- `src/slowctl/hmp_driver.rs`: `poll` の `INST:NSEL` を `exchange` から `send` + `nsel_query` 確認
  に差し替え、不一致検知テストを追加。
- `src/slowctl/fake_hmp.rs`: `selected_channel: Option<u8>` の状態と `INST:NSEL`/`INST:NSEL?` の
  特別扱いを追加(明示登録があればそちらを優先)+ テスト 2 本。
- `TODO/089_hv_drivers.md`: 本節を更新。

`hivolta.rs` / `fake_hv.rs`(HiVolta 側のコーデックと fake 本体)は最後まで無変更。

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

**HMP2020(serial のみ、9600 8N1、レビュー反映後)**: ch あたり `INST:NSEL n`(**一方向書き込み、
応答を待たない** `ScpiLink::send`)1 回 + `INST:NSEL?`(選択確認)/`MEAS:VOLT?`/`MEAS:CURR?`/
`OUTP?` の **4 往復**、最後に `SYST:ERR?` 1 往復 = **往復(応答あり)は 9 のまま**(旧設計と同数
——旧設計は `INST:NSEL n` 自体を空応答つきの疑似往復として数えていたが、新設計はそれを
一方向書き込みに変え、代わりに `INST:NSEL?` という本物の往復が増えたので往復数は相殺されて
変わらない)。ただし一方向書き込みが ch あたり 1 回(計 2 回)増えた。実文字列(例
`INST:NSEL 1\n`→(無応答)、`INST:NSEL?\n`→`1\n`、`MEAS:VOLT?\n`→`5.000\n` 等)で総バイト数を
積むと **160 バイト**(ch あたり 68 B × 2 + `SYST:ERR?` 24 B)。9600 8N1 で伝送だけ
**約 0.167 秒**。1 Hz に桁で収まる(レビュー前の見積もり ~0.38 秒より実測が精緻になっただけで
結論は変わらない)。

### 発注書との食い違い(§1/§2、実装前に確認できなかったため実装 + 報告で対応)
発注書 §1 は HiVolta の 1 ch あたりパラメータを `VMON`/`IMON`/`STATUS`/`VSET` の **4 項目**とだけ
明記しているが、同じ §1 の見積もり文「1 poll = (5 × nch + 2) 往復」と §2 のテスト文言
「nch = 2 の fake で 12 サンプル」は **1 ch あたり 5 項目**を前提にした数(5×2+2=12。4 項目なら
4×2+2=10)。5 個目のパラメータ名が発注書のどこにも無いため、**§1 の明記された 4 項目のみを実装し
(`4×nch+2` 往復、nch=2 で 10 サンプル)、この食い違いをそのまま報告する**(5 個目を憶測で足すのは
設計判断の代行になるため避けた)。5 個目が必要であれば追加チケットでパラメータ名を指定してほしい。

### 設計判断が必要な分岐 → レビュー裁定により実装済み(2026-09-23、追加コミット)
初回実装では以下 2 点を「設計判断が残る」として実装せず報告のみに留めていたが、レビュー
(2026-09-23)で裁定が下ったため追加コミットで実装した。

1. **HiVolta の `config.rs::validate_slowctl`**: `hivolta` は `addr` と `port` の**どちらか
   1 つ以上**を必須にするよう変更(両方あれば `component.rs::open_driver` は addr 優先、
   従来どおり)。他の型の「serial 系は port 必須 / それ以外は addr 必須」という二分法は崩さず
   (`SLOWCTL_SERIAL_TYPES` は無変更)、`device.device_type == "hivolta"` だけ特別扱いの OR 検証
   にした。エラー文言は `"needs an \`addr\` (TCP :1470) or \`port\` (USB VCP)"`。
   単体テスト 3 本(`config::tests`): `a_hivolta_device_with_only_an_addr_is_accepted` /
   `a_hivolta_device_with_only_a_port_is_accepted` /
   `a_hivolta_device_without_addr_or_port_is_a_startup_failure`。
2. **HMP2020 の `INST:NSEL`**: `src/slowctl/hmp.rs` の `ScpiLink` に `send(&mut self, cmd: &str)
   -> Result<(), LinkError>`(書くだけ、応答を読まない)を追加(085 の変更禁止はレビューで解除、
   既存の `exchange` とテスト済みの fake 挙動は変えていない)。併せて `nsel_query() -> &'static str`
   (`"INST:NSEL?"`)も追加(**実機にこのクエリが存在するかは未確認** —— `hmp.rs` の doc コメント
   に明記)。`HmpDriver::poll` は ch ごとに `send("INST:NSEL n")` → `exchange("INST:NSEL?")` で
   選択を確認し、返りが `n` と一致しなければ `DriverError::Protocol`(黙って別 ch の値を読まない)。
   `fake_hmp.rs` に `selected_channel: Option<u8>` の状態を持たせ、`INST:NSEL n`(非クエリ、応答
   無し)で更新、`INST:NSEL?` はその状態を読み戻す(明示 `push_response`/`set_response` の登録が
   あればそちらを優先 —— テストで「選択が効かなかった」場合を模せるように)。
   単体テスト: `hmp.rs`(`nsel_query_matches_the_nsel_setter_form` /
   `send_writes_without_waiting_for_a_reply` / `send_then_nsel_query_confirms_the_selected_channel`)、
   `fake_hmp.rs`(`a_non_query_nsel_command_gets_no_reply` /
   `nsel_query_echoes_back_the_last_selected_channel`)、`hmp_driver.rs`
   (`a_selection_that_did_not_take_effect_is_a_protocol_error` — 新規の不一致検知経路)。
   既存の poll テストは `register_nsel_and_measurements` → `register_measurements`(NSEL の
   固定応答登録が不要になったため)にリネームして簡素化。
   **実機で `INST:NSEL?` が本当に存在するかは依然未確認**(実機受け入れの範囲、下記参照)。

### テスト数(before → after)
**初回実装(812 → 829)**: 新規 17 件 —— `hivolta_driver::tests`(6)、`hmp_driver::tests`(6)、
`component::tests`(5、経路確認)。詳細は git log 参照。

**レビュー反映の追加コミット(829 → 838)**: 新規 9 件:
- `config::tests`(3): `a_hivolta_device_with_only_an_addr_is_accepted` /
  `a_hivolta_device_with_only_a_port_is_accepted` /
  `a_hivolta_device_without_addr_or_port_is_a_startup_failure`
- `hmp::tests`(3): `nsel_query_matches_the_nsel_setter_form` /
  `send_writes_without_waiting_for_a_reply` /
  `send_then_nsel_query_confirms_the_selected_channel`
- `fake_hmp::tests`(2): `a_non_query_nsel_command_gets_no_reply` /
  `nsel_query_echoes_back_the_last_selected_channel`
- `hmp_driver::tests`(1): `a_selection_that_did_not_take_effect_is_a_protocol_error`

**cargo test 全体: 812 → 838 passed(0 failed)。**

### 実行コマンド・結果
```
$ git merge --ff-only main          # 1cead35 まで前進(既存 812 passed を確認してから着手)
$ cargo test                        # 812 passed (着手前ベースライン)
$ cargo test                        # 829 passed, 0 failed(初回実装後)
$ cargo fmt && cargo clippy --tests -- -D warnings && cargo clippy --all-targets -- -D warnings  # 警告 0
# --- レビュー反映(config.rs OR 検証 + hmp.rs send/nsel_query) ---
$ cargo test                        # 838 passed, 0 failed(レビュー反映後)
$ cargo fmt                         # 差分無し(--check も通過)
$ cargo clippy --tests -- -D warnings        # 警告 0
$ cargo clippy --all-targets -- -D warnings  # 警告 0
```
production コードに `.unwrap()` / `.expect()` 無し(`#[cfg(test)]` より前を grep して確認、
追加コミット分も含めて再確認済み)。

### 実機受け入れ(未実施)
CAEN GUI(HiVolta)/ HMP2020 前面表示との 1 分間一致確認は**未実施**。本チケットは fake のみで
縛る前提(発注書 冒頭)。TCP :1470 の実機疎通、USB VCP 9600 8N1 の実機疎通(HiVolta フォールバック
・HMP2020)はいずれも今後の実機セッション待ち。**特に `INST:NSEL?` が HMP2020 実機に実在するか
は完全に未確認**(SCPI 一般の類推のみで実装した——`hmp.rs::nsel_query` の doc コメント参照)。
実機で `INST:NSEL?` が `#CMD:ERR` 相当のエラーを返す、または無応答なら、`HmpDriver::poll` は
その ch で `DriverError::Protocol`(または `Timeout`)を返して停止する——silent に別 ch の値を
読むことは無いが、実機セッションでの確認は必須。

### Fable レビュー(2026-09-23、マージ `worktree-agent-af04083a470b676fe` → main `b6f27df`)
- 初回の報告 2 点を裁定して実装させた: ①config の `hivolta` は `addr` or `port`(両方あれば addr 優先)②HMP の `INST:NSEL n` は `ScpiLink::send`(書くだけ)+
  `INST:NSEL?` で選択確認(不一致は Protocol)。発注書の「5 項目」は誤記で 4 項目(VMON / IMON / STATUS / VSET)が正。serial フォールバック時の 1 poll 1.57 s は許容
  (TCP が既定)。書き込み系の経路なし(grep で確認)。
- 残 = 実機(CAEN GUI / HMP 前面表示と 1 分間一致、`INST:NSEL?` の実在確認)。

