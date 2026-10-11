# 112 — Parameters の「Apply & restart」(Stop → Save → Start を 1 ボタンに。遷移の穴を先に塞ぐ)

**Status: COMPLETED**(2026-10-11 起票・同日完了 Fable。ユーザー「パラメーターを変えながら信号が見たい」→「実装して。コマンドの遷移で一定時間コマンドを受け付けない
みたいなのはよくあるので、じっくり検証して」)

## 0. 結論(1 分で)
- AGET のレジスタは ECC の configure でしか書けない(GET の仕組み)。「変えながら見る」= **Stop run → PUT /api/params → Start run** を 1 操作にする。
  サーバに新しい状態機械は足さない(KISS)。UI が既存 REST 3 本を順に呼び、各段の結果を画面に出す。1 周の目安 = stop 数 s + start ≈ 7 s。
- **先に遷移の穴を統合テストで叩く**(レーン A)。穴があればサーバ側を直す(UI で sleep を入れて誤魔化さない — silent 禁止)。
- 最後に ELITPC 実機で 10 周(Fable)。110 の「止めずに落とすと古いフレームが混ざる」の逆 = **正規の stop → start なら混ざらない**ことも同時に確認する。

## 1. 既知の遷移(src/controller.rs、2026-10-11 読み)
- `run/stop`: 相 Running → Stopping → **Idle にしてから 200 を返す**(spawn_blocking 内で `phase = Idle`)。ECC は `stop` まで(状態 Ready)。
- `run/start`: 冒頭で `walk_ecc_back_to_idle`(status → Ready なら breakup → reset → Idle)→ describe → prepare → configure → start。
- `PUT /api/params`: 相が Idle でないと 409。ECC の状態は見ない。
- 疑うべき穴: ① stop の 200 と相 Idle の間(無いはず — 検証)、② stop 直後の start で walk-back が ECC の遅延(stop 直後の Ready 遷移中)に当たらないか、
  ③ receiver の listen 解放 → 再 listen(同じポート、TIME_WAIT)、④ root-sink の run close が終わる前に次の run が開く(raw / PEventTPC のファイル名 inprogress の取り合い)、
  ⑤ 連続 N 周で token / audit / logbook が壊れないか、⑥ PUT と start の間で別クライアントが token を奪った場合の挙動(409 / 401 が UI に出ること)。

## 2. レーン A(Rust: 遷移の検証 + 必要なら修正)— implementer、**model: opus**
1. `tests/controller_integration.rs` に **back-to-back 試験**を足す(既存の `params_harness` と fake ECC / replay のハーネスを使う。`params_id_drives_prepare_and_configure_config_id_at_run_start` が参考):
   - (a) run/start → 数十フレーム → **run/stop の 200 が返った直後に、待ち時間ゼロで PUT /api/params(値を 1 つ変える)→ run/start**。PUT が 200、start が 200、
     logbook の `run_start.config_id` が新しい id、ECC(fake)が受けた configure の id が新しい id。
   - (b) これを **5 周**繰り返す(毎周 triggerDelay を変える)。各周で run 番号 +1、audit の順序(run/stop → params/put → run/start)、`forced_eos` / `eos_closed`、
     root-sink 側(replay ハーネスに root_sink がいれば)の run close 件数 = 周回数、**inprogress ファイルが残らない**、receiver が毎周 listen できる。
   - (c) 異常系: stop 直後に **別の operator が acquire で token を奪った**状態で PUT → 401(文言)、start → 401。UI が出せる形で返ること。
   - (d) stop の 200 を受けた瞬間の `GET /api/status` が `phase: Idle`(タイミング穴 ① の直接検証。100 回ループで一度も Stopping を返さない)。
2. 穴が見つかったら**サーバ側を直す**(例: stop の返答前に ECC の状態が Ready に落ち着くまで待つ、root-sink の close を GetStatus で待つ等)。直し方が SPEC に触れる(§1.3 の停止手順)なら**実装せず報告**。
3. ゲート: `cargo fmt && cargo clippy --tests -- -D warnings && cargo test`。報告 = 試験の各項目の実測(周ごとの所要 ms)と、見つかった穴 / 直した点。

## 3. レーン B(UI: Apply & restart)— implementer、**model: sonnet**
1. `ui/src/app/params/params-view.{ts,html,scss}` に **Apply & restart** ボタン(Save の隣)。有効条件 = Save と同じ(token あり、値が全部ある)+ **Running 中でも押せる**(Save は Running で無効のまま)。
   Idle のときは Save → Start run と同じ(stop を飛ばす)。
2. 順序と表示(各段を画面に出す。進行中は全ボタン無効):
   `1/3 Stopping run N…` → `POST /api/run/stop {token}` → **`GET /api/status` を 200 ms 間隔で `phase == "Idle"` になるまで(上限 30 s)**(stop の 200 を信用し切らない — 穴 ① の保険。
   上限超過は「Stop did not reach Idle in 30 s」でエラー表示して止める)→ `2/3 Saving parameters…` → `PUT /api/params` → `3/3 Starting run…` → `POST /api/run/start {token, comment}`
   (comment は Parameters の note 欄があればそれ、無ければ `apply: <params id>`)→ `Run N+1 started with <id>`。
   どの段で失敗しても **サーバの文言をそのまま**出し、以降の段は実行しない(stop 済みで start 失敗 = run は止まったまま — それも明示)。
3. 既存の `RunControlApiService` / `ParamsClient` を使う(新しい fetch を足さない)。`run/start` / `run/stop` の呼び方は `run-control-api.ts` にある。
4. テスト(`params-view.spec.ts`、モック): Running → 3 段成功 / Idle → 2 段 / stop 失敗で止まる / Idle 待ちの上限超過 / PUT 400 で start しない / start 401 で「run は止まったまま」の表示 / 進行中はボタン無効。
   ゲート: `cd ui && npm test -- --watch=false` + `npm run build`。
5. **やらないこと**: サーバ側、ノブ追加、Waveform 画面の変更。

## 4. 共通
- worktree 隔離、着手前 `git merge --ff-only main`(main `0a591b1` 以降)。発注書の絶対パス `/Users/aogaki/Workspace/tpcdaq-rs/TODO/112_apply_and_restart.md`。
- 完了報告 = 変更ファイル / テストのコマンドと件数 / 迷った点。Fable が一括レビュー → マージ → ELITPC 実機 10 周(所要時間、ECC 状態、stale frame なし、root-sink close)。

## 結果(2026-10-11、Fable 一括レビュー + 実機)

- マージ: レーン A `4ff0410` → `(merge)`、レーン B `1e04d04` → **`84245cc`**。ゲート(main): `cargo test` **871 passed / 0 failed / 1 ignored**、clippy 警告 0、UI **310 passed / 5 skipped**、build 成功。
- レーン A(統合テスト 4 本、`tests/controller_integration.rs` 30 本に): (a)(b) fake ハーネスで stop の 200 直後に待ち 0 で PUT → start を 5 周(周 12〜30 ms、毎周 id が切り替わり
  ECC の configure id / logbook / audit 順序が一致)、実コンポーネント版 5 周(receiver は毎周同じポートに 1 回目で bind、decoder の PULL bind は 3〜4 回リトライ 70〜160 ms = 034 の想定内。
  予算 6 回 / 620 ms に対し余裕 ≈ 4 倍)、(c) token 横取り → PUT / start とも 401 `stale or unknown token`、(d) stop の 200 直後の status は **100 周すべて Idle**。
  **正常系に穴なし、サーバのコード変更なし**。見つかった穴 X(停止時の Reset 不達 → 以後 start が失敗し続ける、正常環境では 100 周で 0 回)は **113** に起票(裁定: start がコンポーネントも歩き戻す)。
  `put_params` のロック外書き込み(別クライアントとの競合)も 113 に同梱。
- レーン B(UI): Apply & restart = Stop → `GET /api/status` を 200 ms 間隔で Idle まで(上限 30 s、取得失敗は再試行)→ PUT → Start、各段を `1/3 Stopping run N…` の形で表示、失敗はサーバ文言のまま +
  「run は止まったまま」を明示、進行中は全ボタン無効。spec 11 → 12 本、**変異確認**(Idle 待ち除去 / PUT 失敗後に start / 進行中の無効化除去)でそれぞれ該当 spec が赤。
- **実機 10 周(ELITPC、`~/apply_cycle.py`、13:24–13:27、α 線源 91 mbar、triggerDelay を 330 / 320 と交互に)**: run 41–50。

  | 項目 | 実測 |
  |---|---|
  | stop | 3.0〜4.4 s(1 周目のみ Idle からなので 5 ms) |
  | Idle 待ち | 毎周 1 回目の status で Idle(5〜7 ms)= 穴 ① なし |
  | PUT | 4 ms |
  | start | 7.46〜7.97 s(ECC walk-back `status->Ready, breakup->Prepared, reset->Described, reset->Idle` が毎周、arm_retries 空) |
  | **1 周** | **≈ 12 s** |
  | 取得 | 各 run 8 s で 33〜49 frames(≈ 4〜6 Hz)、ECC Running |
  | root_sink | run 41–50 すべて `closed … late_fragments=0`、`run_inprogress` 残り 0 |
  | 古いフレーム | 全 run で graw の先頭 eventIdx = 0、連番、CoBo 時刻単調増加 = **正規の stop → start では 110 の混入は起きない** |
  | 生成 xcfg | `configure-elinp__b77878fe` triggerDelay 330 / `…7f355552` 320(交互に反映)、ECC ログに `Configure: loading … elinp__…` 11 回 |

  試験後、`elinp` 無変更(note だけ違う → id `elinp__6e39bcc8`、`configure-elinp.xcfg` とバイト一致)で α run 51 を再開、取得中。ユーザーも run 38–40 で手動の stop → 保存 → start を回していた(実運用の先行例)。
- 副産物: **108 の原因が判明** — root_sink の `run N closed (events_complete=…)` はプロセス通算(49 / 98 / 137 / 173 …)。108 に記録。
- UI の目視: ユーザーがブラウザで確認(2026-10-11)「UI はいいと思います」。
