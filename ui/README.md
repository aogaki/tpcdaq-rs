# tpcdaq Web UI

Angular + Angular Material の operator UI。**027 = 基盤**(WS 本番デコーダ / WsClient /
シェル / status バー / SPEC §10.4 適合テスト)、**028 = モニタビュー(JSROOT 9 ヒスト +
イベント表示)と波形ビュー(ECharts)**、**029 = ログブック / Run 制御(レイアウトのみ・
全 disabled)/ Power + 意匠仕上げ**。5 ビューすべてが埋まった。

モックデータ・仮バックエンドは**作らない**(ユーザー決定 2026-08-13)。開発中に画面へ値が
出るのは、実物のリプレイ経路(graw_replay → receiver → decoder → root-sink → monitor → WS)を
動かしているときだけ。

## 開発

```sh
cd ui
npm ci          # 初回のみ(lockfile はコミットされている)
npm start       # ng serve → http://localhost:4200
```

`ng serve` は `/api` を `http://localhost:8080`(controller REST)へ proxy する
(`proxy.conf.json`)。**WS は proxy しない** —— monitor の :9000 へ直結する
(WS に CORS は無いので経路を増やさない)。

monitor だけを相手に動作確認したいとき(検出器も root-sink も不要):

```sh
# リポジトリのルートから(config 内の geometry パスが相対)
cargo run --bin monitor -- --config ui/dev/monitor.toml
```

root-sink がいないので `status` は届かない → status バーが **stale** になるのが正
(monitor は独自タイマを持たない。026 申し送り)。`meta` は monitor がジオメトリから
作るので接続直後に届く。

## 本番ビルド

```sh
cd ui
npm ci
npm run build   # → ui/dist/ui/browser
```

出力を controller の `[controller] ui_dir` に指すと controller(:8080)が配信する。

```toml
[controller]
ui_dir = "/path/to/tpcdaq-rs/ui/dist/ui/browser"
```

### 既知の制限 — deep link

`ui_dir` は `ServeDir` でそのまま配るだけなので、`/monitor` などを**ブラウザに直接
打ち込むと 404** になる。`/` から入ってサイドナビで移動する分にはすべて動く。
SPA fallback(未知パス → `index.html`)を足すには Rust(controller)側の変更が要るので、
027 の範囲外。

## 接続先の決定規則

SPEC §3.2 で controller REST = 8080、monitor WS = 9000 の**別プロセス・別ポート**。

|      | 既定                                                                                |
| ---- | ----------------------------------------------------------------------------------- |
| WS   | `ws://{ページの hostname}:9000/ws`(ページが https なら `wss://`)。パスは `/ws` 固定 |
| REST | same-origin の `/api/...`                                                           |

起動時に same-origin の `ui-config.json` を fetch し、`{"wsUrl": "...", "apiBase": "..."}`
があれば優先する。404 / パース失敗 / 型違いは既定へ戻し、**`console.info` を 1 回出す**
(silent failure を作らない)。このファイルは環境ごとの運用配置物なのでリポには入れない
(`.gitignore` 済み)。

```json
{ "wsUrl": "ws://daq-pc.lan:9000/ws", "apiBase": "https://daq-pc.lan/api" }
```

本番は `ui_dir` が指す配信ディレクトリの直下(`index.html` の隣)に置く。
開発中に試したいときは `ui/public/ui-config.json`(`.gitignore` 済み)に置けば
`ng serve` / `ng build` がそのまま配る。

## テスト

```sh
npm test                  # vitest(@angular/build:unit-test)。純ロジックのみ
bash run_ws_conformance.sh # SPEC §10.4 クロス言語適合(Rust 生成 → TS 本番デコーダ)
```

適合テストは毎回 `cargo run --bin ws_proto_sample` でフィクスチャを作り直して
一時ディレクトリに置き、終了時に消す(**コミットしない** = 陳腐化が構造的に起きない)。
`TPCDAQ_WS_SAMPLE` が無いときは適合テストだけ skip され、他の単体テストは走る。

DOM の見た目テストは書かない(UI の自動 E2E は P3 の E2E ユニット送り)。

## 描画スタック(028、SPEC §11)

| 何を | 何で | どこ |
| ---- | ---- | ---- |
| 9 ヒスト(§5.2)+ イベント表示(0x02 Uvw) | **JSROOT** の TH1D/TH2D | `/monitor` |
| 波形(0x03) | **ECharts** | `/waveform` |

- 両方とも **遅延ロード**。`/monitor` `/waveform` は `loadComponent` の遅延ルートで、
  その中でさらに `await import('jsroot/core' | 'jsroot/draw' | 'echarts')` する。
  ラッパライブラリ(ngx-echarts 等)は入れない。
- **初期チャンクに jsroot / echarts は入らない**(`ng build --verbose` の chunk 表で確認)。
- JSROOT は `jsroot`(= `modules/main.mjs`)を **import しない**。geom / three / io / tree / gui を
  全部引くため。使うのは `jsroot/core`(`createHistogram`)と `jsroot/draw`(`draw` / `redraw` /
  `cleanup`)だけ。型は `src/jsroot.d.ts` に**使う分だけ**書いてある(配布物の `types.d.ts` は
  空宣言のみ)。
- 更新は `redraw(dom, obj, opt)` = JSROOT の `updateObject` + `redrawPad`。
  jsroot 7.11 の `THistPainter.updateObject` が `zoomChangedInteractive()` を見るので、
  **ユーザーのズームは 1 Hz 更新を跨いで保たれる**(028 で実測確認)。
- **ダークモードと colz パレット(029 必須処置 A)**: JSROOT の DarkMode は canvas SVG への
  `filter: invert(100%)`(色相回転なしの単純反転)なので、既定の **57 = kBird** では低域の
  暗い青が**画面で一番明るいクリーム色**になる = 「明るい = 多い」と読む目には逆に見える。
  `jsroot-loader.ts` で **`settings.Palette = 56`(kInvertedDarkBodyRadiator = 53 の逆順)**
  にしてあり、反転後は**低 = 暗い / 高 = 明るい**の明度単調なランプになる。
  1D(`hist`)はパレットを使わないので影響なし。
- **2D は stats box を出さない(029 必須処置 B = ユーザー裁定)**: StripTime{U,V,W} と
  イベント表示の目的は**各ストリップの時間変化を一枚絵にすること**で、統計量としては意味を
  持たない。`panelDrawOption()` が 2D にだけ `nostat` を付ける。1D(Charge / ChargeMax)は
  波高分布なので stats box を**残す**(1 エントリ 1 カウントなので `fEntries = Σbins` が正しい)。
- `angular.json` の `externalDependencies` に **node 専用の jsroot 依存**
  (`jsdom` / `@resvg/resvg-js` / `canvas` / `tmp` / `xhr2` / `mathjax` / `@oneidentity/zstd-js`)を
  並べてある。jsroot の `BasePainter.mjs` が `isNodeJs()` ガード付きの動的 `import()` で
  これらを参照するため、外さないと esbuild が `node:fs` 等を解決できずビルドが落ちる。
  **ブラウザでは実行されない経路**なので外部化して問題ない。

### freeze / 表示間隔(§5.4)

`DisplayClock`(`src/app/display/`)がクライアント側だけで表示の刻みを持つ。
**freeze は表示だけ**止める —— WS の購読も DAQ も保存も積算も止まらない
(status バーは動き続ける = 画面でそれが見える)。run Stop と混同させないため、
文言は「FREEZE (display only)」+「DAQ / 保存 / 積算は動き続けています」、色は
Stop の赤ではなくシアン。飛ばした表示更新の回数もその場に出す(silent 禁止)。

### 波形ビューの間引き

`(cobo,asad) → AGET → ch` の 3 段選択。`nAget` / `nCh` / `nBuckets` は**メッセージ由来**
(焼き込み禁止)。描画系列数の上限とサンプルの stride で間引き、**落とした系列数・
サンプル数・範囲外指定の数を必ず画面に出す**。波形は**生 ADC・FPN 込み・減算なし**(R13)。

`WsClientService.waveforms()` は最新 1 通しか持たないので、(cobo,asad) を選ばせるために
ビュー側で面毎の最新を覚える(`waveform-select.ts` の `updateWaveformCache`)。

## ログブック / Run 制御 / Power(029)

| 何を | どこから | どう |
| ---- | -------- | ---- |
| ログブックのタイムライン | `GET /api/logbook?since_seq=N` | **5 s ポーリング**(WS には載っていない) |
| コメント追記 | `POST /api/logbook/comment {author, text}` | **token 不要**(SPEC v1.10 §8.1 の明文化された例外) |
| 状態(phase / run / components / ecc) | `GET /api/status` | 5 s ポーリング。閲覧系なので認証なし |
| Run 制御・ECC 段階操作 | —— | **REST を呼ばない**(レイアウトのみ・全 disabled) |

- **Run 制御は完成形レイアウト + 全 disabled**(ユーザー決定 2026-08-13: モック関数・
  仮バックエンドを作らない)。切替点は `src/app/run/run-actions.ts` の
  **`RUN_CONTROL_ENABLED` 1 定数**。P4 ではこれを `true` にし、`run-view.ts` の
  `proceed()`(唯一の配線点)に `POST` を足す。破壊的操作(run/stop・ecc stop/breakup/reset)は
  確認ダイアログを通る作りにしてある(ネイティブ `<dialog>`。ガワまで実装済み)。
- **追従は `since_seq`**: 取得済みの最大 `seq` を次の `since_seq` に使う。重複しない・
  巻き戻らない(`logbook-feed.ts`、テストあり)。
- **silent failure を作らない**: ポーリング失敗(URL と HTTP ステータスと最終試行時刻)/
  `tail_corrupt`(JSONL 末尾行の破損 = SPEC §9.1)/ 未知 `type` の件数 / 取り込めなかった
  行の件数を、すべて画面上端に出す。`GET /api/status` が取れないときは値を出さない
  (取れていないものを `0` や「正常」と書かない)。ECC の `Unknown` は**「不明」**であって
  「異常」ではないので色も文言も分けてある。
- **未知の `type` は捨てない**(前方互換 — P6 の psu 詳細化に備える)。生 JSON で表示して数える。

## フォント(外部ネットワーク依存ゼロ)

`ng add @angular/material` が `index.html` に置いた Google Fonts の CDN 参照は**削除した**。
CSS はビルド時にインライン化されるが `.woff2` は runtime に取りに行くので、オフラインの
DAQ 機では毎回失敗してシステムフォントに落ちる(Warsaw 展開の前提)。Roboto(可変、latin +
latin-ext)と Material Icons は `public/fonts/` に同梱し、`@font-face` は `src/styles.scss` にある。
**ビルド成果物に外部ホストへの参照は残っていない**(`dist/` を grep して確認)。

## 構成

```
src/app/api/         controller REST(**閲覧系 + コメントだけ**。状態変更系は書かない)
  controller-api.ts     ControllerApiService(fetch 直。3 本だけ)+ apiUrl
  controller-status.ts  **純ロジック** GET /api/status → 画面用(ecc Unknown → 「不明」)
src/app/logbook/     ログブック(遅延ルート)
  logbook-record.ts  **純ロジック** §9.2 の 5 型 + 未知 type の整形
  logbook-feed.ts    **純ロジック** since_seq 追従 / tail_corrupt / 失敗の保持
  logbook-view.*     タイムライン + コメント追記フォーム
src/app/run/         Run 制御(遅延ルート)
  run-actions.ts     **純データ** §8.1 の操作表 + RUN_CONTROL_ENABLED
  run-view.*         状態表示(GET /api/status)+ 全 disabled のボタン + 確認ダイアログ
src/app/power/       Power(P6 のプレースホルダ)
src/app/ws/          WS まわり。Angular 非依存の純 TS(vitest から直接 import)
  wire.ts            §10.1/§10.2 バイナリ本番デコーダ(0x02/0x03/0x10/0x11)
  json.ts            §10.3 meta/status/run のパーサ + subscribe エンコーダ
  endpoints.ts       接続先の決定規則
  state.ts           staleness 判定 + 再接続バックオフ
  ws-client.ts       WsClientService(Angular signals。上の 4 つを束ねるだけ)
  endpoints.service.ts  解決済み接続先の置き場(REST は 029 が使う)
src/app/status-bar/  全ページ共通の status バー(§10.3 + §5.2 の飽和率)
src/app/display/     freeze / 表示間隔(§5.4)。DisplayClock + 共通コントロールバー
src/app/monitor/     モニタビュー(遅延ルート)
  root-histo.ts      **純ロジック** ヒストメッセージ → JSROOT オブジェクト(§5.2 の 9 枚表)
  jsroot-loader.ts   jsroot の動的 import(1 回だけ)
  jsroot-panel.ts    パネル 1 枚(draw / redraw / log 切替 / ResizeObserver)
  monitor-view.*     3×3 グリッド + イベント表示タブ + イベント ID 常時表示
src/app/waveform/    波形ビュー(遅延ルート)
  waveform-select.ts **純ロジック** 選択 + 間引き(落とした数の申告つき)
  echarts-loader.ts  echarts の動的 import(1 回だけ)
  waveform-view.*    3 段選択 + overlay / grid
src/app/views/       残り 3 ルートのプレースホルダ(029 が置き換える)
src/jsroot.d.ts      jsroot サブパスの型(使う分だけ)
```

## ライセンス(同梱する第三者コード)

| パッケージ | ライセンス | 確認 |
| ---------- | ---------- | ---- |
| jsroot 7.11.1 | **MIT** | `node_modules/jsroot/LICENSE`(package.json の `"license": "MIT"` と一致) |
| echarts 6.1.0 | **Apache-2.0** | `node_modules/echarts/LICENSE` + `NOTICE` |
| Roboto(可変、v51) | **Apache-2.0** | `public/fonts/LICENSE.txt`(取得元 URL つき) |
| Material Icons(v145) | **Apache-2.0** | `public/fonts/LICENSE.txt`(取得元 URL つき) |

`wire.ts` は 13 B ヘッダのせいで本体オフセットが型境界に揃わない型があるため、
**常に `ArrayBuffer.slice` で 1 回コピーしてから typed array にする**(型によって
「元バッファのビュー」か「コピー」かが変わらないようにするため)。詳細は
`wire.ts` の冒頭コメント。
