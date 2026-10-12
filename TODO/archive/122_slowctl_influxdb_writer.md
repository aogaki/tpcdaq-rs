# 122 — slowctl の全サンプルを InfluxDB に書く(時系列の保存先)

**Status: COMPLETED**(2026-10-12 起票・同日完了。ユーザー「保存先は InfluxDB にしましょう。テキストファイルはバカデカくなるのでつらい」。Opus implementer、Fable 一括レビュー承認)

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

## 進捗(2026-10-12)
- 実装(Opus `3344b5b` + レビュー修正 `49e7e45` → merge `68c614e`):
  - `src/slowctl/influx.rs`: line protocol の組み立て、`InfluxSink`(try_send)、writer スレッド、再送バッファ(古い方から落として数える)、バックオフ 1 → 60 s、NaN / inf は数えて落とす、停止時は 1 回だけ書き出す。
  - `ureq` 3.3 を default-features なし(TLS / gzip なし)で使い、`proxy(None)`。
  - status PUB の counters に `influx_points_written` / `_dropped` / `influx_write_errors` / `influx_last_error`(`serde(default)` で旧版とも互換)。
  - 「点」の単位は Sample 1 個。
- Opus のレビューで直したもの:
  - (a) トークンは slowctl の起動時にだけ読む。`config::parse` では読まない。同じ daq.toml を読む controller / monitor が、監視系のトークンの不備で起動不能にならないように。
  - (b) `https://` は検証で拒否する(TLS なしのため)。
  - (c) UI の Counters 表に influx の 4 項目を出す。
- line protocol の実例(テストのオラクル): `hv,ch=0 vmon=341.06,imon=9.593,status=33,vset=350,iset=15,alarm=true 1760259600000` / `hv,ch=board bdalarm=0,bdilk=0,alarm=false …` / `lv,ch=1 volt=3.6,curr=0.997,outp=1,vset=3.6,iset=1.2,alarm=false …`
- ゲート(main `68c614e`、Mac): fmt / clippy 0 / **cargo test 929 passed / 0 failed / 1 ignored** / UI **325 passed / 5 skipped**、build OK。
- **実機は、ユーザーが influxdb2 を入れて org / bucket / トークンを用意するのを待っている**(12:13 時点で未導入)。

## 実機(2026-10-12 13:1x–13:20、ELITPC)
- InfluxDB の導入はユーザー(sudo)。スクリプト `~/influx_install.sh` / `~/influx_setup.sh` を Opus が用意した。
  - 1 回目の install が黙って失敗した。原因は、Opus が鍵の確認用に daq で落とした `/tmp/influxdata-archive.key` が残っていたこと。
    ELITPC は `fs.protected_regular = 2` なので、root でもその上書きを拒否する。`curl --silent` だったのでエラーが見えなかった。
    → 一時ディレクトリを `mktemp -d` にし、`curl -fsSL` で失敗が見えるように直して、2 回目で成功した。
  - 結果: **influxdb2 2.9.1-1**(CLI は influxdb2-cli 2.8.0-1)、`/health` pass。org `elitpc`、bucket `slowctl`(保持期間無期限)。
    書き込み専用トークンは `/opt/tpcdaq/influx.token`(89 bytes、600、daq 所有。画面には出していない)。
- 全バイナリを main `93cf2ef` で release ビルド(走っているプロセスは旧 inode のまま)。daq.toml(バックアップ `daq.toml.bak-2026-10-12-influx`)に `[slowctl.influx]` を追加し、10:19:34Z に slowctl だけ入れ替えた。
  - ログ: `slowctl: writing every sample to InfluxDB url="http://127.0.0.1:8086" org="elitpc" bucket="slowctl"`、hv / lv を識別。
  - `influx query`(直近 2 分の last()): hv ch0 imon 9.595 / iset 15 / alarm false(121 D の status_ignore が効いている)、lv ch1 curr 1.235 / iset 2.5 / alarm false。
- 残り: Data Explorer での目視(ユーザー)、Slow control 画面の Counters の `influx_points_written`、InfluxDB を止めたときの dropped / errors の確認(run 69 の後で)、SPEC(Fable)。
- **停止試験(13:21–13:23 EEST、ユーザーが `sudo systemctl stop influxdb; sleep 60; sudo systemctl start influxdb`)**:
  - slowctl は `cannot write to InfluxDB — retrying error=io: Connection refused` を、retry_in_s 1 / 2 / 4 / 8 / 16 / 32 と指数で伸ばしながら出した。
    その間も読み出しは続き、溜まった点は buffered = 10 → 572。
  - InfluxDB の復旧(10:22:38Z)後の再試行で、溜めた分が書き込まれた。
  - 10 s 窓ごとの点数(`aggregateWindow(10s, count)`、10:20:30–10:24:30Z):
    - hv ch0 vmon は停止中の窓を含めてすべて 2(5 s 周期)。
    - lv ch1 volt はすべて 10(1 s 周期)。
    - 例外は 10:21:10 の 7(10:21:03 のスタック上げ直しで slowctl を入れ替えた瞬間)と、問い合わせ時点で進行中だった最後の窓だけ。
  - → **60 s の停止で欠損なし**(再送バッファの上限 100,000 点 ≈ 1.5 h に対して 572 点)。

## 結果(まとめ)
- ゲート(Mac): `cargo fmt --check && cargo clippy --tests -- -D warnings && cargo clippy --tests --features slowctl-fake -- -D warnings && cargo test` と `cd ui && npm test -- --watch=false && npm run build`。
  - merge `68c614e` の時点で cargo **929 passed / 0 failed / 1 ignored**(ignored は既存の `tests/root_sink_monitor_pub.rs:1031`)、UI **325 passed / 5 skipped**(既存の conformance)。最終 main `651b734` で 931。
- 新規テスト:
  - influx.rs 単体 10 本(line protocol の実機例 / エスケープ / まとめ方 / v2 write への POST とトークン / 500 → 再試行 / 4xx / サーバ不在で古い方から落とす / チャネル満杯 / NaN / finish で書き出す)と、トークン読み込み 1 本。
  - config 7 本(+ レビュー後の https 拒否とトークンを読まない parse)、mod 1 本、統合 2 本(実バイナリ + 偽 InfluxDB / トークンが無いと slowctl だけが起動失敗)、UI `json.spec.ts` 1 本。
- 環境: Mac(テスト)と ELITPC(InfluxDB 2.9.1、2026-10-12)。実機の数値は上の「実機」節(InfluxDB の 60 s 停止で欠損なし、停止中の 572 点を復旧後に書き込み)。
- **未実施**: ユーザーによる Data Explorer と UI Counters の目視(移動のため後日)。
- SPEC **v1.41** §9.3(InfluxDB = モニタ系、正本は §9.2 のスナップショット)、§2.3 counters、§3.1 `[slowctl.influx]`(Fable 確定、`9e0dc43`)。
