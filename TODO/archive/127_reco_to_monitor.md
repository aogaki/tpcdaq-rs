# 127 — 飛跡の再構成を monitor(Rust、`src/reco/`)へ移し、結果を 0x04 Track で配る

**Status: COMPLETED**(2026-10-13 起票 Opus。ユーザー「B(複数飛跡)で行きたいが、まずリファクタリングも」→ 範囲を絞って合意:
126(分割、完了)とこの 127。**挙動は変えない** — 125 改訂 2 の単一飛跡を、そのまま monitor に移す。Fable 条件付き承認 2026-10-13 — 条件は下に反映済み)

## 目的
- 125 は再構成(macros の analyzeUVW.C + makeTracks.C の移植)を UI(TypeScript)に置いた。理由は「閾値をブラウザごとに変えたい」
  だったが、macros 方式で定数を固定したので理由が消えた。
- 複数飛跡(130)では strip ごとの複数パルス・view ごとの直線探索・view 間の対応付けが加わり、今の数倍の規模になる。
  **1 イベント 1 回の計算・全画面で同じ結果・cargo のテストと macros オラクル・run 全体の統計(SPEC §5.5 の QA 集計)**のために、
  再構成は monitor の純関数モジュールに置く。SPEC §5.5 が元々予定していた形(QA 再構成は monitor 内、トラック重畳は 0x04)。

## 設計(Fable 条件付き承認、2026-10-13 — 条件反映済み)

### `src/reco/`(純関数、IO なし)
- 入力: 面ごとの strip × bucket の生 ADC(**0x02 Uvw を作るのと同じ格子** — section 合算済み)と、`RecoGeometry`(strip 位置 s と pitch ベクトル)。
- 処理: `ui/src/app/monitor/track-fit.ts` を Rust に移す。**f64・同じ演算順・同じ定数**(125 改訂 2 の節のとおり)。端点のための並べ替えは
  `l` → `q` の安定ソート(TS と同じ)。
- 作業バッファ(ヒット・点)は `Reco` 構造体に持って使い回す(`DisplayConverter` と同じ流儀、イベントごとの heap 確保をしない)。
- `RecoGeometry` は**起動時に 1 回** `strip_xy::StripGeometry` から作る(s = `pitchUnit·(x0, y0)`、section 番号が最小の線分)。
  今の Hub は stripGeometry の JSON 文字列しか持たないので、`strip_geometry_at_startup` で構造体も保持する。
  **stripGeometry が unavailable のときは reco を呼ばず 0x04 も送らない**(UI は既存の unavailable 表示で足りる)。

### monitor のフック
- Ingest スレッドで、0x02 を作った直後に**同じ格子**で reco を呼ぶ(`DisplayConverter` に格子の読み取り口 `grids()` を足す)。
  0x02 を作らないイベントでは呼ばない。順序は単一スレッドなので 0x02×3 → 0x04。
- 0x04 は **live の broadcast(バイナリ枠、drop-oldest + カウント)**で運ぶ。**JSON テキスト経路(reliable、詰まると切断)には載せない**。
  `subscribe` の `uvw` ストリームに含める(新しいストリーム名は作らない)。

### WS 0x04 `Track`(SPEC §10.2)
- §10.1 の 13 バイトヘッダ(イベント系 = run・event・incomplete)+ **本文 = UTF-8 JSON**(§10.2 で唯一。130 で形が増えるため —
  N 本・頂点・親子が増えても版上げ・オフセット試験・適合の作り直しが要らない。f64 のまま届く)。
- 本文の例(1 本見つかったとき。数値は例):
```json
{"ok":true,"reason":"","hits":[72,40,34],"unplaced":0,
 "tracks":[{"phiDeg":2.7,"dir":[0.998,0.047,-0.031],"start":[-53.1,-7.9,26.4],"end":[53.0,-3.0,23.1],
            "startF":[-53.5,-8.0,26.4],"endF":[53.6,-3.0,23.0],"length":106.2,"lengthF":107.2,"xyLength":106.1,"charge":412345.0}],
 "points":[[-52.9,-7.9,152.0,5480.0,0,0],[-51.4,-7.8,151.6,5512.0,0,0]]}
```
  - `start` / `end` などの z と `dir` の z 成分は **mm(macros 既定の換算 0.173592 mm/cell)**、`points` の 3 つ目 `t` は **time bucket**。
    `points` = `[x, y, t, q, view, track]`(view 0/1/2 = U/V/W、track = 何本目か)。`hits` = view ごとの帯内ヒット数。
    `unplaced` = 直線上に置けなかった(|pitch·dir| < MIN_PROJ の view の)ヒット数。
  - 直線が引けないとき: `{"ok":false,"reason":"W has 2 strip hits after the halo cut (needs 4)","hits":[..],"unplaced":0,"tracks":[],"points":[]}`
    (理由の文は 125 と同じ)。
  - UI は未知キーを許す(`meta` と同じ)。壊れた JSON は DecodeError の理由 `bad-json` で数える。

### UI
- `track-fit.ts`(計算)を撤去し、0x04 を `ws/wire.ts` でデコードして `ws-client` の `track` signal に入れる。
- monitor-view は **`shownUvw` と同じ tick で `shownTrack` を取る**(ラベルと絵を揃える)。
- XZ / YZ の Z 軸の bucket 数は **`meta.nBuckets`** から取る(0x04 だけで描けるように)。stripGeometry は rangeXY とビン幅に引き続き使う。
- 0x04 の event# が表示中の 0x02 と違えば **`≠ UVW` の印を出す。点は描く**(077 と同じ)。
- `projectionBlocker` は「stripGeometry がある・ビンが退化していない」だけに縮める。状態表示の文は 125 と同じ + `waiting for track (0x04)`。
  ヒント文を「computed by monitor」に。

### オラクルとテスト(1 実装 — TS の計算とオラクルは撤去)
- 125 の macros オラクル(env-gated、`TPCDAQ_MACRO_ORACLE` / `TPCDAQ_MACRO_ORACLE_GEOMETRY`)を Rust のテストに移す。
  **production(s を stripGeometry から)と csv-s(channel_map.csv の s)の 2 モードと許容(TOL)をそのまま写す**。
  オラクル JSON は 100 MB 級なので `cargo test --release` を推奨と書く。`tools/macro_oracle/README.md` の手順 3 を cargo に書き換える。
- 合成テスト(`track-fit.spec.ts` の 39 本)を Rust に移し、**結果節に TS の `it` 名 ↔ Rust のテスト名の対応表**を付ける。
- 鏡像対策は分ける: Rust = 点の座標と象限(非対称な直線)、UI = 既知の 1 点が正しいビンに入る(`xy-projection.spec.ts` に残す)。
- 0x04 のエンコード(ヘッダ + JSON)の単体テストと、§10.4 の適合(`ws_proto_sample` に 0x04 のサンプル → UI の本番デコーダ)。

### SPEC(マージ後に主対話が入れる — v1.45)
- §4.4: v1.43 の「UI の Event Display 投影」→「`src/reco/`(monitor)。UI は 0x04 の点をビンに詰めるだけ」。
- §5.4: 投影は monitor が計算して 0x04 で配る。
- §5.5: アルゴリズム項を「simplefit 移植、Python が正」→「**macros/analyzeUVW.C + makeTracks.C の移植、本物の macros が正**
  (§4.5 の二重実装 = C++/ROOT 側が macros)」に差し替え。1 トラック仮説の設定値化は足さない(0x04 をリストにすることで構造的に満たす)。
  **TODO/074 は superseded**(QA ヒスト id 20 番台の部分だけ 130 以降に残す)。
- §10.2: 0x04 Track(ヘッダ + JSON 本文)、「0x04 を起票する」注記を解消。§10.3: `uvw` ストリームに 0x04 を含む。§10.4: サンプルに 0x04。履歴 v1.45。

## レーン
- **A(Opus)**: `src/reco/` + Rust のオラクル・合成テスト + monitor のフック + 0x04 エンコーダ + `ws_proto_sample` のサンプル。
- **B(Sonnet)**: UI のデコード・描画・テスト + 適合 spec。**上の JSON 例を契約として A と並行**。
- 各レーンの報告に必須: 変更ファイル、発注書の各項目に対応するテスト名、ゲート結果、**実行できなかった操作**(無ければ「なし」)。

## 受け入れ
- Rust: macros オラクル(run70 の 201 イベント、2 モード)で 125 と同じ一致。合成テスト一式(対応表つき)。0x04 エンコードの単体テスト。
- 適合: `ui/run_ws_conformance.sh` で 0x04 が UI の本番デコーダで読める。
- UI: 表示は 125 と同じ(状態表示の文・パネル)、`≠ UVW`、`meta.nBuckets`。
- ゲート: cargo fmt / clippy / test、UI ng test / ng build、適合スクリプト。
- ELITPC に反映して、ユーザーの目視で 125 と同じ絵になること。**反映前後で status の `wsDropped` / `monitorGaps` の増え方が変わらない**こと。

## 結果(2026-10-13)

### 実装とマージ
- レーン A(Opus)e24122b → merge ac12906: `src/reco/{mod.rs, tests.rs}`(track-fit.ts の移植、f64・同じ演算順、`total_cmp` の安定ソート)、
  `RecoGeometry`(起動時 1 回)、monitor のフック(`DisplayConverter::grids()`、0x02×3 → 0x04、stripGeometry が unavailable なら reco も 0x04 も無し)、
  0x04 エンコーダ(13 B ヘッダ + UTF-8 JSON、live の broadcast、`uvw` 購読に含む)、サンプル(5 通目、run 7 / event 44、本文 = 発注書の例そのもの)、
  `tests/reco_macro_oracle.rs`(env-gated、production / csv-s の 2 モード)、`tools/macro_oracle/README.md` の手順 3 を cargo に。
- レーン B(Sonnet)d6424a0 + 8b3d045 → merge 5734eea: UI の計算(track-fit.ts と spec・フィクスチャ)を撤去、`wire.ts` に 0x04 のデコード(`bad-json`)、
  `ws-client.track`、monitor-view は `shownTrack` を `shownUvw` と同じ刻みで、Z の bucket 数は `meta.nBuckets`、`≠ UVW`、`unplaced` の文を 127 の意味
  (「N strip hits not placed on the line (view nearly parallel to the track)」)に。適合 spec に 0x04。
- 照合: `git show --stat`(A は ui に、B は src / tests に触れていない)と各項目のテスト名で確認してから受け入れた。
- 発注書からの逸脱(承認): bucket 数がペデスタル窓に足りないとき、TS は計算しない(null)だったが Rust は `ok:false` + 理由の文の 0x04 を送る
  (monitor では N_BUCKETS = 512 固定なので起きない)。stripGeometry に線分の無い strip のヒットは `unplaced` に入れず、warn 1 回 + SUB スレッド停止時のログ
  `track_no_segment_strips` で数える(JSON キーは足していない)。meta 未着・ok なのに飛跡なしの文は UI 側で補った。

### TS → Rust のテスト対応(39 本)
- レーン A の報告の対応表のとおり(定数・z 換算・stripHits 5 本・RecoGeometry・帯選択 2 本・3×3 の 3 本・fitTrack 6 本・既知の直線 mini / ELITPC 各 6 本・
  halo / 帯外 / 理由の文 3 本・ドリフト方向・incomplete(sub のテスト)・bucket 不足(逸脱)・線分なし)。

### オラクル(run70 の 201 イベント、`cargo test --release --test reco_macro_oracle`、主対話が main で実行 7 / 7 green、0.49 s)
- ok / not ok 201 / 201(not ok = event 192、125 と同じ理由)、帯内ヒット集合の食い違い 0 / 20,960。
- 最大差 production / csv-s: z 1.94e-6 mm、q 4.88e-8、端点 4.65e-4 / 3.04e-6 mm(event 90)、方向 4.81e-8、length 4.35e-6 mm、charge 4.69e-8 — 125(TS)と同じ。
- 変異試験: QUANTILE 0.025 で 2 本が落ちる(125 と同じ)。

### コスト(実データ、production)
- release: `Reco::run` 平均 217 µs(最大 289 µs)、0x04 のエンコード込み 225 µs。最大の 0x04 は 13,233 B。debug 0.80 ms。

### ゲート(main 5734eea → SPEC c7d1647、Mac)
- cargo fmt / clippy clean、**cargo test 1069 passed / 0 failed / 1 ignored**(着手前 1018、+51)。
- UI ng test **392 passed / 10 skipped**(env-gated)、**`ui/run_ws_conformance.sh` 402 passed / 0 skipped**(0x04 の適合を含む)、ng build OK。

### ELITPC への反映(2026-10-13 08:28Z、ユーザー了承「今すぐ反映」、run 71 は継続)
- monitor だけ入れ替え(bundle → `cargo build --release --bin monitor` → SIGINT → 再起動)、UI を差し替え(旧版 `/opt/tpcdaq/ui.prev127`)。
- Node の WS クライアントで確認: 5 s で 0x02 52 通・0x04 17 通(全件 ok、例: hits 17/10/10、φ 0.35°、xy 長 27.1 mm、点 37、3,476 B)。
- **反映前後の取りこぼし**: 前 08:25:05 → 08:26:06 で monitorGaps 0 → 0・wsDropped 0 → 0、後 08:28:55 → 08:29:56 で 0 → 0・0 → 0(どちらも 60 s、4.7 Hz)。
  monitor の CPU 2.3 %、RSS 14 MB、ログに WARN / ERROR なし。
- **ユーザーの目視: 「同じ絵ではもちろんないですが、良い絵です」(2026-10-13)**(別のイベントなので絵そのものは違う — 表示として良好)。

### SPEC
- v1.45(c7d1647): §4.4・§5.4・§5.5(アルゴリズム = macros の移植、本物の macros が正、実体 `src/reco/`)・§10.2(0x04)・§10.3(`uvw` に 0x04)・
  live キューに 0x04・履歴。TODO/074 は superseded。

### 実行できなかった操作
- レーン A・B: worktree 隔離の権限チェックで複合コマンドが拒否 → Edit / 単純なコマンドでやり直し、未実行は残っていない。
