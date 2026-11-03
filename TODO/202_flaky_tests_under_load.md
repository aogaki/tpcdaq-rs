# 202 — 負荷のかかった全体実行でだけ落ちるテスト(3 件)を直す

**Status: TODO**(2026-11-03 起票 Opus。11/9 の後でよい — どれも本番の挙動の欠陥ではなく、テストの待ち方の問題と読む)

## 動機
184–186 のマージ後(2026-11-03)、`cargo test` の全体実行(並列、CPU が混む)で、毎回ではなく 1 件ずつ落ちた。単独では通る。揺らぐテストは放置すると本物の失敗を見逃す原因になる。

## 対象
1. `tests/receiver_stale_link.rs::eof_after_data_still_ends_the_run`(470 行目、フレームの中身の比較)— 3 回の全体実行のうち 1 回。単独 10 / 10 通る。**読み**: 184 で decoder のリンクの送信を DONTWAIT(送れなければ数えて落とす)にしたため、テストの PULL がまだ接続しきっていない最初のフレームを、仕様どおり落としている。本番では decoder は run のずっと前から接続済み。→ テスト側で接続を確かめてから送る(または decoder リンクについては `decoder_link_dropped_frames` との和で照合する)。
2. `tests/root_sink_monitor_pub.rs::monitor_root_is_written_within_ten_seconds_of_the_eos`(884 行目)— 3 回のうち 1 回。10 s の期限が負荷で足りない。→ 期限の意味(SPEC の要求か、テストの都合か)を確かめて、要求でなければ余裕を持たせる。
3. `graw_replay_tool::connect_failure_exits_non_zero_with_clear_message`(184 の implementer が報告、ポートの TOCTOU)。

## 受け入れ
- 全体の `cargo test` を 10 回続けて回して 0 failed。各テストの直し方の理由を記録。
