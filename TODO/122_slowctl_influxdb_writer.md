# 122 — slowctl の全サンプルを InfluxDB に書く(時系列の保存先)

**Status: IN PROGRESS**(2026-10-12 起票 Opus。ユーザー「保存先は InfluxDB にしましょう。テキストファイルはバカデカくなるのでつらい」。Opus implementer)

## 背景
- 121 B で、slowctl の定期値(VALUE)はログブックに書かなくなった。ログブックに残るのは run_start / run_stop の `slow` スナップショットと、IDENT / ALARM / RECOVER / ERROR だけ。
- 全サンプルは解析用に InfluxDB へ保存する(ユーザー裁定)。
- 量の見積もり: HV は 5 s ごとに 42 値、LV は 1 s ごとに 10 値 → ≈ 18 値/s ≈ 160 万値/日。InfluxDB は圧縮が効くので、数十 MB/日の規模。
- ELITPC には InfluxDB **2.9.x**(OSS、`influxdb2` パッケージ、:8086、Data Explorer の UI 付き)をユーザーが入れる(sudo)。

## 仕様
1. **書き込みは InfluxDB v2 HTTP API**: `POST {url}/api/v2/write?org={org}&bucket={bucket}&precision=ms`、ヘッダは `Authorization: Token {token}`、本文は line protocol。
   - InfluxDB 3.x もこの API を受け付けるので、将来の版の違いに縛られない。
2. **line protocol の形**(1 回の poll = 機器・ch ごとに 1 行):
   - measurement = 機器名(`hv` / `lv` / …)。
   - tag `ch` = node の番号(node が無い = 機器全体のサンプルは `ch=board`)。
   - fields = param 名ごとに float(`vmon=341.08,imon=9.585,status=33,vset=350,iset=15`)と、`alarm=true|false`(その行のどれかが alarm なら true)。
   - timestamp = サンプルの `ts_ms`。同じ (device, node, ts_ms) のサンプルを 1 行にまとめる。
   - 単位は列に持たない(param ごとに固定。SPEC に表で書く)。
   - line protocol のエスケープ(measurement / tag のカンマ・空白・等号)は規則どおりに行う。
   - IDENT の文字列(fw / model / serial)は書かない(ログブックに残っている)。
3. **書き込みスレッドを読み出しループから切り離す**: device_loop → **有界チャネル** → writer スレッド(1 本)。
   - チャネルが満杯なら `try_send` で落とし、`influx_points_dropped` を数える。読み出しは決して止めない(モニタ系の規約: 落としてよいが silent にしない)。
   - writer は `batch_ms`(既定 1000)ごと、または一定数たまったら POST する。
   - 失敗(接続不可・HTTP 4xx/5xx・タイムアウト)したら、そのバッチを有界の再送バッファに戻して指数バックオフで再試行する。
     上限(`buffer_max_points`、既定 100,000 ≈ 1.5 h 分)を超えたら古い方から捨てて数える。
   - 4xx(認証エラー、bucket が無いなど)は再試行しても直らないので、`error!` を出し、間隔を空けて再試行する(設定ミスを可視化する。silent にしない)。
4. **設定** `[slowctl.influx]`(任意。無ければ書かない = 今と同じ):

   | キー | 必須 / 既定 |
   |---|---|
   | `url` | 必須(例 `http://127.0.0.1:8086`) |
   | `org` | 必須 |
   | `bucket` | 必須 |
   | `token_file` | 必須。トークンは TOML に書かない。ファイルの中身は 1 行(前後の空白は除く) |
   | `batch_ms` | 1000 |
   | `buffer_max_points` | 100000 |
   | `timeout_ms` | 2000 |

   - 検証: url は `http://` か `https://` で始まること。token_file は起動時に読めること(読めなければ起動エラー)。
5. **可視化**: slowctl の status PUB の counters に `influx_points_written` / `influx_points_dropped` / `influx_write_errors` / `influx_last_error`(文字列、null 可)を足す。
   UI の Slow control 画面の Counters 表は、counters を汎用に出しているならそのまま出る(出ないなら報告)。
6. **HTTP クライアント**: 依存を 1 つ足してよい(例 `ureq` = ブロッキングで小さい。slowctl はスレッド型なので合う)。
   TLS は不要(localhost)なので、可能なら TLS の feature を切って依存を小さくする。選んだ理由を報告すること。

## テスト(TDD)
- line protocol の組み立て(純関数): hv の実機サンプル(ch0 status 33 で alarm、ch1 は alarm なし、ボード)と lv(ch1 / ch2)から、期待の行と完全一致すること。
  エスケープ(空白・カンマを含む機器名)と、同じ ts のまとめ方も確かめる。
- writer: ローカルの偽 HTTP サーバ(`TcpListener` で受けて記録する)で次を確かめる。
  - パスとクエリ(org / bucket / precision=ms)・`Authorization` ヘッダ・本文。
  - サーバが 500 を返す → 再試行 → 成功で、points_written が正しい。
  - サーバ不在 → 再送バッファが上限で古い方から落ち、dropped が数えられる。
  - チャネル満杯 → dropped。
- 設定: `[slowctl.influx]` が無い / 必須キーが欠ける / token_file が無い / url が不正。

## ゲート
`cargo fmt && cargo clippy --tests -- -D warnings && cargo clippy --tests --features slowctl-fake -- -D warnings && cargo test`
(controller_integration は TIME_WAIT を大量に作るので、連続で回さない)。

## 実機(Opus)
ユーザーが influxdb2 を入れ、org / bucket / 書き込み専用トークンを作ったら、`/opt/tpcdaq/daq.toml` に `[slowctl.influx]` を足して slowctl を再起動する。
Data Explorer で hv / lv の系列が見えること、counters の written が増えること、InfluxDB を止めても slowctl の読み出しが止まらず dropped / errors に出ることを確かめる。

## SPEC(Opus 下書き → Fable 確定)
§3.1 に `[slowctl.influx]`、§2.3 か §9 に line protocol の形(measurement / tag / field と単位の表)、§11 の slowctl の行に「全サンプルは InfluxDB」を追記する。
