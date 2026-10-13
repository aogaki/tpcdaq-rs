# geometry_xy_oracle — TPCReco GeometryTPC の出力(TODO/125)

合成ジオメトリ `tests/fixtures/geometry_xy_synth.dat`(`tools/geometry_xy_oracle/make_synth_geometry.py`
の出力 — 値はすべて架空)を TPCReco HIGS2026_online の `GeometryTPC` に読ませた結果。

再生成(TPCReco のソースはリポに入れない — パスで指すだけ):

```sh
make -C tools/geometry_xy_oracle fixtures TPCDAQ_TPCRECO_DIR=/abs/path/to/TPCReco-HIGS2026_online
```

| ファイル | 中身 | 使う側 |
|---|---|---|
| `segments.json` | 全線分 `[plane, section, strip, x0, y0, ux, uy, length]`(x0,y0 = reference point + Offset())、面ごとの `unit` / `pitchUnit`、`padPitch` / `stripPitch`、`rangeXY`(`GeometryTPC::rangeXY()` = TH2Poly の軸 = Get2D のビン範囲。`SetFloat(true)` で菱形 pad の頂点まで広がる)、`rangeXYCentreLines`(参考: 全線分の始点・終点の外接矩形 = InitTH2Poly が TH2Poly を作るときの初期値) | `tests/strip_xy_oracle.rs`(Rust `strip_xy` と 1e-9 で一致)、`ui/src/app/ws/strip-geometry-conformance.spec.ts`(Rust が書いた stripGeometry を TS 本番パーサで読んで 1e-9 で一致) |

`match_cases.json`(`GeometryTPC::MatchCrossPoint` の結果)は TODO/125 改訂 2 で撤去した — Event Display の
投影は U/V/W 三面一致をやめ、macros/analyzeUVW.C + makeTracks.C の移植(`ui/src/app/monitor/track-fit.ts`、
照合は `tools/macro_oracle/`)になった。合成ジオメトリの W 許容誤差の探針(W15–W18)は .dat と
`segments.json` を変えないために残してある。
