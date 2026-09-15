# 080 — 次の mini TPC 実験に向けた「自前で詰め切る」プログラム(Mikolaj 非依存)

**Status: READY**(2026-09-15 起票 Fable。発端 = ユーザー方針「Mikolaj は mini TPC にかかりっきり、
次の実験でも mini を使う → 逆にこちらでやれることをやりまくる TODO を作りたい」。
**同日ユーザー裁定 5 点(§3)で READY**。本票はプログラム(束)であり、実行は既存チケット
(074/076/077/078/079)と本票から切り出す新規ユニットで行う — 連番ユニットの粒度ルールは変えない。
**着手は Wave 単位でユーザー指示**)

## 0. 前提(棚卸し 2026-09-15、TODO + archive 000–075 + CURRENT 保留/Fable 待ち + WARSAW_PLAN §6–7)

- **mini eTPC は自前の機械**。Warsaw 待ちだった 3 点は全部こちらで閉じられる:
  ①応答パラメータ受領(062)→ **069 Phase 2.5 の較正メニュー(C-1〜C-5)で自分で測る**
  ②ゲイン較正ツール(061)→ **内製で行く(062 裁定済み)。AsAd 内蔵パルサー + 068 の解析コード**
  ③TPCReco 再配布(020)→ 影響は third_party 昇格の可否だけ。**reference の TPCReco = 現地と同一**
  (ユーザー確認 2026-09-15)なので互換ターゲットは固定済み。
- **100 Hz は mini で達成済み**(064、SPEC v1.21)。ELITPC 性能は追わない(2026-08-18 裁定)。
- ハード: 実 zCoBo + GDED(daq@192.168.144.101、Ubuntu 24.04)は **SSH で遠隔 run 可**(2026-09-14
  実証、メモリ geteccserver-native-gded)。GDED に ROOT は無い。**新 DAQ 機(Ubuntu 24.04 LTS)を新設予定**。
- 現地が要るのは「ケーブルを挿す」(079 Phase 0)と「線源・ガスを入れて run する」(C-3〜C-5)だけ。
  それ以外は Mac(開発・リプレイ・仮想スタック)と GDED(実機 run、内蔵パルサー)で進む。
- **実験(ユーザー 2026-09-15)**: チャンバー内に **¹⁰B ターゲット**を置き γ 線を当て、出てくる **α を計測**。時期未定。
  いまは線源(²²⁸Th 密閉線源)を外して**バックグラウンド測定中**、遠隔で大抵のことは可能。
- **ガス圧の検討は別リポ `~/test/gas-pressure-test`**(Geant4 + Garfield++ + AGET 応答、CO₂ 140–300 mbar を
  スキャン中、²²⁸Th スペクトル・窓薄膜・GEM 実効ゲイン 3600 較正済み)。**Stage 4 の出力 `_raw.root` は
  075 の raw ツリーと同一契約**(`eventId/i`、`eventTime/l`、`adc[4][68][512]/S`)で、我々の macros を
  `external/tpcdaq-macros/` として使っている → **シミュレーションと DAQ が同じ解析チェーンに乗る**。
  含意: ①074 QA reco のフィクスチャに真値付き合成 raw が使える ②D-5 Unfolding の順方向応答
  (Magboltz 拡散 ⊗ AGET 整形 223 ns)は既にある ③運転圧は run 毎に変わりうる(100 → 140–300 mbar)ので
  **B-4 の v_drift 自動導出は必須**(.dat の 0.724 焼き込みだと z スケールが圧に追随しない)。
- **新 DAQ 機は来週からインストール・セットアップ開始**(ユーザー 2026-09-15)→ A-1 手順書は今週。
- **mini の HV も HiVolta、AsAd 電源も同じ R&S HMP2020**(ユーザー 2026-09-15)。**資料は揃った(同日)**:
  `reference/HiVolta/`(CAEN DT1415ET 取説 rev16 — `~/Workspace/TPC/TechnicalReports/HiVolta CAEN.pdf` から複写。
  ASCII プロトコル §Remote Control: `$CMD:MON,CH:X,PAR:VMON|IMON|STATUS` → `#CMD:OK,VAL:*;*;…`、SET は LOCAL で
  `#LOC:ERR` = **監視は LOCAL のまま無干渉**、STATUS bit 3 OVC / 4 OVV / 5 UNV、`PAR:BDILK` interlock。
  + GET-AS-002-0004 AsAd Power Supply & Monitoring = AsAd 側の電圧系 VDD/VDD1 3.3 V、VDD2 2.5 V、VDD3 1.5 V、
  リモートセンシング必須)/ `reference/HMP2020/`(HAMEG HMP シリーズ SCPI Programmers Manual + R&S HMP ユーザーマニュアル、
  公開 PDF を取得。**mini の HMP2020 は現物確認で LAN ポート無し = 標準 HO720(RS-232 / USB)**(ユーザー 2026-09-15)。
  → 接続は **USB(CDC = 仮想 COM、Linux では /dev/ttyACM*。USB-TMC クラスでも可)か RS-232**(CTS/RTS ハンドシェイク推奨、
  無しなら 50–100 ms の間隔)。コマンドは同じ: `INST:NSEL n` → `MEAS:VOLT?` / `MEAS:CURR?`、`OUTP?`、
  `STAT:QUES:ISUM<n>:COND?`、終端 LF か CR-LF、`SYST:MIX` で前面パネル併用。HMP2020 は ch 1–2 のみ。
  073 #7 の netstat の LV SCPI(.20/.21:5025)は ELITPC 側の別個体(HO732 付き)で、mini の個体とは別。
  DAQ 機には **USB-B ケーブル 1 本**(Bronkhorst の USB-RS232 と並んでシリアル 2 本目)。

## 1. 項目(実験で困ることから逆算。★ = 今すぐ・誰も待たない)

### A. 本番機に載せる(全部の土台)
| # | 項目 | 元 | モデル / 場所 | 目安 |
|---|---|---|---|---|
| A-1 | **新 DAQ 機セットアップ手順書 + 実施**: OS → ROOT → Ice 3.7 で ECC 再ビルド → tpcdaq-rs / UI ビルド → 常駐化(systemd)→ 疎通。順序は 076 §6 が正 | 076 §6、メモリ geteccserver-native-gded | Fable 手順書(★)→ 実施は新機到着後 | 手順書 半日、実施 1 日 |
| A-2 | **076 Rust Ice client**(Ice 3.8 依存の切断。C++ 撤去は実機確認後) | 076 READY | Phase 0 Sonnet + Phase 1 Opus(★、Mac の ECC 3.7 ローカルビルド + vcobo でゴールデン) | 1 週 |
| A-3 | **069 の残り**: #7 連続 ×10 / 異常系 #8–#10, #13, #14 / #16 オラクル照合 / Phase 4(worker 数・ディスク 28 MB/s・24 h soak・RSS と `--queue` バイト建て裁定) | 069 | GDED 遠隔(zCoBo が空いているとき)→ 新機で再測 | 2〜3 日 |
| A-4 | **復旧リハーサル**(停電・zCoBo 再起動 `S99gethwserver` / ECC halt からの歩き戻し / receiver 余分接続) | 069 #14、SURVEY §4、CURRENT 保留(ECC 例外 2 箇所) | GDED 遠隔 | 半日 |

### B. 運転面(オペレータが実験中に触る所)
| # | 項目 | 元 | モデル / 場所 | 目安 |
|---|---|---|---|---|
| B-1 | **078 パラメータ UI(JSON → xcfg)+ ラン種別プリセット**(pedestal / pulser / 較正ランプ / physics を 1 クリックで。072 §3.3–3.4 の 3 相 ConfigId 組) | 078 READY | Opus レーン A 先行(★) | 2 週 |
| B-2 | **077 AsAd 272×512 2D** | 077 READY | Sonnet(★) | 半日 |
| B-3 | **074 QA 再構成(simplefit → monitor)**: 電場スキャン 4 指標をライブ表示 = 条件出しを「その場」に | 074 DRAFT、SPEC §5.5 | Fable 発注書化 → Opus | 1 週 |
| B-4 | **run 条件の自動記録 + v_drift 導出**: 圧力(079)+ HV 設定(手入力 or C-3)→ `v_drift = 0.7233 × E/p`(CO₂、SURVEY §2)を run_start に書き、monitor/macros の z スケールが run 条件から決まるようにする(.dat の 0.724 焼き込み事故の防止) | メモリ minitpc-analysis-pipeline、SPEC §9 | Opus(小) | 1 日 |
| B-5 | **RUNBOOK_ja.md**: 起動/停止順序(SURVEY §4)、zCoBo 再起動、復旧、ラン手順(pedestal → pulser → physics)、チェックリスト、069 0-3 のリハーサル記録 | 069 0-3、071 SURVEY §4、072 §4 | Fable 骨子(★)→ Sonnet 肉付け | 1 日 |

### C. スローコントロール(監視のみ、SPEC §11 安全境界)
| # | 項目 | 元 | モデル / 場所 | 目安 |
|---|---|---|---|---|
| C-1 | **079 Phase 1–2**(ProPar codec + slowctl 骨格 + logbook `slow`)。Phase 0(node scan)は現地でケーブルを挿すとき | 079 READY | Sonnet(★)+ Opus | 3 日 |
| C-2 | 079 Phase 3 Arduino 自前スケッチ + ドライバ | 079 | Sonnet(MKR1000 が手元にあれば Mac で) | 1 日 |
| C-3 | **HV / LV 監視**: HiVolta DT1415ET(TCP :1470 ASCII、LOCAL のまま `$CMD:MON` で読むだけ、STATUS の OVC/OVV/UNV/TRIP と interlock を記録)+ **R&S HMP2020**(AsAd 電源、**HO720 = USB CDC / RS-232 のシリアル SCPI**、`MEAS:VOLT?/CURR?` + `OUTP?`、ch 毎の V/I と出力 ON/OFF を記録)。どちらも書き込み API を持たない | SPEC §13-8(P6)、reference/HiVolta、reference/HMP2020 | Opus(★ 資料は揃った。fake は TCP 応答器 / pty で縛れる — 079 の ProPar fake と同じ流儀) | 2 + 1 日 |
| C-4 | UI トレンド(圧力・流量・HV・環境)+ ALARM バッジ | 079 Phase 4 | Sonnet | 2 日 |

### D. 較正・解析プロダクト(Warsaw 待ち 3 点の自前化 — 059 優先順位 Unfolding > ゲイン正規化 > ZS)
| # | 項目 | 元 | モデル / 場所 | 目安 |
|---|---|---|---|---|
| D-1 | **較正表ツール化**: 068 のサーベイコード(reference/_spike/asset_survey)を `tools/calib` に昇格 — pedestal run → per-ch baseline/noise、pulser run → per-ch 相対ゲイン、出力 JSON(異常 ch は中央値 + k·MAD で機械判定、k を記録) | 068 結果節 | Sonnet(★、実データはローカル) | 1 日 |
| D-2 | **較正表の適用**: ①per-ch 閾値の個別化(onMultiplicity 自トリガの閾値 → 078 の「例外リスト」に流し込む)②ゲイン正規化(macros / ROOT プロダクト側。生 graw は不変 — 059 大原則) | 059、068「恒常異常 7 ch は閾値個別化」 | Opus | 2 日 |
| D-3 | **パルサー振幅スキャン**(073-#10)を **GDED 遠隔で実施** → ゲイン直線性 + **電子回路応答の平均波形**(066(b) 前半)。configure-pulser.xcfg の amplitudeStart/Stop/Step を変えた別名ファイル(072 流儀) | 073 #10、066(b)、062 ②、069 C-2 | ユーザー + Sonnet 解析 | 半日 + 1 日 |
| D-4 | **069 Phase 2.5 C-3〜C-5**(α 線源/宇宙線トラック → σ_T・σ_L・strip PRF・イオンテール平均波形 / v_drift = L/t_max / ADC/keV) | 069 Phase 2.5 | **現地 or 遠隔(§3-2 次第)**、解析は 066 ハーネス(mini .dat 切替 = 069 0-5 は★で先に) | run 半日、解析 2 日 |
| D-5 | **イオンテールモデル + Unfolding 本実装**(066(b) 後半。応答 = C-2 の電子回路応答 ⊗ 拡散 ⊗ イオンテール、逆問題として ROOT プロダクトに追加) | 066 裁定、059 ① | D-3/D-4 の後、Fable 設計 → Opus | 2 週 |
| D-6 | **logbook JSONL → minitpc-analysis `runs.yaml` 生成**(run 条件の唯一の情報源を DAQ 側に。Higs リポは読み取り専用なので出力ファイルを渡すだけ) | メモリ minitpc-analysis-pipeline、SPEC §9 | Sonnet(★) | 半日 |
| D-7 | macros 配布準備(LICENSE / channel_map.csv の扱い / ビルド産物 .gitignore) | CURRENT 保留 | Sonnet | 半日 |
| D-8 | **シミュレーション ↔ DAQ の契約共有**: gas-pressure-test の `_raw.root` を tpcdaq の合成フィクスチャ供給元にする(074 の真値付きテスト、D-5 の応答カーネル、B-4 の v_drift 照合 = Magboltz vs `0.7233×E/p`)。raw ツリー契約(SPEC §6.7)を変えるときは両リポ同時 | ~/test/gas-pressure-test、075 | Fable(契約)+ Sonnet(フィクスチャ生成手順) | 半日 |

### E. 073 の残骸(現地フェーズ終了 → 本票で吸収、073 は結果節付きで archive)
- **済**: #1 patched-v2 差分 / #7 netstat / #8 dataRouter / **#5(ROOT 6.08.06 想定どおり、TPCReco = reference と同一 — ユーザー確認 2026-09-15)**。
- **メールで済む**: #3 Magboltz テーブル(D_L/D_T 一次値。SURVEY §1 で熱限界一致まで導出済みなので「あれば嬉しい」)/
  #9 Mikolaj 3 点 → **応答ファイルは「受領できれば照合に使う」に格下げ**(D-4/D-5 で自前化)。
- **遠隔で済む**: #2 physics run 追加コピー(DAQ PC から scp を依頼)/ #4 InfluxDB エクスポート(GDED から 8086 に届くか)/
  #10 パルサー振幅スキャン → **D-3 に移管**(ビーム窓不要、内蔵パルサーで遠隔可 — 2026-09-14 実証)。
- **破棄**: E 節(ラップトップ直結ベンチ)= GDED 遠隔ベンチで用済み。#6 ログブック原本写真は現地の誰かに依頼可(SURVEY §5-2)。

## 2. 推奨順(案 — §3 の裁定で組み替える)

- **Wave 1(今週、並列、誰も待たない)**: **A-1 手順書(Fable、新機が来週なので最優先)** / B-2 077 / C-1 079 Phase 1 /
  A-2 076 Phase 0+1(新機で最初に試す — 076 裁定)/ D-1 較正表ツール / D-6 runs.yaml 生成 / D-8 契約共有 /
  069 0-5 mini ジオメトリ切替スモーク / B-5 RUNBOOK 骨子(Fable)。
  → Sonnet 3〜4 レーン + Opus 1 レーン、Fable レビューはフェーズ境界 1 回ずつ。
- **Wave 2(GDED が空いているとき — 「大抵のことは可能」)**: D-3 パルサースキャン(遠隔)/ A-3 069 残 / A-4 復旧リハーサル。
  並行して Mac で B-1 078 と B-4 run 条件 → v_drift。
- **Wave 3(来週〜、新 DAQ 機)**: A-1 実施(ROOT → ECC 3.7 → tpcdaq-rs)/ 076 実機検証 → C++ ecc_bridge 撤去 / 069 Phase 4 / 079 Phase 0(ケーブル)/ C-3 HV。
- **Wave 4(線源を戻して run できたら)**: D-4 → D-5 Unfolding / B-3 074 QA reco(D-4 のデータ + D-8 の合成 raw で受け入れ)。

## 3. ユーザー裁定(2026-09-15 — READY の根拠)
1. **実験**: 時期未定。チャンバー内の ¹⁰B ターゲットに γ 線 → 放出 α を計測。ガス圧の検討は `~/test/gas-pressure-test`。
2. **遠隔 run**: いまは線源を外してバックグラウンド測定中だが、**大抵のことは可能**(D-3 は即、D-4 は線源を戻すとき)。
3. **新 DAQ 機**: **来週からインストール・セットアップ開始**(Wave 3 = 来週)。
4. **HV**: mini も **HiVolta**。
5. **優先順位**: 変更なし(§2 の案どおり)。

## 4. 見積もり(合計、直列換算)
Wave 1 ≈ 1.5 週(並列なら 3〜4 日)/ Wave 2 ≈ 2.5 週(078 支配)/ Wave 3 ≈ 1 週 + 新機待ち / Wave 4 ≈ 3 週。
**実験までに最低限**: A-1 + B-1(プリセットまで)+ B-5 + C-1 + D-1/D-2 — 圧力が記録され、ラン種別がワンクリックで、
較正表が出て、手順書がある状態。Unfolding(D-5)と QA reco(B-3)は「実験中に育てる」扱いでも成立する。

## 5. 参照
069(Phase 0/2.5/4、問い Q1–Q4)、073(残骸)、074/076/077/078/079、archive 059/061/062/064/066/068/071/072/075、
docs/WARSAW_PLAN_ja.md §6–7、docs/XCFG_TUNING_GUIDE_ja.md、reference/logbook/_survey/SURVEY.md、
メモリ minitpc-analysis-pipeline / elitpc-100hz-deprioritized / geteccserver-native-gded。
