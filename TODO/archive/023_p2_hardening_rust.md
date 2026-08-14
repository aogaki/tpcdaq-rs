# 023 — P2 レビュー改修: Rust 異常系を decoder 水準に揃える

**Status: COMPLETED**(2026-08-14 implementer/Opus(worktree)→ Fable レビュー PASS →
main 取り込み)

## 結果

- **実装**(7 ファイル +1057/-60): receiver 送信経路の decoder パターン移植
  (SendOutcome 3 値 / SNDTIMEO + EAGAIN リトライ / 送る前に abandon 確認 /
  `messages_abandoned`・`encode_errors`・`send_errors` を metrics 露出 / Reset・畳み込みで
  sender スレッドを **join**)+ framer リセット初回 warn + decoder/graw-writer の
  poisoned Mutex 即 Error + graw-writer 異常系 4 経路(EOS run 不一致・期待外ソース EOS は
  **数えて捨てる = run を閉じる材料にしない**、decode_errors、heartbeats_in)+
  decoder `heartbeats_out`/`heartbeats_abandoned` + doc 修正。
- **テスト(worktree でエージェント実行 + Fable がゲート・コード読みで裏取り、2026-08-14、
  macOS Darwin 25.5.0)**: fmt / clippy クリーン、`cargo test` 281 passed / 0 failed
  (新規 12。TDD 赤確認済み — poisoned テストは旧実装で Running≠Error の失敗を実測)。
  実データ gate: p2_e2e 2 件(13.6 s)+ intake 9 件 green、ELITPC 2022 構造オラクル
  2 件 green(172 s)。**検収実測: 下流不在で Stop 609 µs / Reset 1.000 s、
  messages_abandoned=2、スレッド join 確認**。main 取り込み後の統合ゲートは CURRENT.md
  記載値。スキップ: elitpc_pevent_e2e(env パス未特定 — 本ユニット無関係)。
- **レビュー(Fable)**: 逸脱 4 件すべて受理 —
  ①**`Stop` では打ち切らない**(発注書の誤り。Stop = §1.3 v1.6 の強制 EOS 経路であり、
  ここで abandon すると一時的な詰まりで EOS を捨てる。打ち切りは Reset と畳み込みのみ —
  decoder と同型)②join は do_reset/Drop、Reset 応答 ~1 s(SNDTIMEO 分、検収 2 s 内)+
  送信待ち 100 ms 頭打ち ③`batches` を「送れたバッチ」に厳格化(v1.4 契約の整合)
  ④EOS run 不一致は計上のみで Error ラッチしない(SPEC §7 の Error 事由列挙に整合。
  不一致 EOS は run を閉じない)。
- **申し送り**: (a) Stop 直後・下流不在のまま Start し直すと前 run の送信スレッドが
  blocked のまま残る経路が理論上ある(修正前から同じ。Start 時 abandon は正常系 EOS を
  落とし得るため未着手)— controller は Stop 後 Reset まで送る(016)ので実運用では
  踏まない。(b) poisoned 時の respond() metrics は `{}`(状態は Error で可視。
  `PoisonError::into_inner` で読ませる改善余地)。
**仕様**: SPEC **v1.10** §1.4-5(役割非対称 — source は Error + drain 継続)/ §1.3 v1.6
(abandon の可視カウント)。所見の詳細 = [P2_REVIEW.md](P2_REVIEW.md) の
R-P2-8 / R-P2-3 / R-P2-9 / R-P2-10 / R-P2-12 / R-P2-13(logbook)。
**発注先想定**: implementer/**Opus**(receiver 送信ループの再設計判断が残る)

## やること(所見番号順)

1. **R-P2-8 [high] poisoned Mutex を「エラーなし」に丸めない**:
   decoder.rs / graw_writer.rs の `latch_error` — `.lock()` が Err(poisoned)なら
   **即 Error 遷移 + warn**(「worker thread panicked — entering Error」)。
   `unwrap_or(false)` で握りつぶさない。
2. **R-P2-3 [high] receiver 送信経路を decoder パターンへ**(src/receiver.rs):
   - PUSH に **SNDTIMEO**(decoder の `send_timeout_ms` と同じ出所・同じ既定。config に
     [receiver] 側が無ければ decoder と同じ流儀で追加)。
   - `send_on` を **EAGAIN リトライループ**に(stop フラグを見る。**捨てない・ただし
     中断可能**)。Stop/Reset で中断したら `messages_abandoned` カウント + 初回 warn。
   - encode 失敗 → `encode_errors` カウント + error(現在は error ログのみ・無カウント)。
     ETERM 以外の send エラー → `send_errors` カウント + error。**全カウンタを
     `Metrics::json()`(GetStatus metrics)に露出**。
   - 検収: 下流不在(PULL 不在)で Stop→Reset が即座に返り、abandoned が数えられ、
     スレッドがリークしない(join まで確認)。既存のバイト一致・overflow テスト無影響。
3. **R-P2-9 [med] graw-writer を decoder の異常系水準へ**(src/graw_writer.rs):
   - EOS の run_number 不一致 → `run_mismatches` に**計上**(現在 warn のみ)。
   - 期待外 source_id の EOS → `eos_received` に**入れず**カウント(`unexpected_sources`)+
     info(現在は無言で insert)。
   - デコード不能メッセージ → `decode_errors` カウント(現在 warn のみ)。
   - Heartbeat 受信 → `heartbeats_in` カウント(現在無言で捨てる)。
   - すべて metrics_json に露出 + テスト。
4. **R-P2-10 [med] framer リセットの能動ログ**(src/receiver.rs): `framer_resets` の
   増分検知時に**一度だけ warn**(decoder の `logged_*` 方式。以降はカウンタが担う)。
5. **R-P2-12 [low] decoder に `heartbeats_out`**(送出成功)+ 送出打ち切り
   `heartbeats_abandoned`。`let _ =` をやめる。metrics_json に露出。
6. **L3(doc のみ)**: decoder `batch_abandoned` のコメント「Reset 中に限り」を実装
   (一般 ZMQ エラーでも通る)に合わせて修正。
   ※ R-P2-13 の logbook(recover_next_seq の warn)は **025 に移管**(logbook.rs の
   所有権衝突を避ける — 並行発注のため)。

## テスト(TDD。全カウンタは「metrics_json に載っていること」まで機械照合)

- poisoned: テスト内でロック保持中に panic するスレッドを作って poisoned Mutex を用意 →
  latch_error → state が Error になることを機械照合(decoder / graw-writer 両方)。
- receiver: 下流不在 + 満杯 HWM で送信ブロック → Stop → 期限内(< 2 s)に完了 +
  `messages_abandoned` ≥ 1 + metrics 露出。encode 失敗経路は到達可能なら単体で、
  不能なら理由を記録してカウンタ配線のみ検査。
- graw-writer: EOS run 不一致 / 期待外 source EOS / 壊れた msgpack / Heartbeat の 4 経路 →
  各カウンタ + 「期待外 EOS では run が閉じない」こと。
- 既存全テスト無影響。

## 受け入れ

- `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test` 全 green(既存無影響)。
- ファイル所有権: src/{decoder.rs, receiver.rs, graw_writer.rs} +
  config.rs([receiver] send_timeout の追記が要る場合のみ)+ 対応する tests/*。
  **他ユニットのファイル(controller / geometry / tools/)に触らない**。
  発注書に無い設計分岐に出会ったら実装せず報告して戻る。
