# 011 — GDataFrame TTree 書き出し + third_party/get 隔離(C++/ROOT)

**Status: COMPLETED**
**仕様**: SPEC v1.4 §6.4(TTree = 本ユニットの核)、§6.5(ファイル命名・ライフサイクル)、
§6.6(third_party 隔離)、§6.2-8(run 境界でブロッキング外部 IO 禁止)、§6.3(BuiltEvent の順序は
010 で確定済み — (cobo,asad) 昇順)
**依存**: 010(COMPLETED — eb_core の `absorb()` が接続点、コメント明記済み)。ROOT 6.36.10 @ /opt/ROOT
**発注先想定**: implementer/Opus

## やること

1. **third_party/get/ 新設**(CLAUDE.md「GET 由来コード(CeCILL)は third_party/ に隔離」):
   - `reference/20190315_patched/CoBoFrameViewer/src/root/` から GDataFrame 一式をコピー:
     GDataFrame.{h,cpp} / GFrameHeader.{h,cpp} / GDataChannel.{h,cpp} / LinkDefGET.h
     (GDataSample が別ファイルならそれも。**コードは無改変**が原則 — 改変が必要なら停止して報告)。
   - `third_party/get/LICENSE`(CeCILL 本文)+ `third_party/get/README.md`(出自 =
     GET software 20190315 / CoBoFrameViewer、コピー日 2026-08-13、無改変の旨)。
   - **reference/ 自体はコミット禁止のまま**(third_party/get/ はコミット対象 — この区別を README に明記)。
2. **tools/root_sink に Recorder スレッド追加**(delila-rs `tools/root_sink/root_sink.cxx` +
   `sink_core.hpp` の Recorder 流儀を参考に。出自コメント明記):
   - 配線: 集計スレッド(EventBuilder)→ **有界 Channel<BuiltEvent>** → Recorder スレッド
     (ROOT IO はこのスレッドに隔離 — §6.2-8: run 境界の finalize/rename が intake を止めない)。
     Channel は既存 rs_core の Channel<T>(背圧 = ロスレス)。
   - TTree(SPEC §6.4 を一字違わず): ツリー名 `"tree"`、
     `Branch("GDataFrame", &frame, 32000, 99)`、**1 エントリ = 1 CoBo フレーム(= 1 Fragment)**、
     圧縮 505(ZSTD-5)。
   - **GDataFrame の充填が graw2root 互換の核心**。正 =
     `reference/20190315_patched/CoBoFrameViewer/src/graw2root/cobo-frame-graw2root.cpp`(本家)と
     `~/test/get/tpcdaq/src/`(C++ 版 tpcdaq の RootWriter — 実験で実際に使っている充填)。
     ヘッダは OwnedFragment の全フィールド(§2.4)から、items(u32 パック: [31:30] aget /
     [29:23] chan raw 0–67 / [22:14] bucket / [11:0] ADC)を (aget, chan) 毎の GDataChannel に
     展開、チャンネル内サンプルは **bucket 昇順で連続 AddSample**(標準リーダの前提)。
     `fHitPatterns` は未充填(C++ 版同様 — 生 graw がバックストップ、SPEC §6.4)。
     ジオメトリ変換はしない(chan は raw 0–67 のまま — FPN 込み。オフライン互換)。
   - **ファイルライフサイクル(§6.5)**: `<output_root>/run{run:04}/` に
     書き込み中 `run_inprogress_<unixtime>.root` → 全 EOS の finalize で `run{run:04}.root` へ
     rename。異常終了は inprogress のまま残す(完全 run に化けない)。rollover:
     ファイルサイズ > `--max-root-bytes`(既定 1 GiB)で現ファイルを finalize し
     `run{run:04}_{part:04}.root`(part=0001〜)へ。ROOT の自動分割(fgMaxTreeSize)は
     無効化して自前で制御(命名規則を ROOT 任せにしない)。
   - CLI 追加: `--output-root DIR`(既定 `.`)/ `--max-root-bytes N`。`--no-root`(TTree を書かず
     従来の数えるだけ — 既存テストの高速経路 + ROOT 無し環境の逃げ道)。
   - JSON カウンタ追加: entries_written / root_files(パス + entries + bytes)/
     recorder_queue(瞬間値)。既存カウンタ・既存動作(malformed exit 2 / seq exit 3 / 背圧)は不変。
3. **ビルド**: Makefile に ROOT を追加(`root-config --cflags --glibs`、辞書は `rootcling` で
   LinkDefGET.h から生成)。**既存の 4 テスト(tpc_wire / rs_core / eb_core / conformance)は
   ROOT 非依存のまま**(ヘッダテストに ROOT を混ぜない)。`make`(本体)のみ ROOT 必須。
4. テスト:
   - `test_recorder.cxx`(ROOT リンク、`make test-root` として分離 — `make test` は従来どおり
     ROOT 非依存): 合成 BuiltEvent(2 event × 2 fragment、既知の aget/chan/bucket/ADC)を書き、
     **同プロセスで TFile を開き直して読み戻し**: エントリ数 = fragment 総数、ヘッダ全フィールド
     一致、チャンネル数・サンプル値・bucket 順を機械照合。rename ライフサイクル
     (inprogress → run{run:04}.root)と rollover(小さい --max-root-bytes で part ファイル)も検証。
   - `tests/root_sink_intake.rs` に 1 本追加(env ゲート既存踏襲): Data + EOS → SIGTERM 後、
     `run{run:04}.root` が存在し inprogress が残っていないこと + JSON の entries_written 一致。
   - **E2E(env `TPCDAQ_REAL_GRAW` + `TPCDAQ_ROOT_SINK_BIN`)**: graw_replay → receiver(006)→
     decoder(009)→ root_sink で run ファイル生成、**entries = 108**(P1 オラクル)を
     test_recorder 側の読み戻しユーティリティ or root マクロで照合。実測値を `## 結果` に記録。
     ※ C++ 版 tpcdaq の ROOT 出力との TTree 全値比較(§12-3)は 012 の仕事 — ここではやらない。

## 受け入れ

- `make test`(ROOT 非依存 4 本)+ `make test-root`(recorder)+ run_conformance.sh +
  Rust 側 env-gated 統合、すべて green。既存 Rust テスト(215)無影響。
- E2E で entries=108。inprogress → rename の動作確認。rollover 動作確認。
- ファイル所有権: third_party/get/(新規)、tools/root_sink/(Makefile / root_sink.cxx /
  test_recorder.cxx 新規 / 必要なら eb_core.hpp の absorb 差し替え)、tests/root_sink_intake.rs。
  **src/*.rs と Cargo.toml に触らない**(009 が v1.4 対応中)。

## 結果

**実行環境**: macOS 26.5.2(Darwin 25.5.0)/ arm64 / Apple clang(`c++ -std=c++17 -O2 -g -Wall -Wextra`、
**NDEBUG 未定義**)/ ROOT **6.36.10** @ `/opt/ROOT` / libzmq 4.3.5(pkg-config)/
rustc(cargo test プロファイル dev)。**2026-08-13**。

### 変更・新規ファイル

| ファイル | 区分 | 中身 |
|---|---|---|
| `third_party/get/{GDataFrame,GFrameHeader,GDataChannel,GDataSample}.{h,cpp}` + `LinkDefGET.h` | 新規(**無改変コピー**) | GET software 20190315_patched / CoBoFrameViewer `src/root/` から。md5 一致を README に記録 |
| `third_party/get/LICENSE` | 新規 | CeCILL 本文(コピー元 `CoBoFrameViewer/COPYING`) |
| `third_party/get/README.md` | 新規 | 出自・コピー日・無改変の宣言・md5 表 / **`reference/` は非コミット・`third_party/get/` はコミット対象**の区別を明記 |
| `tools/root_sink/root_recorder.hpp` | 新規 | `Recorder`(TTree 書き出し + ファイルライフサイクル)。**ROOT に依存する唯一のヘッダ** |
| `tools/root_sink/test_recorder.cxx` | 新規 | Recorder 単体テスト(`make test-root`)+ **inspect モード**(読み戻しユーティリティ) |
| `tools/root_sink/root_sink.cxx` | 改変 | Recorder スレッド + 有界 `Channel<RecordItem>`、`--output-root`/`--max-root-bytes`/`--no-root`、JSON カウンタ追加、遅延フラグメントの TTree 行き配線 |
| `tools/root_sink/Makefile` | 改変 | ROOT + `rootcling` 辞書、`make test-root` を分離(`make test` の 4 本は ROOT 非依存のまま) |
| `tools/root_sink/.gitignore` | 改変 | `test_eb_core` / `test_recorder` / 辞書生成物(`getdict.cxx` / `getdict_rdict.pcm` / `libGET.rootmap`) |
| `tests/root_sink_intake.rs` | 改変 | 既存 7 本を `--no-root` の高速経路に、統合テスト 2 本追加(finalize / 実 .graw E2E) |

`src/*.rs` / `Cargo.toml` / `examples/` には**一切触っていない**(並列作業中の領域)。
`reference/` は読んだだけ(コミット対象外のまま)。

### 実行コマンドと結果

| コマンド | 結果 |
|---|---|
| `make -C tools/root_sink clean && make -C tools/root_sink`(本体) | **警告ゼロ**(`-Wall -Wextra`、NDEBUG なし) |
| `make -C tools/root_sink test`(ROOT 非依存 4 本) | **68 / 71 / 175 / SKIP** すべて 0 failed |
| `make -C tools/root_sink test-root` | **test_recorder: 163 passed / 0 failed** |
| `tools/root_sink/run_conformance.sh` | exit 0(**68 / 71 / 175 / 49** すべて 0 failed) |
| `TPCDAQ_ROOT_SINK_BIN=$PWD/tools/root_sink/root_sink cargo test --test root_sink_intake` | **9 passed / 0 failed**(2.37 s。既存 7 + 新規 2。E2E は `TPCDAQ_REAL_GRAW` 未設定で skip) |
| 上記 + `TPCDAQ_REAL_GRAW=/Users/aogaki/TPC/CoBo_2025-09-01T08_51_06.203_0000.graw` | **9 passed / 0 failed**(3.69 s。E2E 込み) |
| `cargo test --no-fail-fast`(env 未設定) | **220 passed / 0 failed**(010 時点の 215 + 009 の 3 + 本ユニットの 2) |
| `cargo clippy --all-targets -- -D warnings` | 警告ゼロ |
| `rustfmt --edition 2021 --check tests/root_sink_intake.rs` | 差分なし |

### C++ 単体テスト

| テスト | 結果 | 備考 |
|---|---|---|
| `test_tpc_wire` | **68 passed / 0 failed** | 無改変 |
| `test_rs_core` | **71 passed / 0 failed** | 無改変 |
| `test_eb_core` | **175 passed / 0 failed** | 無改変 |
| `test_conformance` | **49 passed / 0 failed**(GOLDEN 指定時)/ 未指定は SKIP | 無改変 |
| `test_recorder`(新規、`make test-root`) | **163 passed / 0 failed** | ROOT リンク。`make test` からは分離 |

`test_recorder` の 5 ケース:
1. `test_writes_one_entry_per_fragment_and_reads_back` — 2 event × 2 fragment を書き、
   **同プロセスで TFile を開き直して読み戻し**。エントリ数 4 / ヘッダ全フィールド /
   チャンネル分解 / サンプル値 / **bucket 昇順**を機械照合。
   手計算: 投入 items を `(aget1,ch5,b3,100) (aget0,ch67,b1,200) (aget1,ch5,b1,101)
   (aget0,ch67,b0,201) (aget1,ch5,b2,102)` と **(aget,chan) 混在・bucket 逆順**で入れ、
   出口は 2 チャンネル `(0,67)={(0,201),(1,200)}` / `(1,5)={(1,101),(2,102),(3,100)}`。
2. `test_shutdown_without_eos_keeps_the_inprogress_name` — EOS 無しの停止で
   `run0012.root` は**作られず**、`run_inprogress_*.root` が残り、かつ**読める**。
3. `test_rollover_splits_the_run_into_numbered_parts` — `max_root_bytes=1` で 3 イベント →
   `run0012.root` / `run0012_0001.root` / `run0012_0002.root` の 3 本(各 1 エントリ)、
   **空の末尾ファイルなし**・inprogress 残なし。
4. `test_out_of_range_channel_is_counted_not_silently_dropped` — chan=68/127 の 2 件が
   `items_out_of_range=2`、生き残るのは (aget2,ch67) のみ。
5. `test_a_single_fragment_event_becomes_one_entry` — 遅延到着相当(単一フラグメントの
   BuiltEvent、event_idx 7→8→6)が**書いた順に 3 エントリ**になる(捨てない・並べ替えない)。

### red → green の確認(TDD)

テストを先に書いたが初回ビルドで通ってしまったため、**実装側を故意に壊して赤を確認**した
(いずれも確認後にバイト一致で復旧、`diff` で照合済み):

| 壊した箇所 | 結果 |
|---|---|
| チャンネル内 bucket 昇順ソートを除去 | **10 failed**(bucket / adc の並びが投入順に化ける) |
| finalize の `rename` を無効化 | **14 failed**(`run0007.root` 不在 / inprogress 残 / rollover の part 名 3 本すべて) |
| 範囲外 chan のカウンタ加算を除去 | **1 failed**(`out_of_range == 2` が 0) |

### E2E(実 .graw、P1 オラクル)

`graw_replay(全速)→ receiver(006)→ decoder(009)→ **root_sink プロセス**`。
入力 `/Users/aogaki/TPC/CoBo_2025-09-01T08_51_06.203_0000.graw`(30,108,684 B)。

**root_sink の終了時 JSON(実測)**:

```json
{"batches":4,"fragments":108,"items":15040512,"eos":1,"runs":1,
 "events_complete":108,"events_incomplete":0,"late_fragments":0,
 "unexpected_fragments":0,"duplicate_fragments":0,"pending_events":0,
 "heartbeats":1,"unknown":0,"stale_eos":0,"unexpected_sources":0,
 "run_number_mismatch":0,"entries_written":108,"items_out_of_range":0,
 "recorder_queue":0,
 "root_files":[{"path":".../run0007/run0007.root","entries":108,"bytes":46041087}],
 "fatal":""}
```

**書かれた .root の読み戻し**(`test_recorder <file>` = inspect モード):

```
entries=108 channels=29376 samples=15040512 event_idx=[0,107]
first: cobo=0 asad=0 event_idx=0 event_time=103955324 revision=5 read_offset=0
       status=0 window_out=4294967295 channels=272
```

- **entries = 108**(P1 オラクル)。1 エントリ = 1 CoBo フレーム(SPEC §6.4)。
- **samples = 15,040,512 = items オラクルと完全一致**(1 item = 1 GDataSample、**1 個も落ちていない**)。
- channels = 29,376 = 108 × **272**(= 68 ch × 4 AGET = フル読み出し)。
- 所要 **2.28 s**(replay + decode + ROOT 書き出し、30 MB)。出力 46,041,087 B(圧縮 505 = ZSTD-5)。
- **inprogress → rename**: `run_inprogress_<unixtime>.root` で開き、全 EOS で
  `run0007.root` へ rename。テストが「`run0007.root` が存在」+「`run_inprogress_*` が
  1 個も残っていない」+「JSON の `root_files[0].bytes` が実ファイルサイズと一致」を機械照合。

### rollover の実データ確認(ad-hoc、`--max-root-bytes 8000000`)

同じ E2E 経路で `--max-root-bytes` を 8 MB に絞ったラッパを噛ませた**手動プローブ**:

```
finalized .../run0007/run0007.root      (10 entries, 8,739,711 bytes)
finalized .../run0007/run0007_0001.root (10 entries, 8,738,141 bytes)
finalized .../run0007/run0007_0002.root (10 entries, 8,740,432 bytes)
残: run_inprogress_1786634961.root (885,038 bytes)
```

命名規則(`run{run:04}.root` → `_{part:04}`)・閾値超過での切り替わり・**途中終了で
inprogress のまま残る**ことを実データで確認。この run が 108 に達する前で終わっているのは
**プローブ側の都合**(E2E テストは「run 1 本 = ファイル 1 本」を前提に組んであるので、
最初の rollover で `run0007.root` が現れた時点で assert が走り、パイプラインを畳んでしまう)。
完走する rollover の網羅は `test_recorder` の 3 番(3 part・空ファイルなし・内容照合)が担当。
プローブ用ラッパと生成物は削除済み。

### スキップしたテストとその理由

- `tests/root_sink_intake.rs` の 9 本は **`TPCDAQ_ROOT_SINK_BIN` 未設定なら skip**(008 踏襲)。
  E2E はさらに **`TPCDAQ_REAL_GRAW` も必要**(実 .graw はリポに入れない — CLAUDE.md)。
- `test_conformance` は GOLDEN 未指定なら SKIP(`make test` を cargo に依存させない)。
- **C++ 版 tpcdaq の ROOT 出力との TTree 全値比較(§12-3)はやっていない** —— 発注書の
  明示的な指示どおり **012 の仕事**。本ユニットは「標準の ROOT リーダで読み戻せる」までを主張する。
- ヒストグラムの ROOT 書き出し(`run{run:04}_monitor.root`、SPEC §6.5)は本ユニットの範囲外。
- 2 CoBo(ELITPC)での実データ rollover / マージは実機・実データが無いので未検証
  (合成データでは `test_recorder` と統合テストが (cobo,asad) 昇順のエントリ順を照合済み)。

### 発注書からの逸脱・追加(レビュー対象)

1. **`Channel<BuiltEvent>` ではなく `Channel<RecordItem>`**。`RecordItem` = `BuiltEvent` +
   **in-band 制御マーカー**(`RunClose`)。run の finalize を別経路のフラグでやると、
   キューに残ったイベントを追い越して先に閉じてしまう。in-band マーカーは delila-rs の流儀で
   SPEC §6.1 が「そのまま流用」に挙げているもの。
2. **チャンネルの並びを `(aget, chan)` 昇順に固定した**(発注書は「(aget, chan) 毎の
   GDataChannel に展開」としか書いていない)。本家 graw2root は `chan` 外側 × `aget` 内側、
   C++ 版 tpcdaq RootWriter は **items の初出順**。実 GRAW の item 順(bucket 外側 → aget → chan)
   では初出順 = `(aget, chan)` 昇順なので、**C++ 版と同じ並びでありながら到着順に依存しない**
   方を選んだ(§12-3 の TTree 比較が順序で偽陰性にならないように — 010 の (cobo,asad) 昇順と同じ理屈)。
   **本家 graw2root とはチャンネルの並び順が違う**ので、012 の比較相手は C++ 版 tpcdaq にすること。
3. **カウンタを 1 個追加**: `items_out_of_range`。item の chan は 7 bit(0–127)だが AGET は
   68 ch しかないので 68 以上は GDataChannel に置き場が無い。落とすしかないが**黙って落とさない**
   (CLAUDE.md)。本家 graw2root / C++ 版 tpcdaq はどちらも try/catch と `continue` で黙って捨てていた。
4. **終了コード 5(`kExitRootIo`)を追加**。ROOT の `mkdir` / `TFile::Open` 失敗を
   「ログして続行」にすると、**書けていないのに走り続ける**保存系になる。`Recorder::fatal_reason()`
   を呼び手が見て、既存の `fatal()`(カウンタ JSON + `_Exit`)に載せた。
5. **finalize 先が既にある場合は rename しない**(inprogress のまま残してエラーを出す)。
   delila-rs は `_<unix_ns>` を足して逃げるが、完成済み run ファイルを `std::rename` で
   黙って上書きする危険を避けた。発注書に規定が無かったので最も保守的な側に倒した。
6. **`AutoSave("SaveSelf")` を 30 s 間隔で入れた**(発注書に記載なし)。SPEC §6.1 が Recorder
   ライフサイクルの「そのまま流用」に挙げている項目。これが無いと異常終了時の inprogress が
   **読めないファイル**として残り、「異常終了は inprogress のまま残す」(§6.5)の意図
   (そこまでのデータは残る)が満たせない。不要ならこの 6 行を削るだけで戻せる。
7. **既存の統合テスト 7 本に `--no-root` を付けた**(発注書 §2 の「既存テストの高速経路」を
   そのまま実施)。`spawn_sink()` が `--no-root` を足し、ROOT を使う 2 本だけ `spawn_sink_raw()`
   を直接呼ぶ。これが無いと cargo test の cwd(リポジトリ直下)に `run0007/` が生える。
8. **E2E を `tests/root_sink_intake.rs` に置いた**(ファイル所有権上ここしか触れないため)。
   receiver/decoder の in-process 配線は `tests/decoder_pipeline_real_graw.rs` からの写しなので、
   **`ReceiverParams` / `DecoderParams` の形が変わると両方が同時に壊れる**(申し送り)。
9. **ロールオーバ判定はイベント単位**(フレーム単位ではない)。1 イベントのフラグメントが
   2 ファイルに割れないようにした。C++ 版 tpcdaq は `write()` = フレーム毎に見ている。
10. **ファイルサイズの見方は `TFile::GetEND()`**(C++ 版 tpcdaq は `GetBytesWritten()`)。
    GetEND は実ファイル末尾オフセット = 実サイズで、バスケット未フラッシュでも 0 に張り付かない。
    小さい `--max-root-bytes` を与えるテストが決定的に書ける。
11. **`rootcling` の `-I` を絶対パスにした**。相対パスだと辞書に相対パスが焼き込まれ、
    cwd の違う実行(cargo test から起動される root_sink)で
    `Missing FileEntry for GDataFrame.h` が stderr に出る(実害は無いが黙っていない体裁を優先)。

### 判断が要る点(実装せずに残したもの)

- **`GFrameHeader` に置き場の無いフィールドが 2 つある**: `frame_type` と `run_number`。
  発注書は「ヘッダは OwnedFragment の全フィールド(§2.4)から」と書いているが、
  GET の `GFrameHeader` には `frameType` フィールドが無く、`fDataSource`(MFM の 1 バイト)は
  Fragment が運んでいない。**third_party/get は無改変が原則**なので:
  - `fDataSource = 0`(C++ 版 tpcdaq RootWriter と同じ)
  - `frame_type` は TTree に書かない(生 graw がバックストップ)
  としてある。TTree に載せる必要があるなら **SPEC 側の判断**(GDataFrame 派生を作るか、
  別ブランチを足すか、`fDataSource` に詰めるか)が要る。
- **run に 1 件もイベントが無かった場合、ROOT ファイルを作らない**(lazy open)。
  空の `run{run:04}.root` を置くべきかは発注書に無い。現状は stderr に
  `run N closed with no ROOT file (no events)` を出すだけ。

### 最終レビュー(2026-08-13 Fable)

- **判定: 受理(COMPLETED)**。逸脱 11 件すべて受理(RecordItem の in-band RunClose は
  追い越し防止として正しい判断。items_out_of_range + exit 5、AutoSave 30 s も受理)。
- 保留 2 点は裁定して SPEC §6.4/§6.5 に明文化した:
  ① frame_type / run_number は TTree に載せない(現実装のまま)。
  ② 0 イベント run は ROOT ファイルを作らない(現実装のまま)。
  チャンネル並び (aget,chan) 昇順と「012 の比較は C++ 版 tpcdaq を正としキー突き合わせ」も
  SPEC §6.4 に記載。
- レビュー側で独立再検証: make test(68/71/175)+ test-root(163)+ 統合 9/9(実 .graw E2E 込み、
  entries=108・samples=15,040,512 一致)+ cargo 220 passed / 0 failed。
- 申し送り(012 へ): E2E が tests/root_sink_intake.rs に同居しているため、
  ReceiverParams/DecoderParams 変更時は decoder_pipeline_real_graw.rs と同時に壊れる。
