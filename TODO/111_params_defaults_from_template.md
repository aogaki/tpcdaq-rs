# 111 — Parameters UI の初期値をテンプレート xcfg から読む(mini で 25 MHz に化ける穴を塞ぐ)

**Status: IN PROGRESS**(2026-10-09 起票 Opus。2026-10-10 ユーザー「パラメーター画面をお願いします」→ Fable が発注書を精度上げ(§5)し、レーン A(Opus)/ B(Sonnet)を並列発注)

## 0. 問題(1 分で)

- 078 の Parameters UI(A/B/C 完了、SPEC v1.34 §11)は、「Load defaults from template」で**テンプレート名だけ**入れ、ノブの初期値は
  `ui/src/app/params/params-view.ts` の `DEFAULT_KNOBS`(= HIgS の configure-physics: **25 MHz / triggerDelay 1748** / 120 fC / 232 ns …)を直書きで使う
  (078 §11「テンプレートは読まない」)。
- mini の現用テンプレート `configure-elinp.xcfg`(= VM の `delay_test.xcfg`、**100 MHz / triggerDelay 320** — 101 追記 2、107)に対して
  そのまま保存すると、render がノブ値でテンプレートを上書きする → **100 MHz → 25 MHz、320 → 1748 に化けた xcfg ができる**。
- 現状の回避: 運用マニュアル(Claude Doc)で「Parameters 画面は使わない」と明記。設定の切替は daq.toml の `config_id` を手で書き換えて再起動。

## 1. やること

1. **抽出(render の逆)**: `src/params/` に「テンプレート xcfg → `Params`(knobs + exceptions)」を足す。各ノブの xcfg 上の位置は既存の
   `knob_path` / `KnobSpec` を使う。CompoundConfig の継承(`id="*"` が既定、個別 id が上書き、**粗い例外は一様値より優先されない** — SPEC v1.32 §3.3)
   に従い、一様値 + 例外に分ける。テンプレートに無いノブは「未設定」を明示(黙って既定値を入れない — CLAUDE.md)。
2. **REST**: `GET /api/params/defaults?template=<name>`(`ecc_config_dir` 配下のファイルだけ、パス外は 400)→ 抽出結果の JSON。
3. **UI**: 「Load defaults from template」で 2. を呼び、フォームを埋める。`DEFAULT_KNOBS` の直書きはやめる(テンプレート無しの空フォームは「未設定」表示)。
4. **(任意)ノブ追加**: `triggerPeriod`(periodic トリガの周期、ms。107 / 109 の periodic 試験で使用)。
5. **実機確認(078 §5-3 を兼ねる)**: ELITPC で elinp テンプレートから抽出 → 無変更で保存 → 生成された `configure-<id>.xcfg` が
   `configure-elinp.xcfg` と**バイト一致** → その id で短い run(periodic か線源)を 1 本。zCoBo は ELITPC に直結済みなので遠隔で可。

6. **運用マニュアル(Claude Doc、日英)の更新**: ユーザー方針(2026-10-09)「マニュアルはパラメーターができてから配布」。
   完了時に「Parameters 画面は使わない」(取得設定・やってはいけないこと の 2 か所、日英)を使い方の説明に置き換え、PDF を作り直す。

## 2. テスト(TDD、オラクル)

- 実テンプレート 3 種で**往復バイト一致**: 抽出 → render → 元の xcfg と同一。
  対象 = mini `delay_test.xcfg`(md5 0845d6c4、ELITPC `~/ELITPC/vm_configs_2026-10-08/`)、HIgS `configure-physics-zCobo1k-extTrig_120fC_25MHz_232ns_trigDelay1748.xcfg`
  (reference/config、md5 1d7b8854)、VM のパルサー `configure-pulser.xcfg`(md5 86a8ab10)。実ファイルはリポに入れない → 環境変数パスの任意回帰 +
  合成フィクスチャ(`tests/fixtures/params/`)で例外ありのケース。
- 078 の CompoundConfig オラクル(`tools/ccfg_resolve`)で、抽出した値が ECC の解決結果と全インスタンス一致。

## 2.1 手元の実テンプレート(2026-10-09 配置、gitignore)
`reference/_vm_configs_2026-10-08/` = VM の `TPCConfigFiles/` 17 本 + `delay_test.xcfg`(md5 0845d6c4)+ VM / GDED のパルサー
(`share_configure-pulser.xcfg` = 86a8ab10、`gded_configure-pulser.xcfg` = e1043496)。HIgS の physics は `reference/config/`。
ELITPC 上の同じもの = `~/ELITPC/vm_configs_2026-10-08/` と `~/ELITPC/GetSoftware_config/configure-elinp.xcfg`。

## 3. 見積もり(2026-10-09 にユーザーへ提示)

| 作業 | 担当 | 目安 |
|---|---|---|
| 1. 抽出 + 2. の往復テスト | Opus | 1〜1.5 日 |
| 2. REST + 3. UI | Sonnet | 0.5〜1 日 |
| 4. triggerPeriod ノブ(任意) | Sonnet | 0.5 日 |
| 5. 実機確認 | Opus(主対話) | 1 時間 |
| SPEC §3.3 / §11 反映 + 完了時レビュー | Fable | Fable キューでまとめて |

応急処置(DEFAULT_KNOBS を mini の値に差し替える、15 分)は ELITPC(HIgS)側で逆の事故を起こすので採らない。

## 4. 参考: UI に出しているのは「必要そうなものだけ」

- ノブは 11 個(triggerDelay / writingClockFrequency_MHz / triggerMode / globalThreshold / peakingTime_ns / readout / lsbThreshold / gain /
  triggerInhibition / zeroSuppression / pulser)。xcfg の残り(delay_test で値を持つ要素 ≈ 284、インスタンス展開で 4,423)は**テンプレートから無変更で引き継ぐ**。
- 特定の AsAd / AGET / ch だけ変える値は「Exceptions (N)」(折りたたみ、件数バッジ + ログブックで常に見える)— 078 §0。
- 構成・識別子系(AsAd `isActive`、readoutDepth 等)はノブにせず、テンプレートの選択が運ぶ(078 §2)。

## 5. 発注書(Fable、2026-10-10)— 2 レーン並列、契約は §5.0

### 5.0 契約(両レーン共通。ここが正 — 実装中に変えない)

`GET /api/params/templates`(閲覧系、認証なし)→ 200:
```json
{"templates": ["configure-elinp.xcfg", "configure-pulser-vm.xcfg", "..."]}
```
`ecc_config_dir` 直下の `configure-*.xcfg` の**ファイル名**を辞書順(prepare-* / describe-* / 生成物 `*__<hash8>.xcfg` は除く)。
503 = `ecc_config_dir` 未設定(既存 `ECC_CONFIG_DIR_NOT_CONFIGURED` と同文言)。

`GET /api/params/defaults?template=<name>`(閲覧系、認証なし)→ 200:
```json
{
  "template": "configure-elinp.xcfg",
  "knobs": { "triggerDelay": 320, "writingClockFrequency_MHz": 100.0, "triggerMode": "external",
             "globalThreshold": 1, "lsbThreshold": 15, "gain": "120fC", "triggerInhibition": "none",
             "peakingTime_ns": 232, "readout": "partial",
             "zeroSuppression": {"enabled": false, "threshold": 600},
             "pulser": {"enabled": false, "voltage_mV": -1000.0, "period_ms": 1000} },
  "exceptions": [ {"at": {"asad": 1, "aget": 2, "ch": 5}, "knob": "lsbThreshold", "value": 3} ],
  "unset": [],
  "unsupported": []
}
```
- `knobs` = **テンプレートの `*` 連鎖から読めて、かつ `KnobKind` の型・範囲に収まった葉だけ**(JSON の形は `tpcdaq-params/1` の `knobs` と同じ入れ子)。
- `unset` = `*` 連鎖に要素が無い葉の名前(`.` 区切り、例 `"pulser.period_ms"`)。**黙って既定値を入れない**(CLAUDE.md)。
- `unsupported` = 要素はあるが型・範囲外、または合成ノブ(`readout`)の構成要素が同じモードに写らないもの:
  `[{"knob": "triggerMode", "found": "onMultiplicity", "why": "not in {external, …}"}]`。`found` は xcfg のテキストそのまま(複数要素なら `isAllChannelRead=true,isFPNRead=false` の形)。
- `exceptions` = 明示 id ノードにあるノブ値。`at` は**明示 id の接頭辞だけ**(`AsAd[1].Aget[*].channel[*].Gain` → `{"asad":1}`、
  `AsAd[1].Aget[2].channel[5].LSBThresholdValue` → `{asad:1,aget:2,ch:5}`)。`*` の下に明示 id が来る形(`AsAd[*].Aget[1]…`)は CompoundConfig の
  探索規則上の意味が render と一致しないので **`unsupported`** に入れる(`why: "explicit id below a wildcard"`)。`note` は付けない。
- エラー: 400 = `template` 欠落 / 空 / パス区切り(`/`, `\`)や `..` を含む / `.xcfg` で終わらない / `ecc_config_dir` の外に解決される / xcfg として読めない(`ParamsError` の文言)。
  404 = ファイルが無い。503 = `ecc_config_dir` 未設定。本文は既存と同じ `{"error": "..."}`。

### 5.1 レーン A(Rust: 抽出 + REST)— implementer、**model: opus**

1. `src/params/extract.rs`(新規)+ `mod.rs` の公開。`pub fn extract(template_name: &str, template_xml: &str) -> Result<Extracted, ParamsError>`、
   `Extracted { template: String, knobs: BTreeMap<String, Value>(入れ子に組み立てた JSON `Value` でもよい), exceptions: Vec<Exception>, unset: Vec<String>, unsupported: Vec<Unsupported> }`
   + `impl Extracted { pub fn to_json(&self) -> Value }`(§5.0 の形)。**純関数**(ファイル IO なし)。`render.rs` の私有パーサ(`parse` / `Node` / `Element` / `find_cobo_node` /
   `TemplateIds`)を `pub(super)` に開いて再利用する — パーサを二重に持たない。値のテキスト → JSON は `KnobKind` ごとに(`Int`/`IntEnum` → 整数、`Real`/`RealEnum` → f64、
   `StrEnum` → 文字列、`Bool` → bool、`Derived` → 全エントリのテキストが同じモードに写るときだけそのモード)。既存 `value_to_xcfg_text` の逆であること(往復テストで担保)。
2. REST(`src/controller.rs`): §5.0 の 2 エンドポイント。既存 `get_params_render` と同じ `spawn_blocking` + `Reply` の流儀。パスの検査は **canonicalize してから `starts_with(ecc_config_dir)`**。
3. テスト(TDD — red を先に):
   - `tests/params_extract.rs`(新規): (a) `template_mini.xcfg` → `knobs` が `params_roundtrip.json` の `knobs` と**完全一致**、`exceptions`/`unset`/`unsupported` 空。
     (b) `template_stale_overrides.xcfg` → 明示ノードの値が `exceptions` に出る(件数と `at` を assert)。(c) **冪等性**: `extract` → `Params` に組んで `render` → もう一度 `extract` した結果が
     (a)(b) それぞれで最初の抽出と一致。(d) `Derived` 不整合(`isAllChannelRead=true` + `isFPNRead=false` の合成フィクスチャ、文字列置換で作る)→ `unsupported` に `readout`。
     (e) `Generator.period_ms` を削ったフィクスチャ → `unset` に `pulser.period_ms`、他は正常。(f) `triggerMode=onMultiplicity` → `unsupported`。
   - **実テンプレート往復**(任意回帰、リポには入れない): 環境変数 `TPCDAQ_PARAMS_REAL_TEMPLATES`(`:` 区切りの xcfg パス)があれば各ファイルで `extract` → `render` が
     **バイト一致**、無ければ `SKIP …` を出して緑(`tests/params_ccfg_oracle.rs` と同じ流儀)。手元の実物 = `reference/_vm_configs_2026-10-08/delay_test.xcfg`、
     `reference/_vm_configs_2026-10-08/share_configure-pulser.xcfg`、`reference/config/configure-physics-zCobo1k-extTrig_120fC_25MHz_232ns_trigDelay1748.xcfg`
     (**発注の受け入れ時にこの 3 本で実行して結果節に貼る**)。
   - `tests/controller_integration.rs` に REST 2 本: templates の一覧(prepare-*/生成物を除く)、defaults の 200(フィクスチャ template_mini を `ecc_config_dir` に置く)/ 400(`../x.xcfg`、`a/b.xcfg`、`.xcfg` なし)/ 404 / 503。既存 `params_harness` を使う。
   - `tests/params_ccfg_oracle.rs`: bin があるとき、`extract` した `knobs`+`exceptions` の `effective_value` が ccfg_resolve の解決値と一致する検査を 1 本足す(既存のヘルパで足りる)。
4. `docs/PARAMS_KNOBS_ja.md` に §8「抽出(render の逆)」を 10 行で(規則 = §5.0 の箇条書き)。
5. ゲート: `cargo fmt && cargo clippy --tests -- -D warnings && cargo test`(全部 green、結果の件数を結果節に)。`.unwrap()` 禁止(テストは `#![allow(clippy::unwrap_used)]`)。
   **やらないこと**: ノブ表 `KNOBS` の変更、`triggerPeriod` の追加、UI。設計分岐に当たったら実装せず報告。

### 5.2 レーン B(UI: Parameters ビュー)— implementer、**model: sonnet**

1. `ui/src/app/api/params-api.ts`: `getTemplates(): Promise<string[]>`、`getDefaults(template: string): Promise<ParamsDefaults>`(§5.0 の型を `ParamsDefaults` として定義。
   `unsupported` の要素型 `{knob, found, why}`)。エラーは既存 `ParamsApiError` の流儀(サーバの `error` 文言を加工しない)。
2. `ui/src/app/params/params-view.ts` / `.html`:
   - **`DEFAULT_KNOBS` を削除**。`params-view.spec.ts` の `DEFAULT_KNOBS ↔ params_roundtrip.json` の describe は、「`getDefaults` の応答(フィクスチャ JSON の knobs をモックで返す)でフォームが埋まる」テストに置き換える。
   - テンプレートの選択: `getTemplates()` の一覧を `<select>` に(読めなければテキスト入力にフォールバックし、理由を表示)。「No parameters yet」の画面と、読み込み済み画面の両方に
     **Load from template** ボタンを置く(読み込み済み側は「下書きを捨てて読み直す」ので、下書きに変更があれば確認文言を出してから)。
   - 読み込んだら `knobs` でフォームを埋め、`exceptions` を Exceptions 表に、`unset` の葉は**空欄 + 赤い「unset in template」バッジ**、`unsupported` は**フォームの上に警告一覧**
     (`knob: found — why`)として出す。`unset` または `unsupported` が残っている間は Save を無効にし、`saveDisabledReason` に理由を出す(silent 禁止)。
   - `unsupported` のノブは該当フィールドを空欄にする(テンプレートの値をそのまま入れない — 入れると render で 400 になる)。
3. テスト(`params-view.spec.ts` / `params-api.spec.ts`): templates 一覧の表示、defaults でフォームが埋まる、`unset` で Save が無効 + バッジ、`unsupported` で警告 + Save 無効、
   読み込み済み画面からの読み直し。既存テスト(現在 UI 全体で 272 以上)を落とさない。
4. ゲート: `cd ui && npm test -- --watch=false`(件数を結果節に)+ `npm run build`。`ui/dist` はコミットしない。**やらないこと**: ノブの追加、Rust 側、SPEC。

### 5.3 共通
- worktree 隔離、着手前に `git merge --ff-only main`。発注書の絶対パス = `/Users/aogaki/Workspace/tpcdaq-rs/TODO/111_params_defaults_from_template.md`(§5.0 + 自レーン節)。
- コミットは各レーンのブランチに。完了報告 = 変更ファイル一覧 + テスト実行コマンドと件数 + 設計上迷った点。Fable が完了時に一括レビューしてマージ。
- §5.1 の実テンプレート 3 本・ELITPC 実機確認(§1-5)・SPEC §3.3/§11 の反映・マニュアル更新(§1-6)は Fable(主対話)。`triggerPeriod` は見送り(ユーザーの当面の目的 = 25 MHz への切替には不要)。

## 5.4 追加発注(Fable、2026-10-10 — レーン A の報告を受けた裁定)— implementer、**model: opus**

**事実**: mini の現用 `delay_test.xcfg`(= `configure-elinp.xcfg`)は `isAllChannelRead=false` + `isFPNRead=true`。合成ノブ `readout`(full = true/true、
partial = false/false)では表せず `unsupported` → mini でパラメータを保存できない。HIgS / VM パルサーは true/true なので表せていた。

**裁定: `readout` を分ける。** `readout` = `isAllChannelRead` だけの合成ノブ(full → true、partial → false)、新ノブ **`fpnRead`** = `Global.Reg1.isFPNRead`(AGET スコープ、bool)。
物理の意味(全 ch 読むか / FPN 4 ch を常に読むか)がそれぞれ 1 ノブに対応し、4 通りすべてが表せる。3 つ目のモード名(`partial+fpn` 等)は意味が伝わらないので採らない。
スキーマ id `tpcdaq-params/1` は据え置き(本番の current.json はまだ無い。フィクスチャだけが影響)。

やること(main `c57cfb6` 以降から):
1. `src/params/schema.rs`: `KNOBS` の `readout` エントリを `isAllChannelRead` の 1 本に、`fpnRead`(`Scope::Aget`、`["Global","Reg1","isFPNRead"]`、`KnobKind::Bool`)を追加。
   `knob_scope_depth` 等はそのまま動くはず。ノブは 12 個(葉 15)になる。
2. フィクスチャ `tests/fixtures/params/params_roundtrip.json` に `"fpnRead": true`。`tests/params_render.rs` の `KNOBS_BODY` と件数 assert、`tests/params_extract.rs`、
   `tests/params_ccfg_oracle.rs`、`tests/controller_integration.rs` の該当を更新。(d) の「合成ノブの構成要素が不整合」テストは、合成ノブが 1 要素になるので
   削除せず**「`fpnRead` が bool でないテキスト → unsupported」**に書き換える。
3. UI: `ui/src/app/params/knob-spec.ts` に `fpnRead`(bool、scopeDepth 2、label "Read FPN channels"、xcfgElements `["isFPNRead"]`)を `readout` の直後に追加。
   `readout` の `xcfgElements` から `isFPNRead` を外す。`knob-spec.spec.ts`(フィクスチャとの突き合わせ)と `params-view.spec.ts` が通るように(件数の assert があれば更新)。
   `cd ui && npm test -- --watch=false` + `npm run build`。
4. `docs/PARAMS_KNOBS_ja.md` §7 の表と §8 を更新(`readout` の行を 2 行に)。
5. **受け入れ**: `TPCDAQ_PARAMS_REAL_TEMPLATES=<3 本>` で `cargo test --test params_extract` が **3 本ともバイト一致**(delay_test が通ること)。
   オラクル `TPCDAQ_CCFG_RESOLVE_BIN=/Users/aogaki/Workspace/tpcdaq-rs/tools/ccfg_resolve/build/ccfg_resolve TPCDAQ_XCFG_TEMPLATE=<physics> cargo test --test params_ccfg_oracle` が緑。
   ゲート `cargo fmt && cargo clippy --tests -- -D warnings && cargo test`。

レーン A 報告の「迷った点」への裁定: 1(CoBo 明示インスタンスの値 → unsupported)採用。2(`why` は schema の文言)採用。3・4 採用。
5(粗い `at` の例外は render で明示 id に展開され、テンプレート上では効いていなかった値が効くようになる)— 実テンプレートに無い形なので今回は §5.0 のまま、
`docs/PARAMS_KNOBS_ja.md` §8 に 1 行注記。6(例外の列挙順)— 今回の経路では起きないので据え置き、§8 に注記。

## 結果(2026-10-10、Fable 一括レビュー + 受け入れ再実行。実機確認は下に追記)

- マージ: レーン A `4d4dd53` → `18e505b`、レーン B `adac11c` → `c57cfb6`、§5.4(readout 分割 + fpnRead)`218f5e1` → **`1165cf8`**。
- **実テンプレート往復(Fable が main `1165cf8` で再実行)**: `TPCDAQ_PARAMS_REAL_TEMPLATES=<delay_test : share_configure-pulser : HIgS physics> cargo test --test params_extract`
  → 14 passed、**3 本とも byte-identical**、unset / unsupported / exceptions すべて空。抽出値: mini = 320 / 100.0 MHz / external / **readout partial + fpnRead true** / 232 ns / 120 fC、
  VM パルサー = 750 / onSlowControl / 1014 ns / pulser enabled、HIgS = 1748 / 25.0 MHz / readout full。
- **CompoundConfig オラクル**(`tools/ccfg_resolve` を reference/20190315_patched/CompoundConfig でビルド): 4 passed(mini フィクスチャ 238 インスタンス × JSON 値・抽出値、実テンプレート 4,423 インスタンス)。
- ゲート(main `1165cf8`、Mac): `cargo fmt --check` / `cargo clippy --tests -- -D warnings` 警告 0 / **cargo test 867 passed / 0 failed / 1 ignored**(着手前 848。+13 extract +4 REST +2 render 他)。
  UI: **298 passed / 5 skipped / 0 failed**(着手前 283)、`npm run build` 成功。
- レーン B のレビュー(Fable): 契約どおり。空欄は NaN を作らず「未設定」、enum は空欄のときだけ `—`、下書きに変更があれば画面内 2 ボタンで確認、unset / unsupported / 手で空にした欄は
  バッジ + Save 無効 + 理由表示。`DEFAULT_KNOBS` は消えた(= 25 MHz 直書きの穴は塞がった)。
- 裁定の記録: §5.4(readout / fpnRead)、レーン A の迷った点 1〜6 は §5.4 末尾のとおり。`schema.rs` 冒頭の「ノブ集合(7 個)」の古い doc コメントは次の機械作業で直す(Sonnet)。
- 未了: §1-5 実機確認(ELITPC、VPN 待ち)、§1-6 マニュアル更新、SPEC §3.3 / §11 への転記(ノブ 12 個・`fpnRead`・`/api/params/{templates,defaults}`)。

### 実機確認(§1-5、ELITPC、2026-10-10 12:55、Fable)
- main `5f7417f` を bundle で ELITPC へ、`cargo build --release`(16 s)、UI dist を `/opt/tpcdaq/ui` に入れ替え(旧は `ui.old`)。α run 33(88,711 s)を停止 → daq_down / up。
- `GET /api/params/templates` → 20 本(`configure-*.xcfg`。生成物 `*__<hash8>` と prepare-* は出ない)。
- `GET /api/params/defaults?template=configure-elinp.xcfg` → knobs = 320 / 100.0 MHz / external / **readout partial + fpnRead true** / 232 ns / 120 fC / thr 1 / LSB 15 / ZS off 600 / pulser off、unset・unsupported・exceptions 空。
- 無変更で `PUT /api/params` → id **`elinp__4bd7bfd1`**、`configure-elinp__4bd7bfd1.xcfg` と `prepare-…` が生成され、**どちらも `configure-elinp.xcfg` とバイト一致**(md5 0845d6c4 = VM の delay_test.xcfg)。
- `run/start` → run 34: ECC ログ `Prepare: using configuration 'elinp__4bd7bfd1'` / `Reading configuration file …configure-elinp__4bd7bfd1.xcfg` / `Configure: loading … 'elinp__4bd7bfd1'`、
  logbook `run_start.config_id = elinp__4bd7bfd1` + `params {id, sha256, exceptions 0}`。40 s で 202 frames(≈ 5 Hz)、overflow 0。**run 34 は α 本番として続行**(中身は elinp と同一)。
- これで UI の Parameters 画面は mini で使える(次のステップ = 25 MHz への切替は UI から: writingClockFrequency 25、triggerDelay を窓に合わせて決め直す — 101)。
