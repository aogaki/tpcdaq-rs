# 113 — run/start の冒頭で Idle でないコンポーネントを Reset する(停止時の Reset 不達で start が失敗し続ける穴)

**Status: COMPLETED**(2026-10-11 起票・同日完了。Opus 実装、Fable レビュー)

## 0. 事実(112 レーン A、`tests/controller_integration.rs`)
- 停止の畳む段で、あるコンポーネントへの `Reset` が ZMQ のタイムアウトで届かないと(再現 = エフェメラルポート枯渇、TIME_WAIT ≈ 15.6k)、
  `run/stop` は `ok:true` + `notes: ["receiver0: Reset unreachable: no reply ... within 5000 ms"]` で返り、相は Idle、**そのコンポーネントは Configured のまま**。
- 次の `run/start` は `Configure refused in state Configured` で 500。拒否されたコンポーネントは `touched=false` で rollback の対象外 → Configured のまま → **プロセス再起動まで start が失敗し続ける**。
  コンポーネントを Reset する REST は無い。
- Apply & restart(112)では「stop 成功 → start 500」として見える(silent ではない)が、復旧手段が無い。正常環境では 100 周で一度も起きていない。

## 1. 裁定(Fable、SPEC §1.3 への影響)
- **(ii) を採る**: `run/start` の冒頭で各コンポーネントの `GetStatus` を見て、**Idle でないものに `Reset` を打ってから Configure** する(ECC の `walk_ecc_back_to_idle` と同じ考え方。
  audit `run/start` に `component_walk_back: ["receiver0: Configured->Idle", …]` を残す)。`run/stop` の契約(`ok` = EOS と保存の完結、Reset 不達は `notes`)は変えない。
- SPEC §1.3 に 1 行追記(v1.37): 「start は ECC とコンポーネントの両方を Idle まで歩き戻してから始める」。

## 2. やること
1. `src/controller.rs` の start 経路(`walk_ecc_back_to_idle` の隣)に `walk_components_back_to_idle`。Reset が通らないコンポーネントがあれば start は 500 + 文言(どれが、何の状態で)。
2. テスト(TDD): 112 の fake ハーネスで「stop 後に receiver0 を Configured のまま残す(Reset をわざと落とす or 直接 Configure を打つ)→ start が成功し、audit に walk-back が残る」。
   既存 30 本 + 112 の 4 本を落とさない。
3. 併せて 112 レーン A の「読んで気づいた疑い」= `put_params` が Idle 確認後にロックなしでファイルを書く(別クライアントの start と競合し得る、`current.json` 非アトミック)
   → **`current.json` は一時ファイル + rename でアトミックに**、相の確認と書き込みを同じロック区間に(KISS で足りるなら)。設計が割れたら報告。
4. ゲート: `cargo fmt && cargo clippy --tests -- -D warnings && cargo test`。注意: 112 の新規テストは 1 回 ≈ 5,000 TIME_WAIT を生む(30 s 内に 3 回以上回すとポート枯渇)。

## 結果(2026-10-11)
- 実装(Opus `3a86037` + 追補 `444d80b` → merge **`c9b17cd`**): `src/controller.rs` に `walk_components_back_to_idle`(ECC 歩き戻しの後・Configure の前、上流から、
  Running は Stop → Reset、他は Reset、跡を `StartReport.component_walk_back` → audit)、失敗は stage `walk-back` で 500(文言に対象と状態)。
  `put_params` は `save_params` として相の確認・render・書き込みを 1 ロック区間に、`current.json` は `.tmp` + rename。
- 裁定: 判断点 1 = **Stop → Reset**(v1.6 の規則と同じ理由)、2(GetStatus 不達も walk-back で失敗)採用、3(render もロック内)採用。
  追補の逸脱 2 件(中間状態は実在する `Configured`、順序は上流から)採用。
- テスト: 単体 +6(controller 66 本)、統合 +2(32 本。「Reset を 1 回落とす → 次の start が通り audit に `receiver0: Configured->Idle`」「Reset を常に断る → 500 + 文言、Configure は誰にも出ない」)、
  フェイクに `Recorder::refuse(command, times)`、params の一時ファイルが残らない assert。既存 `a_failed_start_rolls_back…` は前提を「フェイクに Configure を断らせる」に変更(期待値は不変)。
  ゲート(main `c9b17cd`、Fable 再実行): clippy 警告 0、**cargo test 879 passed / 0 failed / 1 ignored**。
- SPEC **v1.38**: §1.3 と §3.3。
- 残課題: 歩き戻しの Stop の後の EOS 伝播を待たない(異常経路のみ。必要なら Fable キュー)。`late_fragments == 0` の初回ログ判定(108 の別件)も未。
- ELITPC 反映(13:55): main `990259b` を bundle → release build → run 56 停止 → daq_down / up → α run 57(25 MHz / trigDelay 1500 = ユーザーの current.json)。audit `run/start.component_walk_back = []`(全員 Idle)、ECC `status->Idle`。
