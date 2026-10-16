# 160 — ガス(Bronkhorst FLOW-BUS)を ELITPC から読む: メーターの D-sub を第 2 local host に

**Status: READY**(2026-10-16 起票 Fable。131 B-1 から独立させた — 方式が確定して着手できる作業になったため。
ユーザーは当面別作業なので、**現地に行けるときに拾う**。締め切り無し)

## 背景(確定事項、詳細は [131](131_onsite_and_hardware.md) B-1 の追記 3 本)
- FLOW-BUS の RS232 マスタ口 = MFC 1(node 4)の D-sub → ガス担当の Windows 機 COM10(FlowDDE V4.81、FlowView ×4 / FlowPlot がクライアント)。BPC(node 7)の D-sub = ガス異常時に HV を落とすインターロック線。**どちらにも触らない**。
- メーター PT104(node 10)/ PT-Inlet(node 12)の D-sub は保護キャップが被った未使用の male 9 ピン(9.17.101 §2.8.2: FLOW-BUS 接続と同時に側面 D-sub で RS-232 可)。**ここを使ってよいとガス担当が同意(2026-10-16)**。
- バス上は 8 台(最高ノード 24)。4 台は FlowView で判明(下表)、残り 4 台は Bronkhorst の制御・監視用で同じバスにいる(用途は scan 時に UserTag で)。
- Windows 機と ELITPC は同じネットワーク。配線が駄目なら代替 = Windows に FlowDDE クライアントを置いて ELITPC へ送る(131 B-1 の案 B)。
- ドライバ(ProPar codec / `bronkhorst_flowbus` device / node scan / IDENT・VALUE・ALARM / InfluxDB)は 079 Phase 1–2 と 122 で実装済み。**コードは書かない前提。** 手順の本体 = docs/NEW_DAQ_SETUP_ja.md §9「現地 Phase 0 = node scan」。

| node | UserTag | 種別 | 2026-10-15 の値 | serial |
|---|---|---|---|---|
| 4 | MFC 1 | DMFC controller、250 mln/min | 100.0 mln/min(40 %) | M21211652B |
| 7 | BPC NEW-PT-TPC | DEPC controller、0.2 bar(a) | 0.0910 bar(a) = setpoint(**TPC 圧の正本**) | M23213733B |
| 10 | PT104-TPC | DEPM meter、1.5 bar(a) | 0.090 bar(a) | M23213733D |
| 12 | PT- Inlet D V P | DEPM meter、1.5 bar(a) | 0.033 bar(a)(ポンプ入口) | M23213733F |
| ≤ 24 ×4 | ? | ? | ? | scan で |

## 用意するもの
- USB-RS232 変換(FTDI 系、Bronkhorst 9.09.122 相当。HiVolta で偽トリガを出した不良ケーブルの前例があるので**まともな個体**)。
- **Bronkhorst RS-232 専用ケーブル(7.03.367 相当)**。機器側 male D-sub の RX/TX は **pin 6 / pin 1**(9.17.027 §2.2)— PC 配列(2/3)と違うので市販ストレート・クロスは不可。無ければ 9.17.027 のフックアップ図どおりに自作(電源ピン 7 は**繋がない**)。
- **PiPS(プラグイン電源)付き T-part は使わない** — 機器は FLOW-BUS(RJ45)給電。二重給電は基板を壊す(9.17.101 §2.7)。

## 手順(現地、HV OFF・run 無し、ガス担当に一声)
1. PT104(node 10)または Inlet(node 12)のキャップを外し(嵌め込み、まっすぐ引く)、RS-232 ケーブル → USB-RS232 → ELITPC。
2. 挿した瞬間と直後 1 分、Windows の FlowDDE `Errors` カウンタと FlowView の値が変わらないことを目視(2 台目のホストがトークンリングに入る瞬間の確認)。変わるなら抜いて記録、案 B へ。
3. ELITPC: `ls -l /dev/serial/by-id/` で by-id を控える。scan 用 TOML(`[slowctl]` + 下の device、`nodes = "scan"`)で
   `RUST_LOG=info target/release/slowctl --config scan.toml` → 起動ログと logbook の `slow` IDENT レコードから **全 8 台のノード番号・機種・serial・単位・UserTag**。
   - 無応答 → その D-sub が RS485 設定。多機能スイッチ 12–16 s で RS232 へ(9.17.101 §3.6.2)。それでも駄目なら 079 §1.2 の罠 (a)(b) を順に。
   - scan の出力をそのまま `reference/Bronkhorst/scan_YYYY-MM-DD.txt` に(gitignore)。
4. 本番 `daq.toml` に device を追加(`nodes = [..]` 8 台、`expect_serial` = 挿した機器の serial、alarms)。`daq_up.sh` で slowctl を再起動。
5. **繋ぐ前後でトリガレートを比較**(NEW_DAQ_SETUP §9)。UI の Slow control タブに gas の 8 行、InfluxDB bucket `slowctl` に点が増えること。
6. 1 run(どれでも)回して `run_start` / `run_stop` の `slow` に TPC 圧と流量が載ることを確認。

```toml
[[slowctl.device]]
type = "bronkhorst_flowbus"
name = "gas"
port = "/dev/serial/by-id/usb-FTDI_..."     # 手順 3 の by-id
nodes = [4, 7, 10, 12]                      # scan 後に 8 台へ
expect_serial = "M23213733D"                # PT104 に挿したとき。Inlet なら M23213733F
alarms = [
  { node = 7, param = "fmeasure", lo = 0.085, hi = 0.097 },   # TPC 圧 91 mbar ± 6 %(単位は IDENT で確認してから)
  { node = 4, param = "fmeasure", lo = 80.0,  hi = 120.0 },   # CO₂ 100 mln/min ± 20 %
]
```
(`param` 名と単位は 079 Phase 2 の実装と IDENT の unit に合わせる。アラームは可視化だけで DAQ は止めない — HV 遮断はハードのインターロックが担う。)

## 完了条件
- [ ] 8 台の IDENT 表(node / 機種 / serial / 単位 / UserTag)をこの md と `reference/Bronkhorst/scan_*.txt` に
- [ ] FlowDDE Errors の前後、トリガレートの前後(数値)
- [ ] 本番 TOML の device 行(by-id、nodes、expect_serial、alarms)と、run_start に載った `slow` の例
- [ ] RUNBOOK §4 の機器 × ch 表に gas を追加、131 B-1 を ✔
- [ ] 後続の起票判断: ①`v_drift = 0.7233 × E/p` を run_start の圧力から自動計算(解析バックログ B-4)②露点計(Michell Easidew、COM6、Modbus RTU)を読むか

## 実行できなかった操作
なし(起票のみ)。

## 参照
131 B-1(2026-10-15/16 の追記 3 本)、archive/079(§1.2 配線・罠、§1.5 安全境界、Phase 1–2 実装)、archive/122(InfluxDB)、docs/NEW_DAQ_SETUP_ja.md §9、docs/RUNBOOK_ja.md §4、reference/Bronkhorst/(9.17.101 / 9.17.027 / 9.17.024、screenshot_2026-10-15_gas_pc.png、IMG_3204–3208)。
