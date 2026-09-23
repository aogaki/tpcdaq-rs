# 079 — スローコントロール: ガス系(Bronkhorst FLOW-BUS: EL-FLOW MFC + EL-PRESS ×2)+ 環境(Arduino MKR1000 + ENV shield)

**Status: READY**(2026-09-15 起票、Fable。同日 写真 5 枚(reference/Bronkhorst/IMG_3204–3208)で
機種系列・接続形態が判明、Arduino は「温度を取るだけなので自前で書く」とユーザー裁定 → 着手条件
「機種名などが判明してから」を満たしたので READY。**着手はユーザー指示待ち**。残る確認(§3)は
着手を止めない — デフォルトで進めて現地 Phase 0 で確定する)

## 0. 結論(1 分で)

- **ガス系は Bronkhorst の FLOW-BUS ネットワーク**(RJ45 の「Ethernet ケーブル」= FLOW-BUS の
  RS485 + 15–24 V 給電、Y アダプタ 7.03.241 でカスケード)。PC は **USB–RS232 変換 → 先頭機器の
  9 ピン D-sub(ProPar ASCII 38400 8N1)** に入り、その機器が **local host** として FLOW-BUS 上の
  全ノードへ中継する(9.17.024 §1.1 / §2 例 7 「Local host」、推奨 ≤ 10 台)。→ **DAQ 機からは
  シリアルポート 1 本で 3 台すべて読める。** ベンダーソフト不要、Rust `serialport` + 数百行。
- **機器(写真)**: ① EL-FLOW Select 質量流量計/制御器「MFC 1」 ② EL-PRESS 圧力計/制御器
  「BPC NEW / PT-TPC」(BPC = back pressure controller → P-700 系) ③ EL-PRESS「TPC」(FLOW-BUS
  ラベル、ノードアドレスのロータリ SW 付き)。**model key はノードから読む**(113/2 等、§2.2)ので
  銘板の転記は不要。**圧の正はこの EL-PRESS**(±0.5 % FS 級で十分 — ガスエンジニア裁定、§6)。
- **受け皿は SPEC に既にある**: `slowctl`(§1.1 v1.29、旧 psu。source_id 200、logbook レコード、§11
  トレンド)。**読み取り・記録・アラームのみ**(§1.5 安全境界、裁定済み)。
- **物理側の効き目**: 圧力は v_drift ∝ E/p の分母。run 毎の圧力を logbook に残せば analyzeUVW.C の
  `PRESSURE_MBAR` 定数を run 記録から引ける(今は手打ち)。流量(MFC)はガス交換の健全性の指標。
- 見積もり ≈ **1.5 週間 + 現地 Phase 0 半日**(§4)。

## 1. 資料と現物

### 1.1 写真からの機器特定(reference/Bronkhorst/IMG_3204–3208、2026-09-15)
| 写真 | 機器 | 読めたこと |
|---|---|---|
| 3204 | **EL-FLOW Select** mass flow meter/controller、手書きタグ「MFC 1」 | 側面 9 ピン D-sub にケーブル、上面 RJ45(FLOW-BUS)。ラボ型 MBC。Identification number は 7(DMFC)のはず |
| 3205 | **Bronkhorst 7.03.241 Modular Y-adapter cable** | FLOW-BUS の RJ45 二又(9.17.024 §2.3.1)。これが「カスケード」の実体 |
| 3206 | **EL-PRESS**「BPC NEW / PT-TPC」+ 後方に **EL-PRESS**「TPC」(FLOW-BUS ラベル、`ADDRESS 00 = SOFT-ADDRESS` の LSD/MSD ロータリ SW)、Y アダプタ ×2 | BPC = back pressure controller(P-700 系、9.17.101 §1.3.3)。「TPC」は計測のみ(P-500)か制御器かは 113/12 で判別。左の黒赤の箱 = **Pfeiffer ActiveLine ゲージヘッド**(ユーザー確認 2026-09-15。黒筐体・赤天面、Hirschmann ロック螺子付コネクタ、ピン押し「ADJ」+ 16 進ロータリ「CAL」。系列は §6 の判別手順で確定) |
| 3207 | ラック上の他機器: NTRON Microx O₂ 計(322 ppm)、Michell Easidew 露点計(−61.9 °Cdp)、青表示「10.0 mBar」パネル(機種不明)、Agilent AGD-100 真空ゲージ表示(タグ VPT 101) | **本チケットの対象外**(候補として §6 に記録) |
| 3208 | **Pfeiffer Vacuum DualGauge**(前面 USB → TPG 36x 系) | 対象外(§6) |

### 1.2 接続トポロジ(ユーザー申告 2026-09-15 + 9.17.024 / 9.17.101 で裏取り)
```
DAQ 機 --USB--> USB-RS232(9.09.122 相当) --T-part 7.03.366--> [先頭機器 D-sub: RS232/ProPar 38400 8N1]
                                                                    | 先頭機器 = FLOW-BUS "local host"
                                                                    RJ45 --Y 7.03.241-- RJ45 --Y-- RJ45 ... (RS485 187.5 kbaud + 給電)
                                                                    MFC1 / BPC / TPC(各ノード 3..120)、末端に終端(黒)
```
- **ノード指定**: RS232 直結の先頭機器は **node 128** で必ず応答(9.17.027 §3.2、実アドレス非依存)。
  他機は **FLOW-BUS ノードアドレス**(3…120、ロータリ 00 = ソフト設定、既定 3)で指定 —
  local host が中継する。FLOW-BUS は **マルチマスタ・トークンリング**(9.17.024 §2)。
- **罠 3 つ**(9.17.101 §3.6.2): (a) D-sub が RS485(FLOW-BUS/Modbus)側に設定されていると RS232 に
  **応答しない** → 多機能スイッチ 12–16 s で configuration mode(RS232 38400)。(b) RS232 は **10 m 制限**
  (38400 baud)。(c) RJ45 チェーンは**給電も兼ねる**(コネクタ定格 1.5 A)— 抜き差しで他機が落ちる。
- **未確定(現地 Phase 0 で決める)**: どの機器が先頭(D-sub にケーブルが刺さっているのは写真では MFC1 と
  BPC の 2 台)、各ノード番号、**いまその RS232 の相手はどの PC か**(FlowSuite/FlowPlot の Windows
  ノート?)→ §1.5 の単一マスタ問題の解き方が決まる。

### 1.3 Arduino(1 MB)
- **MKR1000**(SAMD21 + WiFi)+ **MKR ENV Shield ASX00011**(温度・湿度・気圧・照度・UV)。
  ELI-NP の A. State → T. Roman のメール(2021-11-16)で購入(cleste.ro)。
- 現行 = LabVIEW 製 Windows .exe(`MKR1000 + ENV Shield.exe`)、スケッチのソース無し。
  **ユーザー裁定 2026-09-15: 温度を取るだけなので自前スケッチで置き換える**(§2.3)。

### 1.4 マニュアル(reference/Bronkhorst、gitignore)
- 9.17.101 EL-PRESS(§1.3 系列、§2.8 RS232 接続部品、§3.6 通信、§3.7 パラメータ)、9.17.027 RS232/ProPar
  (§3 プロトコル、§3.9 実例 = ゴールデン、付録 FlowDDE 値表)、9.17.024 FLOW-BUS(§2 配線・終端・
  local host、§4 アドレス)、9.17.023 汎用デジタル計器(**§4–§8 パラメータ表 = process/param の正**)、
  9.17.011 RS232/FLOW-BUS interface box(今回は不使用)。
- ログブック 5 年分に Bronkhorst の記載は無し(CO₂ 130 mbar **流通運転**、NIM 電源投入が圧力計に干渉した
  事例 p.352–353 のみ = 071 SURVEY)。

## 1.5 安全境界(**裁定済み 2026-09-15、SPEC v1.29 §11**)

- **web 操作面(controller REST / UI)= DAQ の run 制御と監視のみ。物理的作動(HV / ガス圧 setpoint /
  流量 setpoint / バルブ)は web に経路を作らない。** DAQ 統合は**モニタ(読み取り・記録・アラーム)のみ**。
- **衝突点 = RS232 ProPar のマスタは 1 本**(先頭機器の D-sub 1 口)。解は 2 つ、どちらも web には載せない:
  1. **制御は既存手段**(FlowSuite の Windows ノート等)、DAQ は読むだけ。**FLOW-BUS はマルチマスタ**
     なので、DAQ 用に**別の機器の D-sub を第 2 の local host** として使えば Windows 側を抜かずに済む
     可能性がある(9.17.024 は禁止していない — **現地 Phase 0 で実証**。ダメなら運用ルール「FlowSuite を
     繋ぐ間は slowctl を止める」)。
  2. **slowctl がポートを専有**し、setpoint は **Unix ドメインソケット / 127.0.0.1 の CLI** からだけ。
- HV(P6)も同じ規則。

## 2. 設計(読み取りのみ)

### 2.1 component `slowctl`(Rust / tokio、SPEC §1.1)
- TOML `[[slowctl.device]]`: `type = "bronkhorst_flowbus" | "arduino_env" | "hivolta" | "hmp2020"`、
  `port = "/dev/ttyUSB0"`(シリアル系: Bronkhorst / Arduino / **HMP2020 = HO720 USB CDC、LAN 無し — 2026-09-15 現物確認**)
  or `addr = "192.168.x.y:1470"`(HiVolta TCP)、`poll_hz = 1`、`nodes = "scan" | [3, 4, 5]`、`alarms = [{node, channel, lo, hi}]`。
  **シリアルポートは udev で固定名**(`/dev/serial/by-id/…`)にして取り違えを防ぐ(DAQ 機に USB シリアルが 3 本並ぶ)。
- **ポートの取り違え対策(設計既定、2026-09-15)**: ①TOML は `/dev/ttyUSBn` を書かず `/dev/serial/by-id/`(個体シリアル
  番号入りの symlink、Ubuntu 標準 udev が自動生成)を書く。同型で個体シリアルが無いアダプタ(安物 PL2303 等)は
  `/dev/serial/by-path/`(USB の物理ポート位置で固定)にする。②**open 時に必ず機器に名乗らせて照合**する —
  ProPar は node 128 に 113/3 serial + 113/12 種別、SCPI は `*IDN?`、Arduino は JSON 行の `"dev":"mkr_env"` — 期待した
  型でなければ **エラーとして数えて再試行**(silent 禁止)。これで列挙順が変わっても誤読しない。
- **HiVolta は Ethernet(TCP :1470)を既定**(USB は仮想 COM 9600 baud も可だが、①ケーブル長・ハブに縛られず
  ②シリアルの本数を増やさず ③DAQ 機以外からも読めるので Ethernet を採る)。**USB は CAEN 純正 GUI が TCP を
  専有するときの逃げ道**として残す(DT1415ET 取説: TCP は同時 1 クライアント想定 — PROPOSAL §R12)。
- ドライバ trait: `poll() -> Result<Vec<Sample{device, node, channel, value, unit, ts}>>`。読み出し失敗・
  CRC/フォーマット違反・タイムアウトは**数えて可視化**(silent 禁止)、再接続はドライバ内。
- 出力 3 系統(すべて既存の口): (a) **logbook**(§9): `psu` レコード型を **`slow`** に一般化
  (`device / node / channel / event / values`)。1 Hz 全サンプルは書かない — **変化閾値 or 60 s 毎 +
  ALARM/RECOVER**。(b) **status PUB**(source_id 200、§3.2)→ monitor → WS(status JSON に `slow: {...}`、
  新バイナリ型は作らない)→ UI トレンド(ECharts、§11)。(c) **run_start / run_stop** に直近値と run 平均
  (`gas_pressure_mbar`、`gas_flow_mln_min` 等 — 単位は機器から読んだ文字列をそのまま)。
- アラーム: `[lo, hi]` 逸脱 + 機器の Alarm info(1/20)≠ 0 で `event: "ALARM"` + status バッジ。
  **DAQ は止めない**(既定。§3-3)。

### 2.2 Bronkhorst ドライバ(ProPar ASCII、local host 経由で全ノード)
- **起動時 node scan**: 128(先頭)+ 3…120 を順に ping(Identification number 読み)。応答したノードごとに
  下表の識別群を読み、logbook `slow` の `event: "IDENT"` に残す。以後 1 Hz で計測群を**チェーン要求
  1 本/ノード**(9.17.027 §3.7 chaining)。
- パラメータ(**9.17.023 §4–§8 で照合済み**、process/param、FlowDDE 番号は参考):

| 名前 | proc/par | 型 | DDE | 用途 |
|---|---|---|---|---|
| Identification number | 113/12 | u8 | 175 | 7 DMFC / 8 DMFM / 9 DEPC / 10 DEPM — 機器種別の判定 |
| BHT model number | 113/2 | string | 91 | model key(例 `F-201CV-5K0-AAD-33-V`、9.17.027 §3.9) |
| Serial number | 113/3 | string[20] | 92 | 識別 |
| Firmware version | 113/5 | string[6] | 105 | 識別 |
| User tag | 113/6 | string[16] | 115 | 現場名(「MFC 1」等が入っていれば拾う) |
| Capacity 100 % | 1/13 | float | 21 | レンジ(焼き込み禁止) |
| Capacity unit | 1/31 | string[7] | 129 | 単位文字列(`mbar` / `mln/min` …) |
| Fluid name | 1/17 | string[10] | 25 | 較正ガス(CO₂ か) |
| Sensor type | 1/14 | u8 | 22 | 0 圧力(制御器)/ 128 圧力(センサ)… |
| **fMeasure** | 33/0 | float | 205 | **計測値(工学単位 = Capacity unit)** |
| fSetpoint | 33/3 | float | 206 | 現在の設定値(読むだけ) |
| Measure / Setpoint | 1/0, 1/1 | u16(32000 = 100 %) | 8, 9 | 整数版(照合用) |
| Valve output | 114/1 | u32 | 55 | バルブ開度(制御器のみ) |
| Temperature | 33/7 | float | 142 | センサ温度(MBC3) |
| Alarm info | 1/20 | u8 bitmask | 28 | 0x04 min / 0x08 max / 0x80 HW 等 |
| Control mode | 1/4 | u8 | 12 | 読むだけ(0 = bus setpoint, 3 = valve closed …) |
- **書かない**: Setpoint / fSetpoint / Control mode / `B` secured 群。ドライバの型で書き込み API を持たない。
- テスト = **pty 上の fake ProPar 応答器**(9.17.027 §3.9 の実例を hex のままゴールデンに、エラー `:01xx`、
  タイムアウト、node scan で無応答ノードを飛ばす)。実機受け入れ = FlowSuite / 前面表示と 1 分間一致。
- 参考実装: Bronkhorst 公式の Python `bronkhorst-propar`(PyPI)。**Phase 0 の現地 node scan と
  ゴールデン採取に使う**(コードは持ち込まない — ライセンスと codec の食い違いはテストで検出)。

### 2.3 Arduino ドライバ + スケッチ(自前、裁定済み)
- `tools/arduino_env/env_json.ino`(Arduino_MKRENV ライブラリ)が USB シリアル 115200 に **JSON 1 行/秒**
  (`{"t_c":..,"rh":..,"p_hpa":..,"lux":..,"uva":..,"uvb":..}`)。WiFi は使わない(KISS)。
- ドライバは行を読んで Sample に変換するだけ。フォーマット違反は数える。

## 2.5 Phase 1 発注(ProPar codec + fake + node scan — Sonnet、2026-09-17 着手)

**範囲 = 純粋なコーデックと同期リンク。シリアルポートも tokio も component 配線も入れない**(Phase 2)。

- `src/slowctl/mod.rs`(`pub mod propar; pub mod link;`)+ `src/lib.rs` に `pub mod slowctl;`。
- `src/slowctl/propar.rs` — **IO 無し**の純関数:
  - `Request { node: u8, items: Vec<ReadItem{process:u8, param:u8, ty: ParType}> }` → ASCII 行
    (`:` + 2 桁 hex ×N + `\r\n`。長さ・node・command 04(read)・chaining のビット規則は 9.17.027 §3.5–3.7)。
    書き込み(command 01)は**作らない**(型として存在させない — §1.5)。
  - `parse_reply(line) -> Result<Reply, ProParError>`: `:01xx` エラー(コード表 §3.6)、length 不一致、
    hex 不正、node 不一致は全て `Err`(silent 禁止)。値の型 = u8 / u16 / u32 / f32(IEEE、big-endian)/
    string(長さ付き)。**chaining の複数値**を順に返す。
  - 便利関数: `ident_request(node)`(113/12 u8, 113/2 string, 113/3 string, 113/5 string, 113/6 string)、
    `capacity_request(node)`(1/13 f32, 1/31 string, 1/17 string, 1/14 u8)、`measure_request(node)`
    (33/0 f32, 33/3 f32, 1/0 u16, 1/1 u16, 114/1 u32, 33/7 f32, 1/20 u8, 1/4 u8)。
- `src/slowctl/link.rs` — 同期リンク `ProParLink<T: Read + Write>`: `exchange(&Request) -> Result<Reply>`
  (1 行送って `\r\n` まで読む。`T` のタイムアウトは呼び出し側が設定 — 後で `serialport` がそのまま入る)、
  `scan(nodes: impl Iterator<Item=u8>) -> Vec<(u8, Ident)>`(無応答 = タイムアウト `Err` はスキップして数える)。
  カウンタ `LinkStats { ok, timeout, malformed, node_mismatch }` を持ち、`Display` で 1 行に出す。
- **fake** `src/slowctl/fake.rs`(`#[cfg(any(test, feature = "slowctl-fake"))]`): `FakeProPar` = `Read + Write`
  を実装するインメモリ応答器。ノード表 `{node → {(process,param) → 値}}` を持ち、`node 128` は先頭ノードとして
  応答、未知ノードは無応答(EOF ではなく `WouldBlock`/タイムアウト相当)、故意のエラー応答・壊れ行・遅延を
  設定できる。
- **テスト(TDD、`cargo test slowctl`)**: ①9.17.027 §3.9 の実例をそのまま hex ゴールデンに(reference/Bronkhorst/
  917027-Manual-RS232-interface.pdf を `pdftotext -layout` で読む。serial number / capacity unit / fluid name /
  measure の例。**文面の hex 文字列を一字も変えずに定数化**)②エラー表 `:01xx` 全コード ③float の
  big-endian 復号(手計算値)④chaining の 3 値 ⑤fake 経由の node scan(3 ノード + 無応答 2 ノード、
  カウンタの値まで断定)⑥壊れ行(長さ矛盾、奇数 hex、CR 無し)は `Err` で、カウンタ `malformed` が増える。
- **やらないこと**: 書き込みコマンド、serialport 依存の追加、slowctl コンポーネント本体、TOML、logbook。
- 完了条件: `cargo fmt && cargo clippy --tests -- -D warnings && cargo test` green、テスト数の増分を結果節に。

## 2.6 Phase 2 発注(slowctl 骨格 — Opus、2026-09-22 着手)

前提: Phase 1 済(`src/slowctl/{propar,link,fake}.rs`、main `cargo test` 614 passed)。**契約 = SPEC v1.31**(§2.3 slowctl→monitor PUB、
§3.1 `[slowctl]` 例、§3.2 ポート 47006 / source_id 200、§9.2 `slow` レコード + run_start/run_stop の `slow`、§10.3 status の `slow`)。
着手前に worktree で `git merge --ff-only main`(ff できなければ固有コミットが無いことを確認して `git reset --hard main`)。
発注書・SPEC は本体ツリーの絶対パス(`/Users/aogaki/Workspace/tpcdaq-rs/TODO/079_slow_control_gas_env.md`、`docs/SPEC_ja.md`)で読んでよい。
reference/ は worktree に無い(本票では不要)。

### 作るもの
1. `src/slowctl/mod.rs` — **ドメイン型(IO 非依存、tokio 非依存)**:
   - `Sample { device: String, node: Option<u8>, param: String, value: f64, unit: String, ts_ms: u64, alarm: bool }`
   - `Ident { device: String, node: Option<u8>, fields: BTreeMap<String, String> }`
   - `trait Driver { fn ident(&mut self) -> Result<Vec<Ident>, DriverError>; fn poll(&mut self) -> Result<Vec<Sample>, DriverError>; }`
     同期(ブロッキング)。`DriverError { Io(std::io::Error), Timeout, Protocol(String), Identity { expected: String, got: String } }`。
   - `AlarmRule { node: Option<u8>, param: String, lo: f64, hi: f64 }` と判定の純関数(前回状態 + Sample → `Some(Alarm)` / `Some(Recover)` / `None`)。
   - `LogPolicy`(純関数): 前回書いた値から `|Δ| ≥ 2 %`(0 近傍は絶対値 `1e-9`)か `log_every_s` 経過で VALUE を書く。
2. `src/slowctl/bronkhorst.rs` — `Driver` 実装。`ProParLink<Box<dyn serialport::SerialPort>>`(Phase 1 の入口そのまま)。
   `ident`: `nodes = "scan"` → 128 + 3..=120 を順に、`[..]` → 指定のみ。応答ノードごとに 113/12, 113/2, 113/3, 113/5, 113/6, 1/13, 1/31, 1/17, 1/14。
   `poll`: ノード毎に 33/0 fmeasure・33/3 fsetpoint・33/7 temperature・1/20 alarm_info・114/1 valve_output(sensor type が制御器のときだけ)。
   単位 = 1/31 の文字列(fmeasure/fsetpoint)、`degC`(temperature)、`%`(valve、生 u32 を % に直さず生値 + unit "raw" でよい — 換算式は現地で確定)。
   `expect_serial` が空でなければ 113/3 と照合、不一致は `DriverError::Identity`。**書き込みは型として存在しない**(Phase 1 と同じ)。
   依存追加: `serialport = { version = "4", default-features = false }`(libudev 不要。by-id の解決は OS の symlink で足りる)。
3. `src/slowctl/fake_device.rs`(cfg(test) + feature `slowctl-fake`)— `Driver` を **Phase 1 の FakeProPar を in-memory の双方向パイプで挟んだ
   bronkhorst ドライバ**で実現(別の偽物を作らない)。テストから値の時系列と無応答を注入できること。
4. `src/slowctl/component.rs` + `src/bin/slowctl.rs`:
   - `src/config.rs`: `Config.slowctl: Option<SlowctlConfig>`(既存 TOML は無変更で読める)、`MonitorConfig.slow_sub: Option<String>`。
     `deny_unknown_fields` 維持。`port` が `/dev/serial/by-id/` `by-path/` 以外なら **warn**(拒否しない — Mac 開発機に by-id は無い)。
   - デバイス毎に **std スレッド**(ブロッキング IO)で `ident` → `poll` ループ(`poll_hz`)。結果は `std::sync::mpsc` で本体へ。
     エラーはカウンタ(`poll_ok / poll_err / parse_err / reconnects / identity_mismatch`)+ `warn!` + 指数バックオフ再接続(1 → 30 s)。
   - 本体(tokio の 1 タスク、または std のまま — KISS で選び、理由を結果節に): (a) **status PUB**(`status_pub`、map 形式 msgpack、
     kind = `slow`、1 Hz に集約、最新値のみ)(b) **LogPost**(既存 PUSH → controller `log_pull_bind`、JSON 1 行、`type: "slow"`、
     §9.2 の方針: IDENT 起動時 / VALUE は LogPolicy / ALARM・RECOVER 即時 / ERROR は 1 分毎の増分、0 なら書かない)。zmq_helper の既存 API を使う。
   - CLI `--config x.toml`、`RUST_LOG`。SIGINT で PUB/PUSH を閉じて終了。
5. `src/logbook.rs`: `LogbookRecord::Psu` → **`Slow`**(`type: "slow"`、§9.2 のフィールド順)。`PsuValues` 削除。psu の手組みオラクル 3 本は
   slow の手組みオラクルに置換(件数維持)。`src/runs_yaml.rs` の psu 参照も追従。
6. `src/monitor.rs`: `slow_sub` があれば slowctl の PUB を **同じ SUB ソケットに追加 connect**。**recv 前に poll する既存規律を守る**
   (メモリ zmq-fairqueue-starvation: 複数上流の SUB は recv 前に poll)。最新 `slow` ペイロードを保持し WS `status` JSON に `slow` として同梱。
   `slow_sub` 無しなら従来と同一 JSON(既存テストが無変更で green = 証拠)。
7. `src/controller.rs`: log-post スレッドで `type == "slow"` の VALUE/ALARM/RECOVER を見て `slow_latest`(`HashMap<String, {value, unit, ts}>`、
   キー `"<device>/<node>/<param>"`、node 無しは `"-"`)を更新。`run_start` / `run_stop` に `slow`(nullable、無ければ省略)。run 平均は作らない。
8. `config/slowctl_example.toml`(SPEC §3.1 の例をそのまま)。既存の実運用 TOML には `[slowctl]` を **書かない**(現地 Phase 0 の後)。

### テスト(TDD: 赤を実測してから緑)
- 単体: AlarmRule(境界・復帰・node 一致)/ LogPolicy(2 % と時間)/ config パース(省略・by-id 警告・未知キー拒否)/
  `Slow` レコードの手組みオラクル(§9.2 のフィールド順、JSON 文字列完全一致)/ kind=`slow` msgpack の往復。
- 統合 `tests/slowctl_integration.rs`: fake device 1 台の TOML で `slowctl` bin を子プロセス起動(既存統合テストの流儀、ポートは OS 任せ)→
  ① SUB で `kind == "slow"` を受け samples に fmeasure がある ② PULL(controller の代わりにテストが bind)に IDENT → VALUE の順で届く
  ③ 閾値を跨ぐ値を注入 → ALARM → RECOVER ④ 無応答を注入 → poll_err 増加、復帰後に VALUE 再開。
- monitor: `slow_sub` あり/なしで status JSON の `slow` の有無(既存 monitor テストの流儀)。controller: `run_start.slow` が載る(controller_integration に 1 本)。
- 実機受け入れ(現地、Phase 0 後): FlowSuite / 前面表示と 1 分間一致 — 本票では未実施と結果節に明記。

### 受け入れ
`cargo fmt && cargo clippy --tests -- -D warnings && cargo test` green(614 → 増分を結果節に)。production に `.unwrap()` 無し。
silent failure 無し(全エラー経路にカウンタか `warn!`)。**設計判断が残ったら実装せず報告**(例: serialport の feature で Linux ビルドが変わる、
monitor の SUB を 1 本にするか 2 本にするか)。

## 2.7 Phase 3 発注(Arduino MKR ENV スケッチ + ドライバ — Sonnet、2026-09-23 着手)

前提: Phase 2 済(`Driver` trait / `Sample` / `Ident` / `DriverError` は `src/slowctl/mod.rs`、`open_driver` は `src/slowctl/component.rs`、
シリアルの open は `bronkhorst.rs` の `open_serial` の流儀)。着手前に worktree で `git merge --ff-only main`。発注書は本体の絶対パス
`/Users/aogaki/Workspace/tpcdaq-rs/TODO/079_slow_control_gas_env.md`(§2.3、§2.6、Phase 2 結果節、本節)で読んでよい。

### 作るもの
1. `tools/arduino_env/env_json.ino` + `tools/arduino_env/README.md`: ボード = Arduino MKR 系 + **MKR ENV Shield**(ライブラリ `Arduino_MKRENV`)。
   `Serial.begin(115200)`、`ENV.begin()` 失敗時は 1 秒毎に `{"dev":"mkr_env","error":"ENV shield not found"}`。成功時は **1 秒毎に JSON 1 行**:
   `{"dev":"mkr_env","seq":123,"t_c":24.31,"rh":41.2,"p_hpa":1003.7,"lux":120.5,"uva":0.12,"uvb":0.03}`(小数 2 桁、`seq` は起動からの通番、行末 `\n`)。
   WiFi は使わない。README に配線(シールド装着のみ)、`arduino-cli` での書き込み手順(core / lib のインストールコマンド)、`/dev/serial/by-id/` の見つけ方。
2. `src/slowctl/arduino.rs`: `ArduinoEnv<S: Read + Write>` が `Driver` を実装。
   - `ident`: 最大 3 行(各行のタイムアウトは呼び手の設定)を読み、`"dev":"mkr_env"` を確認して `Ident { device, node: None, fields: {"dev": "mkr_env", "seq": ...} }` を返す。
     `"error"` 行なら `DriverError::Protocol(<error 文言>)`、`dev` が違えば `DriverError::Identity { expected: "mkr_env", got }`。
   - `poll`: バッファに溜まった行を全部読み **最後の完全な行**だけを使う(1 Hz 未満で読んでも古い行で埋めない)。Sample は node = None で
     `t_c`[`degC`] / `rh`[`%`] / `p_hpa`[`hPa`] / `lux`[`lux`] / `uva`[`raw`] / `uvb`[`raw`]。JSON でない・キー欠落は `Protocol`(parse_err に数えられる)、行が来なければ `Timeout`。
   - `serde_json` で復号(依存追加なし)。`seq` の飛びは `warn!` で可視化(数えるだけ、エラーにしない)。
3. `component.rs::open_driver` に `"arduino_env"`(`port` 必須、115200 8N1、`open_serial` と同じ by-id 警告と `expect_serial` 不使用 = 名乗りで照合)。
4. `src/slowctl/fake_arduino.rs`(cfg(test) + feature `slowctl-fake`): 行の列と無応答を注入できる in-memory 応答器。

### テスト(TDD)
- 単体: 正常行のゴールデン(上の JSON をそのまま)→ Sample 6 個と単位 / `error` 行 / `dev` 不一致 / 壊れ JSON / キー欠落 / 複数行溜まりで最後だけ採用 / 無応答 Timeout。
- `open_driver("arduino_env")` の経路は config パースの単体(既存の `[[slowctl.device]]` テストの流儀)で `type` と `port` の検証まで。
- `cargo fmt && cargo clippy --tests -- -D warnings && cargo test` green(現 784)、production `.unwrap()` 無し。実機受け入れ(現地でシールド接続、1 分間 FlowSuite 相当の目視)は未実施と明記。

## 3. 確認事項(着手は止めない — 既定で進め、現地で確定)
1. ~~Bronkhorst の機種・接続~~ → **判明**(§1.1–1.2)。残: 先頭機器・ノード番号(Phase 0 で scan)、
   **RS232 の現在の相手 PC**(第 2 local host が使えるかで運用が変わる、§1.5)。
2. ~~Arduino スケッチ~~ → **自前で書く**(裁定)。残: 設置場所(検出器近傍? 室温・大気圧の用途)。
3. ALARM で run を止めるか → **既定: 止めない**(記録 + バッジ)。
4. HV(HiVolta / HMP2020、P6)との統合順序 → **既定: ガス系を先に、HV は同じ slowctl に後乗せ**。

## 4. フェーズと見積もり
| Phase | 内容 | モデル | 日数 |
|---|---|---|---|
| 0 | **現地**: DAQ 機に USB-RS232、`bronkhorst-propar` で node scan → 各ノードの識別群 + 計測値を hex ダンプ(reference/ にゴールデン、リポには合成 fixture のみ)。第 2 local host の可否も試す | ユーザー + Sonnet(手順書) | 0.5 |
| 1 | ProPar ASCII codec(encode/decode、chaining、`:01xx`)+ pty fake + node scan。9.17.027 §3.9 の実例で縛る | Sonnet | 1 |
| 2 | slowctl 骨格(TOML / driver trait / LogPost / status PUB)+ logbook `slow` 一般化 + controller の run 記録拡張 | Opus | 2 |
| 3 | Arduino スケッチ + ドライバ + 単体 | Sonnet | 1 |
| 4 | UI トレンド(ECharts、§11)+ status バッジ + ALARM 表示 | Sonnet | 2 |
| — | SPEC §1.1 / §2.3 / §3 / §9 / §11 改訂 + フェーズ境界レビュー | Fable | — |

Phase 1 は Phase 0 を待たずに着手できる(マニュアルの実例がゴールデン)。Phase 0 は新 DAQ 機のセットアップ
(076 §6 の順序)に相乗り。

## 5. 参照
- reference/Bronkhorst/IMG_3204–3208.jpeg(現物)、917101(EL-PRESS)、917027(RS232/ProPar)、917024(FLOW-BUS)、
  `English Bronkhorst manuals/917023`(パラメータ表)、917011(RS232/FLOW-BUS box、不使用)。
- reference/Arduino/(購入メール、LabVIEW アプリ)。MKR ENV Shield: https://docs.arduino.cc/hardware/mkr-env-shield
- SPEC §1.1(slowctl)、§2.3(LogPost)、§3.2(source_id 200)、§9.2(`psu` → `slow`)、§11(トレンド・安全境界)。
- 071 SURVEY(CO₂ 130 mbar 流通運転、圧力計への電源干渉 p.352–353)。
- Pfeiffer: TPG 361/362 取説(ajvs.com/library/Manual_Instructions_TPG361_TPG362_…pdf)、同 Communication Protocol
  (ajvs.com/library/Pfeiffer_Operating_instructions_TPG361_TPG362_…Communication_Protocol.pdf)、TPR 280 取説
  (mpi-hd.mpg.de/gerda/TG04_TECHNICAL/manuals/bg805178be_d_web.pdf)。PDF は reference/ に置く(コミット不可)。

## 6. Pfeiffer DualGauge — **見送り(裁定 2026-09-15)**

ガスエンジニアの見解(ユーザー経由): Pfeiffer のゲージは真空用で **100–200 mbar では役に立たない**。
また EL-PRESS の **±0.5 % FS 級(130 mbar で ≈ 4 %)はこの分野では非常に良い精度**。
→ **圧の正 = EL-PRESS の fMeasure(33/0)**。run 記録・v_drift の引き当てもこれを使う。Pfeiffer ドライバは
起票しない(調査した TPG 361/362 プロトコル要点は履歴 `0da03dc` に残っている。必要になれば掘り出す)。
黒赤の箱 = Pfeiffer ActiveLine ゲージヘッド(系列未確定、確定させる必要なし)。

## 7. 対象外だが見えた機器(将来候補、起票は要望が出てから)
- NTRON Microx O₂ 計、Michell Easidew 露点計、Agilent AGD-100 — 接続口(4–20 mA / RS232 / RS485)は
  現地確認が要る。O₂・露点はガス純度の指標なので価値は高い。

## 結果(Phase 1 — ProPar codec / link / fake、2026-09-17、Sonnet レーン → main `617ab71`)

- 実装: `src/slowctl/{mod,propar,link,fake}.rs`(919 + 336 + 165 行)、`Cargo.toml` に feature `slowctl-fake`(依存追加なし)。
  書き込みコマンド(01)は型として存在しない。`link.rs` は `Read + Write` ジェネリック(serialport は Phase 2 で差す)。
- テスト: **cargo test(main)578 → 614 passed / 0 failed**(レーン内 472 → 508、+36: propar 28 / link 5 / fake 3)。fmt / clippy clean。
  ゴールデン = 9.17.027 §3.9 の実例 7 種(measure p.24 / fmeasure p.24 / capacity unit p.26 / fluid name p.25 / serial number p.30 /
  valve output p.26 / iostatus p.31、hex を一字も変えず)+ エラー表 `:01xx` 全 7 種(p.13)+ status 表(p.17)+ 3 値 chaining
  (§3.7/§3.8 のビット配置から手計算)+ 壊れ行 6 種 + fake 経由の node scan(応答 3 / 無応答 2、カウンタまで断定)。
- 承認した逸脱: ①`parse_reply(line, request)` — Float と Long がワイヤ上同じ型タグ(40h)で行だけでは復号不能 ②index は常に
  FBnr(§3.8 の注記)③decode は chain ビット駆動で process byte 省略の有無を自動判定、encode は常に process byte を起こす。
- **マニュアルの不備(実装に反映せず、現地 Phase 0 で確認)**: §3.9.9 の hex に `no1` 混入 / §3.9.6 はページ跨ぎで崩れ /
  §3.9.5 の fmeasure・valve output の Answer が node 03(消し忘れ)/ **serial number の Answer は長さバイト 00 のまま 11 B の
  実データが続く**(実機の挙動か誤植か不明 — node scan で 113/3 を読んで確かめる)。
- Phase 2(slowctl 骨格、Opus)の入口: `ProParLink::new(port)` に `serialport` の `Box<dyn SerialPort>`(Read + Write)を渡すだけ。

## 結果(Phase 2 — slowctl 骨格、2026-09-22、Opus レーン)

### 実行コマンド(すべて green / 警告ゼロ)

```
git merge --ff-only main      # ベース = c797792
cargo fmt && cargo fmt --check
cargo clippy --tests -- -D warnings
cargo clippy --all-targets -- -D warnings
cargo test
```
実行環境: macOS 15(Darwin 25.6.0)、worktree `agent-ad12512bd609e755e`、2026-09-22。

### テスト数(before → after)

**`cargo test` 614 passed → 658 passed / 0 failed / 1 ignored(+44)**
(→ レビュー指摘 2 の追加実装で **662 passed / 0 failed(+48)**。下の「追記」節)。
内訳(新規): `src/slowctl` 22(bronkhorst 9 / component 6 / domain 5 / fake_device 2)、
`src/config.rs` 8、`src/monitor.rs` 2、`src/logbook.rs` 1、`src/controller.rs` 1、
`src/bin/slowctl.rs` 2、`tests/slowctl_integration.rs` 2、`tests/monitor_ws.rs` 2、
`tests/controller_integration.rs` 2。
psu のオラクル 3 本(`sample_psu` を使う手組みオラクル / フィールド順 / 5 型 round-trip)は
**slow のオラクルに 1 対 1 で置換**(件数維持)。`slow_matches_a_hand_assembled_oracle` は
VALUE / IDENT / ERROR の 3 形を JSON 文字列完全一致で固定する。

### 統合テスト `tests/slowctl_integration.rs` の 4 項目(実バイナリを子プロセス起動、ポートは OS 任せ)

デバイスは `type = "fake"`(= Phase 1 の `FakeProPar` を共有ハンドルで挟んだ**本物の
Bronkhorst ドライバ**)。値の時系列はスクリプトファイルで注入(`100,100,120,120,100,silent,silent,105`)。

| # | 見たこと | 実測 |
|---|---|---|
| ① | SUB に `kind == "slow"`、`samples` に `fmeasure` | source_id = 200、`device="gas"` / `node=3` / `unit="mbar"`(機器から読んだ 1/31 の文字列) |
| ② | PULL に IDENT → VALUE の順 | 1 通目 = `IDENT`(`capacity_unit="mbar"`、`serial` あり)、2 通目 = `VALUE` |
| ③ | 閾値([90, 110])を跨ぐ | `ALARM` の値 = 120.0、`RECOVER` の値 = 100.0、順序も ALARM → RECOVER |
| ④ | 無応答 → 復帰 | `counters.poll_err == 2`(`silent` 2 行ぶんちょうど)、`parse_err == 0` / `identity_mismatch == 0`、復帰後に fmeasure = 105.0 が PUB に再登場 |

+ `[slowctl]` の無い TOML で bin が**何もせず正常終了**することも 1 本(既存 TOML 無変更の証拠)。
monitor 側は `tests/monitor_ws.rs` に 2 本: `slow_sub` 無し = status JSON に `slow` キーが**無い**
(従来と同一)、`slow_sub` あり = 同じ SUB の 2 本目から来た `slow` が status に同梱され、
`monitorGaps == 0`(2 ソースの seq を混ぜていない)。

### 承認が必要な逸脱(発注書に無い判断)

1. **`[slowctl] log_push`(省略可、既定 `tcp://127.0.0.1:47005`)を足した** —— SPEC §3.1 の例には
   無いキー。controller のログ投稿 PULL をコードに焼き込まないため(CLAUDE.md「ポートを焼き込まない」)
   + 統合テストが宛先を動的ポートに向ける必要があるため。SPEC §3.1 に追記するかは Fable 裁定。
2. **LogPost の送信は `sndtimeo = 200 ms`(ノンブロッキングではない)** —— ロスレス PUSH は
   `ZMQ_IMMEDIATE` なので、起動直後 ZMTP ハンドシェイク前の 1 本目(= IDENT)が「相手が居るのに」
   落ちる(実際に `cargo test` 全体実行で 1 度再現した)。無限待ちは controller 不在で計測を
   止めるので不可。200 ms は握手(localhost で ms 級)に桁で足り、controller 不在時も 1 本あたり
   この時間しか止まらない。落ちた分は `log_dropped` + `warn!`。
3. **`Driver::poll` は最初のノードの失敗でそのまま `Err`**(部分成功を返さない)。`ident` の
   node scan は無応答ノードを飛ばすので、`poll` の失敗は「起動時に応答したノードが落ちた」= 異常。
   5 連続で失敗したらリンクを開き直す(指数バックオフ 1 → 30 s)。
4. **`alarm_info`(1/20)を `unit = "bitmask"` の Sample としても出す** —— 発注書の単位表には
   無いが、機器側アラームのビットが記録に残らないと「なぜ ALARM になったか」が読めないため。
   Sample の `alarm` フラグはそのノードの全サンプルに立てる。
5. **monitor の seq 追跡を source_id 毎に分けた** —— v1.31 で SUB の上流が 2 本(root-sink 101 /
   slowctl 200)になり、1 本の `GapTracker` では 101 と 200 の seq が交互に来て巨大なギャップ・
   巻き戻りに見える。`BTreeMap<source_id, GapTracker>` にして `monitorGaps` の意味を保った
   (テスト `sequence_gaps_are_tracked_per_source_id` で固定)。
6. **`Cargo.toml` に自分自身への dev-dependency**(`tpcdaq = { path = ".", features = ["slowctl-fake"] }`)
   —— 統合テストが**子プロセスとして起動する** `slowctl` バイナリに fake を入れるため
   (`cfg(test)` は bin には効かない。cargo#2911 の定石)。`cargo build` / `cargo install` には無影響
   (`cargo build --bins` で確認済み)。
7. **本体は tokio ではなく std スレッド + `mpsc` + 同期 ZMQ**(発注書が「KISS で選び理由を結果節に」)
   —— デバイス IO はブロッキングで周期 1 Hz 級。tokio に載せても `spawn_blocking` でスレッドに
   戻るだけで、async 境界ぶんの複雑さしか増えない。tokio は `src/bin/slowctl.rs` の SIGINT 待ちのみで、
   本体へは `AtomicBool` を 1 本渡す。
8. **`PSU_SOURCE_ID` → `SLOWCTL_SOURCE_ID` に改名**(値 200 は不変。SPEC §3.2 v1.31 の「旧 psu」に追従)。
9. **偽デバイスの時系列はスクリプトファイルで注入**(`type = "fake"` の `port` がそのパス)。
   子プロセス越しに時系列と無応答を注入する口が他に無いため。`fake_device.rs` は
   `cfg(any(test, feature = "slowctl-fake"))` に閉じている。

### 未実施

- **実機受け入れ(現地、Phase 0 後)**: FlowSuite / 前面表示と 1 分間一致 —— **本票では未実施**。
  実機が無いと測れない(先頭機器・ノード番号・by-id の実名も現地で確定)。
- `arduino_env`(Phase 3)/ `hivolta` / `hmp2020`(085 + 後続)のドライバ: TOML の `type` としては
  受理するが、`open_driver` が「未実装」で明示的に失敗する(黙って何も読まないデバイスを作らない)。
- 実運用 TOML への `[slowctl]` 追記: **していない**(現地 Phase 0 の後 — 発注書 §2.6-8)。
  リポには見本 `config/slowctl_example.toml` だけを置き、その TOML が実際に `config::parse` を
  通ることをテストで固定した。
- UI トレンド / status バッジ(Phase 4)。

### 追記(Phase 2 レビュー対応 — LogPost 再送キュー、2026-09-22)

Fable レビュー: **逸脱 1(`log_push` キー)承認**(SPEC §3.1 への追記は Fable が main 側で実施)、
**逸脱 3–9 承認**、**逸脱 2 は「IDENT が落ちるのは許容できない」** —— 実運用の起動順では
controller が最後に上がる(docs/NEW_DAQ_SETUP_ja.md)ので、slowctl が先に立つと起動時の IDENT が
**確実に**落ちる。→ 再送キューを追加実装した。

**仕様(`src/slowctl/component.rs`)**

- `is_retryable(record)`: 再送するのは **`IDENT` / `ALARM` / `RECOVER`**(= 一度きりの事実。
  落とすと二度と記録に現れない)。**`VALUE` は再送しない**(次の VALUE が来る)、
  **`ERROR` も再送しない**(次回の 1 分集計に増分が含まれる)。古い値で logbook を埋めない。
- `RetryQueue`: FIFO、上限 **64**(`RETRY_QUEUE_LIMIT`)。超えたら**最古から**捨てて
  `log_dropped` を数え `warn!`(無制限に抱えない = CLAUDE.md のメモリ規律)。
- `LogPoster`: 本体ループの**周回の先頭で `flush()`** —— 溜まった再送を投函順に流してから
  新しいレコードを投函する。よって controller が後から上がっても **1 通目は必ず IDENT**。
  1 周回で失敗する送信は高々 1 本(最初の失敗で `down` を立て、その周回は以後 send を試さない)
  = `LOG_SEND_TIMEOUT_MS`(200 ms、逸脱 2 で承認済み)のブロックが積み上がらない。
  送れなかった 1 本は**先頭へ戻す**ので順序は崩れない。
- 捨てた投稿の累計 `log_dropped` は LogPoster が持つ(SPEC §2.3 の `counters` には載らない
  slowctl 自身の健全性。warn に `queued` と一緒に出る)。

**テスト(658 → 662 passed / 0 failed、+4)**

- `src/slowctl/component.rs` +3: `only_one_off_records_are_worth_resending`(5 種の event +
  slow 以外)/ `the_retry_queue_is_bounded_and_drops_the_oldest_first`(上限ちょうどまで
  1 本も捨てない → 65 本目で最古 `IDENT0` が落ち、先頭が `IDENT1` になる)/
  `records_queued_while_the_controller_is_down_arrive_in_order_once_it_is_up`
  (**実 ZMQ**。相手不在で IDENT・ALARM が溜まり VALUE は捨てられる → PULL を bind →
  `IDENT, ALARM` の順で届き `log_dropped == 1`)。
- `tests/slowctl_integration.rs` +1(発注どおり):
  `records_from_before_the_controller_was_up_arrive_in_order_starting_with_ident` ——
  **PULL を bind せずに** slowctl を起動 → 1 s(poll_hz 20 で 20 tick、スクリプト 8 行は 0.4 s で
  流れ切る)→ PULL を bind → 受けた順が **`[IDENT, ALARM, RECOVER]`**。停止中の VALUE は届かない。
- 既存 4 項目は**変更なし**。`tests/monitor_ws.rs` の `a_slow_sub_...` に `client.settle()` を
  1 行足した(全体実行時に `warm_up` の "idle" 在庫を読んで `state` の assert が落ちたため。
  harness が同じ目的で用意している作法で、slow の検証内容は不変)。
- 再実行 5 回連続 green(`--test slowctl_integration` / `--test monitor_ws`)+ `cargo test` 全体 2 回連続 green。
  `cargo fmt --check` / `cargo clippy --tests -- -D warnings` / `cargo clippy --all-targets -- -D warnings` clean。

**残る既知の限界**(設計判断が要るなら Fable 裁定): 再送キューは**プロセス内のみ**。
slowctl が controller より先に落ちればキューの中身は失われる(永続化はしていない)。

### Fable レビュー(Phase 2、2026-09-22、マージ `worktree-agent-ad12512bd609e755e` → main `c3f3a0c`)
- 逸脱 1(`[slowctl] log_push`)承認 → SPEC §3.1 に追記済み(`8788bf9`)。逸脱 2(sndtimeo 200 ms)は **IDENT / ALARM / RECOVER の再送キュー**(上限 64、
  最古から捨てて `log_dropped`)を追加させて承認(起動順で controller が最後 = IDENT が落ちる経路を塞ぐ)。逸脱 3–9 承認
  (std スレッド + 同期 ZMQ の KISS 構成、monitor の seq 追跡を source_id 毎に分離、alarm_info を bitmask Sample、self dev-dependency、偽デバイスのスクリプト注入)。
- **裁定: 再送キューの永続化はしない**(プロセス内のみ)。slowctl 再起動時に IDENT は必ず再送されるので、失われるのは停止中の ALARM/RECOVER だけ。
  それは controller 側の run 記録 `slow` と次の VALUE で分かる。永続化は KISS に反する。
- マージ時の衝突は `src/slowctl/mod.rs` の `pub mod` ブロック(085 と本票)のみ、両方残して解決。
- 次(Phase 3 = Arduino スケッチ + ドライバ、Phase 4 = UI トレンド、+ 085 のコーデックを `Driver` に載せる小チケット)は現地 Phase 0 と並行で可。

## 結果(Phase 3 — Arduino MKR ENV スケッチ + ドライバ、2026-09-23、Sonnet レーン)

### 実行コマンド(すべて green / 警告ゼロ)

```
git merge --ff-only main      # ベース = a11013a
cargo test                    # 着手前ベースライン確認
cargo fmt && cargo fmt --check
cargo clippy --tests -- -D warnings
cargo clippy --all-targets -- -D warnings
cargo build --features slowctl-fake
cargo clippy --all-targets --features slowctl-fake -- -D warnings
cargo test                    # 3 回連続実行(安定性確認)
```
実行環境: macOS 15(Darwin 25.6.0)、worktree `agent-aa08b0edab240cf62`、2026-09-23。

### テスト数(before → after)

**`cargo test` 784 passed → 803 passed / 0 failed / 1 ignored(+19)**。
着手前ベースラインの `cargo test` 実行で `tests/slowctl_integration.rs` の
`records_from_before_the_controller_was_up_arrive_in_order_starting_with_ident` が
1 度だけ FAILED(タイミング依存、Phase 2 結果節に記載済みの既知の揺れ)。同テストを
単体で 3 回連続実行すると green — 本チケットの変更とは無関係(`src/slowctl/component.rs`
の LogPost 再送タイミングの話で、Phase 3 では触っていない)。以後の `cargo test` 全体実行
(fmt/clippy 後 + 最終確認の計 3 回)はすべて 0 failed。

内訳(新規 19): `src/slowctl/arduino.rs` 14(ident 6: 正常行 / error 行 / dev 不一致 /
壊れ行 3 回リトライで成功 / 3 回とも壊れて失敗 / 無応答 Timeout。poll 8: 6 サンプル + 単位 /
複数行溜まりで最後だけ採用 / 尻切れ末尾行は不使用 / 壊れ JSON / キー欠落 / error 行 /
無応答 Timeout / seq 飛びでも失敗しない)、`src/slowctl/fake_arduino.rs` 3(push_line の
読み戻し / 空キューは WouldBlock / push_raw は終端を足さない)、
`src/slowctl/component.rs` 2(`open_driver("arduino_env")` が
`arduino::open_serial` にルーティングされている確認 —— 実在しないポート名を渡して
「open_driver 未実装」ではなく「シリアルポートを開けない」で落ちることを見る / `port` 省略は
`SlowctlError::Device`)。

### スケッチの書き込み手順の要旨(`tools/arduino_env/README.md` に詳細)

- ボード = Arduino MKR 系 + MKR ENV Shield(ASX00011、配線は装着のみ)。
- `arduino-cli core install arduino:samd` → `arduino-cli lib install "Arduino_MKRENV"` →
  `arduino-cli compile --fqbn arduino:samd:mkr1000 tools/arduino_env/env_json.ino` →
  `arduino-cli upload -p /dev/serial/by-id/... --fqbn arduino:samd:mkr1000 ...`。
- 出力は 1 秒毎 JSON 1 行(`{"dev":"mkr_env","seq":...,"t_c":...,"rh":...,"p_hpa":...,"lux":...,"uva":...,"uvb":...}`、
  小数 2 桁)。`ENV.begin()` 失敗時は `{"dev":"mkr_env","error":"ENV shield not found"}` を
  1 秒毎に出し続ける(復旧はシールドの物理的な再装着のみ、自己修復しない)。
- `p_hpa` は `ENV.readPressure(MILLIBAR)` を使う(既定の kPa ではなく hPa 相当の桁に揃える —
  発注書の JSON 例(1003.7)は hPa 桁なので、単位換算をスケッチ側で済ませた)。

### 承認が要る逸脱(発注書に無い判断)

1. **`ident` の「無応答」と「壊れた内容」を明確に分けた** —— 発注書は「最大 3 行を読み…」
   としか書いていないが、無応答(1 バイト目からタイムアウト)をリトライすると本来の
   `Timeout` 経路と「3 回壊れた行を試して Protocol」経路が曖昧になる。実装は
   **無応答は即座に `Timeout` を伝播(リトライしない)、壊れた JSON だけ最大 3 行まで
   リトライする**とした(open 直後にスケッチの出力の途中から読み始める場合の救済が
   「最大 3 行」の狙いと解釈)。テスト `ident_times_out_when_no_line_arrives` /
   `ident_skips_malformed_lines_up_to_three_attempts_then_succeeds` /
   `ident_fails_after_three_malformed_lines` で 3 経路を別々に固定した。
2. **`poll` の「バッファに溜まった行を全部読む」の停止条件** —— 実際のシリアル IO では
   「もう読めるバイトが無い」を検出する唯一の手段が read のタイムアウト/WouldBlock なので、
   `drain_lines` は「1 バイト読もうとしてタイムアウトしたら、そこまでに集めた完全な行を
   返す」とした。終端 `\n` が来ていない末尾の断片は**捨てる**(次の `poll` 呼び出しで
   また 1 から読み直す —— モニタ系なので CLAUDE.md の drop-with-count の範囲。テスト
   `a_trailing_partial_line_is_not_used` で固定)。
3. **`poll` は `dev` フィールドを再照合しない** —— 発注書の poll の記述に `dev` 確認は無く
   (`ident` だけが「名乗りで照合」する)、`open_driver` 経由では一度 `ident` した後の
   同じシリアルポートを使い続けるので、`poll` 毎に `dev` を見直す理由が無い(KISS)。
4. **UV(`uva`/`uvb`)の単位は `"raw"`** —— 発注書の表に単位の指定が無い(較正単位が
   存在しない)。bronkhorst の valve output(生値 + `unit: "raw"`)と同じ扱いにした。
5. **1 行の上限 512 バイト**(`MAX_LINE_BYTES`)—— 発注書に無い値。終端 `\n` が来ない
   壊れた入力でバッファが無制限に伸びないための防御(CLAUDE.md のメモリ規律)。
   通常行は ≈100 バイトなので実運用には無関係。
6. **`open_driver` の component.rs テストは実機を使わず「存在しないポート名を開こうとして
   失敗する」ところまでで止めた**(発注書 §2.7-2「config パースの単体で `type` と `port` の
   検証まで」の実装として、`arduino::open_serial` へのルーティングと `missing_port` 防御の
   2 本)。実機の識別・計測確認は Phase 0 後の現地作業(未実施)。

### 未実施

- **実機受け入れ**(現地でシールド接続、1 分間 FlowSuite 相当の目視/前面表示との一致確認)
  —— **本票では未実施**(発注書どおり)。ボード購入・現物は reference/Arduino(gitignore)に
  記録済みだが、Phase 0(現地)後の作業。
- UI トレンド / status バッジ(Phase 4、未着手)。
- 実運用 TOML への `[[slowctl.device]]`(`type = "arduino_env"`)追記(現地確認後、本票の
  範囲外)。

