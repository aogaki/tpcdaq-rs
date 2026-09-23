# 092 — 自然 EOF 後の receiver: 「downstream is full — blocking on backpressure」→ teardown で 1 通 abandoned(069 #13 で観測)

**Status: READY**(2026-09-23 起票 Fable。要調査、Opus 0.5 日)

## 0. 事実(069 #13、run 27)
- 板の getHwServer 停止で receiver が `TCP EOF — end of stream` → `EndOfStream sent to both links`(10:28:59)。
- 2 s 後 `downstream is full or absent — blocking on backpressure (lossless; only Reset / teardown may …)`(10:29:01)。
- 68 s 後、controller の stop(ECC stop の 60 s timeout 後の Reset)で `send abandoned while stopping (Reset / component teardown) — this message is lost and counted`(10:30:07)。
- 疑問: EOS 送出後に receiver は何を送ろうとしたか(ハートビート? 空バッチ?)、なぜ下流が「full」になったか(graw-writer / decoder は EOS 受領後に PULL を止める?)。
  データは失われていない(EOS の前に全フレームは送出済み)はずだが、**silent 禁止の観点で「何が 1 通」かを特定して記録する**。

## 1. 発注
- `src/receiver.rs` の EOF 経路を読み、EOS 送出後にループが送るメッセージ(Heartbeat?)と、下流 PULL 側(graw_writer / decoder)が EOS 後に recv を続けるかを確認。
- 再現テスト: 既存の receiver 統合テスト(EOF → EOS)に「EOS 後 3 s 待ってから Reset」を足し、abandoned カウンタが 0 であるべきか 1 であるべきかを仕様として決めて固定
  (仕様案: **EOS 後はハートビートも送らない**(下流は EOS で run を閉じる)= abandoned 0 が正)。
- SPEC §1.3 / §2.2 に「EOS 後の receiver は静止」を 1 行(Fable)。

## 2. 参照
069 #13 結果、032(silent stall)、SPEC §1.3(停止シーケンス)、§2.2(EOS はバリア)。
