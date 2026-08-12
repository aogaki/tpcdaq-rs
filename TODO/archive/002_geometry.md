# 002 — ジオメトリ抽象

**Status: COMPLETED**(2026-08-12。実装 = implementer/Sonnet、レビュー = Fable)
**仕様**: SPEC §4 全節、§12-12(受け入れ)
**依存**: 001(ワークスペース)

## やること

1. `.dat` パーサ: NEW 10 欄 / LEGACY 7 欄の両対応(トークン数で判別 — TPCReco 方式)。
   ヘッダ 7 キー全部(**DRIFT CAGE ACCEPTANCE 含む** — rust_reference の取りこぼしを直す)、
   AUX 5 欄、`#` コメント/空行、重複 `(cobo,asad,aget,ch)` は警告 + 先勝ち。
2. ルックアップ: キー `(cobo, asad, aget, raw_ch 0–67)` のフラット配列(累積 AsAd オフセット)。
   返り値は `ChannelRole { Strip{plane,section,strip} / Fpn{index} / Aux{name} / Unmapped }`。
   Unmapped の出現はカウント可視化のフックを持つ(silent 禁止)。
3. FPN リオーダ: 64 要素定数表(`REORDER_FROM_GEOMETRY_TO_GRAW`)をパース時に一度だけ適用。
   TPCReco `Aget_normal2raw` ループの移植版と**全 64 入力一致**のテスト。
4. TSV ダンプ(全チャンネル → role/plane/section/strip): §4.5 の Rust/C++ 一致テスト用。
5. 合成フィクスチャ(`tests/fixtures/`): mini 縮小版 / **架空 2-CoBo 版** / LEGACY 版。
   **実 .dat はリポに入れない。**
6. 実ジオメトリ回帰(env 変数パス、未設定なら skip — real graw 方式):
   mini = U72/V92/W92、ELITPC = ストリップ行 1018 + AUX 6、strip 最大 U132/V225/W226。

## 受け入れ(= P0 出口条件)

- ELITPC `.dat`(ローカル実ファイル)が読めて ch→(plane,section,strip) が引ける。
- FPN/Aux/Unmapped が役割として区別される。全テスト green。

## 結果

**実行環境**: macOS(Darwin 25.5.0, arm64)/ rustc 1.97.1 / cargo 1.97.1 / 2026-08-12。

### 実行コマンドとテスト結果

```
cargo test        # workspace 全体(003 並列実装分含む)
cargo clippy --tests -- -D warnings
cargo clippy --all-targets -- -D warnings
cargo fmt -- --check
```

- 全 green(`cargo fmt -- --check` 差分なし、clippy 警告 0、`cargo test` 全 98 件 passed / 0 failed)。
- 本ユニット(002)で追加したテストは 35 件、すべて green:
  - `src/geometry.rs` インライン単体テスト 8 件:
    `reorder_loop_matches_constant_table_for_all_64_inputs` /
    `plane_parse_accepts_only_uppercase_uvw` /
    `aux_agent_ch_uses_same_signal_reorder_as_strip_columns` /
    `duplicate_channel_key_warns_and_keeps_first` /
    `malformed_lines_are_recorded_not_fatal` /
    `aget_index_out_of_hardware_range_is_malformed_not_corrupting` /
    `unmapped_hit_count_increments_on_every_unmapped_lookup` /
    `dump_tsv_does_not_increment_unmapped_hit_count`
  - `tests/geometry_new_format.rs` 9 件(mini 縮小フィクスチャ: header/strip/aux/fpn/unmapped/
    max_strip/cobo_count/warnings なし/load 失敗)
  - `tests/geometry_legacy_format.rs` 4 件(LEGACY 7 欄フィクスチャ)
  - `tests/geometry_multi_cobo.rs` 5 件(架空 2-CoBo フィクスチャ、cobo 別解決・AsAd 枚数差・FPN)
  - `tests/geometry_tsv_dump.rs` 7 件(全走査・タブ区切り列数・昇順・代表行の厳密一致)
  - `tests/geometry_real_regression.rs` 2 件(env 変数ゲート、下記参照)

### 実ジオメトリ回帰の実測値(env セット実行)

```
TPCDAQ_REAL_GEOMETRY_MINI=.../TPCReco-HIGS2026_online/resources/geometry_mini_eTPC.dat \
TPCDAQ_REAL_GEOMETRY_ELITPC=.../TPCReco-HIGS2026_online/resources/geometry_ELITPC.dat \
cargo test --test geometry_real_regression -- --nocapture
```

→ `mini_real_geometry_matches_oracle_values` / `elitpc_real_geometry_matches_oracle_values` ともに ok。

- mini: `max_strip = [72, 92, 92]`、ストリップ行 256、AUX 0、duplicate/malformed 各 0 件
  — 発注書オラクル(U72/V92/W92・ストリップ行 256)と完全一致。
- ELITPC: `max_strip = [132, 225, 226]`、ストリップ行 1018、AUX 6、duplicate/malformed 各 0 件
  — 発注書オラクル(ストリップ行 1018 + AUX 6、strip 最大 U132/V225/W226)と完全一致。
- env 未設定時(通常の `cargo test`)は両テストとも早期 return で green のまま(skip 相当)。

### 逸脱・判断に迷った点

1. **AUX 行の `AGET_CH` が信号番号(0–63)か raw(0–67)か — 参照実装からは確定不能。**
   TPCReco `GeometryTPC::LoadAnalog`(`AUX 5 欄`の読み込み処理)は調査した全版(master 含む
   13 変種すべて)で「パースして表示するだけ」の未実装スタブであり、実際にチャンネル登録は
   一切していない。実 ELITPC `.dat` の AUX 値(58/60/62)もどちらの解釈でも 0–67 の範囲内に
   収まってしまい、実測でも切り分けできなかった。本実装は STRIP 行と同じ列名 `AGET_CH` を
   共有している以上、同じ「信号番号 0–63」規約とみなし `REORDER_FROM_GEOMETRY_TO_GRAW` を
   AUX にも適用する設計にした(`src/geometry.rs` の `parse_aux_line` にコメントで明記、
   `aux_agent_ch_uses_same_signal_reorder_as_strip_columns` でこの規約自体をテスト対象にした)。
   実 ELITPC ファイルでは AUX の signal_ch(58/60/62)がいずれも 11 未満の FPN 境界を跨がない
   ため、この解釈でも「raw そのまま」の解釈でも実際の raw_ch 値は変わらず(恒等写像域内)、
   今回の実測回帰(AUX 行数 6 の一致)には影響していない。将来 AUX の signal_ch が 11 以上に
   なる `.dat` が出てきた場合に解釈の当否が初めて可視化されるので、**要レビュー**として明記する。
2. **`parse`/`load` は(`Config` と同じく)自由関数、`lookup` 等は `Geometry` のメソッド。**
   発注書が `dump_tsv(&Geometry) -> String` を自由関数として明示していたのに合わせ、
   コンストラクタ相当(`parse`/`load`)も `src/config.rs` の既存パターン(`config::parse`/
   `config::load` が自由関数)に揃えた。一方 `lookup` はホットパスで頻用するため
   `Geometry::lookup(&self, ...)` のメソッドとした(発注書に明記はないが、
   `Config` に相当する前例がないための素直な選択)。
3. **`cobo/asad/aget/raw_ch` の型は `u32` に統一。** 発注書の疑似コードには型指定がないため、
   `src/config.rs` の `CoboConfig::id: u32` に合わせて統一した(mini/ELITPC 実測とも
   cobo≤1・asad≤3・aget≤3・raw_ch≤67 で実害はない)。
4. **重複 `(cobo,asad,aget,ch)` は FPN との衝突も同じ「重複」扱いにした。** 実データでは
   REORDER 表が 11/22/45/56 を絶対に生成しないので発生しないが、AUX の解釈次第では
   理論上あり得るため、STRIP/AUX 同士の重複と同じ「先勝ち + 警告」に倒した(FPN が先に
   置かれるので FPN が勝つ)。
5. **malformed 行の分類は 1 種類(`MalformedLine{line_number}`)にまとめた。** トークン数不一致・
   plane 不正・数値パース失敗・AGET_CH 範囲外・AGET 範囲外(0–3 超過、稠密配列破壊防止)を
   すべて同じ構造体で記録している。原因別の enum 化は発注書に指定がなく、可視化フックとしては
   `line_number` だけで十分と判断した(KISS)。
6. **ログ出力(`info!` 等)は実装していない。** `Cargo.toml` に `log`/`tracing` 系クレートが
   未登録(新規依存の追加は本ユニットの権限外)。CLAUDE.md の Clean Architecture 原則(domain
   核は IO 非依存)にも沿う形で、`duplicate_warnings()` / `malformed_lines()` / `unmapped_hit_count()`
   を構造化データとして公開し、実際のログ出力は呼び出し側(decoder 等の後続ユニット)に委ねる
   設計にした。ロギング基盤が導入され次第、それらのユニットがこのフックを叩けばよい。

いずれも実装を止めるほどの分岐ではないと判断し進めたが、1. は AUX を実際に使う後続ユニット
(monitor の波形ビュー等)が確定する前にレビューを推奨する。

### 公開 API 署名一覧(`src/geometry.rs`)

```rust
pub const AGET_CHIPS_PER_ASAD: u32 = 4;
pub const RAW_CH_PER_AGET: u32 = 68;
pub const SIGNAL_CH_PER_AGET: u32 = 64;
pub const REORDER_FROM_GEOMETRY_TO_GRAW: [u32; 64] = [..];

pub enum Plane { U, V, W }
impl Plane { pub fn as_str(self) -> &'static str; }

pub enum ChannelRole {
    Strip { plane: Plane, section: u8, strip: u16 },
    Fpn { index: u8 },
    Aux { name: String },
    Unmapped,
}

pub struct HeaderScalars {
    pub angles_deg: Option<[f64; 3]>,
    pub diamond_size_mm: Option<f64>,
    pub reference_point_mm: Option<(f64, f64)>,
    pub drift_velocity_cm_per_us: Option<f64>,
    pub sampling_rate_mhz: Option<f64>,
    pub trigger_delay_us: Option<f64>,
    pub drift_cage_acceptance_mm: Option<(f64, f64)>,
}

pub struct DuplicateChannel { pub cobo: u32, pub asad: u32, pub aget: u32, pub raw_ch: u32, pub line_number: usize }
pub struct MalformedLine { pub line_number: usize }

pub enum GeometryError { Io { path: PathBuf, source: std::io::Error } }

pub struct Geometry {
    pub header: HeaderScalars,
    pub max_strip: [u16; 3],
    // 以下は private フィールド
}
impl Geometry {
    pub fn lookup(&self, cobo: u32, asad: u32, aget: u32, raw_ch: u32) -> ChannelRole;
    pub fn unmapped_hit_count(&self) -> u64;
    pub fn duplicate_warnings(&self) -> &[DuplicateChannel];
    pub fn malformed_lines(&self) -> &[MalformedLine];
    pub fn cobo_count(&self) -> usize;
}

pub fn parse(text: &str) -> Geometry;
pub fn load(path: impl AsRef<Path>) -> Result<Geometry, GeometryError>;
pub fn dump_tsv(geometry: &Geometry) -> String;
```

### 変更ファイル

- `src/geometry.rs`(スタブ → 本体実装)
- `tests/fixtures/geometry_mini_reduced.dat`(新規、合成)
- `tests/fixtures/geometry_2cobo_fake.dat`(新規、合成)
- `tests/fixtures/geometry_legacy.dat`(新規、合成)
- `tests/geometry_new_format.rs`(新規)
- `tests/geometry_legacy_format.rs`(新規)
- `tests/geometry_multi_cobo.rs`(新規)
- `tests/geometry_tsv_dump.rs`(新規)
- `tests/geometry_real_regression.rs`(新規)
- `TODO/002_geometry.md`(本 `## 結果` 節を追記。`Status` は `OPEN` のまま)

### レビュー裁定(Fable、2026-08-12)

コードレビュー + ゲート独立再実行(fmt / clippy / 全 98 テスト green)+ 実ジオメトリ回帰の
独立再実行(mini・ELITPC ともオラクル一致を確認)。合成フィクスチャが実データの値を写して
いないことも確認。裁定:

1. **AUX の AGET_CH = 信号番号 0–63 で確定(承認)**。決め手: 実 ELITPC .dat 自身の列コメントが
   `AGET_ch[0-63]` と明記している(TPCReco 調査 2026-08-12 で引用確認済み)。実装の解釈は
   ファイルの自己文書と一致する。SPEC §4.1 にこの規約を明文化した。
2. parse/load 自由関数 + lookup メソッド — 承認(config.rs の前例と整合)。
3. u32 統一 / FPN 衝突も重複扱い / MalformedLine 一本化 — 承認(KISS)。
4. ログは構造化ゲッタ公開 — 承認。tracing 導入は P1 receiver ユニットで行い、呼び出し側が
   これらのフックを叩く。
5. 微細な指摘(非ブロッカー): `lookup` が `Aux{name: String}` を clone するのは表示レートの
   低頻度チャンネル(実 ELITPC で 6/1088)なので実害なし。将来ホットパスに乗せる場合は
   `&ChannelRole` 返しへの変更を検討。
