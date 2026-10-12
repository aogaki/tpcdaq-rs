# 121 — slowctl の運用仕上げ: 起動スクリプト / ログブックに定期値を書かない / HV ch0 の UNV を alarm から外す

**Status: IN PROGRESS**(2026-10-12 起票 Opus。118 の残り 1〜3 へのユーザー回答に基づく)

## ユーザー回答(2026-10-12)
1. 「進めてください。設定としてデバイスファイルを指す方がいいですか? それとも自動検出?」
   → Opus の推奨: 今のまま設定で by-id を指す(挿し直しや口を変えても不変、open 時にシリアル照合、自動検出は同型 2 台や類似機器で曖昧になる)。
2. 「ログへは start, stop 時の電圧情報だけで大丈夫です。ほかは InfluxDB なんかに突っ込んで、解析時に見るくらいにしましょう。」
3. 「alarm から外すのは安全にできるならしてください。絶えずおかしいと出てるだけなら問題ないです。というかユーザーがおかしいことに気づくべき。」

## A. daq_up / daq_down に slowctl(Opus が直接。シェル数行)
- daq_up: 設定に `[slowctl]` があれば controller の後に `start slowctl "$BIN/slowctl" --config "$CONF"` を実行し、`slowctl: device identified` を待たない。
  起動ログの `status PUB bound` を待つ。機器が無くても slowctl は再試行で生きるので、起動の失敗にはしない。
- daq_down: ORDER の先頭に slowctl を足す(SIGINT)。
- `config/daq_example.toml` に `[slowctl]`(hivolta / hmp2020 の by-id 例、`expect_serial`、`poll_s`)と `[monitor] slow_sub` の例。NEW_DAQ_SETUP に 1 節。

## B. ログブックに定期値(VALUE)を書かない(Sonnet)
- 今: slowctl は VALUE(2 % 変化か `log_every_s` ごと)/ IDENT / ALARM / RECOVER / ERROR を controller の LogPost PULL に送る。
  controller は `apply_slow_record` で直近値(`slow_latest`)を更新してから、**すべて**をログブックに書く。
  - 実機: 1 日足らずで slow 2,180 件(VALUE 2,121)。
- 変更: controller の LogPost 受信で、**`Slow { event: VALUE }` は `slow_latest` の更新だけ行い、ログブックには書かない**。
  IDENT / ALARM / RECOVER / ERROR は今どおり書く(まれで、安全上の意味がある)。
  run_start / run_stop の `slow` スナップショット(全デバイスの直近値、値ごとの ts 付き)は今どおり = 「start / stop 時の電圧情報」。
- カウンタ: 書かなかった VALUE の件数を数える(例 `slow_values_not_logged`)。controller の status か log の counters に出す(silent にしない)。既存の流儀に合わせる。
- テスト: VALUE を投稿 → ログブックに行が増えない・`slow_latest` は更新される・次の run_start の `slow` にその値が載る。ALARM はログブックに書かれる。
- SPEC §9.2 の slow の行(v1.31)への 1 文の追記は Opus が下書きし、Fable が確定する。
- 時系列の保存先(InfluxDB / 日ごとのファイル)は C で別に決める。B はそれを待たない。

## C. 時系列の保存先(ユーザーと決める → 別票)
- 候補: (i) slowctl が日ごとのファイル(CSV / JSONL)に全サンプルを書く(インフラ不要、解析は pandas / ROOT)。
  (ii) InfluxDB(+ Grafana で閲覧)。ELITPC への導入(sudo)・保持期間・書き込みが失敗したときの扱いが要る。
- どちらでも、読み出しのループを書き込み先が詰まって止めない(有界キュー、落とした数を数える)。

## D. HV の STATUS ビットを ch ごとに alarm から外す(Sonnet)
- `[[slowctl.device]]`(hivolta)に任意キー `status_ignore = [{ ch = 0, flags = ["UNV"] }]`。
- **外せるのは電圧のずれの警告 `UNV` / `OVV` だけ**。それ以外(OVC / TRIP / OVP / OVT / KILL / INTLK / FAIL / LOCK / TWN / DIS / UP / DOWN / ON)を書いたら設定検証でエラー
  = 安全側。ch は 0–7、重複は可。
- driver の alarm 判定で、その ch のその bit だけを除く。STATUS の値(ビットマスク)はそのまま出す
  → UI の Status 欄には `ON UNV` が出続ける(ユーザーが気づける)。
- テスト:
  - 実機ゴールデン(ch0 STATUS 33)で、`status_ignore` なし → ch0 alarm、ch0 UNV を外す → ch0 alarm なし・status 値は 33 のまま。
  - ch0 が 33 + TRIP(64)なら外していても alarm。
  - `flags = ["TRIP"]` は設定エラー。

## 進捗(2026-10-12)
- **A**(Opus `8f8a804`)
  - daq_up: `[slowctl]` があれば controller の後に slowctl を起動し、`status PUB bound` を待つ。
  - daq_down: ORDER の先頭と取り残しの掃除に slowctl を加えた。
  - `config/daq_example.toml` に HV / LV の by-id 例、NEW_DAQ_SETUP を v0.6 に。
  - `bash -n` は通過。**実機のスタック上げ直しでの確認は run 69 の停止待ち**。
- **B**(Sonnet `ce3c591` → merge `95c7168`)
  - LogPost の `Slow{VALUE}` は `slow_latest` を更新するだけで、ログブックには書かない。
  - `/api/status` の `log_posts.slow_values_not_logged` で件数を数える。
  - テストは red(VALUE が 3 行書かれる)→ green。
  - **ELITPC への反映(controller の入れ替え)は run 69 の停止待ち**。
- **D**(Sonnet `2e3e743` → merge `c3714cd`)
  - `status_ignore`(UNV / OVV だけ)で、alarm の判定からそのビットを除く。STATUS の値は変えない。
  - **ELITPC に反映済み(12:41)**: `status_ignore = [{ ch = 0, flags = ["UNV"] }]`。ch0 は status 33 のまま VALUE として出て、ALARM は出ない。
- ゲート(main `95c7168`、Mac、TIME_WAIT 10): fmt / clippy 0 / **cargo test 907 passed / 0 failed / 1 ignored**。
  D のレーンで receiver_integration が `AddrNotAvailable` で落ちたのは、並列のテストによるポート枯渇だった(main で再現しない)。
- C → 122(InfluxDB、ユーザー裁定)。
