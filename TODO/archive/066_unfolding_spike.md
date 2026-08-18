# 066 — Unfolding 実現可能性スパイク(検証ハーネス + データ駆動パラメータ拘束の見積もり)

**Status: COMPLETED**(起票 Fable 2026-08-18 / 実施同日。時間箱スパイク — 038 と同じ流儀)

> **Fable 最終裁定(2026-08-18、クローズ時)**:
> ①**データ駆動拘束の本実装はしない** — 拘束できるのは σ_T と σ_L[cell] の 2 つのみで、
> χ²/ndf ≈ 9(モデル不足支配)の下では Δχ²=1 を額面で読めない。**Mikolaj 待ちに徹する**。
> 受け入れ試験は `SIGMA_T=... SIGMA_L=... ./run_all.sh` の形で完成済み。
> ②**ベイズ層は積まない(確定)** — v_drift×σ_L[mm] は弱相関でなく**厳密縮退**であり、
> 事後分布は事前分布の写しにしかならない。ユーザーの当初の問い「ベイズ/総当たりは時間潰しか」
> への実証的回答: 総当たり(χ² グリッド)は縮退の構造を出し切るのに十分で、ベイズは不要。
> ③次に価値が高いのは (a) D_L をガス物性から与えて v_drift を決める較正方針(物理側相談)
> (b) パルサーによる電子回路応答実測 + イオンテールのモデル化(068 ゲイン表と同根ユニット)。
> 起票は 1 フェーズ先ルールに従い保留。
> ④FINDINGS.md はサブエージェントのハーネスガードで書けなかったため**主対話が作成済み**
> (内容は本結果節が正、FINDINGS.md は要約 + 裁定)。
> ⑤ROOT 6.08⇄6.36 の PEventTPC 互換問題は P5 Warsaw「旧 ROOT 互換」の具体機序として
> CURRENT.md に追記。ゲート表記の注: 本ユニットはコード無変更だが、並行の 067 により
> リポ全体ゲートは cargo 468 passed が最新(結果節の「454 無変更」は本ユニット視点)。

## 背景・裁定

- **2026-08-18 ユーザー裁定: ELITPC 100 Hz は本気で後回し**(コラボの誰も可能と思っておらず
  「やりすぎ」。mini 100 events/s = 064 で最低目標は達成済み)。→ ソフト側の次の主軸は
  解析プロダクト系。優先順位は 059 裁定のとおり **Unfolding > ゲイン正規化 > ZS**。
- 062: 応答パラメータは Mikolaj モデル由来で**受領待ち**。本スパイクは受領を待たず、
  (a) 受領後の**受け入れ試験ハーネスを先行構築**し、(b) 実データからの
  **データ駆動パラメータ拘束の実現可能性**を見積もる。ベイズ推論はやらない(判断材料が
  出てから。まず χ² グリッドで縮退の形を見る — KISS)。
- 060: ROOT プロダクトは処理込み可(生データは graw のみ)— unfolding 済み量の出口は
  ROOT 側で良い。ただし本スパイクは出口設計はしない。

## 素材(すべてローカル、リポ持ち込み禁止)

- `reference/exp_data/2026/` — HIgS_2026 実 physics run(4 AsAd .graw 計 14 GB +
  **`PEventTPC_2026-08-11T07-47-37.051_0000.root`** ← ハーネスはここ起点で良い。
  graw 再デコードは不要)。設定は 065 の結果節が正(外部トリガ・フル readout・生 ADC・
  peaking 232 ns、CKW 25 MHz = trigDelay1748 か 12.5 MHz = 3576 のどちらか —
  **着手時に時間幅から同定して記録すること**)。
- `reference/TPCReco/` — **StripResponseCalculator = forward モデル。改変禁止・
  ライブラリ利用のみ**(一次資料はテストダブルではない、の流儀)。
- 実 mini ジオメトリ .dat(メモリ local-real-assets の正しいペア)。

## やること(A → B → C の順、各段で go/no-go)

- **A. イベント選別 + プロファイル抽出**: PEventTPC ROOT から単一クリーントラックを選別
  (multiplicity・連結性の素朴カット)。各イベントで①strip 横断方向の電荷プロファイル
  ②時間方向の幅 vs ドリフト位置、を統計として抽出。
- **B. forward モデル接続**: TPCReco StripResponseCalculator を呼ぶ読み出し専用 C++ ツール。
  仮パラメータで A と同じ統計量を合成できること。
- **C. 拘束マップ**: per-event 直線フィット(track = nuisance)× パラメータグリッド
  (σ_T, σ_L, v_drift, 全体ゲインの 3–4 軸)の χ² で、拘束できる量と縮退する量を分離。

## Exit criteria(この 3 つが出たら終了 — 良否によらず)

1. **ハーネスが 1 コマンドで回る**(選別 → 抽出 → forward 比較 → χ² マップ)。
   これは Mikolaj パラメータ受領時の受け入れ試験にそのまま転用する。
2. **拘束可能量と縮退の一覧**(例: σ_T/シェアリングは拘束可、σ_L×v_drift は比のみ、等)。
3. **裁定材料**: データ駆動拘束を本実装する価値があるか / Mikolaj 待ちに徹するか /
  ベイズ層を積む価値があるか — の Fable 裁定に足る 1 ページ。

## 非スコープ(明記)

- unfolding 本体の実装・オンライン化・TPCReco 改変・ベイズ実装・出口(ROOT ブランチ)設計。
- **パルサーラン解析**(電子回路応答 + strip 毎ゲイン)— **素材は到着済み**
  (2026-08-18: `reference/exp_data/2026/pulser/` 26 run。frameType 1・全 ch 固定長と
  実測確定 — 詳細は 067)。読み側対応(067)が先。解析本体は本スパイクの結果を見て別ユニット。
- ランダムトリガの **pedestal 32 run**(`exp_data/2026/pedestal/`)も到着済み —
  ベースライン/ノイズの per-channel 統計は A の選別カットの土台に使ってよい。

## 置き場・モデル・時間箱

- コードは `reference/_spike/unfold/`(038 と同じ扱い — リポ本体を汚さない。
  ROOT/TPCReco 依存は将来も tools/ か _spike に閉じ込める)。
- 実装は Opus(工学判断が残る)。A の選別カットと C の縮退の読み・最終裁定は Fable。
- 時間箱: 実働 1–2 日相当。C が重すぎたら B までで一旦報告(それでも exit 1 は満たせる)。

## 結果

**A / B / C すべて実施(打ち切りなし)。裁定材料 = `reference/_spike/unfold/FINDINGS.md`。**

### 実行環境・日付

- 2026-08-18、macOS(Darwin 25.5.0 / arm64)、ROOT **6.36.10**(`/opt/ROOT`)、Apple clang C++17。
- 作業場所 `reference/_spike/unfold/`(.gitignore 済み)。**リポ本体は 1 バイトも変更していない**
  (`TODO/066` のこの節と `FINDINGS.md` 以外に書き込みなし。TPCReco も無改変)。
- 素材: `reference/exp_data/2026/PEventTPC_2026-08-11T07-47-37.051_0000.root`(3,852 events)/
  `reference/TPCReco/TPCReco-HIGS2026_online/resources/geometry_ELITPC.dat`。

### 実行コマンド

```sh
cd reference/_spike/unfold
./run_all.sh            # A → B → A/B 比較 → C → 閉包テスト → 縮退デモ(全部で ~10 分)
FAST=1 ./run_all.sh     # 先頭 300 イベントの煙テスト(数十秒)
SIGMA_T=... SIGMA_L=... ./run_all.sh   # ← Mikolaj 受領値での受け入れ試験はこの形
```

個別(run_all.sh が呼ぶもの):

```sh
./extract <PEventTPC.root> <geometry.dat> out_A.root \
    --min-strips 8 --max-strips 150 --max-resid 3 --max-trms 12 --dump-max 300
./forward out_A.root <geometry.dat> out_B.root --sigmaT 0.90 --sigmaL 1.74 --peaking 232 --noise 12.2
root -l -b -q 'compare_ab.C("out_A.root","out_B.root")'
./chi2map out_A.root <geometry.dat> out_C.root --events 40 --nT 13 --nL 13 \
    --sT-lo 0.2 --sT-hi 2.6 --sL-lo 1.0 --sL-hi 13.0
./xcheck_compare tpcreco_probe/xcheck.log 1.0 1.0 0.2897   # TPCReco 本家との数値照合
```

### ジオメトリの同定(誤ったジオメトリで進めない、の宿題)

データ側のキーが **U(section 1,2)最大 132 / V(0,1,2)最大 225 / W(0,1,2)最大 226、
合計 1,018 strip × 502 cell = 511,036 キー/event** で `geometry_ELITPC.dat` と完全一致。
mini eTPC の .dat(256 strip・section 0 のみ)ではあり得ない。→ **ELITPC 系 .dat で確定**。
ELITPC 系 .dat 群はチャンネル対応が全て同一で、差分は `DRIFT VELOCITY` /
`SAMPLING RATE` / `TRIGGER DELAY` の 3 行のみ(実測 diff)—— これが C の縮退の話に直結する。

### A: 選別と抽出(実測値)

- **3,852 events → 334 events(8.67 %)** が「U/V/W すべてで連結成分ちょうど 1 つ + 直線 + 細い」。
  落ちた内訳: multiplicity 582 / strip 幅 2,227 / 直線フィット 698 / 太さ 11。
- ノイズ(pedestal 減算後)per-strip σ の中央値 **12.2 ADC**、per-strip ピーク電荷 **≈ 718 ADC**。
- per-strip 時間 RMS **7.47 ± 1.53 cells**(平坦トラック |slope|<0.3 に限れば **5.54 ± 0.87**)。
- **時間幅の分解**(32,734 strip の 3 パラメータ最小二乗):
  `RMS_t² = 45.37 + 0.2084·slope² + 0.04611·t_mean [cell²]`
  - 切片 45.37 → σ_L = **6.04 cells**(25 MHz の電子回路 RMS 2.99 cell を差し引いた値。
    12.5 MHz 仮定なら 6.57 cells)。C の χ² の 6.0 cells と独立に一致。
  - slope² 係数 0.2084 → 1 strip 内の幾何項 1/12 を引くと **σ_T = 0.354 strips = 0.53 mm**
    (C の横断 χ² は 0.60 strips = 0.90 mm。**推定器間で 2 倍弱ばらつく** = モデル不足の目安)。
  - **ドリフト依存 +0.0461 cell²/cell** を実測(観測範囲 475 cell で +21.9 cell² = 切片の約 48 %)。
    → 拡散の伸びは見えている。ここから `v_drift/f_sample = 2·D_L / 0.0461` なので、
    **D_L をガスから与えれば v_drift が決まる**(§C の縮退を破る唯一の実務的な道)。
- 横断プロファイルは Gaussian より裾が重い(±4〜5 strip で model/data ≈ 0.5〜0.7)。

### B: forward モデル接続(**TPCReco 直リンクに成功**、再実装は照合済み)

- **TPCReco `StripResponseCalculator` は無改変でリンクできる**ことを実測(macOS/arm64・ROOT 6.36)。
  必要な .cpp は 11 本(`StripResponseCalculator` + `GeometryTPC` / `StripTPC` / `RunConditions` /
  `GeometryStats` / `PEventTPC` / `EventInfo` / `UtilsMath` / `CommonDefinitions` / `RunIdParser` /
  `CoBoClock`)、**rootcling 辞書は不要**。初期化 2.2 s、`addCharge` 0.072 ms。
  → 手順とログは `reference/_spike/unfold/tpcreco_probe/`。
- ただし C の χ² グリッド(数万回のモデル評価)には遅すぎるので、**同じ式の 1D 射影版を解析的に
  再実装**(`response.hpp`。出典は行番号つきでヘッダに明記):
  横 = `Reconstruction/src/StripResponseCalculator.cpp:703-707`(等方 2D ガウス。**strip は平行帯
  なので pitch 軸への射影 + 帯積分 = erf 差分**で本家の MC 積分と同値)/
  縦 = 同 `:836-847`(本家も erf 差分)/ 電子回路 = 同 `:889-895`(AGET シェーパ、peaking 232 ns)。
- **本家との数値照合**(点電荷 1 個、σ_xy = σ_z = 1.0 mm、peaking 0):
  - **時間プロファイル: 全 30 cell で比 0.999〜1.000、食い違いの総和 1e-4**(実質同一)。
  - **横断プロファイル: 食い違いの総和 0.041**(コア 1〜3 %、翼 16〜19 %。本家は TH2Poly の
    pad 形状に対する MC 積分 = 既定 10,000 点で、翼ビンの MC 誤差だけで 4 % 級)。
  - 本家は電荷 Q を **U/V/W の 3 方向に約 1/3 ずつ配る**(3 方向の合計が Q。実測 QTOT ≈ 333,860 × 3
    ≈ 1e6)。**我々の χ² は振幅を全レベルで nuisance にしているので、この規格化の違いは結果に影響しない。**
- 仮パラメータ(σ_T 0.90 mm / σ_L 1.74 mm / peaking 232 ns / ノイズ 12.2)で A と同じ統計量を合成:
  per-strip 時間 RMS data 7.47 / model 7.20、積み上げ時間プロファイルは中心 ±10 cell で ±10 % 一致。
  **合わない点**: ①データはピーク後 +15〜25 cell に長い尾を持つがモデルには無い(イオンテール未モデル化)
  ②傾き依存がデータの方が強い。

### C: 拘束マップ(40 events × 3 方向 = 120 画像、13×13 格子、~14 s)

| 量 | 拘束 | 実測 |
|---|---|---|
| σ_T [mm] | **可** | 0.600 strips = **0.90 mm**、Δχ²=1 で [0.40, 0.80] strips = [0.60, 1.20] mm |
| σ_L [cell] | **可** | **6.0 cells**、Δχ²=1 で [5.0, 7.0] |
| σ_L [mm] | **不可(v_drift と厳密縮退)** | v_drift = 0.7242 / 0.4931 / 0.390 cm/µs の .dat で **χ² が 6.507e6 で完全同一**、σ_L[cells] も同一、σ_L[mm] だけが 1.74 / 1.18 / 0.94 とスケール |
| v_drift | **不可**(同上) | トラックは (strip, cell) 空間の直線 —— v_drift はどこにも現れない |
| 全体ゲイン | **不可** | 入射エネルギー未知 → 振幅は必ず nuisance。χ²_T/χ²_L/χ²_G のいずれにも現れない |
| peaking time | 弱い | 70 / **232** / 502 ns で χ² = 6.532 / 6.507 / 6.601 e6(差 1.4 %)、σ_L が 6.5 / 6.0 / 2.5 cells と入れ替わって補償。**1014 ns は明確に棄却**(1.65e7) |

- **閉包テスト(推定器の健全性)**: 既知 σ_T = 0.667 strips / σ_L = 3.452 cells で合成 →
  **回収 σ_T = 0.700 [0.600, 0.800](3 種の χ² すべて)/ σ_L = 3.000 [2.333, 3.667](χ²_T)・
  3.667 [3.000, 4.333](χ²_L)**、χ²/ndf = 1.90(χ²_L)〜2.20(χ²_T)。
  **どちらも格子刻み(0.1 strip / 0.67 cell)以内で真値を回収** = バイアスなし。
- **実データでは χ²/ndf = 8.7〜11**。→ **統計誤差ではなくモデル不足が支配的**。
  Δχ²=1 の区間は「モデルが正しければ」の値であり、額面どおりには読めない。
- 途中で自己摘発したバグ 2 件(どちらも閉包テストが赤にした):
  ①モデルの重心シフト(電子回路応答で +7.7 cell)を外さずに ±4 cell の平行移動だけで
  合わせようとして σ_L が過大に出た。②χ² の和を「モデルが非ゼロの画素」だけで取っていたため、
  **足す画素数がパラメータに依存**して「σ が小さいほど χ² が下がる」偽の勾配が出た(実データで
  σ_T が下端に張り付いた)。→ 画素集合を固定して解消。

### CKW 周波数の同定(発注書の宿題)

**時間幅からの決定的な同定はできなかった。** AGET シェーパ(peaking 232 ns)は
ピーク 270.6 ns / RMS **119.6 ns** / FWHM 293.4 ns = 25 MHz なら 3.0 / 7.3 cells、12.5 MHz なら
1.5 / 3.7 cells。観測幅 5.5〜7.5 cells に対し電子回路の寄与はどちらでも小さく、σ_L が差を吸収する
(χ²_L = 1.208e7(25 MHz)vs 1.226e7(12.5 MHz)= **25 MHz を 1.5 % だけ選好**)。
→ **一次資料(065 の live `workspace.xcfg` = `…_25MHz_232ns_trigDelay1748.xcfg`)を正として 25 MHz を採用**。
本スパイクの χ² も弱く同じ向きで、矛盾はない。

### 付随して見つかった事実(本題外だが Fable 判断が要る)

1. **実機 PEventTPC ROOT が我々の ROOT 6.36 では素直に読めない**。当該ファイルは
   **ROOT 6.08.06 が書いた**(`TFile::GetVersion` = 1060806)。chargeMap のキー
   `std::tuple<int,int,int,int>` の**メンバのメモリ順が libstdc++(`_3,_2,_1,_0`)と libc++(`_0.._3`)で逆**で
   checksum が食い違い、ROOT がファイル側 StreamerInfo を紐付けられず、
   `Could not find the StreamerInfo with a checksum of 0xba6edd70` を出して**壊れた値を返す**。
   本スパイクは `pevent_read.hpp` で StreamerInfo を貼り替えて回避(`BuildOld()` が名前で照合。
   貼り替え後は chargeMap サイズが 511,036 = 1,018 strip × 502 cell と一致)。
   **逆向き(我々が ROOT 6.36 で書いた PEventTPC を先方の 6.08 で読む)も同じ理由で危ない** ——
   SPEC §6.4 のオフライン互換に関わる。**要裁定**。
2. **TPCReco `GeometryTPC::LoadAnalog` にスタックバッファ 1 バイト overflow**
   (`DataFormats/src/GeometryTPC.cpp:508`、`char name[12]` に `%12s` = NUL 込み 13 バイト書く)。
   macOS の hardened libmalloc では 5 回に 4 回 abort する(Linux/glibc では顕在化していない)。
   TPCReco をライブラリとして使う計画があるならガードが要る。**本スパイクでは TPCReco を一切改変していない。**

### スキップしたもの

- リポ本体の `cargo test` / C++ テストは**対象外**(コード変更なし。ゲートは 064 時点の
  cargo 454 passed から無変更)。
- unfolding 本体・オンライン化・ベイズ・出口設計は非スコープ(発注書どおり未着手)。
