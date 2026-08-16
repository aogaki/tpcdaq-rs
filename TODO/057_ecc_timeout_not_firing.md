# 057 — controller の ECC REQ タイムアウトが発火しない疑い(056 実測起因)

**Status: OPEN(調査ユニット — 原因が局所化していないので Fable/主対話が一次対応)**
**証拠**: 056 受け入れ実測(2026-08-16)の初回 elitpc run

## 観測事実

- 実 ECC の `configure` が初回のみ **261 s** かかった(2 回目以降は 8.3 s。ECC 側の
  初回スロー自体も原因未特定 — 4 AsAd xcfg の初回パース等の仮説はあるが未検証)。
- その間、controller の `DEFAULT_ECC_TIMEOUT = 60 s`(src/controller.rs:85 —
  ecc-bridge への REQ の rcvtimeo)が**発火せず**、controller は 261 s 待って完走した。
- クライアント(curl `--max-time 180`)は先に切れて空応答になったが、controller の
  run/start 自体は成功として完了。

## 調べること

1. rcvtimeo が実際にどのソケット・どの await 経路に効いているか(REQ の送受どちらか /
   tokio 側の待ちに吸われていないか)。60 s で切れるべきだったのか、仕様として
   「configure は長くてよい」のか(SPEC §8 の応答時間の約束を確認)。
2. タイムアウトすべきなら: 発火しない原因の特定 + 修正(TDD — 遅い fake ECC で red)。
   しないなら: SPEC に「ECC 段階操作の待ちは無制限(クライアント側でタイムアウトする)」
   を明文化し、UI の待ち表示(spinner)が長時間ケースで正しいか確認。
3. HTTP 層: クライアントが切れた後の run/start 完走はそれ自体は正しい(操作は冪等でない
   ので途中で殺す方が危険)が、**結果をクライアントが受け取れていない**。UI 側の
   リカバリ(status ポーリングで Running を拾う)が効くことを確認。

## 非スコープ

- ECC 本体の改変(初回 261 s の内因調査はログ観察まで)。
