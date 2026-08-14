# 021 — 同 run 実データ値一致(§12-3 v1.8 ③のクローズ)+ graw_replay マージ送出

**Status: COMPLETED**
**仕様**: SPEC v1.8 §12-3 ③(同 run ペアの全イベント値一致)、§6.4(runId)、§12 末尾(graw_replay)
**背景**: ユーザーが 2026-08-14 に **同一 run のペア**を取得 —
`reference/exp_data/2026/` = `CoBo0_AsAd{0..3}_2026-08-11T07:47:37.{043..051}_0000.graw`(4 本)+
その実機 grawToEventTPC 変換 `PEventTPC_2026-08-11T07-47-37.051_0000.root`。
020 の残タスク「同 run ペアでの実データ値一致」を閉じる。
**担当**: Fable 直接(020 のレビュー延長 + リプレイ設計)

## やること

1. `src/decode.rs`: `peek_event_idx`(peek_asad と同一ゲートの eventIdx 覗き見)。
2. `src/bin/graw_replay.rs`: **複数ファイル対応**(位置引数 2 個目以降を全部ファイルに)。
   - 1 ファイル = 従来のバイトそのままチャンク送出(§12-2 の土台を崩さない)。
   - 複数 = **eventIdx 昇順 k-way マージ送出**(同 idx 内は引数順 = AsAd 昇順で渡す)。
     実機ワイヤ(DataRouter が受けた形)の再現。ファイル逐次ではイベントビルダの
     タイムアウトを正しく通せない。制御フレーム(eventIdx なし)は遭遇時に即時送出。
     ストリーミング(常駐 = 各ファイル先頭の 1 フレームのみ)。末尾の切れ端は警告。
     **D1 デモ(ELITPC 4 本組リプレイ)にも必須の恒久機能。**
3. `tools/root_sink`: `--run-id N`(EventInfo.runId 上書き。0 = 従来どおり壁時計)。
   実データ照合と、P4 controller が正式 run TS を配る経路(SPEC §6.4 実装注記)の受け口。
4. `tests/elitpc_pevent_e2e.rs`(env-gated、全 env 揃わなければ skip):
   graw 4 本 → graw_replay マージ(ペーシング必須 — sink の chargeMap + ZLIB が下流
   ボトルネック、receiver は never-stop なので全速では有界キューが溢れる。溢れは
   overflow_frames==0 の assert で検出)→ receiver → decoder → root_sink(pevent、
   実ジオメトリ、--run-id はファイル名 TS 由来)→ **compare_pevent で実機ファイルと
   全イベント突き合わせ**。カウンタ(fragments=15,408 / items=2,145,779,712 /
   complete=3852 / incomplete=0 / duplicate=0 / out_of_range=0)も §12-1 実測普遍値で固定。

## 受け入れ

- 単体(peek_event_idx 3 本 + parse 1 本)+ 統合(マージ順序・内容の機械照合 1 本)green。
- 既存 graw_replay テスト・全ゲート(fmt/clippy/cargo test/make test/test-root)無影響 green。
- **同 run E2E green = compare_pevent exit 0**(SPEC §12-3 v1.8 ③ クローズ)。


## 結果

実行環境: macOS(Darwin 25.5.0)、release ビルド、2026-08-14。担当: Fable 直接
(015/017 の implementer 並列発注と同時進行)。

### 核心: 同 run 実データ値一致(SPEC §12-3 v1.8 ③)— **クローズ**

`cargo test --release --test elitpc_pevent_e2e`(env 4 つ + 既定 40 Mbps)→ **1 passed**:

- graw 4 本組(4,295,503,872 B)を eventIdx マージで 859.3 s(40 Mbps どおり)送出 →
  receiver → decoder → root_sink(pevent、実 ELITPC ジオメトリ、--run-id 20260811074737)
- counters: batches=15,408 / **events_complete=3852 / incomplete=0 / late=0 /
  duplicate=0 / charge_keys_out_of_range=0 / frames_outside_geometry=0 /
  receiver overflow=0** / fatal なし
- channels_without_strip=23,112 = 6 ch/イベント(1024 信号 ch − 1018 strip と正確に整合)
- **compare_pevent: `compared 3852 events, 0 differences`**(849.9 s、EventInfo 全フィールド +
  chargeMap 全 key/値の double 厳密一致、許容差 0)
- 我々の run0001.root = 8,168,932,525 B vs 実機 11,047,705,553 B — **中身同一**なので
  差は ROOT 6.36/6.08 の書き込み効率のみと確定

### 単体・統合(新規分)

- `peek_event_idx` 単体 3 本 + graw_replay parse 複数ファイル 1 本 → green
- graw_replay マージ統合(`multiple_files_are_interleaved_by_event_idx`: ctrl 最優先 +
  (eventIdx, 引数順) 昇順 + 長さ不揃い、期待バイト列を手組みで機械照合)→ green
- test_pevent に --run-id 上書きテスト追加 → **101 passed**(env 有 122)
- リポ全体: fmt / clippy -D warnings / cargo test **29 バイナリ全 ok**(既存無影響。
  2022 側 `TPCDAQ_REAL_GRAW_DIR` 回帰も新データ配置後に green 再確認済み)

### 変更ファイル

src/decode.rs(peek_event_idx + 単体 3)/ src/bin/graw_replay.rs(複数ファイル k-way マージ)/
tests/graw_replay_tool.rs(マージ統合)/ tools/root_sink/{root_sink.cxx,root_recorder.hpp}
(--run-id)/ tools/root_sink/test_pevent.cxx(+3)/ tests/elitpc_pevent_e2e.rs(新規)/
docs/SPEC_ja.md §12 graw_replay 追記

### これで閉じたもの

- 020 の残タスク「同 run ペアでの実データ値一致」→ **解消**(残りは TPCReco 再配布許諾のみ)
- 「我々の run.root = 先方オフライン変換出力」が実データで実証 — D1 デモの主張
  「何も壊さず、出てくるものは今と同じ」の根拠が値レベルで揃った
