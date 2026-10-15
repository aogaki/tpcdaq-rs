# 143 — スキャン台帳(docs/SCANS_ja.md)と表の生成ツール(tools/scan/ledger.py)

**Status: IN PROGRESS**(2026-10-15 起票 Fable。ユーザー「まずはこの一連のスキャンを記録に残しましょう。もちろん今後分量が増えていきます」)

## 1. 台帳(済、本票で作成)
`docs/SCANS_ja.md`: 索引 + 規則 + S1–S9(2026-10-14/15 の全スキャン)。1 スキャン = 1 節。数字は archive チケットから転記。

## 2. 生成ツール(Sonnet implementer、発注)
`tools/scan/ledger.py`(標準ライブラリのみ、`scan.py` / `test_scan.py` の流儀):
- 入力: scan.py の結果 JSON(`scan_<name>_<date>.json`: `name`, `points[] {i, knobs, run, params_id, frames, elapsed_s, stop_reason}`)と指標ディレクトリ(`run%04d_metrics.json` = run_metrics.C の出力)。任意で graw のバイト数(`--run-root DIR` があれば `DIR/run%04d/*.graw` の合計 ÷ events を kB/event で出す)。
- 出力: Markdown の表 1 つ(標準出力)。列 = 点 / run / knobs(base との差分だけ、`k=v` を `, ` で連結)/ events / kB/event(あれば)/ first・last hit 中央値 / 幅 / ヒット ch / 総電荷(M)/ 飽和 event(%)/ クリップ 先頭・末尾(edge、%)/ ok / 長さ 中央値・IQR / 飛跡電荷(M)/ dEdx 中央値・IQR / dip RMS。指標が無い run は「—」。tracks が null の run も落とさない。
- 使い方: `python3 tools/scan/ledger.py SCAN_JSON --metrics DIR [--run-root DIR] [--title "S10 …"]`。`--title` があれば `## <title>` 見出しを先頭に付ける。
- テスト `tools/scan/test_ledger.py`: 合成の scan JSON + 指標 JSON 3 run(1 つは指標なし、1 つは tracks null)+ 擬似 graw(バイト数既知)で、表の行・列・数値の書式(小数、% 表記、M 表記)を固定。`python3 -m unittest tools/scan/test_ledger.py`。既存 `test_scan.py` 8 本の回帰。
- README に節を 1 つ。

## 結果
