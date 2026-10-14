# tools/scan — パラメータスキャンのドライバ(TODO/135)

設定点の列を回して run を取る。Python 3 標準ライブラリのみ(ELITPC でそのまま動く)。

- 使い方: `python3 scan.py examples/trigger_delay.json --url http://127.0.0.1:8080 --out-dir ./out`
  - `--dry-run` = PUT 本文と手順を表示するだけ(何も送らない)/ `--resume out/scan_<name>.json` = ok=true の点を飛ばす
  - passphrase は `--passphrase-file`(既定 `/opt/tpcdaq/daq.toml` の `passphrase = "…"`)。ログ・JSON には出さない
- 各点: Idle 確認 → `PUT /api/params`(base の knobs を点の knobs で上書き)→ `run/start` → status を 2 s ごとに読む →
  receiver `frames` ≥ `stop.events` または経過 ≥ `stop.max_s` で `run/stop`。最後に **base を PUT し直す**(失敗は大きく表示)。
- 異常停止(run を stop し base に戻して exit 2): PUT/start/stop が 2xx でない、phase が Idle でない、decoder `malformed > 0`、
  `acquisition_restarts > 0`、`late_fragments > 0`、frames が 30 s 伸びない。出力は `scan_<name>.json`(1 点ごとに書き直す)。
- **HV / LV / power 系の API は一切呼ばない**(SPEC §11)。振るのは GET のパラメータだけ。
- テスト: `cd tools/scan && python3 -m unittest`
