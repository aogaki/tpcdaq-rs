# 026 — monitor(Rust): root-sink PUB 購読 → 表示変換 → WS 配信

**Status: OPEN(発注可)**
**仕様**: SPEC **v1.10** §5.3(PUB ワイヤ — 受け側)/ §5.4(monitor の責務)/
§10(WS プロトコル — 本ユニットの核)/ §3.2(WS 9000、PUB 47004)/ §12-10(WS 適合性)
**依存**: 022(PUB ワイヤ実装 — tests/root_sink_monitor_pub.rs の named struct パーサが
本番パーサの先行形)/ 024(status 11 キー = pending_events 込み)
**発注先想定**: implementer/**Opus**(tokio + WS の並行設計が残る)

## 責務(§5.4 — これだけ。集計はしない)

root-sink PUB を購読 → 表示用に変換 → WS で全クライアントへ配る。**モニタ系 = 落として
よいが silent にしない**(ドロップ・ギャップは全部数えて可視化)。保存系には一切触れない。

## やること

1. **`src/monitor.rs` + `src/bin/monitor.rs`**(controller/receiver bin の流儀):
   - **SUB**: config の endpoint(既定 `tcp://127.0.0.1:47004`)へ connect、購読は全部。
     §5.3 v1.10 の 3 種(status / hist_snapshot / built_event)を named struct で
     デコード(**tests/root_sink_monitor_pub.rs のテスト用パーサを本番モジュールへ昇格**し、
     テストは本番パーサを使う形に一本化してよい — 二重定義を残さない)。
   - **ギャップ計数**(§5.4): エンベロープ seq の飛びを `monitor_gaps` として累積
     (PUB リンクは run リセット無し単調 — §5.3 v1.10)。未知 kind は数えて無視(前方互換)。
   - **ジオメトリ**(config 必須): ロード時に素性ログ(root_sink.cxx と同じ項目)。
     **R-P2-13 の可視化フック結線**: 変換中に `unmapped_hit_count` が増えたら初回だけ warn
     (logged-once 方式)。
2. **表示変換**(§5.4。純関数モジュールにして単体テスト — IO 非依存):
   - built_event → **Uvw ×3**(0x02): 面毎 `nStrips×512` の u16 グリッド、
     `idx=(strip-1)*nBuckets+bucket`、生 ADC。同一ビンへの複数チャンネル(セクション
     合流)は **saturating add**(u16 天井で clamp — 表示専用、正値はヒスト/monitor.root)。
     FPN/Aux/Unmapped は入れない。
   - built_event → **Waveforms**(0x03): (cobo,asad) 毎 `4×68×512` u16 dense、aget-major・
     raw ch 順・**FPN 込み・減算なし**(R13)。
   - hist_snapshot → **Histo1d/Histo2d**(0x10/0x11): f64→f32、id 1–9(§5.2 と一致)、
     軸: 2D x=[1,N+1) y=[0,512)、1D [0,4096)。
   - ヘッダ 13 B(§10.1): magic 'T''P'、version=2、flags bit0 = incomplete、
     runNumber、eventNumber(ヒスト・status 系は 0)。
3. **WS サーバ**(§10、axum の WebSocket — 016 で導入済みの axum を使う。ポート既定 9000):
   - JSON: `meta`(接続時 + run 変化時 — nBuckets=512、planes=max_strip、geometry=
     設定パス名、anglesDeg=`HeaderScalars::angles_deg`(無ければ null)、detector=設定名、
     cobos、run)/ `status`(1 Hz — §5.3 の status をそのまま + `{monitorGaps, clients,
     wsDropped}`)/ `run`(state 遷移検知時)/ `subscribe`(C→S、既定 = waveforms 以外 ON)。
   - **送信キュー**: live(0x02/03/10/11)= 有界 + **drop-oldest + `ws_dropped` 計数**、
     JSON 制御 = reliable(詰まったらそのクライアントを切断 + ログ — 黙って無限に溜めない)。
   - クライアント管理: subscribe フィルタ per client。切断は静かに掃除 + clients 更新。
   - monitor は **REP を持たない**(run 制御の外 — 016 の決定を踏襲。純コンシューマ)。
4. **WS ワイヤエンコーダ**(§10.1/10.2)は IO 非依存の純モジュール +
   **`src/bin/ws_proto_sample.rs`**(§10.4-1: 全メッセージ型を既知値でエンコードし
   `u32 長さ + ペイロード` 連結をファイルへ)。**TS 側検証(§10.4-2/3)は UI ユニット
   (027)で実装** — 本ユニットは Rust 側レイアウトテスト(§10.4-4: バイトオフセット
   assert)まで。
5. **config**: `[monitor]` — `sub_endpoint` / `ws_listen`(既定 0.0.0.0:9000)/
   `geometry` / `live_queue`(既定 64)。§3.2 既定 + 上書き、パースエラーは起動失敗。

## テスト(TDD)

- 単体(純関数): Uvw 変換(セクション合流 saturating add / FPN 除外 / idx 順)、
  Waveforms(aget-major 順・FPN 込み)、Histo 変換(f64→f32・軸)、13 B ヘッダの
  バイトオフセット assert(§10.4-4)、ギャップ計数(連続 / 飛び / 未知 kind)。
  オラクルは手計算 + 022 の Rust 独立再計算と同素材。
- 統合(実 ZMQ + 実 WS、全ポート動的): テスト内 PUB(rmp-serde で §5.3 メッセージを合成)
  → monitor → WS クライアント(tokio-tungstenite はテスト依存に追加可)で
  meta→status→uvw→histo を受けて機械検証(§10.4-5 の probe の先行形)。
  subscribe で waveforms が既定 OFF・ON にすると届く。遅いクライアント模擬で
  drop-oldest + wsDropped 増加 + status/JSON は落ちない。
- E2E(env `TPCDAQ_ROOT_SINK_BIN` gate): graw_replay → …(022 の流儀)… → root_sink
  実バイナリの PUB → monitor → WS で実データが流れる(mini 実 graw があれば
  `TPCDAQ_REAL_GRAW` も)。
- `ws_proto_sample` が決定的出力(同一入力 → 同一バイト)であることの回帰。

## 受け入れ

- `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test` 全 green
  (既存無影響。tests/root_sink_monitor_pub.rs のパーサ昇格はテスト green 維持が条件)。
- ファイル所有権: src/{monitor.rs}、src/bin/{monitor.rs, ws_proto_sample.rs}、
  src/config.rs([monitor] 追記)、src/lib.rs(1 行)、Cargo.toml(WS 依存追加)、
  tests/monitor_*.rs(新規)、tests/root_sink_monitor_pub.rs(パーサ昇格のみ)。
  **tools/ と他コンポーネント(decoder/receiver/graw_writer/controller)に触らない**。
  発注書に無い設計分岐に出会ったら実装せず報告して戻る。
