# 014 — run.root の圧縮設定化(既定 ZLIB-1 = 旧 ROOT 互換)

**Status: COMPLETED**
**仕様**: SPEC v1.5 §6.4(圧縮 = **101(ZLIB-1)既定・設定可能**)、docs/WARSAW_PLAN_ja.md §2
**依存**: 011(COMPLETED — root_recorder.hpp が対象)
**発注先想定**: implementer/Sonnet(完全に縛れる小変更)

## 背景(一行)

Warsaw はオフライン解析も DAQ 計算機の同一(旧)ROOT で行うため、ZSTD(ROOT 6.20+ 必須)の
run.root は**先方で開けない**。既定を全時代互換の ZLIB-1(= C++ 版の「ROOT 既定」と同じ)へ。

## やること

1. `tools/root_sink/root_recorder.hpp` — 圧縮設定の焼き込み(505)を廃し、`Recorder` の
   コンストラクタ引数 `compression`(int)に。**既定 101**。
2. `tools/root_sink/root_sink.cxx` — CLI `--root-compression N` を追加(既定 101)。
   検証: 非負整数のみ受理(不正は usage + exit 1 — 既存の parse_* の流儀)。
   起動ログの 1 行(既存の設定表示)に compression を追記。
3. テスト(`test_recorder.cxx`):
   - 既定で書いたファイルの `TFile::GetCompressionSettings() == 101` を照合。
   - 明示指定(例 505)がファイルに反映されることを照合。
   - 既存 163 CHECK が 505 前提を含むなら 101 前提へ更新(数値の理由コメント付き)。
4. 再検証: `make test-root` / `make test` / `run_conformance.sh` /
   `TPCDAQ_ROOT_SINK_BIN + TPCDAQ_REAL_GRAW` の統合 + E2E(entries=108 不変)。
   **run.root のファイルサイズ変化(ZSTD-5 比)を実測して結果節に記録**(ZLIB-1 は太る想定 —
   低レート運用なので許容だが数値は残す)。

## 受け入れ

- 上記テスト green、既存テスト無影響(cargo 226 / C++ 68/71/175/163+/49)。
- 既定 101・CLI 上書き可の動作確認。E2E オラクル維持。
- **先方旧 ROOT での実開封はデモ D1 の受け入れ項目**(本ユニットでは不可能 — 手元に旧 ROOT なし。
  WARSAW_PLAN §2 に記載済み)。
- ファイル所有権: tools/root_sink/(root_recorder.hpp / root_sink.cxx / test_recorder.cxx)のみ。

## 結果

**実行環境**: macOS 26.5.2(Darwin 25.5.0)/ arm64 / Apple clang(`c++ -std=c++17 -O2 -g -Wall -Wextra`、
**NDEBUG 未定義**)/ ROOT **6.36.10** @ `/opt/ROOT`(root-config は PATH に無いため
`/opt/ROOT/bin/root-config` を Makefile の既定フォールバックで使用)/ libzmq 4.3.5(pkg-config)/
rustc(cargo test プロファイル dev)。**2026-08-13**。

### 変更ファイル(発注書の所有権どおり、他は無改変)

| ファイル | 中身 |
|---|---|
| `tools/root_sink/root_recorder.hpp` | 焼き込み定数 `kRootCompression = 505` を廃し `kDefaultCompression = 101` に。`RecorderConfig` に `compression`(既定 `kDefaultCompression`)フィールドを追加(`output_root`/`max_root_bytes` と同じ既存パターン)。`open_part()` の `TFile::Open` 第4引数を `cfg_.compression` に |
| `tools/root_sink/root_sink.cxx` | `Options::root_compression`(既定 `rootsink::kDefaultCompression`)追加。CLI `--root-compression N` を追加(`parse_nonnegative` 流用 — 既存 `--throttle-ms` と同じ「非負整数のみ受理、不正は usage + exit 1」)。`usage()` に説明追記。`RecorderConfig::compression` へ配線。起動ログ 1 行に `compression=%d` を追記 |
| `tools/root_sink/test_recorder.cxx` | 新規テスト 2 本追加(`test_default_compression_is_101_zlib1` / `test_explicit_compression_setting_is_honored`)+ 共有ヘルパ `read_compression_settings` / `write_one_entry_run`。既存 5 テストは無改変(505 前提の CHECK は無かった — grep で確認済み) |

`src/*.rs` / `Cargo.toml` / `tests/*.rs` / `Makefile` には一切触っていない。

### 実行コマンドと結果

| コマンド | 結果 |
|---|---|
| `make -C tools/root_sink clean && make -C tools/root_sink`(本体) | **警告ゼロ**(`-Wall -Wextra`、NDEBUG なし) |
| `make -C tools/root_sink test`(ROOT 非依存 4 本) | **68 / 71 / 175 / SKIP**(GOLDEN 未指定)すべて 0 failed |
| `make -C tools/root_sink test-root` | **test_recorder: 169 passed / 0 failed**(既存 163 + 新規 6 CHECK) |
| `tools/root_sink/run_conformance.sh` | exit 0(**68 / 71 / 175 / 49** すべて 0 failed) |
| `TPCDAQ_ROOT_SINK_BIN=$PWD/tools/root_sink/root_sink cargo test --test root_sink_intake` | **9 passed / 0 failed** |
| 上記 + `TPCDAQ_REAL_GRAW=/Users/aogaki/TPC/CoBo_2025-09-01T08_51_06.203_0000.graw` | **9 passed / 0 failed**(5.01 s。E2E 込み、`entries=108` 不変) |
| `cargo test --no-fail-fast`(env 未設定) | **226 passed / 0 failed**(011 時点と同数 — src/*.rs 無改変につき増減なし) |
| `cargo clippy --all-targets` | 警告ゼロ |

### red → green(TDD)

新規テストを先に書いた後、`kDefaultCompression` を意図的に旧値 `505` へ戻して赤を確認し、
`101` へ復旧した:

| 状態 | `make test-root` 結果 |
|---|---|
| `kDefaultCompression = 505`(故意に壊した状態) | **168 passed / 1 failed** — `FAIL test_recorder.cxx:617  read_compression_settings(path) == 101 (got 505, want 101)` |
| `kDefaultCompression = 101`(復旧後) | **169 passed / 0 failed** |

既存 5 テストへの影響なし(壊した箇所が新規 2 テストのみで検出される設計どおり)。

### `GetCompressionSettings()` 実測

`test_recorder` の新規 2 ケース(`TFile::GetCompressionSettings()` を直接読み戻し):

1. `test_default_compression_is_101_zlib1` — `RecorderConfig` 既定のまま 1 エントリを書き、
   `GetCompressionSettings() == 101`(ZLIB アルゴリズム 1 × レベル 1)を確認。
2. `test_explicit_compression_setting_is_honored` — `RecorderConfig::compression = 505` を
   明示指定して同じ 1 エントリを書き、`GetCompressionSettings() == 505`(ZSTD アルゴリズム 5 ×
   レベル 5)を確認。

CLI 側も別途スモークテストで確認(`test_recorder.cxx` の範囲外なのでこのユニットの受け入れ
テストには含めていないが、動作記録として残す):

```
$ ./root_sink --bind tcp://127.0.0.1:47099 --output-root /tmp/... --root-compression 505
root_sink: writing TTree "tree" under /tmp/... (max-root-bytes=1073741824 compression=505)
```

`--root-compression -1` / `--root-compression abc` はいずれも
`root_sink: --root-compression expects a non-negative integer, got '...'` + `exit=1`
(usage 相当)。

### run.root のファイルサイズ変化(ZSTD-5 比、実測)

E2E(`real_graw_replayed_end_to_end_writes_108_entries`、実 .graw
`/Users/aogaki/TPC/CoBo_2025-09-01T08_51_06.203_0000.graw`、entries=108、
決定的な同一入力・同一デコード結果)で比較:

| 圧縮設定 | run.root サイズ | 出典 |
|---|---|---|
| 505(ZSTD-5、旧既定) | 46,041,087 B | TODO/archive/011_gdataframe_ttree.md `## 結果`(2026-08-13 実測、同一 E2E 経路) |
| **101(ZLIB-1、新既定、本ユニット実測)** | **63,201,385 B** | 本ユニット(上表のコマンド、2 回再実行して同一値を確認) |

差分 +17,160,298 B、**比率 1.373(+37.3%)** — ZLIB-1 は ZSTD-5 比で約 4 割太る。想定どおり
(発注書「ZLIB-1 は太る想定 — 低レート運用なので許容」)。小サンプルの単体テスト
(`test_recorder` の 1 エントリ・1 サンプル)でも同傾向: ZLIB-1 = 14,310 B、ZSTD-5 = 11,683 B
(圧縮対象データが小さいため絶対比率はこの単体テストでは参考値)。

### 逸脱・追加対応

- 実装後の CLI ヘルプ文言確認で `--root-compression` の説明が「(default 101 = 101 = ZLIB-1」と
  二重表示になるバグを発見(printf の `%d` と手書きの `101` が重複)。発注書にない追加変更では
  なく実装バグの修正として `root_sink.cxx` の該当 1 行を訂正し、再ビルド・再テストで全 green を
  再確認した(上表は訂正後の最終結果)。
- 発注書の想定を超える設計判断は発生せず(`RecorderConfig` に `compression` フィールドを足す
  やり方は既存の `output_root` / `max_root_bytes` と同じ既存パターンをなぞっただけ)。
- 「先方旧 ROOT での実開封」は発注書どおり本ユニットの範囲外(手元に旧 ROOT 環境なし)。

### 最終レビュー(2026-08-13 Fable)

- **判定: 受理(COMPLETED)**。発注書どおりの最小実装 + usage 文言の実装バグ自己修正(正当)。
  red→green(故意に 505 へ戻して 1 failed を確認)も実施済み。
- レビュー側で独立再検証: test_recorder 169 passed / 0 failed、env 付き intake 9 passed
  (E2E entries=108 不変)。
- サイズ実測 +37.3%(46.0 → 63.2 MB / 30 MB 入力)は低レート運用で許容と判断。参考: 実機の
  同 run 変換(63,747,101 B)とほぼ同サイズになった = 実機も ZLIB 系だった傍証。
- 残る受け入れ = デモ D1 での先方旧 ROOT 実開封(WARSAW_PLAN §2 / §5 に記載済み)。
