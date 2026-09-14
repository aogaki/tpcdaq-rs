# 075 — root-sink raw ツリー出力(macros 解析チェーン直結)

**Status: COMPLETED**(2026-09-14 着手・同日完了 — ユーザー指示「TODO/075 進めてください」。
裁定 = SPEC v1.26 §6.7、実装前の精度上げ = **SPEC v1.28 §6.7(Fable、同日)**。結果は末尾「## 結果」)

**正 = SPEC §6.7(v1.28)。契約の現物 = [macros/graw2root.C](../macros/graw2root.C)**(読んで
一致させる。下流 = macros/analyzeUVW.C が `SetBranchAddress` する branch 名・型が凍結対象:
`eventId`(UInt_t)/ `adc`(`Short_t adc[4][68][512]`)。`eventTime`(ULong64_t)は契約に含める)。

## 目的

mini eTPC のスタンドアロン解析チェーン(macros/)を**下流無変更**で DAQ に直結する。
root-sink が run 毎に `run{run:04}_raw.root` を書けば、graw2root.C(変換ステップ)は不要になる。

## レーン分割(並列。ファイルは重ならない)

| レーン | モデル | 触るファイル |
|---|---|---|
| **A: C++ 本体 + テスト + 実データオラクル** | implementer **Opus** | `tools/root_sink/raw_tree.hpp`(新規)/ `root_sink.cxx` / `test_raw_tree.cxx`(新規)/ `compare_raw.cxx`(新規)/ `Makefile` / `tests/mini_rawtree_e2e.rs`(新規) |
| **B: Rust 設定キー + 負荷ハーネス pass-through** | implementer **Sonnet** | `src/config.rs` / `src/bin/soak_harness.rs` |
| **C: §12-8 変形の実測(raw on/off スループット)+ 一括レビュー + 結果節** | Fable(本セッション) | 本チケット / CURRENT.md / archive |

## A. 発注内容(C++)

### A-1. `tools/root_sink/raw_tree.hpp`(新規、ROOT 依存ヘッダ)

include してよいのは `root_sink.cxx` / `test_raw_tree.cxx` だけ(root_recorder.hpp と同じ規約)。
ROOT 非依存ヘッダ(tpc_wire / rs_core / eb_core)には触らない。**TPCReco には依存しない**
(PEventTPC もジオメトリも要らない)。

```cpp
namespace rootsink {
constexpr const char* kRawTreeName = "raw";
struct RawTreeConfig {
  std::string output_root = ".";
  int compression = kDefaultCompression;   // --root-compression と同じ値を渡す
  std::set<FragmentKey> expect;             // --expect(§6.3)。空は設定ミス → ctor で fatal
};
class RawTreeWriter {
 public:
  explicit RawTreeWriter(RawTreeConfig cfg);
  void write(const BuiltEvent& ev, uint64_t now_ms);   // run 切替は内部で検知(遅延オープン)
  void close_run(uint32_t run);                         // EOS: finalize → run{run:04}_raw.root
  void shutdown();                                      // 停止: inprogress のまま閉じる
  void tick(uint64_t now_ms);                           // AutoSave 期限(30 s、kAutoSaveIntervalMs)
  // 他スレッドから読まれるものは atomic(root_recorder.hpp の RecorderWorker と同じ流儀)
  bool is_open() const; uint64_t entries_written() const; uint64_t run_entries() const;
  uint64_t bytes_written() const; uint64_t items_out_of_range() const;
  uint64_t unexpected_fragments() const;
  std::vector<RootFileRecord> files_snapshot() const;
  const std::string& leaflist() const;     // 例 "adc[4][68][512]/S"(テスト用)
  const std::string& provisional() const;  // 開いている inprogress パス(テスト用)
  const char* fatal_reason() const; const std::string& fatal_detail() const;
};
}
```

- **adc 配列**: `nAsad = cfg.expect.size()`、`std::vector<Short_t>` 長 `4×nAsad×68×512`。
  リーフリスト文字列 `adc[{4×nAsad}][68][512]/S` を ctor で組む。AsAd ブロック index =
  期待集合を昇順に並べたときの (cobo,asad) の順位(`std::set` は既に昇順)。
- **ブランチ**(graw2root.C と同じ名前・型):
  `Branch("eventId", &event_id_, "eventId/i")` / `Branch("eventTime", &event_time_, "eventTime/l")` /
  `Branch("adc", adc_.data(), leaflist.c_str())`。ツリー名 `raw`。タイトルは自由(契約外)。
- **fill**: 毎イベント `std::fill`(memset)で 0 → 各フラグメントを順に展開。item 32bit
  (SPEC §2.4 / pevent_fill.hpp `scatter_items` と同じ): `aget=(w>>30)&3`、`chan=(w>>23)&0x7F`、
  `bucket=(w>>14)&0x1FF`、`adc=w&0xFFF` → `adc_[((block*4+aget)*68+chan)*512+bucket]`。
  `chan>=68` は書かず `items_out_of_range_` +1(初回のみ stderr 1 行)。期待集合外の
  (cobo,asad) は書かず `unexpected_fragments_` +1(初回のみ 1 行)。
  `eventId = ev.event_idx`、`eventTime = ev.fragments.front().event_time`(eb_core が
  (cobo,asad) 昇順に整列済み。fragments が空なら書かない)。incomplete は届いた分だけ。
- **ライフサイクル**(RecorderWorker と同じ規則、名前空間だけ別):
  遅延オープン(0 イベント run はファイルを作らない)/ 書き込み中 `run_raw_inprogress_<unixtime>.root`
  (同秒衝突は `_2`,`_3`… — pick_provisional と同じ)/ `close_run` で `run{run:04}_raw.root` へ rename
  (既存なら **rename しない**で ERROR + inprogress 維持)/ `shutdown` は inprogress のまま /
  run 中に別 run のイベントが来たら PROTOCOL VIOLATION を出して旧ファイルを finalize せずに閉じる /
  `Write("", kOverwrite)` / AutoSave("SaveSelf") 30 s(write と tick 両方から)/ 100 エントリ毎
  `FlushBaskets` / **ロールオーバ無し** / ctor で `TTree::SetMaxTreeSize(max)`(冪等)。
  出力先は `run_dir_of(output_root, run)`(PEventTPC と同じ run ディレクトリ)。
- **fatal**: mkdir / TFile::Open 失敗 → `set_fatal("raw-mkdir" / "raw-open", detail)`。
  呼び手(root_sink.cxx)が既存と同じく `fatal(counts, kExitRootIo, …)` で落とす。
- root_recorder.hpp の小道具(`mkdir_p` / `path_exists` / `file_size_bytes` / `run_dir_of` /
  `RootFileRecord` / `kDefaultCompression` / `kAutoSaveIntervalMs`)は **include して再利用**
  (raw_tree.hpp が root_recorder.hpp を include してよい。逆は不可)。

### A-2. `root_sink.cxx` の配線

- `Options::raw_tree`(既定 false)、フラグ `--raw-tree`、usage に 1 行。
  **`--no-root` と独立**(`--no-root --raw-tree` は raw のみ。`--geometry` 必須条件は
  PEventTPC を書くときのまま = 変更しない)。
- `gROOT->SetBatch` / `ROOT::EnableThreadSafety` は `(!no_root || raw_tree)` で呼ぶ。
- `RawItem { Kind{Event, RunClose}; run_number; BuiltEvent event; }`、
  `Channel<RawItem>` 深さ `kRecorderQueue`(16)。**専用スレッド**(record_thread と同じ形:
  `pop_for(kRecorderTickMs)` → write / close_run / tick、fatal 検知で `fatal(...)`、Closed で `shutdown()`)。
- `absorb()`: raw チャネルがあれば **rec への move の前に** `BuiltEvent` を 1 回コピーして push
  (rec が無い = `--no-root` なら move)。push は満杯なら待つ(背圧、捨てない)。
- EOS Closed: `eb.flush()` を流した**後**に RunClose を raw チャネルにも流す(in-band)。
- main の終了順: aggregate.join → rec_chan close/join → **raw_chan close/join** → publisher。
- 起動ログ 1 行(何を書くかを黙って決めない): `root_sink: writing raw TTree "raw" under <dir> (adc[N][68][512]/S, single writer, no rollover)`。
- **終了 JSON**(print_counts_json)に追加: `"raw_tree":0|1`, `"raw_entries_written"`,
  `"raw_items_out_of_range"`, `"raw_unexpected_fragments"`。raw のファイルは既存
  `root_files` 配列に**続けて**載せる(path/entries/bytes)。`g_recorder` と同様に
  `std::atomic<RawTreeWriter*> g_raw` を置き、fatal 時にも見えるようにする。
  status(§5.3)の `bytes_written` は **PEventTPC のまま**(意味を変えない)。

### A-3. `tools/root_sink/test_raw_tree.cxx`(新規、ROOT のみリンク、TPCReco 不要)

check.hpp の CHECK/CHECK_EQ + main 方式(test_recorder.cxx と同じ)。小道具(`scratch_dir` /
`remove_tree` / `pack_item` / `push_item` / `make_fragment`)は test_recorder.cxx から**コピー**
(ヘッダ化はしない — KISS)。読み戻しは **analyzeUVW.C と同じ書き方**で行う
(`TTree* tree=(TTree*)in.Get("raw"); UInt_t eventId; ULong64_t eventTime; static Short_t adc[4][68][512];
SetBranchAddress(...)`)。最低限:

1. **契約一致(mini, expect={0:0})**: 3 イベント(event_idx 5,6,7、各 2〜3 item を aget/chan/bucket を
   ばらして置く)→ close_run → 読み戻し: エントリ数 3、`eventId` 列 = 5,6,7、`eventTime` =
   各 fragment.event_time、置いた item の値一致、置いていない要素は 0。TLeaf:
   `GetLeaf("eventId")->GetTypeName()=="UInt_t"`、`eventTime`=="ULong64_t"、`adc`=="Short_t"、
   `GetLeaf("adc")->GetLenStatic()==4*68*512`、ブランチ数 3、`leaflist()=="adc[4][68][512]/S"`。
2. **ELITPC 形(expect={0:0,0:1})**: `leaflist()=="adc[8][68][512]/S"`。asad1 のみの incomplete
   イベント → ブロック 0 全 0・ブロック 1 に値、`eventTime` = その fragment のもの。
   両方揃ったイベント → `eventTime` = asad0 のもの。
3. **ライフサイクル**: 書き込み中は `run_raw_inprogress_` 始まりで `run00NN_raw.root` は無い →
   close_run 後は `run00NN_raw.root` があり inprogress は無い / shutdown(EOS 無し)は inprogress
   のまま残り `_raw.root` 無し / 0 イベント run はファイル無し(`files_snapshot` 空)/
   既存 `run00NN_raw.root` があるときは上書きせず inprogress 維持 + `files_snapshot` のパスが
   inprogress。
4. **防御カウンタ**: chan 68 と 127 の item → `items_out_of_range()==2`、書かれない /
   期待集合外 (1,0) のフラグメント → `unexpected_fragments()==1`、書かれない。
5. **compare_raw の負性対照**(test_recorder の `test_compare_pevent_detects_a_one_adc_difference`
   と同じ流儀): 同内容の 2 ファイル → `./compare_raw a b` exit 0 / 1 要素だけ ADC を変えた
   ファイル → exit 非 0 かつ出力に `1 differences` / エントリ数違い → exit 非 0。

### A-4. `tools/root_sink/compare_raw.cxx`(新規、ROOT のみ)

`compare_raw <a.root> <b.root>`: 両方の tree `raw` を開き、リーフ型と adc 長の一致、エントリ数
(不一致なら両方の数を出す)、全エントリの eventId / eventTime / adc 全要素を比較。差分は先頭
10 件(entry / field / index / a / b)と総数を印字、最後に `compared N events, D differences`。
exit 0 = 全一致、1 = 差分あり、2 = open / tree 無し。**compare_pevent と同じ役割**(§12 の
オラクル照合はこれ 1 本で機械判定)。

### A-5. Makefile

`ROOTTESTS += test_raw_tree`、`test_raw_tree: test_raw_tree.cxx raw_tree.hpp root_recorder.hpp …`
(ROOT のみ。**TPCReco の辞書・ソースはリンクしない**)、`compare_raw: compare_raw.cxx`、
`compare: compare_pevent compare_raw`、`test-root` に `./test_raw_tree` を追加(compare_raw も
依存に)、`root_sink` の依存に `raw_tree.hpp`、`clean` に追加。ヘッダコメントの一覧も更新。
`make test`(ROOT 非依存 7 本)は**無変更**で通ること。

### A-6. `tests/mini_rawtree_e2e.rs`(新規、env-gated 実データオラクル = §6.7 受け入れ)

**テンプレ = tests/elitpc_pevent_e2e.rs**(スタック起動・rpc Start・graw_writer 代役の PULL bind・
EOS 待ち・JSON 回収・compare 呼び出しの流儀をそのまま流用)。環境変数:
`TPCDAQ_ROOT_SINK_BIN`(compare_raw も同じ dir)/ `TPCDAQ_REAL_GRAW`(2025 mini 実 .graw =
`~/TPC/CoBo_2025-09-01T08_51_06.203_0000.graw`、108 frames、compact)/ `TPCDAQ_REAL_GEOMETRY_MINI`
/ `root` が PATH に無ければ SKIP(理由を出す)。未設定は SKIP。

手順: 一時 dir に **.graw のシンボリックリンク**を置く(graw2root.C は入力の隣に
`<stem>_raw.root` を書く — ユーザーの `~/TPC` を汚さない)→ root_sink を `--expect 0:0 --geometry
<mini.dat> --raw-tree --run-id <固定> --output-root <tmp>` で起動 → decoder / receiver → Start(run=RUN)
→ graw_replay(`--rate-mbps 100` 程度)→ EOS 到達 → `run{RUN:04}_raw.root` の出現を待つ → SIGTERM
→ JSON: `entries_written==108`、`raw_entries_written==108`、`raw_items_out_of_range==0`、
`raw_unexpected_fragments==0`、`events_incomplete==0` → `root -l -b -q '<repo>/macros/graw2root.C("<link>")'`
(環境は継承 — Mac は `SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX26.sdk` が要る、
メモリ mac-root-cling-sdkroot)→ `compare_raw <run dir>/run{RUN:04}_raw.root <tmp>/<stem>_raw.root`
が exit 0 で `compared 108 events, 0 differences` → **負性対照**: graw2root.C を
`maxEvents=107` で再実行した出力との比較が exit 非 0(比較器が差を見られることの証明)。

### A-7. ゲート(A の完了条件)

`make -j8 test-root`(test_recorder 170 / test_pevent 99 / **test_raw_tree 新規**)全 green、
`make test` 無変更 green、`cargo test --test mini_rawtree_e2e`(env 付き)green、
`cargo fmt && cargo clippy --tests -- -D warnings && cargo test` クリーン。
**実行コマンドと出力の要点(JSON 1 行・compare_raw の出力)を報告に含めること**。

## B. 発注内容(Rust)

### B-1. `src/config.rs`

`RootSinkConfig` に `#[serde(default)] pub raw_tree: bool`(SPEC v1.28 §3.1 / §6.7 への参照を
doc コメントに)。`deny_unknown_fields` のため無いと TOML に書けない。既存テストの構造体リテラル
があれば追従。テスト追加: 省略時 false / `raw_tree = true` が読める。
(現状 root_sink を TOML から起動するランチャは無い — キーは仕様どおりの受け口。将来のランチャが
`--raw-tree` に写像する。root_sink の CLI 側はレーン A)。

### B-2. `src/bin/soak_harness.rs`

`--raw-tree` フラグ(既定 off)→ root_sink 起動引数に `--raw-tree` を**フラグがあるときだけ**
追加(`--recorder-workers` と同じ「既定では渡さない」流儀)。USAGE 追記。run 合格判定に
「`--raw-tree` のとき run dir に `run{run:04}_raw.root` が存在すること」を加える(無ければ
その run は FAIL — 黙って通さない)。出力の削除(`--keep-outputs` 無し時)は run dir ごとなら
そのままでよい。args パーサにテストがあれば 1 本追加。

### B-3. ゲート

`cargo fmt && cargo clippy --tests -- -D warnings && cargo test`(config / soak_harness の
ユニット)green。**soak_harness の実走は不要**(C で Fable が回す)。

## C. Fable(§12-8 変形 + レビュー)

`soak_harness --mode burst --runs 3 --burst-min 1` を `--raw-tree` 無し / 有りで回し、PEventTPC
書き出しの events/s が ±5% 以内であることを実測(A/B 完了後)。両レーンの diff + テスト出力を
一括レビューし、本チケットに `## 結果` を書いて archive、CURRENT.md 更新、コミット。

## 注意(監査で判明した契約の細部)

- graw2root.C は 2026-09-14 に **frameType 1/2 両対応**になった(compact = per-AGET カーソル)。
  本ユニットは built event 由来なので frameType 非依存(decoder が吸収)。
- **2026 pulser 実データ(reference/exp_data/2026/pulser)はオラクルに使えない** — ELITPC 4 AsAd
  の単一ファイル形式で、graw2root.C は 1 フレーム = 1 エントリ(AsAd 毎)に書くため built event
  (4 AsAd マージ)と合わない。オラクルは 2025 mini(1 AsAd)で行う。
- eventTime はイベント先頭フラグメント基準。mini(1 AsAd)では graw2root.C と自明に一致。
- macros/ 側は**一切変更しない**(graw2root.C はオラクルとして凍結)。
- Mac の ROOT は Xcode CLT 27 更新で ACLiC/cling が壊れている → `SDKROOT` 回避策(メモリ
  mac-root-cling-sdkroot)。`make` のコンパイル自体は clang++ 直なので影響しないはず。
  **作業中にユーザーが /opt/ROOT を 6.40.04 に入れ替えた**(旧 6.36.10 は /opt/ROOT.bu)。6.40.04 は
  `SDKROOT=…/MacOSX27.sdk` が必要(rootcling の辞書生成も)。以下の結果は全て 6.40.04 で再ビルドして取った。

## 結果(2026-09-14、Mac(macOS 25.6 / Apple Silicon)/ ROOT 6.40.04 / Rust release)

### 実装物

- 新規: `tools/root_sink/raw_tree.hpp`(`RawTreeWriter`、418 行)/ `test_raw_tree.cxx`(609 行、88 CHECK)/
  `compare_raw.cxx`(209 行)/ `tests/mini_rawtree_e2e.rs`(452 行)。
- 変更: `root_sink.cxx`(`--raw-tree`、専用スレッド + `Channel<RawItem>` 深さ 16、in-band RunClose、
  終了 JSON の `raw_tree`/`raw_entries_written`/`raw_items_out_of_range`/`raw_unexpected_fragments` +
  `root_files` 追記、`--no-root` と独立)/ `Makefile`(`test_raw_tree`・`compare_raw`、test-root に追加)/
  `.gitignore` / `src/config.rs`(`[root_sink] raw_tree`、テスト 2)/ `src/bin/soak_harness.rs`
  (`--raw-tree` pass-through + raw ファイル存在・entries 照合、テスト 1)/ `macros/README.md`(DAQ 直結の注記)。
- Fable の修正 1 件: soak_harness の entries 集計が `run{N}_raw.root` の `finalized` 行も足して 2 倍に
  カウントし、初回計測で raw on 側が偽 FAIL(6048 → 12096)。raw を除外し、raw 側は別途
  「= laps × 108」で照合するよう強化(黙って通さない)。

### ゲート

| スイート | 結果 |
|---|---|
| `make -C tools/root_sink test-root` | test_recorder **170** / test_pevent **99** / **test_raw_tree 88(新規)**、全 0 failed |
| `make -C tools/root_sink test`(ROOT 非依存 7 本) | 68 / 71 / 175 / conformance SKIP / 426 / 202 / 92 — **無変更 green** |
| `cargo fmt` / `cargo clippy --tests -- -D warnings` | 差分なし / 警告ゼロ |
| `cargo test`(全体) | **472 passed / 0 failed**(47 スイート)— harness 修正後のコミット前ゲートで再確認(`cargo fmt --check` / `clippy --tests -D warnings` / `cargo test`、2026-09-14 16:0x) |

### (b) 実データオラクル(SPEC §6.7 受け入れ)

```
export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX27.sdk
TPCDAQ_ROOT_SINK_BIN=$PWD/tools/root_sink/root_sink \
TPCDAQ_REAL_GRAW=$HOME/TPC/CoBo_2025-09-01T08_51_06.203_0000.graw \
TPCDAQ_REAL_GEOMETRY_MINI=$HOME/TPC/miniTPC_UVW_pcb_info/new_geometry_mini_eTPC.dat \
cargo test --release --test mini_rawtree_e2e -- --nocapture      # ok. 1 passed (7.0 s)
```

- 2025 mini 実 .graw(108 frames、compact)を receiver → decoder → root_sink `--raw-tree` に通し、
  `run0001_raw.root`(108 entries、20,495,708 B)と `run0001.root`(108 entries)を finalize。
  終了 JSON: `events_complete=108 events_incomplete=0 entries_written=108 raw_tree=1
  raw_entries_written=108 raw_items_out_of_range=0 raw_unexpected_fragments=0 fatal=""`。
- graw2root.C(同じ .graw、一時 dir のシンボリックリンク経由)→ `compare_raw`:
  **`compared 108 events, 0 differences`**(exit 0、adc_len=139264 両側一致)。
- 負性対照: graw2root.C `maxEvents=107` の出力と比較 → **`compared 107 events, 1 differences` /
  `DIFF entries A=108 B=107`(exit 1)**。単体側でも 1 ADC 差(`1 differences`)とエントリ数差を検出。

### (c) §12-8 変形 — raw on/off で PEventTPC 書き出しスループット不変(±5%)

soak_harness `--mode burst --runs 3 --run-minutes 1 --recorder-workers 4`、mini 実 .graw ループ、
fake_ecc + ecc_bridge + controller + 全スタック(RSS/fd/全カウンタ監視つき)。

| 条件 | raw **off** | raw **on** |
|---|---|---|
| 224 Mbps(100 Hz 相当)× 3 run | 56 laps/run = 6,048 entries/run、3/3 合格。ビルド率 358,682 events/h | 56 laps/run = 6,048 + **raw 6,048**/run、60.3 s/run、達成 223.8 Mbps(1.00×)、3/3 合格。ビルド率 359,820 events/h(**+0.3%**) |
| 672 Mbps(3× 目標)× 3 run | 168 laps/run = 18,144 entries/run、3/3 合格。540,898 events/h | 168 laps/run = 18,144 + **raw 18,144**/run、60.3 s/run、達成 671.5 Mbps(3.00×)、3/3 合格。541,826 events/h(**+0.2%**) |

- 両レートともペーシング上限に張り付いたまま raw on/off で差なし(±5% 内)。ロスレスカウンタ
  (recv_overflow / seq_gaps / incomplete / late / pending / FATAL)は全構成で 0。
- raw ファイルサイズ(パルサーデータ、ZLIB-1): 6,048 events → 1.163 GB(**192 KB/event**)、
  18,144 → 3.461 GB。1 GiB 超の単一ファイルが問題なく finalize される(ロールオーバ無しの実証)。
- root_sink RSS(セッション末尾): 224 off 709 MB / on 719 MB、672 off 1,070 MB / on 1,233 MB。
  672 on の初回計測で中間サンプル 6.17 GB を観測 — 064 が記録した 672 Mbps 時の 6.88 GiB ピーク
  (`--queue` 1000 バッチ由来の一過性)と同じもので raw 固有ではない(off 側は同時刻のサンプル無し)。

### 逸脱・申し送り

1. `test_raw_tree` は**リンクは ROOT のみ**だが、raw_tree.hpp が root_recorder.hpp の小道具を再利用する
   ため**コンパイル時の include パスだけ** TPCReco が要る(発注書 A-1 の再利用指示を優先)。
2. E2E は raw に加えて PEventTPC の finalize も待ってから SIGTERM(`entries_written==108` の判定が
   finalize と競合しないため。判定条件は追加していない)。
3. 期待集合が空のときの ctor fatal は `raw-no-expect`(CLI からは到達不能)。
4. **未検証**: ELITPC 4 AsAd の raw(`adc[16][68][512]/S`)の実データ照合 — graw2root.C は 1 フレーム =
   1 エントリなのでオラクルを作れない。2 AsAd のブロック配置は単体テストで固定済み。
5. **GDED(ROOT 無し)では root_sink を動かせない**ので、実機での raw ツリーは新 DAQ 機に ROOT を
   入れてから(null_sink が代役の間は .graw + graw2root.C 経路)。
6. reference/_spike/demo(gitignore)に `TPCDAQ_RAW_TREE=1` で `--raw-tree` を渡す入口を追加(ローカルのみ)。
