# 190 — 全事象の UVW を 1 つの PDF に(web UI のイベント表示と同じ描き方)

**Status: COMPLETED**(2026-11-02 起票 Opus、同日完了。実装 = implementer / Sonnet)

## 動機
ユーザー(2026-11-02、run 209 = PuBe の解析の手順 1): 「1 つのキャンバスに UVW それぞれをプロットした横長の絵を作るマクロ。アルゴリズムは現在の web UI の UVW プロットと同じで。キャンバス 1200 × 300、1 つの絵は 400 × 300。内部で止まったものだけでなく全てを 1 つの PDF にしてページを変えながら 1 つずつ見たい。ヒストグラムのタイトルにイベント番号」。「4.4 GB でも問題ない」(= 既定は UI と同一の描き方で、ファイルの大きさは気にしない)。173 §4 の 86 本の目視分類の道具。

## 調べ済みの事実(主対話 2026-11-02)
- web UI の描き方:
  - `src/monitor/display.rs` の `accumulate_uvw`: (cobo, asad, aget, raw ch) → ジオメトリで `Strip { plane, strip }` に引き、`grid[plane][(strip−1)·512 + bucket] += adc`(**生 ADC、ペデスタルを引かない**)。同じ strip に複数 ch(セクション)があれば加算し、u16 で clamp(0..65535)。FPN / Aux / Unmapped は積まない。面ごとのストリップ数 = ジオメトリの最大 strip 番号。
  - `ui/src/app/monitor/root-histo.ts` の `buildUvw` / `stripBaseline`: strip ごとのベースライン = **先頭 25 cell(0..24)の平均**(`BASELINE_CELLS = 25`)。値 = grid − ベースライン(**負値はそのまま**)。TH2D は x = time bucket(512 bin、[0, 512))、y = strip(nStrips bin、[1, nStrips + 1))。描画は `colz;nostat`(log 切替で `logz`)。
  - ZS の復元(TODO/152)は ZS フレーム + ペデスタル表のときだけ。raw ROOT を読むこのマクロでは扱わない(範囲外)。
- 入力 `*_raw.root`: tree `raw`、`eventId`(UInt_t)、`adc`(Short_t [4][68][512]、FPN 込みの生 ADC)。読み方は `macros/analyzeUVW.C` の 172–181 行と同じ。
- `macros/channel_map.csv`(gitignore、内部データ — コミット禁止): `aget,raw_channel,view(0=U 1=V 2=W),strip,s_mm`。主対話が実ジオメトリ `new_geometry_mini_eTPC.dat` と突き合わせ、(aget, raw ch) → (view, strip) が **256 行すべて一致**(FPN {11, 22, 45, 56} を飛ばした raw ch)= UI と同じストリップ番号。mini は 1 strip = 1 ch(U 72 / V 92 / W 92)。
- PDF の大きさ(run 209 の 20 事象で実測): しきい値なしの colz = **≈ 564 kB / ページ**(7,946 事象 ≈ 4.4 GB — ユーザー了承)。`SetMinimum(20)` + フレームを最下位のパレット色で塗る = 105 kB、`SetMinimum(35)` = 42 kB。PDF のストリームは既に圧縮済み(`mutool clean -z` で縮まない)。`col2z` は PDF に何も描かれない(使わない)。
- `c.Print(pdf, "Title:event 635")` で PDF のしおり(outline)にページ名が入る(ROOT 6.40.04 で確認)。

## 作るもの
### 1. `macros/drawUVW.C`
```
void drawUVW(const char *rawFile, const char *pdfFile = "", Long64_t first = 0, Long64_t n = -1,
             double drawMinAdc = -1, bool logz = false)
```
- 入力の検査(何も作る前に): tree `raw` が無い / `channel_map.csv` がカレントに無い / 範囲 `first < 0 || first >= nTotal || n == 0` → ERROR を出して何も書かない(analyzeUVW.C と同じ流儀・同じ文言の型)。
- 出力名: `pdfFile` が空なら `rawFile` の `_raw.root` → `_uvw_events.pdf`。入力名に `_raw.root` が無く `pdfFile` も空、または出力名 = 入力名 → ERROR、何も書かない。
- 範囲 `[first, first + n)`(n < 0 は最後まで)を 1 事象 1 ページで書く: `Print("x.pdf[")` → 各事象 `Print("x.pdf", "Title:event <eventId>")` → `Print("x.pdf]")`。
- キャンバス 1200 × 300、`Divide(3, 1)`(= 1 枚 400 × 300)、左から U / V / W。統計箱なし。
- 1 面の中身は**純関数**に分ける(テストが直接呼ぶ):
  `void fillUVW(const Short_t adc[4][68][512], const int view[4][68], const int strip[4][68], TH2D *h[3])`
  (view = −1 は map に無い ch。h の軸は呼ぶ側が nStrips から作る)。中身は上の「web UI の描き方」そのまま: 面・strip ごとに double で加算 → [0, 65535] に clamp → 先頭 25 cell の平均を引く → `SetBinContent(bucket + 1, strip, 値)`。map に ch の無い strip は全 cell 0 のまま(ベースラインも 0)。
- nStrips[面] = map のその面の**最大 strip 番号**(本数ではない — 番号に穴があっても UI と同じ軸になる)。
- タイトル: `Form("event %u (entry %lld)  %s;time bucket;strip", eventId, entry, "U"/"V"/"W")`。
- `drawMinAdc < 0`(既定): 何もしない = UI と同じ(負値も描く)。`drawMinAdc >= 0`: 各面で `SetMinimum(drawMinAdc)` し、pad のフレームを `TColor::GetColorPalette(0)` で塗る(しきい値未満のセルを PDF に書かず、背景色で見せる — ファイルを小さくしたいとき用)。
- `logz`: 各 pad に `SetLogz()`。
- TH2D は 3 枚を最初に作って事象ごとに `Reset()` で使い回す(事象ごとに new しない)。
- 進捗: 500 事象ごとに 1 行、最後に「書いたページ数・ファイル名」。
- ファイル先頭のコメントは analyzeUVW.C と同じ形(何をするか・UI のどの関数と同じか・使い方・しきい値の意味と大きさの目安 564 / 105 / 42 kB)。

### 2. `macros/test_drawUVW.C`(`root -l -b -q macros/test_drawUVW.C` → exit code = 失敗数、0 = green。`tools/scan/test_uvw_closure.C` の流儀)
`drawUVW.C` を `#include` して `fillUVW` を直接呼ぶ単体テストと、一時ディレクトリでの通しテスト。合成 channel_map は一時ディレクトリに書き、そこへ `gSystem->ChangeDirectory` して呼ぶ(実物の map を読まない)。
1. ベースライン: strip の cell 0..24 = 100、cell 200 = 300 → 値は cell 0 で 0、cell 200 で 200。
2. ベースラインは先頭 25 cell だけ: cell 0 = 100、cell 1..24 を 90 / 110 交互(平均ちょうど 100)、cell 25 = 1000 → cell 25 の値 = 900、cell 30(= 100)の値 = 0。
3. 負値を保つ: cell 0..24 = 100、cell 300 = 40 → 値 −60。
4. FPN の ch(raw 11)と map に無い ch に大きな値 → どの面にも入らない。
5. 同じ strip に 3 ch(合成 map のセクション)を割り当て、各 30,000 → 和 90,000 は 65,535 に clamp してからベースラインを引く。2 ch の和が clamp 未満なら単純な和。
6. map に ch の無い strip(合成 map で strip 番号に穴: 1, 2, 5 だけ)→ その行は全 cell 0、nStrips = 5(軸 y が 5 bin [1, 6))。
7. 軸: x = 512 bin [0, 512)、タイトルに eventId と entry と面の名前が入る。
8. 通し: 合成 raw(3 事象、eventId = 10, 11, 12)→ `drawUVW` → PDF ができ、ページ数 3(`/Type /Page` を数える等)、しおりに `event 10`〜`event 12`。`first = 1, n = 1` → 1 ページで `event 11`。
9. 異常系: 範囲外の `first`、tree の無いファイル、map の無いディレクトリ、`_raw.root` を含まない入力で pdfFile 空 → どれも PDF を作らない。
10. `drawMinAdc = 20` で通ること(ページ数が同じ。大きさは見ない)。

### 3. `macros/README.md`
表に `drawUVW.C` の行を足す(入力 `*_raw.root` → 出力 `*_uvw_events.pdf`、UI と同じ描き方、1 事象 1 ページ、しおり = event 番号、大きさの目安、`drawMinAdc`)。

## 受け入れ(実データ、ローカルのみ)
- `cd /Users/aogaki/Workspace/tpcdaq-rs/macros && root -l -b -q 'drawUVW.C("data/run0209_raw.root", "/private/tmp/claude-501/-Users-aogaki-Workspace-tpcdaq-rs/c5696e1c-f7d9-4a2e-ab7f-ab99dda8905b/scratchpad/t190/r209_20.pdf", 0, 20)'` → 20 ページ、≈ 560 kB / ページ。同じく `drawMinAdc = 20` で ≈ 105 kB / ページ。所要時間を記録。
- 1 ページ目を PNG にして(`qlmanage -t -s 1200 -o <dir> <pdf>` が使える)目で確認: U は strip ≈ 55–65・bucket ≈ 190–230 に斜めの飛跡、W は strip ≈ 0–30 に逆向きの飛跡、背景は濃い青でノイズの縦縞が W に見える(主対話の試作と同じ絵)。
- 全事象の PDF は作らない(主対話が受け入れ後に作る)。

## 禁止・注意
- `macros/data/`・`channel_map.csv`・PDF・PNG をコミットしない / 変更しない / 消さない。コミット自体しない(主対話がまとめる)。
- 触るのは `macros/drawUVW.C`・`macros/test_drawUVW.C`・`macros/README.md` だけ。作業ツリーには他チケットの未コミットの変更があるので、それ以外のファイルに触れない。
- ROOT は `export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX27.sdk` を付けて動かす(ROOT 6.40 / CLT 27)。
- 発注書に無い設計の分岐に出会ったら、実装せずに報告して戻る。

## 報告の形
変更したファイルと行数 / テスト名ごとの PASS・FAIL と実行コマンド / red の証拠(実装前にテストが落ちたこと)/ 受け入れの数値(ページ数・kB / ページ・所要時間)と PNG のパス / 発注書から外れたところとその理由 / **実行できなかった操作**(権限で拒否された・失敗したコマンド。無ければ「なし」)。

## 結果
- 実装: implementer(Sonnet、2026-11-02)。主対話が照合(`git status`: 新規 `macros/drawUVW.C` 206 行・`macros/test_drawUVW.C` 297 行、`macros/README.md` +1 行。ほかのファイルは無変更。`macros/canvas.ps`・`source_410to470.pdf` は 9-28 からある無関係の未追跡ファイル)。コードは発注書のアルゴリズムどおり(UI の clamp は加算ごと、こちらは和のあと — 加える値がすべて非負なので結果は同じ)。
- テスト: `export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX27.sdk; root -l -b -q macros/test_drawUVW.C` → **32 PASS / 0 FAIL(GREEN、exit code 0)**。発注書の [1]–[10] がすべてテスト名に対応(主対話が再実行して確認、2026-11-02、Mac / ROOT 6.40.04)。
- red の証拠: `drawUVW.C` が無い状態で `fatal error: 'drawUVW.C' file not found`、exit code 1。
- 実データの受け入れ(`macros/data/run0209_raw.root`、entry 0–19):
  | 条件 | ページ | 大きさ | 1 ページ | 時間 |
  |---|---|---|---|---|
  | しきい値なし(既定 = UI と同じ) | 20 | 11,556,488 B | ≈ 578 kB | 2.5 s |
  | `drawMinAdc = 20` | 20 | 2,162,345 B | ≈ 108 kB | 0.9 s |
  1 ページ目の PNG は主対話の試作と同じ絵(U の斜めの飛跡 strip ≈ 55–65 / bucket ≈ 190–230、W の逆向きの飛跡と縦縞のノイズ)。
- 発注書から外れたところ(受け入れ): 補助関数 3 つ(`uvwdrawLoadMap` / `uvwdrawMakeHists` / `uvwdrawTitle` — テストが nStrips とタイトルを見るため)、`fillUVW` の中で `Reset()`、ERROR を 1 つ追加(map に strip が 1 本も無い面)、範囲外の map 行は WARNING で飛ばす。
- 実行できなかった操作: なし(implementer の報告)。
- **全事象(主対話、2026-11-02)**: `cd macros && root -l -b -q 'drawUVW.C("data/run0209_raw.root")'` → 7,946 ページ・4,184,345,025 B(≈ 527 kB / ページ)・13 分 44 秒。
  - **2 GiB を超えると ROOT の TPDF が壊れた xref を書く**(バイト位置を 32 ビット int で持つ — `startxref -111419595`)。MuPDF は開くときに修復(8 s)して 7,000 ページ目も描けたが、Preview が修復する保証はない。
  - `mutool clean` で書き直し → `startxref 4183342737`、修復なしで 0.8 s で開く、7,946 ページ、しおり 7,946 個(`event 0` → page 1 … `event 6999` → page 7000)。壊れた方を消し、書き直した方を `macros/data/run0209_uvw_events.pdf`(4,184,138,444 B)に置いた。
  - 主対話の追加(silent にしない): 書き終えた PDF が 2 GiB を超えたら WARNING と直し方(`mutool clean` か first / n で分割)を出す(`drawUVW.C` の末尾、`gSystem->GetPathInfo`)。ヘッダのコメントと README にも注意を 1 行。未使用の `#include <cmath>` を削除。テスト再実行 → GREEN(0 failure)。WARNING の経路自体は 2 GiB の出力が要るのでテストなし(上の実走で条件を確認済み)。
