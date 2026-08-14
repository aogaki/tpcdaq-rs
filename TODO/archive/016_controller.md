# 016 — controller(Rust、REST + run シーケンス + ログブック統合)

**Status: COMPLETED**(2026-08-14 implementer/Opus worktree 実装 → Fable レビュー PASS →
main へ取り込み済み)

## 結果

- **実装**: src/controller.rs(新規 2760 行 = コード 1536 / テスト 22 本)/
  src/bin/controller.rs(116)/ tests/controller_integration.rs(972、実 ZMQ + 実 REST
  7 本)/ tests/controller_ecc.rs(474、env gate 2 本)/ config.rs +296([controller]
  8 キー + 検証 + テスト 6)/ command.rs +163(クライアント `request()` **追加のみ**)/
  lib.rs +3 / Cargo.toml(axum 0.8.9 / tower-http 0.6.11 / sha2 0.10.9)。
  既存テストの改変ゼロ(既存行削除は型注釈 1 行のみ)。
- **テスト(worktree でエージェント実行 + Fable がゲート再実行で裏取り、2026-08-14、
  macOS Darwin 25.5.0)**: `cargo fmt --check` / `clippy --all-targets -- -D warnings`
  クリーン、`cargo test` **310 passed / 0 failed**(新規 41)。Fable 再実行 exit 0、
  `--test controller_integration` 単独 7/7 実測。ecc 実配線
  (`TPCDAQ_ECC_BRIDGE_BIN` + `TPCDAQ_FAKE_ECC_BIN`)2 本 green — describe→prepare→
  configure→start(Running)→stop(Ready)+ listen-before-start 負性(実機文言
  "Could not establish data link")。worktree 内で tools/ecc_bridge を make ビルド
  (.ice は本体 reference/ 参照)。取り込み後の main で全体ゲート再実行(本文書
  アーカイブ時点の CURRENT.md 記載値)。
- **スキップ**: main 取り込み後の ecc-gated 2 本は tools/ecc_bridge のビルド有無に依存
  (env 未設定なら明示 skip — 実装確認済み)。
- **レビュー詳細と逸脱 9 件の裁定は下のレビュー節**(受理 6 / ユーザー判断待ち 2
  (counters 0 埋め vs Option 化・comment API の token 要否)/ フォローアップ 2
  (Geometry::asad_counts() / run 番号手動設定 REST))。

## レビュー(Fable、2026-08-14)

- **ゲート再検証**: worktree で fmt/clippy/`cargo test` を Fable が再実行 → exit 0
  (310 passed、新規 41)。`cargo test --test controller_integration` 単独でも 7/7 実測。
  ecc 実配線 2 本はエージェント実行ログで green を確認(describe→…→start→stop +
  listen-before-start 実機文言)。
- **設計確認**: start_run の順序(Configure 下流から → Arm ポート回収 → Start → ecc 4 段)、
  巻き戻し(touched のみ逆順 Stop+Reset)、停止(ecc stop → EOS 待ち → 強制 EOS 二段 →
  **collect_status を Reset 前に取る** → 上流から畳む)、v1.6(EOS 観測後にのみ decoder
  Reset — 機械照合テストあり)を現物で確認。production `.unwrap()` ゼロ。
  fake が本物の状態機械で不許可遷移を断る試験設計は §1.3 適合の実質検証になっており良い。
- **逸脱 9 件の裁定**: 受理 = ①run TS を Configure.config で配布(既存テスト無改変制約の
  帰結。全コンポーネント同一値をテストで照合済み)/ ②Stop 後 Reset(発注書の穴 —
  §1.3 で Configure は Idle からのみ。v1.6 不変条件は保持)/ ④sha2 追加(§9.2 必須)/
  ⑤abort 起動口は /api/run/stop の自動判定 / ⑦router_ip 新設(不定アドレス warn)/
  ⑨キー命名。**ユーザー判断待ち** = ③counters の取得不能項目が 0 埋め(§9.2 の u64 制約。
  0 と「不明」の混同はログブックの記録品質に関わる — Counters を Option 化する SPEC 改訂を
  提案)/ ⑧/api/logbook/comment の token 不要(シフト全員が書ける vs §8.1「状態変更系は
  token 必須」の字面)。**フォローアップ起票候補** = ⑥`Geometry::asad_counts()` アクセサ
  (dump_tsv パース経由の暫定を置換)/ §8.1「run 番号手動設定 REST」(発注書の列挙漏れ =
  チケット不備。API 形は Fable が決めて小ユニット)。
- **注記**: eos-timeout 後の畳みで decoder Reset が走る経路は、root-sink 生存時に
  §6.2-5 の正しい fatal を誘発し得る(reason="error:eos-timeout" で可視)。既に壊れた
  状態の後始末であり受理。P3 E2E の異常系シナリオに 1 本入れる。
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
