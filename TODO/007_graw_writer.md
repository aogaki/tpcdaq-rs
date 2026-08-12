# 007 — graw-writer コンポーネント(CoBo 毎ファイル、バイト一致)

**Status: OPEN**
**仕様**: SPEC §7(全部)、§6.5(出力配置)、§1.3/§1.4(状態機械・過負荷)、§2.2/§2.3(Batch/EOS)
**依存**: 003(msg/command/zmq_helper)、006(receiver — E2E で使用)、005(graw_replay — E2E で使用)

## やること

1. `src/graw_writer.rs` — graw-writer 本体:
   - **PULL bind**(既定 `tcp://*:47001`、zmq_helper の有限 HWM)。受信スレッド(専用 OS スレッド、
     006 送信側と同じ理屈で同期 zmq)→ 書き込み処理。
   - RawFrames バッチを **source_id(= cobo_id)毎のファイル**へバイトそのまま append。
     リシリアライズ禁止(連結 = 元ストリームと同一)。
   - 出力: `<output_root>/run{run:04}/run{run:04}_cobo{K}_{seq:04}.graw`(SPEC §6.5)。
     run ディレクトリ・ファイルは**当該 run の最初のフレーム到着時に遅延作成**(run_number は
     Batch に載っている)。ハンドルは run 中開きっぱなし(per-frame open/close 禁止)。
   - ローテーション: `cur + n > max_file_bytes(既定 1 GiB)` で次 seq へ。**フレームはファイル間で
     分割しない**。単発の巨大フレームはそのまま書く(`cur_bytes > 0` ガード — C++ 版と同じ)。
   - flush 1 秒毎、fsync はローテーションと close 時(ホットパスで fsync しない)。
   - **ロスレス検証**: ソース毎 sequence_number 連続性チェック。ギャップ = Error 状態 + カウント
     (silent 禁止)。EOS 前に run_number が変わるのもプロトコル違反 = Error。
   - **EOS**: 期待ソース集合(設定の `[[cobo]]` 全 id)から全 EOS 受領 → 当該 run の全ファイルを
     flush + fsync + close。metrics にファイル実績(パス・バイト数)を出す(将来の run_stop 記録の材料)。
   - 書き込み失敗(ディスクフル等)= Error 状態 + write_errors カウント + **PULL 消費停止**
     (HWM が詰まり上流へ背圧 → receiver 側 overflow が可視化される、SPEC §1.4 のカスケード)。
   - 状態機械: Configure(設定確定)→ Arm(PULL bind)→ Start{run}(消費開始)→ Stop / Reset。
     REP は 003 `run_command_task`。カウンタ: bytes/frames/batches(cobo 毎)、seq_gaps、write_errors、
     files(パス + サイズ)。
2. `src/bin/graw_writer.rs` — `graw_writer --config <toml>`(tracing 初期化 + 起動)。
3. config: `[graw_writer]` セクション(`pull_bind` 既定 47001 / `max_file_bytes` 既定 1 GiB /
   `flush_interval_ms` 既定 1000)を `src/config.rs` に追加(既存フィールド・テストは無改変)。

## テスト

- 単体: ファイル命名、ローテーション境界(フレーム非分割・連結一致)、巨大フレーム、遅延作成。
- 統合(port 0、PUSH で直接 Batch 投入):
  (a) 2 CoBo 分のバッチ → cobo 毎ファイルがそれぞれ**入力連結とバイト一致**
  (b) ローテーション跨ぎでも連結一致
  (c) 全 EOS でファイル close + metrics にファイル実績
  (d) seq ギャップ → Error / EOS 前の run 変更 → Error
  (e) Configure→Arm→Start→Stop の全シーケンス
- **E2E(env `TPCDAQ_REAL_GRAW` 時)**: graw_replay(全速)→ receiver(006)→ graw-writer で、
  出力ファイル連結が**元 .graw とバイト完全一致**(= SPEC §12-2 の受け入れそのもの)。
  実測値(バイト数・ファイル数・所要)を `## 結果` に記録。

## 受け入れ

- 上記全テスト green。E2E バイト完全一致。`cargo fmt && cargo clippy --all-targets -- -D warnings
  && cargo test` 通過。
