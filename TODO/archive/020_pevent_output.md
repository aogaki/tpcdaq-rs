# 020 — run.root を PEventTPC(TPCReco 互換)へ変更

**Status: COMPLETED**
**仕様**: SPEC **v1.8** §6.3(1 エントリ = 1 イベント、遅延フラグメント改訂)/ §6.4(全面改訂 —
本ユニットの核)/ §12-3(受け入れ改訂)
**背景(ユーザー裁定)**: GDataFrame 出力は瑕疵。オフライン解析は TPCReco であり、Warsaw は
`grawToEventTPC` で PEventTPC に変換して解析している。我々の run.root を**変換不要でそのまま
解析に使える形式**へ合わせる。
**意味論の正** = `reference/TPCReco/TPCReco-HIGS2026_online/`(実運用スナップショット):
- `EventSources/src/ConvertGrawFile.cpp`(TTree 形: TPCData / Branch("Event",…,128000,2) /
  eventId 重複排除 / 100 イベント毎 FlushBaskets / kOverwrite)
- `EventSources/src/EventSourceGRAW.cpp::fillEventFromFrame`(充填: normal ch のみ・
  `Aget_normal2raw` リオーダ・signal 窓・strip 射影・ペデスタル減算)
- `GrawToROOT/src/PedestalCalculatorGRAW.cpp` + `DataFormats/src/PedestalCalculator.cpp`
  (FPN 4ch 平均 + チャンネルオフセット。TProfile 256 ビン整数中心 = 純算術で等価)
- `EventSources/src/EventSourceROOT.cpp`(読み手の期待: ツリー `TPCData`・ブランチ `Event`)
- runId = `RunIdParser` 由来 `%Y%m%d%H%M%S`(long)
- 実機オラクルファイル: `reference/exp_data/2026/PEventTPC_2026-08-11T07-47-37.051_0000.root`
  (streamer: PEventTPC v1 / eventraw::EventInfo v1 / tuple<int,int,int,int> v1、ZLIB-1、
  ROOT 6.08/06 書き)

## やること

1. **TPCReco クラスのビルド時参照(コミットしない — ライセンス調査 2026-08-13 の帰結)**:
   TPCReco は**ライセンス無指定(all rights reserved)**のためコピーを公開リポに入れられない。
   017 の .ice と同じ流儀で、Makefile が `TPCDAQ_TPCRECO_DIR`(既定
   `../../reference/TPCReco/TPCReco-HIGS2026_online`)から include・コンパイルする。
   未設定・不在なら PEventTPC 出力を **明示 skip でビルド不可**(黙って劣化させない)。
   - 最小集合(調査確定、依存はここで閉じる — GeometryTPC は StripTPC.h の前方宣言のみ):
     `DataFormats/include/TPCReco/PEventTPC.h` / `EventInfo.h` / `StripTPC.h`、
     `Utilities/include/TPCReco/CoBoClock.h`(boost ヘッダのみ)、
     `DataFormats/src/PEventTPC.cpp`(AddValByStrip/Clear — リンク必須)。
     `EventInfo.cpp` は reset()/operator<< を呼ばなければリンク不要(実装時判定。必要なら
     RunIdParser 連鎖に注意)。
   - **我々のファイルは LinkDef のみ**(pragma: `PEventTPC+` / `eventraw::EventInfo+` /
     `eventraw::EventInfo::global_properties+` + `nestedclasses`)→ rootcling 辞書生成。
   - **streamer checksum を受け入れテストで固定**(実測 2026-08-13: PEventTPC v1
     **0xf71c32cf** / eventraw::EventInfo v1 **0xfea093e4** / global_properties v1
     **0x49e6428c**。実機オラクルファイルの streamer と照合。コピー元は HIGS2026_online 固定 —
     myChargeArray が出入りした他スナップショットと混ぜると割れる)。
   - Warsaw の再配布許諾が得られたら third_party/tpcreco へ昇格(NOTICE 付き)。
2. **tools/root_sink/pevent_fill.hpp(新規、純ヘッダ)**: GDataFrame(内部表現、011 で
   実証済み)→ PEventTPC 充填。fillEventFromFrame と同一意味論:
   normal ch 0..63 → `Aget_normal2raw`(geo.hpp の FPN リオーダ)→ strip lookup(geo.hpp)→
   signal 窓 [5,506] → (減算 ON なら)補正 = チャンネルオフセット + FPN_ave_signal[cell] →
   `AddValByStrip(tuple, val)`(**+= 加算**)で chargeMap へ。
   キー = `{dir(U=0/V=1/W=2), section(0="-"/1=A/2=B), number(1 始まり), cell}`。
   EventInfo: eventId(uint32_t)/ timestamp(eventTime、10 ns 単位)/
   runId(long `%Y%m%d%H%M%S`)/ pedestalSubtracted。global_properties は 0 のまま
   (実運用変換と同一 — 埋めるのは解析側)。
3. **ペデスタル移植(同ヘッダ内)**: PedestalCalculatorGRAW の算法を純算術で
   (per (cobo,asad) フレーム毎リセット、FPN 4ch の cell 毎平均 ×2 窓、normal ch の
   pedestal 窓 `raw − FPN平均` の per-ch 平均)。ジオメトリは geo.hpp(GeometryTPC 非依存)。
4. **root_recorder.hpp**: 既定を PEventTPC 出力へ(TPCData / Branch("Event",…,128000,2)、
   1 エントリ = 1 ビルド済みイベント、eventId 一回きり、遅延フラグメントは書かずカウント —
   SPEC §6.3 v1.8)。**`myChargeArray*` ブランチは既定で無効**
   (`SetBranchStatus("myChargeArray*", false)` — 実運用既定 disabledBranches と同一。
   4.7 MB/イベントの生配列を書かない)。100 イベント毎 FlushBaskets・
   `Write("", kOverwrite)` も実運用と同形。**`--format gdataframe` でテスト専用の旧出力**
   (既存テスト・§12-3 旧オラクル回帰の維持)。inprogress→rename・rollover・圧縮設定は
   現行のまま。
5. **config/CLI**: `pedestal_remove`(既定 true)/ `min/max_pedestal_cell`(5/25)/
   `min/max_signal_cell`(5/506)。
6. **比較ツール**: compare_pevent(TPCData 同士の全 key 値一致、eventId 突き合わせ)。
   §12-3 v1.8 の実データ照合(env-gated、同 run ペア入手まで skip)に使う。

## テスト

- 単体(test_pevent): 合成 GDataFrame → chargeMap 期待値(手計算オラクル: strip 射影・
  signal 窓・FPN リオーダ・ペデスタル数値 — 「FPN 4ch 平均」「オフセット平均」を含む最小例)。
  減算 OFF/ON 両方。EventInfo 各フィールド。
- 構造一致(env `TPCDAQ_REAL_PEVENT` = 実機オラクルファイル): ツリー名・ブランチ名・
  クラス streamer バージョン・圧縮設定の一致 + 実ファイルの EventInfo から
  pedestalSubtracted 実運用値を読んで既定と一致することを確認。
- E2E: 既存 root_sink 統合(2 ソースビルド・EOS・inprogress→rename)が PEventTPC 既定でも
  green。`--format gdataframe` で既存 §12-3 mini オラクル回帰が不変。
- 実データ値一致(env-gated、**同 run ペア入手後**): 我々のフルチェーン出力 vs
  grawToEventTPC 出力の全イベント全 key 一致。

## 受け入れ

- make test / make test-root / cargo test 全 green(Rust 側無改変)。
- 構造一致テスト green(実機オラクルファイル)。
- SPEC v1.8 §6.4/§6.3/§12-3 との一致。ライセンス隔離(third_party/tpcreco)完備。
- **未解決で残るもの**: 同 run ペアの実データ値一致(ユーザーが次回 LAN 接続で
  「08-11 run の graw 4 本」or「08-12 run の PEventTPC」を取得後にクローズ)。


## 結果

実行環境: macOS(Darwin 25.5.0)、ROOT 6.36(/opt/ROOT)、boost(Homebrew)、2026-08-13。
実装: implementer/Opus(委譲)、レビュー + Rust 側テスト追随 + 検証: Fable。

### テスト結果(最終、レビュー後の再実測)

- `make`(clean から): 成功・警告ゼロ。TPCReco 参照ビルド(`TPCDAQ_TPCRECO_DIR` 既定 =
  reference/TPCReco/TPCReco-HIGS2026_online、コピーなし — git status で混入なし確認)
- `make test`: 68 + 71 + 175 + 426 passed / 0 failed(conformance は GOLDEN 経由で 49 passed)
- `make test-root`: test_recorder **169 passed**(GDataFrame 回帰維持)/ test_pevent **98 passed**
- `TPCDAQ_REAL_PEVENT=<実機ファイル>`: test_pevent **119 passed**(実機照合 +21 —
  streamer checksum 三者一致(自辞書・自出力・実機): PEventTPC v1 0xf71c32cf /
  eventraw::EventInfo v1 0xfea093e4 / global_properties v1 0x49e6428c。実機の
  pedestalSubtracted=true(既定 ON の裏取り)・runId=20260811074737・myChargeArray
  ブランチ空・圧縮 level 1)
- `cargo fmt / clippy --tests -D warnings / cargo test`: 27 バイナリ全 ok・警告ゼロ
- env-gated(レビューで修正後): `TPCDAQ_ROOT_SINK_BIN` intake **9/9**、
  `TPCDAQ_REAL_GRAW+TPCDAQ_REAL_ROOT` p2_e2e **2/2**(mini GDataFrame オラクル全値一致の
  回帰を `--format gdataframe` 明示で維持)
- 負のオラクル: ペデスタル式から FPN 減算を外すと 10 CHECK が落ちる(式の噛み合い確認)

### レビューでの判定・修正(Fable)

1. implementer の警告「ELITPC は 256 strip 上限に当たる」は**誤り** — 実 ELITPC .dat の
   strip 番号は (dir, section) 毎採番で最大 226 ≤ 256(awk 実測)。これが本家の固定長配列が
   実運用で壊れない理由。範囲外 drop+count ガード自体は防御として妥当なので維持。
2. Rust env-gated テスト 4 箇所に `--format gdataframe` を明示(root_sink_intake 2 +
   p2_e2e 2)— 旧オラクル回帰の存置(SPEC §6.4 v1.8 の設計どおり)。
3. runId のローカル時刻導出(ワイヤに run 開始 TS が無い)を SPEC §6.4 実装注記 + 016 の
   設計入力として明文化(graw TS と数秒ずれ得る、対応の正はログブック、P4 で一元化)。
4. EventInfo.cpp リンク要否: **必須**(PEventTPC.cpp の operator<< 連鎖)。参照リンク集合は
   PEventTPC.cpp + EventInfo.cpp + RunIdParser.cpp + CoBoClock.cpp の 4 本(発注書想定 +2、
   依存はここで閉じる)。

### 未解決(次アクション)

- **同 run ペアの実データ値一致**(§12-3 v1.8 ③): ユーザーが次回 LAN 接続で
  「08-11 run の graw 4 本」または「08-12 run の PEventTPC」を取得後、compare_pevent で
  全イベント全 key 照合してクローズ。
- TPCReco 再配布許諾(third_party/tpcreco 昇格)は Warsaw へ確認待ち。
