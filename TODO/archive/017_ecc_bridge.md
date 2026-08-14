# 017 — ecc-bridge(C++/Ice)+ fake-ECC

**Status: COMPLETED**
**仕様**: SPEC v1.6 §8.2(本体)、§8.3(検証)、§3.2(REP = `tcp://*:47200`)、§1.3(シーケンス
中の位置)。CLAUDE.md「制御プレーンを改変しない — Ice **クライアント**として話すだけ」
**流用元**(ユーザー自身のコード、読み取り + 移植可、出自コメント明記):
`~/test/get/tpcdaq/src/control/ecc_controller.cpp`(Ice pimpl 済み EccController)と同リポの
ビルド設定(CMakeLists の Ice リンク方法)・fake-ECC テストハーネス(同リポ内を探すこと)。
`.ice` 定義の正: `reference/20190315_patched/`(実験使用中と同一版)
**発注先想定**: implementer/Opus(言語横断 + Ice の罠)

## やること

1. `tools/ecc_bridge/` 新設(C++17、Ice + libzmq。ROOT 不要):
   - `ecc_bridge` — ZMQ **REP bind**(既定 `tcp://*:47200`)の薄い JSON サーバ。
     リクエスト形式は SPEC §8.2 例のとおり:
     `{"action": "describe|prepare|configure|start|stop|breakup|reset|status", ...}` →
     `{"ok": bool, "state": "Off|Idle|Described|Prepared|Ready|Running|Paused|Unknown",
     "error": "..."}`。
   - 中身は C++ 版 `EccController` の流用(コピーして `third_party` ではなく tools/ecc_bridge/ 内
     — ユーザー自身のコードなのでライセンス隔離不要。出自コメントのみ)。
   - **実機の罠を仕様として固定(§8.2)**: DataLinkSet XML は links 配列から生成、
     DataSender id は `CoBo[k]` 形式・flowType は**大文字 `TCP`**・**Ice encoding 1.1 を明示強制**
     (インストール済み Ice が 3.7 系でも 1.1 で話す — C++ 版の設定を踏襲)。
   - 例外は全部 Result 化(**never throw**)。ECC 不達 = `{"ok": false, "state": "Unknown"}`。
     ZMQ REP は 1 スレッド逐次で十分(コマンドは人間スケール)。
2. **fake-ECC**(`tools/ecc_bridge/fake_ecc`): 実 `.ice` 定義(reference/)の servant を最小実装:
   - 状態機械 Off→Described→Prepared→Ready→Running を正しく遷移、順序違反はエラー。
   - **`start` 時に DataLinkSet の向き先へ実 TCP connect を試み、繋がらなければ
     "Could not establish data link" エラー**(listen-before-start の負性テストの実体 — §8.3)。
     繋がったら 1 バイトも送らず保持(データ送出はしない — replay の仕事)。
   - C++ 版のテストハーネスに相当物があれば流用、なければ新規。
3. ビルド: `tools/ecc_bridge/Makefile`(Ice は Homebrew: slice2cpp で reference/ の .ice から
   スタブ生成 — **生成物はコミットしない**(.gitignore)。.ice 自体も reference/ のままコピー禁止、
   ビルド時参照のみ。参照パスは環境変数 `TPCDAQ_ICE_DIR` で与え、未設定ならビルドを明示 skip)。

## テスト

- 単体(`test_ecc_bridge.cpp`、assert + main、Ice 不要部分): **DataLinkSet XML 生成の文字列照合**
  (CoBo[0] / 大文字 TCP / 2 CoBo で DataLink 2 本)、JSON リクエスト parse / レスポンス生成、
  状態文字列マップ。
- 統合(`run_ecc_e2e.sh`): fake-ECC 起動 → ecc_bridge 起動 → ZMQ REQ で
  describe→prepare→configure→start(listen なし)= **"Could not establish data link" エラー** →
  listen を用意して start = 成功 → stop → 全状態遷移を機械照合。
- Rust 側 env-gated テスト 1 本(`tests/ecc_bridge_intake.rs`、env `TPCDAQ_ECC_BRIDGE_BIN` +
  `TPCDAQ_FAKE_ECC_BIN` 未設定なら skip): controller 視点の JSON REQ/REP を Rust から実施
  (016 の接続先の契約を Rust 側テストとして固定)。

## 受け入れ

- 単体 + 統合 + Rust env-gated すべて green。listen-before-start の負性テスト green。
- `cargo fmt/clippy/test` 無影響(追加は env-gated テスト 1 本のみ)。
- ファイル所有権: tools/ecc_bridge/(新規)+ tests/ecc_bridge_intake.rs(新規)。
  **src/*.rs・Cargo.toml・tools/root_sink/ に触らない**(並列で 015 が src/ を作業中)。
- Ice 環境(バージョン・リンク方法・encoding 1.1 の強制方法)を「## 結果」に記録
  (P5 の実 ECC コンテナ検証の材料)。


## 結果

実行環境: macOS(Darwin 25.5.0)、Ice 3.8.2(Homebrew /opt/homebrew/opt/ice)、
libzmq 4.3.5、2026-08-14。
実装: implementer/Opus(再発注 — 初回 08-13 発注はエージェント停止・作業ゼロ)。レビュー: Fable。

### テスト結果(レビュー時に Fable が再実行)

- `make test`(Ice 非依存単体): **136 passed / 0 failed**
  (DataLinkSet XML 全文照合: `<DataSender id="CoBo[0]"/>`・大文字 TCP・2 CoBo = DataLink 2 本 /
  JSON parse・レスポンス / 状態文字列 / fake 遷移表 / json_min)
- `./run_ecc_e2e.sh`(fake-ECC + ecc_bridge + ZMQ REQ 実配線): **27 passed / 0 failed、exit 0**
  - **listen-before-start 負性テストは実機と同一文言で green**:
    `Could not establish data link. connect to ... failed: Connection refused`
    (文言の出典 = GetBench DaqCtrlNodeI.cpp:399)
- Rust env-gated `tests/ecc_bridge_intake.rs`: env 有 **2 passed** / env 無 skip 経路 green
- リポ全体 cargo fmt / clippy -D warnings / cargo test(29 バイナリ)全 green

### Ice 環境の記録(P5 実 ECC 検証の材料)

- Ice **3.8.2** + slice2cpp 3.8.2。スタブは reference/20190315_patched の .ice から
  ビルド時生成(9 unit、生成物はコミットしない)。同 .ice は ~/test/get の C++ 版と
  **バイト一致**を確認 = 実験使用中と同一定義。
- **encoding 1.1**: プロキシに `.ice_encodingVersion(Ice::Encoding_1_1)`(流用元踏襲)+
  接続時に実測値を stderr へ出す(主張でなく実測: `connected to Ecc:tcp ... (encoding 1.1)`)。
- Ice 3.8 は `Ice.Override.ConnectTimeout` 廃止(Client.ConnectTimeout 既定 10 s)。

### 逸脱・設計判断(レビューで受理)

1. EccController は ecc_bridge.cpp に取り込み(pimpl 廃止 — 隠す相手がいない。出自明記)。
2. `ecc_e2e_client.cpp` 追加(nc 依存を避け、機械照合を C++ 側に寄せた)。
3. **小文字 `tcp` は明示エラー**(黙って大文字化して罠を隠さない)。
4. `status` の ok = 状態が取れたか(不達 = `{"ok":false,"state":"Unknown"}` — §8.2 どおり)。
5. sender の形式検証はしない(`CoBo[Crate00_Slot00]` があり得るため。罠は e2e 照合で担保)。
6. pause/resume は bridge の action に含めない(§8.2 の一覧どおり。fake-ECC 側は .ice の
   実装義務があるため実装)。
7. fake-ECC の data link connect は 2 s 非ブロッキング(制御プレーンを固めない)。
