# 031 — 負荷ハーネス(§12-5 連続 24 h / §12-6 瞬発 10 分 = R-P2-11、Warsaw 前必須)

**Status: OPEN(030 完了後に発注 — モニタ経路込みで測る意味があるため = R-P2-11 修正案)**
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
