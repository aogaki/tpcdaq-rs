# 113 — run/start の冒頭で Idle でないコンポーネントを Reset する(停止時の Reset 不達で start が失敗し続ける穴)

**Status: READY**(2026-10-11 起票 Fable。112 レーン A の実測で発見。Opus 向け)

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
