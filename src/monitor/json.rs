use super::*;
use crate::strip_xy::{StripGeometry, StripXyError};

// =====================================================================
// SPEC §10.3 — JSON テキストメッセージ(純)
// =====================================================================

/// `meta`(S→C、接続時・run 変化時)の材料。
#[derive(Debug, Clone, PartialEq)]
pub struct Meta {
    pub n_buckets: u16,
    /// 面毎の最大ストリップ番号(= `Geometry::max_strip`)。
    pub planes: [u16; N_PLANES],
    /// ジオメトリの設定パス名。
    pub geometry: String,
    /// `HeaderScalars::angles_deg`(無ければ `null`)。
    pub angles_deg: Option<[f64; 3]>,
    /// 検出器名(`[system] experiment`)。
    pub detector: String,
    pub cobos: Vec<u32>,
    pub run: u32,
}

/// `meta` を JSON 文字列にする(SPEC §10.3)。
pub fn meta_json(meta: &Meta) -> Result<String, serde_json::Error> {
    let value = serde_json::json!({
        "type": "meta",
        "nBuckets": meta.n_buckets,
        "planes": {
            "U": meta.planes[0],
            "V": meta.planes[1],
            "W": meta.planes[2],
        },
        "geometry": meta.geometry,
        "anglesDeg": meta.angles_deg,
        "detector": meta.detector,
        "cobos": meta.cobos,
        "run": meta.run,
    });
    serde_json::to_string(&value)
}

/// `stripGeometry`(S→C、接続時 1 回・`meta` の次 — TODO/125 の WS 契約)を JSON 文字列にする。
///
/// 作れたときは [`StripGeometry`] の中身 + `type`、作れなかったときは
/// `{"type":"stripGeometry","unavailable":"<理由>"}`(UI は理由をそのまま表示する)。
pub fn strip_geometry_json(
    geometry: &Result<StripGeometry, StripXyError>,
) -> Result<String, serde_json::Error> {
    let value = match geometry {
        Ok(geometry) => {
            let mut value = serde_json::to_value(geometry)?;
            if let Some(map) = value.as_object_mut() {
                map.insert("type".to_string(), serde_json::json!("stripGeometry"));
            }
            value
        }
        Err(reason) => serde_json::json!({
            "type": "stripGeometry",
            "unavailable": reason.to_string(),
        }),
    };
    serde_json::to_string(&value)
}

/// monitor 自身の可視化値(SPEC §10.3 の `status` に足す 3 つ + TODO/152 の ZS 復元 3 つ)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitorStats {
    pub monitor_gaps: u64,
    pub clients: u64,
    pub ws_dropped: u64,
    /// [`ConvertCounts::uvw_zs_fragments_restored`](TODO/152)。
    pub uvw_zs_fragments_restored: u64,
    /// [`ConvertCounts::uvw_zs_fragments_no_table`](TODO/152)。
    pub uvw_zs_fragments_no_table: u64,
    /// [`ConvertCounts::uvw_table_missing_channels`](TODO/152)。
    pub uvw_table_missing_channels: u64,
}

/// `status` を JSON 文字列にする(SPEC §10.3: §5.3 の status **そのまま** +
/// `{monitorGaps, clients, wsDropped}` + TODO/152 の
/// `{uvwZsFragmentsRestored, uvwZsFragmentsNoTable, uvwTableMissingChannels}`)。
pub fn status_json(
    status: &StatusPayload,
    stats: MonitorStats,
    slow: Option<&crate::slowctl::SlowPayload>,
) -> Result<String, serde_json::Error> {
    let mut value = serde_json::to_value(status)?;
    if let Some(map) = value.as_object_mut() {
        map.insert("type".to_string(), serde_json::json!("status"));
        // SPEC §10.3 v1.31: nullable —— 一度も受け取っていなければ **キーごと出さない**。
        if let Some(slow) = slow {
            map.insert("slow".to_string(), serde_json::to_value(slow)?);
        }
        map.insert(
            "monitorGaps".to_string(),
            serde_json::json!(stats.monitor_gaps),
        );
        map.insert("clients".to_string(), serde_json::json!(stats.clients));
        map.insert("wsDropped".to_string(), serde_json::json!(stats.ws_dropped));
        // TODO/152: 0x02 の ZS 復元。monitor 自身の値なので上の 3 つと同じ camelCase、常に出す。
        map.insert(
            "uvwZsFragmentsRestored".to_string(),
            serde_json::json!(stats.uvw_zs_fragments_restored),
        );
        map.insert(
            "uvwZsFragmentsNoTable".to_string(),
            serde_json::json!(stats.uvw_zs_fragments_no_table),
        );
        map.insert(
            "uvwTableMissingChannels".to_string(),
            serde_json::json!(stats.uvw_table_missing_channels),
        );
    }
    serde_json::to_string(&value)
}

/// `run`(S→C、state 遷移時)を JSON 文字列にする(SPEC §10.3)。
pub fn run_json(state: &str, run: u32, ts: &str) -> Result<String, serde_json::Error> {
    let value = serde_json::json!({ "type": "run", "state": state, "run": run, "ts": ts });
    serde_json::to_string(&value)
}

/// クライアント毎の購読集合(SPEC §10.3)。**既定は waveforms 以外 ON**。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamSet {
    pub uvw: bool,
    pub waveforms: bool,
    pub histos: bool,
    pub status: bool,
}

impl Default for StreamSet {
    fn default() -> Self {
        Self {
            uvw: true,
            waveforms: false, // 帯域制御(波形ビューを開いたクライアントだけ)
            histos: true,
            status: true,
        }
    }
}

impl StreamSet {
    /// この live ストリームを受けるか。
    pub fn wants(&self, stream: Stream) -> bool {
        match stream {
            Stream::Uvw => self.uvw,
            Stream::Waveforms => self.waveforms,
            Stream::Histos => self.histos,
        }
    }
}

/// C→S の `subscribe` の解釈結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscribe {
    pub streams: StreamSet,
    /// 知らないストリーム名(数えて可視化する — silent にしない)。
    pub unknown: Vec<String>,
}

/// `{"streams": [...]}`(SPEC §10.3)を解釈する。`streams` 配列が無ければ `None`。
///
/// **配列に無いものは OFF**(既定値との OR ではない)。UI が「波形を閉じた」ことを
/// 表現できる必要があるため。
pub fn parse_subscribe(text: &str) -> Option<Subscribe> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let list = value.get("streams")?.as_array()?;
    let mut streams = StreamSet {
        uvw: false,
        waveforms: false,
        histos: false,
        status: false,
    };
    let mut unknown = Vec::new();
    for item in list {
        match item.as_str() {
            Some("uvw") => streams.uvw = true,
            Some("waveforms") => streams.waveforms = true,
            Some("histos") => streams.histos = true,
            Some("status") => streams.status = true,
            Some(other) => unknown.push(other.to_string()),
            None => unknown.push(item.to_string()),
        }
    }
    Some(Subscribe { streams, unknown })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::monitor::test_util::*;

    #[test]
    fn status_json_is_the_spec_5_3_status_plus_the_monitor_three() {
        let status = status_payload();
        let text = status_json(
            &status,
            MonitorStats {
                monitor_gaps: 12,
                clients: 3,
                ws_dropped: 45,
                uvw_zs_fragments_restored: 0,
                uvw_zs_fragments_no_table: 0,
                uvw_table_missing_channels: 0,
            },
            None,
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();

        assert_eq!(value["type"], "status");
        // §5.3 のフィールドがそのまま
        assert_eq!(value["run"], 7);
        assert_eq!(value["state"], "running");
        assert_eq!(value["events_built"], 11);
        assert_eq!(value["events_incomplete"], 2);
        assert_eq!(value["late_fragments"], 3);
        assert_eq!(value["pending_events"], 4);
        assert_eq!(value["frames_per_cobo"]["0"], 108);
        assert_eq!(value["bytes_written"], 123_456);
        assert_eq!(value["saturation"]["W"]["saturated"], 4);
        assert_eq!(value["saturation"]["W"]["counted"], 5);
        assert_eq!(value["publish_drops"], 9);
        // + monitor の 3 つ(SPEC §10.3)
        assert_eq!(value["monitorGaps"], 12);
        assert_eq!(value["clients"], 3);
        assert_eq!(value["wsDropped"], 45);
        // SPEC §10.3 v1.31: slowctl を購読していなければ `slow` は**出ない**
        // (従来と 1 バイトも変わらない JSON)。
        assert!(value.get("slow").is_none());
    }

    /// TODO/152: 0x02 の ZS 復元カウンタも `status` に載る(monitor 自身の値なので
    /// `monitorGaps` と同じ camelCase。常に出す — 表なしの `NoTable` こそ見せたい値)。
    #[test]
    fn status_json_carries_the_uvw_zs_restore_counters() {
        let text = status_json(
            &status_payload(),
            MonitorStats {
                monitor_gaps: 0,
                clients: 1,
                ws_dropped: 0,
                uvw_zs_fragments_restored: 31,
                uvw_zs_fragments_no_table: 5,
                uvw_table_missing_channels: 2,
            },
            None,
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["uvwZsFragmentsRestored"], 31);
        assert_eq!(value["uvwZsFragmentsNoTable"], 5);
        assert_eq!(value["uvwTableMissingChannels"], 2);
        // §5.3 の status と monitor の 3 つは不変。
        assert_eq!(value["run"], 7);
        assert_eq!(value["monitorGaps"], 0);
    }

    /// slowctl を購読していれば同じ `status` に `slow` が同梱される(SPEC §10.3 v1.31)。
    #[test]
    fn status_json_carries_the_latest_slow_payload_when_there_is_one() {
        let slow = crate::slowctl::SlowPayload {
            kind: crate::slowctl::SLOW_KIND.to_string(),
            ts_ms: 1_758_000_000_123,
            samples: vec![crate::slowctl::Sample {
                device: "gas".to_string(),
                node: Some(4),
                param: "fmeasure".to_string(),
                value: 130.5,
                unit: "mbar".to_string(),
                ts_ms: 1_758_000_000_100,
                alarm: false,
            }],
            counters: crate::slowctl::SlowCounters {
                poll_ok: 7,
                ..crate::slowctl::SlowCounters::default()
            },
        };
        let text = status_json(
            &status_payload(),
            MonitorStats {
                monitor_gaps: 0,
                clients: 1,
                ws_dropped: 0,
                uvw_zs_fragments_restored: 0,
                uvw_zs_fragments_no_table: 0,
                uvw_table_missing_channels: 0,
            },
            Some(&slow),
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["slow"]["kind"], "slow");
        assert_eq!(value["slow"]["samples"][0]["param"], "fmeasure");
        assert_eq!(value["slow"]["samples"][0]["unit"], "mbar");
        assert_eq!(value["slow"]["counters"]["poll_ok"], 7);
        // §5.3 の status 側は一切変わらない。
        assert_eq!(value["run"], 7);
    }

    #[test]
    fn meta_json_matches_the_spec_10_3_shape() {
        let meta = Meta {
            n_buckets: 512,
            planes: [72, 92, 93],
            geometry: "config/geometry_mini_eTPC.dat".to_string(),
            angles_deg: Some([11.0, 22.0, 33.0]),
            detector: "mini_eTPC".to_string(),
            cobos: vec![0, 1],
            run: 57,
        };
        let value: serde_json::Value = serde_json::from_str(&meta_json(&meta).unwrap()).unwrap();
        assert_eq!(value["type"], "meta");
        assert_eq!(value["nBuckets"], 512);
        assert_eq!(value["planes"]["U"], 72);
        assert_eq!(value["planes"]["V"], 92);
        assert_eq!(value["planes"]["W"], 93);
        assert_eq!(value["geometry"], "config/geometry_mini_eTPC.dat");
        assert_eq!(value["anglesDeg"][0], 11.0);
        assert_eq!(value["anglesDeg"][2], 33.0);
        assert_eq!(value["detector"], "mini_eTPC");
        assert_eq!(value["cobos"][1], 1);
        assert_eq!(value["run"], 57);

        // ANGLES を持たないジオメトリでは null(発注書「無ければ null」)。
        let mut without = meta.clone();
        without.angles_deg = None;
        let value: serde_json::Value = serde_json::from_str(&meta_json(&without).unwrap()).unwrap();
        assert!(value["anglesDeg"].is_null());
    }

    /// `stripGeometry`(TODO/125 の WS 契約): `type` + strip_xy の中身そのまま。
    #[test]
    fn strip_geometry_json_is_the_strip_xy_contents_plus_its_type() {
        let geometry = geometry::parse(
            "ANGLES: 0.0 -120.0 -60.0\nDIAMOND SIZE: 1.0\nREFERENCE POINT: 3.0 2.0\n\
             W\t1\t2\t0\t0\t0\t2\t-0.5\t1.0\t2\n",
        );
        let built = crate::strip_xy::strip_geometry(&geometry).map(|(g, _)| g);
        let value: serde_json::Value =
            serde_json::from_str(&strip_geometry_json(&built).unwrap()).unwrap();
        assert_eq!(value["type"], "stripGeometry");
        assert!(value.get("unavailable").is_none());
        // 手計算(strip_xy の 0/−120/−60 ケースと同じ行): pad_pitch = √3、strip_pitch = 1.5、
        // W(section 1)2 は x0 = (3 − √3, 2)、length = 2√3。
        let sqrt3 = 3.0_f64.sqrt();
        assert!((value["padPitch"].as_f64().unwrap() - sqrt3).abs() < 1e-12);
        assert_eq!(value["stripPitch"], 1.5);
        assert_eq!(value["rangeXY"].as_array().unwrap().len(), 4);
        assert!(value["planes"]["U"]["segments"]
            .as_array()
            .unwrap()
            .is_empty());
        let w = &value["planes"]["W"]["segments"][0];
        assert_eq!((w[0].as_u64(), w[1].as_u64()), (Some(2), Some(1)));
        assert!((w[2].as_f64().unwrap() - (3.0 - sqrt3)).abs() < 1e-12);
        assert!((w[3].as_f64().unwrap() - 2.0).abs() < 1e-12);
        assert!((w[4].as_f64().unwrap() - 2.0 * sqrt3).abs() < 1e-12);
    }

    /// 作れないときは `{"type":"stripGeometry","unavailable":"<理由>"}` だけ(UI は理由をそのまま出す)。
    #[test]
    fn strip_geometry_json_says_unavailable_with_the_reason() {
        let geometry = geometry::parse("ANGLES: 0.0 -120.0 -60.0\nREFERENCE POINT: 0.0 0.0\n");
        let built = crate::strip_xy::strip_geometry(&geometry).map(|(g, _)| g);
        let value: serde_json::Value =
            serde_json::from_str(&strip_geometry_json(&built).unwrap()).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "type": "stripGeometry",
                "unavailable": "the geometry header has no DIAMOND SIZE",
            })
        );
    }

    #[test]
    fn run_json_carries_the_state_run_and_timestamp() {
        let value: serde_json::Value =
            serde_json::from_str(&run_json("running", 8, "2026-08-14T10:00:00.123+09:00").unwrap())
                .unwrap();
        assert_eq!(value["type"], "run");
        assert_eq!(value["state"], "running");
        assert_eq!(value["run"], 8);
        assert_eq!(value["ts"], "2026-08-14T10:00:00.123+09:00");
    }

    #[test]
    fn subscribe_defaults_to_everything_but_waveforms() {
        // SPEC §10.3: 既定は waveforms **以外** ON。
        let default = StreamSet::default();
        assert!(default.uvw && default.histos && default.status);
        assert!(!default.waveforms, "波形は開いたクライアントだけ");
        assert!(default.wants(Stream::Uvw));
        assert!(!default.wants(Stream::Waveforms));
        assert!(default.wants(Stream::Histos));
    }

    #[test]
    fn parse_subscribe_reads_the_streams_array() {
        let parsed =
            parse_subscribe(r#"{"streams":["uvw","waveforms","histos","status"]}"#).unwrap();
        assert_eq!(
            parsed.streams,
            StreamSet {
                uvw: true,
                waveforms: true,
                histos: true,
                status: true
            }
        );
        assert!(parsed.unknown.is_empty());

        // 配列に無いものは OFF(「波形を閉じた」が表現できる)
        let parsed = parse_subscribe(r#"{"type":"subscribe","streams":["status"]}"#).unwrap();
        assert_eq!(
            parsed.streams,
            StreamSet {
                uvw: false,
                waveforms: false,
                histos: false,
                status: true
            }
        );

        // 知らない名前は数えるために残す(捨てるが silent にしない)
        let parsed = parse_subscribe(r#"{"streams":["uvw","tracks3d"]}"#).unwrap();
        assert!(parsed.streams.uvw);
        assert_eq!(parsed.unknown, vec!["tracks3d".to_string()]);

        // subscribe ではないもの
        assert!(parse_subscribe(r#"{"cmd":"start"}"#).is_none());
        assert!(parse_subscribe("not json").is_none());
        assert!(parse_subscribe(r#"{"streams":"uvw"}"#).is_none());
    }
}
