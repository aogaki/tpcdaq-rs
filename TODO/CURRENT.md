# CURRENT — tpcdaq-rs 現在地

**最終更新: 2026-08-18(064 並列 recorder = mini 100 events/s + 065 reference/config 全数調査。
前波は
[archive/CURRENT_2026-08-14_p2_p3wave.md](archive/CURRENT_2026-08-14_p2_p3wave.md))**

## いま(1 分で読める要約)

- **P0/P1/P2 完了**(出口はすべて実データオラクルで実測クローズ)。**P4 の核も前倒し完了**
  (015 logbook / 016 controller / 017 ecc-bridge)。
- **P3 完了(UI 含む)**: 018 → 020 → 022 → 026 → **027/028/029 Web UI** →
  **030 P3 E2E で §12-7〜12-11 を全クローズ**。
  **ブラウザまで一気通貫で開通**(実 .graw リプレイ → root-sink 集計 → PUB → monitor → WS →
  Angular で 9 ヒスト + イベント表示 + 波形 + ログブック。Run 制御は完成形レイアウト + 全 disabled)。
- **run 制御の実機ハードニング完了**: **032**(receiver 単一リンク + silent stall の可視化)/
  **034**(連続 run)/ **036**(実 ECC の `reset` = `EV_UNDO` への対応 + テストダブルの実機準拠化)。
- **リポ全体ゲート: cargo 454 passed / 0 failed / 1 ignored**。C++ 側 `test_ecc_bridge 457`
  / `ecc_e2e 52` / root_sink 8 スイート green(**recorder 170** / pevent 99)+
  **vcobo 148+**(マージ対応後)。clippy -D warnings クリーン。
  UI 適合 **192** tests green(**全面英語化済み**)、dist 4.9 MB。
- **root-sink は PEventTPC 1 形式 + P1 並列書き出し(v1.21)**: `--recorder-workers 4` で
  **mini 100 events/s 達成**(30 分 @224 Mbps 持続 100.4 /s)。burst 672 Mbps drop 0。
  §12 の負荷受け入れは全て宅内でクローズ。
- **run/stop 所要 1.3 s**(033-E 静止検出。旧 5.6 s)。run/start ≈ 7 s(実 ECC 支配)。
  **decode 26% 短縮**(045)。production の panic 起点ゼロ(046)。
- **仮想 zCoBo スタック稼働**(2026-08-15): 実 ECC(実験と同一版)+ `tools/vcobo/` で
  検出器なしに run 一周が本番経路で回る。正本 = docs/VIRTUAL_ZCOBO_ja.md v1.2、
  レシピ = reference/_spike/demo/。
- **一晩 soak 合格(2026-08-16、031)**: 45 Mbps × 8.4 h / 50 run / ロス 0 / RSS 平坦。
  ソフト側の耐久はクリア — 残る性能課題は root-sink 天井(054)のみ。
- 実装の正本 = **docs/SPEC_ja.md v1.21**。モデル使い分け・完了時ルール = CLAUDE.md。
- 公開リポ: https://github.com/aogaki/tpcdaq-rs(実データ・FW・実 .dat は reference/ = .gitignore)。

## 次にやること(次セッションの入口)

**2026-08-18 ユーザー裁定: ELITPC 100 Hz の追求は本気で後回し**(コラボの誰も可能と
思っておらず「やりすぎ」。mini 100 events/s = 064 で最低目標達成済み)。
→ **次の入口 = [066_unfolding_spike.md](066_unfolding_spike.md)(READY)**: Unfolding の
検証ハーネス先行構築 + 実データ(reference/exp_data/2026)からのパラメータ拘束見積もり。
ベイズは判断材料が出てから。~~ユーザー依頼中: pulser .graw~~ **到着済み(2026-08-18)**:
`reference/exp_data/2026/` に **pedestal 32 run / 35 GB**(内部 periodically = ランダム
トリガ、physics と同一 compact)と **pulser 26 run / 4.3 GB(+中断 0 バイト 3 本)** を確認。
全数ヘッダ走査で新事実 3 つ: ①**FDT 接続開設時に CoBo が topology frame(frameType 7、
12 B、asadMask 付き)を必ず送る**(一次資料 MemRead.cpp:362 — 我々の receiver は未テスト =
**ELI-NP 地雷**)②**pulser は frameType 1(rev 5、itemSize 4、557,312 B 固定)= frameType 1
の実データ照合が初めて可能に** ③GetController 経由ランは `CoBo_{TS}_{idx}.graw` 単一
ファイル AsAd インターリーブ + 1 GiB 分割。→ ~~067~~ **完了(2026-08-18 同日、SPEC v1.23)**: topology frame 防御(decoder カウンタ +
INFO、欠落 0 テスト固定)/ **frameType 1 実データ照合を GET 純正 MFM ライブラリと全一致で
クローズ**(304 frames / 42,336,256 items、hit pattern = FPN 除外・データ = FPN 込みと意味確定)/
0 バイト .graw 耐性(--loop 無限ループの副産物修正)。ゲート **cargo 454 → 468 passed**、
clippy クリーン、vcobo 148 無変更。**実データで新事実**: pedestal の AsAd 到着順回転 +
eventIdx 後退(幅 1)を実測 → §6.3 に「ビルダは順序に依存しない」を実データ根拠付きで明文化。
**Fable 裁定: flowType は TCP 維持・FDT 非対応を §8.2 に明文化**(FDT は IMALIVE/GOODBYE/
run 毎再接続のワイヤ差分あり — 将来必要時は 3 点セット新ユニット)。vcobo への topology 送出は
撤回(誤前提)。[archive/067](archive/067_real_stream_compat.md) 結果節が正。
**2026-08-18 追記(並列消化中)**: 067 / 066 / 068 を implementer(Opus)3 レーン並列で実行。
~~068~~ **完了(同日)— pedestal 36 run / pulser 25 run を全数走査(40 GB / 84 s、サンプリング
なし、デコードは本体 Decoder と 1 item 照合してから使用)**。066/ゲイン正規化が使う数値:
**per-channel ベースライン必須(ch 毎 mean 224–512 ADC)/ ノイズ RMS 中央値 6.64(FPN 2.05)/
相対ゲイン ±4.4%(最悪 ±8.4%、AGET 単位系統差 5.3%、AsAd3 低め)/ 18 日間安定(pedestal 表・
ゲイン表とも 1 セットで足りる)/ 恒常異常 7 ch(全て生きている — pulser ゲインは正常。マスク
ではなく閾値個別化の対象)/ pulser の FPN は 4095 飽和 = FPN からゲイン不可**。
イベントビルダ裁定 2 件(固定窓なし / run 末尾 incomplete は既定 emit)は SPEC v1.23 同日追補。
成果物 = reference/_spike/asset_survey/out/(SUMMARY.md + CSV 17 種)。
[archive/068](archive/068_asset_survey.md) 結果節が正。
~~066~~ **完了(同日)— Unfolding スパイク A/B/C 完走、Fable 裁定確定**:
実 physics 334 events 選別 → TPCReco StripResponseCalculator **無改変リンク成功** +
1D 解析再実装(本家と数値照合済み: 時間 1e-4、横断 0.041)→ χ² 拘束マップ。
**拘束可 = σ_T 0.90 mm [0.60,1.20] と σ_L 6.0 cells [5.0,7.0] のみ。v_drift×σ_L[mm] は
厳密縮退・ゲインは恒久 nuisance。実データ χ²/ndf 8.7〜11 = モデル不足支配(イオンテール
未モデル化)**。閉包テストでバイアスなし。**裁定: データ駆動本実装せず・ベイズ層積まず・
Mikolaj 待ち(受け入れ試験 = `SIGMA_T=... SIGMA_L=... ./run_all.sh` で完成済み)**。
縮退を破る道 = (a) D_L をガス物性から与え v_drift を決める(物理側相談)
(b) パルサーで電子回路応答実測 + イオンテール追加(068 ゲイン表と同根 — 起票は保留)。
CKW は 25 MHz 採用(一次資料 + χ² も同向)。ROOT 6.08⇄6.36 互換問題は Warsaw 残確認へ追記。
[archive/066](archive/066_unfolding_spike.md) 結果節 + reference/_spike/unfold/FINDINGS.md が正。**[069_elinp_test_plan.md](069_elinp_test_plan.md)
(DRAFT)を Fable が起草** — ELI-NP 実機テスト計画(Phase 0〜4 + 現地で確定させる問い Q1〜Q4。
032/036/041/043/057/034/§13 の実機項目を全数集約。ユーザーレビュー待ち)。
SPEC は **v1.22**(§13-7 制御プレーン注記)。なお topology frame はソース上 FDT 限定送出
(`MemRead.cpp` の senderType ガード)で、**我々の TCP 経路では来ないはず** — 「必ず受ける」
は過大だった(069 Q1 で現地確定。067 の防御実装は GetController 形式リプレイに必要なので継続)。

## 旧・次にやること(順序は 044 で裁定済み・ユーザー合意 2026-08-15 — 履歴として保持)

**043 → 033 → 044(リファクタ窓)→ P4 UI 実配線 → 031 soak → ELI-NP** の順。

1. ~~043~~ **完了(2026-08-15)** — `ecc_error` 全経路開通、set/clear 規則を一次資料で固定、
   実 ECC で 041 D-1 再現。archive 済み。
2. ~~033~~ **完了(2026-08-15)** — run_stop 2 フィールド / eos_out 3 点判定 / 異常系 E2E
   F0-F2 / **quiesce 検出で run/stop 5.6 s → 1.3 s**。不達 receiver の停止分単位化の穴も
   発見・修正。ゲート **430 passed**。archive 済み。
3. ~~044 リファクタ窓~~ **完了(2026-08-15、045〜049 全 5 テーマ)**: decode 26% 短縮 /
   production panic 起点ゼロ / dist −2.9 MB / 5 bin 共通化 + vcobo SIGINT /
   **ECC 遷移表パリティテスト(64 組全一致、034 事故ケースの red 確認済み)**。
   見送り裁定(controller 分割 = P4 見積もり後 / E2E ハーネス = 031 移管)は
   [archive/044](archive/044_refactor_window.md) が正。**次 = P4 UI 実配線の起票(Fable)**。
4. ~~P4~~ **完了(2026-08-15、050/051/052)**: **Run 制御 UI 実配線**(disabled 解除・
   token/横取り・確認ダイアログ・実スタック smoke 合格)+ **表示強化**(ecc_error /
   forced_eos·eos_closed の三値表示 / config_ids)+ **SPA fallback**(直リンク 404 解消)。
   C3 = controller 分割は見送り確定(P4 の controller 追加は 052 の fallback 125 行のみ)。
   **ブラウザ受け入れデモの手順 = [archive/050](archive/050_run_control_wiring.md) 結果節**
   (UI ビルド → start_demo.sh(ui_dir 自動)→ http://127.0.0.1:8080/ → 操作列 ①〜⑧)。
5. ~~031~~ **完了(2026-08-16)— 一晩 soak 合格(§12-5(a) v1.15 ✔)**: 45 Mbps × 8.40 h、
   **50 run 全合格・全 run バイト/エントリ完全一致・ロスレスカウンタ全 0・RSS 単調性
   全 8 プロセス OK**(053 リーク修正の長時間実証: 後半 4.2 h で +56 KiB)。610,200 events /
   158 GiB。§12-5 フルレート持続 + **§12-6(672 Mbps burst)は未達 → 054 に移管**
   (原因 = root-sink 天井そのもの)。[archive/031](archive/031_load_harness.md) 結果節が正。
6. ~~054~~ **完了(2026-08-16)— GDataFrame 全撤去 + IMT 採用、100 Hz は単スレッドでは
   届かないことを実測確定**: 隔離 20.3 → 16.1 ms/event(−21%)、実稼働 32 → **41.5 events/s**
   (+29%)。third_party/get/ ごと削除、テスト −1,276 行、021 オラクル
   `3852 events, 0 differences` 無変更 green。A(map hint)は実測して棄却(libc++ で悪化 +
   TPCReco 無改変では口が無い)。80 Mbps × 30 分 soak 合格。**ELITPC 実測 ≈10 /s = 10× 不足**。
   [archive/054](archive/054_root_sink_throughput.md) 結果節が正。
7. ~~055/064~~ **完了(2026-08-18)— 並列化 P1 で mini 100 events/s 達成(SPEC v1.21)**:
   N=4 で隔離 152 /s・**30 分 @224 Mbps 持続 100.4 /s(10/10 run・全カウンタ 0)**・
   **burst 672 Mbps drop 0** → §12 受け入れ表 5/6 クローズ。旧保留②(過負荷 stop の
   eos-timeout)は並列化で解消。021 オラクル N=1 無改変 + N=2 ユニオンとも 0 differences。
   test_recorder 170 / cargo 454。compare_pevent の複数ファイル化で混入した silent
   failure を自己摘発(ネガティブコントロール常設 — 教訓は結果節)。
   [archive/064](archive/064_recorder_parallel_p1.md) 結果節が正。
   **凍結前の推奨(任意)**: 一晩 @224 Mbps / N=4 soak を 1 本(§12-5(a) の完全形)。
8. ~~056~~ **完了(2026-08-16)— ELITPC 構成デモ開通**: vcobo に eventIdx マージ
   (テスト 92→148、実 4 本組 15,408 frames / eventIdx 0..3851)+ elitpc xcfg 3 点 +
   `TPCDAQ_DEMO_PROFILE=elitpc` 切替。実測 304 events complete / late 0 / graw 4 本
   sha256 一致 / mini 回帰無変更。[archive/056](archive/056_elitpc_demo_profile.md)。
9. ~~057~~ **完了(2026-08-17)— タイムアウトは健全だった(SPEC v1.20)**: per-REQ
   60 s は正しく発火(遅延 fake で実測固定、新規テスト 3 本 → cargo 453)。261 s の正体は
   「シーケンス全体は意図的に無期限(最大 9 REQ)」— §8.2 に明文化し、`elapsed_ms`
   ログを常設。[archive/057](archive/057_ecc_timeout_not_firing.md)。
10′. ~~059~~ **完了(2026-08-16)— Wojciech 要望 3 点の成立性調査**: ZS(FW 対応済み・
   frameType 1 化に注意)/ Unfolding(TPCReco に無いが順方向応答 StripResponseCalculator
   あり = 逆問題として定式化)/ ゲイン正規化(greenfield、FW パルサー較正あり)。
   **裁定: 生データは graw のみ・ROOT は処理込み可 / 優先 Unfolding > ゲイン > ZS**。
   WARSAW_PLAN §7 新設。[archive/059](archive/059_wojciech_wishlist.md)。
10″. ~~060~~ **完了(2026-08-16)— SPEC v1.19**: 生データは graw のみ、ROOT は
   解析レディープロダクト(処理込み可・基底互換不変)を §6.4/§14-8 に明文化。
   [archive/060](archive/060_root_analysis_ready.md)。
10‴. ~~061~~ **完了(2026-08-16)— Warsaw 疑義 3 点の回答記録**: **ZS 事実上不採用**
   (Mikolaj は 1 ビットも変えない主義 → 055 はソフト並列化で閉じる前提が確定)/
   ゲイン較正は外部パルサー注入(ツール所在を Mikolaj に確認)/ **データリンクは
   コンピュータへ 1 本**(032 設計が適合、残確認は現地裏取りのみ)。
   [archive/061](archive/061_warsaw_answers.md)。
10⁗′. **英語ミニプロポーザル完成・配布(2026-08-18)**: 本文(6 節)+ データフロー図
   (docs/figures/dataflow.svg / .drawio)+ UI スクリーンショット 7 図の PDF。
   **Warsaw 大(Wojciech/Mikolaj)と ELI-NP mini TPC チームへ配布**。
   マスター = reference/proposal/(proposal.html → Chrome headless で PDF 再生成可。
   たたき台・英語 md も同所。リポには figures のみ)。
10⁗. ~~062/063~~ **完了(2026-08-16)— Warsaw プロポーザル準備**: 応答パラメータは
   Mikolaj モデル由来(受領待ちのみ)/ ゲイン較正は内製(FW 内蔵パルサー)で確定(062)。
   **UI 表示文字列を全面英語化**(22 ファイル、192 テスト件数不変、dist 再ビルド済み —
   英語ミニプロポーザルのスクリーンショット前提。063)。
10. ~~058~~ **完了(2026-08-16)— モニタ 2D 改善(SPEC v1.18)**: 2D 転置(縦 strip /
   横 time)+ Event Display に表示専用ベースライン減算(先頭 25 cell 平均・負値保持)。
   UI のみ +6 テスト(192)。Waveform 生 ADC 確定 / StripTime 積算のまま /
   elitpc デモ 1 event/s。[archive/058](archive/058_monitor_striptime_v2.md)。
6. **デモ改良トラック(継続、テスト計画より先 — 2026-08-14 ユーザー)**: 当面はデモの
   完成度を延々と上げる。目玉候補 = **graw ファイルをデータソースとする仮想 zCoBo**
   (getHwServer ではなく**板ごと偽る** — 2026-08-14 ユーザー用語確定)。制御面
   (getHwServer Ice 面)+ データ面(configure でリンク確立 / start でフレーム送出 /
   stop で close しない)を実機挙動どおり再現し、Run 制御 UI を「全 disabled」から実配線へ
   進める鍵にする。挙動の正 = reference/ の実 GET ソース + ZC706 FW 資料(テストダブルの
   コメントを根拠にしない)。AsAd 数・CoBo 数は設定駆動(mini 1 AsAd / ELITPC 4 AsAd / 多 CoBo)。
   **フォーク B(実 ECC + 仮想 zCoBo)採用 — 2026-08-14 ユーザー裁定**。
   **038 スパイク完了(同日実走)— 方針の正本 = [docs/VIRTUAL_ZCOBO_ja.md](../docs/VIRTUAL_ZCOBO_ja.md) v1.0**:
   実 ECC は macOS でビルド・起動・実通信まで成立(パッチ 2 行、Ice 3.7 keg、
   `reference/_spike/build_all.sh` で無人再現)。実 ECC ↔ getHwServer(Sim デバイス)接続も
   実証済み。**自作は vcobo-daq 1 プロセスのみ**(46004 の DaqCtrlNode 5 op + graw 送出 +
   シーダ。ECC→ハードは encoding 1.0 注意)。
   **トラック完了(2026-08-15、ユーザー指示「最後まで実装」— 038〜042 全ユニット green)**:
   039(実 ECC フルウォーク)/ 042(ConfigId 3 相化 = SPEC v1.13、ゲート 402 → 415)/
   040(vcobo-daq 本体 = `tools/vcobo/` 2,764 行、オラクル照合 8/8・xcfg 無改変)/
   041(統合デモ = 実 .graw で run 3 本、**実 ECC 歩き戻し初実証**、graw sha256 一致、
   異常系観測)。**正本 = docs/VIRTUAL_ZCOBO_ja.md v1.2、起動レシピ =
   `reference/_spike/demo/`(`start_demo.sh` → `run_once.sh` → `stop_demo.sh`)**。
   041 の発見 → SPEC **v1.14**(§8.2 `ecc_error` / §9.2 `forced_eos:false` 注記)+
   **[043_ecc_error_surfacing.md](043_ecc_error_surfacing.md) 起票(READY、P4 の前提)**+
   033 に追記(A 未実装の実測確認 + v1.14 織り込み指示)。
   **次の起票対象 = P4 Run 制御 UI 実配線チケット群**(disabled 解除 — 041 完了で前提成立)。
7. **ELI-NP 実機テスト(1 CoBo mini TPC、徹底的に)** — **Warsaw より先(2026-08-14 裁定)。
   直前はコード凍結(044 の裁定)**。
   実 CoBo・実 ECC 相手の確認項目は **032/036 の結果節に機械確認手段つきで書いてある**
   (`extra_connections` / `peer` / audit の `ecc_walk_back` / run/start 所要と
   `ecc_timeout` 60 s の余裕)— これらは ELI-NP で先行クローズする。
   **テスト計画の起票はデモ改良が一段落してから**(2026-08-14 ユーザー)。
8. **P5 Warsaw 展開** = docs/WARSAW_PLAN_ja.md — **完全後回し**。実機受け入れ試験・残確認
   (WARSAW_PLAN の工学項目)は ELI-NP テスト完了後。Warsaw 固有で残るのは
   旧 ROOT 互換 / grawToEventTPC 実機互換 / zCoBo リンク本数 / 先方 LAN 条件 /
   **zCoBo 台数構成**(2018 describe-elitpc は 2 台 2 リンクだが現行 HIGS は zCobo1k 1 台
   4 AsAd — 065 の観察。受信系は複数 CoBo 前提を維持しつつ現地裏取り)。
   **旧 ROOT 互換の具体機序が判明(066)**: PEventTPC の chargeMap キー
   `std::tuple<int,int,int,int>` は **libstdc++ と libc++ でメンバのメモリ順が逆**で
   StreamerInfo checksum が食い違い、ROOT 6.08 製ファイルを 6.36 で読むと**エラーではなく
   壊れた値**が返る(読みは StreamerInfo 貼り替えで回避可 — reference/_spike/unfold/
   pevent_read.hpp)。**我々の出力を先方の旧 ROOT で読む向きの実地確認が必須**
   (本番が Linux/libstdc++ 同士なら顕在化しない可能性が高いが、実測で潰す)。

## この波(2026-08-14 後半)で決まったこと・分かったこと

### ユーザー裁定

- **連続 run は「毎 run 完全リセットして一からやり直す」**(ワルシャワ大学の作法に合わせる)。
  オペレータに手で `ecc/reset` を挟ませない。
- **2D ヒストの stats box は出さない**(目的は各ストリップの時間変化を一枚絵にすること。
  統計量としては意味を持たない)。1D は残す。
- **Run 制御は完成形レイアウト + 全 disabled、モック禁止**(2026-08-13 決定を 3 ユニットとも継承)。
- **Warsaw 展開の工学項目(実機受け入れ試験・残確認)は完全後回し**(2026-08-14)。
  まず **ELI-NP で 1 CoBo の mini TPC による徹底的な実機テスト**を行う。実 ECC 相手の
  現地確認項目(032/036 の結果節)は ELI-NP で先行クローズし、Warsaw 固有の互換確認だけを
  現地送りにする。
- **テスト計画の前にデモ改良を継続する**(2026-08-14)。**graw ファイルをデータソースと
  する仮想 zCoBo**(getHwServer でなく板ごと偽る)の構築を視野に入れる。
  (整理: Fable 2026-08-14 — 「モック禁止」裁定は **UI の偽装の禁止**。仮想 zCoBo は境界の
  反対端(ハード)の置き換えで、UI から見える経路は全て本物になるため両立する。)
  → 同日、**フォーク B(実 ECC + 仮想 zCoBo)を採用**。実装は行わず**起票のみ**(= 038。
  仮想 zCoBo 本体 039 は 038 の結果待ち)。

### 実 GET ソース調査で確定した事実(→ SPEC v1.12)

- **データリンクを張るのは CoBo 自身**。ECC は Ice で指示するだけで **probe 接続を張らない**。
  接続確立は **`configure` の時点**(決定打: GET 純正 DataRouter は接続確立後に **listen 自体を
  閉じる** ので、probe があれば純正が動かない)。
- **`ecc stop` はデータリンクを close しない**(close は breakup か次の configure)。
  → 実機では stop 時に EOF が来ないので **強制 EOS が正規経路**。
- **実 ECC の `reset` は `EV_UNDO` = 1 段戻す**。`Active`(Ready/Running)からは**無音で無視**され、
  `configure` も `ST_PREPARED` ガードで**黙ってスキップ**される。
  → `Ready → Idle` は **`breakup → reset → reset`** の歩き戻しが必要。
- **SPEC §1.3 v1.6 の fatal 機序は誤りだった**(decoder Reset は同一 run 内 seq ギャップを作らない。
  実害は EOS バリア喪失 → 次 run 冒頭の exit 6 遅発)。誤りの起源は P2 レビュー R3 の仕様合成。

### 運用上の教訓(メモリにも記録済み)

- **制御プレーンの一次資料は `reference/20190315_patched` の実 GET ソース**。
  テストダブル(`fake_ecc` / `ecc_core.hpp`)のコメントを根拠にしない —— これを根拠に SPEC を
  誤り、034 が実機で 2 本目の run を壊すところだった(036 で修正)。
- **テストダブルが実機より甘いと誤実装が green で通る**。036 は「実機準拠に厳格化 → 034 実装が
  red になることを実測 → 直す」の順で進めた。

## Fable 待ち(キュー)

Opus 主対話中に出た設計判断・SPEC 疑義・レビュー依頼をここに積み、Fable セッション 1 回で
まとめて消化する(運用ルールの正は CLAUDE.md「モデル使い分け」+ 037)。

- (現在なし)

## 保留・確認事項

- **SPEC 検討(残 1 件)**: ①有界キューの単位が「メッセージ個数」— **064 の burst で
  実測が付いた**(672 Mbps 時に root_sink RSS ピーク 6.88 GiB、主因 = `--queue` 既定
  1000 バッチ)。バイト建て上限 or 既定値変更の裁定は ELI-NP の実機メモリ事情を見てから。
  ~~②過負荷 stop の eos-timeout~~ **解消(2026-08-18、064)**: N=4 で 224 Mbps
  10/10 run 正常停止 — EOS 予算 5 s は据え置きで成立(SPEC v1.21 履歴に記録)。
- **実 ECC の例外取りこぼし 2 箇所(043 発見 — 上流仕様なので改変しない。運用留意)**:
  `GetEccImpl::breakup` は失敗時 Ice **UnknownException**(SM::Exception を catch しない)/
  `onUnPrepare`(reset の Prepared→Described)は失敗時 dhsm が **halt** = 我々の map では
  `Off` に見える。P4 の UI 表示と P5 の運用手順(歩き戻し失敗時のリカバリ)で考慮すること。
- **Warsaw 確認**: TPCReco 再配布許諾(020 — third_party/tpcreco 昇格の条件)/
  PROPOSAL v0.5 反映判断。
- **物理屋向け資料・デモは UI + ファイルデータソース完成まで待つ**(ユーザー決定)。
- 小粒フォローアップ: **2026-08-15 の 044 窓 + P4 でほぼ完済** — geometry 参照アクセサ(046)/
  quiesce=0 拒否(046)/ vcobo SIGINT(047)/ UI lazy チャンク(048)/ SPA deep link 404(052)。
  残: poisoned 時 metrics の `PoisonError::into_inner`(023 申し送り — 032 実施済み分以外の
  箇所が残っていれば。次に該当ファイルを触るユニットで確認)。
- **delila-rs への申し送り**: pop_for 競合 → issue 化済み
  https://github.com/ELI-NP/delila-rs/issues/26 / ZMQ fair-queue 飢餓(013)も要点検。

## 運用メモ(常時適用はメモリ・CLAUDE.md 側が正)

- C++ の make は必ず `-j`。
- **実 .graw + 実 mini ジオメトリの正しいペア**はメモリ参照(合成 fixture と混同しない)。
- リプレイ経路のライブ起動レシピ = [archive/028_web_ui_monitor.md](archive/028_web_ui_monitor.md)
  の付録 A(実走で実証済み。落とし穴一覧つき)。
- どんな小修正でも連番チケット + 結果節 + archive(ユーザー方針)。

## 完了ユニット台帳

000〜065 すべて [archive/](archive/) に結果節つきで格納(単位の詳細・テスト実測値・逸脱の裁定は
すべて各 md の「結果」節が正)。
直近(2026-08-18): **064** 並列 recorder = mini 100 events/s / **065** reference/config
全数調査(現行実験 ECC 設定の生コピー — 現用 = trigDelay1748@25MHz・フル readout 確認、
大半は ZC706 とバイト同一、physics xcfg は 2022 年以来 delay 以外不変)。
前々日(2026-08-16): **031** 一晩 soak 合格 / **054** GDataFrame 全撤去 + IMT(+29%)。
前日(2026-08-15): **038〜042** 仮想 zCoBo トラック / **043** ecc_error 可視化 /
**033** 異常系セマンティクス(quiesce 停止 1.3 s 化)/ **044〜049 リファクタ窓** /
**050〜052 P4**(Run 制御 UI 実配線 + 表示強化 + SPA fallback)/ **053** RSS リーク根治。
前日(2026-08-14): 027〜037。
