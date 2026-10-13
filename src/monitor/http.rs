use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast;

use super::*;
use crate::geometry::{AGET_CHIPS_PER_ASAD, RAW_CH_PER_AGET};

// ---------------------------------------------------------------------
// WS 側(axum)
// ---------------------------------------------------------------------

/// WS ルータ(SPEC §10。monitor は **REP を持たない** — 純コンシューマ)。
///
/// `/` と `/ws` の両方で upgrade を受ける(SPEC は WS のパスを定めていない。
/// ポート 9000 は monitor 専用なので、UI がどちらを叩いても繋がるようにしておく)。
pub(super) fn ws_router(hub: Arc<Hub>) -> Router {
    Router::new()
        .route("/", get(ws_upgrade))
        .route("/ws", get(ws_upgrade))
        .route("/api/last-event", get(last_event))
        .with_state(hub)
}

/// `GET /api/last-event`(TODO/123)。直近の built event を graw 相当の数値で返す。
/// 変換は要求が来たときだけ(モニタ系 — 通常経路に負荷をかけない)。
async fn last_event(State(hub): State<Arc<Hub>>) -> Response {
    MonitorCounters::bump(&hub.counters.last_event_requests, 1);
    let latest = lock(&hub.last_event).clone();
    let (status, body) = match latest {
        Some((event, received)) => (
            StatusCode::OK,
            last_event_json(&event, received, SystemTime::now()),
        ),
        None => (
            StatusCode::NOT_FOUND,
            serde_json::json!({"error": "no event received yet"}),
        ),
    };
    (
        status,
        [(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")],
        axum::Json(body),
    )
        .into_response()
}

fn unix_ms(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// built event → `GET /api/last-event` の JSON(TODO/123)。
///
/// `adc[aget][ch][bucket]` は生 ADC(FPN 込み・減算なし、0x03 と同じ並び)。
/// 読まれていないセルは `-1`。範囲外 item は捨ててフラグメント毎に
/// `items_out_of_range` として数える(silent にしない)。
pub fn last_event_json(
    event: &BuiltEventPayload,
    received: SystemTime,
    now: SystemTime,
) -> serde_json::Value {
    let n_aget = AGET_CHIPS_PER_ASAD as usize;
    let n_ch = RAW_CH_PER_AGET as usize;
    let n_bk = N_BUCKETS as usize;
    let mut counts = ConvertCounts::default();
    let mut logged = false;
    let fragments: Vec<serde_json::Value> = event
        .fragments
        .iter()
        .map(|f| {
            let mut grid = vec![-1i16; n_aget * n_ch * n_bk];
            let mut out_of_range = 0u64;
            for word in ItemWords::new(&f.items, &mut counts, &mut logged) {
                let item = crate::msg::unpack_item(word);
                let (a, c, b) = (item.aget as usize, item.chan as usize, item.bucket as usize);
                if a >= n_aget || c >= n_ch || b >= n_bk {
                    out_of_range += 1;
                    continue;
                }
                grid[(a * n_ch + c) * n_bk + b] = item.adc as i16;
            }
            let adc: Vec<Vec<&[i16]>> = grid
                .chunks(n_ch * n_bk)
                .map(|aget| aget.chunks(n_bk).collect())
                .collect();
            serde_json::json!({
                "cobo": f.cobo, "asad": f.asad,
                "event_idx": f.event_idx, "event_time": f.event_time,
                "frame_type": f.frame_type, "revision": f.revision,
                "read_offset": f.read_offset, "status": f.status,
                "mult": f.mult, "window_out": f.window_out, "last_cell": f.last_cell,
                "n_aget": n_aget, "n_ch": n_ch, "n_buckets": n_bk,
                "items_out_of_range": out_of_range,
                "adc": adc,
            })
        })
        .collect();
    let age = now.duration_since(received).unwrap_or_default();
    serde_json::json!({
        "run": event.run,
        "event_idx": event.event_idx,
        "complete": event.complete,
        "received_unix_ms": unix_ms(received),
        "age_ms": age.as_millis() as u64,
        "fragments": fragments,
    })
}

async fn ws_upgrade(State(hub): State<Arc<Hub>>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| serve_client(socket, hub))
}

/// 1 クライアント分の面倒を見る。
///
/// 送信(live + JSON)と受信(subscribe)は同じソケットを触るので split して 2 つに分ける。
/// どちらかが終わればもう片方も畳んで、クライアントを掃除する。
async fn serve_client(socket: WebSocket, hub: Arc<Hub>) {
    let (mut sink, mut stream) = socket.split();
    let (id, streams, mut json_rx, mut live_rx) = hub.register();
    let counters = Arc::clone(&hub.counters);
    let meta = hub.meta_json();
    let strip_geometry = hub.strip_geometry.clone();

    let writer = tokio::spawn(async move {
        // 接続時の 1 通目は meta(SPEC §10.3)、2 通目は stripGeometry(TODO/125)。
        for text in [meta, strip_geometry].into_iter().flatten() {
            if sink.send(WsMessage::Text(text.into())).await.is_err() {
                return;
            }
        }
        loop {
            tokio::select! {
                json = json_rx.recv() => match json {
                    Some(text) => {
                        if sink.send(WsMessage::Text(text.into())).await.is_err() {
                            return;
                        }
                    }
                    None => return, // hub がこのクライアントを切った
                },
                live = live_rx.recv() => match live {
                    Ok(frame) => {
                        if !streams.wants(frame.stream) {
                            continue;
                        }
                        if sink.send(WsMessage::Binary(frame.bytes.clone())).await.is_err() {
                            return;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(missed)) => {
                        // drop-oldest(SPEC §10.3)。落ちた分は必ず数える。
                        counters.ws_dropped.fetch_add(missed, Ordering::Relaxed);
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                },
            }
        }
    });

    while let Some(Ok(message)) = stream.next().await {
        match message {
            WsMessage::Text(text) => hub.apply_subscribe(id, &text),
            WsMessage::Close(_) => break,
            _ => {}
        }
    }

    writer.abort();
    hub.remove_client(id);
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::monitor::test_util::*;

    #[test]
    fn last_event_json_places_known_items_and_marks_the_rest_unread() {
        let t0 = UNIX_EPOCH + std::time::Duration::from_millis(1_760_259_600_123);
        let now = t0 + std::time::Duration::from_millis(120);
        let event = built_event(
            70,
            1234,
            false,
            vec![
                // asad 0: (aget1,ch11,bk7)=1234, (aget3,ch67,bk511)=4095, 範囲外 2 個
                // (aget3,ch68,bk0) = ch 超過、(aget0,ch0,bk0) は範囲内で ADC 0 と -1 を区別。
                fragment(
                    0,
                    0,
                    &[
                        (1, 11, 7, 1234),
                        (3, 67, 511, 4095),
                        (3, 68, 0, 5),
                        (2, 127, 3, 6),
                        (0, 0, 0, 0),
                    ],
                ),
                fragment(0, 1, &[(2, 5, 9, 77)]),
            ],
        );
        let v = last_event_json(&event, t0, now);

        assert_eq!(v["run"], 70);
        assert_eq!(v["event_idx"], 1234);
        assert_eq!(v["complete"], false);
        assert_eq!(v["received_unix_ms"], 1_760_259_600_123u64);
        assert_eq!(v["age_ms"], 120);
        let frags = v["fragments"].as_array().unwrap();
        assert_eq!(frags.len(), 2, "asad 0 / 1 の両方");

        let f = &frags[0];
        assert_eq!((f["cobo"].as_u64(), f["asad"].as_u64()), (Some(0), Some(0)));
        // fragment() ヘルパの固定ヘッダ値がそのまま出る。
        assert_eq!(f["event_time"], 0x0000_1234_5678_9abcu64);
        assert_eq!(f["frame_type"], 2);
        assert_eq!(f["revision"], 5);
        assert_eq!(f["read_offset"], 136);
        assert_eq!(f["status"], 0);
        assert_eq!(f["mult"], serde_json::json!([68, 0, 17, 3]));
        assert_eq!(f["window_out"], 512);
        assert_eq!(f["last_cell"], serde_json::json!([7, 9, 11, 13]));
        assert_eq!(
            (
                f["n_aget"].as_u64(),
                f["n_ch"].as_u64(),
                f["n_buckets"].as_u64()
            ),
            (Some(4), Some(68), Some(512))
        );
        assert_eq!(f["items_out_of_range"], 2);
        assert_eq!(f["adc"][1][11][7], 1234);
        assert_eq!(f["adc"][3][67][511], 4095);
        assert_eq!(f["adc"][0][0][0], 0, "ADC 0 は読まれた値");
        assert_eq!(f["adc"][0][0][1], -1, "未読は -1");
        assert_eq!(f["adc"][1][11][8], -1);
        assert_eq!(f["adc"][2].as_array().unwrap().len(), 68);
        assert_eq!(f["adc"][2][0].as_array().unwrap().len(), 512);

        let g = &frags[1];
        assert_eq!(g["asad"], 1);
        assert_eq!(g["items_out_of_range"], 0);
        assert_eq!(g["adc"][2][5][9], 77);
        assert_eq!(g["adc"][1][11][7], -1, "フラグメント間で混ざらない");
    }
}
