//! WS バイナリメッセージのエンコーダ(SPEC §10.1/§10.2)。
//!
//! バイトオフセットは SPEC の表そのもの。**ここを動かすと UI(027)の
//! デコーダが黙って壊れる**ので、レイアウトはオフセット assert 付きの単体テスト
//! (SPEC §10.4-4)で固定してある。

use serde::ser::{SerializeSeq, Serializer};
use serde::Serialize;

use crate::reco::{NoTrack, Track, TrackEvent, TrackPoint};

/// 13 バイトヘッダ(SPEC §10.1)。
pub const HEADER_LEN: usize = 13;
/// マジック `'T' 'P'`(off 0–1)。
pub const MAGIC: [u8; 2] = *b"TP";
/// off 3。型再定義のため 2(SPEC §10.1)。
pub const VERSION: u8 = 2;
/// off 4 の bit0 = incomplete event。
pub const FLAG_INCOMPLETE: u8 = 0x01;

/// off 2 の msgType(SPEC §10.2)。
pub const TYPE_UVW: u8 = 0x02;
pub const TYPE_WAVEFORMS: u8 = 0x03;
/// 飛跡(ヘッダ + UTF-8 JSON 本文 — TODO/127)。
pub const TYPE_TRACK: u8 = 0x04;
pub const TYPE_HISTO1D: u8 = 0x10;
pub const TYPE_HISTO2D: u8 = 0x11;

/// 13 バイトヘッダの中身。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub msg_type: u8,
    /// bit0(イベント系のみ。ヒストは常に false)。
    pub incomplete: bool,
    pub run_number: u32,
    /// ヒスト・status 系は 0(SPEC §10.1)。
    pub event_number: u32,
}

impl Header {
    /// イベント系(0x02/0x03/0x04)のヘッダ。
    pub fn event(msg_type: u8, run_number: u32, event_number: u32, incomplete: bool) -> Header {
        Header {
            msg_type,
            incomplete,
            run_number,
            event_number,
        }
    }

    /// ヒスト系(0x10/0x11)のヘッダ。`eventNumber` は 0、flags は 0(SPEC §10.1)。
    pub fn hist(msg_type: u8, run_number: u32) -> Header {
        Header {
            msg_type,
            incomplete: false,
            run_number,
            event_number: 0,
        }
    }

    /// ヘッダを書き出す(全フィールド LE)。
    pub fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&MAGIC);
        out.push(self.msg_type);
        out.push(VERSION);
        out.push(if self.incomplete { FLAG_INCOMPLETE } else { 0 });
        out.extend_from_slice(&self.run_number.to_le_bytes());
        out.extend_from_slice(&self.event_number.to_le_bytes());
    }
}

/// ヒストの軸レンジ(f32 で運ぶ — 表示専用)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Axis {
    pub min: f32,
    pub max: f32,
}

/// `0x02 Uvw` — `u8 plane`, `u16 nStrips`, `u16 nBuckets`, `u16 ADC × nStrips×nBuckets`
/// (strip-major: `idx=(strip-1)*nBuckets+bucket`)。
pub fn uvw(header: Header, plane: u8, n_strips: u16, n_buckets: u16, grid: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + 5 + grid.len() * 2);
    header.write(&mut out);
    out.push(plane);
    out.extend_from_slice(&n_strips.to_le_bytes());
    out.extend_from_slice(&n_buckets.to_le_bytes());
    for v in grid {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// `0x03 Waveforms` — `u8 cobo`, `u8 asad`, `u8 nAget`, `u8 nCh`, `u16 nBuckets`,
/// `u16 ADC × nAget×nCh×nBuckets`(aget-major、raw ch 順、FPN 込み・減算なし)。
pub fn waveforms(
    header: Header,
    cobo: u8,
    asad: u8,
    n_aget: u8,
    n_ch: u8,
    n_buckets: u16,
    grid: &[u16],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + 6 + grid.len() * 2);
    header.write(&mut out);
    out.push(cobo);
    out.push(asad);
    out.push(n_aget);
    out.push(n_ch);
    out.extend_from_slice(&n_buckets.to_le_bytes());
    for v in grid {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// `0x04 Track` — 13 B ヘッダ + **UTF-8 JSON 本文**(§10.2 で唯一の JSON 本文 — TODO/127)。
///
/// 本文(TODO/127 の契約。キーの並びもこのとおり):
/// `{"ok", "reason", "hits":[U,V,W], "unplaced", "tracks":[{phiDeg, dir, start, end, startF, endF,
/// length, lengthF, xyLength, charge}], "points":[[x, y, t, q, view, track], ...]}`。
/// z と `dir` の z 成分は mm、`points` の `t` は time bucket。ok でなければ `tracks` / `points` は空、
/// `reason` に 125 と同じ文。ok なら `reason` は `""`。
///
/// 本文は作業バッファを借りたまま直接書く(中間の `serde_json::Value` を作らない)。
/// `Vec` への書き込みは失敗しない — Err は serde_json の内部エラーだけ。
pub fn track(header: Header, event: &TrackEvent<'_>) -> Result<Vec<u8>, serde_json::Error> {
    let track = event.fit.as_ref().ok().map(TrackJson::from);
    let body = TrackBody {
        ok: event.fit.is_ok(),
        reason: Reason(event.fit.as_ref().err()),
        hits: event.hits,
        unplaced: event.unplaced,
        tracks: track.as_slice(),
        points: Points(event.points),
    };
    // 1 点 ≈ 60–110 B(f64 4 つ + 整数 2 つ)。
    let mut out = Vec::with_capacity(HEADER_LEN + 512 + event.points.len() * 112);
    header.write(&mut out);
    serde_json::to_writer(&mut out, &body)?;
    Ok(out)
}

/// 0x04 の JSON 本文(フィールドの宣言順 = キーの並び)。
#[derive(Serialize)]
struct TrackBody<'a> {
    ok: bool,
    reason: Reason<'a>,
    hits: [usize; 3],
    unplaced: usize,
    tracks: &'a [TrackJson],
    points: Points<'a>,
}

/// `tracks` の 1 本。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TrackJson {
    phi_deg: f64,
    dir: [f64; 3],
    start: [f64; 3],
    end: [f64; 3],
    start_f: [f64; 3],
    end_f: [f64; 3],
    length: f64,
    length_f: f64,
    xy_length: f64,
    charge: f64,
}

impl From<&Track> for TrackJson {
    fn from(t: &Track) -> Self {
        Self {
            phi_deg: t.phi_deg,
            dir: t.dir,
            start: t.start,
            end: t.end,
            start_f: t.start_f,
            end_f: t.end_f,
            length: t.length,
            length_f: t.length_f,
            xy_length: t.xy_length,
            charge: t.charge,
        }
    }
}

/// `reason`: ok なら `""`、そうでなければ理由の文(`Display` をそのまま書く — String を作らない)。
struct Reason<'a>(Option<&'a NoTrack>);

impl Serialize for Reason<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            Some(reason) => serializer.collect_str(reason),
            None => serializer.serialize_str(""),
        }
    }
}

/// `points` = `[[x, y, t, q, view, track], ...]`。`track` = 何本目か(今は 1 本だけなので 0)。
struct Points<'a>(&'a [TrackPoint]);

impl Serialize for Points<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for p in self.0 {
            seq.serialize_element(&(p.x, p.y, p.t, p.q, p.view, 0u8))?;
        }
        seq.end()
    }
}

/// `0x10 Histo1d` — `u16 id`, `u32 nbins`, `f32 xmin`, `f32 xmax`, `f32 × nbins`。
///
/// `bins_le` は SPEC §5.3 の生バイト列(f64 LE)。中間 Vec を作らずに f32 へ落とす。
pub fn histo1d(header: Header, id: u16, x: Axis, bins_le: &[u8]) -> Vec<u8> {
    let nbins = bins_le.len() / 8;
    let mut out = Vec::with_capacity(HEADER_LEN + 14 + nbins * 4);
    header.write(&mut out);
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&(nbins as u32).to_le_bytes());
    out.extend_from_slice(&x.min.to_le_bytes());
    out.extend_from_slice(&x.max.to_le_bytes());
    for chunk in bins_le.chunks_exact(8) {
        out.extend_from_slice(&(super::f64_le(chunk) as f32).to_le_bytes());
    }
    out
}

/// `0x11 Histo2d` — `u16 id`, `u16 nx`, `u16 ny`, `f32 xmin,xmax,ymin,ymax`,
/// `f32 × nx×ny`(**iy 外側 row-major**)。
///
/// 入力 `bins_le` は SPEC §5.3 の PUB 順(`(strip-1)*512 + bucket` = **ix が外側**)なので、
/// ここで**転置**する。この 1 行が §5.3 と §10.2 の並び順の違いを吸収している。
pub fn histo2d(
    header: Header,
    id: u16,
    nx: u16,
    ny: u16,
    x: Axis,
    y: Axis,
    bins_le: &[u8],
) -> Vec<u8> {
    let (nxu, nyu) = (nx as usize, ny as usize);
    let mut out = Vec::with_capacity(HEADER_LEN + 22 + nxu * nyu * 4);
    header.write(&mut out);
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&nx.to_le_bytes());
    out.extend_from_slice(&ny.to_le_bytes());
    for v in [x.min, x.max, y.min, y.max] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for iy in 0..nyu {
        for ix in 0..nxu {
            let at = (ix * nyu + iy) * 8;
            let value = match bins_le.get(at..at + 8) {
                Some(chunk) => super::f64_le(chunk) as f32,
                None => 0.0,
            };
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::reco::Shortage;

    /// TODO/127 の発注書の JSON 例(契約)を最小化したもの。
    const CONTRACT_OK: &str = concat!(
        r#"{"ok":true,"reason":"","hits":[72,40,34],"unplaced":0,"#,
        r#""tracks":[{"phiDeg":2.7,"dir":[0.998,0.047,-0.031],"start":[-53.1,-7.9,26.4],"end":[53.0,-3.0,23.1],"#,
        r#""startF":[-53.5,-8.0,26.4],"endF":[53.6,-3.0,23.0],"length":106.2,"lengthF":107.2,"xyLength":106.1,"charge":412345.0}],"#,
        r#""points":[[-52.9,-7.9,152.0,5480.0,0,0],[-51.4,-7.8,151.6,5512.0,0,0]]}"#
    );

    fn u32_at(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
    }

    #[test]
    fn track_message_is_the_13_byte_header_plus_the_contract_json() {
        let points = [
            TrackPoint {
                x: -52.9,
                y: -7.9,
                t: 152.0,
                q: 5480.0,
                view: 0,
            },
            TrackPoint {
                x: -51.4,
                y: -7.8,
                t: 151.6,
                q: 5512.0,
                view: 0,
            },
        ];
        let event = TrackEvent {
            hits: [72, 40, 34],
            halo_hits: [75, 41, 36], // JSON には載らない
            unplaced: 0,
            no_segment: 3, // JSON には載らない(monitor が warn とログで数える)
            fit: Ok(Track {
                phi_deg: 2.7,
                dir: [0.998, 0.047, -0.031],
                start: [-53.1, -7.9, 26.4],
                end: [53.0, -3.0, 23.1],
                start_f: [-53.5, -8.0, 26.4],
                end_f: [53.6, -3.0, 23.0],
                length: 106.2,
                length_f: 107.2,
                xy_length: 106.1,
                charge: 412345.0,
            }),
            points: &points,
        };
        let message = track(Header::event(TYPE_TRACK, 0x0A0B_0C0D, 23618, true), &event).unwrap();

        // SPEC §10.1 の 13 B ヘッダ(イベント系)
        assert_eq!(&message[0..2], b"TP");
        assert_eq!(message[2], 0x04, "msgType = 0x04");
        assert_eq!(message[3], VERSION);
        assert_eq!(message[4], FLAG_INCOMPLETE, "incomplete = flags bit0");
        assert_eq!(u32_at(&message, 5), 0x0A0B_0C0D, "runNumber(LE)");
        assert_eq!(u32_at(&message, 9), 23618, "eventNumber(LE)");
        // 本文 = UTF-8 JSON、キーの並び・数の書式とも契約の例どおり
        let body = std::str::from_utf8(&message[HEADER_LEN..]).unwrap();
        assert_eq!(body, CONTRACT_OK);
    }

    #[test]
    fn track_message_without_a_track_has_the_reason_and_empty_lists() {
        let event = TrackEvent {
            hits: [40, 38, 0],
            halo_hits: [44, 40, 2],
            unplaced: 0,
            no_segment: 0,
            fit: Err(NoTrack::TooFewHits([
                Shortage::Enough,
                Shortage::Enough,
                Shortage::AfterHaloCut(2),
            ])),
            points: &[],
        };
        let message = track(Header::event(TYPE_TRACK, 70, 192, false), &event).unwrap();
        assert_eq!(message[4], 0, "complete = flags 0");
        assert_eq!(
            std::str::from_utf8(&message[HEADER_LEN..]).unwrap(),
            r#"{"ok":false,"reason":"W has 2 strip hits after the halo cut (needs 4)","hits":[40,38,0],"unplaced":0,"tracks":[],"points":[]}"#
        );

        // 非 ASCII の理由(en dash)も UTF-8 のまま JSON として読める
        let short = TrackEvent {
            fit: Err(NoTrack::ShortWindow(20)),
            hits: [0, 0, 0],
            ..event
        };
        let message = track(Header::event(TYPE_TRACK, 70, 193, false), &short).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&message[HEADER_LEN..]).unwrap();
        assert_eq!(
            value["reason"],
            "20 time buckets do not cover the pedestal window (cells 1–20)"
        );
        assert_eq!(value["ok"], false);
    }
}
