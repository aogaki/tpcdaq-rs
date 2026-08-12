# CURRENT — tpcdaq-rs 現在地

**最終更新: 2026-08-12(P1 完了 — ここで一旦区切り〔ユーザー指示〕)**

## いま

- **P0・P1 完了**。リポ全体 **153 テスト green**、clippy `-D warnings` クリーン。
  - P0 出口: 実 ELITPC .dat が読めて ch→(plane,section,strip) が引ける(オラクル一致)。
  - P1 出口: デコーダが実 .graw オラクル完全一致(events=108 / items=15,040,512 / malformed=0)。
    receiver は実 .graw 全速リプレイ(≈188 MB/s = 100 Hz 相当の 6.7 倍)で**バイト完全一致 + drop 0**。
- 実装済みモジュール: config(TOML)/ geometry(.dat、FPN、ChannelRole)/ msg(ZMQ ワイヤ + 漂流ガード)/
  command(状態機械 + REP タスク、tracing 化済み)/ zmq_helper(有限 HWM)/ framer / decode /
  receiver(never-stop)+ bin: receiver, graw_replay。
- **git 未コミット**(リポ創設からまだ一度もコミットしていない — 区切りにあたり初回コミットを推奨)。
- 実装の正本 = docs/SPEC_ja.md v1.0。モデル使い分け運用 = CLAUDE.md。
- serena: reference/ 索引済み(次回 activate から有効)。

## アクティブ

- (なし — 区切り中)

## 次(再開時: P2 の詳細起票から)

- P2 = graw-writer(CoBo 毎ファイル、バイト一致)+ root-sink 接続(eventIdx ビルダ + GDataFrame TTree)。
  SPEC §6/§7。root-sink の C++ 手術は SPEC §6.1 の表が発注書の種。
- 006 レビュー指摘の再訪: 下流全死 + EOF 前 Reset での EOS 再試行が畳めない件(shutdown 経路の上限)。
- Warsaw 確認事項(継続): 2-CoBo ジオメトリ .dat の有無(SPEC §13-7)、PROPOSAL v0.5 反映判断。

## 最近完了

- 2026-08-12: [006_receiver.md](archive/006_receiver.md) — receiver(16 tests。**P1 出口達成**: 実 .graw 全速リプレイ byte 一致 + overflow 0。過負荷でも drain 継続を実証。tracing 導入)
- 2026-08-12: [005_graw_replay.md](archive/005_graw_replay.md) — graw_replay(16 tests。--rate-mbps = Mbit/s 確定 → SPEC §12)
- 2026-08-12: [004_framer_decoder.md](archive/004_framer_decoder.md) — framer + デコーダ(23 tests。**実 .graw オラクル完全一致**)
- 2026-08-12: [003_zmq_core.md](archive/003_zmq_core.md) — ZMQ メッセージ核 + 状態機械(48 tests)
- 2026-08-12: [002_geometry.md](archive/002_geometry.md) — ジオメトリ抽象(35 tests。実 .dat オラクル一致 = P0 出口)
- 2026-08-12: [001_scaffold_config.md](archive/001_scaffold_config.md) — scaffold + TOML 設定(15 tests)
- 2026-08-12: [000_spec.md](archive/000_spec.md) — 仕様書 docs/SPEC_ja.md v1.0
