# CURRENT — tpcdaq-rs 現在地

**最終更新: 2026-08-15(仮想 zCoBo トラック完走 = 038〜042。実 ECC + vcobo-daq で
run 一周がフルスタックで回る。前波は
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
- **リポ全体ゲート: 415 passed / 0 failed**(042 で +13)。C++ 側 `test_ecc_bridge 187` /
  `ecc_e2e 29` / root_sink 各テスト green + **vcobo 161**(92+57+6+6)。
  clippy -D warnings クリーン。UI 適合 126 tests green。
- **仮想 zCoBo スタック稼働**(2026-08-15): 実 ECC(実験と同一版)+ `tools/vcobo/` で
  検出器なしに run 一周が本番経路で回る。正本 = docs/VIRTUAL_ZCOBO_ja.md v1.2、
  レシピ = reference/_spike/demo/。
- 実装の正本 = **docs/SPEC_ja.md v1.14**。モデル使い分け・完了時ルール = CLAUDE.md。
- 公開リポ: https://github.com/aogaki/tpcdaq-rs(実データ・FW・実 .dat は reference/ = .gitignore)。

## 次にやること(次セッションの入口 — 順序は 044 で裁定済み・ユーザー合意 2026-08-15)

**043 → 033 → 044(リファクタ窓)→ P4 UI 実配線 → 031 soak → ELI-NP** の順。

1. **[043_ecc_error_surfacing.md](043_ecc_error_surfacing.md)** — READY(SPEC v1.14 適用済み。
   P4 の前提)。
2. **[033_error_path_semantics.md](033_error_path_semantics.md)** — READY(裁定済み +
   2026-08-15 追記あり: v1.14 の `forced_eos:false` 意味論を A に織り込む。041 の実測
   logbook が照合材料)。発注書 A/C/D/E。**run 経路コアの最後の計画変更**。
3. **[044_refactor_window.md](044_refactor_window.md)** — リファクタ窓(BLOCKED、033 完了で
   開く。**P4 起票前に必ず実施**。タイミング方針・進め方・凍結ルールはチケット本文が正)。
4. **P4 Run 制御 UI 実配線チケット群の起票**(Fable — 044 の後)。
5. **[031_load_harness.md](031_load_harness.md)** — 負荷ハーネス。**044 の後に実走**
   (soak は実機に持っていく最終形で行う — 044 の裁定)。vcobo-daq の全速モードがソースに
   使える。**24 h 実走はマシン占有なのでユーザー合意が要る**。
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
   旧 ROOT 互換 / grawToEventTPC 実機互換 / zCoBo リンク本数 / 先方 LAN 条件。

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

- **Warsaw 確認**: TPCReco 再配布許諾(020 — third_party/tpcreco 昇格の条件)/
  PROPOSAL v0.5 反映判断。
- **物理屋向け資料・デモは UI + ファイルデータソース完成まで待つ**(ユーザー決定)。
- 小粒フォローアップ(次に該当ファイルを触るユニットへ相乗り): vcobo-daq の SIGINT
  graceful 化(041 発見⑤ — 現状 stop_demo.sh が SIGKILL で対処、実害なし)/
  geometry.rs の参照アクセサ
  (Aux ch の per-sample String 確保解消 — 026 申し送り)/ poisoned 時 metrics を
  `PoisonError::into_inner` で読む(023 申し送り。032 では実施済み)/
  **SPA deep link 404**(`ui_dir` = ServeDir 直配り。Rust 側変更が要る)/
  UI の未使用 lazy チャンク(jsroot geom の three addons 2.38 MB)。
- **delila-rs への申し送り**: pop_for 競合 → issue 化済み
  https://github.com/ELI-NP/delila-rs/issues/26 / ZMQ fair-queue 飢餓(013)も要点検。

## 運用メモ(常時適用はメモリ・CLAUDE.md 側が正)

- C++ の make は必ず `-j`。
- **実 .graw + 実 mini ジオメトリの正しいペア**はメモリ参照(合成 fixture と混同しない)。
- リプレイ経路のライブ起動レシピ = [archive/028_web_ui_monitor.md](archive/028_web_ui_monitor.md)
  の付録 A(実走で実証済み。落とし穴一覧つき)。
- どんな小修正でも連番チケット + 結果節 + archive(ユーザー方針)。

## 完了ユニット台帳

000〜042 すべて [archive/](archive/) に結果節つきで格納(単位の詳細・テスト実測値・逸脱の裁定は
すべて各 md の「結果」節が正。未完は 031/033/043 のみ)。
直近(2026-08-15、仮想 zCoBo トラック): **038** 実 ECC ローカルスパイク / **039** 実 ECC
フルウォーク実走 / **040** vcobo-daq 本体 / **041** 統合デモ(実 ECC 歩き戻し初実証)/
**042** ConfigId 3 相化(SPEC v1.13)。前日(2026-08-14): 027〜037(Web UI / P3 E2E /
run 制御ハードニング / モデル運用)。
