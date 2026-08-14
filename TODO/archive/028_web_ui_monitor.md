# 028 — Web UI: モニタビュー(JSROOT 9 ヒスト)+ 波形ビュー(ECharts)

**Status: COMPLETED**(2026-08-14 implementer/Opus → 発注側(Opus)レビュー PASS。
**2 件を 029 の必須処置として送る** — 下記「レビュー」)

## 結果

### 実装(`ui/` のみ。新規 13 ファイル + 変更 4)

- **純ロジック**(jsroot を import しない → vitest から本物の `createHistogram` を渡して検証):
  `monitor/root-histo.ts`(228)= §5.2 の 9 枚の行列定義 + **ROOT の fArray レイアウト変換**
  (TH1: `fArray[i+1]` / TH2: `fArray[(iy+1)*(nx+2)+(ix+1)]`。ワイヤ 2D は iy 外側、
  Uvw は strip-major なので**転置して**入れる)/ `waveform/waveform-select.ts`(114)=
  (cobo,asad) キャッシュ・(aget,ch) 抽出・系列上限と stride 間引き(**落とした数を申告**)。
- 描画: `jsroot-loader.ts` / `jsroot-panel.ts`(202)/ `monitor-view.*` / `echarts-loader.ts` /
  `waveform-view.*`(336+111+97)/ 共有の `display/display-clock.ts` + `display-controls.ts`
  (interval・freeze・イベント ID)/ `src/jsroot.d.ts`(配布物の型が空宣言のため使う分だけ)。
- 変更: `app.routes.ts`(`/monitor` `/waveform` を `loadComponent` の遅延ルートへ)/
  `angular.json`(`externalDependencies` + budget warning 500→600 kB)/ `package.json` +
  lock(`jsroot@7.11.1` / `echarts@6.1.0`)/ `ui/README.md`(028 節)。

### テスト(2026-08-14、macOS Darwin 25.5.0 / Node 26.7.0 / Angular CLI 22.1.4)

**発注側で再実行して確認**(エージェント報告値と一致):

| コマンド | 結果 |
|---|---|
| `bash ui/run_ws_conformance.sh` | **8 files / 75 tests passed、EXIT=0**(027 の適合を壊していない) |
| `npm test`(フィクスチャ無し) | 70 passed / 5 skipped(**新規 19 本**。skip は 027 と同じ §10.4 の 5 本) |
| `npm run build` | 成功。**初期 509.44 kB**(027 比 +20.8 kB = 遅延ルート化に伴うシェル分のみ) |

- **chunk 表で jsroot / echarts が遅延チャンクにあることを確認**(`Addons` 2.38 MB /
  `index`(echarts)1.14 MB / `zstd` 387 kB / `HierarchyPainter` 86.7 kB はすべて lazy。
  初期チャンクを文字列 grep しても `jsroot:0 echarts:0`)。budget warning は発注書の
  条件どおり 600 kB へ(**error 1 MB は据え置き**)。
- **TDD**: `waveform-select` / `root-histo` とも spec 先行で red を実測してから実装(報告に出力あり)。
- **オラクル**: 2D は `bins[iy*nx+ix] → fArray[(iy+1)*(nx+2)+(ix+1)]`、Uvw は
  `adc[(strip-1)*nBuckets+bucket] → fArray[(bucket+1)*(nStrips+2)+strip]` を
  **非対称データ(nx=3/ny=2)で全ビン検査**。発注側もソースを読んで添字を確認済み。

### 実表示(付録 A のリプレイ経路、**実 .graw + 実ジオメトリ**、モック無し)

- WS 実測: `meta`(planes U72/V92/W92、anglesDeg [90,-30,30])/ `status`(running、
  frames_per_cobo{"0":n})/ `0x02`(73,746–188,451 B)/ `0x10`(2,075 B ×6)。
- **発注側がスクリーンショットを実見**して確認:
  - **9 ヒスト**が 3×3(行 = StripTime / Charge / ChargeMax、列 = U/V/W)で実データ描画。
    1D は x=[0,4096] 固定・512 bins、2D は 72×512 / 92×512(**すべてメッセージ由来**)。
    status バー: STATE running / RUN 1 / EVENTS 3 / SAT 0.0% / GAPS 0 / WSDROPPED 0 /
    DECODEERRORS 0。
  - **波形ビュー**: 「requested 272 series · drawn 16 · **256 series not drawn (max series)**
    · stride 1 · raw ADC, FPN included, no subtraction (R13)」を**画面に明示**
    (= 間引きを silent にしない)。FPN ch がフラットトップで見えており R13 の
    「生 ADC・FPN 込み・減算なし」がワイヤどおり出ている副次的裏取りになっている。
  - **freeze** は「Freeze display」+ イベント ID(`run 1 · event #2` / `complete`)。
    同ページに Stop 系の操作は無く、run Stop と混同する余地なし(§5.4 適合)。
- **Rust/C++ 無変更**(`git status --porcelain -- src tools docs Cargo.toml` = 0 件)。

### §11 確認項目の回答

- **①ズーム保持 = 成立(実測)**。CDP で**実マウスドラッグ**を送ってズーム → x 軸ラベルが
  `["1000","1200",…]` に変わり、**1 Hz 更新を 3 回以上跨いでも同じラベル**のまま
  (`ZOOM_PRESERVED=true`)。同時に Entries が 72→144 に増えて**更新が実際に起きている**こと、
  隣の非ズームパネルが [0,4096] のままであることも確認。機序も jsroot 7.11 のソースで裏取り:
  `redrawObject` → `updateObject` 内の `if (!fp || !fp.zoomChangedInteractive()) this.checkPadRange();`
  で対話ズーム済みなら pad レンジ再計算を飛ばす。**代替策は不要だった**。
- **②ダークモード = 1D は良好、2D(colz)は不可**(下記レビューで裁定)。実体は
  `settings.DarkMode` → SVG に `filter: invert(100%)`(色相回転なしの単純反転)。
- **③ライセンス = JSROOT は MIT**(`node_modules/jsroot/LICENSE` 実物 + package.json + lock で
  一致確認)。**ECharts は Apache-2.0**(LICENSE + NOTICE 実物)。両方 `ui/README.md` の表に記録。

### レビュー(発注側 Opus)

- 逸脱 12 件のうち **10 件は受理**: `externalDependencies`(jsroot の node 専用 動的 import を
  外部化。ブラウザでは通らない経路)/ `jsroot/core`+`jsroot/draw` のサブパス限定 import /
  `jsroot.d.ts` / budget 600 kB / 表示クロックの共有 / 波形の (cobo,asad) キャッシュ
  (`ws.waveforms()` は最新 1 通しか持たない 027 設計の当然の帰結)/ イベント ID を表示クロックに
  同期(freeze 中に「絵は #77・ラベルは #80」の嘘を作らない — **良い判断**)/ モニタ 2 タブ /
  echarts フル import(遅延チャンクなので初期に影響なし)/ `<option [selected]>` 修正。
- **2 件は「029 の必須処置」として差し戻し**(下記)。どちらも**物理屋の誤読を招く**ため
  意匠の好みの問題ではないと判断した。
- **申し送り**(次に触るユニットへ): `graw_replay --loop` は 2 周目以降 eventIdx 重複で
  `events_built` が止まる(長時間表示は `--rate-mbps` を下げて 1 周を延ばす)/ `npm ci` が
  jsroot 経由で native の `canvas` を引く(ブラウザ用途では不使用。Warsaw で `npm ci` が
  詰まったらこれが候補)/ 未使用の巨大 lazy チャンク(`Addons` 2.38 MB = jsroot geom の
  three addons)が dist に残る / ECharts は `notMerge: true` なので波形のズームは更新毎にリセット。

**仕様**: SPEC **v1.11** §11(描画スタック・確認項目①②③)/ §5.2(ヒスト定義・軸固定 0–4096・
飽和天井)/ §10.2(0x02 Uvw / 0x03 Waveforms / 0x10 Histo1d / 0x11 Histo2d)/ §5.4(freeze は
表示のみ)/ R9・R13
**依存**: **027 = COMPLETED**([archive/027_web_ui_foundation.md](archive/027_web_ui_foundation.md)
の結果節と申し送りを**必読**)
**発注先想定**: implementer/**Opus**(JSROOT painter の更新戦略・描画性能の裁量が残る)

## 確定済みのユーザー決定(変更不可)

- **モック関数・仮バックエンドを作らない**。動くのはリプレイ経路のみ(付録 A)。

## 027 が用意済みの口(これを使う。再実装しない)

- `WsClientService`(signals): `histos()` = **id → 最新ヒスト**の `ReadonlyMap`(§5.2 の id 1–9)/
  `uvwByPlane()` = `[U, V, W]` の最新 `0x02` / `waveforms()` = 最新 `0x03` / `meta()` / `status()` /
  `run()` / `health()`(`offline`/`waiting`/`fresh`/`stale`)。
- **`ws.setWaveforms(true|false)`** — 波形ビューが**表示中だけ ON**(離脱で OFF)。値が変わった
  ときだけ subscribe を再送する実装済み。
- `decodeBinary` の返す `adc`(`Uint16Array`)/ `bins`(`Float32Array`)は**常に独立配列**
  (元バッファのエイリアスではない)。`Histo2d.bins` は **iy 外側 row-major**(`idx = iy*nx + ix`)。

## 発注側で確定させた設計(このとおりに作る。変えたくなったら実装せず報告)

1. **依存の入れ方**: `jsroot` と `echarts` を npm 依存に足し、**動的 `import()` で遅延ロード**
   (ngx-echarts のようなラッパは入れない = KISS。Angular の遅延ルートと組み合わせる)。
   **ビルドの chunk 表で初期チャンクに入っていないことを確認して報告**。
   027 実測の初期チャンクは **488.65 kB**(既定 warning 500 kB / error 1 MB)。
   シェル成長だけで 500 kB を超えたら `ui/angular.json` の `maximumWarning` を **600 kB** に
   上げ、理由を結果節に書く(**error 1 MB は上げない**)。
2. **モニタビューの構成**: 9 ヒストは **U/V/W を列、`StripTime`(2D)/`Charge`(1D)/`ChargeMax`(1D)
   を行**の 3×3 グリッド(id 1–3 / 4–6 / 7–9 = §5.2)。各パネルに log 切替。2D は `colz`。
3. **軸は固定**(§5.2): 1D の x は **[0,4096] 固定・オートレンジ禁止**(飽和天井 4095 が常に
   見えること)。2D は x=[1,N+1)、y=[0,512)。**軸範囲はメッセージの xmin/xmax/ymin/ymax を使い、
   ビン数はメッセージ由来**(ch 数・ビン数をコードに焼き込まない — プロジェクト不変条件)。
4. **イベント表示**(R9)= `0x02 Uvw` の 3 面(strip×bucket)。**JSROOT の TH2 で描く**
   (ヒストと同じ描画系に揃える)。**イベント ID(`run` / `eventNumber`)を常時表示**、
   `incomplete` フラグも出す。
5. **freeze / interval はクライアント側のみ**(§5.4): freeze は**表示だけ**止める。
   **run Stop と視覚的に混同させない**(文言は「FREEZE(display only)」相当 + Stop とは
   別色。DAQ は動き続けている旨をその場で分かるようにする)。interval は表示更新間隔の選択
   (例: 0.5 / 1 / 2 / 5 s)。**WS の購読は止めない**(status バーは動き続ける)。
6. **ズーム保持**(§11 確認項目①): 1 Hz の更新でユーザーのズーム状態を保つ。
   painter の `updateObject` + redraw を第一候補とし、**実際に検証して結果を報告**
   (ズームしてから 2 回以上の更新を跨いでも範囲が保たれること)。できない場合は
   理由と採った代替(例: 更新の一時停止)を報告。
7. **波形ビュー**(R13): ECharts。**(cobo,asad) 選択 → AGET 選択 → ch 選択**の 3 段
   (`0x03` の `nAget`/`nCh`/`nBuckets` はメッセージ由来 = 焼き込み禁止)。重ね描き / グリッド
   切替。**クライアント側間引き**(描画チャンネル数の上限 + サンプルの stride。間引いたことは
   画面に出す = silent 禁止)。**生 ADC・FPN 込み・減算なし**(R13)をそのまま描く。
8. **§11 確認項目の回答を報告に含める**: ②JSROOT ダークモードの Grafana 風ページへの馴染み
   (所見)/ ③**JSROOT ライセンス最終確認**(MIT のはず。実際に配布物を見て確認結果を書く)。

## テスト・受け入れ

- **vitest(純ロジック)**: ヒストメッセージ → JSROOT オブジェクトの組み立て(ビン配列の並び・
  `nx`/`ny`・軸範囲。**2D の `bins[iy*nx+ix]` が正しい行列位置に入ること**)/ 波形の間引き
  (上限・stride・間引き件数の申告)/ 選択(cobo,asad,aget,ch)の絞り込み。DOM の見た目テストは
  書かない(P3 E2E 送り)。
- `npm run build` 成功 + **chunk 表**(jsroot / echarts が遅延チャンクにあること)。
- `bash ui/run_ws_conformance.sh` が引き続き **EXIT=0**(027 の適合を壊していない)。
- **実表示確認(モック禁止)**: **付録 A のリプレイ経路**を実際に流して
  **9 ヒスト + イベント表示 + 波形**が動く。**スクリーンショットを報告に添付**
  (①9 ヒスト全体 ②ズーム保持の前後 ③波形ビュー ④freeze 表示中)。
- **Rust/C++ 無変更**(`git status --porcelain -- src tools tests docs TODO Cargo.toml` が 0 件)。
- ファイル所有権: **`ui/` のみ**(`package.json` / `package-lock.json` / `angular.json` の
  必要最小限の変更を含む)。`ui/README.md` に 028 分を追記してよい。
  **`TODO/` `docs/` `src/` `tools/` に触らない**。commit / add / ブランチ操作をしない。
  発注書に無い設計分岐に出会ったら**実装せず報告して戻る**。

## 完了時(CLAUDE.md 絶対ルール)

本 md に `## 結果` 節(実行コマンド / テスト数 green・red / 実測値(chunk 表・ズーム保持の
検証方法と結果・§11 確認項目②③の回答)/ 実行環境と日付 / スキップとその理由 / 逸脱と申し送り)を
書き、`Status: COMPLETED` にして `TODO/archive/` へ移動、`CURRENT.md` を更新する。

## 付録 A — リプレイ経路のライブ起動レシピ(2026-08-14 実走で実証済み)

実データ(mini eTPC 実 .graw、108 events / 15,040,512 items)で**通して確認済み**。
実資産のパスは**環境変数で渡す**(リポに実パスを書かない):
`TPCDAQ_REAL_GRAW`(実 .graw)/ `TPCDAQ_REAL_GEOMETRY_MINI`(実 mini ジオメトリ .dat。
**`tests/fixtures/geometry_mini_reduced.dat` は合成の別物** — 実 .graw と組ませない)。

**controller と ecc-bridge はバイパスする**(controller の Arm/Start は実 ECC 操作を伴うため、
検出器なしのデモでは各コンポーネントの command REP を直接叩く = `tests/p2_e2e.rs` の `rpc()` と同じ)。

```text
# 0) cargo build --bins   (root_sink は tools/root_sink/root_sink に既にビルド済み。
#                          ROOT の env 設定は不要 — バイナリが LC_RPATH /opt/ROOT/lib を持つ)
# 1) root_sink  --geometry <実 .dat> --output-root <dir>     … "monitor PUB bind" を待つ
# 2) monitor    --config <toml>                              … "monitor WS listening" を待つ
#    (SUB は slow-joiner。PUB 接続後 300–400 ms の猶予を置く = テスト群と同じ margin)
# 3) graw_writer / decoder / receiver --cobo-id 0            … 各 "command socket listening" を待つ
# 4) 各 command REP へ Configure → Arm → Start(この順。**Arm までデータポートは開かない**
#    = listen-before-start。graw_writer=47100 / decoder=47101 / receiver(cobo0)=47110)
# 5) graw_replay 127.0.0.1:46005 <実 .graw>                  … 全速送出 → TCP FIN
#    受信側は FIN を run 境界と解釈し EndOfStream を伝播 → ROOT ファイルが確定する
#    (明示 Stop は不要。UI の長時間表示には `--rate-mbps` / `--loop` を使う)
```

最小 config(SPEC §3.2 の既定でポートは全部揃うので上書き不要。`[root_sink]` と
`[controller]` は `src/config.rs` の検証を通すためだけに必要 — controller は起動しない):

```toml
[system]
experiment = "mini_eTPC"
output_root = "<scratch>/data/graw"
geometry = "<実 .dat のパス>"

[[cobo]]
id = 0
listen = "127.0.0.1:46005"
data_sender_id = "CoBo[0]"

[decoder]
workers = 1

[root_sink]
snapshot_hz = 1.0
event_publish_hz = 20.0
build_timeout_ms = 1000

[monitor]
ws_listen = "0.0.0.0:9000"

[controller]
passphrase = "demo-only-not-used"
ecc_proxy = "GetEcc:tcp -h 127.0.0.1 -p 46002"
config_id = "default"
```

**観測実績**(`ws://127.0.0.1:9000/ws`、15 s): meta ×2 / status ×15 / run ×2 /
`0x02 Uvw` ×66(plane=U nStrips=72 nBuckets=512)/ `0x10 Histo1d` ×90(id=4 nbins=512
xmin=0 xmax=4096)/ `0x11 Histo2d` ×45(id=1 nx=72 ny=512 xmin=1 xmax=73 ymin=0 ymax=512)。
EOS 後の status は `events_built=108` / `frames_per_cobo={"0":108}`(= P1 オラクル)。

**落とし穴**(実走で踏んだもの):
- **Arm しないとデータポートが開かない**(listen-before-start)。`graw_replay` が接続に失敗する。
- コマンド JSON は serde の externally-tagged 表現: unit variant は裸の文字列(`"Arm"` / `"Stop"`)、
  struct variant は `{"Configure":{"run_number":1}}` / `{"Start":{"run_number":1}}`。
  `{"Arm":null}` はパース失敗。
- **ログはファイルへリダイレクトしても ANSI 色つき**。`component="graw-writer"` のような
  `key="value"` 文字列で grep 待ちすると**永久に一致しない**。素のメッセージ文
  (`command socket listening` 等)で待つこと。
- Rust バイナリの graceful stop は **SIGINT のみ**(`kill -INT`)。SIGTERM ハンドラは無い
  (root_sink は SIGINT/SIGTERM 両方 graceful)。
- macOS の `/bin/bash` は 3.2.57(連想配列なし)。スクリプトは POSIX 寄りに書く。
