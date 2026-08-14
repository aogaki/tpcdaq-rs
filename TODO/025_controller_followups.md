# 025 — controller フォローアップ: asad_counts + run 番号手動設定 REST

**Status: OPEN(発注可)**
**仕様**: SPEC **v1.10** §8.1(`POST /api/run/next {token, next_run}` — run 中拒否・
正整数のみ・audit 記録)/ §9.2(counters nullable — 016 で 0 埋めにした 3 項目を null 化)。
出所 = archive/016_controller.md レビュー節のフォローアップ 2 件 + 逸脱③の確定処置。
**発注先想定**: implementer/**Sonnet**

## やること

1. **`Geometry` に AsAd 在庫アクセサ**(src/geometry.rs):
   `pub fn asad_inventory(&self) -> Vec<(u32, u32)>` — .dat に現れた (cobo, asad) の
   ソート済み・重複なし一覧。**lookup を使わず内部データから**(診断カウンタ
   unmapped_hit_count を汚さない — 016 が dump_tsv パースで回避した理由)。
2. **controller の期待フラグメント導出を置換**(src/controller.rs): dump_tsv 出力の
   先頭 2 列パース → `asad_inventory()` 呼び出しへ。TSV パースコードは削除。
3. **`POST /api/run/next {token, next_run}`**(SPEC §8.1 v1.10):
   - token 必須。run 実行中(Phase が Idle 以外)は拒否(409 相当)。`next_run` は正整数、
     それ以外は 400。成功で `tpcdaq_state.json` の next_run を**state.rs の
     atomic-rename 流儀のまま**更新し、audit レコード(action="run/next")。
     次の run/start から有効。
   - src/state.rs に `set_next_run` を追加(take_next_run と同じ書き込み規律。
     kill -9 耐性 = 一時ファイル + rename)。
4. **run_stop counters の null 化**(SPEC §9.2 v1.10、016 逸脱③の確定処置):
   src/logbook.rs の `Counters` の events_built / events_incomplete / late_fragments /
   overflow_frames / malformed を `Option<u64>` に(**null = 取得不能、0 と混同しない**)。
   frames はそのまま。controller は root-sink 由来 3 項目に None、GetStatus で取れた
   項目は Some。既存 015 の golden 照合はスキーマが変わるので **golden を更新**し、
   変更点(null 許容)を 015 の申し送りとしてテストコメントに明記。

5. **R-P2-13 logbook `recover_next_seq` の warn**(023 から移管): 末尾から遡って
   スキップした行数を内部で返す形にし(公開挙動不変)、**スキップ > 1 行なら warn**
   (「末尾 1 行だけ壊れる」前提 §9.1 からの逸脱の可視化)。

## テスト(TDD)

- geometry: mini 縮小合成(1 CoBo 1 AsAd)と合成 2-CoBo .dat で asad_inventory の
  期待集合を機械照合。unmapped_hit_count が呼び出しで**増えない**こと。
- controller 単体: 期待フラグメント導出が従来テストと同値。
- 統合(tests/controller_integration.rs): run/next 設定 → run/start が その番号で走る /
  run 中は拒否 / 0・負・非数は 400 / audit が記録される / token 無しは 401 相当。
  state.rs 単体: set_next_run の atomic rename + 再読込一致。
- logbook: run_stop レコードで None → JSON `null`、Some → 数値の直列化を golden 照合。
  末尾 2 行破損 → 復旧成功 + スキップ数 2(warn 経路の機械照合)。

## 受け入れ

- `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test` 全 green
  (既存無影響 — logbook golden の更新は上記 4 の範囲のみ)。
- ファイル所有権: src/{geometry.rs, controller.rs, state.rs, logbook.rs} +
  tests/controller_integration.rs(+ geometry 系テストの追記)。
  **decoder / receiver / graw_writer / tools/ に触らない**(023/024 が並行中)。
  発注書に無い設計分岐に出会ったら実装せず報告して戻る。
