# 016 — controller(Rust、REST + run シーケンス + ログブック統合)

**Status: OPEN(015 完了後に発注 — logbook/state API に依存)**
**仕様**: SPEC v1.6 §8.1(REST = 本ユニットの核)、§1.3(run 開始/停止/**中止(v1.6)**
シーケンス)、§2.6(コマンドチャネル)、§9(logbook は 015 のモジュールを使う)、
§3.1/§3.2(ログ投稿 PULL 47005、REST 8080)
**依存**: 015(logbook/state)、017(ecc-bridge — JSON REQ/REP 契約。E2E は fake-ECC)
**発注先想定**: implementer/Opus(シーケンス設計・非同期の判断が残る)

## やること

1. `src/controller.rs` + `src/bin/controller.rs`:
   - **REST(axum — Cargo.toml へ追加可)**、§8.1 のエンドポイントを全部:
     `GET /api/status` / `POST /api/control/acquire|release` / `POST /api/run/start|stop` /
     `POST /api/ecc/{describe|prepare|configure|start|stop|breakup|reset}` /
     `GET /api/logbook?since_seq=N` / `POST /api/logbook/comment`。
   - **操作権**: token 1 つ、acquire は常に横取り可(C++ 版 CommandRouter 方式)、横取り・
     全状態変更は audit レコード(015 経由)。閲覧系は認証なし(二層 — §8.1)。
     passphrase は config `[controller]` から。
   - **コンポーネントクライアント**: §2.6 JSON REQ/REP(タイムアウト付き、config のエンドポイント
     表から。src/command.rs にクライアント側が無ければ追加 — サーバ側実装・既存テスト無改変)。
   - **run TS の一元化(SPEC §6.4 v1.8 実装注記)**: Start コマンドに正式な run 開始 TS を
     載せ、root-sink の runId(PEventTPC EventInfo)と graw-writer の TS 生成を将来これに
     揃える(現状は各コンポーネントのローカル時刻で数秒ずれ得る — 対応の正はログブック)。
   - **run 開始シーケンス(§1.3 を忠実に)**: 015 `take_next_run` → Configure(下流から:
     graw-writer → decoder → receivers ※monitor は P3 後波)→ Arm(**receiver の実 bind ポートを
     応答から回収**)→ Start{run} → ecc-bridge 経由 describe→prepare→configure(DataLinkSet =
     回収したポート)→start → run_start レコード。**どの段階の失敗も巻き戻し**(以降を実行せず、
     実行済みコンポーネントを Stop、audit に失敗記録 — 半端な Running を残さない)。
   - **run 停止シーケンス**: ecc stop → EOS 伝播待ち(root-sink の run クローズを graw-writer/
     decoder の GetStatus と時間で監視、**EOF 5 秒不達なら receiver へ Stop = 強制 EOS**)→
     コンポーネント Stop(上流から)→ **run_stop レコード(§9.2 の counters / files を各
     GetStatus 実測から充填**。root-sink は REP を持たないため counters は decoder/graw-writer の
     metrics + root-sink 終了 JSON の将来統合を見据えた「取れる分だけ、無い項目は null」)。
   - **中止(abort)**: SPEC §1.3 v1.6 の正規経路をそのまま実装(EOS で閉じてから Stop/Reset。
     run クローズ前に decoder を Reset しない。run_stop は ok=false, reason="abort:...")。
   - **ログ投稿 PULL bind**(既定 `tcp://*:47005`、§2.3 LogPost): 受けた JSON 文字列に
     controller が ts/seq を付与して logbook へ(015 の writer は controller が単独所有)。
   - **静的ファイル配信**: config の `ui_dir`(既定なし = 無効)を `/` に配信(UI は後波。
     axum の ServeDir で枠だけ)。
   - 状態: controller 自身は §1.3 の状態機械の**外**(オーケストレータ)。`GET /api/status` は
     各コンポーネント GetStatus + ecc status + 自身の run 状態(Idle/Starting/Running/Stopping)を
     集約。
2. config: `[controller]` に `rest_listen`(既定 0.0.0.0:8080)/ `passphrase` /
   `log_pull_bind`(47005)/ `eos_timeout_s`(既定 5)/ `ui_dir`(任意)。§3.1 の既存例と整合。
   コンポーネントのエンドポイント表(REP 47100/47101/47110+k、ecc 47200)は §3.2 既定 +
   設定上書き。

## テスト

- 単体: シーケンサを純ロジック化(コマンド送信を trait で抽象化)し、**開始の巻き戻し**
  (Arm で 1 個失敗 → 実行済みだけ Stop される)/ 停止の強制 EOS 分岐 / 中止経路
  (run クローズ前に decoder Reset が**発行されない**こと)を mock で機械照合。
- 統合(実 ZMQ、port 0): fake コンポーネント = 既存 `command::run_command_task` を
  テスト内で複数起動 → 実 REST(axum test / reqwest)で acquire → run/start → 各 fake の受信
  コマンド列が §1.3 の順序どおりであること → run/stop → logbook に run_start/run_stop/audit が
  正しい順・正しい中身で並ぶこと。token 無し = 401 相当 / 横取り = audit。
  LogPost PULL 経由の投稿が seq 付きで記録されること。
- ecc-bridge との実配線(env `TPCDAQ_ECC_BRIDGE_BIN` + `TPCDAQ_FAKE_ECC_BIN` 時):
  describe→…→start が bridge 経由で通ること(全通し E2E = §12-7 は後波の P3 E2E ユニット)。

## 受け入れ

- 上記全テスト green。`cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test`
  通過(既存無影響)。
- ファイル所有権: src/controller.rs・src/bin/controller.rs・src/config.rs([controller] 追記)・
  src/command.rs(クライアント関数の追加のみ・既存無改変)・src/lib.rs(1 行)・
  Cargo.toml(axum 系追加)・tests/controller_integration.rs・tests/controller_ecc.rs。
