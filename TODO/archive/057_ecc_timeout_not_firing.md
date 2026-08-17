# 057 — controller の ECC REQ タイムアウトが発火しない疑い(056 実測起因)

**Status: COMPLETED**(2026-08-17 — 結果は末尾。裁定込み)
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

---

## 結果(2026-08-17 — implementer/Opus 調査、発注側(Fable)裁定)

- **結論: タイムアウトは壊れていない**。rcvtimeo 60 s は `ZmqTransport::ecc`
  (controller.rs:349-385)の recv 側に正しく効き、90 s 固まる相手に **60.00 s
  ちょうどで Err** することを実測。tokio に吸われてもいない(spawn_blocking 内の同期 zmq)。
- **261 s の正体(最有力)**: run/start は最大 9 本の ECC REQ(歩き戻し + 4 相)を
  順に撃ち、**シーケンス全体には期限が無い**(最悪 540 s)。個々の 60 s は全部生きて
  いた。curl --max-time 180 が先に切れ run は完走 — 観測と完全整合。ecc_bridge/
  ecc_server ログは無タイムスタンプで一次資料は消失(コールドスタート再現は ELI-NP
  観察項目へ — elapsed_ms ログ追加により次回は即断できる)。
- **裁定**: A) 全体期限は**設けない**(非冪等な歩きを途中放棄する方が危険。SPEC v1.20
  §8.2 に明文化)。B) `ecc command applied` に **elapsed_ms を追加**(発注側が実装 —
  1 行 + fmt/clippy/対象テスト green)。D) UI 回復は status ポーリングで既に成立。
- **テスト**: tests/controller_ecc_timeout.rs 新規 3 本(タイムアウト発火 / REST 通しで
  HTTP 500 + audit ok=false — 従来 1 本も無かった経路)。cargo **453 passed / 0 failed /
  1 ignored**(基準 450 + 3)。
- 実行環境: macOS Darwin 25.5.0、2026-08-17。

**Status: COMPLETED**
