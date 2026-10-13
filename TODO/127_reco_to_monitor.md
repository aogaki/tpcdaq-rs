# 127 — 飛跡の再構成を monitor(Rust、`src/reco/`)へ移し、結果を 0x04 Track で配る

**Status: DRAFT**(2026-10-13 起票 Opus。ユーザー「B(複数飛跡)で行きたいが、まずリファクタリングも」→ 範囲を絞って合意:
126(分割、完了)とこの 127。**挙動は変えない** — 125 改訂 2 の単一飛跡を、そのまま monitor に移す。設計の確定は Fable)

## 目的
- 125 は再構成(macros の analyzeUVW.C + makeTracks.C の移植)を UI(TypeScript)に置いた。理由は「閾値をブラウザごとに変えたい」
  だったが、macros 方式で定数を固定したので理由が消えた。
- 複数飛跡(130)では strip ごとの複数パルス・view ごとの直線探索・view 間の対応付けが加わり、今の数倍の規模になる。
  **1 イベント 1 回の計算・全画面で同じ結果・cargo のテストと macros オラクル・run 全体の統計(SPEC §5.5 の QA 集計)**のために、
  再構成は monitor の純関数モジュールに置く。SPEC §5.5 が元々予定していた形(QA 再構成は monitor 内、トラック重畳は 0x04)。

## 設計(Fable 確認待ち)
### `src/reco/`(純関数、IO なし)
- 入力: 面ごとの strip × bucket の生 ADC(**0x02 Uvw を作るのと同じ格子** — `DisplayConverter` の `grids`、section 合算済み)、
  strip 位置 s と pitch ベクトル(`strip_xy` から — 125 と同じ `s = pitchUnit·(x0, y0)`、section 番号が最小の線分)。
- 処理: `ui/src/app/monitor/track-fit.ts` を Rust に移す(**式・順序・定数は同じ** — 125 改訂 2 の節のとおり)。
- 出力 `RecoResult`: `status`(ok / 理由コード + 理由の文)、`tracks: Vec<Track>`(127 では 0 本か 1 本)、`points: Vec<Point>`
  (帯内ヒットを直線上に置いた点 — `x, y`(mm)、`z`(time bucket)、`q`、`view`、`track`(何本目か))、view ごとのヒット数など、
  UI の状態表示に要る数。`Track` = anchor・方向・φ・tanθ・端点(2–98 % と最外)・xy の長さ・電荷。

### monitor
- built event ごとに 0x02 を作るところ(`monitor/sub.rs` の Ingest → `DisplayConverter::uvw`)で、**同じ格子**から `reco` を呼び、
  0x04 を live 側(drop-oldest + カウント)に流す。0x02 を作らないイベントでは呼ばない(コストは 1 イベント 0.1 ms 程度の見込み — 実測を結果節に)。
- 0x04 は `subscribe` の `uvw` ストリームに含める(新しいストリーム名は作らない)。

### WS 0x04 `Track`(SPEC §10.2 に追加 — §10.1 の 13 バイトヘッダ、イベント系 = run・event・incomplete)
- 本文(すべて LE):
  `u8 status`(0 = ok、1 以上 = 理由コード)、`u16 reasonLen` + UTF-8 の理由の文、
  `u8 nTracks`、各 track に `f32 × N`(anchorX, anchorY, dirX, dirY, tanTheta, phiDeg, lStart, lEnd, lStartF, lEndF, lengthXY, charge)、
  `u16 nViewHits[3]`、`u32 nPoints`、各点に `f32 x, f32 y, f32 z, f32 q, u8 view, u8 track`。
  **最初から飛跡のリスト**にする(130 で形式を変えないため)。
- エンコーダは `monitor/ws.rs`、§10.4 の適合(`ws_proto_sample` に 0x04 のサンプルを足し、UI の本番デコーダで読む)。

### UI
- `track-fit.ts`(計算)を撤去し、0x04 を `ws/wire.ts` でデコードして XY / XZ / YZ に詰める(ビン・範囲・状態表示の文は今と同じ)。
  stripGeometry は軸の範囲(rangeXY)とビン幅に引き続き使う。
- 0x02 と 0x04 は同じイベントから作るが、live キューは drop-oldest なので**イベント番号が違うことがある** → 0x04 の event# が
  表示中の 0x02 と違えば `≠ UVW` を出す(077 の AsAd パネルと同じ流儀)。

### オラクル
- 125 の macros オラクル(`tools/macro_oracle/` の JSON、env-gated)を **Rust のテストに移す**(`TPCDAQ_MACRO_ORACLE` /
  `TPCDAQ_MACRO_ORACLE_GEOMETRY`)。許容は 125 と同じ。TS 側のオラクル spec は撤去。
- 合成テスト(既知の直線、mini と ELITPC の角度、鏡像対策の非対称直線、halo・帯・ok でない理由、3×3 と macros の閉じた式)も Rust に移す。

### SPEC
- §5.4(投影は monitor が計算して 0x04 で配る)、§5.5(QA 再構成モジュール = `src/reco/` が実体)、§10.2(0x04)、§10.3(`uvw` に 0x04 を含む)。

## 受け入れ
- Rust: macros オラクル(run70 の 201 イベント)で 125 と同じ一致(ok / not ok・ヒット集合・端点・方向)。合成テスト一式。0x04 のエンコーダのオフセット固定テスト。
- 適合: `ui/run_ws_conformance.sh` で 0x04 が UI の本番デコーダで読める。
- UI: 表示は 125 と同じ(状態表示の文・パネル)。`≠ UVW`。
- ゲート: cargo fmt / clippy / test、UI ng test / ng build、適合スクリプト。
- ELITPC に反映して、ユーザーの目視で 125 と同じ絵になること。

## 結果
(未記入)
