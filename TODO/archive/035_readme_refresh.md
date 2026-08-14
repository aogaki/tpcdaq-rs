# 035 — ルート README の現況反映(Status 行 / SPEC 版番号 / P3 完了分)

**Status: COMPLETED**(2026-08-14 implementer/Sonnet → 発注側レビュー PASS)

## 結果

- **変更は `README.md` の 1 ファイルのみ**(発注側が `git diff README.md` を実見して確認)。
  `git status --porcelain -- src tools tests docs TODO ui` に本ユニットの差分なし。
- 直した箇所: ①Status 行 → **P0–P3 完了**(「run-control groundwork in place」は**維持** —
  Run 制御は disabled レイアウトのみで REST 未配線なので**事実として正しいまま**。良い判断)
  ②SPEC 版番号 v1.8 → **v1.12** ③Implemented so far にモニタ経路 + Web UI を追加
  (JSROOT で 9 ヒスト + イベント表示 / ECharts で生波形 / ログブック / Run 制御は P4)
  ④Next 行を「24 時間 soak + Warsaw 実機展開」へ ⑤Layout の `ui/` 行は 029 が追記済みで作業不要。
- **逸脱 1 件(受理)**: 発注書本文は「v1.11」と書いていたが、`docs/SPEC_ja.md` のヘッダ実物と
  `TODO/CURRENT.md` が **v1.12** を示していたので v1.12 を採用。
  **発注書より現物を優先したのは正しい**(発注書が起票後の版上げに追随していなかった = 発注側の
  更新漏れ)。
- 実測値の創作なし(既存の「3852 events, 0 differences」を再利用)。相対リンクは全て実在確認済み。
- 実行環境・日付: macOS Darwin 25.5.0、2026-08-14。テスト実行は不要(README のみ)。
**起票**: 2026-08-14(029 の申し送り)
**発注先想定**: implementer/**Sonnet**(事実の反映だけ。設計判断なし)

## 事実

ルート `README.md` は 029 で **UI 段落と Layout の `ui/` 行だけ**追記された。残りが古い:

- **Status 行**: 「receive → decode → dual storage complete (P0–P2), run-control groundwork in place」
  → 実際は **P3 も完了**(モニタ集計 → PUB → monitor → WS → **Web UI 3 ユニット**、
  §12-7〜12-11 も 030 でクローズ)。
- **SPEC 版番号**: 「v1.8」→ 現行 **v1.11**。
- 「Next: online monitoring (histogram aggregation, WebSocket streaming, web UI)」→ 済み。
  次は負荷ハーネス(031)+ P5 実機展開。
- `tools/` の説明に **ecc-bridge の `--no-data-link`** 等の細目は不要だが、
  root-sink / ecc-bridge の現況(実データ検証済み)は 1 文で足せる。

## やること

- 上記を**事実だけ**反映(誇張しない。数値を書くなら実測値のみ = CURRENT.md と各 archive の結果節が正)。
- 英語のまま(README は英語、SPEC は日本語という現行の切り分けを崩さない)。
- **内部情報を書かない**(実データのパス・コラボ内部事情・FW の詳細)。

## 受け入れ

- ファイル所有権: ルート `README.md` のみ。
- Rust/C++/TODO/docs に触らない。`cargo test` に無影響(README だけなので自明)。
