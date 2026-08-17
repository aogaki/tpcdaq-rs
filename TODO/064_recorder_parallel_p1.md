# 064 — Recorder 並列化 P1(worker 毎 TTree、mini 100 Hz へ)

**Status: READY**(起票 2026-08-17 Fable — 裁定 = [archive/055](archive/055_recorder_parallel_ruling.md)。
ELI-NP 実機テスト前に済ませる = 案 Y)
**仕様**: SPEC §6.4/§6.5(ファイル命名は本チケット完了時に v1.20 で改訂)/
§12-5(a) フルレート持続・**§12-6 burst(672 Mbps × 10 分 drop 0)= 031→054 から移管された
最終受け入れをここで閉じる**
**発注先想定**: implementer/**Opus**(並行性 + ROOT スレッド安全の工学判断)
**性能の現状**: 単スレッド 16.06 ms/event = 実稼働 41.5 events/s(054 結果節)。
目標 **≥100 events/s(mini)** + 実環境マージン(worker 数は設定可変)。

## 設計(P1 — 発注側で確定済みの骨子)

1. **イベントビルダは単一のまま**(eventIdx マージの意味論不変)。ビルド済みイベントを
   **round-robin で N worker の有界キューへ**分配(ロスレス — 満杯時はビルダが待つ。
   意図的ドロップ禁止)。
2. **worker = Filler + TTree 一式を専有**: 各 worker が自分の PEventTPC/Filler
   (ジオメトリは共有 read-only)と自分の TFile/TTree を持ち、fill + `Fill()`(圧縮込み)
   を丸ごと並列化する。`ROOT::EnableThreadSafety()` を必ず呼ぶ。
3. **出力命名**: `--recorder-workers N`(既定 1)。**N=1 は現行と完全同一**(コード
   パスも出力名も。既存の全テスト・オラクルが無改変で通ることがその証明)。
   N>1 は worker k が `run{N}_w{k}.root` + 1 GiB ローテーション
   `run{N}_w{k}_0001.root…`。エントリ順は worker 内で eventIdx 単調(round-robin
   なので自然に成立)。ファイル横断の全順序は保証しない(イベント自立・オラクルは
   eventId キーの内容一致なので問題ない)。
4. **カウンタ**: worker ローカルに素の整数で数え、Recorder 層で atomic に集約
   (054 と同じ流儀)。status/終了 JSON は全 worker 合算 + `workers` フィールド追加。
5. **モニタ経路は無改変**: ヒスト充填・スナップショット/イベント publish の現行構造を
   触らない(充填がビルダ側にあるならそのまま。Recorder スレッドにあるなら分配前へ
   移すのが最小手 — 構造を調査し、移動が必要なら報告してから実施)。
6. **EOS/stop**: stop で全 worker のキューを flush → join → 全パート finalize →
   合算カウンタで run close。`eos_closed`/logbook の意味論不変。
7. **IMT との関係**: N>1 のとき `--root-imt` は既定 0 に落とす(worker 並列と
   スレッドプールの二重取りは計測で正当化できた場合のみ)。組み合わせは実測で決め、
   結果を記録。

## 受け入れ(実測 — すべて caffeinate 前置、30 分以内/本)

1. **隔離プローブ**(prof_054_probe 系): N=1/2/4/8 の ms/event スケーリング曲線。
2. **soak_harness 30 分 @224 Mbps**: **events_built ≥ 100/s + 全ロスレスカウンタ 0 +
   RSS 平坦(32 MiB フロア込み判定)+ stop が eos-timeout にならない**(保留②の検証)。
3. **§12-6 burst: 672 Mbps × 10 分で保存系 drop 0**(recv_overflow=0)。✔/✘ を明記 —
   ✔ なら §12 受け入れ表 6 が閉じる。
4. **021 オラクル(内容一致)**: N=2 以上で実 ELITPC 4 本組を流し、**全パートの
   ユニオンが実機 .root と全イベント全 key 一致**(compare_pevent を複数ファイル対応に
   拡張してよい — 拡張自体もテスト対象)。N=1 は既存オラクル無改変 green。
5. **mini 回帰**: N=1 でデモ一周(出力名・内容とも現行一致)。root_sink 全スイート +
   conformance + cargo(触った範囲)green。
6. ELITPC 実効レートの再実測(N=4 での events/s — 100 Hz は目標外だが記録)。

## 移送チェックリスト(壊しやすい点)

① Filler の counted drop 群(items/keys_out_of_range 等)が worker 合算後も
Rust 側 assert と突き合うこと ②duplicate eventId 判定は**分配前(ビルダ)に一元化**
(worker 分散後では見えない)③`run<N>_monitor.root` は従来どおり 1 つ
④終了 JSON の `root_files` 台帳に全パートが載る ⑤SIGINT/SIGTERM graceful が
全 worker join まで待つ ⑥ELITPC probe の AsAd 数引数(054 で追加)を壊さない。

## 非スコープ

- キュー単位のバイト建て化(CURRENT.md 保留①)/ モニタ経路の構造変更 /
  SPEC 文書の改訂(完了時に発注側が v1.20 で行う)/ ZS(不採用確定 — 055)。
