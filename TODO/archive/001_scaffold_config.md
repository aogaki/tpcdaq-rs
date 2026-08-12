# 001 — ワークスペース scaffold + 設定読込

**Status: COMPLETED**(2026-08-12。実装 = implementer/Sonnet、レビュー = Fable)
**仕様**: SPEC §3(設定スキーマ)、§1.1(リポ配置)
**依存**: なし

## やること

1. cargo ワークスペース初期化(Rust 2021)。`src/` 共有 lib + `src/bin/` コンポーネント毎バイナリ方式
   (SPEC §1.1。バイナリは当面プレースホルダで可)。
2. コーディング規約の機械化: `clippy::unwrap_used` を deny(production コード)。
   `cargo fmt && cargo clippy --tests -- -D warnings && cargo test` が通る状態を作る。
3. 設定モジュール: SPEC §3 の TOML を serde で読む。
   - `[[cobo]]` 配列(id / listen / data_sender_id)、`[system]` / `[decoder]` / `[root_sink]` /
     `[monitor]` / `[controller]`。
   - 既定ポート規約(§3.2)の適用と明示上書き。
   - `deny_unknown_fields`(キーの typo を silent にしない)。
   - 検証: cobo id 重複、listen 重複、geometry パス存在。
4. パースエラー・検証エラーは**起動失敗**(半端な既定値で走らない — SPEC §3.2)。

## テスト

- mini 1 CoBo / ELITPC 2 CoBo 相当の合成 TOML が読めて期待値どおり。
- 不正系(id 重複・ポート衝突・未知キー・欠落必須キー)がすべて Err。
- 既定値(ポート規約・source_id 割当)の充足。

## 受け入れ

- 上記テスト green + fmt / clippy `-D warnings` 通過。

## 結果

**実施日: 2026-08-12**

### 実行環境

- OS: macOS 26.5.2 (Darwin 25.5.0, arm64)
- rustc 1.97.1 (8bab26f4f 2026-07-14) / cargo 1.97.1 (c980f4866 2026-06-30)
- libzmq: Homebrew 4.3.5(`pkg-config --modversion libzmq` で確認、事前インストール済みでビルド阻害なし)

### 実行コマンドと結果

```
$ cargo test
running 15 tests
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo clippy --tests -- -D warnings
Finished `dev` profile [unoptimized + debuginfo] target(s) — warning 0 件

$ cargo fmt -- --check
(差分なし)
```

TDD 手順: `src/config.rs` を「doc コメント 1 行 + テストモジュールのみ」のスタブに戻した状態で
`cargo test` を実行し、未定義シンボル(`PathBuf` / `DEFAULT_MONITOR_WS_LISTEN` /
`DECODER_SOURCE_ID` 等)によるコンパイルエラー(red)を確認 → 実装を復元して green を確認。

### 新規テスト(`src/config.rs` の `tests` モジュール、全 15 件 green)

- `mini_1_cobo_toml_parses_with_expected_values` — SPEC §3.1 の mini TOML そのものが期待値どおり読める
- `elitpc_2_cobo_toml_parses_with_expected_values` — ELITPC 相当 2 CoBo(値は mini と非対称にして取り違え検出)
- `cobo_listen_defaults_to_port_base_plus_id_when_omitted` — `listen` 省略時 `46005+id`(id=7 で検証)
- `monitor_ws_listen_defaults_when_omitted` — `ws_listen` 省略時 `0.0.0.0:9000`
- `controller_rest_listen_defaults_when_omitted` — `rest_listen` 省略時 `0.0.0.0:8080`
- `source_id_constants_match_spec_3_2` — `DECODER_SOURCE_ID=100` / `PSU_SOURCE_ID=200`
- `duplicate_cobo_id_is_err` / `duplicate_cobo_listen_is_err` — 重複検証
- `unknown_field_in_system_is_err` / `missing_required_key_experiment_is_err` — `deny_unknown_fields` と必須キー欠落
- `geometry_path_missing_is_err` — `parse()` 経由での geometry 未存在エラー
- `validate_geometry_path_ok_for_existing_file` / `validate_geometry_path_err_for_missing_file` — 分離した検証関数の単体テスト
- `load_reads_file_and_parses` / `load_missing_file_returns_io_err` — ファイル読込 API

### 変更ファイル

- `Cargo.toml`(新規)— パッケージ `tpcdaq`、Rust 2021、依存(serde/toml/thiserror/serde_json/
  rmp-serde/tokio/zmq/tmq)、`[lints.clippy] unwrap_used = "deny"`
- `Cargo.lock`(新規、コミット対象 — `.gitignore` は `Cargo.lock.orig` のみ除外)
- `src/lib.rs`(新規)— `pub mod config;` 本体 + `geometry`/`msg`/`command`/`zmq_helper` の空スタブ宣言
- `src/config.rs`(新規、本体実装)— TOML → `Config` のパース・既定値解決・検証
- `src/geometry.rs` / `src/msg.rs` / `src/command.rs` / `src/zmq_helper.rs`(新規)— doc コメント 1 行のみの空スタブ(002/003 の作業場所)
- `src/.gitkeep` を削除(`src/` に実ファイルができたため不要)

### 発注書からの逸脱・判断に迷った点(設計分岐は実装せず報告)

1. **§3.2 の既定ポート規約のうち、TOML フィールドとして実装したのは
   `cobo.listen`(`46005+id`)/ `monitor.ws_listen`(`9000`)/ `controller.rest_listen`(`8080`)
   の 3 つと `source_id` 定数(`DECODER_SOURCE_ID=100`, `PSU_SOURCE_ID=200`)のみ。**
   §3.2 表にはこの他に graw-writer PULL(47001)/ decoder PULL(47002)/ root-sink PULL・PUB
   (47003/47004)/ controller ログ投稿 PULL(47005)/ コンポーネント REP(47100+連番)/
   ecc-bridge REP(47200)も既定値として載っているが、これらは SPEC §3.1 の TOML 例にも
   001 発注書が列挙する 5 セクション(`[system]`/`[decoder]`/`[root_sink]`/`[monitor]`/
   `[controller]` — `[graw_writer]` 節は無い)にも一切現れない。よってこれらは 001 の
   スコープ外(各コンポーネントが自身の TODO で内部固定値として持つか、後続ユニットで
   TOML フィールド化するかは未確定)と判断し、`Config` 構造体には追加しなかった。
   この切り分けが意図と違う場合は指摘してほしい。
2. `parse(&str)` に加えてファイル読込用の `load(path)` を追加した(「TOML を読む」の自然な
   実用最小構成として)。発注書に明記はないが新規依存追加や抽象化は伴わない小さな追加。
3. `thiserror` はバージョン指定が発注書になかったため `cargo add` 解決の最新版(2.0.20)を
   採用した。delila-rs は `thiserror = "1"` — 揃えが必要であれば差し替える。
4. `data_sender_id` と `decoder.workers` は §3.2 の既定値表に無いため必須フィールド(省略不可)
   とした(id からの自動導出などは行っていない)。
5. `geometry` パス存在チェック(`validate_geometry_path`)はモジュール内 private 関数として分離。
   発注書の「ユニットテストが一時ディレクトリで通せる形に」を満たすため、同一クレート内の
   テストから直接呼べる可視性にとどめ、公開 API には出していない。

いずれも実装を止めるほどの分岐ではないと判断し進めたが、1. は特に P1 以降(graw-writer 等の
TODO)のスコープと接続するため、レビューでの確認を推奨する。

### レビュー裁定(Fable、2026-08-12)

コードレビュー + ゲート独立再実行(fmt / clippy -D warnings / test 15 passed)で確認済み。
報告された判断 5 点の裁定:

1. **内部 ZMQ ポート(47001–47200)の TOML フィールド化は「それを使うユニットで行う」で確定**
   (SPEC §3.2 の「すべて設定で上書き可」は最終形の要求。P1 以降の各ユニットが自分の
   エンドポイントを config に足す)。
2. `load(path)` 追加 — 承認。
3. thiserror 2.x — 承認(delila-rs との整合が要るのはワイヤ形式であり内部エラー型ではない)。
4. `data_sender_id` / `decoder.workers` 必須 — 承認(実機の罠に直結する値は明示 > 暗黙)。
5. `validate_geometry_path` private — 承認。

残課題(ブロッカーではない、後続で拾う): listen 文字列の SocketAddr 形式検証と
`46005+id` のポート範囲上限チェックは未実装 — P1 receiver ユニットの入力検証に含めること。
