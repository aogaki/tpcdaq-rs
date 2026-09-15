# 079 — スローコントロール: ガス圧(Bronkhorst EL-PRESS / ProPar)+ 環境(Arduino MKR1000 + ENV shield)

**Status: DRAFT**(2026-09-15 起票、Fable。発端 = ユーザーが `reference/Arduino` と `reference/Bronkhorst` に
資料を投入。**§3 の確認事項が埋まってから READY**)

## 0. 結論(1 分で)

- **どちらもベンダーの Windows ソフト無しで Linux から直接読める。**
  Bronkhorst は RS232 の **ProPar ASCII**(38400 8N1、`:` 始まり CR 終わり、RD=04 / WR=01、
  点対点なら node 128 で必ず応答)— Rust の `serialport` crate で数百行。Arduino MKR1000 は USB シリアル
  (兼 WiFi)— ただし**現行の読み出しは LabVIEW 製の Windows .exe で、Arduino 側スケッチのソースが無い**
  ので、自前スケッチ(JSON 1 行/秒)に置き換えるのが最短。
- **受け皿は SPEC に既にある**: P6 の `psu`(HiVolta/HMP2020、source_id 200、logbook `psu` レコード、
  §11 のトレンド表示)。これを **`slowctl` = 汎用スローコントロール(HV / ガス / 環境)** に一般化する。
- **物理側の効き目**: 圧力は v_drift ∝ E/p の分母。run 毎に圧力を logbook に残せば、
  analyzeUVW.C の `PRESSURE_MBAR` 定数を run 記録から引ける(今は手打ち)。
- 見積もり ≈ **1.5 週間**(slowctl 骨格 + Bronkhorst ドライバ + fake + logbook 一般化 = Opus 3 日 /
  Arduino スケッチ + ドライバ = Sonnet 1 日 / UI トレンド = Sonnet 2 日 / SPEC = Fable)。

## 1. 資料の中身(reference/ — gitignore、コミット不可)

### Arduino(1 MB)
- **MKR1000**(SAMD21 + WiFi)+ **MKR ENV Shield ASX00011**(温度・湿度・気圧・照度・UV)。
  ELI-NP の A. State → T. Roman のメール(2021-11-16)で購入(cleste.ro)。
- `MKR1000 + ENV Shield.exe` = **LabVIEW ビルドの Windows アプリ**(`.ini` は LabVIEW ランタイム設定、
  `.aliases` は `My Computer=localhost`)。**Arduino スケッチのソースは無い** → 現行のワイヤ
  プロトコル(USB シリアルか WiFi TCP か、フォーマット)は不明。

### Bronkhorst(145 MB、マニュアルのみ)
- **EL-PRESS シリーズ**(9.17.101、2023): P-5xx 圧力計 / P-6xx forward / P-7xx back-pressure /
  P-8xx process 制御器。2–100 mbar … 8–400 bar。**標準 = RS232(ProPar)+ アナログ(0–5/0–10 V、
  0/4–20 mA)**、オプションで FLOW-BUS / Modbus / PROFIBUS / EtherCAT 等。
- **RS232 / ProPar**(9.17.027): 38400 8N1、ASCII `:` + len + node + data + CR(2 桁 HEX 化)、
  enhanced binary(DLE STX …)もあり。node 3–120、**点対点なら 128 で応答**。コマンド 00 status /
  01 WR(status 返る)/ 04 RD。chaining 可。エラー `:01xx`。
- FLOW-BUS(9.17.024)/ E-8000 電源・表示・RS232 ハブ(9.17.076)/ FlowDDE / FlowPlot / FlowSuite
  (Windows 用ツール)/ LabVIEW ドライバ。+「English Bronkhorst manuals」= 全製品のカタログ的一式。
- **機種(model key)・ノード番号・接続形態(直 RS232 / USB 変換 9.09.122 / E-8000 経由)は資料に無い。**
  ログブック 5 年分に Bronkhorst の記載は無し(CO₂ 130 mbar **流通運転**、NIM 電源投入が圧力計に
  干渉した事例 p.352–353 のみ = 071 SURVEY)。

## 1.5 安全境界の裁定候補(2026-09-15 ユーザー提起「制御を web 越しに誰でも触れるのは問題」)

- **web 操作面(controller REST / UI)= DAQ の run 制御と監視のみ。物理的作動(HV / ガス圧 setpoint /
  バルブ)は web に経路を作らない。** 根拠 = SPEC §3.1 の token + passphrase は「事故防止であり認証では
  ない」、TLS/認証は自前実装しない方針(CLAUDE.md)。ガス圧・HV は物理的に壊せる。
- したがって **DAQ 統合はモニタ(読み取り・記録・アラーム)のみ**。これは冗長ではなく、価値のある部分
  (run 毎の圧力 → v_drift、漏れ検知、logbook 自動記録)は全部読み取りで済む。setpoint の変更は
  実験構成の切替時(130 / 190 / 250 mbar)だけで稀。
- **本当の衝突点 = RS232 ProPar は master 1 つ**(点対点)。制御プログラムと監視プログラムを別々に
  立てると同じポートを取り合う。解は 2 つ、どちらも web には載せない:
  1. **制御は機器側の既存手段**(E-8000 パネル / FlowSuite on Windows ノート)、DAQ は読むだけ(推奨、
     開発ゼロ)。FlowSuite を繋ぐ間は slowctl を止める運用ルール。
  2. **slowctl がポートを専有**し、制御は **Unix ドメインソケット / 127.0.0.1 の CLI** からだけ受ける
     (controller / UI に制御 API を作らない)。スクリプトでの圧力スキャン等が必要になったら。
- HV(P6)も同じ規則: DAQ は監視 + TRIP 記録、電圧設定はベンダー GUI をローカルで。
- SPEC への反映案: §11(または新設「安全境界」1 行)に上記規則を明文化(Fable、ユーザー確認後)。

## 2. 設計(案 — §1.5 により **読み取りのみ**。setpoint 書きは選択肢 2 を採るときだけ)

### 2.1 component `slowctl`(Rust / tokio、SPEC §1.1 の psu 行を置換)
- TOML `[[slowctl.device]]`: `type = "bronkhorst_propar" | "arduino_env" | "hivolta" | "hmp2020"`、
  `port = "/dev/ttyUSB0"`、`node = 128`、`poll_hz = 1`、`channels = [{name, unit, lo, hi}]`。
- ドライバ trait: `poll() -> Result<Vec<Sample{device, channel, value, unit, ts}>>`。読み出し失敗は
  **数えて可視化**(silent 禁止)、再接続はドライバ内。
- 出力 3 系統(すべて既存の口):
  (a) **logbook**(§9): `psu` レコード型を **`slow`** に一般化(`device / channel / event / values`)。
  1 Hz の全サンプルは書かない — **変化閾値 or 60 s 毎 + ALARM/RECOVER イベント**。
  (b) **status PUB**(source_id 200 系、§3.2)→ monitor → WS(status JSON に `slow: {...}` を載せる、
  新バイナリ型は作らない)→ UI トレンド(ECharts、§11 に既に「レート/PSU トレンド」の枠)。
  (c) **run_start / run_stop レコード**に直近値と run 平均(`gas_pressure_mbar` 等)を載せる。
- アラーム: `[lo, hi]` 逸脱で `event: "ALARM"` + status バッジ。**DAQ は止めない**(止めるかは §3-3)。

### 2.2 Bronkhorst ドライバ(ProPar ASCII、読みのみで開始)
| パラメータ | process/param(FLOW-BUS 表記、**EL-PRESS §4 で要照合**) | 型 | 用途 |
|---|---|---|---|
| Measure | 1/0 | uint(32000 = 100 %) | 圧力(容量比) |
| Setpoint | 1/1 | uint | 現在の設定値(読むだけ) |
| fMeasure | 33/0 | float | 圧力(工学単位) |
| Capacity / Capacity unit | 1/13 / 1/31 | float / string | 単位・レンジの自動取得(焼き込み禁止) |
| Serial number / model | 113/3 等 | string | 識別(logbook に残す) |
- `B`(secured)パラメータと Control Mode には触らない。setpoint 書きは UI 要件が出てから(§3-3)。
- テスト = **pty 上の fake ProPar 応答器**(マニュアル §3.9 の例文をゴールデンに、エラー `:01xx` と
  タイムアウトも)。実機受け入れ = FlowSuite の表示値と 1 分間一致。

### 2.3 Arduino ドライバ + スケッチ
- **自前スケッチ**(`tools/arduino_env/env_json.ino`、Arduino_MKRENV ライブラリ)が USB シリアル 115200 に
  **JSON 1 行/秒**(`{"t_c":..,"rh":..,"p_hpa":..,"lux":..,"uva":..,"uvb":..}`)。WiFi は使わない(KISS、
  ネットワーク設定不要)。LabVIEW アプリは置き換え(Windows 依存を切る)。
- ドライバは行を読んで Sample に変換するだけ。フォーマット違反は数える。

## 3. 確認事項(ユーザー / ELI-NP に聞くこと — READY の条件)
1. **Bronkhorst の機種**(銘板 model key、P-5xx 計測のみか P-6xx/7xx 制御器か)、**接続**(RS232 直 /
   USB 変換 / E-8000 経由)、ノード番号、いま設定に使っているソフト(FlowSuite?)。DAQ 機まで
   RS232-USB で届くか(10 m 制限)。
2. **Arduino**: 現行スケッチのソースは入手できるか(T. Roman / A. State)。無ければ自前で書き直してよいか。
   設置場所と目的(検出器近傍の室温・気圧? 大気圧はガス圧の補正に使う?)。
3. **圧力の用途**: §1.5 により記録のみ(web から setpoint は触らない)。制御は選択肢 1(機器側)か 2(ローカル CLI)か。ALARM で run を止めるか(現状案 = 止めない)。
4. HV(HiVolta / HMP2020、P6)との統合順序(同じ slowctl に載せる前提)。

## 4. 見積もり
| 部分 | モデル | 日数 |
|---|---|---|
| slowctl 骨格(TOML / LogPost / status)+ logbook `slow` 一般化 + controller の run 記録拡張 | Opus | 2 |
| Bronkhorst ProPar ドライバ + pty fake + 単体 | Opus | 1 |
| Arduino スケッチ + ドライバ + 単体 | Sonnet | 1 |
| UI トレンド(ECharts、§11)+ status バッジ | Sonnet | 2 |
| SPEC §1.1 / §2.3 / §3 / §9 / §11 改訂 + レビュー | Fable | — |

## 5. 参照
- reference/Bronkhorst/917101-Manual-EL-PRESS.pdf(§2.8 通信、§4 パラメータ)、917027-Manual-RS232-interface.pdf
  (§3 ProPar)、917024(FLOW-BUS)、917076(E-8000)。
- reference/Arduino/(購入メール、LabVIEW アプリ)。MKR ENV Shield: https://docs.arduino.cc/hardware/mkr-env-shield
- SPEC §1.1(psu)、§2.3(LogPost)、§3.2(source_id 200)、§9.2(`psu` レコード)、§11(トレンド)。
- 071 SURVEY(CO₂ 130 mbar 流通運転、圧力計への電源干渉 p.352–353)。
