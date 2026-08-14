# 027 — Web UI 基盤: Angular workspace + WS クライアント/本番デコーダ + §10.4 適合 + シェル

**Status: COMPLETED**(2026-08-14 implementer/Opus → 発注側(Opus)レビュー PASS)

## 結果

### 実装(新規 `ui/` 38 ファイル + ルート `.gitignore` +9 行のみ)

- **本番デコーダ** `ui/src/app/ws/wire.ts`(344 行): §10.1 の 13 B ヘッダ + §10.2 の 4 型。
  **例外を投げず `DecodeError` を値で返す**(理由 6 種: short-header / bad-magic / bad-version /
  unknown-type / truncated / length-mismatch)。呼び手が数えて status バーに出す。
- `json.ts`(341)= §10.3 の meta/status/run パーサ + subscribe エンコーダ(未知キー・未知 type を
  **数えて**無視 = 前方互換)/ `endpoints.ts`(106)= 接続先の決定規則 / `state.ts`(58)=
  staleness + 指数バックオフ(500 ms → 上限 8 s、ジッタ 50–100 %)/ `ws-client.ts`(357)=
  `WsClientService`(signals。再接続・購読・最新値保持・全カウンタ)。
- シェル(5 ルート + サイドナビ)+ status バー + プレースホルダ + `run_ws_conformance.sh` +
  `ui/dev/monitor.toml`(開発用 config)+ `proxy.conf.json` + `ui/README.md`。
- テスト 1077 行(wire / ws-client / json / endpoints / state / conformance)。

### テスト(2026-08-14、macOS Darwin 25.5.0 / Node 26.7.0 / Angular CLI 22.1.4 / vitest 4.1.10)

**発注側で再実行して確認した値**(エージェント報告の数字と一致):

| コマンド | 結果 |
|---|---|
| `bash ui/run_ws_conformance.sh` | **6 files / 56 tests passed、EXIT=0** |
| `npm test`(フィクスチャ無し) | 5 files passed / **1 skipped**、51 passed / **5 skipped** |
| `npm run build` | 成功。**初期バンドル 488.65 kB raw / 116.77 kB transfer**(main 480.61 + styles 8.04) |
| 失敗時の挙動(エージェント確認) | 生成物の ADC 1 バイトを +1 → `1 failed / 55 passed`、**EXIT=1** |

- **skip の理由**: `TPCDAQ_WS_SAMPLE` 未設定時は適合 5 本だけ skip(通常の `npm test` を
  Rust ビルドに依存させない設計)。適合は `run_ws_conformance.sh` が必ず走らせる。
- **§10.4 照合値**(Rust `ws_sample_messages()` の**生成規則を読んで TS 側で独立に起こした**もの。
  バイト列のコピペ無しを実装者が申告、レビューでもコピペの痕跡なしを確認):
  - 0x02: run=7 / event=42 / plane=1(V)/ 3×4、`adc[(strip-1)*4+bucket] == strip*10+bucket` 全 12 ビン
  - 0x03: run=7 / event=43 / **incomplete=true** / cobo=0 / asad=1 / 2×3×2、
    `adc[(aget*3+ch)*2+bucket] == aget*100+ch*10+bucket` 全 12
  - 0x10: id=4 / nbins=4 / x=[0,4096] / bins=[0,1.5,2.25,3.75](ε=1e-5)/ **event=0**
  - 0x11: id=1 / 3×2 / x=[1,4] y=[0,2]、**PUB の ix 外側 → ワイヤ iy 外側**を
    `bins[iy*3+ix] == pub[ix*2+iy]` で 6 ビン全数(§5.3 → §10.2 の転置の受け側検証)
- **デコード実測**(実機規模): Uvw 226×512 = 0.019 ms/msg、Waveforms 4×68×512 = 0.012 ms/msg、
  Histo2d 226×512(463 kB)= 0.021 ms/msg。slice+typed array は DataView 逐次読みの **2.5 倍速**。
  1 Hz×9 + 20 Hz でも合計 ~1 ms/s。
- **ライブスモーク(実 `monitor` プロセス・モック無し)**: `cargo run --bin monitor --
  --config ui/dev/monitor.toml`(root-sink 無し)+ `ng serve` で
  ①接続 → `meta` 表示 ②status 未着 → **stale**(全カウンタが `0` ではなく `—`)
  ③monitor 停止 → **offline**(赤・別文言)④再起動 → **自動再接続で復帰(RECONNECTS 4)**。
  monitor 側ログで `subscribe applied ... StreamSet { uvw: true, waveforms: false, histos: true,
  status: true }` を確認 = **既定 waveforms OFF が実サーバで成立**。
  スクリーンショット 5 枚(scratchpad、リポ外)。
- **Rust/C++ 無変更**: `git status --porcelain -- src tools tests Cargo.toml Cargo.lock docs
  config examples third_party` が **0 件**。`cargo test` は不要(1 行も触っていない)。

### レビュー(発注側 Opus)

- **バイトレイアウトを Rust 本番エンコーダ(`src/monitor.rs` の `ws` モジュール)と
  1 フィールドずつ突き合わせて一致を確認**: 本体オフセット 18 / 19 / 27 / 35、
  id・nbins・軸の型と順序、LE。SPEC §10.2 の表とも一致。
- 逸脱 7 件は**すべて受理**:
  ①**常に slice で 1 回コピー**(型ごとにビュー/コピーが変わる罠を消す + 実測 2.5 倍速。
  BE 環境は DataView 経路 + `console.info` 1 回 = silent 禁止に適合)/ ②vitest は
  `ng new` 既定がすでに `@angular/build:unit-test` + vitest(切替不要)/ ③Angular CLI
  **22.1.4 は Node 26.7.0 で警告なし**(ローカルの ng 20.2.0 は使わず `npx ng` に統一)/
  ④`tsconfig` に `strict: true` 追加 / ⑤`EndpointsService`(決定規則は Angular 非依存側に
  置いてテスト済み)/ ⑥プレースホルダは 1 コンポーネント + route `data`(KISS)/
  ⑦健全性 4 値(`offline` / `waiting` / `fresh` / `stale`)+ カウンタ追加
  (`jsonErrors` / `unknownJsonTypes` / `reconnects` / `lastIssue`)= 「未接続と status 未着を
  区別」「silent にしない」の素直な実装。
- **手順違反として記録**: `endpoints` の spec だけ red を観測せずに書いた(他 4 モジュールは
  red 実測済み)。内容は受理。次回は全モジュールで test-first。
- **未解決として次ユニットへ送るもの**(下記「申し送り」)。

### 申し送り

1. **バンドル budget**: 初期 488.65 kB に対し既定の warning が 500 kB(error 1 MB)。
   **028 の方針(発注側決定)**: JSROOT / ECharts は必ず遅延チャンクに置き、ビルドの
   chunk 表で初期チャンクに入っていないことを確認する。シェルの成長だけで 500 kB を
   超えたら `maximumWarning` を 600 kB に上げ、理由を結果節に記録する。
2. **Google Fonts の CDN 参照が `index.html` に残っている**(`ng add @angular/material` 生成。
   CSS はビルド時にインライン化されるが `.woff2` は runtime に `fonts.gstatic.com` へ取りに行く)。
   **オフラインの DAQ 機ではシステムフォントに落ちる** —— 致命的ではないが Warsaw 展開前に
   self-host か削除の判断が要る。**029(意匠)で処置**。
3. **SPA deep link**: `ui_dir` = `ServeDir` 直配りのため `/monitor` を直接打つと 404。
   `/` から入れば全ルート動く。SPA fallback は Rust 側変更なので別ユニット(必要なら起票)。
4. 028 が使う口: `ws.histos()`(id→最新 Map)/ `ws.uvwByPlane()`(0=U,1=V,2=W)/
   `ws.waveforms()` / `ws.meta()` / **`ws.setWaveforms(true/false)`**(表示中だけ ON。
   値が変わったときだけ subscribe 再送)。`decodeBinary` の返す `adc`/`bins` は**常に独立配列**。
5. 029 が使う口: `EndpointsService.apiBase()`(既定 `/api`、`ui-config.json` で上書き可)。
   `ng serve` の `/api` proxy 設定済み。

**仕様**: SPEC **v1.11** §11(スタック・描画・デザイン規律)/ §10.1–10.3(WS ワイヤ — TS 側デコーダ)/
§10.4(クロス言語適合 — **TS 側はここで実装**)/ §3.2(WS 9000 / REST 8080)/ §5.2(飽和率)
**依存**: 026(monitor + WS — 実装済みワイヤの正。**申し送りは
[archive/026_monitor_ws.md](archive/026_monitor_ws.md) の結果節を必読**)
**発注先想定**: implementer/**Opus**(アプリ構成・並行(再接続/購読)の裁量が残る)

## 分割の経緯(2026-08-14、Opus セッションのオーケストレーション判断)

元 027(UI 全部 = workspace + 4 ビュー + JSROOT + ECharts + 適合テスト)は 1 ユニットとして
大きすぎ(process 規約「独立にテスト可能な小単位・目安 数百行未満」から大きく逸脱)、
レビュー単位としても粗い。**3 分割**する:

- **027(本ユニット)** — 基盤: workspace / WS 本番デコーダ / WsClient / シェル / status バー / §10.4 適合
- **[028_web_ui_monitor.md](028_web_ui_monitor.md)** — モニタビュー(JSROOT 9 ヒスト)+ 波形ビュー(ECharts)
- **[029_web_ui_control.md](029_web_ui_control.md)** — ログブック + Run 制御レイアウト(全 disabled)+ Power + 意匠仕上げ

ユーザー決定(2026-08-13)は 3 ユニットすべてに引き続き適用する(下記)。

## 確定済みのユーザー決定(変更不可)

- **モック関数・仮バックエンドを作らない**。開発・デモで動くのはリプレイ経路
  (graw_replay → receiver → decoder → root-sink → monitor → WS)のみ。
- **Run 制御ボタン類は完成形レイアウト + 全 disabled**(→ 029)。

## 発注側で確定させた設計(実装者はこのとおりに作る — ここを変えたくなったら実装せず報告)

1. **接続先の決定規則**(SPEC §3.2 で controller REST = 8080、monitor WS = 9000 と**別プロセス・
   別ポート**。本番は controller が UI 静的ファイルを配信するので、UI から見て REST は同一
   オリジン・WS は別ポートになる):
   - **WS 既定** = `ws://{ページの hostname}:9000/ws`(ページが https なら `wss://`)。
     **パスは `/ws` 固定**(026 申し送り)。
   - **REST 既定** = same-origin の `/api/...`。
   - **上書き**: 起動時に same-origin の `ui-config.json` を fetch し、`{"wsUrl": "...",
     "apiBase": "..."}` があれば優先。404 / パース失敗は既定へフォールバックし **`console.info` を
     1 回出す**(silent failure 禁止)。このファイルはリポにコミットしない(運用配置物)。
   - **開発時** = `ng serve` の proxy で `/api` → `http://localhost:8080`。**WS は :9000 直結**
     (WS に CORS は無いので proxy 不要 — 経路を増やさない)。
   - controller 側に WS プロキシを足す案は **Rust 変更なので本ユニットの範囲外**(採らない)。
2. **027 に ECharts / JSROOT を入れない**(初期バンドルを太らせない — §11。028 が追加する)。
   027 の依存は Angular + Angular Material のみ。
3. **テストランナは vitest 一本**。純ロジック(デコーダ / URL 解決 / staleness 判定 / 購読状態)の
   テストに集中する。Angular CLI 既定が karma/jasmine なら `@angular/build:unit-test`(vitest)へ
   切り替えてよい。**DOM の見た目テストは書かない**(UI の自動 E2E は P3 E2E ユニット送り)。
4. **Angular / Node**: `npx @angular/cli@latest new` の最新安定を使う。この環境は Node 26 で
   Angular CLI が "Unsupported" 警告を出す可能性がある。**警告なら無視して進む**が、
   **実行を拒否されたら実装を止めて報告**(nvm 等の導入を勝手にしない)。

## やること

1. **`ui/` に Angular workspace**(standalone components。Angular Material。lockfile はコミット、
   `node_modules` は .gitignore)。`ng build` の出力を controller の `ui_dir`(016 実装済み)で
   そのまま配信できる構成にする(SPA の deep link は `ui_dir` = `ServeDir` 配信のため
   **ハッシュルーティングは使わず**、代わりに 5 ルートすべてが index からの遷移で成立すれば可 —
   直 URL 叩きが 404 になる件は既知として `ui/README.md` に 1 行残す。Rust 側の fallback 変更は
   本ユニットの範囲外)。
2. **WS 本番デコーダ**(純 TS・**Angular 非依存**の 1 モジュール。vitest から直接 import できること):
   - `decodeBinary(ArrayBuffer)` → 判別可能 union。13 B ヘッダ(§10.1: magic `'T''P'` /
     msgType / version=2 / flags bit0=incomplete / u32 runNumber / u32 eventNumber、すべて LE)+
     ボディ 4 種(§10.2):
     - `0x02 Uvw`: `u8 plane`, `u16 nStrips`, `u16 nBuckets`, `u16 ADC × nStrips*nBuckets`
       (**strip-major**: `idx=(strip-1)*nBuckets+bucket`)
     - `0x03 Waveforms`: `u8 cobo`, `u8 asad`, `u8 nAget`, `u8 nCh`, `u16 nBuckets`,
       `u16 ADC × nAget*nCh*nBuckets`(**aget-major**、raw ch 順、FPN 込み・減算なし)
     - `0x10 Histo1d`: `u16 id`, `u32 nbins`, `f32 xmin`, `f32 xmax`, `f32 × nbins`
     - `0x11 Histo2d`: `u16 id`, `u16 nx`, `u16 ny`, `f32 xmin,xmax,ymin,ymax`,
       `f32 × nx*ny`(**iy 外側 row-major** — monitor 側で転置済み)
   - **異常入力は例外を投げずエラー値を返す**(magic 不一致 / version≠2 / 長さ不足 / 宣言と
     実長の不整合)。呼び手が数え、status バーに `decodeErrors` として出す(silent 禁止)。
   - **ゼロコピーの是非を判断して報告**: 13 B ヘッダのため本体オフセットは 2 バイト境界に
     揃わないことがあり、`new Uint16Array(buf, off, n)` はアライメント例外になり得る。
     DataView 逐次読み / `slice` してから typed array / 独自コピーのどれを採ったか、
     512×N 規模での実測(ざっくりで可)とともに報告する。
   - **JSON**(§10.3): `meta` / `status` / `run` の型定義 + パーサ、`subscribe`(C→S)の
     エンコーダ。**casing は SPEC 文言どおり**: status 本体 = snake_case
     (`events_built` 等)、追加 3 キーのみ camelCase(`monitorGaps` / `clients` / `wsDropped`)。
     未知キー・未知 type は落とさず無視 + カウント(前方互換)。
3. **`WsClientService`**(Angular signals):
   - 接続 / 自動再接続(指数バックオフ、上限 5–10 s、ジッタあり)/ 手動再接続。
   - `subscribe` 送信。**既定は waveforms OFF**(§10.3)。**波形 ON/OFF を外から切り替える
     公開 API**(028 の波形ビューが表示中だけ ON にする)。
   - 最新 `meta` / `status` / `run` の保持、最新 `Uvw` / `Histo*` の signal 公開(028 が読む)。
   - **staleness**: `status` が **3 秒**途絶 = stale(026 申し送り: monitor は独自タイマを持たない
     ので root-sink 停止が status 途絶として現れる)。接続はしているが status が来ない状態と、
     WS 未接続の状態を**区別して**表現すること。
   - カウンタ: 受信通数(型別)/ `decodeErrors` / 再接続回数。
4. **アプリシェル**: Material toolbar + サイドナビ + **5 ルート**(`/monitor` `/waveform`
   `/logbook` `/run` `/power`)。**各ビューは空のプレースホルダ**(「028 で実装」「029 で実装」と
   明記したカード)。既定ダークテーマ(意匠の作り込みは 029 — ここは骨格のみ)。
5. **status バー(全ページ共通)**: `state` / `run` / `events_built` / **saturation %(U/V/W)** /
   `monitorGaps` / `wsDropped` / `clients` / staleness / `decodeErrors`。
   - saturation % = `saturated / counted * 100`(§5.2。`counted == 0` は「—」)。
   - **未知と 0 を混同させない**: 未接続・status 未着は `—` + stale バッジ。
6. **§10.4 適合(TS 側)**: `ui/run_ws_conformance.sh` 1 本で
   ① `cargo run --bin ws_proto_sample -- --out <tmp>`(026 実装済み)→ ② **本番デコーダ**を
   import する vitest がその連結ストリーム(`u32 LE 長さ + ペイロード`)を分解・デコードし
   既知値と照合(float は **ε=1e-5**)→ ③ 一時ファイル掃除。**フィクスチャはコミットしない**
   (毎回再生成 = 陳腐化が構造的に起きない)。既知値の正は
   `tpcdaq::monitor::ws_sample_messages()`(src/monitor.rs)—— **Rust 側を読んで期待値を
   独立に起こすこと**(バイト列をコピペしない)。
   TS 側独立レイアウトテスト(§10.4-4: バイトオフセット assert、異常入力)も併置。
7. **ルート `.gitignore`** に `ui/node_modules/` `ui/dist/` `ui/.angular/` 等の**行追加のみ**。
8. **`ui/README.md`**: 開発(`npm start`)/ 本番ビルド(`npm ci && npm run build` → 出力を
   controller の `ui_dir` に指す)/ 適合テストの回し方 / 接続先の決定規則(上記 1)/
   既知の制限(直 URL の deep link)を簡潔に。ルート README への導線は 029 で足す。

## テスト・受け入れ

- `npm run build`(= `ng build`)成功。**初期バンドルサイズを報告**(028 の JSROOT 遅延ロードの
  基準値になる)。
- vitest green: デコーダ単体(4 型のバイトオフセット / 既知値 / 異常入力でエラー値)+
  JSON パーサ(casing・未知キー)+ URL 解決規則 + staleness 判定 + 購読状態。
- `bash ui/run_ws_conformance.sh` が **green で終了コード 0**、失敗時は非 0。
- **ライブスモーク(バックエンドは実物のみ・モック禁止)**: `monitor` 単体を起動
  (root-sink 無しでよい —— `meta` は monitor がジオメトリから作るので接続時に届き、
  `status` は来ないので **stale が出るのが正**)→ UI が接続し `meta` を表示、staleness バッジ、
  monitor を落として再起動 → 自動再接続が復帰することを確認。**スクリーンショット 1 枚を
  報告に添付**(status バーと stale 表示が見えるもの)。
  - 起動レシピは別途調査済みのものが CURRENT に載る予定。無ければ `tests/monitor_e2e.rs` と
    `src/config.rs` から最小 config を自分で起こしてよい(**リポには置かない** — `ui/dev/` 配下か
    一時ディレクトリ)。
- **Rust/C++ は無変更**(`git status` で `ui/` とルート `.gitignore` 以外に差分が無いこと。
  Rust 側の変更が必要になったら**実装せず報告して戻る**)。
- ファイル所有権: `ui/` 全部(新規)+ ルート `.gitignore`(行追加のみ)。
  `src/` `tools/` `docs/` `TODO/` `tests/` には触らない。
  発注書に無い設計分岐(接続先規則の変更・依存の大物追加・Rust 変更)に出会ったら
  **実装せず報告して戻る**。

## 完了時(CLAUDE.md 絶対ルール)

本 md に `## 結果` 節(実行コマンド / テスト数 green・red / 実測値(バンドルサイズ・適合値)/
実行環境と日付 / スキップとその理由 / 逸脱と申し送り)を書き、`Status: COMPLETED` にして
`TODO/archive/` へ移動、`CURRENT.md` を更新する。
