# 187 — UI: v1.53 / v1.54 の表示(ROOT DROPPED・STOPPING draining・OVERLOAD・ログブックの root_complete)

**Status: IMPLEMENTED — マージ待ち・実機の受け入れ待ち(2026-11-04)**(発注書 — Fable 2026-11-04 起票。11/9 前に ③ と ① は要る。Sonnet implementer、204 のキー名が決まれば 204 と並行可)**(設計 = TODO/181 §4-5 / §5-1、仕様 = SPEC v1.53 §11「v1.53 の表示(TODO/187)」+ 204 の v1.54 案(`/api/status.stopping.root_sink_backlog`))

## 0. 背景(1 行ずつ)
- run 197: Stop の後も root_sink が溜まった分を書き続け、UI は STATE running / RUN 197 のまま → ユーザーに「止まっていない」と見えた。
- run 214(204): 停止が分単位になる設計(SPEC §1.3 v1.53「`POST /api/run/stop` の応答は分単位になりうる(UI は応答を待たず phase を見る、§11)」)。204 の後は root_sink の書き切りまで phase `Stopping` が続く(run 214 の条件で ≈ 12 分)。**今の UI は run/stop の応答を待つ**(`ui/src/app/api/run-control-api.ts`、`run-view.ts`)— 分単位の応答は「固まった」に見える。
- `decoder_link_dropped_frames > 0`(= この run の ROOT は欠けあり、graw から作り直す)は今 `/api/status` の JSON にしか出ない。

## 1. 変更(決定済み。優先順 = ③ → ① → ⑤ → ② → ④)
データ源はすべて既存: `GET /api/status`(`phase`、`stopping`、`components[].state / metrics`)、`GET /api/logbook`(run_stop の `counters.root_complete`、`reason`)、monitor WS の `status`(root_sink の `run` / `state`)。**新しい API は作らない**。
- **③ STOPPING — draining**(ステータスバー + Run 画面): `phase == "Stopping"` のとき「STOPPING — draining · ROOT backlog N · waited S s」(`stopping.root_sink_backlog` が null なら `decoder_backlog`、それも null なら数字無しで「draining」だけ)。**Stop ボタンの実行は応答を待って完了扱いにするのではなく、押した直後から `/api/status` の phase を見て Stopping を表示し、Idle に戻ったら最後の run_stop(ログブック)の `ok` / `reason` を出す**。HTTP の応答が先に切れても(プロキシ・ブラウザのタイムアウト)エラー表示にせず phase を信じる。応答が返れば今どおり `ok:false` を失敗として出す。
- **① ROOT DROPPED**(ステータスバー): いずれかの receiver の `metrics.decoder_link_dropped_frames > 0` で赤いチップ「ROOT DROPPED N」(N = 合計)、ツールチップ「graw is complete; rebuild ROOT from graw (SPEC §1.4-7)」。run ごとに 0 に戻る(receiver の Start で reset される既存のカウンタをそのまま表示)。
- **⑤ ログブック**(`logbook-record.ts`): run_stop 行に `counters.root_complete === false` なら「ROOT incomplete」の印(チップ)、`reason` が `normal` 以外なら reason を今どおり目立たせる。`root_complete` が null(古い run)は印なし。
- **② OVERLOAD**(ステータスバー): receiver または graw-writer の `state == "Error"` で赤「OVERLOAD」(188 の後は run_stop の `abort:overload:…` も同じ赤で)。
- **④ ROOT 書き出し中**(ステータスバー、低優先): controller の `phase == "Idle"` かつ monitor WS `status` の root_sink `state == "running"` → 「ROOT 書き出し中 run N」。204 の後は eos-timeout で打ち切った場合だけ起きる。WS の status が Run 画面に無ければ status バーの既存の購読を使う(無理なら本項は見送って報告)。

## 2. テスト(Angular spec、既存の `status-bar` / `run-view` / `logbook-record` の spec の流儀。名前は固定)
1. `status-bar shows STOPPING draining with the root_sink backlog while phase is Stopping`(backlog null のときの退避表示も 1 ケース)
2. `run view keeps showing Stopping from /api/status even when the stop request has not answered yet`(応答を遅らせたモック → Stopping 表示 → Idle で run_stop の ok を表示)
3. `run view does not show an error when the stop request fails on the wire while phase is Stopping`
4. `status-bar shows ROOT DROPPED with the summed decoder_link_dropped_frames`(0 のとき出ない)
5. `logbook record marks run_stop rows with root_complete false`(null / true は印なし)
6. `status-bar shows OVERLOAD when a receiver or graw-writer is in Error`
7. (④ を実装したら)`status-bar shows ROOT writing when controller is Idle and root_sink still runs`
- 全体: `npm test`(headless)緑、`npm run build` 警告なし。既存 spec 無改変 green。

## 3. 受け入れ(実機 — 読むだけ。デプロイは主対話が RUNBOOK §7.1 で)
- ELITPC に UI を反映後、通常 run の Stop で「STOPPING — draining」→ Idle の順に表示が切り替わること(スクリーンショット)。pevent = true の受け入れ(204)と同時に取れれば backlog の数字が減る様子も。

## 4. 範囲・禁止
- 触ってよい: `ui/src/app/status-bar/*`、`ui/src/app/run/*`、`ui/src/app/logbook/*`、`ui/src/app/api/run-control-api.ts`(応答待ちの扱い)、必要なら `ui/src/app/ws/*`(④ の購読)。
- **触らない**: `src/`、`tools/`、`docs/`、`TODO/`(本チケットの `## 結果` 以外)。新しい API を要求しない(足りないキーがあれば実装せず報告 → 204 / 188 側で足す)。
- モデル: **Sonnet**(発注書と spec で縛れる)。

## 5. 報告(必須)
worktree のブランチとコミット、`git diff --stat`、テスト 1–7 の名前・PASS / FAIL(④ を見送ったら理由)、赤の証拠、spec の総数、発注書からの逸脱と理由、**「実行できなかった操作」の節(無ければ「なし」)**。

## 結果(2026-11-04 — 実装・テスト済み、マージと実機の受け入れは未)
- **実装**: implementer(Sonnet)、worktree `.claude/worktrees/agent-a4088e0bfe5be2ee3`、ブランチ `worktree-agent-a4088e0bfe5be2ee3`、コミット **`95d177c`**(main 663b00d の上、未マージ・未 push)。11 ファイル + 759 / − 2: 変更 `api/controller-status.ts`(+ 74 — §4 の `run-control-api.ts` の代わりに status の型と parse に `stopping`・`rootDroppedFrames` を足した。範囲は `ui/src/app/` の中)、`logbook/logbook-record.ts`、`run/run-view.{ts,html,scss}`、`status-bar/status-bar.{ts,html,scss}`。新規 spec 3 本(`status-bar/status-bar-controller.spec.ts`、`logbook/logbook-record-root.spec.ts`、`run/run-view-stop.spec.ts`)。既存 spec は無改変。主対話が `git show --stat 95d177c` と 7 本のテスト名(各 1 件)を照合。
- **テスト**: 実装前 `npm test` = 10 failed / 403 passed(新規 spec が `.stopping`・`.root-dropped`・`.overload`・`.root-writing`・ROOT フィールドを見つけられない)→ 実装後 **31 files passed / 2 skipped、413 passed / 0 failed / 10 skipped**(10 skipped は元から)。`npm run build` 警告なし。**主対話が worktree で再実行して同じ結果**(2026-11-04 10:20、vitest 4.1.10、1.64 s、build 3.99 s)。
  1. `status-bar shows STOPPING draining with the root_sink backlog while phase is Stopping`(+ 退避表示の別ケース `… falls back to the decoder backlog, then to a bare draining …`)— PASS
  2. `run view keeps showing Stopping from /api/status even when the stop request has not answered yet` — PASS
  3. `run view does not show an error when the stop request fails on the wire while phase is Stopping` — PASS
  4. `status-bar shows ROOT DROPPED with the summed decoder_link_dropped_frames`(receiver だけを足す: 6,251 + 17 = 6,268、decoder の 999 は混ぜない、0 は出ない)— PASS
  5. `logbook record marks run_stop rows with root_complete false`(true / null / キー無しは印なし)— PASS
  6. `status-bar shows OVERLOAD when a receiver or graw-writer is in Error`(decoder の Error では出さない)— PASS
  7. `status-bar shows ROOT writing when controller is Idle and root_sink still runs`(④ は WS の status で実装できた)— PASS
  - 追加: `run view shows a failed run_stop (ok false, reason) once phase returns to Idle`、`run view still shows a wire failure when the controller is not Stopping`、reason が normal 以外でも印と独立に並ぶ。
- **設計の要点**: ステータスバーが `/api/status` を 5 s ごとに poll(取れないときは新設の `ctl` が `—` = 「異常なし」と取り違えない)。Stop は応答を待たず 1 s ごとに phase を追い、Idle に戻ったらログブックの seq 最大の run_stop の ok / reason を出す。通信エラーでも phase が Stopping ならエラーにしない(Running のままなら従来どおり失敗)。ログブックの印は既存の `fields` に `ROOT: incomplete — graw is complete; rebuild ROOT from graw`。
- **逸脱**: ④ の文言は英語(「ROOT writing · run N」— UI が英語)。⑤ はチップでなく既存の `fields`(`LogbookEntry` の型を変えない)。`ctl` 項目の追加(silent failure 回避)。run_stop の探索に `getLogbook(0)` を Stop 完了時に 1 回。prettier が無関係なファイルを整形した分は戻した(コミットに入っていない)。
- **実行できなかった操作**: なし(複合コマンドの権限拒否 3 回は分割 / Edit で回避)。
- **未**: 204(controller の `stopping.root_sink_backlog`・`stopping.root_sink`)と一緒にマージ → ELITPC に反映 → 実機の受け入れ(§3、通常 run の Stop で「STOPPING — draining」→ Idle、pevent = true の受け入れで backlog が減る様子)。
- **マージ(2026-11-04)**: main に `654f2fd`(Merge 187、204 と同時)。main の上で `npm test` 413 passed、`npm run build` 警告なし。未: ELITPC 反映と実機の目視(§3)。
