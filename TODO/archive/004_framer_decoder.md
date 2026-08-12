# 004 — MFM framer + CoBo デコーダ(純コア)

**Status: COMPLETED**(2026-08-12。実装 = implementer/Sonnet、レビュー = Fable)
**仕様**: SPEC §2.4(Fragment / item パック)、§12-1(オラクル)。フレームレイアウトの正 =
C++ 版 tpcdaq(要点は本票に転記済み。疑義があれば `~/test/get/tpcdaq/src/decode/` を参照)
**依存**: 003(`msg::Fragment` / `pack_item` / `items_to_bytes`)

## やること

1. `src/framer.rs` — MFM フレーマ(バイトストリーム → フレーム境界。IO なし):
   - 8 バイト一次ヘッダ: byte0 = metaType。**bit7 = エンディアン(set=little / clear=big)**、
     bits[3:0] = log2(blkSize)。bytes1–3 = 24bit frameSize_blk(エンディアンは bit7 に従う)。
     **総フレーム長 = frameSize_blk × blkSize**。
   - 不正ヘッダ(フレーム長 0 / < 8 / > max_frame_bytes 既定 64 MiB)→ **バッファ全破棄 +
     reset カウント**(再同期は試みない。panic 禁止)。
   - 実 2025 データは metaType 0x08(blkSize 256、big-endian)。
   - push(チャンク)→ next()(フレームスパン)の逐次 API。フレームはチャンク境界を跨げること。
2. `src/decode.rs` — CoBo フレームバイト列 → `msg::Fragment`:
   - 共通ヘッダ(オフセット表、多バイト読みは metaType bit7 のエンディアン):
     frameType@5(2B) / revision@7 / headerSize@8(2B, blk 単位) / itemSize@10(2B) /
     itemCount@12(4B) / eventTime@16(6B) / eventIdx@22(4B) / coboIdx@26 / asadIdx@27 /
     readOffset@28(2B) / status@30 / mult@67(2B×4) / windowOut@75(4B) / lastCell@79(2B×4)。
     ヘッダ最小 88 B。**itemStart = headerSize_blk × blkSize**(実データでは 256)。
   - 検証: itemSize は frameType1=4 / frameType2=2、itemStart ≥ 88、
     itemStart + itemCount×itemSize ≤ フレーム長。違反 = **malformed カウント**(silent 禁止)。
   - frameType ∉ {1,2} = **unsupported カウントで skip**(topology 等の制御フレーム。
     malformed とは区別する — 生 graw には残るデータなので正常系)。
   - **frameType 1(4B item)**: `aget=(w>>30)&3` / `chan=(w>>23)&0x7F` / `bucket=(w>>14)&0x1FF` /
     `adc=w&0xFFF`。
   - **frameType 2(2B item)**: `aget=(w>>14)&3` / `adc=w&0xFFF`。chan/bucket は AGET 毎カーソルで
     復元: `chanCur[4], buckCur[4]` を 0 起点、item 消費前に `chanCur[aget] >= 68` なら
     `{chanCur[aget]=0; buckCur[aget]+=1}`、割当後 `chanCur[aget]+=1`。
     **AGET 間インターリーブに耐えること**(C++ 版テストの再現: aget0 の途中に aget1 が
     挟まっても aget0 のカーソルが独立に継続する)。
   - 出力 = `msg::Fragment`(items は `pack_item` → `items_to_bytes`)。
     カウンタ: frames / items / malformed / unsupported(構造化ゲッタ、002 と同じ流儀)。
3. 合成フィクスチャはテストコード内で構築(コミットするバイナリなし):
   frameType 1/2 × big/little、インターリーブ、malformed 各種、blkSize 256 ヘッダ。
4. 実データオラクル `tests/decoder_real_graw.rs`(env `TPCDAQ_REAL_GRAW` 未設定なら skip):
   ファイルをチャンク読み → framer → decoder で **events=108 / items=15,040,512 / malformed=0**。
   ローカル実行して実測値を `## 結果` に記録すること
   (実ファイル: `/Users/aogaki/TPC/CoBo_2025-09-01T08_51_06.203_0000.graw`、29 MB)。

## 受け入れ

- 合成テスト green(両 frameType、両エンディアン、インターリーブ、不正系、チャンク跨ぎ)。
- 実 graw オラクル完全一致(events=108 / items=15,040,512 / malformed=0)。
- `src/lib.rs` への `pub mod framer; pub mod decode;` 追加は本ユニットのみが行う。

## 結果

**実行環境**: macOS(Darwin 25.5.0)、rustc/cargo 1.97.1、2026-08-12。

**実行コマンド**:
```
cargo build --lib
cargo clippy --all-targets -- -D warnings
cargo fmt -- --check   # + rustfmt src/decode.rs src/framer.rs(担当外ファイルは触らないため個別指定)
TPCDAQ_REAL_GRAW=/Users/aogaki/TPC/CoBo_2025-09-01T08_51_06.203_0000.graw cargo test
```

**テスト結果**:
- `cargo test --lib`: 88 passed / 0 failed(既存 66 + 本ユニット新規 22)。
  - `src/framer.rs` の `framer::tests`: 10 件(単一フレーム丁度・分割 push・連結 2 フレーム・
    フレーム+端数保持・big/little 両対応・壊れたヘッダ reset・上限超過 reset・reset 後の継続・
    フレーム長 8 未満で reset・blkSize=256 big-endian ヘッダ)。
  - `src/decode.rs` の `decode::tests`: 12 件(frameType1 blk256 実データ符号化・frameType2 blk256
    実データ符号化・frameType2 AGET 間インターリーブ耐性(big/little 両方)・frameType1 ヘッダ+item
    ロスレス往復(big/little 両方、FPN chan=11 保持)・unsupported frameType・非 CoBo 制御フレーム
    skip・itemSize 不整合 malformed・item 本体切り詰め malformed・itemStart<88 malformed・
    フレーム長 8 未満 malformed・共通ヘッダ長(88)未満 malformed・カウンタ累積)。
- `tests/decoder_real_graw.rs`(新規、`TPCDAQ_REAL_GRAW` 未設定なら skip green): 1 passed。
- 全体(`cargo test`): lib 88 + `decoder_real_graw` 1 + 既存統合テスト群、全 green
  (`graw_replay_tool.rs` を含む並列ユニット 005 の統合テストも green。同ツリーの並列実装分は
  未変更・未レビュー対象)。
- `cargo clippy --all-targets -- -D warnings`: 警告 0。`cargo fmt -- --check`: 差分 0。

**実 graw オラクル実測値**(`/Users/aogaki/TPC/CoBo_2025-09-01T08_51_06.203_0000.graw`、
29 MB、8192 バイトチャンクで push):
```
events=108 items=15040512 malformed=0 unsupported=1 reset_count=0
```
発注書のオラクル(events=108 / items=15,040,512 / malformed=0)と完全一致。`unsupported=1` は
ファイル末尾の非 CoBo 制御フレーム(topology 等、frameType ∉ {1,2})1 個で、`malformed` とは
区別される正常系(発注書「malformed とは区別する — 生 graw には残るデータなので正常系」)。
`reset_count=0` はヘッダ破損による再同期が一度も起きていないことを示す。

**公開 API**:
- `src/framer.rs`: `pub struct Framer`、`Framer::new()` / `with_max_frame_bytes(usize)` /
  `push(&mut self, data: &[u8])` / `next(&mut self) -> Option<&[u8]>` / `reset(&mut self)` /
  `buffered(&self) -> usize` / `reset_count(&self) -> u64`。定数
  `PRIMARY_HEADER_SIZE: usize = 8`、`DEFAULT_MAX_FRAME_BYTES: usize = 64 * 1024 * 1024`。
- `src/decode.rs`: `pub struct Decoder`、`Decoder::new()` /
  `decode(&mut self, frame: &[u8]) -> Option<msg::Fragment>` / `frames(&self) -> u64` /
  `items(&self) -> u64` / `malformed(&self) -> u64` / `unsupported(&self) -> u64`。定数
  `pub const HEADER_MIN_BYTES: usize = 88`。

**発注書からの逸脱・迷った点**:
- **`next()` の返り値型**: 発注書は「push(チャンク)→ next()(フレームスパン)の逐次 API」と
  C++ 版の `FrameSpan`(ポインタ+長さの非所有ビュー)を指しているが、Rust では
  `next(&mut self) -> Option<&[u8]>` とした(専用の `FrameSpan` 構造体は起こさず KISS で
  スライスそのものを返す)。借用は `self` に紐づくため、次の `push`/`next`/`reset` を呼ぶまで
  しか有効でない制約は borrow checker がコンパイル時に強制する(C++ 版はコメントで注意喚起
  するだけ)。API の形の判断だが、発注書の記述(「フレームスパン」を返す逐次 API)をそのまま
  Rust 標準の借用として翻訳したものであり、それ以外の自然な選択肢がないと判断しコード化した。
  懸念があれば差し戻しを。
- `next` というメソッド名は `std::iter::Iterator::next` と紛らわしいと clippy
  (`should_implement_trait`)が警告するが、返り値が `self` を借用する streaming/lending 形で
  `Iterator` trait 自体を実装できない(Rust の既知の制約)ため、発注書どおりの名前を維持し
  `#[allow(clippy::should_implement_trait)]` で理由コメント付きで抑止した。
- `decode()` は C++ 版(out-param + `is_cobo` フラグ、例外を投げない)と異なり
  `Option<msg::Fragment>` を返す設計にした(Rust の値返却スタイルへの素直な翻訳。malformed /
  unsupported はいずれも `None` + カウンタ計上で、silent failure を作らない原則は維持)。
- `tests/graw_replay_tool.rs`(並列ユニット 005 所有)は今回のセッション中に 1 回だけ
  `connect_failure_exits_non_zero_with_clear_message` がフレーク失敗したが、004 側の変更とは
  無関係(該当ファイルは触っていない)。再実行では green。担当外のため修正はしていない。
- それ以外、発注書の記述どおりに実装。設計判断が必要な分岐には遭遇しなかった。

### レビュー裁定(Fable、2026-08-12)

コードレビュー(framer.rs 全体 + decode.rs ヘッダ/検証部)+ ゲート・実 graw オラクルの独立再実行
(events=108 / items=15,040,512 / malformed=0 / unsupported=1 / reset=0 を再確認)。裁定:

1. `next() -> Option<&[u8]>` の lending 形 — 承認。C++ 版がコメントで注意喚起していたスパン寿命を
   borrow checker がコンパイル時強制する分、上位互換。
2. `decode() -> Option<Fragment>` の値返却 — 承認(カウンタ計上で silent 化なし)。
3. `#[allow(clippy::should_implement_trait)]` + 理由コメント — 承認(streaming lending は
   Iterator 実装不能、既知の言語制約)。
4. checked_mul/checked_add によるオーバーフローガードは C++ 版に無かった堅牢化。良い。
