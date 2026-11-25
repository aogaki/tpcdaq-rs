# 213 — 中性子の反応らしい「良いイベント」のリスト(run 209 = PuBe、run 220 = N_Wall)と graw の位置

**Status: WAITING(ELITPC に入れたら graw を読んで作る)**(2026-11-25 起票 Opus。研究者からの要望をユーザーが伝達)

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
