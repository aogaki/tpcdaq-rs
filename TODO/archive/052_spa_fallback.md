# 052 — SPA deep link の 404 解消(controller の index.html fallback)

**Status: COMPLETED**(2026-08-15 — 結果は末尾)
**Status(起票時): READY**(起票 2026-08-15 Fable — 050 未決①。長年の小粒が P4 で実害化)
**発注先想定**: implementer/**Sonnet**(axum の定型 + テストで縛れる)

## 事実

controller の静的配信は `ServeDir` 直配りのため、`/run` 等のクライアントルートへの
**直リンク・リロードが 404**(トップからの遷移は正常)。050 のブラウザ受け入れデモでも
「必ずトップから」の但し書きが要る状態。

## やること

- `src/controller.rs` の静的配信(`ui_dir`)に **index.html fallback** を追加
  (`ServeDir::new(dir).fallback(ServeFile::new(dir.join("index.html")))` 相当 —
  tower-http の定型)。
- **`/api/...` と WS の経路には影響しないこと**(fallback は静的側のみ。ルータの
  マージ順を確認)。
- 新規テスト: ①実在しないパス(例 `/run`)への GET が 200 + index.html の内容
  ②`/api/status` 等の API 経路が従来どおり ③実在する静的ファイルは従来どおり。
  (テスト用の仮 ui_dir はテンポラリディレクトリに index.html を置く形で可 —
  既存の ui_dir テストがあればその流儀に従う。)

## 受け入れ

- 既存テスト無変更で `cargo fmt && cargo clippy --tests -- -D warnings && cargo test`
  全 green(+新規テスト)。
- 結果節: テスト一覧と結果 / 変更行数。

## 結果(2026-08-15 implementer/Sonnet → 発注側(Fable)レビュー PASS)

- `src/controller.rs` のみ +123/−2: `ServeDir(..).fallback(ServeFile(index.html))` の定型。
  API ルートは `.route()` で先に確定するため fallback は静的側のみ(マージ順無変更)。
- 新規テスト 3 本(実サーバ + 素の HTTP GET): `/run` → 200 + index.html /
  `/api/status` 不変(JSON)/ 実在静的ファイル不変。
- ゲート: **cargo 435 passed / 0 failed**(432 + 3、既存無変更)/ fmt clean /
  clippy --all-targets 警告ゼロ。
- 逸脱: `#[tokio::test(flavor = "multi_thread")]` — current-thread だと同期 TcpStream の
  blocking read がサーバタスクを飢餓させる(実測・再現確認済み)ための必須設定 = **受理**
  (新規依存なし)。
- 実行環境: macOS Darwin 25.5.0、2026-08-15。

**Status: COMPLETED**
