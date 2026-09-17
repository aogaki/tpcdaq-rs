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
