# CURRENT — tpcdaq-rs 現在地

**最終更新: 2026-08-14(**P2 改修波: 025・023 完了コミット済み(ゲート 335 green)、
024(root-sink + テストインフラ)実装中** — 完了したら 026 monitor + WS 起票へ)**

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

**フェーズ訂正(2026-08-13、ユーザー指摘)**: PROPOSAL v0.4 のとおり **P3 = モニタ + WS + UI、
P4 = run 制御**(「SPEC に P4 が無い」は Claude の読み違い — P4 は P5 に吸収されていない)。
**ユーザー決定: 本来の P3(UI)を先に**(デモ最短到達。リプレイ駆動デモに run 制御は不要)。

**P4 前倒し分**:
- 015 logbook / 017 ecc-bridge → **どちらも完了・archive 済み**(2026-08-14 再発注 →
  全 green、逸脱受理。「最近完了」参照)
- [016_controller.md](archive/016_controller.md) — **完了・archive 済み**(2026-08-14。
  「最近完了」参照。ユーザー判断待ち 2 点は下記)。


**P3(モニタ + WS + UI)— 波 1**:
- 018 C++ ジオメトリ → **完了・archive 済み**(test_geo 426 CHECK、合成 3 + 実 .dat 2 本とも
  ダンプがバイト一致。018 時点で未解決だった §13-7 の 2-CoBo .dat 問題は **019 の実データで
  解消** — ELITPC はワイヤ上 1 CoBo × 4 AsAd。「最近完了」参照)
- 020 PEventTPC 出力 → **完了・archive 済み**(v1.8。「最近完了」参照。残: 同 run ペアでの
  実データ値一致(次回 LAN)+ TPCReco 再配布許諾(Warsaw))。
- [022_root_sink_monitor_pub.md](archive/022_root_sink_monitor_pub.md) — **完了・
  archive 済み**(2026-08-14。「最近完了」参照)。
- 以降順次(番号は起票時に採番。023〜025 は P2 改修が先取り — 下記):
  **026 monitor + WS**(§5.4・§10 — 022 の PUB を購読。tests/root_sink_monitor_pub.rs の
  named struct パーサが本番パーサの先行形。起票時チェック: R-P2-13 geometry 可視化フック)
  → Web UI(§11)→ P3 E2E(§12-8〜10 + R10、跨 run 計測 v1.10)→ 負荷ハーネス起票
  (R-P2-11、§12-5/6)。

**ユーザー決定(2026-08-14)→ SPEC v1.10 に反映済み・改修ユニット発注中**:
1. 016 逸脱 = Fable 推奨どおり(**Counters Option 化 = §9.2 改訂** / comment token 不要を
   §8.1 に明文化)。
2. P2 処置表 = **全面「改善してください」承認**(R-P2-1 の mismatch fatal 昇格を含む —
   §6.2-5 v1.10)。
3. インフラ + フォローアップ = 起票承認(「なんだって記録に残したい」)。

**改修 3 ユニット(2026-08-14 起票・worktree 並列発注)**:
- ~~023 Rust 異常系~~ → **完了・archive 済み・コミット済み**(2026-08-14。取り込み後
  ゲート 335 green。逸脱受理 4 — 特に「Stop では打ち切らない」は発注書の誤りを正す判断。
  「最近完了」参照)。
- [024_p2_hardening_root_sink.md](024_p2_hardening_root_sink.md) — root-sink C++
  (R-P2-1 mismatch fatal exit 6 / R-P2-2 AutoSave 飢餓 / R-P2-5 pending_events)+
  spawn テストインフラ(TOCTOU リトライ + Drop ガード)。implementer/Sonnet。
- ~~025 controller フォローアップ~~ → **完了・archive 済み・コミット済み**(2026-08-14。
  取り込み後ゲート 323 green。「最近完了」参照)。
- 残る P2 処置: R-P2-11 負荷ハーネス(P3 E2E 後に起票、Warsaw 前必須)/ R-P2-13 の
  geometry 可視化フックは 026 monitor 起票時のチェック項目。
- **UI ユニット起票時の方針(ユーザー決定 2026-08-13)**: run 制御のボタン類は完成形レイアウトとして
  **見た目だけ置き disabled**(P4 の REST が来たら配線するだけ)。**デモ用のモック関数・
  仮バックエンドは作らない** — デモで動くのはモニタ経路(リプレイ → PUB → WS → UI)のみ。

**設定方針の確認(2026-08-13 ユーザー質問 → WARSAW_PLAN §2 に明記)**: CoBo/FPGA に入る
ハードウェア設定 xcfg は**先方の既存ファイルをそのまま**(config_id 参照のみ、生成・改変しない)。

**保留(ユーザー決定)**: 物理屋向け PROPOSAL 資料とデモは UI + ファイルデータソースが
できるまで待つ。
**待機解除(2026-08-13)**: ユーザーが閉域 LAN から実データ(2022 / 2026)を取得して
`reference/exp_data/` に配置 → 019 で回帰組み込み完了。P3 波の続き(020〜)を再開できる。

**SPEC 改訂の要点(2026-08-13、v1.1→v1.7)**: AsAd 毎ファイル + 実機 DataRouter 命名(v1.1)/
非 AsAd 制御フレームは `run{N}/ctrl/` へ保全 + REP 採番固定(v1.2)/ イベント内フラグメント順 =
(cobo,asad) 昇順(v1.3)/ ロスレス PUSH に ZMQ_IMMEDIATE 必須(v1.4)/ run.root 圧縮既定
101 ZLIB-1(v1.5)/ 中止も EOS で閉じる正規経路(v1.6)/ **ELITPC = 1 CoBo × 4 AsAd・
frameType 2 rev 5 固定・ローテーション書き込み後判定(v1.7、実データ実測)**

**波 2 以降(順次起票)**:
- 009 decoder コンポーネント(Rust。EOS 集約 = SPEC §2.3「decoder のソース性」2026-08-12 追記)
- 010 eventIdx ビルダ(C++ 純ヘッダ、SPEC §6.3)
- 011 GDataFrame TTree + third_party/get 隔離(C++/ROOT 6.36.10 @ /opt/ROOT 確認済み)
- 012 P2 E2E(graw_replay ×2 → 2 receiver → decoder → root-sink、TTree 互換比較 §12-3)

## 継続事項

- ~~P2 完了時に批判的レビューを一度行う(ユーザー指示 2026-08-13)~~ →
  **完了・クローズ(2026-08-14)**: 所見 14 件([P2_REVIEW.md](P2_REVIEW.md)、
  正常系保存経路への所見なし)→ 処置表を**ユーザー承認**、SPEC v1.10 反映 +
  023/024/025 へ分割発注済み(上記)。残タスクは R-P2-11(負荷ハーネス、P3 E2E 後)のみ。
- ~~P2 批判的レビューの論点(009 逸脱 6): Fragment.cobo と source_id の不一致検出~~ →
  **013 で実装済みと確認**(decoder.rs:251、R-P2-7)。

- ~~006 レビュー指摘の再訪: 下流全死 + EOF 前 Reset での EOS 再試行が畳めない件~~ →
  **023 で解消**(2026-08-14。受信側送信の中断可能ロスレス化 + abandon 可視カウント +
  スレッド join。P2_REVIEW R-P2-3)。
- ~~008 レビュー申し送り: SeqCheck 初回 0 強制~~ → **010 で解消**(厳格モード実装・実配線確認済み)。
- **delila-rs への申し送り**(010 で発見): `tools/root_sink/eb_core.hpp` の `pop_for`(bool 戻り)は
  timeout と closed を区別できず、「空で戻る → 生産者が push+close → 呼び手が break」の競合で
  **最後の 1 通を落とす**。tpcdaq-rs は PopResult 3 値で回避済み。delila-rs 側も要修正 →
  **issue 化済み(2026-08-14): https://github.com/ELI-NP/delila-rs/issues/26**(現行 2 呼び出し
  箇所は in-band pill 終了のため潜在、API 契約が競合を招く旨と 3 値修正案を記載)。
- Warsaw 確認事項: ~~2-CoBo ジオメトリ .dat の有無~~(**019 で解消** — 不要と確定)、
  **データリンク本数(zCoBo 2 枚 → TCP 1 本か 2 本か、SPEC §13-7)**、PROPOSAL v0.5 反映判断。
- ~~P2 レビュー R2(frameType 1 の実データ回帰なし)~~ → **019 でクローズ**(実機は 2022 時点で
  既に frameType 2 — frameType 1 の実データは存在しない。回帰 = `TPCDAQ_REAL_GRAW_DIR`)。
- ~~P2 レビュー R3(Reset カスケードで root-sink が fatal 死)~~ → **016 で解消**
  (2026-08-14。中止経路 = SPEC §1.3 v1.6 を controller が実装、「run クローズ前に
  decoder を Reset しない」を機械照合テストで固定)。

## 最近完了

- 2026-08-14: [023_p2_hardening_rust.md](archive/023_p2_hardening_rust.md) —
  **Rust 異常系改修(P2 R-P2-3/8/9/10/12 クローズ)**(poisoned Mutex 即 Error /
  receiver 送信の中断可能ロスレス化 + abandon 可視カウント + スレッド join(下流死で
  Stop 609 µs・Reset 1.0 s 実測)/ graw-writer 異常系 4 経路の計数 / batches の意味
  厳格化。新規 12 テスト・TDD 赤確認済み。取り込み後 335 green)
- 2026-08-14: [025_controller_followups.md](archive/025_controller_followups.md) —
  **controller フォローアップ**(asad_inventory(lookup 非経由)/ `POST /api/run/next`
  (SPEC v1.10 §8.1)/ Counters `Option<u64>` 化 = null は取得不能(§9.2)/ logbook
  復旧 warn(R-P2-13)。317→取り込み後 323 green。逸脱 4 件受理 — `Value` 受け手検証で
  一律 400 等)
- 2026-08-14: [022_root_sink_monitor_pub.md](archive/022_root_sink_monitor_pub.md) —
  **root-sink ヒスト集計 + モニタ PUB(P3 モニタ経路の起点)**(SPEC v1.9 §5 実装。
  monitor_hist 202 + monitor_pub 87 + recorder 216 CHECK、Rust 統合 6 本。hist_snapshot が
  Rust 独立再計算と全一致、実データ StripTime 総和 963,434,812 一致、R10 実測 green、
  §12-8 初計測 +0.3% 未満。逸脱 8 件受理 — PUB bind 非 fatal(保存 > モニタ)等。
  既知: intake テストの free_endpoint TOCTOU flake を Fable が原因特定 → インフラ小
  ユニット候補)
- 2026-08-14: [016_controller.md](archive/016_controller.md) — **controller(P4 の核、
  worktree 並列発注で前倒し完了)**(REST 8 エンドポイント + Sequencer(開始・巻き戻し・
  停止・強制 EOS・中止 = §1.3 v1.6 機械照合)+ LogPost。新規 41 テスト、統合後リポ全体
  **316 green**。ecc 実配線 2 本 green。逸脱 9 件中 6 受理・2 ユーザー判断待ち・
  フォローアップ 2)
- 2026-08-14: **P2 批判的レビュー完了**([P2_REVIEW.md](P2_REVIEW.md)) — 所見 14 件
  (high 2 / med 5 / low 6 / info 1)。正常系保存経路への所見なし。処置はユーザー判断待ち。
- 2026-08-14: [021_same_run_oracle.md](archive/021_same_run_oracle.md) — **同 run 実データ値一致
  = §12-3 v1.8 ③ クローズ**(実 graw 4 本 → フルチェーン → run.root が実機 grawToEventTPC
  出力と `compared 3852 events, 0 differences` の完全一致。graw_replay 複数ファイル
  eventIdx マージ(D1 必須機能)+ root_sink --run-id 同梱。020 の残は TPCReco 許諾のみ)
- 2026-08-14: [017_ecc_bridge.md](archive/017_ecc_bridge.md) — ecc-bridge(C++/Ice)+ fake-ECC
  (ZMQ REP 47200 の JSON サーバ、DataLinkSet XML 全文照合、listen-before-start 負性テストが
  実機文言一致で green。Ice 3.8.2・encoding 1.1 実測ログ化・.ice は実験同一版とバイト一致。
  単体 136 + 統合 27 + Rust 2)
- 2026-08-14: [015_logbook.md](archive/015_logbook.md) — JSONL ログブック + next_run 永続化
  (src/logbook.rs + src/state.rs。golden 照合・スキーマ漂流ガード・kill -9 耐久・atomic
  rename 重複ゼロ。32 テスト。controller(016)の依存が解消)
- 2026-08-13: [020_pevent_output.md](archive/020_pevent_output.md) — **run.root を PEventTPC
  (TPCReco 互換)へ変更**(SPEC v1.8、ユーザー裁定「GDataFrame 出力は瑕疵」。TPCData/Event、
  grawToEventTPC と同一充填(strip 射影・signal 窓・FPN ペデスタル減算既定 ON)、TPCReco
  クラスは参照ビルド(ライセンス無指定のためコミットせず)。streamer checksum 実機三者一致、
  test_pevent 119、既存回帰全 green(GDataFrame は --format gdataframe のテスト専用に降格)。
  残: 同 run ペア値一致 + Warsaw 再配布許諾)
- 2026-08-13: [019_elitpc_real_data.md](archive/019_elitpc_real_data.md) — **ELITPC 実データ回帰 +
  ローテーション実機一致修正**(reference/exp_data/{2022,2026} 各 4 ファイルを実測:
  **1 論理 CoBo × 4 AsAd・frameType 2 rev 5・各 _0000 = 3852 フレーム連続**。§13-7 解消・
  R2 クローズ。graw-writer を実機 FrameStorage の書き込み後判定へ修正し、実 1 GiB ファイルとの
  完全バイト一致を回帰で固定(修正前 red → 修正後 green)。SPEC v1.7。追記: 実機オフライン変換
  出力 PEventTPC .root も実見 — **ZLIB-1 一致・ROOT 6.08/06 確定**、WARSAW_PLAN §4)
- 2026-08-13: [018_cpp_geometry.md](archive/018_cpp_geometry.md) — root-sink 側 C++ ジオメトリ +
  §4.5 二重実装一致(geo.hpp 純ヘッダ、test_geo 426 CHECK。dump_tsv が Rust とバイト一致 —
  合成 3 本 + 実 mini/ELITPC .dat。意味論の正 = Rust、原本 C++ の欠陥 6 点を記録。レビュー済み)
- 2026-08-13: [014_root_compression.md](archive/014_root_compression.md) — run.root 圧縮設定化
  (既定 505 ZSTD-5 → **101 ZLIB-1**、`--root-compression` で上書き可。SPEC v1.5 / Warsaw 旧 ROOT
  互換。test_recorder 169 CHECK、E2E entries=108 不変、サイズ +37% は許容。レビュー済み)
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
