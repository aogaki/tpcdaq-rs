# 029 — Web UI: ログブック + Run 制御レイアウト(全 disabled)+ Power + 意匠仕上げ

**Status: COMPLETED**(2026-08-14 implementer/Opus → 発注側(Opus)レビュー PASS)

## 結果

### 実装(`ui/` 18 新規 + 9 変更 + 1 削除、ルート `README.md` に 2 点追記)

- **純ロジック**(vitest。Angular/DOM 非依存): `logbook/logbook-record.ts`(338)= §9.2 の 5 型 +
  未知 type の整形 / `logbook/logbook-feed.ts`(96)= `since_seq` 追従・`tail_corrupt`・
  失敗の保持 / `api/controller-status.ts`(151)= `/api/status` 整形 / `run/run-actions.ts`(126)=
  §8.1 の操作表 + `RUN_CONTROL_ENABLED`。
  **レコードのフィクスチャは `src/logbook.rs` の golden 文字列をそのまま転記**
  (Rust が実際に書く形と TS の読み手が同じ表を見ている担保 — 良い作法)。
- ビュー: `logbook-view`(タイムライン + コメント追記)/ `run-view`(状態表示 + **全 disabled** +
  確認ダイアログ)/ `power-view`(P6 プレースホルダ)。3 ルートとも遅延ルート化し、
  `views/placeholder-view.ts` は死にコードとして削除。
- REST クライアント `api/controller-api.ts`(61)は**閲覧系 2 本 + コメント 1 本だけ**。
  **状態変更系の呼び出しコードは 1 行も無い**(発注側が grep で確認: `fetch` は
  `logbook` / `logbook/comment` / `status` と 027 の `ui-config.json` のみ)。

### 必須処置 3 件(すべて実施 — 発注側が実物で確認)

- **A(2D colz の反転パレット)**: `settings.Palette = 56`(**kInvertedDarkBodyRadiator** =
  53 の色を逆順にした表。`node_modules/jsroot/modules/base/colors.mjs` の rgb 表で確認)を追加し、
  `DarkMode = true` は据え置き。`invert(100%)` 後に**「低 = 暗い / 高 = 明るい」の明度単調な
  ランプ**になる。**発注側がスクリーンショットで確認**: before の「低い値が最も明るいクリーム」→
  after は暗い青 → 白。カラーバーの向きも正しい(2000 が暗、16000 が明)。
  ヒートマップとしては kBird の虹より明度単調のほうが読み違いが少なく、結果的に良い選択。
- **B(2D の stats box)= ユーザー裁定どおり**: `panelDrawOption()` の **2D 分岐にだけ `nostat`**
  (`colz;nostat` / `colz;logz;nostat`)。`fEntries` も `root-histo.ts` の組み立ても**無変更**。
  1D は `hist` / `hist;logy` のままで stats box が残る(スクショで
  `Entries 288 / Mean 3838 / Std Dev 66.01` を確認 — 1 エントリ 1 カウントなので意味が正しい)。
  **テストを先に更新して red を実測**してから実装。
- **C(Google Fonts)**: `index.html` の CDN 参照を削除し、Roboto(可変 wght、latin +
  latin-ext)と Material Icons を `ui/public/fonts/`(190 kB)に同梱 + `@font-face`。
  **発注側がビルド成果物を grep して外部ホスト参照ゼロを確認**。両方 Apache-2.0 でライセンス表に追記。

### テスト(2026-08-14、macOS Darwin 25.5.0 / Node 26.7.0)

**発注側で再実行**: `bash ui/run_ws_conformance.sh` → **13 files / 131 tests passed、EXIT=0**。

| コマンド | 結果 |
|---|---|
| `npm test` | **126 passed / 5 skipped**(新規 **56 本**。既存 75 本は無傷) |
| `npm run build` | 成功。**初期 502.01 kB**(028 の 509.44 kB から **−7.4 kB** — 3 ルート遅延化 + 死にコード削除) |
| prettier | クリーン |

- **TDD 手順を是正**(027 の指摘): 全モジュールで red を実測(①モジュール未解決 5 件 →
  ②スケルトンで **51 failed / 74 passed** → ③処置 B の spec 更新で描画オプション 2 本 red →
  ④実装で green)。
- **実接続確認(モック無し)**: 028 付録 A のリプレイ経路 + **`controller` 実バイナリ**同時起動。
  `GET /api/logbook` / **UI のフォームから** `POST /api/logbook/comment`(seq 6 が即反映)/
  `GET /api/status` が往復。ecc-bridge 無しの `ecc.state = "Unknown"` を画面に
  **「不明」+ 理由**として表示、controller 停止時は赤バナー(URL / HTTP 502 / 最終試行時刻)で
  **取得済み一覧は消えない**ことも実見。

### レビュー(発注側 Opus)

- 逸脱 10 件すべて受理。特に: **確認ダイアログをネイティブ `<dialog>`** に(Material モジュールを
  遅延チャンクに増やさない = KISS)/ `proceed()` を空関数にせず「まだ何も送っていません/
  P4 で配線します」を表示(silent failure 禁止の趣旨)/ `RUN_CONTROL_ENABLED` を一時 true に
  して確認ダイアログと「フラグ 1 つ」を**実証してから** false に戻し、テストが false を assert /
  `formatInt` 自前実装(`toLocaleString()` はロケール依存でテストが環境依存になる)。
- **発注側が確認**: `RUN_CONTROL_ENABLED = false`、状態変更系の REST 呼び出しコードが無いこと、
  `nostat` が 2D 分岐にのみ入っていること、ビルド成果物に外部ホスト参照が無いこと。

### 申し送り

- **P4 の配線点は 2 か所だけ**: `run-actions.ts` の `RUN_CONTROL_ENABLED` と
  `run-view.ts` の `proceed()`。
- **[034](034_consecutive_run_ops.md) の裁定が Run 制御の実配線に波及する**(2 本目以降の
  ECC シーケンス。オペレータに「reset を手で挟め」と要求する UI にしないこと)。
- ログブックのスクショに `run_start` / `run_stop` が無いのは、実 run が ECC 操作を伴い
  検出器なしでは回せないため(5 型の整形は Rust golden を使った単体テストで担保、
  `audit` は ok=true/false 両方を実データで表示確認済み)。
- **ルート README の Status 行と「SPEC v1.8」が古い**(現行 v1.11、026–030 未反映)→
  [035](035_readme_refresh.md) で起票。
- 未解決のまま: **SPA deep link 404**(Rust 側変更が要るので 029 では直さず)。

**仕様**: SPEC **v1.11** §8.1(controller REST — logbook / 操作権 / run / ecc)/ §9.2(ログブック
レコードスキーマ)/ §11(デザイン規律: Atlassian Design 準拠、モニタは Grafana 風ダーク)/ R6・R11
**依存**: **027 = COMPLETED**([archive/027_web_ui_foundation.md](archive/027_web_ui_foundation.md)
の結果節・申し送りを必読)。016(controller REST)は実装済み。**028 と同じ `ui/` を触るので
逐次発注**(package-lock / app.routes の衝突を避ける)。
**発注先想定**: implementer(モデルは 027 の出来を見て決める。REST 貼り込み中心なら Sonnet 可)

## 確定済みのユーザー決定(変更不可)

- **Run 制御ボタン類は完成形レイアウトを置き、全部 disabled**(P4 の REST 配線は後日。
  エンドポイントは §8.1 で確定済みなので、**有効化がフラグ 1 つで済む作り**にしてよい)。
- **モック関数・仮バックエンドを作らない**。

## やること

1. **ログブック**(R11): `GET /api/logbook?since_seq=N` → `{records: [...], tail_corrupt: bool}`
   のタイムライン + **コメント追記**(`POST /api/logbook/comment {author, text}` →
   `{appended: true}` — **token 不要**、§8.1 v1.10。author は自己申告)。
   - **レコードは 5 型**(§9.2。共通の先頭 4 フィールド = `ts` / `seq` / `type` / `actor`):
     `run_start`(run, config_id, geometry, cobos, operator, comment, expected_fragments)/
     `run_stop`(run, duration_s, **ok**, reason, counters, files)/ `audit`(action, params,
     operator, **ok**, error)/ `comment`(author, text)/ `psu`(device, channel, event, values)。
     **型ごとに見せ方を変える**(run_stop の ok=false は目立たせる。counters/files は畳んで表示)。
   - **追従は `since_seq` のポーリング**(既定 5 s 間隔。WS には載っていない)。取得済みの
     最大 `seq` を次の `since_seq` に使う。**ポーリング失敗を silent にしない**(直近エラーを表示)。
   - **`tail_corrupt: true` を必ず可視化**(JSONL の最終行が壊れている = 書き込み中断の痕跡)。
   - 未知の `type` は**捨てずに生 JSON で表示 + カウント**(前方互換 — P6 の psu 詳細化に備える)。
2. **Run 制御**: §8.1 の全操作の**完成形レイアウト**、**全 disabled**(ユーザー決定)。
   - 操作権(`POST /api/control/acquire {operator, passphrase}` → `{token}` /
     `/api/control/release`)の **token 取得 UI 込み**。
   - run(`/api/run/start {token, comment?}` / `/api/run/stop {token}` /
     `/api/run/next {token, next_run}`)、ecc 段階操作
     (`/api/ecc/{describe|prepare|configure|start|stop|breakup|reset} {token}` — R6:
     GET controller と同じ操作感で並べる)。
   - **有効化はフラグ 1 つで済む作り**(例: `RUN_CONTROL_ENABLED = false` の 1 定数を見て
     `disabled` を決める)。**REST を呼ぶコードを書いてよいのは `GET /api/status` と
     ログブックだけ** —— 状態変更系は**呼び出しコードを書かない**(モック禁止の趣旨。
     P4 で配線する)。
   - 破壊的操作(stop / reset / breakup)は**確認ダイアログのガワ**まで(§11)。
   - **`GET /api/status` は読んでよい**(閲覧系・認証不要)。取れたら phase / run /
     components / ecc / notes を表示する(取れなければその旨を表示 — controller が
     動いていない開発時に**赤い嘘を出さない**)。
3. **Power**: タブとプレースホルダのみ(P6)。
4. **意匠仕上げ**: Atlassian Design 準拠 + モニタは Grafana 風ダーク(§11)。ルート `README.md`
   に UI の導線 1 段落(**ルート README はこのユニットが所有**)。
   - **必須処置 A(028 レビューでの発注側裁定 — 意匠の好みではなく誤読防止)**:
     **2D(colz)で「低い値が最も明るく見える」状態は不可**。JSROOT の DarkMode は
     `filter: invert(100%)`(色相回転なしの単純反転)なので kBird の低域(暗い青)が
     **明るいクリーム色**になり、統計の薄い背景が画面で一番明るくなる。オンラインモニタで
     「明るい = 多い」と読む目には**逆に見える** = 実験中の誤判断に直結するので直す。
     採る手は実装裁量(推奨順): ①反転後に kBird 相当になるパレットを `settings.Palette` で選ぶ
     ②2D パネルだけ `DarkMode=false` にしてパネル背景を暗い枠で囲む ③全体 `DarkMode=false`。
     切替点は `ui/src/app/monitor/jsroot-loader.ts` の `DARK_MODE` 定数 1 箇所。
     **1D は現状(反転)で良好**なので壊さないこと。**採った手と before/after のスクショを報告**。
   - **必須処置 B(ユーザー裁定 2026-08-14 — 変更不可)**: **2D(StripTime{U,V,W} と
     イベント表示)は stats box を出さない**。理由(ユーザー): 2D の目的は
     **各ストリップの時間変化を一枚絵にすること**であって、**統計量としては何の意味も持たない**。
     描画オプション(JSROOT の `nostat` 相当)で消す。
     - **1D(Charge / ChargeMax)は stats box を残す**(波高分布なので Entries / Mean / Std Dev は
       意味を持つ。1 エントリ 1 カウントなので `fEntries = Σbins` も正しい値になる)。
     - 発端は「2D の `Entries` が実は Σbins(= ΣADC。実測 173,086,508)」という発注側の指摘だが、
       **裁定は「表示自体をやめる」**なので `fEntries` の値をいじる必要はない
       (`root-histo.ts` は変更不要。描画オプションだけで済むならそれが KISS)。
   - **必須処置 C(027 申し送り)**: `ui/src/index.html` に残る **Google Fonts の CDN 参照**
     (`ng add @angular/material` 生成)。CSS はビルド時にインライン化されるが `.woff2` は
     runtime に `fonts.gstatic.com` を取りに行くため、**オフラインの DAQ 機では毎回失敗して
     システムフォントに落ちる**。self-host(フォントを `ui/public/` に同梱)か参照削除の
     どちらかで**外部ネットワーク依存をゼロにする**(Warsaw 展開の前提)。
5. **027 が用意済みの口**(再実装しない): `EndpointsService.apiBase()`(既定 `/api`、
   `ui-config.json` で上書き可)/ `ng serve` の `/api` proxy 済み / `WsClientService`
   (status バーは配線済み)。
6. **既知の制限として扱うもの**: SPA deep link(`ui_dir` = `ServeDir` 直配りのため `/logbook` を
   直接打つと 404)。**Rust 変更が要るので 029 では直さない**(必要なら別ユニットを起票)。

## テスト・受け入れ

- **vitest(純ロジック)**: レコード 5 型の整形(型別の見せ方・未知 type のフォールバック)/
  `since_seq` 追従(重複しない・巻き戻らない)/ `tail_corrupt` の可視化 / 取得失敗時の表示 /
  Run 制御が**全 disabled** であること(フラグ 1 つで切り替わることも)。
- `npm run build` 成功 + `bash ui/run_ws_conformance.sh` **EXIT=0**(027/028 を壊していない)+
  028 のテストも green のまま。
- **実接続確認(モック禁止)**: **`controller` 実バイナリ**を起動して
  `GET /api/logbook` / `POST /api/logbook/comment` / `GET /api/status` が往復すること
  (`ui/dev/monitor.toml` を参考に開発用 config を用意してよい。ecc-bridge 無しでも
  閲覧系とコメントは動く)。**スクリーンショットを報告に添付**(①ログブック(コメント追記後)
  ②Run 制御の完成形レイアウト(全 disabled が見える)③Power)。
- **Rust/C++ 無変更**。ファイル所有権: **`ui/` + ルート `README.md`**。
  `src/` `tools/` `tests/` `docs/` `TODO/` に触らない。commit / add / ブランチ操作をしない。
  発注書に無い設計分岐に出会ったら**実装せず報告して戻る**。

## 完了時(CLAUDE.md 絶対ルール)

本 md に `## 結果` 節(実行コマンド / テスト数 green・red / 実測値 / 実行環境と日付 /
スキップとその理由 / 逸脱と申し送り)を書き、`Status: COMPLETED` にして
`TODO/archive/` へ移動、`CURRENT.md` を更新する。
