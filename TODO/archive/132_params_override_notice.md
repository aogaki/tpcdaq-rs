# 132 — params の current.json が TOML の config_id を上書きしたことを run/start の応答と log に出す

**Status: COMPLETED(2026-10-13、結果は末尾)** — 旧: READY(2026-10-13 起票 Opus。078 の「運用上の罠」(2026-09-23)の小チケット化。Sonnet で縛れる)

## 0. 事実(078「運用上の罠」)
- `params_dir/current.json` があると、run/start は prepare / configure の config_id を params の `<id>` に差し替える(SPEC §3.3、`src/controller/rest.rs` の `run_start_params_snapshot` 以降)。仕様どおり。
- 2026-09-23 の GDED で、前の試験の current.json(pulser、gain 240 fC)が残っていた。「pedestal」「physics」のつもりの run 1 / 2 が pulser の設定で走った。
  `run_start.params` と audit の `params_id`、status の `params` には出ていたが、スクリプト運用では見落とした。
- いまの run/start の 200 応答は `{"run": N, "phase": "Running"}` だけ(`src/controller/rest.rs` の `ok_json`)で、上書きされたことは応答からは分からない。
  `start_run_inner`(仮称、同ファイルの `Ok((run, run_start_ts))` を返す関数)が `params_id` を持っているので、戻り値に足して応答まで運ぶ。

## 1. 仕様
1. run/start の 200 応答に `params_id`(`string | null`)を足す。audit と同じ値。
2. 上書きが起きたとき(`params_id` が非 null)は、応答の `notes`(`string[]`、無ければ空配列)に 1 行入れる:
   `prepare/configure config_id taken from params <id> (template <template>), not from daq.toml (<toml prepare>/<toml configure>)`。
3. **テンプレートの stem と TOML の configure config_id が違う**ときだけ、controller の log に `warn!` を 1 行出す。
   - stem はテンプレート名から `configure-` と `.xcfg` を外したもの(例: `configure-pulser.xcfg` → `pulser`)。
   - 例: テンプレートが pulser で TOML が physics。
   - 同じなら info のまま。
   - 応答の notes は 2. のとおり、上書きのたびに出す。
4. 失敗時(4xx / 5xx)の応答は変えない。

## 2. テスト(TDD、`tests/controller_integration.rs` の Harness の流儀)
- `run_start_reports_params_override_in_response`: current.json あり → 200 の `params_id` が PUT で返った id と一致し、`notes` に `taken from params` を含む 1 行がある。
- `run_start_without_params_has_null_params_id_and_no_notes`: current.json なし → `params_id: null`、`notes: []`。
- stem の比較は純関数に切り出し、単体テストを置く(`configure-pulser.xcfg` と `pulser` → 一致、`physics` → 不一致、`configure-pulser__ab12cd34.xcfg` のような生成物名が来ても stem を正しく取る)。
- UI が run/start の応答を型付きで読んでいるなら(`ui/src/app/` を grep)、足したフィールドで既存の UI テストが落ちないこと。UI の表示変更はしない。

## 3. SPEC
- §8.1 の run/start 応答に `params_id` と `notes` を 1 行。文案は主対話で書く(Fable キューで確定)。

## 4. ゲート
`cargo fmt && cargo clippy --tests -- -D warnings && cargo test`。UI を触った場合は `npm test -- --watch=false`。

## 報告に必ず書くこと
変更ファイル、テスト名、赤の実測、総数。**実行できなかった操作**(権限拒否・失敗したコマンド)。無ければ「なし」と書く。

## 結果(2026-10-13、Sonnet implementer → 主対話で照合・マージ `b825938`)

### 実装(コミット `e1f9f69`、3 ファイル +120 / −6)
- `src/controller/rest.rs`:
  - run/start の 200 応答に `params_id`(string | null)と `notes`(string[])を足した。`start_run_blocking` の戻り値は型エイリアス `StartedRun = (run, ts, params_id, notes)`。
  - 上書き時の notes の文言は §1 のとおり。
  - 純関数 `template_matches_config_id(template, toml_id)` を足した。テンプレートの stem が TOML の configure と違うときだけ `warn!`、同じなら `info!`。
  - 4xx / 5xx の応答は不変。
- `src/params/render.rs`: 既存の `template_stem` を `pub` にして流用。生成物名の `__<hash8>` は最初の `__` で切る。
- UI: `ui/src/app/api/run-control-api.ts` は run/start の応答を型付けしていないので、変更していない。

### 応答の例(統合テストの検証内容から)
- params あり: `{"run":1,"phase":"Running","params_id":"mini-fixture__ab12cd34","notes":["prepare/configure config_id taken from params mini-fixture__ab12cd34 (template configure-mini-fixture.xcfg), not from daq.toml (<prepare>/<configure>)"]}`
- params なし: `{"run":1,"phase":"Running","params_id":null,"notes":[]}`

### テスト
- 新規:
  - `tests/controller_integration.rs::run_start_reports_params_override_in_response`
  - `tests/controller_integration.rs::run_start_without_params_has_null_params_id_and_no_notes`
  - `src/controller/rest.rs` `tests::template_stem_matches_toml_configure_id_ignoring_prefix_and_generated_suffix`
    - `configure-pulser.xcfg` と `pulser` → 一致、`physics` → 不一致
    - `configure-pulser__ab12cd34.xcfg` と `pulser` → 一致、`physics` → 不一致
- 赤: 統合 2 本は `notes` が応答に無い(`left: Null, right: Array []`)で FAILED。単体は関数が未定義でコンパイルエラー。
- **main のゲート**(132 と 110 レーン B をマージした後、Mac、2026-10-13):
  - `cargo fmt -- --check` OK
  - clippy 警告 0
  - `cargo test` **1078 passed / 0 failed / 1 ignored**
  - UI **393 passed / 10 skipped**(28 files)
- スキップしたテスト: なし(ignored 1 は既存のベンチ)。
- 照合(主対話): `git show --stat` で 3 ファイル、上記のテスト名が diff にある。「実行できなかった操作」= なし。

### SPEC
§8.1 の run/start 応答に `params_id` と `notes` → CURRENT の Fable キュー(d)。

**Status: COMPLETED(2026-10-13)**
