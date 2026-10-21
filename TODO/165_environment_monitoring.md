# 165 — 環境モニタ: 露点・外気温(室温・湿度・気圧)・チャンバー温度・チャンバー圧を slowctl へ

**Status: READY**(2026-10-21 起票。ユーザー「露点と外気温、欲を言えばチャンバーの温度が必要ですね。もちろんプレッシャーも」)

## 動機
161 の追記 2・3: 流量の効果(50 ml/min で利得 −3.5 %)と、温度 1 K の効果(利得 ≈ 1.5 %/K、密度 0.35 %/K)が同じ桁。run 178 の差は温度 +1.3 K(密度 −0.45 %)と読んだが、温度・気圧の記録が無いので α 線の止まる位置から逆算するしかなかった。露点は別 PC の通信ソフトの生ログで、run と突き合わせるのに日付の復元が要った。
→ 4 つの量を slowctl で読み、HV と同じく InfluxDB に常時残し、run_start / run_stop の `slow` に直近値を載せる。解析(`gas_trend.C`、metrics)で run ごとの T / p を使い、利得を T/p で補正できるようにする。

## 現状と残り
| 量 | 機器 | slowctl | 残り |
|---|---|---|---|
| チャンバー圧 | Bronkhorst PT104-TPC(node 10、独立圧力計、0.090 bar(a)、131 B-1 の表) | `bronkhorst_flowbus` 実装済み(079) | 配線(131 B-1: ガス PC の FLOW-BUS をどう分けるか。メーターの D-sub に RS-232 で第 2 local host、が第一案) |
| 室温・湿度・気圧 | Arduino MKR1000 + ENV シールド | `arduino_env` 実装済み(079 Phase 3) | 設置(131 B-2)。ガス PC の個体は担当者の LabVIEW 画面用なので別個体を用意 |
| 露点 | Michell Easidew Online(RS485、Modbus RTU、9600 8N1、addr 1) | **無い** | ドライバ新規(下の発注書)。今の RS485 線はガス PC の COM6(Baseblock ComTest Pro で手動ログ)— ELITPC へ移すか分岐するかを現地で決める |
| チャンバー温度 | **無い** | 無い | センサー選定(例: チャンバー外壁に PT100 + MAX31865 を Arduino で読む、またはガス出口配管に挿入型)。ユーザー判断 |

## 先にやれる確認(ユーザー / ガス系の担当者)
- ガス PC の LabVIEW「MKR1000 + ENV Shield.exe」が室温・気圧をファイルに残していれば、10-16 〜 10-21 の室温で run 178 の +1.3 K を直接確かめられる。
- FlowPlot / FlowSuite が PT104-TPC(チャンバー圧)を記録していれば、圧力の線(−0.4 mbar 相当)も消せる。

## 発注書(露点ドライバ、起票のみ — 配線の決定後に Sonnet へ)
- `src/slowctl/easidew.rs`: Modbus RTU 読み取り(FC 4)。レジスタ 0 = 表示値(int16)、レジスタ 8 = 小数点の位置(0–3)、レジスタ 7 = エラー(bit0 断線、bit1 上限超え、bit2 下限割れ)。CRC16(Modbus)。手順書 = `reference/due_point/Procedure Dew Point.pdf` p.37–38。
- 値 = レジスタ 0 / 10^(レジスタ 8)、単位は表示単位(現用は霜点 °C)。エラー ≠ 0 は値を出さずに ALARM(黙って捨てない)。応答なし・CRC 不一致は数えて WARN。
- 設定: `[[slowctl.device]] type = "easidew"`、`port`(/dev/serial/by-id/…)、`addr`(既定 1)、`poll_s`(既定 5)、`expect_serial` は無し(機器が持たない)ので by-id パスで固定。
- InfluxDB: measurement `env`、field `dewpoint_c`(+ `dewpoint_err`)。換算した `h2o_ppm`(チャンバー圧で測定 — ユーザー確認 2026-10-21 — 圧力は bronkhorst の値があればそれ、無ければ設定値)を出すかは解析側で決める(ドライバは生値だけ)。
- テスト: 手順書の例(要求 `01 04 0000 0001 31CA`、応答 `01 04 02 0402 3A31` → 102.6、65270 → −26.6)と、実ログの応答 `[1][4][2][253][43]` → −72.5 を固定。エラー bit と CRC 不一致の負性テスト。fake デバイスで component のルーティングまで。

## 結果
(着手時に記入)

## 実行できなかった操作
(着手時に記入)
