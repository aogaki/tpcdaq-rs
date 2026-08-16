# 063 — UI 表示文字列の全面英語化(Warsaw 向けプロポーザルのスクリーンショット前提)

**Status: COMPLETED**(2026-08-16 — 結果は末尾。起票同日。
背景: Mikolaj / Wojciech 教授の好感触を受け、UI スクリーンショット入りの英語
ミニプロポーザルを作る)
**発注先想定**: implementer/**Opus**(文字列は機械作業だが、テスト 192 本の追随と
物理用語の一貫性に判断が残る)

## スコープ(確定)

- **対象 = 画面に表示される文字列すべて**: ラベル・見出し・説明文(hint)・ツールチップ・
  確認ダイアログ・ボタン・空状態文言・UI が組み立てるエラー表示の枠(例:
  「操作権がありません — …」)・aria-label・タブ名・単位表記の前後語。
- **対象外**: コード内コメント(日本語のまま — 我々の作業言語で SPEC 参照つき)/
  サーバ由来の文字列(controller の error・notes は素通し表示のまま)/ ログ・テスト名 /
  histogram 名等のワイヤ由来識別子(StripTimeU 等 — 元々英語)。
- i18n フレームワークは**入れない**(KISS。英語直書きに置換。日英切替は非スコープ)。

## 用語集(この訳語で統一)

操作権 = control token(節見出しは "Control")/ 取得 = Acquire / 手放す = Release /
横取り = takeover / 監査ログ = audit log / 状態 = Status / ポーリング = polling /
最終取得 = last update / 今すぐ取得 = Refresh now / 実行中の run はありません =
No run in progress / 誰も持っていません = (token) held by no one / 破壊的操作の確認 =
confirmation (destructive) / 静止検出 = quiesce / 歩き戻し = walk-back / 面 = plane /
ストリップ = strip / 時間バケット = time bucket / 波高 = pulse height / 生 ADC =
raw ADC / 積算 = cumulative / 減算 = subtraction / ベースライン = baseline /
イベント表示 = Event display / 波形 = Waveform / 飽和 = saturation / 間引き =
decimation / 取りこぼし = dropped。SPEC・R 番号への言及は "SPEC §8.1" 等そのまま。

## 品質基準

- 物理屋(GET/TPC ユーザー)が読んで自然な英語。直訳調・冗長化を避け、既存の
  文の情報量(SPEC 参照・数値・理由)は落とさない。
- 文体統一: ボタン = 動詞句(Title Case 不要、先頭大文字)/ hint = 平叙文。
- スクリーンショット映えを意識(不自然な改行・あふれの確認 — 長文化したら簡潔に)。

## 受け入れ

- `grep -r '[ぁ-んァ-ヶ一-龠]' ui/src --include='*.html'` が 0 件。
  `.ts` 側は**ユーザー可視文字列に限り** 0 件(コメントは残る)。
- `npx ng test --watch=false` **192 passed / 5 skipped(件数不変 — 文字列 assert の
  追随のみ、テストの意味・数は変えない)**。`npm run build` green + prettier クリーン。
  dist 再ビルドまで(デモがそのまま英語 UI を配信できる状態)。
- 実スタックでの見た目確認(スクリーンショット)は発注側・ユーザーが行う。
- 報告: 変更ファイル一覧 / 訳語の判断に迷った箇所と選択理由 / テスト結果。
  **コミットしない**(発注側レビュー後)。

## 非スコープ

- 日英切替(i18n)/ コメント翻訳 / サーバ側(Rust/C++)の文字列 / docs 類。

---

## 結果(2026-08-16 — implementer/Opus 実装、発注側(Fable)レビュー PASS)

- **22 ファイル**(テンプレート 7 / インラインテンプレート 3 / ts 可視文字列 8 /
  spec 追随 4)、+181/−172。i18n 機構なし(直書き、KISS)。
- **ゲート(発注側追試済み)**: html の日本語 **0 件** / ng test **192 passed /
  5 skipped(件数不変)** / prettier クリーン / build green / **dist 再ビルド済み**
  (残る CJK は ECharts 同梱ロケール表のみ = サードパーティ)。
- **裁定**: ①HTML コメントは翻訳(ブラウザに配信されるため受け入れ条件「html 0 件」を
  優先 — 正当。scss/ts コメントは日本語のまま)②`stopped (ok/fault)`・`result:
  ok/failed` の簡潔形 — 受理 ③`unavailable (null)`(実測 0 と読ませない工夫)—
  受理 ④`不明`→小文字 `unknown` でワイヤ状態 `Unknown` との区別を温存 — 受理
  ⑤開発者向け invariant エラーの英語化 — 受理(害なし)⑥テスト名・フィクスチャの
  サーバ出力模擬文字列は日本語のまま — 発注書どおり。
- 実行環境: macOS Darwin 25.5.0、2026-08-16。

**Status: COMPLETED**
