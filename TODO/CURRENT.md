# CURRENT — tpcdaq-rs 現在地

**最終更新: 2026-08-14(P3 完了 + run 制御ハードニング波。前半波の詳細は
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
- **リポ全体ゲート: 402 passed / 0 failed / 1 ignored**。C++ 側 `test_ecc_bridge 187` /
  `ecc_e2e 29` / root_sink 各テスト green。clippy -D warnings クリーン。
  UI 側 適合 **13 files / 131 tests EXIT=0**、初期バンドル 502 kB。
- 実装の正本 = **docs/SPEC_ja.md v1.12**。モデル使い分け・完了時ルール = CLAUDE.md。
- 公開リポ: https://github.com/aogaki/tpcdaq-rs(実データ・FW・実 .dat は reference/ = .gitignore)。

## 次にやること(次セッションの入口)

1. **[033_error_path_semantics.md](033_error_path_semantics.md)** — **裁定済み・SPEC v1.12 適用済み・
   036 完了で着手条件が揃った = すぐ発注可**(implementer/Opus)。発注書 A(`run_stop` に
   `forced_eos`/`eos_closed`)/ C(decoder `eos_out` カウンタ)/ D(異常系 E2E 3 本 =
   `tests/p3_error_paths.rs`)/ E(受信静止検出 `eos_quiesce_ms` 既定 500 ms)。
2. **[031_load_harness.md](031_load_harness.md)** — 負荷ハーネス(§12-5 24h / §12-6 10 分)。
   **034/036 で連続 run が回るようになったので前提は外れた**。**Warsaw 前必須**。
   **24 h 実走はマシン占有なのでユーザー合意が要る**。
3. **P5 実機展開** = docs/WARSAW_PLAN_ja.md。**現地の確認項目は 032/036 の結果節に
   機械確認手段つきで書いてある**(`extra_connections` / `peer` / audit の `ecc_walk_back` /
   run/start 所要と `ecc_timeout` 60 s の余裕)。

## この波(2026-08-14 後半)で決まったこと・分かったこと

### ユーザー裁定

- **連続 run は「毎 run 完全リセットして一からやり直す」**(ワルシャワ大学の作法に合わせる)。
  オペレータに手で `ecc/reset` を挟ませない。
- **2D ヒストの stats box は出さない**(目的は各ストリップの時間変化を一枚絵にすること。
  統計量としては意味を持たない)。1D は残す。
- **Run 制御は完成形レイアウト + 全 disabled、モック禁止**(2026-08-13 決定を 3 ユニットとも継承)。

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

## 保留・確認事項

- **Warsaw 確認**: TPCReco 再配布許諾(020 — third_party/tpcreco 昇格の条件)/
  PROPOSAL v0.5 反映判断。
- **物理屋向け資料・デモは UI + ファイルデータソース完成まで待つ**(ユーザー決定)。
- 小粒フォローアップ(次に該当ファイルを触るユニットへ相乗り): geometry.rs の参照アクセサ
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

000〜036 すべて [archive/](archive/) に結果節つきで格納(単位の詳細・テスト実測値・逸脱の裁定は
すべて各 md の「結果」節が正)。直近(2026-08-14): 027/028/029 Web UI / 030 P3 E2E /
032 receiver 単一リンク / 034 連続 run / 035 README / 036 ECC 歩き戻し。
