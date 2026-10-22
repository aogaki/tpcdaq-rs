# 172 — UI: status の ZS 復元カウンタ 3 キーを受け取って表示する(「unknown status keys」の解消)

**Status: COMPLETED**(2026-10-22 起票・同日完了 Opus、実装 Sonnet implementer。ユーザー「unknown status keys: uvwTableMissingChannels, uvwZsFragmentsNoTable, uvwZsFragmentsRestored こんなん出てます。なんでしょうか?」)

## 0. 原因
- 152(2026-10-16、`a46c8be`)で monitor の status JSON に ZS 復元のカウンタ 3 キーが加わった(`src/monitor/json.rs` の `status_json`、元は `src/monitor/display.rs:52–56` の `ConvertCounts`)。
- UI の既知キー一覧 `ui/src/app/ws/json.ts` の `STATUS_KEYS`(375 行付近)に無いので、「unknown status keys」として表示される。害は無いが、2 つは 0 でなければ問題を示すカウンタで、見えていないと困る。159(root_sink 側の ZS カウンタ)とは別件。

| キー | 意味 | 0 でないとき |
|---|---|---|
| `uvwZsFragmentsRestored` | ZS フラグメントをペデスタル表で復元した数 | 正常(ZS run で増える) |
| `uvwZsFragmentsNoTable` | ZS フラグメントが来たが表が無く復元できなかった数(0x02 の基線 ≈ 0 で出る) | **問題** |
| `uvwTableMissingChannels` | 表に無い ch に当たった数 | **問題** |

## 1. 変更(UI のみ。Rust・SPEC は触らない)
### A. `ui/src/app/ws/json.ts`
- 3 キーを `STATUS_KEYS` に足す。
- `StatusMessage` に任意フィールドとして足す(例 `readonly uvwZsFragmentsRestored?: number` ほか 2 つ)。
- 読み方は**任意の数値**: キーが無い(152 より前の monitor)→ フィールドを持たない(`undefined`)。キーがあって数値でない → `fail('bad-field', 'status.<key>')`(既存の必須キーと同じ流儀)。
  キーがあれば `status` オブジェクトに載せる(`slow` と同じ `...(x !== undefined ? { x } : {})` の形)。
### B. ステータスバー(`ui/src/app/status-bar/status-bar.ts` / `.html` / `.scss`)
- `wsDropped` の隣に 3 つを表示。ラベルは短く: `zsRestored` / `zsNoTable` / `zsTableMissing`(キー名との対応を ts のコメントに書く)。
- キーが無い(古い monitor)ときは既存の `format()` と同じく「—」。
- `zsNoTable` か `zsTableMissing` が 0 より大きいときは警告の見た目にする(既存の警告 / alarm 用のクラスがあればそれを使う。無ければ scss に 1 クラス足す)。`zsRestored` は普通の表示。
- tooltip(title 属性)に表の「意味」列を短く。

## 2. テスト(受け入れ)
- `ui/src/app/ws/json.spec.ts`:
  - 3 キーつきの status が `unknownKeys` 空で通り、値がそのまま載る。
  - 3 キー無しの status も通る(フィールドは undefined)。
  - どれか 1 つが文字列なら `bad-field`(`status.<key>`)。
- `ui/src/app/status-bar/status-bar.spec.ts`:
  - 3 つの表示(値あり / 無しで「—」)。
  - `zsNoTable > 0` または `zsTableMissing > 0` で警告クラスが付き、両方 0 なら付かない。
- 全体: `cd ui && npx ng test --watch=false` が全 green(既存の件数から +N、skipped は env-gated の既存分のみ)。`npx ng build` が通る。`./run_ws_conformance.sh` が通る(バイナリ側は不変のはず)。
- Rust は触らないが、念のため `cargo test monitor::json` が通ることを確認(キー名の一次資料)。

## 3. 報告(implementer → 主対話)
- `git diff --stat`、追加したテストの名前と結果、`ng test` の passed / skipped 件数、`ng build` と適合スクリプトの結果。
- **実行できなかった操作**(無ければ「なし」)。

## 4. 範囲外
- ELITPC への反映(UI のビルドを入れ替えるのはスタックの差し替え = RUNBOOK §7.1、ユーザーと相談して別途)。
- 159(root_sink の ZS カウンタ)、171(slow の age)。

## 結果(2026-10-22、Mac、実装 Sonnet implementer → 主対話 Opus が照合)
### 変更(UI のみ、5 ファイル +147 / −2)
- `ws/json.ts`: 3 キーを `STATUS_KEYS` に、`StatusMessage` に任意 number フィールド。読みは既存の `optionalNumberAt` / `BAD`(`slow.counters` の influx_* 用)を再利用 = 無ければ undefined、数値でなければ `bad-field status.<key>`。載せ方は `slow` と同じ形。
- `status-bar.ts` / `.html`: `wsDropped` の隣に `zsRestored` / `zsNoTable` / `zsTableMissing`(キー名との対応は ts のコメント)、無ければ「—」、`zsNoTable` か `zsTableMissing` > 0 で既存の `.bad`(decodeErrors / slow と同じ警告の見た目)、tooltip に意味。scss は不変。

### テスト
- 追加 6 件(すべて pass):
  - `json.spec.ts` describe「TODO/172 status の ZS 復元カウンタ 3 キー(任意の数値)」: 3 キーつきで unknownKeys 空・値そのまま / 3 キー無しで通り undefined / 各キーが文字列なら bad-field(3 キーをループ)。
  - `status-bar.spec.ts` describe「StatusBar — TODO/172 ZS 復元カウンタ」: 値の表示(1234 / 0 / 7)/ キー無しで「—」かつ bad なし / zsNoTable > 0 か zsTableMissing > 0 で bad、両方 0 なら付かない(zsRestored は常に普通)。
- `cd ui && npx ng test --watch=false`: **28 files passed / 2 skipped、401 passed / 10 skipped**(skipped は env-gated の既存分)。主対話でも再実行して同じ件数。
- `npx ng build`: 成功(警告なし)。`./run_ws_conformance.sh`: 30 files / 411 passed / 0 skipped。`cargo test monitor::json`: 9 passed / 0 failed。
- 主対話の照合: `git status` で変更は ui の 5 ファイルのみ、両 describe がファイルに存在。
- スキップ: なし。ELITPC への反映は未実施(範囲外、ユーザーと相談)。

## 実行できなかった操作
なし(implementer・主対話とも)。
