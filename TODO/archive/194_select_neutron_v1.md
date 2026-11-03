# 194 — 暫定選別 v1(`selectNeutron.C`): 頂点が内側 + 電荷の並び、理由コードつき

**Status: COMPLETED(受け入れ条件の 1 つ = 208 の vertex-in ≤ 5 /h は未達 — 単純な規則では分けられないことを記録して閉じる)**(2026-11-02 起票 Opus、同日。実装 = implementer / Sonnet)

## 動機
173 の「両端とも内側」は、ユーザーが目で選んだ 16 本のうち 2 本しか残さない(α は最長 ≈ 13 cm、有感 ≈ 10.6 cm)。線源由来の超過は「片端だけ内側」で x の壁から出ていく飛跡にある(173 §4 末尾: 208 → 209 で x の壁 27.6 → 32.8 / h、y の壁 22.5 → 22.5 / h)。背景の本体(≈ 50 / h)は壁から入って中で止まる α。**当てはめができるまでの暫定の選別**を、どちらの端が頂点かを電荷の並びで決めて作る。効率は **run 208(線源なし)を背景の物差し**にして測る。

## 入力(すべてローカル、`macros/data/`)
- run 209: `run0209_tracks.root`・`run0209_uvw.root`(7,946 事象、67.2 h)。
- run 208: `run0208_tracks.root`・`run0208_uvw.root`(≈ 1,826 事象、≈ 20.4 h — 時間は raw の eventTime から出す: `run0208_raw.root`・`run0209_raw.root`、tree `raw`、`eventTime` は 10 ns 刻み)。
- tracks: makeTracks.C の出力(`ok`、`xStartF`…`zEndF`、`lengthF`、`dirX/Y/Z`、`charge`、TParameter `z_window_mm`)。uvw: analyzeUVW.C の出力(strip ヒット `slView`/`slZ`/`slS`/`slQ`)。

## 作るもの: `macros/selectNeutron.C` + `macros/test_selectNeutron.C`
```
void selectNeutron(const char *tracksFile, const char *uvwFile, const char *rawFile, const char *outFile)
```
出力 = ROOT ファイル、tree `sel`(1 事象 1 行): `eventId`、`entry`、`code`(下の理由コード、int)、`q_in`・`q_mid`・`q_wall`(ADC / mm)、`exitWall`(0 なし / 1 x / 2 y / 3 z)、`alongX`(|Δx| > 2|Δy|)、`lengthF`。最後に理由コードごとの件数と率(/ h、raw の eventTime の最初と最後の差で割る)を表で出す。
- 内側の定義は contained.C と同じ: x ∈ (−53 + 10, 53 − 10)、y ∈ (−58 + 10, 48 − 10)、z ∈ (10, z_window − 10)(定数は contained.C と同じ値で写す。コメントで出どころを書く)。
- **理由コード**(上から順に判定):
  1. `blob`: `ok = 0` で、uvw のヒットの広がりが 3 面とも s・z とも ≤ 10 mm、かつヒットあり(compact.C の「短い事象」と同じ定義)。
  2. `reject`: `ok = 0`(blob 以外)、または両端とも外。
  3. `contained`: 両端とも内側。
  4. 片端だけ内側 → 電荷の並びを取る: 直線(始点 S = 内側の端、方向 d = 内側 → 外側の単位ベクトル、長さ L = lengthF)に沿って、uvw のヒットを面ごとに直線上の位置へ射影する。面 v のピッチ方向 p_v(makeTracks.C の `PITCH`)と d の xy 成分 d_xy について a_v = p_v · d_xy、|a_v| ≥ 0.3 の面だけ使い、ヒットの位置 l = (s − p_v · S_xy) / a_v を xy の道のり → 3D の道のり l₃ = l / |d_xy|(|d_xy| ≥ 0.3 のとき。それ未満の急な飛跡は `vertex-in` に送り、q は −1)。直線の内側から 5 mm(l₃ ∈ [0, 5))の電荷の和 / 5 / 使った面の数 = `q_in`、外側の端から 5 mm = `q_wall`、中央 5 mm = `q_mid`。
     - `q_wall < q_mid < q_in` かつ q_in が 3 つの最大 → `wall-alpha`(壁から入って中で止まる α)。
     - それ以外 → `vertex-in-exit-x` / `-y` / `-z`(外側の端がどの壁の外か。x を先に見る)。
- **x 沿いの向きでは切らない**(`alongX` は変数として残すだけ — 測りたい結果)。
- `multi`(1 cell に複数クラスタ)の印はこの版では作らない(198 で)。

## テスト(`test_selectNeutron.C`、exit code = 失敗数、`tools/scan/test_uvw_closure.C` の流儀で合成 tracks・合成 uvw・合成 raw を一時ディレクトリに書く)
1. 理由コードが 1 つずつ出る合成事象: blob、reject(両端外)、reject(ok = 0 で広い)、contained、wall-alpha(壁側から内へ電荷が単調に増える)、vertex-in-exit-x(内側の端に明るい塊、中央は低い)、vertex-in-exit-y、急な飛跡(|d_xy| < 0.3 → vertex-in、q = −1)。
2. q_in / q_mid / q_wall が手計算どおり(ヒットを既知の位置・電荷で置く。|a_v| < 0.3 の面は使わない)。
3. 率: 合成 raw の eventTime の差 = 3,600 s → 件数 = 率。
4. 境界: 内側の定義の境界の値(x = 43.0 は外、42.9 は内 等)。

## 受け入れ(実データ)
- run 208 と run 209 を分類し、**理由コードごとの率(/ h)を並べた表**と、超過(209 − 208)・Poisson 誤差。**208 の vertex-in(x・y・z の和)が ≤ 5 / h** であること(背景を落とせているかの目安。超えたら規則の見直しを報告)。
- ユーザーが目で選んだ 16 本(run 209 の entry 5, 27, 31, 61, 91, 93, 102, 185, 199, 243, 244, 250, 264, 275, 279, 284)それぞれの理由コードと q_in / q_mid / q_wall の表(健全性の確認だけ — 16 本は正解ではない)。
- 結果は報告に書く(主対話がチケットに転記する)。

## 禁止・注意
- 触るのは `macros/selectNeutron.C`・`macros/test_selectNeutron.C` だけ。**`macros/README.md` は触らない**(同時に別のチケットが編集中 — 主対話が後で 1 行足す)。`macros/drawUVW.C` も触らない・include しない(193 で編集中)。コミットしない。
- `macros/data/` は読むだけ。出力は `/private/tmp/claude-501/-Users-aogaki-Workspace-tpcdaq-rs/c5696e1c-f7d9-4a2e-ab7f-ab99dda8905b/scratchpad/t194/`(mkdir -p してよい)。
- ROOT は `export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX27.sdk` を付けて動かす。
- 発注書に無い設計の分岐に出会ったら、実装せずに報告して戻る。

## 報告の形
変更したファイルと行数 / テスト名ごとの PASS・FAIL と実行コマンド / red の証拠 / 受け入れの表(208・209 の率と超過、16 本の表)/ 発注書から外れたところとその理由 / **実行できなかった操作**(無ければ「なし」)。

## 結果(2026-11-02)
- 実装: implementer(Sonnet)。新規 `macros/selectNeutron.C`(247 行)・`macros/test_selectNeutron.C`(279 行)、ほかのファイルは無変更(主対話が `git status` で確認。ACLiC の生成物 `selectNeutron_C.*` は未追跡のまま — コミットしない)。
- テスト: `export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX27.sdk; root -l -b -q macros/test_selectNeutron.C`(リポの根から)→ **0 failure、rc = 0、81 件の ok**。合成 19 事象(理由コード 8 種 + 内側の境界 11 例)、q の手計算(wall-alpha 30 / 12 / 4、exit-x 40 / 2 / 10、exit-y 30 / 1 / 5、急な飛跡は −1)、率(eventTime の差 3,600 s → 件数 = 率)。red: マクロが無い状態で `FAIL output file has tree 'sel'`、rc = 2。
- 実データ(`…/scratchpad/t194/acc.C`、`sel208.root`・`sel209.root`): run 208 = 2,257 事象 / 20.05 h、run 209 = 7,946 / 67.22 h(発注書の 1,826 は古い見積り)。
  | 理由コード | 208 /h | 209 /h | 超過 ± 誤差 |
  |---|---|---|---|
  | blob | 0.85 | 4.08 | **3.23 ± 0.32** |
  | reject | 60.50 | 54.59 | −5.90 ± 1.96 |
  | contained | 0.10 | 4.11 | **4.01 ± 0.26** |
  | wall-alpha | 2.64 | 3.42 | 0.78 ± 0.43 |
  | vertex-in-exit-x | 26.58 | 30.73 | 4.15 ± 1.34 |
  | vertex-in-exit-y | 21.85 | 21.17 | −0.68 ± 1.19 |
  | vertex-in-exit-z | 0.05 | 0.10 | 0.05 ± 0.06 |
  - **受け入れ条件「208 の vertex-in ≤ 5 / h」は未達(48.5 / h)**。規則は変えずに報告(発注書どおり)。
  - ユーザーの 16 本: contained 2(27・93)、blob 1(244)、vertex-in 6(5・243・279 = y、31・185・250・275 = x のうち 5・31・185・243・250・275・279 の 7)、reject 7(61・91・102・199・264・284 — 両端外 or 片端の判定で外)。wall-alpha は 0。
- **主対話の追加調査(ad hoc、`…/scratchpad/noise189/profpeak.C`・`profshape.C`・`profside.C`)— 規則が効かない理由と、そもそも単純な規則で分けられないこと**:
  1. 端の 5 mm の窓は**拡散のにじみ**を見ている: 片端だけ内側のトラックで、3 mm 刻みの電荷の分布の最大は内側の端から **12–20 mm**(208 で 63 %)。面ごとのストリップ位置をトラックの向きへ射影すると、にじみ(≈ 4 mm)が 1 / |a_v|(最大 ≈ 3 倍)に伸び、makeTracks の `…F` の端点もその分外に出る。
  2. 分布の形を 3 分割で分類(内 > 中 > 外 = 下がる = 壁から入って止まる α 型 / 内 > 中 < 外 = 高・低・高 / 上がる / 中が最大): 208 → 209 の率 /h = 下がる 28.0 → 30.0(+2.0 ± 1.4、うち x の壁へ 15.7 → 18.0)、高・低・高 2.7 → 3.3(+0.6 ± 0.4)、上がる 1.8 → 1.8、中が最大 18.5 → 19.3(+0.8 ± 1.1)。**形の内訳は 208 と 209 で同じ** = 超過は特定の形に集まらない。
  3. x の壁へ出るものを上流(−x)/ 下流(+x)に分けても超過は同程度(+2.2 / +1.7 /h、各 ± ≈ 1)、1 mm あたりの電荷の中央値も同じ(7.4–8.0 k)。陽子らしい暗い成分は見えない。
  → **片端だけ内側の組では、線源由来 ≈ 3.5 /h が背景 ≈ 50 /h(壁からの α)と、電荷の並び・出る側・q/mm のどれでも区別できない**(S/B ≈ 0.07)。きれいに取れる組は contained(+4.0 /h、背景 0.1)と blob(+3.2 /h、背景 0.85)。
  - 目の偽陽性率の試験(Fable 案: 208・209 を混ぜて番号を伏せる)の意味が大きくなった: 16 本のうち 12 本は片端だけ内側の組から選ばれている。
- 次: 選別 v2 は Fable キューへ(当てはめ 197–198 の χ² と仮説選択を待つか、別の観測量 — 端の幅 = 拡散からの z、など — を試すか)。`wall-alpha` の 5 mm の窓の規則は「端の 5 mm」ではなく「分布の最大の位置 + 3 分割の形」に直す価値はあるが、上の 2 のとおり背景は落ちても超過も同じ割合で落ちる。
- 実行できなかった操作: なし(implementer の報告)。
