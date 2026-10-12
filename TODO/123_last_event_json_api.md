# 123 — 最新の 1 イベントを JSON で返す web API(analysis team の要望)

**Status: IN PROGRESS**(2026-10-12 起票 Opus。ユーザー「analysis team からのリクエストで、last event だけを返す web API 作れますか? JSON 形式で graw ファイルを数値化した 1 イベントで OK です」。Sonnet)

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
