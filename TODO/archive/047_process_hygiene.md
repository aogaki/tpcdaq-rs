# 047 — プロセス起動・終了の衛生(Rust bin 共通化 + vcobo SIGINT)(044 窓)

**Status: COMPLETED**(2026-08-15 — 結果は末尾)
**Status(起票時): READY**(起票 2026-08-15 Fable — 044 レビュー C4 + 041 発見⑤)
**発注先想定**: implementer/**Sonnet**(イディオムの移植と機械的抽出。発注書で縛れる)

## A. Rust 5 bin のボイラープレート共通化(044-C4)

- 事実: `src/bin/{receiver,decoder,graw_writer,monitor,controller}.rs`(各 113-136 行)で
  tracing 初期化(:24-28 相当)と SIGINT ハンドラ(:50-60 相当、5 bin 同一)が重複。
  decoder.rs と graw_writer.rs は 5 行を除き完全一致(レビュー diff 実測)。
  041 発見⑤級の変更が「5 箇所を一致させて直す」形になっている。
- やること: `src/bin_support.rs`(lib 配下、名前は流儀に合わせ調整可)に
  `init_tracing()` と `spawn_sigint(...)` **だけ**を抽出し、5 bin から使う。
  **`parse_args` と bin 内テストは各 bin に残す**(receiver だけ `--cobo-id` があり、
  テストが bin 内に住むため — 動かすとテスト変更になる。レビュー裁定どおり)。
- 受け入れ: 既存テスト無変更で cargo ゲート全 green。5 bin の挙動(USAGE 文字列・
  終了コード・ログ形式)がバイト同一であること。

## B. vcobo-daq の SIGINT graceful 化(041 発見⑤)

- 事実: `tools/vcobo/vcobo_daq.cpp:475-587` の main に signal 処理なし
  (`<csignal>` は include 済み・未使用)。Ctrl-C は OS 既定で即死し、
  `link.shutdown_worker(); ic->destroy();`(:583-584)のクリーンアップを飛ばす。
- やること: リポ既存イディオムの移植(`tools/ecc_bridge/ecc_bridge.cpp:38-39` /
  `fake_ecc.cpp:55-56,377` — `volatile std::sig_atomic_t g_stop` + ハンドラ +
  `waitForShutdown()` を `while (g_stop == 0) sleep_for(..)` ポーリングに置換 → 既存
  teardown へ落ちる)。**シグナルハンドラから Ice API を呼ばない**(既存パターン厳守)。
  見積もり 10-15 行・vcobo_daq.cpp のみ(**vcobo_core/vcobo_link は触らない** — 044 裁定)。
- 受け入れ: `make -C tools/vcobo test / ci` green(ci は
  `TPCDAQ_ICE_DIR=$PWD/reference/20190315_patched` 付き)。SIGINT/SIGTERM で
  「クリーンアップログを出して exit 0」を手動確認し結果節に記録。

## 受け入れ(共通)

- 結果節: A/B 毎の diff 要旨 / テスト結果(数値)/ 環境と日付。

## 結果(2026-08-15 implementer/Sonnet → 発注側(Fable)レビュー PASS)

- **A**: `src/bin_support.rs` 新設(`init_tracing` / `spawn_sigint` の 2 関数のみ —
  ログ文言・エラーハンドリングはバイト同一移植)。5 bin 各 21 行削減、`parse_args` と
  bin 内テストは無改変。cargo ゲート全 green(bin 単体テスト各 2 passed = 変更前と同数)。
- **B**: vcobo_daq.cpp 15 行差分 — 既存イディオム(ecc_bridge/fake_ecc)の移植、出典
  コメント付き。ハンドラは `g_stop` セットのみ(Ice API 不呼)。
  `make test`(92)/ `ci`(92+57+6)全 green・警告 0。
  **手動確認: SIGINT / SIGTERM とも `link.shutdown_worker() → ic->destroy() →
  "vcobo_daq stopped"` を経て exit 0**(Ice アダプタ稼働中からの割り込みで実証)。
- 逸脱なし。実行環境: macOS Darwin 25.5.0、2026-08-15。

**Status: COMPLETED**
