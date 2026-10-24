# 183 — root_sink: PEventTPC(TPCReco 形式)の書き出しをオプションに(mini の本番は off)

**Status: COMPLETED**(2026-10-24 ELITPC 反映・計測。起票 Opus、実装 Sonnet implementer。ユーザー「TPCReco はワルシャワ向けで我々には不要です。これはオプション扱いにしたほうが良さそうです」)

## 0. 背景
- 182 の実測: root_sink の律速は PEventTPC の充填 + 直列化(1 本 ≈ 60 ms CPU / 事象 = 16.6 Hz、raw ツリーの費用 ≈ 0)。PEventTPC のファイルは 555 kB / 事象(graw の 2 倍)。
- 我々(mini)の解析は raw ツリー(`run{N}_raw.root`、macros)と graw だけを使う。PEventTPC は TPCReco / Warsaw 向け。必要なら graw から grawToEventTPC で後から作れる(CLAUDE.md「grawToEventTPC が我々の .graw を無改造で読めることで接続」)。
- root_sink にはすでに `--no-root`(PEventTPC の TTree を書かない。raw ツリー `--raw-tree` とは独立)がある。ただし **`run{N}_monitor.root`(SPEC §6.5 R10、run 終わりのヒスト)は Recorder(PEventTPC 側)が書いている**ので、`--no-root` だと出なくなる。

## 1. 変更(決定済み)
### A. root_sink(`tools/root_sink/`)
- **`--no-root` かつ `--raw-tree` かつヒストが有効(`--geometry` あり)のとき、`run{N}_monitor.root` を raw ツリーの側で書く**。
  - 今の `Recorder::write_monitor_root`(`root_recorder.hpp:743`)の中身(TH1D / TH2D への写しと TFile への書き出し)を、Recorder に依存しない自由関数
    (例: `rsmon::write_monitor_root_file(const std::string& run_dir, uint32_t run, const HistSnapshot&, int compression)` を `monitor_hist.hpp` か新ヘッダに)へ切り出し、
    Recorder はそれを呼ぶだけにする(**PEventTPC あり時の挙動・ファイルはバイトの意味で不変**)。
  - raw の経路: RunClose のとき、recorder が無ければ raw チャネルの RunClose 項目にヒストのスナップショットを載せ(今 `RecordItem` に載せているのと同じ作り、`root_sink.cxx:915–921`)、
    raw スレッドが raw ツリーを finalize した**後**に書く(R10: EOS から 10 s 以内、0 事象の run には書かない = 今と同じ規則)。
  - recorder も raw writer も無い(`--no-root` だけ = テスト用の数えるだけの道)ときは今のまま書かない。
- 使い方の文言(`usage()`)の `--no-root` の説明を「PEventTPC (TPCReco) の TTree を書かない。`--raw-tree` があれば monitor.root は raw 側で書く」に。
### B. 起動スクリプト `tools/deploy/daq_up.sh`
- `[root_sink]` の新キー **`pevent`**(`true` / `false`)を `toml_get` で読み、`false` なら `--no-root` を渡す。**キーが無ければ true**(今の挙動)。
- 起動時の `echo "config: …"` の行に `pevent: <値>` を足す。
- `pevent = false` で `raw_tree` も false なら fail(何も書かない設定を黙って許さない — 「pevent = false needs raw_tree = true」)。
### C. 設定の雛形 `config/daq_example.toml`
- `[root_sink]` に `pevent = true   # PEventTPC(TPCReco 形式、Warsaw 向け)。false = raw ツリー + graw だけ(mini の本番、TODO/183)` を足し、冒頭コメント(daq_up.sh が読むキー)に `pevent` を追加。
### D. 文書
- `docs/SPEC_ja.md`: §3.1 の `[root_sink]` の例に `pevent`、§6(root-sink の責務 — 263 行の表と §6.4 / §6.5 の該当箇所)に「PEventTPC は **オプション**(既定 on = 互換、`pevent = false` で off。off のとき run の成果物は graw + `run{N}_raw.root` + `run{N}_monitor.root`)」。
  先頭 status を **v1.52(2026-10-24)**、改訂履歴の末尾に v1.52 の 1 項目(契機 = 182 の実測、ユーザー裁定)。`docs/en/` は触らない。
- `docs/RUNBOOK_ja.md`: §7.1 か設定の節に 1 行(`pevent` の意味と、off なら PEventTPC が要るときは grawToEventTPC で graw から)。

## 2. テスト(受け入れ、TDD — 先に赤を確認)
- **T1**(C++ か Rust 統合テスト — 既存の流儀に合わせる。`tests/root_sink_monitor_pub.rs` / `tests/p3_e2e.rs` / `tools/root_sink/test_recorder.cxx` を参照):
  `root_sink --no-root --raw-tree --geometry <合成 or 既存のテスト用ジオメトリ>` に 1 run 流して EOS → `run{N}_raw.root` と **`run{N}_monitor.root` ができる**、`run{N}.root`(PEventTPC)は**できない**。monitor.root の中のヒストの名前・ビン数が PEventTPC あり時と同じ。
- **T2** 0 事象の run では monitor.root を書かない(raw 側でも)。
- **T3** PEventTPC あり(従来)の monitor.root が改修前と同じ(既存テストが無改変で green)。
- **T4** `daq_up.sh` の `pevent` の解釈: スクリプトの単体テストが無ければ、`toml_get` と引数組み立ての部分を確かめる小さなシェルテスト(`tools/deploy/test_daq_up_args.sh` など、実プロセスは起こさない)を足すか、`bash -n` + 手で確かめた内容を報告(どちらにしたか書く)。
- 全体: `cargo fmt && cargo clippy --tests -- -D warnings && cargo test` green、root_sink の `make … -j` と `make … -j test`(Mac: `make TPCDAQ_TPCRECO_DIR=<reference/TPCReco の該当パス> -j`、SDKROOT=MacOSX27.sdk — 既存の Makefile と README の手順どおり)green。

## 3. 報告
- `git diff --stat`(変更してよい: `tools/root_sink/*`、`tools/deploy/daq_up.sh`(+ 新しいテストがあればそれ)、`config/daq_example.toml`、`docs/SPEC_ja.md`、`docs/RUNBOOK_ja.md`、`tests/*.rs` の root_sink 関係)、
  T1–T4 のテスト名と PASS / FAIL、cargo / C++ のテスト数、**実行できなかった操作**(無ければ「なし」)。コミットしない。

## 4. 範囲外(主対話が後で)
- ELITPC への反映(daq.toml に `pevent = false`、root_sink の make、daq_down / up)と、182 の V5(pevent off + raw ツリー)の計測。
- receiver の送信スレッド共用(graw が ROOT の遅さの巻き添えになる)= 181 の設計、Fable。

## 結果
### 実装(2026-10-24、Sonnet implementer、主対話 Opus が照合)
- 変更 9 ファイル(+290 / −67)+ 新規 `tools/deploy/test_daq_up_args.sh`: `tools/root_sink/{root_recorder.hpp, raw_tree.hpp, root_sink.cxx, test_raw_tree.cxx}`、`tools/deploy/daq_up.sh`、`config/daq_example.toml`、`docs/{SPEC_ja.md, RUNBOOK_ja.md}`、`tests/root_sink_monitor_pub.rs`。範囲内。
- 中身: 自由関数 `rootsink::write_monitor_root_file(dir, run, hists, compression)`(`root_recorder.hpp` に置いた — `monitor_hist.hpp` は ROOT 非依存、新ヘッダは循環 include になるため)、
  `Recorder::write_monitor_root` はそれを呼ぶだけ(PEventTPC on の挙動は不変)、`RawTreeWriter::write_monitor_root`、`RawItem.hists`(rec が無いときだけ載せる = 二重に書かない)、
  raw スレッドは `close_run` の後に書く。`snapshot_final_hists()` に切り出し。`daq_up.sh` の `pevent_args`(無ければ true、false → `--no-root`、false + raw_tree≠true と true/false 以外は fail)、`config:` 行に `pevent:`。SPEC v1.52(§3.1・§6.4・§6.5 表・改訂履歴)、RUNBOOK §7.1 に 1 行。
- テスト: T1 `tools/root_sink/test_raw_tree.cxx::test_raw_side_writes_monitor_root_without_pevent`(ヒストの名前・ビン数・軸・値、PEventTPC の run.root が無い)+ e2e `tests/root_sink_monitor_pub.rs::no_root_with_raw_tree_still_writes_monitor_root`(実バイナリ、3 事象 + EOS)、
  T2 `test_raw_side_zero_event_run_writes_no_monitor_root`、T3 既存の monitor.root テスト 3 本が無改変で green、T4 `tools/deploy/test_daq_up_args.sh`(9 項目、実プロセスなし)。
  red の確認は C++ の T1 / T2 のみ(実装前はコンパイルエラー)。e2e と T4 は先に red を走らせていない(TDD の順番からの逸脱として記録)。
- **主対話の再実行(Mac、2026-10-24 18:45)**: `make -j` OK、`make -j test-root`: test_recorder 174 / test_pevent 163 / test_raw_tree 107 passed・0 failed、`make -j test`: tpc_wire 68 / rs_core 98 / eb_core 391 / geo 426 / monitor_hist 202 / monitor_pub 92 / ped_table 27 passed・0 failed。
  `cargo fmt --check` / `clippy --tests -D warnings` OK、`TPCDAQ_ROOT_SINK_BIN=… cargo test` **1109 passed / 0 failed / 1 ignored**(新しい e2e を含む — 単独でも 1 passed)。`test_daq_up_args.sh` 全 ok。
- 実行できなかった操作: なし(implementer 報告)。残ったプロセスなし(主対話が確認)。
### ELITPC への反映 1 回目(2026-10-24 19:05–19:20、ユーザー「了承します。進めてください」)— **失敗して戻した**
- `cffcbdb` を bundle → ff → cargo build(Rust 不変)→ root_sink の make / make test / make test-root(全 0 failed)→ daq.toml の `[root_sink]` に `pevent = false`(退避 `daq.toml.bak-2026-10-24-183`)→ 計測スクリプト `~/test/bench183/`(V5 10 ms / V6 3 ms / V7 1 ms)。
- **daq_up で monitor が起動しなかった**: `unknown field 'pevent', expected one of snapshot_hz, event_publish_hz, build_timeout_ms, raw_tree` —
  Rust の `RootSinkConfig` は `deny_unknown_fields` で、implementer は daq_up.sh と雛形だけに `pevent` を足し Rust の設定構造に足していなかった(発注書も Rust の config に触れていなかった = **発注書の不備**)。
  cargo test が雛形 `config/daq_example.toml` を読むテストを持たなかったので見逃した。計測は 3 本とも `set_variant failed`、最後の restore も controller 不在で params を戻せなかった。
- 復旧(19:2x): daq.toml を退避から戻す(失敗版は `daq.toml.failed-pevent-2026-10-24`)→ daq_down / up → status 200・`/` 200・Idle・8 + root_sink(PEventTPC あり = 従来)。
  params を物理に(API、operator claude: `triggerMode external`、12.5 MHz・120 fC・full・delay 2500、id `elinp__818acd22`)。**DAQ が使えなかったのは ≈ 19:11–19:2x**。
### 修正(主対話 Opus、TDD)
- `src/config.rs` `RootSinkConfig` に `pevent: bool`(`#[serde(default = "default_pevent")]` = true)。Rust 側は値を使わない受け口。
- テスト(先に red = `no field pevent` のコンパイルエラーを確認): `root_sink_pevent_defaults_to_true_when_omitted`、`root_sink_pevent_false_is_read`、
  **`example_root_sink_section_is_accepted`**(`include_str!("../config/daq_example.toml")` の `[root_sink]` 節をそのまま最小構成に差し込んで parse — daq_up.sh だけが知っているキーを足したら赤になる)。
- root_sink の起動ログ: `--no-root` + `--raw-tree` のとき「no PEventTPC TTree (the raw tree and run{N}_monitor.root are still written)」(以前は「counting only, no TTree is written」で誤解を招いた)。
- Mac: `cargo fmt --check` / `clippy --tests -D warnings` OK、`TPCDAQ_ROOT_SINK_BIN=… cargo test` **1112 passed / 0 failed / 1 ignored**、root_sink `make -j test-root` 0 failed。
### ELITPC への反映 2 回目(2026-10-24 19:20–19:24)
- `2911209` を bundle → ff → cargo build --release(15.8 s)→ root_sink の make → daq.toml に `pevent = false`(退避 `daq.toml.bak-2026-10-24-183b`)→ daq_down / up(失敗したら自動で戻す手順 — 不要だった)。
- 確認: Idle・status 200・`/` 200、`pevent: false`、root_sink のログ「--no-root: no PEventTPC TTree (the raw tree and run{N}_monitor.root are still written)」+「writing raw TTree」、7 プロセス + root_sink(pgrep -f は自分の bash にも当たるので以前の「8」は 7 + bash)。
- 182 の V5–V7(run 202–204、周期 10 / 3 / 1 ms): 78 / 169 Hz に完全追従、263 Hz で ≈ 220 Hz 頭打ち、溢れ 0、全 run 正常に閉じ、monitor.root は raw 側で書けた(182 の結果節)。
- 終了時: daq.toml は `pevent = false` のまま(本番の設定)、params は物理(external、12.5 MHz・120 fC・full)。
