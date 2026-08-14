# 034 — 連続 run の運用性(ECC を毎回 describe から始める / Arm の bind 競合 / run 番号の空費)

**Status: COMPLETED**(2026-08-14 implementer/Opus → 発注側レビュー PASS。
**⚠ 実 ECC の reset 意味論の発見により続きは [036](036_ecc_walk_back.md)**)

## 結果

### 実装(3 ファイル。`src/state.rs` は変更不要だった)

- `src/controller.rs`(+374/−14): **論点 A** = `Sequencer::start_run` の**第 1 手**が `ecc reset`
  (コンポーネントに 1 コマンドも触る前。1 本目も同じ経路。reset 失敗なら run を始めない)/
  **論点 B** = `arm_with_retry`(20 ms から倍々、最大 6 試行 = 眠り合計 620 ms。**詰まった
  コンポーネントだけ**撃ち直し、成功しても `ArmRetry{component, attempts, waited_ms}` を audit へ)/
  **論点 C** = 失敗経路で `set_next_run(path, run)` により巻き戻し(`run_start` は成功経路でしか
  書かないので「書いた後に巻き戻す」経路は構造上存在しない)+ 巻き戻しの成否も audit。
- `tests/controller_integration.rs`(+75/−6)/ `tests/p3_e2e.rs`(**E2E-H 新規** +
  E2E-D から手動 workaround を撤去)。

### テスト(2026-08-14、macOS Darwin 25.5.0 / ROOT 6.36.10)

| コマンド | 結果 |
|---|---|
| `cargo test` | **388 passed / 0 failed**(034 前 381 = **+7**) |
| `cargo fmt --check` / `clippy --all-targets -D warnings` | OK / 警告ゼロ |
| `cargo test --release --test p3_e2e`(全 env・実データ・実 Ice 配線) | **5 passed / 0 failed / 236 s** |

- **E2E-H(新規)**: run 番号 **[1, 2, 3]**、`run/start` 所要 [0.011, 0.161, 0.153] s、
  Arm リトライ実績 `[{decoder, attempts:4, waited_ms:155}, {decoder, attempts:4, waited_ms:151}]`。
  **テスト側に手動 `ecc/reset` もリトライループも置いていない**(置いたら壊れても気づけない)。
  3 本とも `entries=108` / `fatal=""` / `late_fragments=0`。
- **E2E-D 跨 run**: 実 run 番号 [1, 2]、**境界停滞 0.000 s(030 の結論は不変)**、
  3 点計測 0 Hz −0.83% / 2 Hz −0.05% / freeze −0.46%(±2% ✔)。
- **リトライ上限の根拠(実測 16 サンプル)**: 詰まるのは**常に decoder**(graw-writer は 0 件)。
  debug 12 本 = 3 回 ×5 / 4 回 ×7、release 4 本 = 4 回 ×4。**最悪 4 試行 / 157 ms** に対し
  上限 6 試行 / 620 ms = **4.4 倍の余裕**。上限を残したのは恒久的な bind 失敗で run 開始が
  永久に返らないのを避けるため。

### 既存テストの変更(6 件 — すべて「仕様が変わったから」)

単体 3 件(ECC 呼び出し列の先頭に `reset` を追加 / `arm_failure…` の主張を
「ECC を**走らせる**段へ進まない」= `action != "reset"` に精密化)+ 統合 3 件
(ecc 列に reset 前置 / `a_failed_start_rolls_back…` は「run 番号が 1 個飛ぶ」コメントを削除し
**状態ファイルを直読みして `next_run` が戻っていること**の検証を追加 = 論点 C の出口 /
E2E-D から手動 workaround 撤去)。**発注側レビュー: すべて妥当**(順序の正本テストが順序変更に
追従するのは正しい。主張の中身は保存されている)。

### ⚠ Warsaw 必須の発見 — 実 ECC の `reset` は「1 段戻す」意味論(→ [036](036_ecc_walk_back.md))

**発注書と SPEC v1.12 初稿の前提「reset はどの状態からでも Idle へ行ける」は誤り**でした
(発注側が `ecc_core.hpp` のコメントを鵜呑みにしたのが原因 — **発注側の落ち度**)。
実 ECC ソースで**発注側も裏取り済み**:

- `GetBench/src/get/rc/BackEnd.cpp:924` — `reset()` は `engine.step(EV_UNDO)` のみ。
- 同 `:250-270` — `EV_UNDO` の遷移は **`Described→Idle`** と **`Prepared→Described`** の 2 本だけ。
  **`Active`(= Ready/Running/Paused)からの `EV_UNDO` は存在しない**(そこにあるのは
  `EV_BREAK`: `Active→Prepared`)。
- `StateMachine/src/dhsm/Engine.cpp:344` — 未定義遷移は例外も出さず **`Ignored`**(完全な無音)。
- `BackEnd.cpp:955-962` — `configure` は **`if (state == ST_PREPARED)` ガードで黙ってスキップ**。

→ `ecc stop` 後の `Ready` から reset を 1 回打っても**何も起きず**、describe / prepare も無音、
**configure がスキップされて CoBo がリンクを張り直さないまま `start` だけ成功**する。
`Ready → Idle` には **`breakup → reset → reset`** の歩き戻しが要る。
**実装は 036**(判断が要るので 034 は実装せず報告 = 掟どおりの正しい停止)。

034 が入れた**可視化だけは有効**: `reset` 直後の ECC 申告状態を `warn!` + audit
`ecc_state_after_reset` に必ず残す(実機で `"Ready"` が出たら即座に判る)。
なお `fake_ecc` / `ecc_core.hpp` は「どこからでも Idle」なので**テストでこの経路を踏めない** —
036 で遷移表を実 SM に合わせる。

### レビュー(発注側 Opus)

- 逸脱 7 件すべて受理。特に **reset を `Configure` より前**に置いた判断(前 run が Running のまま
  残った状況で、新しい listen が古い CoBo ストリームを拾わないようにする)は正しい。
  エラー文字列(`Address already in use`)での分岐を**意図的に避けた**のも妥当
  (文言が変わった日に黙って壊れる)。
- **発注側で再確認**: `BackEnd.cpp` / `Engine.cpp` の該当行を直接読み、reset 意味論の指摘が
  正しいことを確認。**SPEC v1.12 §1.3 を同日中に訂正済み**(誤った前提は正本から除去)。
- 積み残し(036 へ): `src/controller.rs` の `stop_run` 冒頭コメント「ecc stop(CoBo が送信を止め
  TCP が閉じる)」は 032 の事実により誤りだが、停止側は 032/033 の担当のため未修正。
**起票**: 2026-08-14([archive/030_p3_e2e.md](archive/030_p3_e2e.md) の跨 run 実測で判明。
発注側が `ecc_core.hpp` / `src/controller.rs` のソースで裏取り済み)
**仕様**: SPEC §1.3(run シーケンス)/ §8.1(controller REST)/ §8.2(ecc-bridge)/ §9.2
**Warsaw 必須度**: **高**(ビームタイムは run を連続で取る。1 本目しか綺麗に回らない状態で
現地に行かない)

## 事実(実測 + ソース確認)

### 1. `run/stop` の直後に `run/start` を打つと **ECC 順序違反で必ず失敗する**

- `ecc stop` 後の ECC は **`Ready`**。しかし controller の run 開始は必ず `describe` から始まり、
  `describe` が許されるのは **`Off` / `Idle` / `Described`** だけ
  (`tools/ecc_bridge/ecc_core.hpp` の遷移表。出典は実 ECC の
  `reference/20190315_patched/GetBench/src/get/rc/SM.cpp`)。
- 実測: `fake_ecc: invalid transition: describe in state Ready` →
  `run/start failed: ecc describe failed in state Ready`。
- 現状、連続 run には**オペレータが `POST /api/ecc/reset` を挟む**必要がある(030 のテストはそうした)。
  **実 ECC でも `daqStop` 後は `Ready` なので実機でも同じはず**。

### 2. `run/start` の Arm が `Address already in use` になり得る(実測 3 回に 2 回)

- decoder / graw-writer の PULL は固定ポート bind。`Reset` でソケットを落としても
  **libzmq の close は非同期**なので直後の再 bind が負ける。
- 実測: 1 本目は 1 回・0.010 s で成功、**2 本目は 3 回・0.118–0.121 s** かかって成功(3 実行とも)。

### 3. 失敗した `run/start` が **run 番号を空費する**

- `src/controller.rs`: `crate::state::take_next_run()`(採番 + 永続化)が
  `sequencer.start_run()` の**前**にある。よって 1 と 2 で失敗するたびに番号が 1 つ消える。
- 実測: 1 本目 = run 1、2 本目 = **run 4**(2 と 3 が消滅)。
- 番号が飛ぶこと自体は §12-11 が許容するが、「**連続 run のたびに 2 個捨て、オペレータには
  500 が 2 回返る**」という運用の見え方は仕様判断が要る。

## 裁定

### 論点 A = **ユーザー裁定(2026-08-14、変更不可)**: 毎 run 完全リセットして一からやり直す

> 「現在の運用は完全にリセットして、一からやり直すです。ワルシャワ大学の人たちはこれでやってるので
> それに合わせます。」(ユーザー)

- **採る形**: run 開始のたびに **ECC を Idle に戻してからフルシーケンス**
  (`reset` → `describe` → `prepare` → `configure` → `start`)。turn-around より
  **実運用の作法との一致**を優先する(オフライン互換と同じ思想 — 現地の手順を変えない)。
- **オペレータに `ecc/reset` を手で挟ませない**。controller の run 開始シーケンスが
  **自動で** reset を打つ(現状は 2 本目が必ず失敗する = 実質 1 本しか取れない)。
- 1 本目(ECC が `Off`/`Idle`)でも同じ経路を通ってよい(**分岐を作らない = KISS**)。
  reset はどの状態からでも Idle へ行ける(`ecc_core.hpp` / 実 ECC の SM.cpp)。
- **SPEC §1.3 の改訂が要る**(run 開始シーケンスの先頭に「ECC を Idle へ戻す」段を明記)。
  SPEC 本文の編集は**ユーザー承認事項**なので、本ユニットは**改訂案の文面を結果節に用意する**
  ところまで(docs/SPEC_ja.md は触らない)。

### 論点 B = **発注側裁定**: controller 側でリトライ(可視化必須)

- **(b1) を採る**: Arm の bind 失敗は controller が**指数バックオフでリトライ**(上限つき)。
  **リトライしたら回数と所要を必ず audit / ログに出す**(silent 禁止。実測は 3 回・0.12 s 級なので
  上限は 1 秒級で足りるはず — 実測して決めること)。
- (b2)(Reset 側で close 完了を待つ)は**併用してよい**が、libzmq の close 完了を外から
  観測する確実な手段が無いので**単独では採らない**。
- (b3)(PULL ポートの動的化)は SPEC §3.2 の固定ポート規約に触るので**採らない**。

### 論点 C = **発注側裁定**: 失敗したら採番を巻き戻す

- `take_next_run()` は現状のまま**先に採番**してよい(Configure に run 番号が要るので順序は動かせない)。
  **run 開始シーケンスが失敗したら `next_run` を元に戻す**(controller は単一書き手なので安全)。
- ただし**`run_start` をログブックに書いた後は巻き戻さない**(記録と番号の不整合を作らない)。
- 「番号は飛んでよい」(§12-11)は維持。**運用でふつうに使って飛ぶ**状態だけをなくす。

## やること

1. **controller の run 開始シーケンスに reset 段を入れる**(論点 A)。TDD。既存 016 テスト群を壊さない。
2. **Arm のリトライ + 可視化**(論点 B)、**失敗時の採番巻き戻し**(論点 C)。
3. **連続 run(2〜3 本 back-to-back)の回帰**を `tests/p3_e2e.rs` に追加
   (030 は跨 run の**スループット**を測ったが、**運用シーケンスとしての連続 run は未固定**)。
   E2E-D と同じ流儀・全ポート動的・env gate。**`ecc/reset` を手で挟まずに 2 本目が通ること**が出口。
4. **SPEC §1.3(必要なら §8.1)の改訂案の文面**を結果節に用意(**docs/ は触らない**)。
5. **UI(P4)への申し送り**: Run 制御の実配線時、オペレータに reset を要求しないこと
   (029 は完成形レイアウト + 全 disabled で待っている)。

## 備考

031(負荷ハーネス)は「中規模 run を back-to-back で 24 h 反復」方式なので、**本ユニットの
裁定が入らないと 24 h soak が回らない**(2 本目で必ず詰まる)。**031 の前提**として扱う。
