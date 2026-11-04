# 208 — controller の root_sink status 購読(204)の小さな残り 2 件

**Status: TODO**(2026-11-04 起票 Opus。出どころ = 204 の Fable フェーズ境界レビュー。どちらも頻度・影響が小さいので 11/9 の後でよい)

## 1. run 番号の再利用で前回の close を取り違える
- `POST /api/run/next` で番号を戻した / start 失敗の巻き戻しで同じ番号 N を再利用し、その run が **0 事象**で止まると、root_sink の status は前回の `idle run N` のまま → controller が「run N を閉じた」と採り、run_stop の `events_built` / `events_incomplete` / `late_fragments` が**前回の値で非 null** になる(204 の C の誤記録)。
- 直し方の案(Fable): `Sequencer::with_run(run, started: Instant)` にして、`received_at < started` の status は close に採らない(MockTransport の台本は `Instant::now()` 基準なので既存テストは無改変)。テスト: 同じ番号で 2 回目の 0 事象 run → 3 キーは null。

## 2. hist_snapshot の全復号
- controller の SUB スレッド(`src/controller/root_sink_status.rs` の `ingest`)は hist_snapshot(W 面の 2D ≈ 1 MB)も `decode_message` で全部復号してから捨てる → 1 Hz × ≈ 1 MB の msgpack 復号が常駐。topic フレームが無いので SUB のフィルタは使えない。
- 直し方の案: 復号の前に先頭の `kind` だけ覗く、または monitor 側の decode に「status 以外は skip」の口を足す。まず ELITPC で controller の CPU を 1 度測って、要るかを決める。

## モデル
Sonnet(発注書で縛れる)。
