# 053 — root-sink の RSS 成長 + スループット天井(031 soak 初回捕獲の実欠陥)

**Status: COMPLETED**(2026-08-15 — 結果は末尾)
**Status(起票時): READY**(起票 2026-08-15 Fable — **031 の一晩 soak はこの修正が前提**)
**仕様**: SPEC v1.15 §12-5(一晩 soak: RSS 平坦 + ロスゼロ)/ **v1.16 §12-6**
(≥3× = mini 672 Mbps、10 分 drop 0)/ 絶対ルール(保存系ロスレス)
**証拠**: `reference/_spike/soak_evidence_031/`(CSV 3 本 + report + root_sink ログ)
**発注先想定**: implementer/**Opus**(原因が局所化しない性能調査 + C++ 修正)

## 事実(031 サニティの実測 — 2026-08-15)

1. **RSS 単調成長**: 45 Mbps(≈20 Hz、events_built は offered に**完全追随**、
   `pending_events=0`・終了時 `recorder_queue=0`)でも root_sink RSS が
   **+660 MB/min ≈ +0.55 MB/event** で成長。滞留ではなく**保持**。
   **手がかり: 0.55 MB ≈ 1 イベントの生 payload(139,264 items × 4 B)にほぼ一致** —
   書き終えたイベントを何かが持ち続けている疑い(ただし**推測で直さない** — 計測で特定)。
2. **スループット天井 ≈ 30 events/s**(224 Mbps = 100 Hz 相当を注ぐと 0.30× しか
   捌けない)。decoder RSS 3.2 GB 頭打ち(= PUSH 背圧で正しく滞留)、
   **graw_writer は 13 MB で平坦 = 生 graw 経路は無罪**。詰まりと成長はどちらも
   decoder→root-sink 側。
3. 108 イベント級の既存テスト・オラクルでは検出不能だった(初の数千イベント連続実測)。

## やること

- **A. 原因の計測特定**(プロファイル/heap 計測 — 手段は任せる。macOS なら leaks /
  Instruments / malloc 計測等)。①保持の主体(どの構造が伸びるか)②天井の主体
  (CPU どこで焼けているか)。**結果節に計測の生数値**。
- **B. 修正**(`tools/root_sink/` C++)。**不変条件**: ロスレス意味論・出力バイト列
  (run.root / monitor.root / 終了 JSON)完全不変 — 既存 root_sink 全スイート +
  conformance + 実データオラクル(021 の 3852 events / 0 differences 級)無変更 green で証明。
- **C. 受け入れ実測(soak_harness で)**:
  ① `--mode soak` 30 分 @224 Mbps(100 Hz 相当)— **events_built が offered に追随
  (≥100 events/s)+ 全プロセス RSS 平坦(report の単調性判定 OK)+ 全ロスレスカウンタ 0**
  ② `--mode burst` @672 Mbps(§12-6 v1.16 形)10 分 — drop 0。
  ③ 修正後に decoder 側が新たなボトルネックとして残る場合は**実測を報告**(追加修正は裁定)。
- **D. 相乗り**: soak_harness に SIGINT graceful(現 run を完走 → report 出力 → exit 0。
  一晩走行の運用要件 — 031 実装者の申し送り⑦)。

## 受け入れ

- C++ 全スイート + conformance green(make -j)。cargo ゲート全 green(D 以外 Rust 非接触)。
- C の実測数値(CSV/report)を結果節に。証拠は `reference/_spike/soak_evidence_031/` に追加。

## 結果(2026-08-15 implementer/Opus(スリープ中断 → 再開)→ 発注側(Fable)レビュー PASS)

- **A(計測)**: 成長の主体 = `GET::GDataChannel::fSamples`(TRefArray)の `fUIDs` —
  `TClonesArray::Clear()` はデストラクタを呼ばず、次フレームの placement new で毎フレーム
  迷子になる(malloc_history 最上位スタックで確定、**0.55 MB = 272ch×512×4 B の完全一致**)。
  天井の主体 = Recorder 単スレッドの PEventTPC 生成 **21 ms/event**(Fill 45%(うち zlib 25%)/
  AddValByStrip の std::map insert 29%(131k/event)/ GDataFrame 中間表現 15%)。
  **リークと天井は独立**(修正前後で ms/event 不変)。
- **B(修正)**: `root_recorder.hpp` 実質 1 行(`GetChannels()->Delete()` を Clear() 前に)。
  third_party 無改変。**red 実測済みの成長回帰テスト付き**(200 events で +109 MB → green)。
  挙動不変の証明: root_sink 全 7 スイート + test-root 337 + conformance + **021 実データ
  オラクル `compared 3852 events, 0 differences`** 全て無変更 green。cargo 448 passed。
- **C(実測)**: 45 Mbps 対照 720 s — RSS **585 → 2.0 KiB/event(293×)**・平坦判定 OK・
  4 run 合格・完全追随。ELITPC 級(2.2 MB/event)でも平坦を 6 点実測。
  **C① ≥100 events/s と C② 672 Mbps drop 0 は未達**(実測 32.2 /s = 天井そのもの) —
  掟どおり実装せず報告 = **受理。天井は 054 に分離**(発注書の受け入れを事後修正する裁定)。
  216× 時の counted drop・graw_replay の ENOBUFS 落ちの証拠も保全。
- **D**: soak_harness SIGINT graceful(+64 行、新依存/unsafe なし、現 run 完走 → report。
  実プロセス kill テスト付き)+ soak_smoke のスタックテスト直列化。
- **逸脱の裁定**: 45 Mbps 対照走行の追加 = 受理(キュー在庫と分離するため必須だった)/
  static Mutex = 受理。
- **未決 → 処置**: ①天井 = **TODO/054 起票**(hint 挿入 + ImplicitMT、受け入れは内容一致)
  ②キュー単位(個数→バイト、ELITPC で効く)+ ③過負荷時 EOS 予算 = **SPEC 検討として
  CURRENT.md 保留節へ** ④単調性判定の過敏(小絶対値プロセス)= 一晩レポートの読みで
  人裁定 + 054 相乗り候補。
- 実行環境: macOS Darwin 25.5.0、2026-08-15(途中マシンスリープで中断 → コンテキスト保持
  のまま再開、ゲートは全て通し直し)。

**Status: COMPLETED**(C①② の未達は仕様側の天井 = 054 へ分離、という発注側裁定込み)
