# 017 — ecc-bridge(C++/Ice)+ fake-ECC

**Status: OPEN**
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
