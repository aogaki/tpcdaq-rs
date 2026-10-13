# geometry_xy_oracle — TPCReco GeometryTPC の出力(TODO/125)

合成ジオメトリ `tests/fixtures/geometry_xy_synth.dat`(`tools/geometry_xy_oracle/make_synth_geometry.py`
の出力 — 値はすべて架空)を TPCReco HIGS2026_online の `GeometryTPC` に読ませた結果。

再生成(TPCReco のソースはリポに入れない — パスで指すだけ):

```sh
make -C tools/geometry_xy_oracle fixtures TPCDAQ_TPCRECO_DIR=/abs/path/to/TPCReco-HIGS2026_online
```

| ファイル | 中身 | 使う側 |
|---|---|---|
| `segments.json` | 全線分 `[plane, section, strip, x0, y0, ux, uy, length]`(x0,y0 = reference point + Offset())、面ごとの `unit` / `pitchUnit`、`padPitch` / `stripPitch`、`rangeXY`(全線分の始点・終点の外接矩形)、`rangeXYTh2Poly`(参考: `GeometryTPC::rangeXY()` = TH2Poly の軸。菱形 pad の角まで含むので strip に直交する向きに pad_size/2 広い) | `tests/strip_xy_oracle.rs`(Rust `strip_xy` と 1e-9 で一致) |
| `match_cases.json` | `GeometryTPC::MatchCrossPoint(U, V, W, radius = 2.0)` の結果。行 = `[set, uStrip, uSection, vStrip, vSection, wStrip, wSection, match, x, y]`(`match = false` なら x,y は null)。`set`: `"r"` = 乱数(mt19937 seed 20261013)の (u,v,w) 1000 組、`"b"` = 境界集合(U×V 交点に最も近い W ±2 本 / 線分の有効範囲の端 ±1e-3 mm)。各組は存在する section の全組み合わせ | TODO/125 レーン C(UI `matchCrossPoint` との照合) |

`numericalTolerance` は TPCReco `Utils::NUMERICAL_TOLERANCE`(HIGS2026_online = 1e-5)。合成ジオメトリの
W15–W18 は許容誤差の内外を 1 本ずつ突く探針で、`["b",7,0,8,1,15,0,true,…]` / `…,16,0,false,…`(始点側)、
`…,17,0,true,…` / `…,18,0,false,…`(終点側)になる。
