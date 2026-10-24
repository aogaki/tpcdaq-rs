# 179 — params: 周期トリガの周期 `triggerPeriod_ms` をノブに

**Status: COMPLETED**(2026-10-24 ELITPC 反映。起票 Opus、実装 Sonnet implementer。ユーザー「現在ペデスタルを取ってますが、遅いです。早くしたいですが UI にパラメーターがありません。追加してもらえますか」)

## 0. 問題
- run 196(12.5 MHz のペデスタル、tpcsim への受け渡し = 177)は template `configure-pedestals.xcfg` の `Module.triggerPeriod = 4000 ms` のまま = 0.25 Hz(6.5 分で 100 事象)。
  周期はノブに無いので UI から変えられず、テンプレートを選び直すしかない(`configure-elinp-per{1,2,3,5,10,50,200,1000}.xcfg` = 150 の最大レート試験で作った、周期の行だけ違う写し)。
- GET の意味論(一次資料 `reference/20190315_patched`):
  - `GetBench/src/get/rc/CoBoNode.cpp:3192` `setTriggerPeriod` と `GetController/src/get/TestController.cpp:3340` `getTriggerPeriod_10ns`:
    `Module.triggerPeriod_10ns` があればそれ、無ければ `100000 * Module.triggerPeriod.getIntValue()`(ms → 10 ns)をレジスタ `triggerPeriod_10ns` へ。
  - `getIntValue()` の型 `Int` = `int`(`CompoundConfig/src/CCfg/TypeInfo.h:47`)→ **21474 ms を超えると `100000 * ms` が int で溢れる**。
  - 実際の間隔は「周期 + 1 事象の読み出し」(SPEC §13 v1.46: 全 ch 読み出しで + 2.931 ms)。
  - xcfg の要素は `<triggerPeriod unit="ms">N</triggerPeriod>`(range 属性なし、`Module` の `triggerMode` の 2 行後)。実テンプレートの値: 1, 2, 3, 5, 10, 50, 100, 200, 1000, 4000。`triggerPeriod_10ns` を持つテンプレートは無い。

## 1. 変更(決定済み)
### A. スキーマ `src/params/schema.rs`
- `KNOBS` の `triggerMode` の直後に:
  ```rust
  KnobSpec {
      name: "triggerPeriod_ms",
      scope: Scope::Cobo,
      suffix: &["Module", "triggerPeriod"],
      // xcfg に range 属性なし(unit="ms")。GET は 100000 * ms を int で計算する
      // (CoBoNode::setTriggerPeriod、Int = int)ので 21474 ms が上限。0 は FW の挙動が未確認。
      kind: KnobKind::Int { min: Some(1), max: Some(21474) },
  },
  ```
- モジュール先頭の doc コメントのノブ集合・数(**13 ノブ・葉 16**)を更新。`SCHEMA_ID` は `tpcdaq-params/1` のまま(111 の fpnRead と同じ扱い)。
### B. レンダラ `src/params/render.rs`
- テンプレートの CoBo の `Instance` のどれかに `Module.triggerPeriod_10ns` があれば、render を `ParamsError::TemplateShape`(既存の型)で失敗させる。
  文言: `Module.triggerPeriod_10ns があると GET は triggerPeriod を無視する(CoBoNode::setTriggerPeriod)— テンプレートから外してください`。
  (黙って効かないノブを作らない = CLAUDE.md の silent failure 禁止。現行テンプレートには無いので実害なし。)
### C. 抽出 `src/params/extract.rs`
- 新ノブは既存の機構でそのまま読めるはず(Int、CoBo スコープ)。要素が無いテンプレートでは `unset` に `triggerPeriod_ms`。特別扱いは不要なら足さない。
### D. 合成フィクスチャ `tests/fixtures/params/`
- `template_mini.xcfg` と `template_stale_overrides.xcfg` の `Instance[*]` の `Module` に、`triggerMode` の後ろへ実テンプレートと同じ形で
  `<triggerPeriod unit="ms">1000</triggerPeriod>` を足す(往復のバイト一致を保つ)。
- `params_roundtrip.json` に `"triggerPeriod_ms": 1000`。config id のハッシュが変わるので、それを固定しているテストの期待値を更新。
### E. UI `ui/src/app/params/`
- `knob-spec.ts` の `KNOB_GROUPS` の `triggerMode` の直後に:
  `{ name: 'triggerPeriod_ms', label: 'Trigger period (periodically only)', fields: [{ key: 'triggerPeriod_ms', label: 'Trigger period (periodically only)', kind: 'int', scopeDepth: 0, unit: 'ms', min: 1, max: 21474, xcfgElements: ['triggerPeriod'] }] }`
- ノブ数のコメント(knob-spec.ts の「12 ノブ」、`params-view.html` の「the 12 knobs」)を 13 / 葉 16 に。
- `knob-spec.spec.ts`・`params-view.spec.ts`・`params-api.spec.ts` のノブ一式のフィクスチャに `triggerPeriod_ms` を足す(roundtrip の突き合わせが通るように)。
### F. 文書
- `docs/PARAMS_KNOBS_ja.md` §7 の表に行 `| triggerPeriod_ms | CoBo | Module.triggerPeriod | int [1, 21474](unit ms) |` と、見出し・本文のノブ数(13 ノブ・葉 16)、表の下に 1 段落(GET の ms → 10 ns 換算と上限の根拠、`triggerPeriod_10ns` があれば render がエラー、実際の間隔は周期 + 読み出し、2026-10-24 TODO/179)。
- `docs/SPEC_ja.md`: §3.3 のノブの行(「**ノブ = 12(JSON の葉 15、v1.36)**」)を 13 / 16(v1.51)に、CoBo の列挙に `triggerPeriod_ms`(`Module.triggerPeriod`、ms、[1, 21474])を足す。
  先頭の status を **v1.51(2026-10-24)**に、改訂履歴の末尾に v1.51 の 1 項目(`triggerPeriod_ms` を追加、契機 = run 196 のペデスタルが 0.25 Hz、上限の根拠、`triggerPeriod_10ns` 検査)。`docs/en/SPEC_en.md` は触らない(LLM 翻訳の再生成物)。

## 2. テスト(受け入れ、TDD — 先に赤を確認)
- **T1** 検証: `triggerPeriod_ms` が無い JSON は `MissingKnob`、0 / 21475 / 1.5 / `"10"` は範囲・型エラー、1 と 21474 は通る。
- **T2** render: `triggerPeriod_ms = 10` で生成 xcfg の `Instance[*].Module.triggerPeriod` が `10`(要素のテキストだけ変わり、unit 属性は残る)。
- **T3** 往復: 既存の「抽出 → render がバイト一致」テストが新フィクスチャで通る(抽出の knobs に `triggerPeriod_ms: 1000`)。
- **T4** 要素の無いテンプレートの抽出: `triggerPeriod_ms` が `unset` に入る。
- **T5** `triggerPeriod_10ns` を持つ合成テンプレートで render がエラー(文言に `triggerPeriod_10ns`)。
- **T6** 実テンプレートの env-gated テスト(`reference/config`、あれば)が通る — 抽出に `triggerPeriod_ms`(100 / 1000)が出る。走らせられなければ理由を報告。
- UI: `cd ui && npx ng test --watch=false` 全 green(knob-spec の roundtrip 突き合わせを含む)。`npx ng build` が通る。
- 全体: `cargo fmt && cargo clippy --tests -- -D warnings && cargo test` が green(テスト数を報告)。

## 3. 報告
- `git diff --stat`(**変更してよいファイル**: 上の A–F に挙げたもの + 期待値を更新するテストファイル `tests/params_*.rs`。それ以外を触ったら理由)、T1–T6 と UI のテスト名と PASS / FAIL、cargo / UI のテスト数、
  **実行できなかった操作**(権限拒否・失敗したコマンド。無ければ「なし」)。
- `macros/` と `tools/scan/metrics_for_run.sh` は別の implementer(178)が作業中 — **触らない**。コミットしない(主対話がまとめる)。

## 4. 範囲外(主対話が後で)
- ELITPC への反映(ビルド・daq_down / up・UI の差し替え)と、**`/srv/tpcdaq/params/current.json` の移行**(新ノブが無いと GET /api/params が 500 → テンプレートの値 = 現 template の `triggerPeriod` を足す)。
  過去の `params/<id>.json` は履歴なので移行しない(読み直す経路は scan.py の土台 = current だけ)。

## 結果
### 実装(2026-10-24、Sonnet implementer、主対話 Opus が照合)
- `git diff --stat -- src tests ui docs`: 16 ファイル +223 / −27(A–F の範囲 + `src/params/mod.rs` のノブ数の doc 1 行、`docs/SPEC_ja.md` §11 Parameters ビューの「ノブ 12(葉 15)」→ 13 / 16)。`extract.rs` は無変更(既存の機構で読めた)。macros / tools/scan / docs/en は無変更。
- テスト(先に赤を確認 → green、主対話がテスト名の実在を確認):
  - T1 `tests/params_render.rs::trigger_period_ms_is_a_required_int_in_1_to_21474`(無し = MissingKnob、0 / 21475 = OutOfRange、1.5 / "10" = BadType、1 / 21474 は通る)
  - T2 `trigger_period_ms_rewrites_only_the_module_trigger_period_text`(10 → `<triggerPeriod unit="ms">10</triggerPeriod>`、変わるのはその 1 か所)
  - T3 既存の往復 `roundtrip_with_template_values_reproduces_the_template_byte_for_byte` ほか抽出 2 本が新フィクスチャで通る。config id の期待値 `1f04446c` → `2e27d9a0`
  - T4 `tests/params_extract.rs::a_template_without_trigger_period_lists_it_as_unset`
  - T5 `a_template_with_trigger_period_10ns_is_rejected`(TemplateShape、文言に triggerPeriod_10ns)
  - T6 `real_templates_round_trip_byte_for_byte_through_extract_and_render` に triggerPeriod_ms(100 / 1000)のアサート。implementer が `TPCDAQ_PARAMS_REAL_TEMPLATES` に reference/config の 3 本(pedestals 12.5 MHz / physics 25 MHz / pulser)を渡して PASS。通常実行では skip
  - oracle `tests/params_ccfg_oracle.rs`: JSON 4 か所 + 照合インスタンス数 238 → 240。`TPCDAQ_CCFG_RESOLVE_BIN` が無く**未実行**(skip)
  - UI: `knob-spec.spec.ts`「triggerPeriod_ms は int [1, 21474] ms、Module.triggerPeriod、triggerMode の直後(TODO/179)」+ 13 グループ・16 葉、params-api / params-view の spec のフィクスチャ
- **主対話の再実行(Mac、2026-10-24 17:00)**: `cargo fmt --check` OK、`cargo clippy --tests -- -D warnings` OK、`cargo test` **1108 passed / 0 failed / 1 ignored**、`ng test` **402 passed / 10 skipped**(28 files + 2 skipped)。implementer の ng build 成功。
- 実行できなかった操作: なし(implementer 報告)。
### ELITPC への反映(2026-10-24 17:10 頃、ユーザー「反映してください」、run 196 停止後の Idle 中)
- commit `1876f8c`(179 の 17 ファイルだけ。178 の macros は作業中なので含めない)→ `git bundle 93baac0..main` → ELITPC `~/tpcdaq-rs` で `git pull --ff-only` → `cargo build --release`(PATH に ~/.cargo/bin、17 s)。C++ の変更は無いので root_sink は再ビルドしない。
- Idle を確かめてから `daq_down.sh` → **current.json の移行**: 退避 `current.json.bak-2026-10-24-179`、`knobs.triggerPeriod_ms = 4000`(template `configure-pedestals.xcfg` の値)を足して serde と同じ形(キー辞書順・空白なし)で書き戻し → `daq_up.sh`。
- UI: Mac で `ng build` → 旧版を `/opt/tpcdaq/ui.prev179` に退避 → `dist/ui/browser/.` を `/opt/tpcdaq/ui/` に scp(新しいノブは `chunk-DQDknlhy.js`)。
- 確認: `/api/status` 200・`/` 200・phase Idle、`pgrep -f target/release` 8 + root_sink、今の時間帯のログに WARN / ERROR なし。
  `GET /api/params` = 200(id `pedestals__71078b40`、`triggerPeriod_ms: 4000`)。`GET /api/params/render` の configure xcfg は **run 196 の `configure-pedestals__2313b063.xcfg` とバイト一致**(移行は生成物を変えない)。
  `GET /api/params/defaults?template=configure-elinp.xcfg` = `triggerPeriod_ms: 1000`、unset / unsupported なし。
- 未: 新しいノブで実際に run を取る確認(ユーザーが次のペデスタルで。間隔 = 周期 + 2.9 ms が期待値)。
