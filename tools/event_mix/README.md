# event_mix — 実データの重ね合わせで複数飛跡イベントを作る(TODO/128)

単一飛跡の実イベントの生波形を足し、本物の検出器応答・ノイズ・halo を持つ複数飛跡イベントを作る。
ROOT のみ(C++17、TPCReco・ZMQ 不要)。入出力は SPEC §6.7 の raw ツリー契約なので、
`macros/analyzeUVW.C` 以降が無改造で読める。`macros/` は触らない。

```sh
make            # event_mix をビルド
make test       # 合成データの単体テスト(test_event_mix)
./event_mix --in RAW.root --out MIX.root --random N [--seed S] [--max-shift D]
./event_mix --in RAW.root --out MIX.root --pairs FILE
```

- 足し方: `out[t] = A[t] + Σ_B (B[t − Δ_B] − ped_B)`。`ped_B` = B のその ch の cell 1〜20 の平均
  (analyzeUVW.C の窓)。`t − Δ_B` が範囲外の cell は 0 を足す。四捨五入して 0〜4095 に頭打ち。
  全 ch(FPN 込み)に同じ規則。AsAd 数は入力の `adc` リーフから読む。
- `--random`: A != B の組を N 個。同じ入力・`--seed` なら出力は同一。Δ は `[-D, D]` の一様乱数(既定 D = 0)。
- `--pairs`: 1 行 1 イベント。`srcA srcB [ΔB]`、3 個以上は `srcA srcB ΔB srcC ΔC ...`。`#` コメント可。
  トークン数が偶数 >= 4 は曖昧なのでエラー。エントリ範囲外もエラー。
- 出力 `raw`: `eventId`(0 からの連番)、`eventTime`(A の値)、`adc`(入力と同形)。
  出力 `mix`(真値の索引): `eventId`、`nSrc`、`srcEntry[nSrc]`、`srcEventId[nSrc]`、`shift[nSrc]`。

## 実データ確認(env-gated・記録のみ)

```sh
tools/event_mix/run_real_check.sh RAW.root WORKDIR [N=100] [SEED=7] [MAX_SHIFT=0] [MACROS_DIR]
```

混合データと元データに本物の macros を一時コピーで掛け、`mixReport.C` が「単一飛跡フィットが元の
どちらに寄るか」(xy 向きの角度)を stdout に出す。WORKDIR は実データ由来なのでリポの外に置く。
