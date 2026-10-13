# macro_oracle — macros/analyzeUVW.C + makeTracks.C を正とする照合(TODO/125 改訂 2 → TODO/127)

Event Display の直線トラック(monitor の `src/reco/`。TODO/127 で UI の `track-fit.ts` から移した)は、
ユーザーの解析マクロ `macros/analyzeUVW.C`(0〜4)と `macros/makeTracks.C`(1〜5)の移植。
ここはその照合元を作る道具。

**実データを使う — 出力 JSON はリポに入れない**(`reference/` か scratch に置く)。

```sh
# 1. 本物のマクロを一時ディレクトリで実行し、JSON に書き出す(macros/ は書き換えない)
tools/macro_oracle/run_macro_oracle.sh RAW_raw.root /tmp/oracle.json [MAX_EVENTS] [MACROS_DIR]
#    MACROS_DIR = channel_map.csv のある macros/(.gitignore 済み — worktree では主チェックアウトの macros/)

# 2. 同じ検出器の stripGeometry を Rust の本番エンコーダで書く
cargo build && target/debug/ws_proto_sample --out /tmp/ws.bin \
  --geometry /path/to/geometry.dat --strip-geometry-out /tmp/strip_geometry.json

# 3. Rust の env-gated テスト(無ければ skip)。オラクル JSON は 100 MB 級なので --release を推奨
#    (run70 の 201 イベント・106 MB で読み込み release 0.4 s / debug 4 s。所要は release で計る)。
#    --nocapture で一致件数・最大差・1 イベントの所要を表示する。
TPCDAQ_MACRO_ORACLE=/tmp/oracle.json TPCDAQ_MACRO_ORACLE_GEOMETRY=/tmp/strip_geometry.json \
  cargo test --release --test reco_macro_oracle -- --nocapture
```

| ファイル | 役割 |
|---|---|
| `run_macro_oracle.sh` | macros/ の analyzeUVW.C・makeTracks.C・channel_map.csv と raw を一時ディレクトリにコピーして実行(既定 100 mbar)→ `dumpMacroOracle.C` |
| `dumpMacroOracle.C` | raw の `adc`(channel_map の行順、FPN は除く)・uvw の `slView/slZ/slS/slQ`・tracks の全ブランチ・channel_map を JSON に |

照合の中身と許容は `tests/reco_macro_oracle.rs` の冒頭(s を stripGeometry から取る production と、
channel_map.csv の s に置き換えて移植そのものを見る csv-s の 2 通り)。
