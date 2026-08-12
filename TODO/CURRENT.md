# CURRENT — tpcdaq-rs 現在地

**最終更新: 2026-08-12(P2 着手 — 保存系: graw-writer + root-sink 接続)**

## いま

- **P0・P1 完了**。リポ全体 **153 テスト green**、clippy `-D warnings` クリーン。
  - P0 出口: 実 ELITPC .dat が読めて ch→(plane,section,strip) が引ける(オラクル一致)。
  - P1 出口: デコーダが実 .graw オラクル完全一致(events=108 / items=15,040,512 / malformed=0)。
    receiver は実 .graw 全速リプレイ(≈188 MB/s = 100 Hz 相当の 6.7 倍)で**バイト完全一致 + drop 0**。
- 実装済みモジュール: config(TOML)/ geometry(.dat、FPN、ChannelRole)/ msg(ZMQ ワイヤ + 漂流ガード)/
  command(状態機械 + REP タスク、tracing 化済み)/ zmq_helper(有限 HWM)/ framer / decode /
  receiver(never-stop)+ bin: receiver, graw_replay。
- **公開済み**: https://github.com/aogaki/tpcdaq-rs (public、2026-08-12 初回 push)。
  公開ルール: 実 .graw / FW / 実ジオメトリ .dat / マニュアル PDF は `reference/` に置き .gitignore 済み
  (コミット前に混入チェックを実施した — 追跡対象はコード・文書・合成フィクスチャのみ)。
- 実装の正本 = docs/SPEC_ja.md v1.0。モデル使い分け運用 = CLAUDE.md。
- serena: reference/ 索引済み(次回 activate から有効)。

## アクティブ(P2 — 出口: graw バイト一致 + TTree 互換 + run 毎単一 ROOT + ×2 リプレイ 2 ソースビルド一致)

**波 1(起票済み・未実装 — 2026-08-12 に実装エージェントを途中停止し、書きかけコードは破棄済み。
発注書は完成品なので次セッションは implementer に再発注するだけ)**:
- [007_graw_writer.md](007_graw_writer.md) — graw-writer(CoBo 毎ファイル、バイト一致)→ implementer/Sonnet に発注
- [008_root_sink_intake.md](008_root_sink_intake.md) — root-sink 取り込み骨格(C++、ロスレス PULL、ROOT 非依存)→ implementer/Opus に発注
- 007 と 008 はファイル所有権が交差しないので並列可(007 = src/graw_writer.rs + config [graw_writer] + lib.rs 1 行、008 = tools/root_sink/ + examples/ + env-gated Rust テスト 1 本)

**波 2 以降(順次起票)**:
- 009 decoder コンポーネント(Rust。EOS 集約 = SPEC §2.3「decoder のソース性」2026-08-12 追記)
- 010 eventIdx ビルダ(C++ 純ヘッダ、SPEC §6.3)
- 011 GDataFrame TTree + third_party/get 隔離(C++/ROOT 6.36.10 @ /opt/ROOT 確認済み)
- 012 P2 E2E(graw_replay ×2 → 2 receiver → decoder → root-sink、TTree 互換比較 §12-3)

## 継続事項

- 006 レビュー指摘の再訪: 下流全死 + EOF 前 Reset での EOS 再試行が畳めない件(shutdown 経路の上限)→ 007/009 の停止設計で考慮。
- Warsaw 確認事項: 2-CoBo ジオメトリ .dat の有無(SPEC §13-7)、PROPOSAL v0.5 反映判断。

## 最近完了

- 2026-08-12: [006_receiver.md](archive/006_receiver.md) — receiver(16 tests。**P1 出口達成**: 実 .graw 全速リプレイ byte 一致 + overflow 0。過負荷でも drain 継続を実証。tracing 導入)
- 2026-08-12: [005_graw_replay.md](archive/005_graw_replay.md) — graw_replay(16 tests。--rate-mbps = Mbit/s 確定 → SPEC §12)
- 2026-08-12: [004_framer_decoder.md](archive/004_framer_decoder.md) — framer + デコーダ(23 tests。**実 .graw オラクル完全一致**)
- 2026-08-12: [003_zmq_core.md](archive/003_zmq_core.md) — ZMQ メッセージ核 + 状態機械(48 tests)
- 2026-08-12: [002_geometry.md](archive/002_geometry.md) — ジオメトリ抽象(35 tests。実 .dat オラクル一致 = P0 出口)
- 2026-08-12: [001_scaffold_config.md](archive/001_scaffold_config.md) — scaffold + TOML 設定(15 tests)
- 2026-08-12: [000_spec.md](archive/000_spec.md) — 仕様書 docs/SPEC_ja.md v1.0
