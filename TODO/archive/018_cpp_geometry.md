# 018 — root-sink 側 C++ ジオメトリパーサ + 二重実装一致(§4.5)

**Status: COMPLETED**
**仕様**: SPEC v1.6 §4.1(.dat 2 レイアウト受理)・§4.2(ChannelRole)・§4.3(FPN リオーダ)・
§4.5(二重実装の一致テスト = 本ユニットの受け入れ)。P3(モニタ)の前提 — 019 のヒスト集計が
strip 対応にこれを使う。
**流用元**: `~/test/get/tpcdaq/src/geometry/geometry.cpp`(C++ 版 = ユーザー自身のコード、
コピー + 改変可、出自コメント)。**意味論の正 = Rust 実装 `src/geometry.rs`(002 完了、
オラクル一致済み)** — 迷ったら Rust 実装に合わせる。
**発注先想定**: implementer/Sonnet(実行可能な仕様 = Rust 実装がある)

## やること

1. `tools/root_sink/geo.hpp` — 純ヘッダ(ROOT/ZMQ 非依存、rs_core.hpp の流儀):
   - TPCReco `.dat` パーサ(2 レイアウトとも受理 — §4.1、Rust `parse` と同一判定)。
   - `lookup(cobo, asad, aget, raw_ch) -> (role, plane, section, strip)`。
     ChannelRole = Signal/FPN/Aux/Unmapped(§4.2、Rust と同一分類)。
   - FPN リオーダ(GRAW 0–67 ↔ geometry 0–63、FPN={11,22,45,56})— Rust §4.3 と同一表。
   - 面毎 Nstrip の取得(mini: U72/V92/W92、ELITPC: U132/V225/W226 — **焼き込み禁止**、
     .dat から導出。Rust と同じ)。
   - 重複チャンネル・不正行はカウント + 保持(silent 禁止 — Rust の
     `duplicate_warnings`/`malformed_lines` と同じ扱い)。
2. **TSV ダンプ**: Rust `geometry::dump_tsv` と**バイト一致**の出力(列・順序・表示名は
   Rust 実装が正 — src/geometry.rs を読んで同一フォーマットにする)。
3. `src/bin/geometry_dump.rs`(新規、Rust 側 CLI): `geometry_dump <file.dat>` → stdout に
   `dump_tsv`。既存モジュールを呼ぶだけの薄い bin(lib.rs 改変不要)。
4. `tools/root_sink/test_geo.cpp`(assert + main、Makefile の TESTS へ追加):
   Rust 側テスト(src/geometry.rs の合成フィクスチャ)から代表ケースを移植 —
   2 レイアウト parse / lookup / FPN 表 / role 分類 / 重複・不正行カウント / ダンプ行数。
5. `tools/root_sink/run_geo_conformance.sh`: 合成フィクスチャ .dat(リポ内に新規作成可 —
   **合成のみ**)で `cargo run --bin geometry_dump` と C++ ダンプを **diff で機械比較**。
   さらに実 .dat を指す env(既存の前例を src/geometry.rs のテスト・TODO/archive/002 で確認し
   同じ変数名を使う)があれば実 .dat でも比較(無ければ SKIP 明示)。

## 受け入れ

- `make test`(既存 4 本 + test_geo)green、run_geo_conformance.sh が合成で exit 0
  (実 .dat は env があれば一致を記録)。
- `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test` 無影響。
- ファイル所有権: tools/root_sink/(geo.hpp / test_geo.cpp / run_geo_conformance.sh /
  Makefile の TESTS 追記)+ src/bin/geometry_dump.rs(新規 1 ファイルのみ)+ 合成 .dat
  フィクスチャ(tests/fixtures/ 等、リポ既存の置き場に従う)。
  **src/logbook.rs・src/state.rs・src/lib.rs・tools/ecc_bridge/ に触らない**(並列作業中)。

## 結果

**実行環境**: macOS(Darwin 25.5.0, arm64)/ rustc 1.97.1 / cargo 1.97.1 /
Apple clang 21.0.0(`g++` = clang フロントエンド、tools/root_sink 既定コンパイラ)/ 2026-08-13。

### 実行コマンドとテスト結果

```
cd tools/root_sink && make clean && make test
cd tools/root_sink && ./run_geo_conformance.sh
cd tools/root_sink && TPCDAQ_REAL_GEOMETRY_MINI=.../TPCReco-HIGS2026_online/resources/geometry_mini_eTPC.dat \
                       TPCDAQ_REAL_GEOMETRY_ELITPC=.../TPCReco-HIGS2026_online/resources/geometry_ELITPC.dat \
                       ./run_geo_conformance.sh
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

`make test`(既存 4 本 + 新規 test_geo、全 green):

- `test_tpc_wire`: 68 passed, 0 failed(既存・無影響)
- `test_rs_core`: 71 passed, 0 failed(既存・無影響)
- `test_eb_core`: 175 passed, 0 failed(既存・無影響)
- `test_conformance`: SKIP(GOLDEN 未指定 — `make test` 素の既定挙動、既存・無影響)
- **`test_geo`(新規): 426 passed, 0 failed**(CHECK/CHECK_EQ 呼び出し数。テスト関数 30 本:
  reorder 表 loop 一致 / plane parse / NEW フォーマット(header 7 キー・strip・aux・fpn・
  unmapped・max_strip・cobo_count・warnings なし・load 失敗)9 本 / 2-CoBo フィクスチャ
  (cobo_count・cobo をまたぐキー衝突なし・asad 枚数差・FPN)5 本 / LEGACY フォーマット
  3 本 / dump_tsv(行数・列数・先頭行・FPN/AUX/Unmapped 行・unmapped_hits 不変)7 本 /
  重複・malformed・aget 範囲外・unmapped_hit_count 4 本)

`cargo test`: 226 passed, 0 failed(全 binary 合計。新規 `geometry_dump` bin は `#[test]`
0 件で既存カウントに無影響。geometry 関連の既存 27 テスト — `geometry_new_format.rs` 9 /
`geometry_multi_cobo.rs` 5 / `geometry_legacy_format.rs` 4 / `geometry_tsv_dump.rs` 7 /
`geometry_real_regression.rs` 2 — は無改変で全 green)。

`cargo fmt -- --check`: 差分なし。`cargo clippy --all-targets -- -D warnings`: 警告 0。

### 二重実装一致(`run_geo_conformance.sh`)の実測値

**合成フィクスチャ(3 本、TODO/002 で作成済みのものを再利用 — 新規フィクスチャは作らず、
「Rust と C++ が同じ入力を読んでいる」保証を強めた)**、いずれも `cmp -s` でバイト一致・
exit 0:

| フィクスチャ | 形式 | 行数 | バイト数 |
|---|---|---|---|
| `tests/fixtures/geometry_mini_reduced.dat` | NEW(10 欄) | 272 | 5,622 |
| `tests/fixtures/geometry_2cobo_fake.dat` | NEW(10 欄、2-CoBo) | 816 | 16,962 |
| `tests/fixtures/geometry_legacy.dat` | LEGACY(7 欄) | 272 | 5,632 |

**実 .dat(env 変数、TODO/archive/002 と同一の変数名・パス形状)** — ローカルに
`reference/TPCReco/TPCReco-HIGS2026_online/resources/` が存在したため実測できた:

| env | 行数 | バイト数 |
|---|---|---|
| `TPCDAQ_REAL_GEOMETRY_MINI`(geometry_mini_eTPC.dat) | 272 | 5,821 |
| `TPCDAQ_REAL_GEOMETRY_ELITPC`(geometry_ELITPC.dat) | 1,088 | 23,818 |

いずれも `cmp -s` でバイト一致・exit 0(`run_geo_conformance.sh` 全体の終了コード = 0)。
env 未設定時は該当行を `SKIP` と明示して exit 0 のまま(実測済みだが、CI 等で
`reference/`(gitignore 済み)が無い環境でも `make test` 相当の完了条件には影響しない)。

### 逸脱・判断に迷った点

1. **`test_geo` を「CHECK 群を走らせるテストバイナリ」と「`<file.dat>` を渡すとダンプを
   吐く CLI」の 2 役にした(独立した dump 専用 .cpp は新設していない)**。発注書の
   ファイル所有権が `geo.hpp` / `test_geo.cpp` / `run_geo_conformance.sh` / Makefile の
   `TESTS` 追記のみに限定されていたため、`compare_gdataframe.cxx` のような別ビルド
   ターゲットを新設する余地がなかった。`test_conformance.cpp` が既に「引数の有無で
   挙動を変える」前例(GOLDEN パスの有無)を持っていたので、その流儀を延長した
   (`argc >= 2` ならダンプモード、無しなら通常の CHECK 群)。設計分岐というほどではないが、
   念のため明記する。
2. **`Geometry` を move-only にした(コピー禁止、move ctor/operator= を手書き)**。
   `unmapped_hits_` が `std::atomic<uint64_t>`(Rust の `AtomicU64` に対応)なため、
   既定の move/copy が暗黙に delete される。`build_geometry` の `return g;` は
   NRVO が効かない場合に move ctor が **アクセス可能であること** を要求する
   (呼ばれなくても delete だとコンパイルエラー)ため、カウンタの現在値を
   `load()` して新しい atomic に積み替えるだけの move ctor/operator= を追加した。
   意味論上の分岐ではなく、C++ の atomic メンバの機械的な帰結。
3. **合成フィクスチャは新規作成せず、TODO/002 の 3 本
   (`geometry_mini_reduced.dat` / `geometry_2cobo_fake.dat` / `geometry_legacy.dat`)を
   再利用した**。発注書は「リポ内に新規作成可」であって必須ではなく、既存 3 本は
   NEW/LEGACY/複数 CoBo(cobo 抜けキーのバグ再発防止)をすべて代表しているため、
   同じファイルを Rust 側 `tests/geometry_*.rs` と共有する方が「本当に同じ入力を
   読んでいる」という一致テストの主張が強くなると判断した。
4. **Rust 実装(`src/geometry.rs`)と C++ 版原本(`~/test/get/tpcdaq/src/geometry/geometry.cpp`)
   の食い違い**(発注書指示どおり Rust を正として実装。差分自体は既に SPEC §4.1/§4.2 と
   TODO/002 で決定済みのものだが、確認のため再掲):
   - 原本のルックアップキーは `{asad, aget, graw_ch}`(**cobo が無い**)。複数 CoBo で
     衝突する(rust_reference と同じ欠陥、SPEC §4.2 が明示的に修正)。
   - 原本は NEW(10 欄)形式の固定 `>>` 抽出のみで、**LEGACY(7 欄)を判別しない**
     (7 欄行を食わせると `iss` が失敗して黙って `continue` される)。
   - 原本は **AUX 行を一切パースしない**(`U`/`V`/`W` で始まらない行は
     `parse_header_line` に渡り、`:` が無ければ何もせず黙って無視される)。
   - 原本は **FPN を独立した役割として扱わない**。raw {11,22,45,56} は
     `by_channel_` に一切登録されないので、「FPN」と「ファイルに記載なし」が
     区別できない(Rust/本実装は `ChannelRole::Fpn` として明示的に可視化)。
   - 原本は **malformed 行 / 重複チャンネルを一切カウントしない**(`if (!iss) continue;`
     で silent に握りつぶす — CLAUDE.md「silent failure を作らない」に反する)。
   - データ構造も `std::map<{asad,aget,graw_ch}, StripInfo>` の 2 本立て
     (`by_plane_strip_` / `by_channel_`)であり、Rust/本実装の「累積 AsAd オフセット
     方式の稠密配列」とは異なる(結果を左右する差ではなく実装戦略の違い)。
   いずれも「意味論の正は Rust」という発注書の裁定どおり、本実装は Rust 側に合わせて
   ある(原本からは NEW 10 欄の判別ロジックと FPN リオーダ表の着想のみを引き継いだ)。

### 変更・新規ファイル一覧

- `tools/root_sink/geo.hpp`(新規、717 行)
- `tools/root_sink/test_geo.cpp`(新規、446 行)
- `tools/root_sink/run_geo_conformance.sh`(新規、実行権限付与済み)
- `tools/root_sink/Makefile`(`TESTS` に `test_geo` を追加 + `test_geo` ビルドルール +
  `test:` ターゲットに `./test_geo` を追加。他の変更なし)
- `src/bin/geometry_dump.rs`(新規、24 行。`src/lib.rs` 改変なしで `tpcdaq::geometry` を
  呼ぶだけの薄い bin)
- 合成フィクスチャの新規追加なし(TODO/002 の既存 3 本を再利用 — 上記「逸脱」3 参照)

### 最終レビュー(2026-08-13 Fable)

- **判定: 受理(COMPLETED)**。逸脱 1〜3 受理(特に 3 のフィクスチャ再利用は一致テストの主張を
  強める正しい判断)。4 の原本との差分列挙は §4.1/§4.2 決定の再確認として価値がある記録。
- レビュー側で独立再検証: make test 全 green(test_geo 426 新規)、run_geo_conformance.sh が
  合成 3 本 + **実 .dat 両方(mini 272 行 / ELITPC 1088 行)でバイト一致** exit 0。
- 追加所見(レビュー時確認): HIGS2026_online の実 `geometry_ELITPC.dat` は**全 1087 ch が
  cobo 0 の単一 CoBo 形式**(ELITPC フルチャンネル数だが 2 CoBo 配分なし)。
  **SPEC §13-7 の Warsaw 確認事項(2-CoBo .dat の有無)は未解決のまま** — 019 以降のヒスト・
  E2E で ELITPC 構成を試す際は合成 2-CoBo フィクスチャを使い続けること。
