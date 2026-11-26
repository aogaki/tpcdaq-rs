# 213 — 中性子の反応らしい「良いイベント」のリスト(run 209 = PuBe、run 220 = N_Wall)と graw の位置

**Status: IN PROGRESS — 一次のリストあり、人の目での確認待ち(2026-11-26)**(2026-11-25 起票 Opus。研究者からの要望をユーザーが伝達)

## 要望(ユーザー 2026-11-25)
- PuBe のデータ(run 209)と N_Wall のときのデータ(run 220)で、**どの graw ファイルのどのイベントが見た感じ良いか**のリスト。
- 見たいのは**中性子による反応** — **α 1 本はあまり重要ではない**(壁から来る α・単独の長い α は外す)。

## 候補の母集団
- **run 209**(PuBe、502 ns、`macros/data/run0209_raw.root`、絵 `run0209_uvw_events/`): ユーザーの目の 16 本(5, 27, 31, 61, 91, 93, 102, 185, 199, 243, 244, 250, 264, 275, 279, 284 — 244 は 3α)、目隠し試験で選ばれた 11 本(archive/200)、内側で止まる ≥ 25 mm の 86 本(archive/197)、2 本の当てはめで hyp = 1 の事象(archive/198 — scratchpad の出力は消えたので要るなら fitProngs を回し直す)。
- **run 220**(N_Wall、232 ns、`run0220_signal_raw.root` ほか、絵 `run0220_{contained,blob,signal}_uvw_events/`): ノイズを除いた contained 228 本、液シンの高い時間帯の blob(211 — 中性子の強さに比例することを確認済み)。
- 分類のラベル(案): `alpha+recoil`(頂点から 2 本)・`3alpha`(3 本以上)・`recoil`(短い塊)・`contained-alpha`(内側で生まれて止まる α 1 本 — (n,α) だが反跳が見えない)・`unclear`。

## 作るもの
1. 一覧(サムネイルの一覧画像): 候補の PNG を 1 枚に 20 事象ずつ並べる(ROOT の TASImage で — ImageMagick は Mac に無い)。人(ユーザー / 研究者)と主対話が速く目で選べるように。
2. 目で選んだ事象の**テキストの表**(212 と同じくテキスト): `run, graw_file, eventIdx, raw_entry, time_eet, label, note, png`。**位置は graw のファイル名 + eventIdx で足りる**(ユーザー 2026-11-25「event index だけで足りる。バイト位置だとどのイベントか分かりづらい」)。
3. **graw の位置**: 今の全 ch の読み出しではフレームが一定の大きさ(run 220 = 278,784 B、1 チャンク 1,073,875,968 B = 3,852 事象)なので、`graw_file = _{floor(i / 3852):04d}`・`index_in_file = i mod 3852` で計算できる。run 209 のフレームの大きさも確かめる。**ELITPC につながったら graw のヘッダを読んで答え合わせ**(rateVsTime.C の流儀、ヘッダだけ)。

## 未決
- 研究者の「良い」の基準(上のラベルでよいか)、リストの本数の目安。
- ~~graw の中の位置~~ → eventIdx で足りる(ユーザー 2026-11-25)。

## 方針(ユーザー 2026-11-25)
- 「raw.root をもとにやるのは分かるし自分もそうしたい。ただ研究者は **graw ファイルでやっている**ので、**ELITPC にアクセスできるようになったら graw を読んでやる**。**どのファイルにあるかも重要**」→ リストの位置(graw のファイル名 + eventIdx)は、計算(1 チャンク 3,852 事象)ではなく **ELITPC で graw のフレームヘッダを読んで**出す(rateVsTime.C の流儀 — ファイルごとに eventIdx を列挙)。raw.root の entry は照合用に残すだけ。
- 済み(目で選ぶ材料、2026-11-25): サムネイルの一覧 `macros/data/sheets/`(gitignore)— `run0220_contained_NN.png`(12 枚 = 228 事象)、`run0209_eye_blind_NN.png`(2 枚 = 目の 16 + 目隠しの 11)、`run0209_contained_NN.png`(13 枚 = selectNeutron の contained、`run0220_ana/sel209.root`)。マクロ = `macros/data/run0220_ana/sheets.C`(4 × 5、1 枚 20 事象、題名 = event 番号 + 時刻)。絵の中身は raw.root から描いたもの(データは graw と同じ)。
- ELITPC で: ① run 209 / 220 の graw のヘッダを読み、ファイルごとの eventIdx の範囲と一覧を作る ② 目で選んだ事象に graw のファイル名 + eventIdx を付ける ③ テキストの表にする。

## 一次のリスト(2026-11-26、ELITPC につながった — ユーザー「つながります。進めてください」)
- **graw の対応表**: ELITPC で `~/test/graw_index/grawIndex.C`(フレームヘッダだけ、`nice -n 19 ionice -c3`、run 221 の取得中)— 全データフレームの graw ファイル名・ファイル内の順番・**eventIdx**(ヘッダのバイト 22–25)・eventTime・cobo・asad。run 209 = graw 3 本・7,946 フレーム(eventIdx 0–7,945 連続、3,852 / 3,852 / 242)、run 220 = graw 47 本・177,735 フレーム(eventIdx 0–177,734 連続、各 3,852・最後の _0046 は 543)。どちらも eventIdx = raw.root の eventId = entry。写し: `macros/data/sheets/graw_index_run0209.tsv`・`graw_index_run0220.tsv`(gitignore)。
- **機械の前選別は不成功**: uvw の strip ヒットで「主軸から 2.5 mm 以上の電荷の割合」(`run0220_ana/topo.C`)を取ったが、analyzeUVW は strip ごとにヒット 1 つ(ピーク)・主軸から ± 10 mm の帯の外を捨てるので、枝は uvw の段階でほぼ消えている。最大の面で並べると W 面のノイズの縞が上位を占め、2 番目の面で並べるとほぼ全部 0。→ 目で見る。
- **目での一次の分類**(主対話、サムネイルの一覧): 長さ ≥ 15 mm の候補だけを長さの順に(`sheets/run0220_long_NN.png` 6 枚 = 103 事象、`run0209_long_NN.png` 10 枚 = 184 事象のうち 01–08 を見た — 09–10 = 15–17 mm は塊にしか見えないので見送り)+ run 209 の目と目隠しの 2 枚。ラベル: **A** = 2 面以上で頂点から 2 本(V・L・折れ)がはっきり、**B** = 折れ / 分かれ目が 1 面だけ・弱い、**C** = 小さな塊が複数(3α らしい)、**contained-alpha** = 両端が内側の太い 1 本、**short** = 5–15 mm の塊(run 220 の contained の大部分、形は見分けられない)。細く淡い長いトラック(run 220 で ≈ 30 本、突き抜け)と、単独の長い α は外した。
  - run 209: **A 18**(27・5・951・4432・990・3067・4282・7766・635・994・3207・7290・4957・1342・1022・4830・5344・279)、**B 28**、**C 8**(244 = ユーザーの 3α を含む)。
  - run 220: **A 4**(1154・2181・171854・174579)、**B 7**、contained-alpha 10、short 125。
- **成果物**(`macros/data/sheets/`、gitignore — 研究者に渡す): **`good_events_run0209.tsv`・`good_events_run0220.tsv`**(テキスト、`#` の行に説明。列 = run・**graw_file**・**eventIdx**・time_local・label・note・png)、ラベルの元 `labels_firstpass.txt`、A・B・C だけの一覧 `good_run0209_NN.png`(3 枚)・`good_run0220_01.png`。
- 未: ユーザー / 研究者が一覧で確認して A / B / C を確定。研究者への渡し方(ELITPC の置き場)。run 220 の blob(中性子の強い時間帯)を入れるか。
