# 128 — 実データの重ね合わせ(イベントミキシング)で複数飛跡のイベントを作る

**Status: COMPLETED**(2026-10-13 起票 Opus。ユーザー「問題は複数トラックのデータがないこと」→ 重ね合わせを主力に
「それ以外に手はないでしょう」。複数飛跡の再構成(後続、選択肢 B)の開発・受け入れ用データ)

## 目的
単一飛跡の実イベント 2 つ(以上)の生波形を足して、**本物の検出器応答・ノイズ・halo を持つ複数飛跡イベント**を作る。
真値は元イベントそれぞれの単一飛跡フィット(macros / track-fit と同じもの)。線源の α はどれも線源付近から出るので、
2 つ重ねると 1 点から 2 本出る V 字になり、反応事象の頂点に近い形になる。片方を時間方向にずらすと z の違う交差・偶然の重なりになる。

## 形式(SPEC §6.7 の raw ツリー契約をそのまま使う — 下流の macros・オラクルが無改造で読める)
- 入力: `raw` ツリーを持つ ROOT ファイル(graw2root.C の出力 / root-sink の `run{N}_raw.root`)。`adc[4][68][512]/S`(mini = 1 AsAd)。
  AsAd 数はリーフから読む(焼き込まない)。
- 出力: `<out>.root` に
  - TTree `raw`: `eventId/i`(0 からの連番)、`eventTime/l`(A の値)、`adc[..][68][512]/S`(入力と同じ形)。
  - TTree `mix`(真値の索引、1 エントリ = 出力 1 イベント): `eventId/i`、`nSrc/I`、`srcEntry[nSrc]/I`(入力のエントリ番号)、
    `srcEventId[nSrc]/i`、`shift[nSrc]/I`(time cell のずれ、先頭の A は 0)。
- **足し方**(全 ch・FPN 込み、同じ規則): A をそのまま土台にし、B ごとに `ped_B` = B のその ch の cell 1〜20 の平均
  (macros の analyzeUVW.C と同じ窓)を引いて足す: `out[t] = A[t] + Σ_B (B[t − Δ_B] − ped_B)`。`t − Δ_B` が範囲外の cell は 0 を足す。
  結果は **0〜4095 に丸めて頭打ち**(12 bit ADC の飽和を再現)。丸めは四捨五入(整数に戻す)。
- 組の選び方: (a) 乱数(seed 指定、同じ入力・seed なら出力がバイト一致)で N 組、(b) 組と Δ を並べたテキストファイル
  (`srcA srcB [ΔB]` を 1 行ずつ、`#` コメント可)。3 個以上の重ね合わせは (b) で `srcA srcB ΔB srcC ΔC …` と書ける。
  Δ は (a) では範囲 `[-D, D]` の一様乱数(既定 D = 0)。
- 実装: `tools/event_mix/`(ROOT のみ。C++17、`root-config` でビルドする小さな実行ファイルか ACLiC マクロ — macros/ と同じく
  素の ROOT だけで動くこと)。**macros/ には置かない・触らない**(ユーザーのファイル)。

## テスト(合成データのみ — 実データはリポに入れない)
- 合成の `raw` ツリーを作るテスト用ヘルパ(既知の波形)で:
  - B が「ペデスタルだけ(cell 1〜20 と同じ値が全 cell)」なら出力 = A(バイト一致)。
  - Δ のずれ方(B の山が Δ cell 後ろに出る)と、範囲外の cell が 0 を足すこと。
  - 4095 での頭打ちと 0 未満の切り上げ。
  - `mix` ツリーの中身(srcEntry / shift)。
  - 同じ seed で 2 回走らせた出力の `raw` ツリーが一致。
  - AsAd 数がリーフから読まれる(2 AsAd の合成入力でも動く)。
- 手元の実データ確認(env-gated・記録のみ): `reference/exp_data/2026/elitpc_run70/run0070_raw_head200.root` から 100 組を作り、
  本物の macros(analyzeUVW.C → makeTracks.C、`tools/macro_oracle/run_macro_oracle.sh` と同じく一時ディレクトリで)が走ること、
  単一飛跡のフィットが「どちらかの元飛跡に寄る / 間に引かれる」様子を数字で残す(複数飛跡が今どう壊れるかの記録)。

## 受け入れ
- 上のテストが CTest か、`tools/root_sink` と同じ流儀の make ターゲットで走って green。
- `cargo fmt / clippy / test` に影響なし(Rust を触らないこと)。
- 報告: 変更ファイル、テスト名、実データ 100 組での macros の結果の要約(ok 件数、元飛跡との角度差の分布の大まか)、
  **実行できなかった操作**(無ければ「なし」)。

## 結果(2026-10-13、Mac、ROOT 6.40.04)

### 実装
- 613fb6c(Sonnet)→ merge fc95be2。`tools/event_mix/`: `event_mix.hpp`(純関数 mixEvent・randomSpecs・parseSpecs と ROOT 入出力 mixFile)、
  `event_mix.cxx`(CLI)、`test_event_mix.cxx`、Makefile、README、`mixReport.C` + `run_real_check.sh`(実データの記録用、出力はリポ外)。
- 使い方: `./event_mix --in RAW.root --out MIX.root (--random N [--seed S] [--max-shift D] | --pairs FILE)`。
  pairs ファイル: 2 トークン = Δ0、3 以上の奇数 = A に続く (src, Δ) の組、4 以上の偶数は曖昧なのでエラー。丸めは floor(x+0.5)。乱数は mt19937_64。
- 照合: `git show --stat` と発注書の各テスト項目のテスト名で確認してから受け入れた。

### テスト
- `cd tools/event_mix && make test`: **597 passed / 0 failed**(主対話が main で再実行、-Wall -Wextra 警告なし)。
  test_pedestal_only_B_leaves_A_unchanged / test_shift_moves_peak_and_out_of_range_adds_zero / test_clamp_and_rounding /
  test_three_sources_add / test_random_specs_and_parse / test_mix_tree_contents / test_same_seed_same_output / test_nasad_read_from_leaf。
- implementer は実装を先に書いた(テストが先に red になるところは見ていない)— 記録として残す。
- Rust・UI は触っていないので cargo / ng は対象外。

### 実データでの記録(run0070_raw_head200.root、201 エントリ、本物の macros を一時ディレクトリで実行)
- 元イベントの単一飛跡フィット ok: 200 / 201。
- **100 組・seed 7・Δ = 0**: 重ねたイベントの単一飛跡フィットは **100 / 100 が ok**(= 壊れていても ok になる)。両方の元が ok の組は 97。
  - 元の 2 本はほぼ平行(なす角の中央値 5.2°、10° 超は 30 組)— 線源が絞られているので V 字は少ない。V 字が要るときは pairs ファイルで角度の離れた組を選ぶ。
  - フィットの向きは近いほうの元から中央値 8.3°、遠いほうから 14.3°。97 組のうち、どちらかの元から 5° 以内 38、2 本の間 9、**両方の外 50**。
  - 10° 超離れた組では (近いほうとの差) / (2 本のなす角) の中央値 0.40。
- **100 組・seed 7・max-shift 100**(z をずらした補足): ok 100 / 100、なす角の中央値 5.4°、近いほう 6.4° / 遠いほう 16.3°、
  5° 以内 42・間 9・両方の外 47。
- 読み: 単一飛跡フィットは 2 本を平均せず、たいてい近いほうに寄り、しばしば両方の外に引かれる。**ok = 1 は混ざったことの手がかりにならない**
  → 複数飛跡の再構成(130)の前でも、「直線 1 本ではない」の判定が要る根拠。

### 実行できなかった操作
- なし(worktree 隔離の権限チェックで複合コマンドがいくつか拒否 → 単純なコマンドに分けて実行、未実行は残っていない)。
