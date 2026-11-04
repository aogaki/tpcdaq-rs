# 207 — analyzeUVW の strip 電荷の切り(`STRIP_Q_MIN = 2000`)が peaking 232 ns で高すぎ、トラックの淡い側が切れて偽の contained が出る

**Status: COMPLETED(macros 側、2026-11-04 — オンライン `src/reco` 側は [210](../210_online_reco_strip_q_min.md) に分けた)**(2026-11-04 起票 Opus。出どころ = run 216(232 ns)の全事象の解析)

## 何が起きたか
- run 216(peaking 232 ns、N1068GE トリガ、線源なし、2.54 h、434 事象)を `par_analyzeUVW.sh -p 198 -s 12.5 -d 1600 --tracks` → `selectNeutron.C` に通すと、**contained が 8 本(3.1 /h)**(215 = 502 ns・16.9 h では 0、208 では 0.10 /h)、wall-alpha 13.4 /h(215 は 5.7)。
- 8 本を drawUVW で見ると(scratchpad `r216/png/`)、どれもまっすぐな長いトラックで、片端が有感の箱の縁(y ≈ 47–48 mm など)にある。**makeTracks の端点が絵の 3D 点と食い違う**(例 entry 230: 絵は z 38 → 88 mm、makeTracks は z 55 → 58 で xy も鏡映)。
- 原因: analyzeUVW の strip ごとの電荷 q(ピーク ± 5 cells の和)の切り `STRIP_Q_MIN = 2000`(2026-09-25 の 25 MHz / 502 ns のデータで「halo < ≈ 1,500、トラック ≈ 5,500 の谷」として決めた値)。232 ns ではパルスの面積が ≈ 0.45 倍(138 の核)で、トラックの淡い側の strip が 2,000 を割って落ちる。entry 230 の V 面は絵では V −19 → 63 だが残ったヒットは −16.5 → 19.5(最小 q 2,076)、entry 21 の V 面は −15 → 50 に対して −9 → 12。片面が短くなり、makeTracks の方向と端点が狂う。
- 電荷 / mm の中央(reject / wall / vin-x / vin-y): 216 = 3,968 / 4,338 / 4,101 / 4,516、215 = 6,703 / 6,539 / 7,617 / 7,533(比 ≈ 0.55–0.66)。

## 切りを変えた試験(2026-11-04、scratchpad `q207/` に analyzeUVW の写し — リポの analyzeUVW.C は無変更)
| STRIP_Q_MIN | 216(232 ns): contained / wall-alpha / vin-x / reject /h | 215(502 ns): contained / wall-alpha / vin-x / reject /h |
|---|---|---|
| 2,000(今) | **3.14** / 13.36 / 42.1 / 82.2 | 0 / 5.73 / 34.8 / 87.2 |
| 1,200 | **0** / 5.11 / 45.6 / 87.7 | 0.06 / 2.95 / 36.0 / 89.6 |
| 900 | **0** / 5.90 / 45.6 / 91.2 | **0.41** / 1.71 / 33.9 / 94.1 |
- 216 は 1,200 で偽の contained が消え、wall-alpha が 215(2,000)と揃う。215 は 900 まで下げると halo を拾って contained が出る → **切りは peaking(と sampling)ごとに変える必要がある**。比 ≈ 0.5–0.6 は電荷 / mm の比・核の面積比と合う。

## 直すこと(案 — 設計は Opus で足りる。KISS)
- analyzeUVW に `stripQMin` を任意引数で足す(既定 2,000 = 今の挙動)か、run の条件(peaking)から引く表を持たせる。推し = **核の面積比で決める**のではなく、**事象ごとの halo の分布から谷を取る**のでもなく、まずは **引数 + README の表(502 ns → 2,000、232 ns → 1,200)**。par_analyzeUVW.sh に `-q` を通す。
- 25 MHz と 12.5 MHz の差(± 5 cells の窓は 12.5 MHz で 880 ns)も同じ表に。202 / 209 / 215 の結果は 2,000 のままでよい(502 ns)。
- テスト: test_uvw_closure 流の合成で、232 ns の核の形の淡い strip が 2,000 で落ち 1,200 で残ること。実データ: 216 で contained 0、wall-alpha が 215 と ± 30 % で揃う。
- 影響: fitProngs は makeTracks の線を初期値と ROI に使うので、232 ns の当てはめ(198「その後」)はこの後に。drawUVW(表示)は影響なし。

## 谷の測定と値の決定(2026-11-04、主対話 — ユーザー「今後は 232 ns で行こうと思う」)
- STRIP_Q_MIN = 300 の写しで strip ヒットの q の分布(対数ビン、scratchpad `q207/qhist.C`): **502 ns(run 215 の先頭 600 事象)** = halo の山 ≈ 500、**谷 1,700–2,000**、トラックの山 ≈ 4,400–5,300(今の 2,000 は谷の上端 = 妥当)。**232 ns(run 216 全 434 事象)** = halo の山 ≈ 700、**谷 950–1,150**、トラックの山 ≈ 2,500–3,000。トラックの山の比 0.56 × 2,000 = 1,130 も谷に入る → **232 ns = 1,100**。
- 切りを変えた試験(上の表)で 900・1,200 とも 216 の偽の contained が 0 になることは確認済み。

## 発注書(macros 側 — implementer / Sonnet)
**作るもの**
1. `macros/analyzeUVW.C`: 引数の末尾に `double stripQMin = 2000.0`(`outFile` の後)。`STRIP_Q_MIN` 定数は既定値として残し、halo の切りはこの引数で行う。`stripQMin <= 0` はエラーで何も書かない。出力の uvw ファイルに `TParameter<double>("strip_q_min")` を書き、起動時の 1 行に値を出す。ヘッダのコメント(halo cut の段落)に「peaking ごとの値 = README の表」を 1 文。
2. `macros/par_analyzeUVW.sh`: オプション `-q <stripQMin>`(既定 = 指定なし = analyzeUVW の既定 2000)を各部分の analyzeUVW に渡す。usage とヘッダの説明に追加。`--tracks` はそのまま。
3. `macros/README.md`: analyzeUVW の行と par_analyzeUVW の段落に `stripQMin` と表 **「peaking 502 ns → 2,000(run 215 の谷 1,700–2,000)/ 232 ns → 1,100(run 216 の谷 950–1,150)」**(sampling 12.5 MHz、± 5 cells の窓)。
**テスト**
- `macros/test_par_analyzeUVW.sh` の流儀か新しい小さな ROOT テスト `macros/test_analyzeUVW_qmin.C`(合成の raw: 1 本の直線トラック、strip ごとのパルスの面積を 232 ns の核に見立てて q ≈ 1,500 の strip を数本含める)で: `stripQMin = 2000` ではその strip が落ち、`1100` では残る。`strip_q_min` の TParameter が書かれる。既定(引数なし)の出力が今の analyzeUVW とヒット単位で一致(回帰)。`-q` が par_analyzeUVW を通って各部分に効く(serial == parallel の比較、`compareUVW.C`)。
**受け入れ(実データ)**
- run 216(`/Users/aogaki/Workspace/tpcdaq-rs/macros/data/run0216_raw.root`、198 mbar / 12.5 MHz / 1,600 V)を `par_analyzeUVW.sh -q 1100 --tracks` → `selectNeutron.C` で contained 0、wall-alpha が 215(2,000)の 5.7 /h と ± 50 % で揃う(数字を報告)。run 215 を既定(2,000)で回して今の `run0215_uvw.root` と一致。
**禁止**
- 触るのは `macros/analyzeUVW.C`・`macros/par_analyzeUVW.sh`・`macros/README.md`・新しいテストファイルだけ。`makeTracks.C`・`selectNeutron.C`・`drawUVW.C`・`fitProngs.C` は触らない。`macros/data/` は読むだけ(出力は scratchpad)。`src/`(オンラインの `src/reco` は別 — Fable キュー)・`docs/` は触らない。
**報告**: 変更ファイルと diff の要約、テスト名と PASS / FAIL、赤の証拠、受け入れの数字、**「実行できなかった操作」節(無ければ「なし」)**。

## オンライン(`src/reco`)側 — Fable キュー
- `src/reco/mod.rs:39` の `STRIP_Q_MIN = 2000.0` も同じ(SPEC §5.5 = macros の移植、定数は macros と同じに固定)。232 ns では UI の Event Display / QA 再構成の線が短く出る(表示だけ、保存には影響なし)。直し方の案: `[monitor] reco_strip_q_min`(既定 2,000、ELITPC は 1,100)を TOML に足して `src/reco` に渡す(SPEC §3.1・§5.5 の改訂 = Fable の確定が要る)。代案: params(current.json)の `peakingTime_ns` から表で引く(monitor は今 params を読まない)。

## 結果(macros 側、2026-11-04)
- **実装**: implementer(Sonnet)、worktree `.claude/worktrees/agent-ab108d6ce9d98de42`、コミット **`91c99ad`** → main に **`5cdfd1e`**(Merge 207)。5 ファイル + 179 / − 13: `macros/analyzeUVW.C`(末尾の引数 `double stripQMin = STRIP_Q_MIN`、≤ 0 は ERROR で何も書かない、起動時に 1 行、出力に `TParameter<double>("strip_q_min")` — hadd 後も残る)、`macros/par_analyzeUVW.sh`(`-q <v>`、正の数でなければ exit 2、未指定なら引数を足さない = 2,000)、`macros/README.md`(usage・analyzeUVW の行・peaking の表 502 ns → 2,000 / 232 ns → 1,100)、新規 `macros/test_analyzeUVW_qmin.C`、`macros/test_par_analyzeUVW.sh` に T8a–d(既存テストへの追記 = 逸脱、承認)。channel_map.csv はコミットしていない。主対話が `git show --stat 91c99ad` を照合。
- **テスト**: `cd macros && root -l -b -q test_analyzeUVW_qmin.C` → **6 PASS / 0 failures**(合成 1 本の直線トラック、q = 4,000 × 4 / 3,000 / 1,500 の 6 strip × 3 面: 2,000 で淡い strip が落ち 5 hits × 3、1,100 で残り 6 × 3、引数なし = 2,000、TParameter 1,100 / 2,000、0 はエラー)。**主対話が main の上で再実行して同じ 6 PASS**(2026-11-04)。赤 = 実装前は「requires at most 7 arguments, but 8 were provided」。`./test_par_analyzeUVW.sh macros/data/run0216_raw.root <scratchpad>/t207/w` → **44 passed / 0 failed**(T8a `-q 1100` exit 0、T8b 直列 == 並列 `-j 3`(compareUVW)、T8c hadd 後の `strip_q_min` 1,100、T8d `-q -5` は exit 2。T8 の赤は構造上(実装前は unknown option)で実測していない)。
- **受け入れ(実データ)**: run 216 を `par_analyzeUVW.sh -p 198 -s 12.5 -d 1600 -q 1100 --tracks` → `selectNeutron.C`(434 事象、2.544 h): blob 3.14 / reject 88.05 / **contained 0** / **wall-alpha 4.72 /h**(215 の 5.7 に対し − 17 %)/ vin-x 46.38 / vin-y 28.30 / vin-z 0。run 215 を既定(2,000)で回して既存の `run0215_uvw.root` と **全 entry / branch / bin が一致**(compareUVW PASS)。
- **実行できなかった操作**: なし(worktree 隔離の検査で複合コマンドが 2 回拒否 → 分割して実行)。
- **今後の 232 ns の run**: `par_analyzeUVW.sh -p 198 -s 12.5 -d 1600 -q 1100 --tracks <raw>`。
