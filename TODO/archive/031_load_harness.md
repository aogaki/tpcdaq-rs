# 031 — 負荷ハーネス(§12-5 連続 24 h / §12-6 瞬発 10 分 = R-P2-11、Warsaw 前必須)

**Status: COMPLETED**(2026-08-16 — 一晩 soak 合格。§12-5 フルレート持続 + §12-6 は 054 に移管)
**仕様**: SPEC **v1.11** §12-5(100 Hz 相当ペース 24 h、保存系 drop 0、RSS 後半 12 h
単調増加なし、「書いて検証して消す」可)/ §12-6(全速 ≥3× 相当 10 分、drop 0)/
§12 末尾(graw_replay ペーシング)/ §1.4(ロスレス系の背圧)
**依存**: 030(controller 駆動の全通し配線を流用)/ 005(graw_replay — 本ユニットで拡張)/
013(2 ソース飢餓の修正 — 本ハーネスが踏み直す)
**発注先想定**: implementer/**Opus**(長時間オーケストレーション + 判定基準の工学判断)

## 前提の設計判断(起票時に確定 — 変更したくなったら報告)

- **`--loop` そのままでは §12-5 を満たせない**: 005 の `--loop` は同一バイトを繰り返すため
  eventIdx が周回で重複し、イベントビルダが 2 周目以降を duplicate/late 扱いにする =
  「全カウンタ 0」が原理的に不成立。また 24 h 単一 run は run.root が TB 級で不可。
- よって方式は **「中規模 run を back-to-back で 24 h 反復」**:
  1 run = mini 実 graw を **N 周、lap 毎に data フレームの eventIdx へ
  +lap×(max_idx+1) を加算して送出**(合成入力の事前生成はしない — ワイヤ上で書き換え)。
  run 終了(EOF=EOS)→ 検証 → 出力削除 → 次 run。run 境界は §12-8 跨 run(028)で
  無停滞を実証済みの正規経路。
- eventTime は書き換えない(ビルダは eventIdx のみ — §6.3。decoder/root-sink に
  eventTime 依存の実行時チェックが無いことは起票時に確認済み)。
- 上限の裁量既定: **RSS 上限 = 4 GiB/プロセス**(引数で調整可)。単調性判定 =
  後半 12 h の最終 1 h 平均 ≤ 後半開始 1 h 平均 +5%(判定式は結果節に明記の上、
  より妥当な式があれば理由つきで置換可)。

## やること

1. **graw_replay 拡張(TDD — 所有権: src/bin/graw_replay.rs + tests/graw_replay_tool.rs)**:
   - `--laps N`: N 周で終了。lap ≥ 1 の data フレーム(frameType 1/2)は eventIdx
     フィールド(ヘッダ 22..26)へ **+lap×(max_idx+1)** を加算して送出(max_idx は
     1 周目に実測)。制御フレームは無変更で毎周送出。既存 `--loop` /
     単発・複数ファイルマージの挙動は不変(既存テスト無改変)。
   - `--laps-until-s S`: S 秒経過後、**現 lap を完走してから**終了(フレーム境界を
     切らない — 途中切断は framer リセット = malformed でハーネスの「全カウンタ 0」を
     壊すため)。
   - テスト: lap 跨ぎで eventIdx が単調連続 / 制御フレーム不変 / lap 内バイト
     (eventIdx 4 B 以外)不変 / until-s が lap 境界で止まる。
2. **ハーネス本体 `src/bin/soak_harness.rs`(新規)**: 030 E2E-C と同じ controller 駆動
   全通し配線(fake-ECC + bridge + 全コンポーネント + monitor + WS probe クライアント)を
   子プロセス起動し、run を反復:
   - 引数: `--mode soak|burst` / `--duration-h`(soak 既定 24)/ `--burst-min`(既定 10)/
     `--rate-mbps`(soak 既定 224 = 100 Hz 相当。burst は 0 = 全速)/ `--run-minutes`
     (1 run の長さ、既定 10 — `--laps-until-s` に変換)/ `--rss-limit-mib`(既定 4096)/
     `--keep-outputs`(既定 off)。
   - **run 毎検証(「書いて検証して消す」)**: 全ロスレスカウンタ 0(receiver / decoder /
     graw-writer の GetStatus + root-sink 終了 JSON: overflow・gap・malformed・late・
     incomplete・duplicate・unexpected・framer_resets)/ graw 出力合計バイト =
     laps × 30,108,684 B / root entries = laps×108 = events_complete / monitor.root 存在
     → 合格なら出力削除。モニタ系 drop(publish_drops / wsDropped)は**不合格にしない**が
     全て記録(絶対ルール: 落としてよいが silent にしない)。バイトレベル全照合は不要
     (§12-2/3 で実証済み — ここは耐久が主眼)。
   - **メトリクス JSONL(逐次 flush)**: 10 s 毎に ts / 各プロセス RSS(`ps -o rss=`)/
     カウンタスナップショット / run 通番。クラッシュしても記録が残る形。
   - **失敗時は即停止し、当該 run の出力・ログを削除せず保全**(証拠第一)。
   - 起動時チェック: 空きディスク < 2 run 分(概算 ~100 GiB、mini 実測比 root ≈ 1.53×
     入力)なら拒否(silent 失敗禁止)。終了時に要約(run 数 / 総バイト / RSS 推移判定 /
     全カウンタ集計)を stdout + JSONL 末尾へ。
3. **CI 用スモーク `tests/soak_smoke.rs`(env gate: 030 と同じ一式)**: `--duration-h` を
   分単位相当まで下げ(引数追加可)、**2 run × 短 lap** でハーネス自体の回帰
   (検証・削除・JSONL・要約が機能する)を green に保つ。24 h 実走を CI に入れない。
4. **実測(本チケットの出口)**:
   - **§12-6 burst**: `--mode burst` 10 分・全速(offered ≥3×。達成スループットも記録)→
     **drop 0**。
   - **§12-5 soak**: `--mode soak` 24 h @ 224 Mbps → **全 run 全カウンタ 0 + RSS 判定
     合格**。実行タイミング(マシン占有 24 h + caffeinate 等)は**ユーザーと合意して
     開始**。結果(メトリクス要約・RSS グラフ化は任意)を結果節に記録するまで
     COMPLETED にしない。
   - 任意(推奨、時間があれば): 2 ソース合成(012 の coboIdx 書き換えヘルパの流儀)で
     burst 10 分 — 013 飢餓修正の負荷下回帰。実施可否と結果を記録。

## 受け入れ

- ファイル所有権: src/bin/graw_replay.rs(拡張)/ src/bin/soak_harness.rs(新規)/
  tests/graw_replay_tool.rs(追記)/ tests/soak_smoke.rs(新規)/ lib.rs・Cargo.toml は
  原則不変(新依存なし — JSONL は serde_json、RSS は ps)。**他コンポーネントの
  production 変更禁止**(ハーネスで見つかった不具合は停止して報告)。
- `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test` green
  (既存テスト無改変)。スモーク green(env あり)。
- 結果節: §12-5 / §12-6 それぞれに ✔/✘ + 実測(run 数・総転送量・全カウンタ・RSS
  推移と判定式・burst 達成スループット・実行環境と日付)。

---

## 追記(2026-08-15 Fable — 044 レビューからの移管)

- **E2E ハーネス共通化を本ユニットに織り込む**: p3_e2e.rs ↔ p3_error_paths.rs に
  **834 行の逐語一致**(Proc / http / E2eEnv / free_port / Sink / Topology / scratch_dir /
  wait_for_file / read_logbook 等 — comm -12 実測)。本ユニットが 3 番目の利用者になるため、
  ここで `tests/common/` へ抽出してから負荷ハーネスを書く(rule of three)。
  注意: Sink/SinkLog は 2 ファイルで型が違う(スーパーセット署名で吸収、無理に潰さない)/
  http は controller_* 側と意味差あり(read timeout・パース失敗時の型)— 同一ペアのみ共有。
  受け入れ = 既存 E2E の assert・件数不変で全 green(044 の「テスト資産」条件)。
- 負荷ソースは **vcobo-daq の全速モード**(040)が使える(グラフ化・計測の要件は本文どおり)。

---

## 追記 2(2026-08-15 Fable — SPEC v1.15 のユーザー裁定を反映。本追記が本文に優先)

- **§12-5 は二段化された(v1.15)**: 自宅ソフト soak = **一晩(≥8 h、既定 12 h)の
  トレンド駆動**。フル 24 h はハード込みで ELI-NP に移管。
- 本文の「24 h」パラメタは次のとおり読み替え: `--duration-h` **既定 12**(引数で可変)。
  RSS 単調性判定は「後半 12 h」→ **「走行後半の半分」**(12 h 走なら後半 6 h)で同式。
- **メトリクスのサンプリングを一級市民に**: RSS / open fd 数 / 全ロスレスカウンタ /
  モニタ系 drop を **1 分毎に CSV へ記録**(トレンド判定の一次データ。ハーネスの合否判定は
  このファイルから機械的に出す — 最終レポートに全系列の始値/終値/傾きを載せる)。
- 実行の段取り(発注範囲): ハーネス実装 + §12-6 バースト 10 分の実測 + **soak の
  30 分サニティ走行**(判定パイプライン一式が動くことの実証)まで。**一晩の本番走行は
  発注側(主対話)がレビュー後に detach 起動**する(マシン占有はユーザー合意済み 2026-08-15)。

---

## 追記 3(2026-08-15 Fable — ハーネス完成、本走行は 053 待ち)

- **ハーネス実装は完了・レビュー PASS**(E2E 共通化 tests/common/mod.rs 553 行(各ファイル
  −480/−468)/ graw_replay --laps(eventIdx 書き換え)/ soak_harness + 1 分毎 51 列 CSV +
  report 自動判定 / cargo **447 passed**)。逸脱は全て受理(WS probe 自前 90 行 = 新依存
  回避 / run 毎カウンタは stop 前採取 / --runs 追加 / ROOT 1 GiB パート分割の発見と総和照合)。
- **サニティが実欠陥 2 件を捕獲**(soak の存在意義がそのまま実証された):
  root-sink の RSS +0.55 MB/event 成長 & 天井 ≈30 events/s → **TODO/053 起票・修正待ち**。
  §12-6 の「全速」定義は v1.16 で明示レート形(≥3× = 672 Mbps)に改訂。
- **一晩の本番走行(§12-5 v1.15)は 053 完了後に起動**(detach コマンドは実装報告③のとおり。
  SIGINT graceful は 053-D で追加される)。本チケットは**その走行の合格をもって完了**。

---

## 結果(2026-08-16 — 一晩本番走行の判定。ハーネス実装自体の結果は追記 3 参照)

### 実行

- コマンド: `nohup caffeinate -i ./reference/_spike/soak_bin/soak_harness --mode soak
  --duration-h 12 --run-minutes 10 --rate-mbps 45 --metrics-interval-s 60
  --out-dir ~/soak_0815`(053 修正込みスナップショットバイナリ、detach 起動)
- 走行: 2026-08-15 20:16 〜 08-16 04:40 EDT。ユーザーの朝の移動予定のため 04:30 に
  SIGINT で graceful 打ち切り(現 run 完走 → report 生成)。**実走 30,244.7 s = 8.40 h ≥ 8 h**
  (§12-5 v1.15 の下限を満たす)。環境: macOS(Darwin 25.5.0)、loopback、
  mini 実 .graw(30,108,684 B/lap)。
- レート 45 Mbps の理由: root-sink 天井 ≈32 events/s(053 実測、修正は 054)があるため、
  持続可能レート(≈20 events/s)でトレンド検証を主眼に走らせた(v1.15 の趣旨 =
  リーク・成長・ドリフトの検出)。224 Mbps は 30 分 ×2 run で別途実測(下記)。

### §12-5(a) 一晩ソフト soak — **✔ 合格**

- **run**: 50 run 全合格(back-to-back、書いて検証して消す)。全 run が
  laps=113 / graw=3,402,281,292 B / entries=12,204 / 604.9 s で**完全一致**(決定論)。
  総計 5,650 laps / 158.43 GiB 送出 / 610,200 events。達成 45.0 Mbps(指示値どおり)。
- **ロスレスカウンタ全 0(8.4 h 通し)**: recv overflow/framer_resets/abandoned/encode/
  send、dec malformed/seq_gaps/run_mismatches/batches_abandoned/eos_abandoned/
  cobo_mismatch、gw 全カウンタ、rs incomplete/late/pending = **全て 0**。
  root-sink 終了 JSON: entries_written=610,200(=50×12,204)/ duplicates=0 /
  items=84,978,892,800(=610,200×139,264 で厳密一致)/ items_out_of_range=0。
- **RSS 単調性(v1.15 式: 後半 [H/2,H]、先頭窓×1.05 ≥ 末尾窓、窓=H/12)**:
  全 8 プロセス **OK**。root_sink は後半先頭窓 530,598 KiB → 末尾窓 530,654 KiB
  (+56 KiB/4.2 h — 053 リーク修正が長時間で実証された。修正前は +0.55 MB/event)。
- **fd**: root_sink 157→209 は起動後 60 s で 209 に達して以後 8.4 h 完全一定
  (プラトー、リークではない)。他プロセスは全て一定。
- **モニタ系 drop(silent にしない)**: mon_publish_drops=341,664(カウント済み・設計どおり)、
  monitor_gaps=0 / ws_dropped=0 / clients_dropped_slow=0。
- 証拠: `reference/_spike/soak_evidence_031/report_overnight_45mbps_8h4.txt` +
  `metrics_overnight_45mbps_8h4.csv`(505 サンプル × 51 列)。

### 224 Mbps(100 Hz 相当)30 分 — 参考実測(053 後)

2 run 合格・達成 223.2 Mbps・全ロスレスカウンタ 0・overflow 0。ただし root-sink /
decoder の RSS が run 中 3〜4.3 GiB まで膨張(天井超過分をキューが吸収し run 間で回復)。
**长時間の持続はできない形** — フルレート持続の受け入れは 054(≥100 events/s)へ。
証拠: `report_053after_224mbps.txt`。

### §12-6 瞬発負荷(v1.16: 672 Mbps × 10 分 drop 0)— **✘ 未達 → 054 に移管**

recv_overflow_frames=94,544(counted drop、silent でない = §1.4 は設計どおり)、
RSS root_sink 13.7 GiB / decoder 10.2 GiB。原因は root-sink 天井(053 で確定、
21 ms/event)。**054 完了後に再実測**(054 受け入れに追記)。
証拠: `report_053after_burst672.txt`。

### スキップ・残課題

- 任意項目「2 ソース合成 burst」: 未実施(root-sink 天井の解消が先 — 054 後に価値が出る)。
- §12-5(b) フル 24 h はハード込みで ELI-NP(v1.15 の仕様どおり、本チケットの範囲外)。
- テストスイート: cargo 448 passed(soak_smoke 含む、追記 3 時点から無変更)。
