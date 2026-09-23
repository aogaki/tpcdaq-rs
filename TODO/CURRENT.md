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

**2026-09-15 プログラム票 [080_mini_experiment_readiness.md](080_mini_experiment_readiness.md)(READY、同日裁定 5 点)**:
ユーザー方針「Mikolaj は mini にかかりっきり、次実験も mini → こちらでやれることをやりまくる」を受け、
TODO + archive を棚卸しして A 本番機 / B 運転面 / C スローコントロール / D 較正・解析プロダクト(Warsaw
待ち 3 点の自前化)/ E 073 残骸 の 5 群 + Wave 1〜4 の推奨順に整理。**裁定: 実験 = ¹⁰B ターゲット + γ → α(時期未定)、
遠隔 run は大抵可、新 DAQ 機は来週から、HV = HiVolta、優先順位は案どおり**。**別リポ ~/test/gas-pressure-test(Geant4 +
Garfield++)の `_raw.root` が 075 raw ツリーと同一契約 = シミュと DAQ が同じ解析チェーンに乗る(080 D-8)**。
**Wave 1 着手(2026-09-17、ユーザー「進められる話を進めたい」)**: 6 レーン並列(worktree 隔離) — 077(Sonnet)/ 079 Phase 1
(Sonnet、079 §2.5)/ [081_calib_tables.md](081_calib_tables.md)(Sonnet)/ [082_logbook_to_runs_yaml.md](082_logbook_to_runs_yaml.md)
(Sonnet)/ 076 Phase 0(Sonnet)+ Phase 1(Opus)。Fable = A-1 新機手順書 + RUNBOOK 骨子 + 完了時レビュー・マージ。
**進捗(同日)**: 077 マージ `3a973f7`(demo 目視待ち)/ 076 Phase 0 マージ `cad5a4c` / ~~082~~ 完了 `96e0997` → archive /
docs: [NEW_DAQ_SETUP_ja.md](../docs/NEW_DAQ_SETUP_ja.md) v0.1 + [RUNBOOK_ja.md](../docs/RUNBOOK_ja.md) v0.1(`b818730`)。
~~081~~ 完了 `87a65da` → archive(pedestal 1088 ch 全数一致、cargo 514)。076 Phase 1 マージ `9fe6c2f`(cargo **578**、ゴールデン 9 本全一致)。079 Phase 1 マージ `617ab71`(cargo **614**)。**SPEC v1.30**(`55a632a`)。~~077~~ **完了(2026-09-22 目視合格 → archive)**: event# 一致・`≠ UVW` 無し・FPN の 4095 飽和が見える。派生 = [086](086_2d_palette_top_color.md)(パレット最上位が白で飽和が見えにくい、Sonnet)/ [087](087_vcobo_loop_renumber.md)(vcobo loop が eventIdx を付け替えず 2 周目以降が late fragment、Sonnet)。
**Wave 1 完了後の次**: 076 Phase 2(Ecc プロキシ + core 移植 + bridge bin、Opus)/ 079 Phase 2(slowctl 骨格、Opus)/ 078(Opus レーン A)/ D-3 パルサースキャン(GDED)/ 新機セットアップ(来週)。
**Wave 2 着手(2026-09-22、ユーザー「実機に触れないが進められることを」)**: **SPEC v1.31**(`c797792`、slowctl 契約 — §2.3 kind=`slow` PUB / §3.1 `[slowctl]` / §3.2 47006・source_id 200 / §9.2 `psu`→`slow` + run_start/run_stop `slow` / §10.3 status `slow`)を先に確定してから 4 レーン並列(worktree 隔離、`git merge --ff-only main` 指示付き): ~~076 Phase 2~~ **完了(同日、`6b02fc0` → merge)**: `src/ecc/{core,proxy}.rs` + `src/bin/ecc_bridge.rs`、457 CHECK → 457 assert、fake ECC e2e **52 passed**、既存統合 14 本を Rust bin で green、main ゲート **cargo 784 passed / 0 failed / 1 ignored**(Wave 2 の 4 レーン全マージ後、clippy clean、SPEC v1.32)。実 ECC の ③④ と C++ 撤去(Phase 3)は新機で/ ~~079 Phase 2~~ **完了(同日、`f1bd804`+`0e55c41` → merge `c3f3a0c`)**: `src/slowctl/{bronkhorst,component,fake_device}.rs` + `src/bin/slowctl.rs`、logbook `psu`→`slow`、monitor status に `slow`、controller run 記録に `slow`、LogPost 再送キュー(IDENT/ALARM/RECOVER)、統合テスト 5 項目 green。Phase 3/4 と 085 コーデックの Driver 化は残/ ~~078 レーン A~~ **完了(同日、`b553830`+`f6ae18f` → merge)**: `src/params/{schema,render,paths}.rs` + `tools/ccfg_resolve/`(CompoundConfig オラクル、Mac ビルド可)、11 ノブ、**実テンプレート 4,423 インスタンス全数一致**、往復バイト一致、**発見: CompoundConfig は粗い例外を一様値より優先しない → 実在 id へ展開(SPEC v1.32 §3.3)**。残 = レーン B(REST)/ C(UI)/ ~~085~~ **完了(同日、`b0dff9d` → merge)**: [archive/085](archive/085_hv_readout_codecs.md) — HiVolta DT1415ET + HMP2020 の読み出しコーデック(純関数 + `Read+Write` リンク + fake)、**cargo 614 → 657**、ゴールデンはマニュアル逐語(NUMCH=8、エラー 5 種、STATUS 15 bit / BDALARM 4 bit 全行)、SET 系生成関数なし。Fable = 完了時レビュー・マージ。09-15/09-17 背景 run の解析は 083 追記 7–8(²²⁴Ra ポート汚染)。Nucleu 2027–2030 提案メモ = `reference/_nucleu/`。
**Wave 2b 進捗(2026-09-23)**: ~~086~~ **完了 → archive**(`2f6022e` kBird 事前反転 → 目視でゼロ付近が青一面 → 追補 2 `e70acfa` 自前ダークランプ(黒→…→黄→橙)、Fable の headless スクショ `cdp_shot.py` で合格、UI 272 passed)/ ~~087~~ **完了 → archive**(vcobo 204 passed、demo 受け入れ: 後接続クライアントに 0x02 307 / 0x03 102 per 15 s、events_built 3977 → 5134、late_fragments 0)/ ~~079 Phase 3~~ マージ `a7516a7`(Arduino スケッチ + `arduino_env` ドライバ、803 passed)/ ~~078 レーン B~~ マージ `a69ad50`(`/api/params` GET/PUT/render、run/start の config_id 結線、`run_start.params`、統合テスト 8 本、**SPEC v1.33**)— 残 = レーン C(UI)。~~088~~ マージ `4997e87`(slowctl_integration / monitor_ws の固定待ちを条件待ち + 上限に、各 20 回連続 green)。**注意(2026-09-23 08:45)**: Mac の負荷が VS Code の ripgrep(160 プロセス超、worktree の target/ と macros/data 49 GB を走査)で load avg 500 超になり、cargo clippy がロック待ちで止まる・demo の run/stop が EOS 5 s timeout を踏むなど計測が崩れた → 負荷低減後に再実行: **main `4997e87` 以降 cargo 812 passed / 0 failed / 1 ignored、fmt / clippy(fake feature 有無)clean**。原因は VS Code 拡張ホストの `rg --files --no-ignore -g **/*.{ts,tsx}` × 161(worktree 増減で再列挙)→ `.vscode/settings.json`(gitignore、ローカル)に除外を設定。**教訓: worktree レーンを多数出すときは VS Code の列挙負荷を見ておく**。
**2026-09-23 GDED 遠隔ベンチ 第 2 弾(ユーザーが VM の GET を全停止)**: 実 ECC をホストにネイティブ起動 + **Rust ecc_bridge を GDED 上で実 ECC・実 zCoBo に対して実走**(トンネル不要)→ 2 run(60 + 30 frames、malformed 0)、**walk-back が C++ 時と同一** = 076 Phase 3 の前提クリア(076 結果節)。続けて **D-3 パルサー振幅スキャン**(−100 / −200 / −300 / −500 / −700 / −1000 mV × 60 s、`scan_d3.sh`)= [091](091_pulser_amplitude_scan.md) **取得完了**(run 6–11、各 60 frames、欠損 0、Mac の `reference/exp_data/2026/d3_pulser_scan/` に回収)。GDED のスタック・ブリッジ・ECC は停止済み(VM の GET 再開可)。**第 3 弾後の main ゲート: cargo 848 passed / 0 failed / 1 ignored、fmt / clippy clean**(2026-09-23 13:1x)。~~091~~ **完了 → archive**(`calib scan` + `pulserScan.C`: −700 mV まで直線・−1000 で −5.3 % 圧縮、FPN は 1.5 倍高感度で −700 以上飽和、平均波形は立ち上がり 0.6 µs / FWHM 1.65 µs @100 MHz、844 passed on base)。**第 3 弾(ユーザー「ガンガン」)**: **078 §5-3 / §5-5 合格**(生成 `configure-pulser__75bb9a9e.xcfg` を実 ECC が読んで run、P1/純正 = 1.005、gain 240fC で 0.554 — 078 結果節)/ **069 #16 合格**(compact の MFM オラクル照合: run 6/11 × 60 frames、ヘッダ全項目 + 8,355,840 items の sha256 一致、`tests/decode_real_compact.rs`、846 passed)/ **069 #15' 余分接続・#12 stop 後 peer・ACM 300 s** 記録済み/ **#13 / #14 合格**(板の getHwServer stop → `forced_eos:false`、restart → breakup 失敗で ECC Off → reset では戻らず getEccServer 再起動で回復、**Rust bridge の再接続経路も実証**。復旧手順を RUNBOOK §5 に)/ 派生 [092](092_receiver_abandoned_after_eof.md)(EOS 後の receiver の abandoned 1 通)/ **069 #7 合格**(run 15–24、10 本とも walk-back 同一・20 frames・malformed 0、90 s アイドル後も再接続なしで成功 = ACM 切断は未発生)/ ~~076~~ **完了 → archive**(Phase 3: C++ ecc_bridge 撤去、統合テストは Rust bin 既定、SIGINT/SIGTERM ハンドラ、e2e 52 passed、**SPEC v1.35** §1.1/§8.2 + CLAUDE.md Tech Stack を Rust に改訂)/ **GDED は 12:5x に全停止・ポート解放済み(VM の GET 再開可)** / 083 は chunk 0004 完了版(9,559 ev、144.6 h)で再フィット: 3.66 d 成分 0.0033 ± 0.0013 Hz(2.5σ、変わらず)、飛跡側は再解析中。**HiVolta 実機読み出し合格**(ユーザーが GECO を閉じ dialout 追加後、MON のみ 70 s: IDENT DT1415ET / 8 ch / FW 1.14 / S/N 165、8 ch × 4 + board 2 = 34 サンプル/poll、30 s 定期 VALUE が ±0.02 s、ERROR 0 — archive/089 実機受け入れ節)。
~~090~~ **完了 → archive**(2026-09-23、マージ `3ac3599`): 波形ビューの系列上限に 68 / 136 / 272、overlay は lttb + large(凡例は 64 超で非表示)、grid は 8 / 12 列。ユーザー目視 OK + Fable の headless スクショ(`cdp_wave.py`)で 272 本・FPN 飽和を確認。**UI 283 passed**。
**Wave 2c 進捗(2026-09-23)**: ~~079 Phase 4~~ マージ `d0781c7`(Slow control UI)/ ~~078 レーン C~~ マージ(Parameters UI、衝突は両方残して解決、**UI 268 passed**、**SPEC v1.34 §11**)/ ~~089~~ **完了 → archive**(マージ `b6f27df`: HiVolta / HMP2020 の `Driver`、addr-or-port、SCPI `send` + `INST:NSEL?` 確認、838 passed on base)。**slowctl のドライバ 4 種(Bronkhorst / Arduino / HiVolta / HMP2020)が揃った** — 残りは現地 Phase 0 と実機一致。**Wave 2c 全マージ後の main ゲート: cargo 838 passed / 0 failed / 1 ignored、fmt / clippy clean、UI 268 passed**。docs/NEW_DAQ_SETUP v0.2(`9a0354d`)。
**Wave 2c 着手(2026-09-23、ユーザー「進めれるものは進めて」)**: Sonnet 3 レーン(worktree、`79df4b2` 以降) — **078 レーン C**(§11: Parameters ビュー + params-api + status バー)/ **079 Phase 4**(§2.8: Slow control ビュー + トレンド + status バッジ、`power/` を置換)/ **[089](089_hv_drivers.md)**(HiVolta / HMP2020 を Driver に配線)。UI 2 レーンは `app.routes.ts` / `status-bar` で小衝突の可能性 → Fable が 3-way。~~088~~ archive。
**Wave 2b 着手(2026-09-23、ユーザー「続けて」)**: Sonnet 4 レーン並列(worktree、`a11013a` 以降) — **086**(パレット 56 → 57)/ **087**(vcobo loop の eventIdx/eventTime 付け替え、demo 受け入れは Fable)/ **078 レーン B**(§10: `/api/params` GET/PUT/render、run/start の config_id 結線、logbook `params`、`[controller] ecc_config_dir` / `params_dir`)/ **079 Phase 3**(§2.7: MKR ENV スケッチ + `arduino_env` ドライバ + fake)。マージ後に SPEC v1.33(§8.1 / §9.2 / §3.1)を Fable が転記。
レーン運用の教訓: Agent の worktree は古いベースで切られることがある → 発注時に「`git merge --ff-only main` してから着手」を明記(メモリ agent-worktree-stale-base)。**次 = 076 Phase 2**(Ecc プロキシ + core 移植 + bridge bin、Opus)。073 は #5 済(ROOT 6.08.06 / TPCReco =
reference)、残りは 080 §1-E に吸収 — archive はユーザー指示待ち。

**2026-09-14(午後)~~075~~ 完了 — root-sink raw ツリー出力(SPEC v1.28 §6.7)**: `--raw-tree` /
TOML `raw_tree` で `run{N}_raw.root`(macros/graw2root.C と同一契約)を第 2 出力として書く。
2025 mini 実 run で **graw2root.C と 108 events / 0 differences**、§12-8 変形は N=4・224/672 Mbps で
raw on/off 差 +0.3% / +0.2%。ゲート: test-root 170/99/**88(新規 test_raw_tree)**、cargo 472。
mini の macros チェーンは DAQ 直結(graw2root.C は .graw 用に残る)。implementer 2 レーン並列
(Opus C++ / Sonnet Rust)+ Fable レビュー、レビューで soak_harness の 2 倍カウント偽 FAIL を修正。
[archive/075](archive/075_rawtree_output.md) 結果節が正。**Mac の ROOT は 6.40.04 に更新済み
(SDKROOT=MacOSX27.sdk 必須、メモリ mac-root-cling-sdkroot)**。
**同日、ユーザー質問「zCoBo を変えない限り Ice 版数問題から逃れられないか」への整理 → 縛りは 2 層
(ワイヤ 1.0 = 板、バイナリ 3.7 = ECC の C++98)で、逃げ道は「ECC はソースから 3.7 で再ビルド、
橋だけ Rust 化」。詳細調査 + 立案 = [076_rust_ice_client.md](076_rust_ice_client.md)(**READY** —
同日ユーザー裁定: 新 DAQ 機で最初に試す項目(ROOT インストールと並ぶ)/ 同期実装で可 /
C++ 撤去は実機動作確認後。**着手指示で Phase 0(Sonnet)+ Phase 1(Opus)を並列に出す**)**。Mac 側の今朝の ecc_bridge(47200)は
停止済み(計測とポート衝突したため)。**GDED の getEccServer / ベンチスタック / getEccClient 残骸は
同日夕方に PID 指定で停止済み**(VM の GET 再開に支障なし。127.0.0.1:47200 の sshd リモート転送
リスナだけが残るが無害)。
**UI 相談(同日)**: 「信号が時間窓に収まっているか」を全チャンネル一括で見たい → **UVW Event
Display(0x02)がそのまま答え**(同一タブに 3 面)。追加要望「272 ch × 512 の電子回路座標 2D を同じ
タブに」= [077_asad_map_panel.md](077_asad_map_panel.md)(READY、Sonnet 半日、着手指示待ち)。
**パラメータ設定 UI(同日)**: xcfg 直接編集ではなく **JSON を正として xcfg を生成**(ユーザー裁定:
JSON / 078 / ノブは「一様値 + 例外リスト」— 隠れているが黙らない)=
[078_param_ui_json_xcfg.md](078_param_ui_json_xcfg.md)(READY、≈ 2 週間、着手指示待ち)。
**2026-09-15 スローコントロール(reference/Arduino, reference/Bronkhorst)**: 写真 5 枚で判明 —
ガス系は **Bronkhorst FLOW-BUS**(EL-FLOW Select「MFC 1」+ EL-PRESS「BPC」「TPC」、RJ45 Y アダプタ
7.03.241 でカスケード)、PC は USB-RS232 → 先頭機器 D-sub(ProPar 38400)→ **local host が全ノードを中継**
= シリアル 1 本で 3 台読める。Arduino は自前スケッチ(裁定)。`slowctl`(P6 psu の一般化、**監視のみ**)=
[079_slow_control_gas_env.md](079_slow_control_gas_env.md)(**READY**、≈ 1.5 週 + 現地 Phase 0 半日、
着手指示待ち。Phase 1 codec は現地を待たず Sonnet に出せる)。

**2026-09-14 遠隔ベンチで実機初データ取得(Fable 5.1)**: DAQ ホスト GDED(daq@192.168.144.101、
Ubuntu 24.04)に tpcdaq-rs をネイティブビルドし、実 ECC(getEccServer 20190315、ホストで
ネイティブ起動)経由で **実 zCoBo + AsAd 内蔵パルサーから run 2 本(122 + 20 frames、
frameType 2 compact、欠損 0)**。069 Phase 1 #1–#5 / Phase 2 #6・#11 / Phase 3 #12・#15 を
クローズ(結果節に転記、SPEC v1.27 §13 反映)。**Q1 = TCP に topology frame は来ない、
Q3 = 261 s は再現せず 7 s** で確定。新 bin `null_sink`(ROOT 無しホスト用 root-sink 代役)。
残: #7 連続 ×10、異常系(#8–#10, #13, #14)、#16 オラクル照合、root-sink 経路(GDED に ROOT
無し)、Phase 4。**GDED 上の ECC/ベンチスタックは常駐中 — VM の GET 再開前に停止すること。**
専用 DAQ 機の新設は Ubuntu 24.04 LTS を推奨済み(理由 = 本日の実測で全縛りが解ける)。
再現手順 = メモリ geteccserver-native-gded。取得 .graw は compact なので macros/graw2root.C
(frameType 1 専用)は要拡張 → 075 の価値上昇。


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
[archive/066](archive/066_unfolding_spike.md) 結果節 + reference/_spike/unfold/FINDINGS.md が正。
~~070~~ **完了(同日)— docs/DRIFT_DIFFUSION_PRIMER_ja.md v1.0**(ドリフトケージ電圧と
ディフュージョンの教育ノート。069 Phase 2.5 / 066 の実測との接続込み。
[archive/070](archive/070_drift_primer_doc.md))。
~~071~~ **完了(同日)— ログブック 5 年分(OCR 407 ページ)全数調査**: Sonnet 4 レーンで
6 分類抽出 → 統合 `reference/logbook/_survey/SURVEY.md` v1.0。**HIGS 2026 運転条件が確定**
(CO₂ 130 mbar / working point #3 = 255/310/1372V / E_drift 70 V/cm / **v_drift 0.390 cm/µs**)、
これで **066 の厳密縮退が解消: σ_L = 0.94 [0.78,1.09] mm、拡散成長率は熱限界と 1% 一致**
(CO₂ 冷たいガスの定量確認)。運転点マスタ表(#1〜#5)・DAQ 設定系譜(delay 1848 の出自)・
069 向け運用知見(zCoBo 再起動手順 / DAQ 起動順 / **zCoBo v.4 の存在**)も回収。
**要検証 2 件**: 実験側 GET「20190315_patched-v2」(hack 入り)と reference の同一性 /
運転点表の原本目視(OCR 依存の数値を SPEC 昇格させる前提)。069 C-6 は軽量化済み。
[archive/071](archive/071_logbook_survey.md) + SURVEY.md が正。
**同日追記**: 気圧 130 mbar・working point #3 の HV は**ユーザー確認済み** → E_drift 70 V/cm
確定、v_drift 0.390 は (p,E) で一意 = 実質確定(最終クロスチェックは 069 C-4)。
~~072~~ **完了(同日)— docs/XCFG_TUNING_GUIDE_ja.md v1.0**(xcfg の 3 層構造・
ワイルドカード継承・実際に触るノブ頻度順・チューニング実務ループ・既知の罠。
065/071/066-068 の統合、[archive/072](archive/072_xcfg_tuning_guide.md))。
**[073_trip_takehome_checklist.md](073_trip_takehome_checklist.md)(進行中)**: HIGS 出張の
持ち帰りリスト。**#7/#8 済(2026-08-20 現地実測 → SPEC v1.24)**: データリンクは
**1 本のみ**(`4.2:46005 ← 4.84:44951`、CoBo が接続する側 = SPEC v1.12 の実機初観測)、
dataRouter = **FDT FrameStorage**(2026 運用も FDT 確定)。geometry .dat =
reference/TPCReco latest 無改変(ユーザー確認)。
**#1 も済(同日)— patched-v2 疑義決着**: reference/20190315_patched_v2 を全数 diff →
**ソース差分は FrameStorage.{cpp,h} のみ**(env `GET_SPLIT_OUTPUT_BY_ASAD` の AsAd 分割
hack)。**制御プレーンは同一 = 既存プロトコル検証は全て有効**。graw 命名 3 系統の出自も確定
(`CoBo{K}_AsAd{A}` = dataRouter+env / `CoBo_ALL_AsAd_ALL` = env 無し / 素の `CoBo_` =
GetController 内蔵 raw 保存)。残り = physics run 追加 / Magboltz / DCS 時系列 /
ROOT・TPCReco 版 / Mikolaj 3 点。
**原則 = 実験に触らない、コピーと観察のみ**。**[069_elinp_test_plan.md](069_elinp_test_plan.md)
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

- **[2026-09-02 起票] オンライントラック再構成のスコープ復活(SPEC §4.4 / §1012 の改訂可否)**

  **発端(ユーザー提案)**: 本番の実験は α のみの検出になる見込み。「1 イベント = 1 トラック」
  と決め打てば、トラック再構成込みで DAQ を作れるのではないか。

  **現行 SPEC との衝突**: §4.4「R5 でオンライン再構成は非スコープ」/ §1012「旧 0x01
  Event(3D 点群)は廃止(R5 非スコープ)」。復活させるならこの 2 箇所の改訂が要る。
  ただし 2026-08-18 裁定(ELITPC 100 Hz は後回し・次の主軸は解析プロダクト)とは整合方向。

  **新しい判断材料(~/Workspace/Higs/minitpc-analysis、2026-09-02 実測)**:
  - `minitpc/simplefit.py`(228 行、Python/uproot+numpy)が **269 ev/s** — 目標 100 Hz に
    対し 2.7 倍。TPCReco makeTrackTree は 1.2 ev/s。
  - 中身は **ビュー毎 PCA → ±3σ バンド再選択 → 再フィット → z 3 mm スライス三角測量 →
    3D PCA**。ミニマイザ・テンプレートフィット無し = per-event コストが決定的で上限が読める。
  - 品質は TPCReco 同等以上(ヒット垂直残差中央値 5.6 mm vs 6.2、長さピーク IQR 31 mm
    vs 41、成功率 100%)。検証記録 = `docs/simplefit-validation.md`。
  - **頭尻は原理的に決まらない**(どの手法でも ~50%、封じ込め条件付きで 45%)。
    100 mbar では α が止まらず Bragg ピークが存在しない(スタック dE/dx 平坦、
    ユーザー確認済み)ため。

  **Opus の推奨(裁定を仰ぐ対象)**:
  1. **reco はモニタ系に置く**。トラックパラメータは導出量で run.root(PEventTPC)から
     完全に再生成できる → 落としてよい = モニタ系の定義そのもの。保存系に入れると
     reco の遅延・バグが graw-writer/root-sink へ背圧をかける(絶対ルール違反)。
  2. **同じエンジンを 2 箇所に**: ①オンライン = 間引き + ドロップカウントで電場スキャン
     4 指標(v_drift / 拡散幅 vs z / 電荷 vs z / charge-per-length)をライブ表示
     ②run 終了後 = run.root を読む後段パスで正本 TrackTree(レート制約ゼロ・全イベント・
     オフライン再走と完全一致)。両立で絶対ルールにも KISS にも触れない。
  3. **「1 トラック」を焼き込まない** — ch 数と同じ扱いで設定側の仮説に(runs.yaml は既に
     `reco: hypothesis: alpha` を持つ)。ELITPC は多分岐を含む(zCoBo FW README 自身が
     MULTIPLICITY / 2-PROTON をトリガモードとして挙げる)。
  4. **仮説違反は必ずカウントして可視化**(多重ブロブ / z スライス < 3 / 残差 RMS 大)。
     silent failure 禁止則により、DAQ 化の瞬間に品質フラグが必須要件へ昇格する。
  5. **頭尻は DAQ に入れない**(コイントス。方向は線源位置の事前情報を持つオフライン側)。

  **着手前に必要な実測が 1 件**: 269 ev/s は hitDump 済みヒットに対する値。オンラインは
  生 ADC からペデスタル減算 + ヒット検出が要り、そのコストは未測定(068 の per-channel
  ベースライン表は利用可)。ここが支配的なら結論が変わりうる。

  **裁定済み・SPEC 反映済み(Fable 2026-09-02、正 = SPEC v1.25 §5.5)**:
  (a) **スコープ入り、ただし「QA 再構成」として** — R5 の非スコープは正本再構成について維持。
  (b) **配置 = monitor(Rust)内の純関数モジュール。root-sink には入れない**(§5.1 の根拠 =
  完全性は QA に不適用 → C3 分離が優先)。入力 = **既存 built_event PUB(§5.3、20 Hz 最新優先)**
  で新規配線ゼロ。**事前スパイクは不要に格下げ**(20 Hz サンプリングで律速不能)— 受け入れ
  基準「reco on/off で §12-8 と WS 配信が不変」に置換。mm は §4.4 将来条項(TPCReco 規約)を
  本モジュール内限定で発動。0x01 は復活させず QA ヒストは 0x10/0x11 の id 20 番台
  (「sampled」明示)、トラック重畳は必要時に 0x04 新設。
  (c) **run.root 不変(friend tree 不可)・run<N>_monitor.root にも入れない**。正本 TrackTree =
  オフライン minitpc-analysis(269 ev/s = run 1 本 1 分)。
  拘束: ①1 トラック仮説は設定値(焼き込み禁止)②仮説違反 reject + カウント + status 表示
  ③頭尻は実装しない ④Rust 移植は Python simplefit 正のフィクスチャ一致テスト(§4.5 型)。
  **反映完了(2026-09-02 同日)**: SPEC v1.25(§5.5 新設 + §4.4 mm 例外 + §10.2 0x01
  非復活 + 履歴)/ CLAUDE.md「やらないこと」R5 行 /
  [074_qa_reco_monitor.md](074_qa_reco_monitor.md)(**DRAFT** — 着手はユーザー指示待ち)。

- ~~**[2026-09-03 裁定済み] macros/ 解析チェーンの DAQ 直結(SPEC v1.26 §6.7)**~~ **完了
  (2026-09-14、075 → archive、SPEC v1.28)**。監査結果(記録として保持): graw 側は無変更で互換 /
  ROOT 側は PEventTPC から生 ADC を復元できないため root-sink 追加出力が唯一の道。
- **[2026-09-14 起票・同日裁定済み] ecc_bridge の Rust 化(Ice クライアント部分集合)** =
  [076_rust_ice_client.md](076_rust_ice_client.md)(**READY**、3 点の裁定は同 md 冒頭。着手指示待ち)。

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
- **macros/ は将来配布予定(2026-09-03 ユーザー表明)**: スライド
  (日本語版 Artifact 済み — 英語化して配布)+ マクロ一式。配布前チェック:
  ①LICENSE 追加(graw2root.C は GET 非依存の自前デコードなので CeCILL 非汚染 = クリーン)
  ②channel_map.csv の扱い(実 mini ジオメトリの派生物 — 同梱可否はコラボ判断。
  代替 = mapDump で各自生成方式)③ビルド産物(*_C.so 等)を .gitignore へ。
- 小粒フォローアップ: **2026-08-15 の 044 窓 + P4 でほぼ完済** — geometry 参照アクセサ(046)/
  quiesce=0 拒否(046)/ vcobo SIGINT(047)/ UI lazy チャンク(048)/ SPA deep link 404(052)。
  残: poisoned 時 metrics の `PoisonError::into_inner`(023 申し送り — 032 実施済み分以外の
  箇所が残っていれば。次に該当ファイルを触るユニットで確認)。
- **delila-rs への申し送り**: pop_for 競合 → issue 化済み
  https://github.com/ELI-NP/delila-rs/issues/26 / ZMQ fair-queue 飢餓(013)も要点検。

## 運用メモ(常時適用はメモリ・CLAUDE.md 側が正)

- C++ の make は必ず `-j`。
- **IDE**: `.vscode/c_cpp_properties.json`(gitignore 内・ローカル)に ROOT include
  (`/opt/ROOT/include`)+ C++17/libc++ を設定済み(2026-09-03 — macros/ の IntelliSense 用)。
- **実 .graw + 実 mini ジオメトリの正しいペア**はメモリ参照(合成 fixture と混同しない)。
- リプレイ経路のライブ起動レシピ = [archive/028_web_ui_monitor.md](archive/028_web_ui_monitor.md)
  の付録 A(実走で実証済み。落とし穴一覧つき)。
- どんな小修正でも連番チケット + 結果節 + archive(ユーザー方針)。

## 完了ユニット台帳

000〜065 すべて [archive/](archive/) に結果節つきで格納(単位の詳細・テスト実測値・逸脱の裁定は
すべて各 md の「結果」節が正)。
直近(2026-09-14): **075** root-sink raw ツリー出力(macros チェーン直結、実データオラクル
0 differences、§12-8 変形 ±0.3%)。
直近(2026-08-18): **064** 並列 recorder = mini 100 events/s / **065** reference/config
全数調査(現行実験 ECC 設定の生コピー — 現用 = trigDelay1748@25MHz・フル readout 確認、
大半は ZC706 とバイト同一、physics xcfg は 2022 年以来 delay 以外不変)。
前々日(2026-08-16): **031** 一晩 soak 合格 / **054** GDataFrame 全撤去 + IMT(+29%)。
前日(2026-08-15): **038〜042** 仮想 zCoBo トラック / **043** ecc_error 可視化 /
**033** 異常系セマンティクス(quiesce 停止 1.3 s 化)/ **044〜049 リファクタ窓** /
**050〜052 P4**(Run 制御 UI 実配線 + 表示強化 + SPA fallback)/ **053** RSS リーク根治。
前日(2026-08-14): 027〜037。
