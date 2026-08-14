# P2 批判的レビュー(フェーズ境界一括、Fable)

**Status: CLOSED(2026-08-14 — 処置表をユーザー承認、「これらは改善してください」。
SPEC v1.10 に反映済み(R-P2-1/4/5/6/14 + 016 逸脱③⑧ + run 番号 REST)。実装は
023(Rust 異常系)/ 024(root-sink C++ + テストインフラ)/ 025(controller
フォローアップ)へ分割発注。R-P2-11 の負荷ハーネスは P3 E2E 後に起票(Warsaw 前必須))**
**根拠**: CURRENT.md 継続事項「P2 完了時に批判的レビューを一度行う(ユーザー指示
2026-08-13)— 単なる green 確認ではなく設計の粗探し」。
**方法**: Fable が C++ 核(eb_core / rs_core / root_recorder / root_sink.cxx)と Rust 側
異常経路を深読み + Explore/Sonnet 2 本(Rust 全域 silent-failure 監査 / SPEC§12・§6.2・
不変条件 ↔ 実テスト網羅マップ)。監査の HIGH 指摘は全件 Fable が現物で裏取り済み。
**対象**: HEAD = d2f3abf(022/016 が並行実装中のため。両ユニットのレビューは別途)。

## 判定サマリ

P2 の出口 4 項目(graw バイト一致 / TTree 互換 / run 毎単一 ROOT / 2 ソースビルド一致)は
実測クローズ済みで、オラクル被覆(mini 15,040,512 サンプル全値・ELITPC 3852 イベント
0 differences)は強い。**正常系の保存経路の正しさに対する所見はなし**。
所見は 14 件: **high 2**(いずれも「異常が起きたときにそれが見えない」系 —
絶対ルール「silent failure を作らない」への違反)、med 5、low 6、info 1。
共通パターンは 2 つ: ①**decoder は厳格だが receiver / graw-writer が同水準にない非対称**
(R-P2-3/9/10/12 — decoder のパターンを移植すれば揃う)、②**未結線フックの地雷**
(R-P2-13 — 016/023 のレビュー時に確認)。

## 所見(深刻度順)

### R-P2-8 [high] poisoned Mutex を「エラーなし」に丸める + スレッド死の検知手段がない

decoder.rs `latch_error`(944–961)と graw_writer.rs 同(794–810)がともに
`.lock().ok().map(|g| g.errored()).unwrap_or(false)` — **ワーカースレッドが Mutex 保持中に
panic して死ぬと、poisoned lock が「errored = false = 正常」に化ける**。JoinHandle も
保持していないため生存確認の別手段がない。結果: スレッド全停止でも Handler は
`state=Running` で GetStatus に success を返し続ける(delila-rs 2026-05-04 事案と同型の
不可視化)。panic 源候補は graw_writer.rs 489–551 の `.expect()` 群(監査指摘)。
**修正案**: `lock()` の Err(poisoned) を「即 Error 遷移 + warn」に(1 箇所 2 行 × 2 ファイル)。
JoinHandle 保持(is_finished 検査)は controller の死活監視(P4)と重複するので任意。

### R-P2-3 [high(med から昇格)] receiver 送信経路: 永久ブロック + 送信失敗の無カウント

receiver の `send_on()` は SNDTIMEO なしのブロッキング送信(zmq_helper の sndtimeo は
テストコード内のみ)。2 つの問題が同居:
1. **下流全死で sender スレッドが send 内永久ブロック**(broadcast stop で起こせない)。
   §1.3 v1.6 の「abandon は可視カウント」が receiver には未実装(006 積み残しの現状)。
2. **失敗が無カウント**(監査 H1): encode 失敗 → `error!` のみで dropped、ETERM 以外の
   send エラー → `error!("this message is lost")` のみ。Metrics に対応カウンタが存在せず、
   ロスレス契約が破れても GetStatus 上は正常に見える。
**decoder は両方解決済み**(sndtimeo = `send_timeout_ms`(decoder.rs:1067)+ retry ループ +
`batches_abandoned`/`eos_abandoned` カウンタ + FULL テスト
`reset_abandons_the_blocked_send_and_counts_what_was_dropped`)。
**修正案**: decoder の `send_lossless` パターンを receiver へ移植(捨てない・中断可能・
abandon 可視)。016/P4 の停止シーケンス設計と同波が自然。

### R-P2-1 [med] 混在 run のイベントが「完成 run」ファイルに化ける(root_recorder)

`Recorder::write()` は開いている run と違う `run_number` のイベントを見ると、旧 run を
**`finalize=true`**(= 正式名へ rename)で閉じて新 run を開く。run_number 混在は
プロトコル違反(RunState が `run_number_mismatch` で計数)だが、発生時:
- EOS を見ていない run が `run{N:04}.root` の正式名を得る — 「異常終了は inprogress の
  まま = 完全 run に化けない」(§6.5)の原則と不整合。
- さらに `close_run(N)` が後から届くと、**新 run の書きかけファイルまで finalize** される。
**修正案**(SPEC 判断): ロスレスリンクの run_number_mismatch を §6.2-5 の seq ギャップと
同格の **fatal に昇格**が一番単純(decoder は単一ストリームで run を混ぜない契約 —
混ざった時点で「正しく続ける」方法は存在しない)。最低限でも mismatch 経路の close は
finalize=false に。

### R-P2-2 [med] 連続データ中に AutoSave が一度も走らない(root_sink.cxx Recorder ループ)

Recorder スレッドは `pop_for` が Value を返す限り `tick()` を呼ばない(tick は
Timeout/Closed 分岐のみ)。データが途切れない run では **AutoSave(30 s)が一度も
実行されず**、kill -9 / 電源断時の inprogress 回復性が下がる(§12-5 の 24 h 試験の意図と
逆)。生 graw がバックストップなのでデータ喪失ではない。
**修正案**: write() 側(または Value 分岐)で deadline チェック 1 行。022 レビュー時に
同梱修正できる規模。

### R-P2-9 [med] graw-writer の異常系が decoder 比で緩い(監査 M2/M3/M4 + Fable 追加)

decoder の `handle_eos` 相当と比べて 4 点(いずれも graw_writer.rs 673–700 周辺):
- EOS の run_number 不一致が warn のみで `run_mismatches` に**計上されない**(M2。
  Batch 側は計上する — 非対称)。
- **期待外 source_id の EOS を無言で `eos_received` に insert**(M4。閉じ判定は
  subset なので早閉じはしないが、DataLinkSet 誤配線の検知材料が残らない)。
- デコード不能メッセージを warn + skip(**無カウント**)。Data ならその後の seq gap で
  間接検出されるが、失われた EOS は「run が閉じない」でしか見えない。
- Heartbeat 受信カウンタなし(M3。decoder は `heartbeats_in` を持つ)。
**修正案**: decoder の handle_eos / カウンタ設計に揃える(1 ユニット未満の作業量)。

### R-P2-10 [med] framer リセット(MFM ヘッダ崩れ)の瞬間に能動ログがない(監査 M5)

receiver.rs 246–248 は `framer_resets` カウンタを転写するだけで、増分検知時の
`warn!` がない。GetStatus をポーリングしない限り、CoBo リンクのフレーミング崩れの
継続に気づけない。**修正案**: 増分時に一度だけ warn(decoder の logged_* 方式)。

### R-P2-11 [med] §12-5(24 h)/ §12-6(10 分瞬発)の負荷試験ハーネスが未起票

網羅マップで判明: 受け入れ基準のうちこの 2 つだけ、**テストどころかチケットも存在しない**
(近縁の全速リプレイ実測はあるが対象時間が別物)。Warsaw 展開前に必要。
**修正案**: P3 E2E の後(モニタ経路込みで測る意味があるため)に「書いて検証して消す」
ループハーネスとして起票。RSS 単調性チェック込み。

### R-P2-4 [low] 重複 (cobo,asad) フラグメントが電荷二重加算になる(eb_core → pevent_fill)

EventBuilder は重複フラグメントを duplicate_fragments で計数しつつイベントに載せる
(「捨てない」原則)。PEvent 充填は fragments を全部 fill するので同一チャンネルの ADC が
chargeMap に二重加算される(022 のヒストも同様)。実機で重複は起きない想定 + カウンタで
可視だが、「重複時にどちらを採るか」の意味論が SPEC 未規定(grawToEventTPC の
fragment 単位重複の挙動は未照合)。**修正案**: SPEC §6.3 に 1 行追記のみ。

### R-P2-5 [low] EventBuilder pending のメモリ上界が build_timeout 比例で無警告

pending ≈ レート × build_timeout × イベントサイズ(ELITPC フル読み出し 100 Hz・1 s で
~220 MB)。`--build-timeout-ms` を大きくすると上界が黙って伸びる(片肺 AsAd 恒常欠落が
最悪ケース)。**修正案**: pending 数の警告閾値 + status 露出(022 の status 配信に相乗り)。
hard limit はロスレス契約と衝突するので入れない。

### R-P2-6 [low] seq ギャップの扱いが decoder(Error + 続行)と root-sink(即 fatal)で非対称

どちらも仕様に根拠はある(§1.4-3 の「source は Error 報告 → controller が止める」/
§6.2-5 の sink fatal)が、役割非対称そのものの明文がない。**修正案**: SPEC §1.4 に
1 行明文化のみ(P4 の異常停止シーケンス設計で迷わないため)。

### R-P2-12 [low] decoder に heartbeat 送出カウンタがない(監査 M1)

`send_heartbeat` は `let _ = send_lossless(...)` で結果を捨てる。Batch/EOS は
out/abandoned が対になっているのに heartbeat だけ非対称。**修正案**: R-P2-3 の
receiver 修正と同波で `heartbeats_out` を足す(任意 — 実害は診断材料の欠落のみ)。

### R-P2-13 [low] 未結線フックの地雷 3 件(結線するユニットのレビュー時に確認)

- **geometry の可視化フック**(unmapped_hits / duplicates / malformed_lines)は
  アクセサだけあって呼び手がいない(監査 L1)。C++ 側(018/020)は起動時に出す —
  **023 monitor の Rust 側ジオメトリ結線で同じ可視化を忘れない**こと。
- **logbook `recover_next_seq`** は「末尾 1 行だけ壊れる」前提の実装が複数行破損でも
  無警告で遡る(監査 L2)。**016 レビュー時に確認**(スキップ行数 > 1 で warn)。
- decoder `batch_abandoned` の doc「Reset 中に限り」は実装(一般 ZMQ エラーでも通る)と
  乖離(監査 L3)。コメント修正のみ。

### R-P2-14 [low] §6.2-8「run 境界中も intake が止まらない」の定量テストがない

構造的分離(専用スレッド + 有界 Channel)と rollover 完走テストで裏付くが、定量測定は
ない。022 の §12-8 非干渉計測が部分的に代替する — **P3 E2E の測定項目に「run 境界跨ぎの
スループット」を 1 行入れる**だけでよい。

### R-P2-7 [info] 009 逸脱 6(Fragment.cobo vs source_id 不一致検出)は解消済み

CURRENT.md の論点は **013 で実装済み**: decoder.rs:251 で `cobo_mismatch` カウント +
初回 warn(DataLinkSet 誤配線を指す文言つき)。継続事項から消してよい。

## テスト網羅マップの要点(詳細は監査結果 — 全文はセッション記録)

- **FULL で閉じている**: §6.2-3/7(Channel 有界・closed push assert)、§2.5 スキーマ漂流
  ガード(Rust 定数表 + C++ 前方互換の両側)、FPN 表両実装一致、frameType 1/2 合成、
  listen-before-start(receiver 側)、中止経路の構成要素(強制 EOS / 全ソース EOS 待ち /
  decoder Reset abandon)。
- **GATED(env 付きでのみ)**: 実データオラクル系は全部 `TPCDAQ_REAL_*` / 
  `TPCDAQ_ROOT_SINK_BIN` gate — 設計どおり(実データはリポに入れない)。
- **GAP のうち 022/023/016 が埋める予定**: §12-8/9/10、§6.2-4、§12-7 全通し、
  §12-11 kill -9 統合。**予定が無いのは §12-5/§12-6 のみ**(→ R-P2-11)。
- 監査の「golden conformance は手動受け渡し」は誤認 — `run_conformance.sh` が生成 →
  make test まで自動化済み(cargo test 外なのは事実)。

## 処置一覧(ユーザーと決める)

| 所見 | 深刻度 | 提案する処置 |
|---|---|---|
| R-P2-8 | high | 独立小修正(2 ファイル × 2 行 + テスト)— 即時実施を推奨 |
| R-P2-3 | high | decoder パターンの receiver 移植 — 016/P4 停止設計と同波で 1 ユニット |
| R-P2-1 | med | SPEC 判断(mismatch fatal 昇格)→ 小修正 |
| R-P2-2 | med | 022 レビュー時に同梱修正(1 行級) |
| R-P2-9 | med | graw-writer を decoder 水準へ — R-P2-10/12 と束ねて 1 小ユニット |
| R-P2-10 | med | 同上に同梱 |
| R-P2-11 | med | P3 E2E 後に負荷ハーネスを起票(Warsaw 前必須) |
| R-P2-4 | low | SPEC §6.3 に 1 行追記のみ |
| R-P2-5 | low | 022 の status に pending 露出(相乗り) |
| R-P2-6 | low | SPEC §1.4 に 1 行明文化のみ |
| R-P2-12 | low | R-P2-3 のユニットに同梱 |
| R-P2-13 | low | 016/023 レビュー時のチェック項目に(実装変更は最小) |
| R-P2-14 | low | P3 E2E の測定項目に 1 行 |
| R-P2-7 | info | CURRENT.md 継続事項から削除 |
