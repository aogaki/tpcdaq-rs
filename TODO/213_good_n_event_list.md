# 213 — 中性子の反応らしい「良いイベント」のリスト(run 209 = PuBe、run 220 = N_Wall)と graw の位置

**Status: TODO**(2026-11-25 起票 Opus。研究者からの要望をユーザーが伝達)

## 要望(ユーザー 2026-11-25)
- PuBe のデータ(run 209)と N_Wall のときのデータ(run 220)で、**どの graw ファイルのどのイベントが見た感じ良いか**のリスト。
- 見たいのは**中性子による反応** — **α 1 本はあまり重要ではない**(壁から来る α・単独の長い α は外す)。

## 候補の母集団
- **run 209**(PuBe、502 ns、`macros/data/run0209_raw.root`、絵 `run0209_uvw_events/`): ユーザーの目の 16 本(5, 27, 31, 61, 91, 93, 102, 185, 199, 243, 244, 250, 264, 275, 279, 284 — 244 は 3α)、目隠し試験で選ばれた 11 本(archive/200)、内側で止まる ≥ 25 mm の 86 本(archive/197)、2 本の当てはめで hyp = 1 の事象(archive/198 — scratchpad の出力は消えたので要るなら fitProngs を回し直す)。
- **run 220**(N_Wall、232 ns、`run0220_signal_raw.root` ほか、絵 `run0220_{contained,blob,signal}_uvw_events/`): ノイズを除いた contained 228 本、液シンの高い時間帯の blob(211 — 中性子の強さに比例することを確認済み)。
- 分類のラベル(案): `alpha+recoil`(頂点から 2 本)・`3alpha`(3 本以上)・`recoil`(短い塊)・`contained-alpha`(内側で生まれて止まる α 1 本 — (n,α) だが反跳が見えない)・`unclear`。

## 作るもの
1. 一覧(サムネイルの一覧画像): 候補の PNG を 1 枚に 20 事象ずつ並べる(ROOT の TASImage で — ImageMagick は Mac に無い)。人(ユーザー / 研究者)と主対話が速く目で選べるように。
2. 目で選んだ事象の**テキストの表**(212 と同じくテキスト): `run, graw_file, index_in_file, eventIdx, raw_entry, time_eet, label, note, png`。
3. **graw の位置**: 今の全 ch の読み出しではフレームが一定の大きさ(run 220 = 278,784 B、1 チャンク 1,073,875,968 B = 3,852 事象)なので、`graw_file = _{floor(i / 3852):04d}`・`index_in_file = i mod 3852` で計算できる。run 209 のフレームの大きさも確かめる。**ELITPC につながったら graw のヘッダを読んで答え合わせ**(rateVsTime.C の流儀、ヘッダだけ)。

## 未決
- 研究者の「良い」の基準(上のラベルでよいか)、リストの本数の目安。
- graw の中の位置は eventIdx で足りるか、バイト位置まで要るか。
