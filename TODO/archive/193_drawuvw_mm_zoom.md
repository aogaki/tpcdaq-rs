# 193 — drawUVW: 上段 UVW を mm 軸に、ヒットの範囲へ自動ズーム、縦横の mm をそろえる

**Status: COMPLETED**(2026-11-02 起票 Opus、同日完了。実装 = implementer / Sonnet)

## 動機
192 の提案 1(ユーザー「1 をやっちゃってください」)。event 244(¹²C(n,n')3α、各 α は数 mm〜1 cm)は今、パネルの ≈ 3 % × 6 % にしか描かれず形が読めない。TPCReco の GUI は UVW を mm で描き自動ズームする(`HistoManager::makeAutozoom`)が、縦横の mm をそろえないので角度が歪む。ここでは**ズーム + 縦横の mm を 3 面共通の縮尺でそろえる**(見たままの角度・長さが正しい)。

## 調べ済みの事実
- `macros/channel_map.csv` の s_mm は 3 面とも **s = 1.5 × strip + 定数**(U −54.75、V −73.5、W −65.25)で誤差なく一直線(主対話が全 256 行で確認)。→ 上段の y 軸は strip 番号の bin をそのまま mm に貼り替えられる。
- 現行 `macros/drawUVW.C`(archive/191): 上段 = x time bucket / y strip 番号、下段 = 3D 点の XY / XZ / YZ(mm、1 mm ビン、範囲は map から)。PNG ディレクトリ既定 / `.pdf` で PDF。

## 作るもの(`macros/drawUVW.C` を変更)
1. 呼び方: 末尾に `bool zoom = true` を足す(`…, double drawMinAdc = -1, bool logz = false, bool zoom = true)`)。
2. **上段を mm 軸に**: x = z [mm](bin b ↔ [b, b + 1) × mmPerCell、範囲 [0, 512 × mmPerCell])、y = ストリップ位置 s [mm](strip k ↔ [s_k − p/2, s_k + p/2]、範囲 [s(1) − p/2, s(nStrips) + p/2])。軸の名前 `z [mm]` / `U [mm]`・`V [mm]`・`W [mm]`。値は `fillUVW` のまま(UI と同じアルゴリズム — 変えない)。
   - 面ごとに map から s = slope × strip + offset を求め、**全行がこの直線に 1e-6 mm で乗ること**を検査(乗らなければ ERROR で何も書かない)。|slope| = その面の pitch。
   - **slope が負**の面は y を逆順に詰める(bin j ↔ s の小さい順)。mini は正だが、ジオメトリを焼き込まないため。
3. **ズームの窓**(純関数に分けてテストする):
   - 信号のセル = 値 > max(`UVWDRAW_HIT_ADC` = 35, その面の最大値 × `UVWDRAW_ZOOM_FRACTION` = 0.1)(ノイズの縦縞を窓に入れないため — TPCReco の 10 % と同じ考え)。
   - 面ごとに信号のセルの z の範囲と s の範囲。z は 3 面の和集合(同じ時刻)。
   - 縮尺 k [mm / px] = max( (Δz + 2m) / W, max_v (Δs_v + 2m) / H, L_min / H )。m = 5 mm、L_min = 20 mm、W × H = 上段のパッドの**フレーム**(軸の内側)のピクセル寸法(パッドの大きさと余白から計算。3 枚とも同じ)。
   - 窓: z = z の中心 ± k·W / 2(3 面共通)、s_v = 面 v の s の中心 ± k·H / 2。**3 面とも同じ k** = 縦横の mm がそろい、面どうしで長さを比べられる。
   - 信号のセルが 1 つも無い事象、または `zoom = false`: 全体の範囲を、同じ規則(縦横そろえ、全体が入る最小の k)で描く。
   - 窓はヒストの軸の外にはみ出してよい(検出器の端の事象)。描き方は実装に任せる(例: `gPad->DrawFrame(z0, s0, z1, s1)` に `colz same`)が、**パレット(z の色の目盛り)が出ること**、タイトルと軸の名前が付くこと。
4. **下段(3D 点)も同じ考え**: XY・XZ・YZ の 3 枚で共通の縮尺 k3(3D 点の x・y・z の範囲 + m、L_min、それぞれのフレームの寸法から上と同じ式)、縦横そろえ。XZ と YZ の z の窓は共通。3D 点が無い事象 / `zoom = false` は全体の範囲(縦横そろえ)。
5. 事象ごとに作る描画物(フレーム等)は次の事象で消す — **全事象を回してメモリが増え続けないこと**(`gSystem->GetProcInfo` の常駐量: entry 500 と 2,500 の差 < 50 MB を受け入れで確認)。

## テスト(`macros/test_drawUVW.C` を拡張、exit code = 失敗数)
既存の [1]–[19] は新しい軸に合わせて直して残す(軸の範囲の期待値が mm になる項目)。追加:
20. **mm 軸**: 合成 map(pitch 2 mm、offset ≠ 0)で上段の y 軸の範囲 = [s(1) − 1, s(n) + 1]、x 軸 = [0, 512 × mmPerCell]。strip k の値が s_k の bin に入る。
21. **負の slope**: s = −2 × strip + c の合成 map で、値が s の小さい順の bin に逆に入る。
22. **直線に乗らない map**(1 行だけ s を 0.3 mm ずらす)→ ERROR、何も作らない。
23. **窓の純関数**: 手計算の例(Δz・Δs_v・W・H を与えて k と窓)。3 面で同じ k、窓が信号の範囲 + m を含む、L_min が効く例、`zoom = false` と信号なしで全体。
24. **信号のセルの規則**: 最大 1000 の面で 90 のセル(> 35 だが < 100)は窓に入らない、150 は入る。
25. **縦横そろえ(描いた結果で)**: 描いた後の 6 枚のパッドで (x 幅 [mm] / フレームの幅 [px]) と (y 幅 [mm] / フレームの高さ [px]) が 1 % で一致。上段 3 枚で同じ値。
26. 3D 点のある合成事象で下段の窓が点を含み、縦横そろい。

## 受け入れ(実データ、ローカルのみ)
- `cd /Users/aogaki/Workspace/tpcdaq-rs/macros && root -l -b -q 'drawUVW.C("data/run0209_raw.root", 198, 12.5, 1600, "/private/tmp/claude-501/-Users-aogaki-Workspace-tpcdaq-rs/c5696e1c-f7d9-4a2e-ab7f-ab99dda8905b/scratchpad/t193/png", 0, 300)'` → 300 枚、1 枚の大きさ、所要時間。
- event 244・5・27・0 の PNG を目で見て報告: 244 の塊がパネルいっぱいに広がる(3 本に分かれて見えるかは問わない — 拡散と同じ大きさなので)、27 の 2 本の角度が上段と下段で矛盾しない、0 の 1 本が切れずに入る。PNG のパスを書く。
- メモリ: entry 0–2,999 を回し、entry 500 と 2,500 の常駐量(上の 5)。
- `zoom = false` でも 10 事象描けること。
- 全事象は作らない(主対話が受け入れ後に作る)。

## 禁止・注意
- 触るのは `macros/drawUVW.C`・`macros/test_drawUVW.C`・`macros/README.md` だけ。作業ツリーには他チケットの未コミットの変更がある。コミットしない。
- `macros/data/` は読むだけ(既存の `run0209_uvw_events/`・`run0209_uvw_events.pdf` にも触れない — 受け入れの出力は scratchpad へ)。`channel_map.csv` を書き換えない。
- ROOT は `export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX27.sdk` を付けて動かす。
- 発注書に無い設計の分岐に出会ったら、実装せずに報告して戻る。

## 報告の形
変更したファイルと行数 / テスト名ごとの PASS・FAIL と実行コマンド / red の証拠 / 受け入れの数値(枚数・kB / 枚・所要時間・メモリ)と PNG のパス / 発注書から外れたところとその理由 / **実行できなかった操作**(無ければ「なし」)。

## 結果
- 実装: implementer(Sonnet、2026-11-02)。触ったのは `macros/drawUVW.C`(798 行)・`macros/test_drawUVW.C`(770 行、≈ 240 行追加)・`macros/README.md`(drawUVW の行)だけ(主対話が `git status` で確認)。
- テスト: `export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX27.sdk; cd macros && root -l -b -q test_drawUVW.C` → **99 PASS / 0 FAIL(GREEN、exit 0)**。追加 [20] mm 軸 7 件、[21] 負の slope 3、[22] 直線に乗らない map 3、[23] 窓の純関数 10、[24] 信号セルの規則 4、[25] 縦横そろえ(描いた結果)7、[26] 3D の窓 3。既存 [1]–[19] は通ったまま。主対話が再実行して確認(2026-11-02、Mac / ROOT 6.40.04)。
- red の証拠: 旧 drawUVW.C で `unknown type name 'UvwdrawPlane'` 等のコンパイルエラー。受け入れ中に見つけた不具合(下)の回帰テストは、修正を戻すと `FAIL [25] … (got 50.000000)`。
- 受け入れ(run 209、entry 0–299): 300 枚、≈ 31.8 kB / 枚、1200 × 600、16.6 s(≈ 55 ms / 事象)、c = 0.0172 mm。メモリ(`gSystem->GetProcInfo`、500 事象ごとの呼び出し、entry 0–2,999): 531,744 → 538,496 kB、500 と 2,500 の差 ≈ 7 MB(上限 50 MB)で平ら。`zoom = false` も 10 事象 OK。
  - 主対話の目視: **244 は塊がパネルいっぱい**(U・V・W に腕状の構造が見える)、5 は長い弧が 3 面いっぱい、0 は切れずに入る。**27 は上段がズームしない**(W の右側の雑音の縦縞が 10 % 規則に入り窓 = 全体)、下段もはぐれた 3D 点 1 つ(z ≈ 74 mm)で窓が伸びる → 199 で直す。
- 受け入れ中に直した不具合: 1 回描いた後は `TH1::GetMaximum()` が描画側のキャッシュ値を返し、次の事象の窓と色の目盛りが狂う(300 枚の試走で 244 がズームしなかった)→ 面の最大値をビンを走査して求め、描く前に `SetMaximum()` / `SetMinimum()` を引数なしで戻す。回帰テストあり。
- 発注書から外れたところ(受け入れ): ① strip 番号のヒストはそのまま(`fillUVW`・点の計算・[1]–[7] を変えない)、描く用の mm 軸のヒストへ写す `uvwdrawToDisp`(1 面 ≈ 31 k ビンの写し)② 信号の無い面は全体の中心・同じ k ③ 全体表示は余白・L_min なしの最小の k ④ map の s_mm を double で読む(1e-6 mm の検査のため)⑤ パッドの余白を 0.14 / 0.16 / 0.13 / 0.10 に固定 ⑥ 描画を `uvwdrawDrawEvent` / `uvwdrawSetupCanvas` に分割(テストが描いた後のパッドを見るため)、`DrawFrame` + `colz same`、パレットあり。
- 実行できなかった操作: なし(`ps` が空を返したので `GetProcInfo` に替えた)。
