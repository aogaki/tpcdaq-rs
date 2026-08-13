# CURRENT — tpcdaq-rs 現在地

**最終更新: 2026-08-13(**P2 完全クローズ** — 007〜013 全 COMPLETED、出口 4 項目実測達成 +
飢餓バグ R1 解消。批判的レビュー済み → [docs/reviews/P2_review_2026-08-13.md](../docs/reviews/P2_review_2026-08-13.md)。SPEC v1.4。次: P3 起票)**

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

**P2 完全クローズ(007〜013)。オープンチケットなし。**

- 次: **P3(run 制御)の起票** — controller / ecc-bridge / JSONL ログブック(SPEC §8/§9)。
  設計入力: レビュー R3(Reset カスケード — decoder の abandon が root-sink を exit 3 で殺す。
  「run 中止の正規手順」を停止シーケンスとして設計する)。
- P2 コミットはユーザー判断待ち(混入チェック済み)。

**SPEC 改訂の要点(2026-08-13、v1.1→v1.4)**: AsAd 毎ファイル + 実機 DataRouter 命名(v1.1)/
非 AsAd 制御フレームは `run{N}/ctrl/` へ保全 + REP 採番固定(v1.2)/ イベント内フラグメント順 =
(cobo,asad) 昇順(v1.3)/ ロスレス PUSH に ZMQ_IMMEDIATE 必須(v1.4)

**波 2 以降(順次起票)**:
- 009 decoder コンポーネント(Rust。EOS 集約 = SPEC §2.3「decoder のソース性」2026-08-12 追記)
- 010 eventIdx ビルダ(C++ 純ヘッダ、SPEC §6.3)
- 011 GDataFrame TTree + third_party/get 隔離(C++/ROOT 6.36.10 @ /opt/ROOT 確認済み)
- 012 P2 E2E(graw_replay ×2 → 2 receiver → decoder → root-sink、TTree 互換比較 §12-3)

## 継続事項

- **P2 完了時に批判的レビューを一度行う(ユーザー指示 2026-08-13)** — 波 1+2 の全 diff・SPEC 整合・
  テスト網羅を Fable が一括で批判的に見る(単なる green 確認ではなく設計の粗探し)。

- 006 レビュー指摘の再訪: 下流全死 + EOF 前 Reset での EOS 再試行が畳めない件(shutdown 経路の上限)→ 007/009 の停止設計で考慮。
- ~~008 レビュー申し送り: SeqCheck 初回 0 強制~~ → **010 で解消**(厳格モード実装・実配線確認済み)。
- **delila-rs への申し送り**(010 で発見): `tools/root_sink/eb_core.hpp` の `pop_for`(bool 戻り)は
  timeout と closed を区別できず、「空で戻る → 生産者が push+close → 呼び手が break」の競合で
  **最後の 1 通を落とす**。tpcdaq-rs は PopResult 3 値で回避済み。delila-rs 側も要修正。
- P2 批判的レビューの論点(009 逸脱 6): Fragment.cobo(ヘッダ値)と受信 source_id の不一致検出を
  入れるか(DataLinkSet 誤配線の早期検出になる)。
- Warsaw 確認事項: 2-CoBo ジオメトリ .dat の有無(SPEC §13-7)、PROPOSAL v0.5 反映判断。
- P2 レビュー R2(frameType 1 の実データ回帰なし)→ **ユーザーが 2022 と 2026 の実データを
  入手予定**(2026-08-13 決定。「現在の実装に合わせるべき」— 入手後に環境変数パスの任意回帰へ
  組み込む)。
- P2 レビュー R3(Reset カスケードで root-sink が fatal 死)→ **P3 で実装**(2026-08-13
  ユーザー決定。controller の停止シーケンス設計に本項を必須入力とする)。

## 最近完了

- 2026-08-13: [013_decoder_starvation.md](archive/013_decoder_starvation.md) — decoder 飢餓修正
  (**真因確定 = libzmq `inbound_poll_rate=100` — recv 100 回に 1 度しか process_commands が
  走らず、fair-queue から外れたパイプが戻れない**。当初仮説は測定で棄却。修正 = recv 前に必ず
  poll(期限ベース待ち 100 ms 上限)。dev/release とも E2E-B skip なし green、226 テスト。
  cobo_mismatch カウンタ同梱。レビュー済み)
- 2026-08-13: [012_p2_e2e.md](archive/012_p2_e2e.md) — **P2 E2E = P2 出口 4 項目すべて実測達成**
  (TTree 互換: 実機オラクルと 15,040,512 サンプル全値一致、許容差は明示 2 件で尽きることを実証。
  2 ソースビルド complete=108/incomplete=0 + 決定性完全一致。decoder 飢餓バグを発見・切り分け
  → 013。レビュー済み)
- 2026-08-13: [011_gdataframe_ttree.md](archive/011_gdataframe_ttree.md) — GDataFrame TTree +
  third_party/get 隔離(CeCILL、md5 記録つき無改変コピー。Recorder スレッドで ROOT IO を分離、
  test_recorder 163 CHECK、実 .graw E2E で entries=108 / samples=15,040,512 完全一致、
  inprogress→rename・rollover 実証。frame_type/run_number 非搭載と 0 イベント run 無ファイルを
  SPEC §6.4/§6.5 に明文化。レビュー済み)
- 2026-08-13: [009_decoder.md](archive/009_decoder.md) — decoder コンポーネント(新規 27+3 テスト、
  リポ全体 218 green。実 .graw E2E で P1 オラクル完全一致(events=108 / items=15,040,512 /
  unsupported=1)、release 0.19 s ≈ 158 MB/s。ZMQ_IMMEDIATE 問題を発見 → SPEC v1.4 に昇格し
  zmq_helper へ集約 + receiver 展開まで完了。Reset 時 EOS 打ち切り設計で 006 積み残しを解消。
  レビュー済み)
- 2026-08-13: [010_event_builder.md](archive/010_event_builder.md) — eventIdx イベントビルダ
  (C++ 純ヘッダ、時刻注入。test_eb_core 175 CHECK 新規、make test 4 本 green、統合 7/7。
  (cobo,asad) 昇順 emit = SPEC v1.3、SeqCheck 厳格モードで 008 申し送り解消、delila-rs pop_for の
  競合バグ発見・回避。レビュー済み)
- 2026-08-13: [007_graw_writer.md](archive/007_graw_writer.md) — graw-writer(AsAd 毎ファイル・実機
  DataRouter 命名・ctrl/ 保全。新規 28 テスト、リポ全体 185 green。実 .graw E2E: AsAd 30,108,672 B +
  ctrl 12 B = 30,108,684 B の完全ロスレス分割、ctrl_frames=1・drop 0、~0.2 s/30 MB。実装が SPEC の
  穴(非 AsAd 制御フレーム)を発見 → v1.2 で ctrl/ 保全を規定して解消。レビュー済み)
- 2026-08-13: [008_root_sink_intake.md](archive/008_root_sink_intake.md) — root-sink 取り込み骨格
  (C++。make test 68+71+49 green、クロス言語適合 144 B/4 通一致、背圧実測 = 非ブロッキング送信が
  2 通目で EAGAIN・256 KiB×16 通ロス 0、cargo 176 green・clippy クリーン。レビュー済み — 逸脱 6 点
  受理、SeqCheck 初回 0 強制は 009/012 へ申し送り)
- 2026-08-12: [006_receiver.md](archive/006_receiver.md) — receiver(16 tests。**P1 出口達成**: 実 .graw 全速リプレイ byte 一致 + overflow 0。過負荷でも drain 継続を実証。tracing 導入)
- 2026-08-12: [005_graw_replay.md](archive/005_graw_replay.md) — graw_replay(16 tests。--rate-mbps = Mbit/s 確定 → SPEC §12)
- 2026-08-12: [004_framer_decoder.md](archive/004_framer_decoder.md) — framer + デコーダ(23 tests。**実 .graw オラクル完全一致**)
- 2026-08-12: [003_zmq_core.md](archive/003_zmq_core.md) — ZMQ メッセージ核 + 状態機械(48 tests)
- 2026-08-12: [002_geometry.md](archive/002_geometry.md) — ジオメトリ抽象(35 tests。実 .dat オラクル一致 = P0 出口)
- 2026-08-12: [001_scaffold_config.md](archive/001_scaffold_config.md) — scaffold + TOML 設定(15 tests)
- 2026-08-12: [000_spec.md](archive/000_spec.md) — 仕様書 docs/SPEC_ja.md v1.0
