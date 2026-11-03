# 199 — drawUVW のズームを、はぐれたセル・雑音の縞・はぐれた 3D 点に強くする(連結成分)

**Status: COMPLETED**(2026-11-02 起票 Opus、同日完了。実装 = implementer / Sonnet)

## 動機
193(archive)で上段 UVW を mm・自動ズーム・縦横そろえにした。run 209 の event 27 では、W 面の右側(z ≈ 70–120 mm)の雑音の縦縞と、はぐれたセルが「信号のセル」(> max(35, 面の最大 × 0.1))に入り、窓が全体になってズームしない。下段も、はぐれた 3D 点 1 つ(z ≈ 74 mm)で窓が伸びる。193 の implementer の確認: 0.2 にすれば 27 の W は z 30–54・s 21–49 に収まる。ただ閾値を上げるだけだと、暗い腕(α の出だし)を窓から落とす。**塊として繋がっているかで選ぶ**ほうが筋がよい。

## 作るもの(`macros/drawUVW.C` を変更)
1. **上段**: 面ごとに、信号のセル(規則は 193 のまま: > max(`UVWDRAW_HIT_ADC` = 35, 面の最大 × `UVWDRAW_ZOOM_FRACTION` = 0.1))を (strip, cell) の格子で **8 近傍の連結成分**に分け、成分の電荷 = 値の和。**最大の成分の電荷 × `UVWDRAW_ZOOM_KEEP` = 0.1 以上の成分だけ**から z と s の範囲を取る(2 本の腕が離れていても、どちらも十分明るければ両方残る)。残りは 193 と同じ(z は 3 面の和集合、共通の k、縦横そろえ)。
2. **下段**: 3D 点を **単連結(点どうしの距離 ≤ `UVWDRAW_LINK_MM` = 5 mm)**で塊に分け、塊の電荷(点の重みの和)が最大の塊 × 0.1 以上の塊の点だけから窓を取る。
3. 定数は先頭にまとめ、コメントで意味を書く。純関数に分けてテストする。
4. 0.1 で 27 がズームしなければ、`UVWDRAW_ZOOM_FRACTION` を 0.2 に上げてよい(どちらにしたか報告)。

## テスト(`macros/test_drawUVW.C` を拡張、exit code = 失敗数。既存 [1]–[26] は通ったまま)
27. 連結成分: 合成の面で、明るい塊(電荷 10,000)+ はぐれたセル 1 つ(電荷 200)+ 縦縞(全 strip × 3 cell、各 50 → 電荷 < 10 %)→ 窓は明るい塊だけから。
28. 離れた 2 本の腕(電荷 10,000 と 3,000、繋がっていない)→ 両方とも窓に入る。2 本目が 500 なら入らない。
29. 8 近傍: 斜めにだけ接するセルは同じ成分。
30. 3D 点: 塊(10 点)+ 20 mm 離れた 1 点(重みが塊の 5 %)→ 窓は塊だけ。5 mm 以内でつながる鎖は 1 つの塊。

## 受け入れ(実データ、ローカルのみ)
- `cd /Users/aogaki/Workspace/tpcdaq-rs/macros && root -l -b -q 'drawUVW.C("data/run0209_raw.root", 198, 12.5, 1600, "/private/tmp/claude-501/-Users-aogaki-Workspace-tpcdaq-rs/c5696e1c-f7d9-4a2e-ab7f-ab99dda8905b/scratchpad/t199/png", 0, 300)'` → 300 枚、所要時間。
- **event 27 の上段がズームする**(W の窓が z ≈ 30–55 mm 付近)、下段の窓が はぐれた点で伸びない。244・5・0 が 193 と同じかそれ以上に収まる。PNG のパスを書く。
- 全事象は作らない(主対話が受け入れ後に作る)。

## 禁止・注意
- 触るのは `macros/drawUVW.C`・`macros/test_drawUVW.C`・`macros/README.md`(drawUVW の行に一言)だけ。同時に別の implementer が `macros/selectNeutron.C`・`macros/fitProngs.C`(と各テスト)を作っている — それらに触れない。コミットしない。
- `macros/data/` は読むだけ。
- ROOT は `export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX27.sdk` を付けて動かす。
- 発注書に無い設計の分岐に出会ったら、実装せずに報告して戻る。

## 報告の形
変更したファイルと行数 / テスト名ごとの PASS・FAIL と実行コマンド / red の証拠 / 受け入れの数値と PNG のパス / 発注書から外れたところとその理由 / **実行できなかった操作**(無ければ「なし」)。

## 結果(2026-11-02)
- 実装: implementer(Sonnet)。触ったのは `macros/drawUVW.C`(798 → ≈ 870 行: 定数 `UVWDRAW_ZOOM_KEEP = 0.1`・`UVWDRAW_LINK_MM = 5.0`、`uvwdrawSignalRange` を 8 近傍の連結成分 + 最大成分 × 0.1 以上だけに、純関数 `uvwdrawKeepPointBlobs` = 3D 点の単連結 ≤ 5 mm、下段の窓はこれを通した点だけ。塗りは全点のまま)・`macros/test_drawUVW.C`(770 → ≈ 860 行)・`macros/README.md`(drawUVW の行に一言)。`UVWDRAW_ZOOM_FRACTION` は 0.1 のままで足りた。
- テスト: `export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX27.sdk; cd macros && root -l -b -q test_drawUVW.C` → **109 PASS / 0 FAIL(GREEN、exit 0)**、[1]–[26] は通ったまま、[27]–[30] 追加。主対話が再実行して確認(Mac / ROOT 6.40.04)。
- red の証拠: 未宣言のコンパイルエラー → 空の仮実装で `RED: 5 failure(s)`([27] 縞・はぐれセル、[28] 500 の腕、[29] 2 セル離れ、[30] 離れた点・6 mm 間隔)。
- 受け入れ(run 209、entry 0–299、`…/scratchpad/t199/png/`): 300 枚 ≈ 16.5 s。**event 27 の上段がズーム**(信号 z ≈ 28–55 mm、W の z ≈ 70–85 の縦縞は窓の端に残るだけ。縦横そろえで U の s の広がりが k を決めるので窓 z ≈ 18–87)、下段ははぐれた点で伸びない(主対話が PNG で確認 — 2 本の腕が 3 面・3D とも見える)。244・5・0 も追従。
- 発注書から外れたところ(受け入れ): テスト [27] の塊の電荷を 10,000 → 100,000(合成の縞 = 61 strip × 3 cell × 50 = 9,150 が 10,000 の 10 % を超え、「縞は 10 % 未満」を作れないため)。
- 実行できなかった操作: なし。
- **全事象(主対話、2026-11-02)**: `cd macros && root -l -b -q 'drawUVW.C("data/run0209_raw.root", 198, 12.5, 1600)'` → `macros/data/run0209_uvw_events/` の 7,946 枚を新しい表示(mm・ズーム・縦横そろえ・連結成分)で上書き、合計 236 MB(≈ 30 kB / 枚)、6 分 48 秒。
