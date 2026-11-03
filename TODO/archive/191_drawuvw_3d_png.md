# 191 — drawUVW に XY / XZ / YZ(模型なしの 3D 点)を足し、既定の出力を事象ごとの PNG に

**Status: COMPLETED**(2026-11-02 起票 Opus、同日完了。実装 = implementer / Sonnet)

## 動機
ユーザー(2026-11-02): 「UVW だけだとなんとも言えない。event 5・27 は明らかに 2 本。XY も入れるべき?」→ 主対話の試作(173 §4 末尾)で、makeTracks の 1 本の直線に頼らない 3D 点が 2 本の事象を見せることを確認。web UI の XY / XZ / YZ は makeTracks の直線に点を置くので 2 本の事象には使えない。
大きさ(run 209 の 20 事象、1200 × 600 の 6 枚組): PDF 569 kB / ページ(全事象 ≈ 4.5 GB、2 GiB 超で ROOT の xref が壊れる — 190)、**PNG 46 kB / 事象(≈ 370 MB)**、JPEG 40 kB(にじむので不採用)。ユーザー「PNG の場合はディレクトリを作成してそこに保存するように」。

## 参照
- `macros/drawUVW.C` / `macros/test_drawUVW.C`(190 で作ったもの — これを拡張する。archive/190 に経緯)。
- 試作(主対話、ad hoc): `/private/tmp/claude-501/-Users-aogaki-Workspace-tpcdaq-rs/c5696e1c-f7d9-4a2e-ab7f-ab99dda8905b/scratchpad/t190/xyproto3.C` と、その出力 `xyproto3.pdf`(event 0 / 5 / 27 / 635)。仕様は下の文章が正で、試作は参考。
- `macros/analyzeUVW.C` の `driftVelocityCmUs` と `DRIFT_LEN_CM`、3 つの必須引数の検査(TODO/178)。`macros/makeTracks.C` の `PITCH`(U = (−1, 0)、V = (0.5, 0.8660254)、W = (0.5, −0.8660254))。
- `tools/scan/test_uvw_closure.C`(合成 map の pitch を 2 mm にして焼き込みを縛る流儀、閉合の定数 c)。

## 作るもの
### 1. 呼び方(190 の引数を置き換える — 今日作ったばかりで利用者はユーザーだけ)
```
void drawUVW(const char *rawFile, double pressureMbar, double samplingMHz, double driftHvVolt,
             const char *out = "", Long64_t first = 0, Long64_t n = -1,
             double drawMinAdc = -1, bool logz = false)
```
- 3 つの run 定数は**必須**。検査は analyzeUVW.C と同じ(サンプリングは 100 / 50 / 25 / 12.5 のどれか、HV > 0、圧力 > 0、外れたら ERROR で何も作らない)。
- `mmPerCell = v / samplingMHz × 10`、v は analyzeUVW.C の `driftVelocityCmUs(driftHvVolt, 20 cm, pressureMbar)` と**同じ値**(関数は `uvwdraw` 接頭辞で写し、E/p の範囲外の WARNING も同じ文言。テストで analyzeUVW.C の値と一致を縛る)。

### 2. 出力
- `out` が空 → `rawFile` の `_raw.root` を `_uvw_events` に替えた**ディレクトリ**に PNG(例 `data/run0209_raw.root` → `data/run0209_uvw_events/`)。入力名に `_raw.root` が無ければ ERROR。
- `out` が `.pdf` で終わる → 1 つの PDF(190 と同じ: 1 事象 1 ページ、しおり `event <eventId>`、2 GiB 超の WARNING)。
- それ以外 → `out` をディレクトリとして PNG。
- ディレクトリ: 無ければ作る(途中の階層も — `gSystem->mkdir(path, kTRUE)`)。既にディレクトリならそのまま使い、同名のファイルは上書きすると INFO を 1 行。同名の**ファイル**があれば ERROR。
- PNG の名前 = `event%07u.png`(eventId を 7 桁ゼロ詰め — 名前順 = 事象順)。
- **検査はすべて、ディレクトリ・ファイルを作る前に**(失敗したら何も残さない)。
- 最後に「書いた枚数・出力先」、進捗は 500 事象ごと(190 と同じ)。

### 3. 画面: キャンバス 1200 × 600、`Divide(3, 2)`(1 枚 400 × 300)
- 上段 U / V / W = 190 のまま(`fillUVW`、`drawMinAdc`、`logz` も上段にだけ効く)。
- 下段 XY / XZ / YZ = 下の 3D 点を電荷で重み付けして `colz`(統計箱なし)。タイトル `event <eventId> (entry <n>)  XY;x [mm];y [mm]`、`…  XZ;x [mm];z [mm]`、`…  YZ;y [mm];z [mm]`。ビン = 1 mm。
- 範囲(map から作る — ジオメトリを焼き込まない): x = U の全ストリップの x = −s_U の範囲 ± 5 mm。y = U × V の全ストリップの組の交点のうち、閉合で予言される s_W が W のストリップ位置の範囲に入るものの y の範囲 ± 5 mm。z = [0, 512 × mmPerCell]。
  (mini では x ≈ ±53 → [−58.25, 58.25]。)

### 4. 3D 点(試作 3 版目、模型を仮定しない)
1 事象ごと、`fillUVW` と同じ「strip ごとに加算 → 先頭 25 cell の平均を引いた」値 v(strip, cell) から:
1. **ヒット**: v > `UVWDRAW_HIT_ADC = 35`(analyzeUVW の `ADC_THRESHOLD` と同じ値)。
2. **クラスタ**(面・cell ごと): ヒットのストリップを位置 s(map の `s_mm`)で並べ、隣どうしの間隔が `2 × pitch + 0.1 mm` 以下なら同じクラスタ(= 1 本抜けまでつなぐ)。pitch = その面の map の相異なる s の隣接差の最小値(load 時に求める)。クラスタの電荷 Q = Σ v、重心 s̄ = Σ v·s / Q。**Q < `UVWDRAW_CLUSTER_Q_MIN = 150` は捨てる。幅の上限は置かない**(読み出し面に平らな部分は 1 cell に線分が丸ごと入る — 3 面の重心は線分の重心の射影なので閉合は成り立つ。上限を置くとそこが消える: 試作 2 版目の失敗)。
3. **閉合の定数 c**(run ごとに 1 回、描く前): tree の先頭 min(300, nTotal) 事象で、3 面とも**クラスタがちょうど 1 つ**の cell の s̄_U + s̄_V + s̄_W を集め、その中央値。件数と値を 1 行出す。1 件も無ければ c = 0 で WARNING。
4. **突き合わせ**(cell ごと): U・V・W のクラスタの全組 (i, j, k) のうち |s̄_U + s̄_V + s̄_W − c| < `UVWDRAW_CLOSURE_MM = 2.0` のものを点にする。
   (x, y) = (1 / 1.5) Σ_v p_v (s̄_v − c / 3)(p_v = makeTracks の PITCH。3 方向が 120° なので Σ p_v p_vᵀ = 1.5 I の最小二乗解)、z = cell × mmPerCell、重み = (Q_U + Q_V + Q_W) / 3。
5. 純関数に分ける(テストが直接呼ぶ): クラスタ化、c の推定、1 cell の突き合わせ、点 → 3 つのヒストへの詰め込み。名前は `uvwdraw` 接頭辞。
6. ホットパスの注意(マクロだが全事象を回す): ヒストは最初に作って `Reset()` で使い回す。

### 5. `macros/README.md`
drawUVW の行を新しい呼び方・出力(PNG ディレクトリ既定 / `.pdf` で PDF)・下段の説明・大きさ(PNG ≈ 46 kB / 事象)に更新。

## テスト(`macros/test_drawUVW.C` を拡張、exit code = 失敗数)
190 の [1]–[10] は新しい呼び方に直して残す(PDF の項目は `out = "...pdf"` で)。追加:
11. **run 定数**: 圧力 ≤ 0 / サンプリング 30 / HV ≤ 0 → ERROR、何も作らない(ディレクトリも)。`mmPerCell` が analyzeUVW.C の `driftVelocityCmUs(1600, 20, 198) / 12.5 × 10` と一致(テストで analyzeUVW.C を `#include` して比べる)。
12. **クラスタ**: 合成 map(pitch 2 mm — 1.5 を焼き込んでいないことを縛る)で、1 本抜け = 1 クラスタ、2 本抜け = 2 クラスタ。重心 = 手計算。Q < 150 は捨てる。v ≤ 35 のサンプルは入らない。
13. **c の推定**: 合成 map の原点をずらして Σ o_v = 4.5 にし、単一クラスタの cell だけから c = 4.5(± 1e-9)。2 クラスタの cell は使わない。
14. **直線 1 本**: 3D の直線を cell 100..139 に置き(各 cell で 3 面とも重心がちょうどストリップ位置に来るように選ぶ)、点が cell ごとに 1 個、(x, y, z) が手計算どおり(± 1e-6)。
15. **ゴースト**: 同じ cell に 2 点(各面 2 クラスタ)→ 8 組のうち本物の 2 組だけ通る(偽の 6 組は閉合が 2 mm より外れる配置を選ぶ)。
16. **平らな線分**: 1 cell に 3D の線分(各面で幅 10 本以上のクラスタ)→ 点 1 個、位置 = 線分の重心の手計算。
17. **範囲**: 合成 map で x / y の範囲が上の定義どおり(手計算)。
18. **出力**: `out` 空 → `<名前>_uvw_events/` が作られ、事象数ぶんの `event%07u.png`、PNG の大きさが 1200 × 600(PNG ヘッダの 16–23 バイト目)。入れ子のパス(`a/b/c`)も作られる。既存ディレクトリは使い回す。同名のファイルがある → ERROR で何も書かない。`.pdf` → PDF のページ数 = 事象数としおり。
19. 検査に落ちる呼び方(範囲外・tree 無し・map 無し・run 定数)で**ディレクトリが作られない**。

## 受け入れ(実データ、ローカルのみ)
- `cd /Users/aogaki/Workspace/tpcdaq-rs/macros && root -l -b -q 'drawUVW.C("data/run0209_raw.root", 198, 12.5, 1600, "/private/tmp/claude-501/-Users-aogaki-Workspace-tpcdaq-rs/c5696e1c-f7d9-4a2e-ab7f-ab99dda8905b/scratchpad/t191/png", 0, 30)'` → 30 枚、1 枚 ≈ 46 kB、所要時間、c の値(試作は 0.011 mm)。
- event 5 と 27 の PNG を目で見て試作 `xyproto3.pdf` と同じ形: 27 = 頂点 ≈ (−8, −30, 42) mm から −z へ ≈ 12 mm の短い 1 本と +x へ ≈ 41 mm の 1 本。5 = −z へ ≈ 5 mm の明るい短い 1 本と、ほぼ平らな ≈ 45 mm の 1 本。PNG のパスを報告に書く。
- 同じく `out = ".../t191/r209_10.pdf"`、10 事象で PDF も出ること。
- 全事象は作らない(主対話が受け入れ後に作る)。

## 禁止・注意
- 触るのは `macros/drawUVW.C`・`macros/test_drawUVW.C`・`macros/README.md` だけ。作業ツリーには他チケットの未コミットの変更がある。コミットしない。
- `macros/data/`・`channel_map.csv` を書き換えない・消さない。`macros/data/run0209_uvw_events.pdf`(190 の全事象 PDF、4.18 GB)にも触れない — 既定の出力先 `data/run0209_uvw_events/` とは別名なので衝突しない。
- ROOT は `export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX27.sdk` を付けて動かす。
- 発注書に無い設計の分岐に出会ったら、実装せずに報告して戻る。

## 報告の形
変更したファイルと行数 / テスト名ごとの PASS・FAIL と実行コマンド / red の証拠 / 受け入れの数値(枚数・kB / 枚・所要時間・c)と PNG のパス / 発注書から外れたところとその理由 / **実行できなかった操作**(無ければ「なし」)。

## 結果
- 実装: implementer(Sonnet、2026-11-02)。主対話が照合: 触ったのは `macros/drawUVW.C`(535 行)・`macros/test_drawUVW.C`(533 行)・`macros/README.md`(drawUVW の行)だけ。核の関数(`uvwdrawClustersAtCell` = 間隔 ≤ 2 × pitch + 0.1 mm でつなぎ Q ≥ 150、入力は `fillUVW` と同じ格子、`uvwdrawClosureC`、`uvwdrawMatchCell`、`uvwdrawRanges`)は発注書どおり。
- テスト: `export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX27.sdk; cd macros && root -l -b -q test_drawUVW.C` → **59 PASS / 0 FAIL(GREEN、exit 0)**。発注書の [1]–[19] がすべてテスト名にある([19] は `[9][19]` の 3 件と [11] の「ディレクトリも作らない」)。主対話が再実行して確認(2026-11-02、Mac / ROOT 6.40.04)。
  - 主な数値: mmPerCell = analyzeUVW.C の `driftVelocityCmUs(1600, 20, 198) / 12.5 × 10` と 1e-12 で一致(0.233794)。c = 4.5 ± 1e-9(単一クラスタの cell だけから)。直線 40 cell → cell ごとに 1 点(± 1e-6)。ゴースト 8 組 → 本物 2 点だけ。平らな線分 → 1 点 (−10.75, 13.1349)。PNG の大きさ 1200 × 600(ヘッダから)。
- red の証拠: 190 版の drawUVW.C で `use of undeclared identifier 'uvwdrawClustersAtCell'` など ≈ 20 個のコンパイルエラー。
- 実データの受け入れ(run 209、entry 0–29): 30 枚、1200 × 600、≈ 41 kB / 枚、3.0 s(c の推定込み)。`c = 0.0172 mm(17,856 cell、先頭 300 事象)`(試作 0.011)。E/p = 0.404 の WARNING(想定どおり)。`event0000005.png`・`event0000027.png` は試作と同じ形(主対話が目視: 27 = 頂点 ≈ (−8, −30, 42) から −z の短い 1 本と +x の ≈ 41 mm、5 = 明るい短い 1 本 + ほぼ平らな ≈ 45 mm)。10 事象の PDF も 566 kB / ページで出る。
- 発注書に無かった判断(受け入れ): ① batch の `TCanvas(…, 1200, 600)` は窓枠を引いて 1196 × 572 になる → `SetCanvasSize(1200, 600)`。② この ROOT の `gSystem->mkdir(相対, kTRUE)` は −1 で何も作らない → 絶対パスにしてから作る `uvwdrawMakeDir`。③ y の範囲に c が要るので `uvwdrawRanges` は c を引数に取る。④ ビン数 = ceil(幅) で ≤ 1 mm。⑤ s が 1 種類しかない面は pitch 0。⑥ U × V の組が 1 つも W の範囲に閉じなければ ERROR。⑦ `uvwdrawLoadMap` に s_mm の引数を追加。
- 実行できなかった操作: なし(implementer の報告)。
- **全事象(主対話、2026-11-02)**: `cd macros && root -l -b -q 'drawUVW.C("data/run0209_raw.root", 198, 12.5, 1600)'` → `macros/data/run0209_uvw_events/` に `event0000000.png` … `event0007945.png` の 7,946 枚、合計 281 MB(≈ 36 kB / 枚)、6 分 28 秒。190 の PDF(4.18 GB、`macros/data/run0209_uvw_events.pdf`)は残してある — 消すかはユーザー次第。
