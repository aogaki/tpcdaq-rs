# 036 — ECC を確実に Idle へ戻す(実 ECC の `EV_UNDO` 意味論への対応)+ fake_ecc の遷移表を実機準拠に

**Status: COMPLETED**(2026-08-14 implementer/Opus → 発注側レビュー PASS +
**ハングする既存テスト 2 本の処置を発注側が実施**)

## 結果

### 実装

- **`Sequencer::walk_ecc_back_to_idle`**(`src/controller.rs`): `status` で現在状態を取り、
  `Running/Paused→stop` / `Ready→breakup` / `Prepared|Described→reset` を**必要な段数だけ**発行。
  `Off`/`Idle` は **1 コマンドも出さない**。上限 4 段(`ECC_WALK_BACK_MAX_STEPS`)。
  **run を始めない条件を 3 つとも実装**: ①歩けない状態(`Unknown` 等)②**コマンドが `ok` なのに
  状態が動かない**(= 実 ECC の無音無視を捕まえる唯一の手がかり)③段の失敗。
  跡は audit に `ecc_walk_back = ["status->Ready","breakup->Prepared","reset->Described","reset->Idle"]`。
- **テストダブルを実機準拠に**(`tools/ecc_bridge/`): `next_state` の戻り値を
  `bool → enum class Step { Applied, Ignored, Denied }` にし、**`Ignored` は副作用も含めて
  何もしない**(`configure` / `reset` の早期 return)。実 ECC の「黙って無視・黙ってスキップ」を再現。

### **red の実物 — 実機準拠の表を先に入れて 034 実装が落ちることを実測**(本ユニットの核心)

```
fake_ecc: reset IGNORED in state Ready (real ECC: no such transition = silence)
ecc_bridge: reset -> ok=1 state=Ready          ← 034 の「reset 1 発」が無音で無効
fake_ecc: invalid transition: describe in state Ready
assertion failed: 1 本目の run/start が 1 発で通らなかった … left: 500  right: 200
```
単体でも `a_ready_ecc_is_walked_back_to_idle_before_the_run_starts` が
`left [ecc reset, graw-writer Configure, …]` vs `right [ecc status, ecc breakup, ecc reset, ecc reset]` で red。
**「テストダブルが甘いと誤実装が green で通る」を実証的に閉じた**。

### テスト(2026-08-14、macOS Darwin 25.5.0 / **発注側で全て再実行**)

| コマンド | 結果 |
|---|---|
| `cargo test` | **402 passed / 0 failed / 1 ignored**(FAILED 行 0) |
| `cargo fmt --check` / `clippy --all-targets -D warnings` | OK / 警告ゼロ |
| `make -C tools/ecc_bridge test -j` | **test_ecc_bridge 187 passed / 0 failed**(036 前 136) |
| `make -C tools/ecc_bridge e2e` | **ecc_e2e 29 passed / 0 failed**(下記処置後) |
| `cargo test --test ecc_bridge_intake`(env 付き) | 2 passed / 0 failed(同上) |
| **E2E-H(release、全 env、実データ・実 Ice 配線)** | **1 passed / 10.90 s。run 番号 [1,2,3] / run/start 所要 [0.013, 0.163, 0.165] s / Arm リトライ [{decoder,4,154ms},{decoder,4,157ms}]** |
| P3 E2E 全体(実装側実行) | 5 passed / 0 failed / 241 s。E2E-D 跨 run 境界停滞 **0.000 s**(不変)、3 点計測 ±2% ✔ |

### 発注側の処置 — ハングする既存テスト 2 本(**実装側は所有権外として報告・掟どおり**)

実機準拠化の帰結として、**`configure` を `Ready` から撃ち直す**前提のテスト 2 本が
「失敗」ではなく**無限ハング**するようになった(実 ECC は `ST_PREPARED` ガードで黙ってスキップ →
リンクが張られず `accept()` が永久ブロック)。**旧遷移表の「Ready から configure を掛け直せる」は
実機に存在しない虚構**だった。発注側が実装側の提案どおり修正(理由コメント付き):

- `tools/ecc_bridge/ecc_e2e_client.cpp` — 張り替え前に `breakup`(Ready→Prepared)を挿入 +
  最後の `reset` を **2 段**(`Prepared→Described→Idle`)に。
- `tests/ecc_bridge_intake.rs` — 同じ 2 点。
- **処置後に発注側が実行して green を確認**(`ecc_e2e 29 passed` / `intake 2 passed`)。

### レビュー(発注側 Opus)

- 逸脱 6 件すべて受理。特に:
  - **`Step::Ignored` / `Denied` の分離** — `configure` の「黙ってスキップ」は `bool` では
    再現できない。API 変更の波及は所有ファイル内に収まっている。
  - **`describe`/`prepare`/`start`/`stop` の順序違反は `Denied` のまま残した**(実機は無音)。
    「ダブルは実機より**甘くしてはならない**が、辛い分は run が可視に失敗するだけ」という理由づけは
    正しい。**しかもこの辛さがあるから 034 の実装が E2E で落ちた**。
  - **`ecc_state_after_reset` のキー名を独断で改名しなかった**のも良い(公開フィールド名は
    発注側の領分)。
- 既存テストの変更 7 件はすべて「実機の意味論に合わせた結果」。特に **E2E-G の手動 `ecc/reset` が
  `"Idle"` → `"Running"` になった**のは、**オペレータの手動 reset は実機では効かない**という
  罠をテストが固定した形で、復旧は続く `run/start` の歩き戻しが担う。正しい。

### P5 現地で見るところ(実装の副産物)

1. **audit の `ecc_walk_back`**(`logbook.jsonl` の `type=audit, action=run/start`):
   2 本目以降が `["status->Ready","breakup->Prepared","reset->Described","reset->Idle"]` なら
   歩き戻しが効いている。**`["status->Ready"]` 1 要素だけで run が始まっていたら 034 の事故の再来**。
   1 本目は `["status->Idle"]` か `["status->Off"]`。**`Off` が出続けるなら**実 ECC の `Off` が
   「ECC 不達」を意味していないかを疑う(我々は `Off` を「これ以上戻れない底」として素通しする)。
2. `ecc <action> left the ECC in <state> — the real ECC ignores undefined transitions in silence`
   が出たら、実機の SM が想定と違う段がある。その組を一次資料と照合する。
3. `ecc is in state <X> — refusing to start a run` で `Unknown` 以外の綴りが出たら、
   実 ECC の状態名がブリッジのマップ(`ecc_core.hpp` の `state_from_string`)に無い。
4. `run/start` は歩き戻しの分だけ 2 本目以降が伸びる(E2E 実測 0.013 → 0.16 s。
   **実機は FPGA 設定で秒オーダーになるはず** — `ecc_timeout` 既定 60 s に対する余裕を現地で 1 度測る)。
**起票**: 2026-08-14([archive/034_consecutive_run_ops.md](archive/034_consecutive_run_ops.md)
の発見。**発注側が実 ECC ソースで裏取り済み**)
**仕様**: SPEC **v1.12** §1.3(run 開始シーケンス — **訂正済みの ⚠ 注記が本ユニットの仕様**)/
§8.2(ecc-bridge の状態)
**Warsaw 必須度: 最高**(これが無いと **2 本目以降の run が成立しない**。ビームタイムは連続 run)
**発注先想定**: implementer/**Opus**(状態機械の歩き戻し + テストダブルの忠実化)

## 事実(発注側が `reference/20190315_patched` で確認済み)

| 出典 | 内容 |
|---|---|
| `GetBench/src/get/rc/BackEnd.cpp:924` | `reset()` は `engine.step(EV_UNDO)` のみ |
| 同 `:250-270` | `EV_UNDO` は **`Described→Idle`** と **`Prepared→Described`** の 2 本だけ。**`Active`(Ready/Running/Paused)からは存在しない**。`Active→Prepared` は **`EV_BREAK`(= breakup)** |
| `StateMachine/src/dhsm/Engine.cpp:344` | 未定義遷移は例外を投げず **`Ignored`**(**完全な無音**) |
| `BackEnd.cpp:955-962` | `configure` は **`if (state == ST_PREPARED)` ガードで黙ってスキップ** |

**帰結**: `ecc stop` 後(= `Ready`)に `reset` を 1 回打つ現状の実装(034)は**何もしない**。
続く `describe` / `prepare` も無音、**`configure` がスキップ**され、`start` だけが成功する。
= **CoBo がデータリンクを張り直さないまま run が始まる**(実機では前 run のソケットを receiver が
閉じているので `start` が「Could not establish data link.」で落ちる見込みだが、いずれにせよ
2 本目は成立しない)。**fake_ecc は「どこからでも Idle」なのでテストでは踏めない**。

## やること

1. **controller: ECC を確実に `Idle` へ戻す**(`src/controller.rs` の run 開始シーケンス先頭)。
   - **現在の ECC 状態を取得**(`{"action":"status"}` — §8.2)し、**必要な段数だけ歩き戻す**:
     `Running/Paused` → (`stop`) → `Ready` → **`breakup`** → `Prepared` → **`reset`** →
     `Described` → **`reset`** → `Idle`。`Off` / `Idle` なら何もしない。
   - **状態が想定外(`Unknown` 等)なら run を始めない**(黙って進まない)。
   - 歩き戻しの各段と最終到達状態を **audit に残す**(034 の `ecc_state_after_reset` を拡張してよい)。
   - **`Idle` に到達できなかったら run/start を失敗させる**(next_run の巻き戻しは 034 実装済み)。
2. **fake_ecc / ecc_core.hpp の遷移表を実 SM 準拠にする**(`tools/ecc_bridge/`)。
   - **これが本ユニットの肝** — テストダブルが甘いから 034 の実装が「通ってしまった」。
     `reset` = 1 段戻す(`Active` からは**無視**)、`breakup` = `Active→Prepared`、
     `configure` は `Prepared` からのみ(それ以外は**無音でスキップ** — 実機と同じ意地悪さ)。
   - **016/017 の既存テストが壊れる可能性がある**。壊れたら「実機の意味論に合わせた結果」なので
     期待値を更新してよいが、**変更したテストごとに理由を報告**すること。
3. **回帰テスト**: E2E-H(034 が入れた連続 3 run)が**実機準拠の遷移表の下で green** であること。
   加えて `Ready` からの歩き戻しを**単体で**固定(状態を偽って与え、発行されたコマンド列が
   `breakup → reset → reset` になること / `Idle` なら何も発行しないこと)。
4. **`stop_run` 冒頭コメントの訂正**(034 の積み残し): 「ecc stop(CoBo が送信を止め TCP が閉じる)」
   は SPEC v1.12 の事実修正により誤り。**コメントのみ**直す(挙動は 033 の担当)。

## 受け入れ

- `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test` 全 green
  (**ベースライン: 034 完了時点 388 passed / 0 failed**)。
- `make -C tools/ecc_bridge test -j` green。P3 E2E(全 env、release)green。
- ファイル所有権: `src/controller.rs` / `tools/ecc_bridge/{ecc_core.hpp, fake_ecc.cpp, test_ecc_bridge.cpp}` /
  `tests/p3_e2e.rs`(E2E-H の補強のみ)/ 既存 controller テスト(期待値更新)。
  **`src/receiver.rs` は 032 が編集中 — 触らない**。`docs/` `TODO/` `ui/` に触らない。
- **P5 現地確認項目**: 実機で歩き戻しが効いていること(audit の ECC 状態遷移列で機械確認)。

## 備考

- **発注側の落ち度の記録**: 034 の発注書と SPEC v1.12 初稿は `ecc_core.hpp` のコメント
  「reset はどこからでも Idle(復旧手段は塞がない)」を鵜呑みにしていた。**テストダブルの
  コメントを一次資料として扱ったのが誤り**。一次資料は `reference/20190315_patched` の実 ECC。
  034 の実装が**実装せず報告して戻った**ので実機事故にならずに済んだ(掟が機能した好例)。
- 033(異常系セマンティクス)も `src/controller.rs` を触るので、**036 → 033 の順**で発注する。
