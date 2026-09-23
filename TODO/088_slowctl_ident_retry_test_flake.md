# 088 — `slowctl_integration::records_from_before_the_controller_was_up_arrive_in_order_starting_with_ident` のタイミング依存(まれに赤)

**Status: READY**(2026-09-23 起票 Fable。079 Phase 3 レーンの報告: 全体 `cargo test` の負荷下で 1 回だけ FAILED、単体では 3 回連続 green。Sonnet 0.5 日)

## 0. 事実
- テストは「PULL を bind せずに slowctl を起動 → 1 s 待つ(poll_hz 20 = 20 tick、スクリプト 8 行は 0.4 s で流れ切る)→ PULL を bind → 受けた順が `[IDENT, ALARM, RECOVER]`」。
  負荷下では 1 s 以内にスクリプトが流れ切らない、または bind 後の受信待ちが短い、のどちらかで順序/件数の期待が崩れる(壁時計に依存)。
- CLAUDE.md: テストは決定的であること。壁時計の固定待ちを条件待ちに置き換える。

## 1. 発注
- `tests/slowctl_integration.rs` の当該テスト(と同じ harness を使う他のテスト)で、固定 `sleep` を**条件待ち**に置き換える: slowctl の status PUB(SUB)で `counters.poll_ok` がスクリプト行数以上になるまで待つ(上限 10 s)→ それから PULL を bind → 受信は「期待の 3 通が揃うか上限 10 s」まで読む。
  期待順序 `[IDENT, ALARM, RECOVER]` と `log_dropped` の検査は変えない。
- 同じファイルの他テスト(4 項目)にも固定 sleep があれば同じ流儀に。`cargo test --test slowctl_integration` を 20 回連続(`for i in $(seq 20)`)、全体 `cargo test` を 3 回連続で green を確認して結果節に回数を書く。
- production コードは触らない(テストの待ち方だけ)。どうしても production 側の観測点(例: 再送キューの長さを status に出す)が要るなら実装せず報告。
