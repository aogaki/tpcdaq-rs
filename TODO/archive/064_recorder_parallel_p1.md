# 064 — Recorder 並列化 P1(worker 毎 TTree、mini 100 Hz へ)

**Status: COMPLETED**(2026-08-18 — 結果は末尾。起票 2026-08-17。
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

---

## 結果(2026-08-17〜18 — implementer/Opus 実装、発注側(Fable)一括レビュー PASS)

### 実装

Recorder を **dispatcher + RecorderWorker**(TFile/TTree/PEventTPC/Filler 専有)に分割。
有界キュー(4 段/worker、満杯は分配側が待つ = 背圧・捨てない)、BeginRun/CloseRun は
in-band マーカー(追い越し構造なし)、stop は全 worker join。**N=1 はスレッドも
キューも作らず呼び手スレッドで直接実行 = 現行と同一コードパス・同一出力名**
(既存テスト無改変 green が証明)。N>1 は `run{N}_w{k}.root` + worker 毎 1 GiB
ローテーション。duplicate eventId 判定は分配前に一元化。モニタ経路は無改変
(充填・publish は元よりビルダ側 = 分配前と実地確認)。

### ゲート(発注側追試済み)

test_recorder **101 → 170**(P1 9 本 + ネガティブコントロール)/ 他 C++ 7 スイート +
conformance 無改変 green / cargo **454 passed / 0 failed / 1 ignored**(soak_harness
+1)/ clippy・fmt クリーン。

### 受け入れ実測(全 ✔)

1. **N スケーリング(mini)**: 51.6 → 96.7 → **152.0(N=4、2.95×)** → 215.8 events/s(N=8)。
   **N=4 で目標 100 の 1.5× マージン**。
2. **soak 30 分 @224 Mbps / N=4**: **10/10 run 合格、100.4 events/s、達成 223.9 Mbps、
   全ロスレスカウンタ 0、eos-timeout なし(= 保留②解消)、RSS 8 プロセス OK**
   (root_sink 傾き負、絶対値 ≈0.87 GiB — N=1 比 2 倍、実環境見積もり材料)。
3. **§12-6 burst 672 Mbps × 10 分 / N=4**: **recv_overflow = 0**(031 実測 94,544 → 0)、
   達成 648.4 Mbps、実効 290.8 events/s、全 run 正常停止。**受け入れ表 6 クローズ**。
4. **021 オラクル**: N=1 無改変 `3852 events, 0 differences`(compare 828.8 s —
   054 と同等 = 実読の裏付け)。**N=2 ユニオン(w0 1,926 + w1 1,926)も
   `3852 events, 0 differences`**(compare_pevent 複数ファイル対応、テスト無改変・
   ラッパ注入)。
5. mini 回帰 green。 6. ELITPC: N=4 で 38.2 events/s(2.95×)。

### チェックリスト①〜⑥

全消し込み(counted drop 合算 10c / duplicate 一元化 10d / monitor.root 単一 10f /
台帳全パート 10a+soak 120 パート / graceful join 10e / ELITPC probe 健在)。

### 逸脱(裁定済み)

1. verify_run の存在チェックを worker 追随(発注時指示との差分 — **裁定で許可**。
   純関数化 + 単体テスト、判定強度不変)。
2. `--root-imt` は N>1 既定 0(裁定確定)。実測 N=4+IMT4 で再現性ある +6% —
   高コア機は `--root-imt 4` 明示で回収可(N=8 は差なし)。
3. **compare_pevent の複数ファイル化で silent failure を自ら混入 → 自己摘発 → 修正**:
   TChain の SetBranchStatus 状態が 1 ファイル chain で再適用されず chargeMap を空読み
   (両側空 = 偽の 0 differences)。「照合 27.5 s は速すぎる」を疑いネガティブ
   コントロール(1 ADC 差を検出できるか)で発見。索引用/比較用 chain 分離で修正し、
   **不一致検出テスト(10i)を対で常設**。教訓: 一致テストには必ず不一致テストを対で。

### 残件(064 範囲外として起票せず記録)

- burst 中 RSS ピーク root_sink 6.88 GiB / decoder 2.98 GiB — 主因は既存の
  `--queue` 既定 1000 バッチ(メッセージ個数建て)。**保留①(バイト建て化)の実測材料**。
- §12-5(a) の「一晩 @100 Hz 相当」は 30 分 @224 + 一晩 @45(031)の合わせ技で実質充足だが、
  **凍結前に一晩 @224 / N=4 を 1 本流すのを推奨**(任意)。

- 実行環境: macOS Darwin 25.5.0 / M4 Pro 14 CPU(バッテリー駆動でも合格値)、
  2026-08-17〜18。証拠 = `reference/_spike/soak_evidence_064/`。

**Status: COMPLETED**
