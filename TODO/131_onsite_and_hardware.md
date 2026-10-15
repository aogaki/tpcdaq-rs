# 131 — 現地・機器・ELITPC 実機でやること(069 / 073 / 079 / 084 / 101 / 107 の残りを 1 本に)

**Status: READY**(2026-10-13 起票 Opus。ユーザー指示「130 は後回しにして、69 などのスタックしているのを確認して解決していきたい」→
棚卸しで、止まっていたチケットの残りはほぼ全部「人が現地にいる / 機器を触る / ELITPC で run を止める」ものだった。
ユーザー裁定(同日): **1 本にまとめ、元のチケットは処置を書いて閉じる**)

各項目は独立。やれるときに 1 つずつ。終わったら ✔・日付・結果(数値)をこの md に書く。全部済むか、残りが不要になったら archive。

## A. ELITPC で run を止めてやること(遠隔で可、ユーザーの了解を取ってから)

| # | 項目 | やり方 | 元 |
|---|---|---|---|
| A-1 | **Arm の bind 競合リトライの実測** | 10-08 以降の ELITPC のログブック(audit `run/start`)から `ArmRetry{attempts, waited_ms}` を全 run 分集計するだけ(run は止めなくてよい)。仮想の実測(最悪 4 試行 / 157 ms)から桁で外れないこと | 069 #8 |
| A-2 | **run 開始失敗時に `next_run` が戻るか** | わざと 1 回失敗させる(例: params の render ができない current.json、または ECC を止めた状態で run/start)→ 番号が消費されないこと、500 が連発しないこと。統合テスト(`a_failed_start_rolls_back_and_leaves_no_half_started_run`、034)は済んでいるので実機の確認だけ | 069 #9 |
| A-3 | **24 h soak(ハード込み)** | periodic トリガ(例 10 Hz)で 24 h。031 と同じ様式で RSS / fd / カウンタを定期サンプリングし、後半半分で単調増加が無いこと。13.5 h の α run(107 の run 2)は成立済み — 足りないのはサンプリング付きの 24 h | 069 #21 |
| A-4 | **systemd ユニットと ufw** | 今は `daq_up.sh` / `daq_down.sh` の手動起動。docs/NEW_DAQ_SETUP_ja.md には方針(systemd ユーザーユニット、`After=` で順序、root_sink は `KillSignal=SIGINT`)だけで、ユニットの実体は未作成。ユニットの文案はこちらで書ける、有効化と ufw は sudo なのでユーザー作業 | 107 残り 7 |

## B. 機器・配線(現地)

| # | 項目 | 中身 | 元 |
|---|---|---|---|
| B-1 | **ガス(Bronkhorst FLOW-BUS)の読み出し** | Windows 側で FlowSuite / FlowPlot と LabVIEW の両方が動いている。COM ポートは排他なので仲介役(FlowDDE 等)がいるはず → ①Windows のポートと仲介役を確認 ②Windows 上の追加クライアントで読むか、別機器の D-sub を第 2 の local host にして読むだけにするか(079 §1.5 案 1)を決める ③node scan で各ノードの識別群 + 計測値のゴールデンを reference/ に。ドライバ(ProPar codec)は 079 Phase 1–2 で実装済み | 079 Phase 0 |
| B-2 | **Arduino MKR1000 + ENV shield の設置** | スケッチとドライバは 079 Phase 3 で実装済み。設置場所(検出器近傍? 室温・大気圧の用途)を決めて USB で ELITPC へ、slowctl の device に足す | 079 Phase 0、080 C-2 |
| B-3 | **GEM 信号トリガの測定**(現行 = CFD のまま、ユーザー確認 2026-10-13) | archive/084 §4 の 4 項目: ①フィルタボックス出力の波形(オシロ、数十イベント)で立ち上がり時間の分布 ②閾値 2〜3 点の turn-on 曲線 ③トリガ遅れ L ④12.5 MHz + delay 3600 で窓に収まるか。あわせて MSCF-16 の型番(F / LN / PMT)・CFD/LE ジャンパ・遠隔制御の接続を確認(084 §6)。CFD は µs の信号に原理的に合わない(084 §2)ので、作り直すなら A 案(積分 + LE)が候補 | 084 |
| B-4 | **前面スイッチ OFF での HV ON 拒否(DIS)** | 任意。124 の実機受け入れで残した 1 項目 | 124 |

## C. 運転条件の判断(ユーザー)

| # | 項目 | 中身 | 元 |
|---|---|---|---|
| C-1 | ~~取得を 25 MHz に戻すか~~ **済**(2026-10-13 に判明) | ユーザーが 2026-10-11 の run 57 から Parameters で **25 MHz / triggerDelay 1500**(params `elinp__dcdaf9ca`)に切り替え済みだった(archive/113 の反映節、ELITPC の current.json で確認)。macros 側の samplingMHz はユーザーの macros の話 | 101 |

## D. 人に頼むもの(任意、あれば嬉しい)

- physics run 追加数本のコピー(別日・別 trigDelay)— 073 #2。
- MAGBOLTZ22 の出力(運転点表の元、D_L / D_T の一次値)— 073 #3。拡散は熱限界一致まで導出済みなので照合用。
- HIGS の DCS / InfluxDB エクスポート(physics run 日の圧力・温度・HiVolta 電流)— 073 #4。
- ログブック原本の写真(p.030、p.123–126、p.254/257–259)— 073 #6。

## 参照
archive/069(Phase 2〜4)、archive/073、archive/079(§1.5、Phase 0)、archive/084、archive/101(追記 3)、archive/107(残り)、124。

- **(2026-10-14 追加)ドリフト端 / v_drift の実測**: α 線源(読み出し面近く)の飛跡はトリガから 2–4 µs しか占めず、陰極(20 cm)に届かない(run 111 / 113、12.5 MHz)。宇宙線 run(自トリガは FW 非対応なので外部トリガ or periodically + オフライン選別)か、線源を陰極側に置いて t_max を測る。道具は `tools/scan/drift_end.C`。

## B-1 追記(2026-10-15、Fable): ガス PC のスクリーンショットで確定したこと

ガス担当から入手した Windows 機の画面(`reference/Bronkhorst/screenshot_2026-10-15_gas_pc.png`、gitignore)。079 §1.2 の「現地 Phase 0 で決める」3 点のうち **先頭機器・ノード番号・RS232 の相手** が埋まった。

### 確定(画面の読み取り)
- **仲介役 = FlowDDE V4.81**(`Interface: RS232-MBCFh at node 4, COM10 38400,n,8,1`、2026-09-29 09:58 起動のまま 16 日連続稼働、Errors 2、176 msg/s)。FlowView ×4 と FlowPlot V3.35 はその DDE クライアント。**FLOW-BUS の local host = MFC 1(node 4)の D-sub → Windows の COM10**。これで 079 §1.5 の「衝突点 = RS232 マスタ 1 本」の実体が決まった。
- **FLOW-BUS 上の機器は 8 ch、最高ノード 24**(FlowDDE ログ「Highest occupied node address = 24」「total channels = 8」)。画面に出ているのは 4 台だけ。残り 4 台(ノード 13–24 のどこか)は不明 — serial が `M23213733B/D/F` と同ロットなので、同時購入の EL-PRESS(A/C/E)がポンプ側に付いている可能性。**現地の node scan で確定**(ドライバの `nodes = "scan"` は 3..120 を舐めるので約 24 s。確定後は明示リストにする)。

| FlowDDE ch | node | UserTag | 型 | Model | 現在値(2026-10-15 13:58) | 設定値 | 備考 |
|---|---|---|---|---|---|---|---|
| 1 | **4** | `MFC 1` | DMFC(controller) | F-201CV-500-RBD | 40.0 % = **100.0 mln/min** | 40.0 % | CO₂ 流量。レンジ 250 mln/min(customer range、STANDARD)。serial M21211652B |
| 2 | **7** | `BPC NEW-PT-TPC` | DEPC(controller) | P-502C-350A-SBD | 45.5 % = **0.0910 bar(a)** | 45.5 % = 0.0910 bar(a) | **TPC 圧 91 mbar の正本**(macros の `--pressure 91` と一致)。レンジ 0.2 bar(a)。valve out 34.95 %。serial M23213733B |
| 3 | **10** | `PT104-TPC` | DEPM(meter) | P-502C-6K0A-RBD | 6.0 % = **0.090 bar(a)** | — | TPC の独立圧力計。レンジ 1.5 bar(a)。serial M23213733D |
| 4 | **12** | `PT- Inlet D V P` | DEPM(meter) | P-502C-6K0A-RBD | 2.2 % = **0.033 bar(a)** | — | ポンプ(dry vacuum pump)入口圧。レンジ 1.5 bar(a)。serial M23213733F |
| 5–8 | ≤ 24 | ? | ? | ? | ? | ? | 未表示。node scan で |

- 全機 `Setpoint/Control Mode = 0. Bus/RS232` → setpoint はバス経由(FlowView 等)で与えられている。ガスの流れは MFC1(100 mln/min)→ TPC(91 mbar、BPC が背圧保持)→ ポンプ入口(33 mbar)。
- FlowView の `SerialNr` 欄の `SN` は表示の接頭辞で、ProPar 113/3 で返る serial は `M21211652B` 形式(`expect_serial` にはこちらを書く)。
- **他にシリアルが 2 本動いている**(どちらも FLOW-BUS とは別の相手。何に繋がるか未確認):
  - LabVIEW `control_presiune_pompa_tpc.vi`(= TPC ポンプ圧制御)が **COM7**。ポンプか別の圧力コントローラ。
  - Baseblock ComTest Pro(Modbus RTU テストツール)が **COM6 9600 8N1、slave 1、function 04、register 0 を 5 s ごとに読む**(応答 `FD37`、当日 12:37 頃から 976 行)。担当者が今日 Modbus 機器を試していた。Bronkhorst の既定 Modbus(19200 8E1)とは合わないので、Bronkhorst ではない別機器とみる。
  - 079 の写真(IMG_3206)で BPC の D-sub にもケーブルが刺さっていた。**COM6 / COM7 のどちらかが BPC の D-sub か**を現地で確認(BPC の D-sub が空いていれば第 2 local host の候補、塞がっていれば PT104 か Inlet の D-sub)。
- `MKR1000 + ENV Shield.exe`(LabVIEW)も同じ PC で稼働中(室温 21–23 ℃、湿度 40–65 %、気圧 99.5–103 kPa)。B-2 の Arduino は**この個体を抜くと担当者の画面が消える**ので、別個体を用意するか担当者と相談。

### 2026-10-15 ユーザー訂正: 機器側に空き D-sub は無い
- MFC 1 の D-sub → Windows PC(COM10、FlowDDE)。
- **もう 1 本の D-sub(BPC と推定)→ ガス異常時に HV を落とすインターロック**。これは安全系の配線なので DAQ の都合で触らない。
- → 079 §1.5 案 1(機器の D-sub を第 2 local host にする)は**不成立**。

### 読み出しの方式(改訂、裁定はユーザー)
前提: FLOW-BUS の RS232 マスタ口は塞がっている。残る入口は「D-sub の RS232 ピンの共用」「Windows 側の仲介役」「RJ45 側に新しい口」の 3 つ。

| 案 | 中身 | 要るもの | 影響 | 評価 |
|---|---|---|---|---|
| **A. T-part で RS232 ピンを分ける** | Bronkhorst の 9 ピン D-sub は**電源・アナログ I/O・RS232 を 1 つのコネクタに同居**させている(9.17.101 §2.8 ピン配置)。インターロックがアナログ出力(または電源)ピンしか使っていなければ、同じ D-sub の **RS232 ピン(TxD/RxD)は電気的に空いている**。Bronkhorst 純正の **T-part(079 §1.2 の 7.03.366 相当 = アナログと RS232 を 1 口から 2 方向に分ける部品)** を挿し、片側に既存のインターロック線、片側に ELITPC の USB-RS232。slowctl の `bronkhorst_flowbus` はそのまま(node 128 = その機器、4/7/10/12 を中継で読む) | T-part 1 個(ケーブル改造はしない)。その機器の D-sub 通信モードが RS232 であること(RS485 側なら多機能スイッチで切替 — 9.17.101 §3.6.2) | **インターロック線に手を触れる**。HV OFF・ガス停止中に、担当者立会いで。アナログ出力が通信モード切替で一瞬揺れるかは要確認(揺れるならインターロックが落ちるだけで安全側) | ソフト変更ゼロで本命。**ただし「インターロック線が D-sub のどのピンを使っているか」が決まるまで着手しない** |
| **B. Windows に FlowDDE クライアントを 1 本足す** | FlowDDE は複数クライアント可。pywin32 の DDE で 8 ch の measure/setpoint/valve を 1 Hz で読み、ELITPC へ JSON 行(TCP)で送る。slowctl 側は `arduino_env` の JSON 行パーサを **TCP 入力で再利用**する新 device 型(SPEC §1.1/§9 の小改訂) | Windows 機 ↔ ELITPC のネットワーク到達性、担当者の PC に常駐スクリプトを置く合意 | 配線に触らない。Windows 機が落ちる/再起動すると値が止まる(slowctl は「停止」を ERROR として可視化 — silent にしない) | 配線の合意が取れないときの第 1 候補 |
| **C. RJ45 側に新しいホスト口** | Bronkhorst の FLOW-BUS 用インターフェース(RS232/FLOW-BUS 箱 9.17.011、または USB/FLOW-BUS)を Y アダプタで RJ45 チェーンに足し、それを第 2 local host にする | 購入(現行製品の有無と価格を Bronkhorst に確認)+ Y アダプタの空き | 既存のケーブルに触らない。**RJ45 の抜き差しは給電を兼ねるので他機が落ちる**(079 §1.2 罠 (c))— 停止中に | 金で解く案。納期があるので、A/B が駄目なら早めに発注 |
| D. RS232 の受動傍受 | MFC1 ↔ PC の TxD/RxD を両方向とも聞くだけ(受信器 2 つ)。FlowDDE の問い合わせと応答を対にして値を復元 | USB-RS232 ×2 + 傍受ケーブル、ProPar の「傍受モード」実装 | バスには一切影響しない。実装がハックで silent failure の温床 | 最後の手段。推奨しない |

### 残り(現地)
- [ ] **インターロック線の相手と使用ピン**(BPC の D-sub からどこへ行き、アナログ出力か・リレーか・電源込みか)を担当者に聞く → A の可否。あわせて「ガス異常」の判定(圧力の閾値? 誰が判定?)を記録(SPEC §11 の安全境界の一次資料)
- [ ] PT104 / Inlet の D-sub は**物理的に**存在するか(Bronkhorst のデジタル機は全機 D-sub を持つ。FLOW-BUS 給電で未使用なら、インターロック線に触らず A ができる)
- [ ] Windows 機のネットワーク(ELITPC 192.168.144.220 に届くか)→ B の可否
- [ ] 残り 4 ch(ノード 13–24)の正体、COM6(Modbus)/ COM7(LabVIEW ポンプ制御)の相手
- [ ] 入口が決まったら node scan のゴールデンを `reference/Bronkhorst/scan_YYYY-MM-DD.txt` に

### ELITPC 側の設定案(A または C のとき。B は別 device 型)
```toml
[[slowctl.device]]
type = "bronkhorst_flowbus"
name = "gas"
port = "/dev/serial/by-id/usb-FTDI_..."     # 現地で by-id を確認
nodes = [4, 7, 10, 12]                      # scan で残り 4 台を確定したら足す
expect_serial = "M23213733B"                # 挿した機器(= node 128)の serial(A で BPC なら B、C なら箱の serial)
alarms = [
  { node = 7, param = "fmeasure", lo = 0.085, hi = 0.097 },   # TPC 圧 91 mbar ± 6 %
  { node = 4, param = "fmeasure", lo = 80.0,  hi = 120.0 },   # CO₂ 流量 100 mln/min ± 20 %
]
```
(param 名と単位は 079 Phase 2 の実装に合わせる — 現地で 1 回 scan して IDENT の unit を見てから。)
