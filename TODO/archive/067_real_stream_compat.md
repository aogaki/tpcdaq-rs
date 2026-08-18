# 067 — 実ストリーム新事実の取り込み(topology frame / frameType 1 実データ / 単一ファイル形式)

**Status: COMPLETED**(起票 Fable 2026-08-18。根拠 = reference/exp_data/2026 の pedestal/pulser
実データ調査 — 実測値は本チケット末尾と CURRENT.md 2026-08-18 節)

> **2026-08-18 実装セッション: A(Rust 側)・B・C は完了(結果節参照)。**
>
> **Fable 裁定(2026-08-18、同日クローズ)**: A の「vcobo への topology 送出追加」は
> **実装しない — 発注書から撤回する**。起票時の前提「receiver は実機で必ず受ける」が誤り
> (一次資料 MemRead.cpp:362 は `"FDT" == senderType` ガード付き・送出は daqStart 時。
> 我々の flowType は TCP)。vcobo は TCP のみ = 実機の TCP センダに忠実であり、
> topology を送らないのが正。選択肢 (a)(b)(c) はいずれも採らない。
> **flowType は TCP 維持を SPEC v1.23 §8.2 に明文化**し、FDT のワイヤ差分
> (IMALIVE/GOODBYE/run 毎再接続)は将来 FDT が必要になった場合の新ユニット 3 点セット
> として同節に契約記録した。TCP 経路での topology 有無の実機確定は 069 の Q1
> (実 2025 mini run の graw 先頭に frameType 7 が実在するため「TCP なら絶対来ない」とも
> 断定しない — decoder の防御はどちらでも無害に働く)。
> これにより本チケットの残件はゼロ。SPEC 反映(frameType 1 格上げ・§6.3 到着順注記・
> §12-1 オラクル追加)も v1.23 で完了。

## 背景(実データで確定した 3 つの新事実)

ユーザーが実験機から持ち帰った pedestal(32 run / 35 GB、内部 periodically トリガ)と
pulser(26 run + 中断 0 バイト 3 本 / 4.3 GB、AsAd 内蔵パルサー)を全数ヘッダ走査して確定:

1. **CoBo topology frame(frameType 7)は FDT 接続開設時に CoBo 自身が必ず送る**。
   一次資料 = `reference/20190315_patched/GetBench/src/get/daq/MemRead.cpp:362`
   (`sendTopology()`、呼び出し元 DaqCtrlNodeI.cpp:395、ChangeLog にも明記)。
   実バイト列(pedestal/pulser 両方の _0000 先頭で実測):
   `40 00 00 0c 00 00 07 00 | 00 0f 00 00`
   = metaType 0x40(big・blkSize1 blob)/ frameSize 12 B / dataSource 0 / frameType 7 /
   payload: coboIdx=0, **asadMask=0x0F**, 2pMode=0。
   **我々の receiver は実機でこれを必ず受けるが、一度もテストされていない**
   (vcobo は AsAd 分割 .graw をリプレイするため送っていない)。
   framer は仕様上 12 B で正しく切り出せる(src/framer.rs — bit7=endian/bits[3:0]=blkSize、
   bit6 は無視で無害)。**decoder の frameType 7 到着時の挙動が未検証**。
2. **pulser ランは frameType 1(rev 5、itemSize 4)** — SPEC の「frameType 1 =
   実データ照合なし・合成のみ」を**実データ照合済みへ格上げできる素材が初めて手に入った**。
   実測: 全フレーム **557,312 B 固定** = ヘッダ 256 + 139,264 items × 4 B
   (= 272 ch × 512 buckets — 部分 readout 設定でも item 数は全 ch 分。
   ただし hit pattern には FPN とみられる歯抜けあり — 照合時に意味を確定させること)。
3. **GetController 経由ランの graw は単一ファイル・AsAd インターリーブ**:
   命名 `CoBo_{TS}_{idx:04}.graw`(AsAd なし)、フレーム列は eventIdx 毎に
   asadIdx 0→1→2→3 の順で 1 本に混載、1 GiB(1,073,875,980 B)で `_0001` へ分割、
   **topology frame は各 run の _0000 先頭のみ**(分割後ファイルには無い)。
   physics ランの `CoBo{K}_AsAd{A}` 4 本組とは別形式(SPEC の書き出し命名規則は
   physics 側で不変 — 本チケットは**読み側**の対応のみ)。

## やること

- **A. topology frame の受信対応(ELI-NP 地雷の除去 — 最優先)**:
  decoder(または receiver 直後の層)で frameType 7 を認識し、
  **INFO ログ(coboIdx/asadMask/2pMode をデコードして出す)+ カウンタ + スキップ**。
  silent drop 禁止(コーディング標準)。上記 12 バイト実バイト列をフィクスチャ化し、
  「run 先頭に topology frame が来ても後続イベントが 1 件も欠けない」テストを固定。
  **vcobo にも FDT 接続開設時の topology frame 送出を追加**(板ごと偽る方針に従い
  実機挙動へ寄せる。MemRead.cpp が正)。
- **B. frameType 1 の実データ照合**: pulser 実データ数イベントを
  reference の CoBoFrameViewer(照合元)でダンプし、我々の decoder 出力と突き合わせ。
  hit pattern の歯抜け(FPN 除外?)と nItems=272×512 の関係もここで確定・記録。
  照合が通ったら **SPEC の frameType 1 記述の格上げは Fable が改訂**(実装ユニットは
  照合値の記録まで)。
- **C. 読み側ツールの耐性**: graw_replay(および関連リーダ)が
  ①単一ファイル AsAd インターリーブ形式を読める ②先頭 topology frame をスキップできる
  ③0 バイト .graw(中断 run、実在 3 本)で panic せずスキップ+可視化する。
  合成フィクスチャで単体テスト、実データはローカル任意回帰(環境変数パス)。

## 検収

- cargo / vcobo テスト green(A のフィクスチャ含む)+ B の照合値
  (イベント数・item 数・サンプル値の一致)を結果節に記録。
- 実データ実測の参照値: pulser `2026-08-17T08:09:11` = topology 1 + type1 304 frames
  (76 events × 4 AsAd、全 557,312 B)/ pedestal `2026-08-16T17:37:09` = topology 1 +
  type2 1868 frames(467 events × 4 AsAd、全 278,784 B、physics と同一 compact)。
  データフレームの dataSource は **1**(xcfg の 0 と不一致、2022/2026 全データで同値 —
  B で由来を一言記録できれば尚可、深追い不要)。

## モデル・規模

- 発注書とテストで縛れる(A/C = Sonnet 可、B は照合判断が絡むので Opus 推奨)。
- 非スコープ: 書き出し側の命名変更、SPEC 本文の改訂作業(Fable)、pulser 波形解析(066 系)。

---

## 結果(2026-08-18 実装セッション)

### 実行環境

macOS 26.5.2 (Darwin 25.5.0) / arm64 / rustc 1.97.1 (8bab26f4f 2026-07-14) /
Apple clang(C++17)。実データは `reference/exp_data/2026/`(ローカルのみ、リポ外)。

### 実行コマンドと結果

```sh
cargo fmt && cargo clippy --tests -- -D warnings   # クリーン(警告 0)
cargo clippy --all-targets                          # 警告 0
cargo test                                          # 468 passed / 0 failed / 1 ignored
cd tools/vcobo && make -j8 test_vcobo_core && ./test_vcobo_core
                                                    # 148 passed / 0 failed(無変更)
```

リポ全体ゲートは **454 → 468 passed**(+14、失敗 0)。ignored 1 は従来どおり。

実データ回帰(環境変数で任意実行、リポには合成のみ):

```sh
TPCDAQ_REAL_GRAW_PULSER=reference/exp_data/2026/pulser/CoBo_2026-08-17T08:09:11.852_0000.graw \
TPCDAQ_REAL_GRAW_PEDESTAL=reference/exp_data/2026/pedestal/CoBo_2026-08-16T17:37:09.555_0000.graw \
cargo test --release --test decoder_real_stream_compat -- --nocapture
# pulser   oracle: data_frames=304  items=42336256  topology=1 unsupported=1 malformed=0 reset_count=0 frame_sizes={557312}
# pedestal oracle: data_frames=1868 items=260145152 topology=1 unsupported=1 malformed=0 reset_count=0 frame_sizes={278784}
# 2 passed / 0 failed
```

### 新規テスト(14 本、すべて red → green を確認)

`src/framer.rs`
- `real_topology_frame_is_cut_at_12_bytes_without_eating_the_next_frame`

`src/decode.rs`
- `parse_topology_reads_the_real_12_byte_frame`
- `parse_topology_keeps_the_three_payload_fields_apart`
- `parse_topology_is_none_for_anything_that_is_not_frame_type_7`
- `decoder_counts_the_topology_frame_separately_from_other_unsupported_frames`
- `frame_type1_matches_the_mfm_oracle_for_the_real_pulser_encoding`(B の合成レプリカ)

`src/decoder.rs`
- `a_topology_frame_at_the_head_of_a_run_costs_no_events`(**A の核心**)
- `topology_frames_are_visible_in_metrics`
- `other_control_frames_do_not_count_as_topology`

`tests/graw_replay_tool.rs`
- `single_file_asad_interleaved_replays_byte_exactly_and_stays_decodable`
- `zero_byte_graw_is_skipped_with_a_visible_warning_and_exit_zero`
- `zero_byte_graw_among_merged_files_is_skipped_and_the_rest_still_replays`

`tests/decoder_real_stream_compat.rs`(新規ファイル、環境変数未設定なら skip)
- `real_pulser_frame_type1_matches_the_mfm_oracle`
- `real_pedestal_frame_type2_stream_is_read_without_loss`

red の確認: A/C の新規 API(`parse_topology` / `RunDecoder::topology_frames`)は
コンパイルエラーで red、0 バイト系 2 本は「stderr に警告が出ない」で red を実測。
実データ照合テストは **オラクル値を 1 だけずらすと red になること**を実測して
感度を確認済み(`(3,0,0,340)` → `(3,0,0,341)` で FAILED、その後 revert)。

### A — topology frame(frameType 7)

実装:
- `decode::parse_topology()` + `decode::Topology{cobo, asad_mask, two_p_mode}` +
  `TOPOLOGY_FRAME_TYPE`/`TOPOLOGY_FRAME_BYTES` を追加。
- `decode::Decoder` に `topology()` カウンタを追加。**`unsupported` の内数**にした
  (`unsupported` の意味を変えると既存の可視化・SPEC 参照・テストが動くため。KISS)。
- `RunDecoder` に `topology_frames` カウンタ + **INFO ログ**(`cobo` / `asad_mask`(hex)/
  `active_asads` / `two_p_mode` をデコードして出す)。ログは run あたり初回のみ
  (`RunDecoder` は Start 毎に新規生成 = 実質「run 先頭で 1 回」)。
- `metrics_json` に `"topology_frames"` を追加(controller/UI から観測可能)。

実測カウンタ:
- 合成: topology 1 + データ 4 フレームの入力で `frames_in=5 / Fragment 4 件
  (eventIdx 1,2,3,4 の順)/ topology_frames=1 / unsupported=1 / malformed=0 /
  Error ラッチ無し`。**後続イベントの欠落 0**。
- 実データ: pulser / pedestal どちらも `topology=1, unsupported=1`
  (= topology 以外の制御フレームは 1 本も無い)、`asad_mask=0x0F`(AsAd 4 枚)、
  `cobo=0`、`two_p_mode=false`。framer の `reset_count=0`(12 B を正しく切り出す)。

### B — frameType 1 の実データ照合(MFM オラクル)

**照合元 = GET 純正 MFM ライブラリの実リンク**(フォールバックは不要だった)。
CoBoFrameViewer 本体は Qt 依存でビルドせず、**同じ `mfm::` コード**
(`reference/_spike/prefix/lib/libMultiFrame.dylib`、既存ビルド成果物)へ直接リンクした
使い捨てダンパを scratchpad に作って照合した。経路は CoBoFrameViewer の
`CoBoEvent::decodeSamples()`(CoBoFrameViewer/src/get/CoBoEvent.cpp:155)と同一
(`mfm::FrameDictionary::addFormats` → `mfm::Frame::read` →
`itemAt(i).field("").bitField(...)`)。フォーマット定義 = `reference/config/CoboFormats.xcfg`
→ `CoboFormats-Rev-5.xcfg`(9 formats loaded)。

対象 = `pulser/CoBo_2026-08-17T08:09:11.852_0000.graw`。

**item のビットパック定義**(出典 `CoboFormats-Rev-5.xcfg` `<Item><Field><BitField>`、
4 B を big-endian u32 W として読む):

| field | xcfg offset/width | 抽出 | 我々の `decode_items` |
|---|---|---|---|
| agetIdx | 30 / 2 | `(W>>30)&0x3` | 一致 |
| chanIdx | 23 / 7 | `(W>>23)&0x7F` | 一致 |
| buckIdx | 14 / 9 | `(W>>14)&0x1FF` | 一致 |
| sample | 0 / 12 | `W&0xFFF` | 一致 |

bit 13..12 はどの BitField にも属さない(予約)。**我々の実装と完全一致**。

照合値(すべて我々の decoder 出力と **一致**):

| 項目 | MFM オラクル | 我々の decoder |
|---|---|---|
| データフレーム数 | 304(= 76 event × 4 AsAd) | 304 |
| フレーム長 | 全て 557,312 B | 全て 557,312 B(実測 1 種類のみ) |
| itemCount / frame | 139,264(= 272 ch × 512 bucket) | 139,264 |
| item 総数 | 42,336,256 | 42,336,256 |
| malformed | — | 0 |
| frame 0 ヘッダ | frameType 1 / rev 5 / cobo 0 / asad 0 / eventIdx 0 / eventTime 103,261,370 / readOffset 0 / status 0 / mult [1,2,4,8] / windowOut 0xFFFFFFFF / lastCell [690,690,690,690] | 全一致 |
| frame 0 先頭 10 item | (3,0,0,340)(0,0,0,372)(1,0,0,256)(2,0,0,335)(3,1,0,275)(0,1,0,373)(1,1,0,254)(2,1,0,329)(3,2,0,364)(0,2,0,363) | 全一致 |
| frame 0 末尾 5 item | (2,66,511,404)(3,67,511,391)(0,67,511,388)(1,67,511,345)(2,67,511,336) | 全一致 |
| frame 0 (aget0,ch0) bucket 0..7 | 372,381,380,382,384,383,382,385 | 全一致 |
| frame 0 AGET 毎 item 数 | 34,816 ずつ(= 68×512) | 全一致 |
| frame 0 chan / buck / sample 範囲 | [0,67] / [0,511] / [232,4095] | 全一致 |
| frame 1 先頭 10 item | (2,0,0,373)(3,0,0,299)(0,0,0,396)(1,0,0,354)(2,1,0,392)(3,1,0,299)(0,1,0,384)(1,1,0,377)(2,2,0,386)(3,2,0,369) | 全一致 |
| frame 1 (aget0,ch0) bucket 0..7 | 396,406,409,405,405,407,409,407 | 全一致 |

**hit pattern の歯抜けの意味が確定した**: `hitPat_0..3` の生バイトは全 AGET・全フレームで
`1f fe ff df ff ff bf f7 ff`。ch 0..67 のうち **立っていないのはちょうど {11, 22, 45, 56}
= FPN と完全一致**。一方 **item 側には FPN ch も 512 bucket 分そのまま入っている**
(nItems = 272×512 = 全 ch 分。テストで ch 毎 512 件を実測)。
→ **「hit pattern は FPN を除く。データは除かない」**。SPEC の「生 ADC(減算なし)が既定」
と矛盾しない。なお bit 68 も立っているが GET 自身が無視する(`MAX_CHANNELS = 68`、
CoBoFrameViewer/src/root/GFrameHeader.h:74 / graw2root/graw2dataframe.cpp:268-271)。

`dataSource` は **1**(xcfg の 0 と不一致、チケット記載どおり)。深追いせず。

### C — 読み側ツールの耐性

- **① 単一ファイル AsAd インターリーブ**: `graw_replay` は 1 ファイル指定なら元から
  バイトそのまま送出なので**実装変更なし**。合成フィクスチャ(先頭 topology +
  event 毎 AsAd 0→1→2→3、5 event)でバイト一致 + 受信側 framer/decoder で
  「topology 1 本 + AsAd 順が保たれる」ことをテストで固定した。
- **② 先頭 topology のスキップ**: 単一ファイル経路はバイト透過、マージ経路は
  `peek_event_idx` が `None` を返すフレームを即時送出する既存規則でそのまま通る。
  decoder 側が A で INFO + カウンタ + スキップするようになったので silent ではない。
- **③ 0 バイト .graw**: `graw_replay` に `is_empty_graw` / `warn_empty_graw` を追加。
  単一ファイル経路は**ループへ入る前に**畳む(**副産物のバグ修正**: 従来は
  `--loop` + 空ファイルで「read 0 → seek(0) → read 0」を全速で回す無限ループだった)。
  マージ経路は空ファイルをスキップして残りをそのまま流す。どちらも stderr に
  `file is empty (0 bytes) — skipped` を出して **exit 0**(スキップであって失敗ではない)。

### 発見した想定外の事実(チケットの前提との差分)

1. **pedestal 実データの AsAd 順は「常に 0→1→2→3」ではない**(チケット③の前提と差分)。
   `CoBo_2026-08-16T17:37:09.555_0000.graw` 全 1,868 frames を実測:
   - event 毎の AsAd 集合は **必ず {0,1,2,3} 揃い**(欠落・重複 0 件、467 event 全部)。
   - しかし到着順が回転している event が 2 件(#105 = `2,3,0,1` / #345 = `3,0,1,2`)。
   - eventIdx も単調増加ではない: **後退が 40 箇所、後退幅は必ず 1**(隣接 event が
     混ざるだけ)。1 event の 4 フレームが占める到着幅は 3(439 event)/ 4(2 件)/ 6(26 件)。
   - pulser(frameType 1)は逆に 76 event すべて厳密に `0,1,2,3`。
   → **イベントビルダは AsAd 順にも eventIdx の単調性にも依存してはならない**。
     eventIdx でグルーピングする現行 root-sink 実装はこの実データで安全(到着幅 6 は
     タイムアウトに対し十分小さい)。テストで実測値ごと固定した。
2. **frameType 1 の item は AGET ラウンドロビンだが開始位相がフレーム毎に違う**
   (frame 0 は aget=3 始まり、frame 1 は aget=2 始まり)。frameType 1 は 4 つ組が
   item に明示されているので影響しないが、**item index から (aget,chan,buck) を
   導く近道を入れてはいけない**ことの実証。テストに非対称フィクスチャとして固定した。
3. **topology frame は「FDT のときだけ」「daqStart で」送られる**(下記)。

### 未実施と理由 —— A の「vcobo への topology 送出追加」= 設計判断が必要

**実装せず差し戻す**(発注書に無い設計分岐に当たったため — 掟どおり)。一次資料が
チケットの前提と食い違う:

- `MemRead::sendTopology()`(MemRead.cpp:362)の本体は
  **`if ("FDT" == dataSender().senderType())`** で始まる。**TCP センダでは 1 バイトも送らない。**
  (`TcpDataSender.hpp:78` は `"TCP"`、`FdtDataSender.hpp:76` が `"FDT"`。)
- 呼び出し元は **`DaqCtrlNodeI::daqStart`(DaqCtrlNodeI.cpp:395)** —— チケットの
  「FDT 接続開設時」ではなく **daqStart 時**(FDT は `start()` の中で connect するので
  結果的に接続直後になる)。
- vcobo は `connect()` で **dataRouterType が `"TCP"` 完全一致でなければ拒否**する
  (vcobo_link.hpp:190)。既存コメント⑪「plain TCP に topology フレームは無い」は
  **一次資料どおりで正しい**(docs/VIRTUAL_ZCOBO_ja.md §4.3)。

つまり vcobo に topology を足すには次のどれかを **選ぶ** 必要があり、これは設計判断:
- (a) vcobo が `"FDT"` も受理し、FDT のときだけ daqStart で topology を送る(最も忠実。
  ただし ⑩⑩ の「TCP 完全一致」と ⑪ を書き換える + 下の FDT 差分も抱える)
- (b) TCP でも無条件に送る(**一次資料に反する** —— 実機 TCP は送らないので、
  vcobo が実機より寛容な方向に嘘をつくことになる)
- (c) vcobo に opt-in フラグを足す(実機に無い挙動を設定で作る)

**さらに重い前提の崩れ(Fable キュー行き)**: ELI-NP の運用スクリプトは全て
`type="FDT"` である(`ZC706_20181031_ELINP/scripts_2asads/README_SCRIPTS.txt:49,74,116` 他、
`configs_1asad_physics_20180605/*.ecc` も全て FDT)。**実データに topology frame が
入っているのはそのため**。ところが FDT は plain TCP と**ワイヤが違う**:

- `FdtDataSender::connect()` は接続直後に **4 バイトの IMALIVE `00 00 00 00`** を送る
  (FdtDataSender.hpp:125 → `sendHeartBeat()`)。
- run 中、**イベントが 3 秒来ないたびに同じ 4 バイト**を送る
  (`MemRead::sendHeartBeat` が InterruptMonitor の timeout action、
  DaqCtrlNodeI.cpp:339 + `heartBeatPeriod_ms = 3000`)。
- `daqStop` → `dataSender().stop()` → `disconnect()` で **4 バイトの GOODBYE
  `FF FF FF FF`** を送ってから **ソケットを閉じる**(= run 毎に接続が張り直される。
  TCP センダは閉じない)。
- これらは MFM フレームではないので、**我々の framer は壊れたヘッダとして
  バッファ全体を破棄する**(`00 00 00 00` → frameSize 0 / `FF FF FF FF` →
  blkSize 32768・巨大長)。実機 dataRouter はこの 4 バイトを剥がしてから .graw に
  書くので、**記録済み .graw には痕跡が残らない**(実測: pulser ファイル長は
  `12 + 304×557,312 = 169,422,860` にバイト単位で一致 = 混入ゼロ)。

→ **我々が FDT で実機に繋ぐなら receiver 側で IMALIVE/GOODBYE の除去と
run 毎 connect/disconnect への対応が要る。TCP で繋ぐなら topology frame は
そもそも来ない**(が、A の対応は保険として無害で有用)。どちらを取るかは SPEC 事項
なので Fable の裁定を仰ぐ。

### 非スコープ(手を付けていない)

- SPEC 本文の改訂(frameType 1 の「実データ照合済み」への格上げ、上記 FDT の扱い)= Fable。
- 書き出し側の命名規則(physics 側は不変)。
- vcobo の `load_graw_set` は 0 バイトファイルを**エラー**にする(panic ではなく明示的な
  起動失敗)。C の対象は graw_replay と読み側なので変更していない。
