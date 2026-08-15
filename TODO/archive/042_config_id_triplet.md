# 042 — ECC ConfigId の 3 相化(SPEC v1.13)+ ecc_proxy 既定 identity の訂正

**Status: COMPLETED**(2026-08-15 implementer/Opus → 発注側(Fable)diff レビュー PASS)

## 結果

- **全ゲート green(2026-08-15、macOS Darwin 25.5.0 arm64)**:
  `cargo test` **415 passed / 0 failed**(リポ全体ゲート 402 → 415)/
  `cargo fmt --check` clean / `cargo clippy --tests -- -D warnings` 警告ゼロ /
  C++ `test_ecc_bridge` **187 passed** / `ecc_e2e` **29 passed**(ログで `ecc proxy = Ecc:` 確認)/
  UI `npm run build` 成功 + `npm test` **126 passed, 5 skipped**(UI 無変更で非破壊)。
- **実装**: `ConfigIds {describe, prepare, configure}` 型(config.rs、serde untagged で
  文字列/テーブル両対応、相欠けは起動失敗)/ controller run シーケンス + `/api/ecc/` 段階操作が
  相ごとの id を送信(id 不要アクションには載せない)/ logbook `run_start` に非同値時のみ
  `config_ids`(`skip_serializing_if` — nullable 規律)、`config_id` は非同値時 configure 相 /
  `GetEcc:` → `Ecc:` 掃除(ecc_bridge.cpp 既定 + --help、monitor.rs テスト、ui/dev/monitor.toml)。
- **新規テスト 13 本**(config 4 / controller 3 / logbook 3 / 統合 3 — 3 値非対称の実 run で
  ECC 3 リクエストの id と logbook の両形を REST 経由で機械照合)。
- **後方互換の証明**: TOML 文字列形のテストは 1 文字も無変更で green
  (`PartialEq<&str> for ConfigIds` は 3 相一致時のみ真)。run_start の golden 文字列
  無変更 = 同値設定ではワイヤ 1 バイト不変。ecc-bridge C++ は無改造で green。
- **逸脱(受理)**: ①`ControllerParams.config_id` → `config_ids: ConfigIds` 改名 —
  構造体リテラル直書きの既存テスト 4 箇所を機械追従(値・挙動不変)②logbook テストヘルパに
  `config_ids: None` 1 行(enum リテラル全列挙の言語制約)。いずれも後方互換条件を満たす
  ための機械的帰結で、golden テストが非破壊を証明している。
  ③untagged 由来の粗いエラーメッセージは仕様どおり(黙って空 id にならない = 起動失敗)。
  親切化は発注書に無いため未実施 — 妥当。

---

(以下、起票時の発注書)

**Status(起票時): READY**(起票 2026-08-15 Fable — SPEC v1.13 適用済み。039/040 と独立、並行可)
**起票**: 2026-08-15(TODO/038 レーン A の発見 → docs/VIRTUAL_ZCOBO_ja.md §6-R9 裁定 →
SPEC v1.13 として確定済み)
**仕様**: SPEC v1.13 — §3.1(controller 設定例)/ §8.2(相ごとの id の渡し方)/
§9.2(run_start の `config_ids`)/ 改訂履歴 v1.13 項
**発注先想定**: implementer/**Opus**(controller の run シーケンスに触るため)

---

## 背景(1 段落)

実 ECC の ConfigId は describe / prepare / configure の 3 組で、実運用は別名を使う
(実例: `describe=zCobo-ZC706, configure=pulser`)。ecc-bridge の JSON は元よりアクション毎
`config_id` なので変更不要 — 単一 id の焼き込みは **controller 設定(`config.rs` の
`config_id: String`)・run シーケンス・logbook run_start** にある。これを SPEC v1.13 の
形に合わせる。

## やること

### A. 設定(src/config.rs)

- `[controller] config_id` を **「文字列」または「`{ describe, prepare, configure }`
  テーブル」の両対応**に(serde untagged 等 — 実装手段は任せる)。内部表現は 3 相構造体で、
  文字列形は「3 相同値」に展開。
- **後方互換が最優先**: 既存 TOML(文字列形)の全テストが**無変更で** green のままであること。

### B. controller(src/controller.rs)

- run シーケンスと段階操作(`/api/ecc/describe|prepare|configure`)で、ecc-bridge への
  JSON に**当該相の id** を渡す(describe → describe 相、prepare → prepare 相、
  configure → configure 相)。start/stop/breakup/reset 等 id を使わないアクションは現状維持。

### C. logbook(run_start レコード)

- 3 相が**非同値のときのみ** `config_ids: {describe, prepare, configure}` を追加。
  `config_id` は同値ならその文字列、非同値なら **configure 相の id**(SPEC §9.2 v1.13)。
  同値時は `config_ids` を**出さない**(nullable 規律 — null/省略 ≠ 空値の混同をしない)。

### D. 相乗り(小粒、同テーマ)

- `tools/ecc_bridge/ecc_bridge.cpp` の `--help` 既定 proxy 文字列 `GetEcc:` → **`Ecc:`**
  (実 servant identity。fake_ecc.cpp:290 は元より正しい — 038 実測)。
- リポ内の設定例・テスト内文字列の `GetEcc:` も同様に掃除(`src/config.rs` のテスト文字列等。
  proxy 文字列はパースされるだけなので挙動不変の機械置換)。

### E. UI 非破壊確認

- run_start に `config_ids` が来ても UI のログ表示が壊れないことを確認(未知フィールド無視で
  通るはず)。壊れる場合のみ最小修正。表示の追加は**しない**(スコープ外)。

## 受け入れ

- `cargo fmt && cargo clippy --tests -- -D warnings && cargo test` 全 green。
  C++ 側 `tools/ecc_bridge` のテスト(`make -j` + test_ecc_bridge / ecc_e2e)green。
- 新規テスト: ①TOML 両形式のパース(文字列 / テーブル)②非同値設定で各相の JSON
  リクエストに正しい id が載る ③logbook run_start の同値(config_ids 無し)/
  非同値(config_ids 有り + config_id=configure 相)。
- 既存の単一 config_id 挙動は**観測可能な変化なし**(既存テスト無変更で green が証明)。
- 結果節に: テスト数と green/red、実行コマンド、環境と日付。
