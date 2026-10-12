# 123 — 最新の 1 イベントを JSON で返す web API(analysis team の要望)

**Status: COMPLETED**(2026-10-12 起票・同日完了 Opus。ユーザー「analysis team からのリクエストで、last event だけを返す web API 作れますか? JSON 形式で graw ファイルを数値化した 1 イベントで OK です」。Sonnet)

## 方針
- **monitor**(port 9000、axum。今は WS の `/` と `/ws` だけ)に `GET /api/last-event` を足す。
- monitor は root-sink の PUB から `BuiltEventPayload`(`kind = "built_event"`、`event_publish_hz` = 20 で間引き済み)を受け取っている。
  → **受け取るたびに最新の 1 件を保持する**(`Arc` の差し替えだけ。WS の購読の有無に関係なく保持する。0x03 は購読者がいるときしか作らないので、それには頼らない)。
- JSON への変換は**要求が来たときだけ**行う(取得系にも monitor の通常経路にも負荷をかけない。モニタ系)。
- 中身は graw と同じ電子回路の座標・生 ADC(FPN 込み・減算なし — CLAUDE.md の既定、R13、0x03 と同じ並び)。

## 応答(200、`Content-Type: application/json`)
```json
{
  "run": 70,
  "event_idx": 1234,
  "complete": true,
  "received_unix_ms": 1760259600123,
  "age_ms": 120,
  "fragments": [
    {
      "cobo": 0, "asad": 0,
      "event_idx": 1234, "event_time": 4063600000,
      "frame_type": 1, "revision": 5, "read_offset": 0, "status": 0,
      "mult": [0, 0, 0, 0], "window_out": 0, "last_cell": [0, 0, 0, 0],
      "n_aget": 4, "n_ch": 68, "n_buckets": 512,
      "adc": [[[ /* 512 個 */ ], /* … 68 ch */ ], /* … 4 AGET */ ]
    }
  ]
}
```
- `adc[aget][ch][bucket]` は int。**読まれていない (aget, ch, bucket) は `-1`**(部分読み出しや ZS で item が無いところ。ADC は 12 bit の 0–4095 なので区別できる)。
- 範囲外の item(aget ≥ 4、ch ≥ 68、bucket ≥ 512)は捨てる。その数を `"items_out_of_range"` としてフラグメントごとに出す(silent 禁止。0x03 の扱いと揃える)。
- 1 イベントあたりおよそ 0.7 MB。
- `received_unix_ms` は monitor がそのイベントを受け取った時刻。`age_ms` は要求時点での経過(run が止まっていれば最後の run の最後のイベントが返り、age で古さが分かる)。
- まだ 1 件も受け取っていなければ **404** で `{"error": "no event received yet"}`。
- ヘッダ `Access-Control-Allow-Origin: *`(読み出し専用のデータ。ブラウザの解析ツールからも呼べるように)。
- メソッドは GET だけ。

## 実装の要点
- `src/monitor.rs`: `Hub`(または同等の共有状態)に `last_event: Mutex<Option<(Arc<BuiltEventPayload>, SystemTime)>>` を持たせ、
  `MonitorPayload::BuiltEvent` の分岐で差し替える。
  - ペイロードが既に `Box` で来ているなら所有権を移す(`Arc::from`)だけで、コピーは増やさない。
  - 0x02 / 0x03 の作成にも同じ参照を使えるなら使う。
- `ws_router` に `.route("/api/last-event", get(last_event))`。
- 変換は純関数 `last_event_json(event, received, now) -> serde_json::Value`(または Serialize する struct)にし、`unpack_item` / `ItemWords` は 0x03 の `waveforms` と同じものを使う。
- counters に `last_event_requests` を足すかどうかは任意(既存の流儀に合わせる)。

## テスト(TDD)
- 純関数: 既知の item(例: aget 1・ch 11・bucket 7 = ADC 1234、aget 3・ch 67・bucket 511 = 4095)を持つフラグメントから、
  `adc[1][11][7] == 1234`、`adc[3][67][511] == 4095`、ほかは `-1`、ヘッダの各値がそのまま出ること。
  範囲外の item が数えられること。2 フラグメント(asad 0 / 1)が両方出ること。
- 統合(`tests/` の monitor 系ハーネス、既存の monitor_ws の流儀):
  - イベント受信前の GET は 404。
  - root-sink 役の PUB から built_event を 2 件送る → GET は**後の方**(event_idx)を返し、`Access-Control-Allow-Origin` が付く。
- ゲート: `cargo fmt && cargo clippy --tests -- -D warnings && cargo clippy --tests --features slowctl-fake -- -D warnings && cargo test`。

## SPEC(Opus 下書き → Fable 確定)
§10 の monitor に HTTP の節(`GET /api/last-event` の形)、§11 の解析向け API 一覧に 1 行。

## 進捗(2026-10-12)
- 実装(Sonnet `e16c312` → merge `5380b43`):
  - `src/monitor.rs`
    - `Hub.last_event` は built_event を受けるたびに `Arc::from(Box)` で差し替える(items はコピーしない。WS 購読の有無に関係なく保持)。
    - 純関数 `last_event_json` は `-1` で埋めたグリッドに item を置き、範囲外の item は `items_out_of_range` に数える。
    - ルート `/api/last-event`:
      - 応答は 200(イベントあり)/ 404(まだ無い)。
      - どちらにも `Access-Control-Allow-Origin: *` を付ける。
      - GET 以外は 405。
    - カウンタ `last_event_requests` を追加。
  - 照合(CLAUDE.md の新ルール):
    - `git show --stat` は 2 files(monitor.rs / tests/monitor_ws.rs)。
    - テスト 2 本・ルート・CORS・範囲外の計数が diff にある。
    - 報告の「実行できなかった操作」は「なし」(sandbox が heredoc を 2 回拒否したが、Edit で回避)。
- テスト:
  - `monitor::tests::last_event_json_places_known_items_and_marks_the_rest_unread`
  - `last_event_api_is_404_before_any_event_and_returns_the_latest_after`(統合。受信前は 404、event_idx 21 と 22 を送ると 22 が返る)
  - red の確認は省略(Sonnet 申告)。
- ゲート(main `5380b43`、Mac): fmt / clippy 0 / **cargo test 933 passed / 0 failed / 1 ignored**(既存)。
- 実機(ELITPC、11:08:35Z に monitor だけ入れ替え、run なし): `GET :9000/api/last-event` → `404`、`content-type: application/json`、`access-control-allow-origin: *`、`{"error":"no event received yet"}`。
- **残り**:
  - 次の run での実イベントの確認(adc の形、event_idx が進むこと、応答時間と大きさ)。
  - analysis team に、ジオメトリ(ストリップ対応)が要るかの確認(ユーザー経由)。
  - SPEC(§10 に monitor の HTTP 節)の Fable 確定。

## 結果(2026-10-12、実イベント)
- ユーザーが run 70 を開始(「見る限り良さそうですね」)。ELITPC 上で `GET 127.0.0.1:9000/api/last-event` を 1.5 s 間隔で 2 回取得した:
  - 応答: run 70、complete true、**event_idx 592 → 603**(進む)、age_ms 401 / 75、**545 kB / 36 ms・546 kB / 40 ms**、CORS `*`。
  - フラグメント: cobo 0 / asad 0、shape **4 × 68 × 512**、frame_type 1、event_time 11600903222、**unread 0**(全読み出し)、**items_out_of_range 0**、ADC 345–1507。
- ゲートは上の進捗節のとおり(cargo 933 / 0 / 1 ignored)。スキップしたテストは無し。
- 残り:
  - analysis team にジオメトリ(ストリップ対応)が要るかの確認(ユーザー経由)。要るなら別票にする。
  - SPEC 追記の Fable 確定(CURRENT の Fable キューに文案)。
