# 135 — パラメータスキャンのドライバ(設定列 → PUT /api/params → run → 待つ → stop → 記録)

**Status: COMPLETED(2026-10-14、結果は末尾)** — 旧: READY(2026-10-14 起票 Fable。ユーザー「パラメータを変えながら run を取って解析したい。Parameters 画面ができたので AI でガンガンやりたい」。
triggerDelay は「目視の範疇だが、最適値の自動化の布石としてアリ」= 最初の試運転に使う。本命は peakingTime / gain / 部分読み出し閾値。Sonnet、1 日)

## 0. 前提(実装済みのもの — 新しいサーバ機能は作らない)
- `PUT /api/params {token, params}`(SPEC §3.3 / §8.1、Running 中は 409)。params JSON の形は `tests/fixtures/params/params_roundtrip.json`(schema `tpcdaq-params/1`、`template`、`knobs` 12 個、`exceptions`、`note`)。
  ELITPC の現用は `/srv/tpcdaq/params/current.json`(template `configure-elinp.xcfg`、25 MHz、triggerDelay 1500、readout partial)。
- `POST /api/control/acquire {operator, passphrase}` → token、`POST /api/run/start {token, comment}` → `{run, phase, params_id, notes}`、`POST /api/run/stop {token}`、`POST /api/control/release {token}`。
- `GET /api/status`: `phase`、`run.run`、`components[].metrics`。mini(1 AsAd)では **receiver の `frames` = イベント数**(1 フレーム = 1 イベント)。decoder の `malformed` / `acquisition_restarts`、`notes`。
- 記録は自動: 台帳の `run_start.params = {id, sha256, exceptions}`、`run_stop.counters`。スキャンは**自己記録される**ので、ドライバの JSON は「run 番号 ↔ 設定点」の索引でよい。
- 安全境界(SPEC §11): 振るのは GET のパラメータだけ。**HV / LV / ガスには触らない**(ドライバに power 系の API を呼ぶコードを書かない)。

## 1. 作るもの — `tools/scan/scan.py`(Python 3 標準ライブラリのみ、ELITPC で動かす。`requests` 不可 — `urllib`)
- 入力: スキャン定義 YAML か JSON(**JSON にする** — 標準ライブラリで足りる。例 `tools/scan/examples/trigger_delay.json`):
  ```json
  {"name": "tdelay_2026-10-14", "base": "/srv/tpcdaq/params/current.json",
   "stop": {"events": 1500, "max_s": 600},
   "points": [{"knobs": {"triggerDelay": 1000}}, {"knobs": {"triggerDelay": 1250}}, {"knobs": {"triggerDelay": 1500}},
              {"knobs": {"triggerDelay": 1750}}, {"knobs": {"triggerDelay": 2000}}]}
  ```
  `base` の params JSON を読み、各点の `knobs` を**上書き**(無いキーはそのまま)、`note` を `scan <name> point <i>: <knobs の差分>` にする。`exceptions` は base のまま。
- 流れ(各点): 相が Idle であることを確認 → `PUT /api/params` → `run/start`(comment = note と同じ)→ `GET /api/status` を 2 s ごとに読み、receiver `frames` ≥ `stop.events` **または** 経過 ≥ `stop.max_s` で `run/stop` → 応答と `run_stop` 後の status を記録 → 次の点。
  最後に **base の params を PUT し直す**(現用設定に戻す — 必ず。失敗したら大きく表示)。
- 止まる条件(silent にしない): PUT / start / stop が 2xx でない、`notes` が空でない以外の警告、decoder `malformed > 0`、`acquisition_restarts > 0`、frames が 30 s 伸びない(レート 0)、`late_fragments` が status に出ていれば > 0。止まったら理由を出し、**run を stop して base に戻してから** exit 2。
- token: 開始時に 1 回 acquire(`operator` は引数、既定 `scan`)、終了時に release。passphrase は `--passphrase-file`(既定 `/opt/tpcdaq/daq.toml` の `passphrase = "…"` を読む — 既存のスクリプトと同じ)。**passphrase をログ・JSON に書かない**。
- 出力: `<out_dir>/scan_<name>.json` — `{name, started, base_sha256, points: [{i, knobs, params_id, run, started_ts, stopped_ts, frames, elapsed_s, stop_reply, status_at_stop(counters 抜粋), ok}]}`。1 点終わるごとに書き直す(途中で落ちても残る)。stdout に 1 点 1 行の進捗。
- `--dry-run`: PUT の本文と手順を表示して何も送らない。`--resume <json>`: 済んだ点(ok=true)を飛ばす。

## 2. テスト(TDD、`tools/scan/test_scan.py`、`python3 -m unittest`。実サーバ不要)
- 純関数: `merge_point(base, knobs)`(上書き・無いキー保持・note 生成)、`should_stop(status, started_at, now, stop)`、`abort_reason(status, prev_frames, ...)`(malformed / restarts / 停滞)、`read_passphrase(toml_text)`。
- 擬似サーバ(`http.server` をスレッドで): acquire / params / start / status(呼ぶたびに frames が増える)/ stop / release を返すフェイクで、5 点を回して JSON の形と「最後に base を PUT し直す」を assert。異常系 1 本(3 点目で malformed=1 → stop して base に戻して exit 2)。
- 名前: `test_merge_point_overrides_only_the_given_knobs`、`test_should_stop_on_events_or_elapsed`、`test_abort_on_malformed_or_restart_or_stall`、`test_passphrase_is_read_from_toml_and_never_logged`、`test_five_point_scan_records_each_run_and_restores_base`、`test_abort_mid_scan_stops_the_run_and_restores_base`。

## 3. 受け入れ
- フェイクで 5 点 green。`--dry-run` を ELITPC の現用 current.json に対して実行し、PUT 本文が現用と triggerDelay だけ違うこと(主対話が確認)。
- README(`tools/scan/README.md`、10 行): 使い方、止まる条件、HV には触らない旨。

## 4. 報告
変更ファイル、テスト名、赤の実測、`python3 -m unittest` の総数、`--dry-run` の出力例、**実行できなかった操作**(無ければ「なし」)。

## 結果(2026-10-14、Sonnet `d0026e2` 系 → merge、試運転 = triggerDelay 5 点、ELITPC、Fable 照合)
- 実装: `tools/scan/scan.py`(標準ライブラリのみ)、`test_scan.py` 8 本(発注 6 + resume / dry-run)、`examples/trigger_delay.json`、README。呼ぶ API は status / params / run start・stop / control acquire・release だけ(power 系なし — diff で確認)。
- 逸脱(承認): 「notes が空でない」は abort 条件にせず `start_notes` として記録(params 由来の note が毎回出るため)。
- dry-run(ELITPC 現用 current.json、25 MHz / delay 1500): PUT 本文の差は `triggerDelay` と `note` だけ。
- **試運転(ユーザー了承「試運転してくれていいですよ」、12:23–12:52 EEST、α 線源・外部トリガ・HV ON のまま無人)**: 5 点 × 1,500 events、全点 ok、最後に base(`elinp__dcdaf9ca`)へ復元、相 Idle。

| 点 | triggerDelay | run | params id | events | 所要 |
|---|---|---|---|---|---|
| 0 | 1000 | 78 | elinp__ac74447b | 1500 | 329 s |
| 1 | 1250 | 79 | elinp__0ae4216d | 1503 | ≈ 330 s |
| 2 | 1500(現用) | 80 | elinp__dcdaf9ca | 1508 | ≈ 330 s |
| 3 | 1750 | 81 | — | 1501 | 335 s |
| 4 | 2000 | 82 | — | 1506 | 337 s |

- レート ≈ 4.5 Hz(α、5 分 / 1,500)。malformed / restarts / late は全点 0。台帳の `run_start.params` に各点の id が残っている(索引は `~/test/scan/scan_tdelay_test.json`)。
- 教訓: 1 点 5 分で 30 分。解析(136)は run 1 本 3 s なので、点を増やすコストは取得だけ。

**Status: COMPLETED(2026-10-14)**
