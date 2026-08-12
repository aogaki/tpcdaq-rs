# 005 — graw_replay ツール(Rust)

**Status: COMPLETED**(2026-08-12。実装 = implementer/Sonnet、レビュー = Fable)
**仕様**: SPEC §12 末尾(受け入れ試験のリプレイ手段)。C++ 版 `tools/graw_replay.cpp`
(ペーシングなし全速のみ)の後継
**依存**: 001(ワークスペース)。`src/lib.rs` / `Cargo.toml` は変更しない(`src/bin/` は自動検出)

## やること

1. `src/bin/graw_replay.rs` — .graw ファイルを TCP で receiver へ送出:
   - 引数: `graw_replay <host:port> <file.graw> [--rate-mbps <f64>] [--loop] [--chunk-bytes <n=65536>]`
     (手書きパース。追加依存なし — clap 等は入れない)。
   - `--rate-mbps`: 単純なペーシング(チャンク送出間隔の調整で可。指定なし = 全速)。
   - `--loop`: EOF でファイル先頭へ戻り続ける(Ctrl-C で停止)。loop なし = 送出完了で close
     (受信側は EOF = run 境界)。
   - partial write 耐性(全書き込みループ)。接続失敗・途中切断は明確なエラー + 非 0 exit。
2. ペーシング計算等のロジックはテスト可能な関数に分離(bin 内モジュールで完結、KISS)。
3. テスト `tests/graw_replay_tool.rs`:
   - テスト内で listener を立て(port 0)、合成バイト列ファイルをリプレイして**バイト一致**。
   - `--rate-mbps` の実効レートが指定値の ±30% に収まる smoke(フレークしないマージンと
     短い実行時間で。厳密な精度試験はしない)。
   - `--loop` は「2 周分以上受けたら切る」形で確認。

## 受け入れ

- バイト一致 + ペーシング smoke + loop テスト green。`cargo build --bin graw_replay` 成功。
- production コード `.unwrap()` なし(lint で強制済み)。

## 結果

**実行環境**: macOS(Darwin 25.5.0)/ cargo 1.97.1 / rustc 1.97.1 / 2026-08-12

**変更ファイル**:
- `src/bin/graw_replay.rs`(新規) — CLI 本体。`args`(手書き引数パース)/ `pace`(ペーシング計算)
  の 2 サブモジュールで完結。`#[cfg(test)]` にユニットテスト 11 本。
- `tests/graw_replay_tool.rs`(新規) — E2E テスト 5 本。

**実行コマンド**:
```
cargo build --bin graw_replay
cargo test --bin graw_replay        # bin 内ユニットテスト
cargo test --test graw_replay_tool  # E2E
cargo fmt --check                   # 全リポジトリ差分なし確認
cargo clippy --tests -- -D warnings
cargo clippy --all-targets -- -D warnings
cargo test                          # フルスイート
```

**テスト結果**: 全 green。
- `graw_replay`(bin 内ユニット、11 tests): `parse_minimal_positional_args_uses_defaults`,
  `parse_accepts_all_optional_flags_in_any_order`, `parse_rejects_wrong_positional_count`,
  `parse_rejects_non_positive_rate`, `parse_rejects_zero_chunk_bytes`,
  `parse_rejects_unknown_option`, `parse_rejects_dangling_flag_without_value`,
  `mbps_to_bytes_per_sec_matches_hand_calculation`, `sleep_for_returns_zero_when_pacing_disabled`,
  `sleep_for_returns_zero_when_already_behind_schedule`,
  `sleep_for_waits_the_remaining_budget_when_ahead_of_schedule`
- `graw_replay_tool`(E2E、5 tests): `replays_file_bytes_exactly_over_tcp`,
  `rate_mbps_paces_within_30_percent_margin`, `loop_flag_repeats_file_at_least_twice`,
  `missing_arguments_exit_non_zero_with_usage_message`,
  `connect_failure_exits_non_zero_with_clear_message`
- リポ全体 `cargo test`: 上記込みで 114 passed / 0 failed(lib 66 + graw_replay 11 + geometry 系
  27 + graw_replay_tool 5 + zmq 系 5。004 の framer/decoder は本タスクと並列進行中のため未計上)。
- ペーシング smoke(`rate_mbps_paces_within_30_percent_margin`)/ loop テストはそれぞれ単独 5 回
  再実行してフレーク無しを確認(前者は実測 1.12〜1.17 秒、許容窓 [0.7, 1.3] 秒に対して安定)。

**CLI 使用例**(手動 smoke、`nc -l` を受信側にして実施・確認済み):
```
$ graw_replay 127.0.0.1:34719 run.graw --rate-mbps 1 --chunk-bytes 8
replayed 39 bytes to 127.0.0.1:34719

$ graw_replay                      # 引数不足
graw_replay: expected 2 positional arguments (host:port, file.graw), got 0
usage: graw_replay <host:port> <file.graw> [--rate-mbps <f64>] [--loop] [--chunk-bytes <n=65536>]
(exit code 2)

$ graw_replay --loop 127.0.0.1:5000 run.graw   # 受信側が EOF まで待たず run 境界を切りたい場合は付けない
```

**逸脱・迷った点**:
- `--rate-mbps` の単位を「メガビット/秒」(ネットワーク慣習の bit 単位、1 Mbps = 1,000,000 bit/s)
  と解釈して実装した。発注書・SPEC §12 とも単位の定義が明記されておらず(SPEC の他箇所のスループット表記は
  すべて `MB/s`)、ここは実装に必要な決定だったため独自に補った。`pace::mbps_to_bytes_per_sec` の
  doc コメントと本節に明記。もし「MB/s」の意図であれば `mbps_to_bytes_per_sec` を
  `rate_mbps * 1_000_000.0` に変える 1 行差分で直せる(呼び出し側・テストへの影響は
  ペーシング smoke テストの期待値 1 本のみ)。API 形状・依存追加を伴わない単位解釈のみの論点なので、
  実装を止めずに進め、ここに明記して報告する。
- partial write 耐性は C++ 版のように手動 retry ループを書かず、`std::io::Write::write_all`
  (内部で部分書き込みを自動的にループして埋める)をそのまま使った。C++ の POSIX `send()` と異なり
  Rust の `write_all` は「全部書けるかエラーになるまでループする」契約を標準ライブラリが保証するため、
  同じ耐性を素直な形で満たせる(明示的なループを自前実装すると却って KISS に反すると判断)。
- E2E テストにケース数を発注書の 3 種(バイト一致・ペーシング smoke・loop)から 2 種
  (`missing_arguments_exit_non_zero_with_usage_message` / `connect_failure_exits_non_zero_with_clear_message`)
  追加した。発注書「接続失敗・途中切断は明確なエラー + 非 0 exit」の受け入れを直接テストで
  裏付けるための最小追加で、新規依存・新規 API 面はない。過剰なら削ってよい旨を申し添える。
- `Cargo.toml` の `[lints.clippy] unwrap_used = "deny"` はワークスペース全体の `[lints]` テーブル
  なので、`Cargo.toml` 自体には触れずに本 bin にも自動適用される(production コードで `.unwrap()`
  未使用を確認済み。テストコードのみ `#[allow(clippy::unwrap_used)]` を bin のテストモジュール /
  `tests/graw_replay_tool.rs` の先頭に付与)。
- 004(framer/decoder)を並走中の同一ツリーで、`src/lib.rs` / `Cargo.toml` / `src/` 直下の既存
  モジュール / `tests/` の他ファイルには一切触れていない(`src/bin/graw_replay.rs` と
  `tests/graw_replay_tool.rs` のみ新規作成)。`cargo fmt --check`(全リポジトリ)は本タスク完了時点
  で差分なし。

### レビュー裁定(Fable、2026-08-12)

コードレビュー + ゲート独立再実行で確認。裁定:

1. **`--rate-mbps` = Mbit/s(メガビット/秒)で確定 — 承認**。フラグ名の慣習どおり。
   SPEC §12 に単位と換算例(mini 100 Hz 相当 ≈ 28 MB/s = 224 Mbps)を明記した。
2. `write_all` による partial write 耐性 — 承認(標準ライブラリの契約で C++ 手動ループと等価)。
3. 追加 E2E 2 本(引数不足・接続失敗)— 承認(受け入れ行の直接裏付け)。
4. 004 側から報告のあった connect_failure テストの単発フレークは再現せず(私の再実行でも green)。
   再発したら間隔・リトライを見直す。
