# 171 — Slow control 画面の age が巨大になる(長時間ほかのページを見た後、リロードで直る)

**Status: DRAFT(優先度低)**(2026-10-22 起票 Opus。ユーザー「slow control の age ですが、たまにとんでもない値になっています。ただ更新だけはされている」
「長時間違うページを見てると起こるのでアクティブなタブとの関係とかでしょうか」「ページリロードすれば直るのでどうでもいいことではある」)

## 0. 症状
- Slow control の Latest 表(機器 × ch)の age が、ときどき非常に大きくなる。値そのものは更新されて見える。
- 長時間ほかのページ(タブ)を見ていた後に起きる。**ページのリロードで直る**。

## 1. 調べたこと(2026-10-22、読み出しのみ)
- age の計算は UI: `ui/src/app/slow/slow-table.ts:116` = 行の全パラメータについて `max(now_browser − sample.ts_ms)`。`slow-history.ts:86` も同じ式。
- **時計ではない**: ELITPC は NTP 同期(chronyc: 0.0002 s)、Mac との差 ≈ 0.18 s(rtt 0.47 s 以内)。
- **ドライバではない**: HiVolta(`hivolta_driver.rs`)も HMP2020(`hmp_driver.rs:208`)も、1 周期で全パラメータを読み、同じ `ts_ms = now_ms()` を付ける。
- **slowctl の間引きではない**: PUB の中身は `Aggregator::latest`(毎 ingest で更新)。ログブックの間引き(`logged`)とは別。
- **monitor のキューではない**: status(slow を同梱、1 Hz)はクライアントごとの `mpsc::channel(JSON_QUEUE = 32)` に `try_send`。満杯ならクライアントを**切断**する(`hub.rs:227`)ので、最大 ≈ 32 s しか溜まらない。今日の monitor.log に切断は 0 件。
- `slow-view.ts` の `ageTimer` は `ngOnDestroy` で止めている(タイマーの漏れは無い)。
- `/api/status`(controller)の `slow` は null(10-19 の気づきと同じ、別件)。UI は monitor の WS status の `slow` を使っているので、この画面には無関係。

## 2. 残った候補(未検証)
- **配送の遅れ**: WS の下(TCP の送受信バッファ、ブラウザ側の受信・処理)に古い status が溜まり、順に遅れて届く。
  リロード = WS の張り直しで溜まりが捨てられる、と整合。裏のタブでの処理の間引きや、重い live(uvw / track / histos / waveforms)の詰まりが引き金の可能性。

## 3. 直し方の案(着手は未定 — ユーザーの判断待ち)
1. **age の定義を変える**: age = `slow.ts_ms`(slowctl の配信時刻)− `sample.ts_ms`(機器で読んだ時刻)。どちらも ELITPC の時計なので、配送の遅れとブラウザの時計に依らない = 「値を取るたびに 0」。
2. **配送の遅れを別に出す**: `now − slow.ts_ms` を slow ページの見出しに「受信遅れ」として表示し、5 s 超で強調(silent にしない)。次に起きたら原因の確認になる(大きければ配送の遅れ、リロードを促す)。
3. (2 で配送の遅れと確認できたら)タブが隠れている間(`visibilitychange`)は重い live の購読を止め、status だけ受ける。
- 1 + 2 は UI の純ロジック(`slow-table.ts` / `slow-history.ts`)+ spec で縛れる → Sonnet implementer 向き。
