use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_bytes::ByteBuf;

use super::*;
use crate::msg::Fragment;

// =====================================================================
// SPEC §5.3 — モニタ PUB のペイロード(named struct パーサ)
// =====================================================================
//
// ワイヤは map 形式 msgpack(`to_vec_named` 相当の自己記述)。エンベロープだけは
// §2.2 と同形の positional array(5) なので、[`crate::msg::Message`] / [`crate::msg::Batch`]
// をそのまま被せて読む。

/// 面毎の飽和カウンタ(SPEC §5.3 の `saturation`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaturationPayload {
    pub saturated: u64,
    pub counted: u64,
}

/// `kind = "status"`(SPEC §5.3、v1.10 で `pending_events` を含む 11 キー)。
///
/// WS の `status` JSON は**このフィールドをそのまま**出し、`{monitorGaps, clients, wsDropped}`
/// を足す(SPEC §10.3)ので `Serialize` も導出する。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusPayload {
    pub kind: String,
    pub run: u32,
    pub state: String,
    pub events_built: u64,
    pub events_incomplete: u64,
    pub late_fragments: u64,
    /// ビルダ組み上げ中の瞬間値(SPEC §5.3 v1.10、R-P2-5)。`late_fragments` の次のキー。
    pub pending_events: u64,
    pub frames_per_cobo: std::collections::BTreeMap<String, u64>,
    pub bytes_written: u64,
    pub saturation: std::collections::BTreeMap<String, SaturationPayload>,
    pub publish_drops: u64,
}

/// ヒスト 1 枚(SPEC §5.3 の `hists[]`)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistPayload {
    pub id: u8,
    pub name: String,
    pub nx: u32,
    /// 1D は 1。
    pub ny: u32,
    /// f64 LE の生バイト列(長さ `nx*ny*8`)。2D の添字は `(strip-1)*512 + bucket`
    /// (**strip が遅い軸** — SPEC §5.3)。
    pub bins: ByteBuf,
}

impl HistPayload {
    /// 宣言どおりのビン数(`nx*ny`)。
    pub fn bin_count(&self) -> usize {
        (self.nx as usize).saturating_mul(self.ny as usize)
    }

    /// `bins` の長さが `nx*ny*8` と辻褄が合っているか。
    pub fn is_consistent(&self) -> bool {
        self.bins.len() == self.bin_count() * 8
    }

    /// ビン値を f64 として取り出す(照合用の便宜メソッド。表示変換は
    /// [`ws::histo1d`] / [`ws::histo2d`] が生バイトのまま読むので中間 Vec を作らない)。
    pub fn values(&self) -> Vec<f64> {
        self.bins.chunks_exact(8).map(f64_le).collect()
    }
}

/// `kind = "hist_snapshot"`(SPEC §5.3)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistSnapshotPayload {
    pub kind: String,
    pub run: u32,
    pub hists: Vec<HistPayload>,
}

/// `kind = "built_event"`(SPEC §5.3。`fragments` は §2.4 の positional array)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuiltEventPayload {
    pub kind: String,
    pub run: u32,
    pub event_idx: u32,
    pub complete: bool,
    pub fragments: Vec<Fragment>,
}

/// `kind` だけを先に読む前哨(3 種の判別 + 未知 kind の前方互換)。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct KindProbe {
    kind: String,
}

/// 判別済みペイロード。**未知 kind は捨てずに [`MonitorPayload::Unknown`] として返す**
/// (呼び手が数えて可視化する — 前方互換、silent 禁止)。
#[derive(Debug, Clone, PartialEq)]
pub enum MonitorPayload {
    Status(Box<StatusPayload>),
    HistSnapshot(Box<HistSnapshotPayload>),
    BuiltEvent(Box<BuiltEventPayload>),
    /// slowctl の kind=`slow`(SPEC §2.3 v1.31)。root-sink とは**別ソース**
    /// (source_id 200)で同じ SUB に入ってくる。
    Slow(Box<crate::slowctl::SlowPayload>),
    Unknown(String),
}

/// モニタ PUB の 1 通(エンベロープ = SPEC §2.2 + 判別済みペイロード)。
#[derive(Debug, Clone, PartialEq)]
pub struct MonitorMessage {
    pub source_id: u32,
    pub run_number: u32,
    /// **run リセットなしの単調増加**(SPEC §5.3 v1.10)。飛び = ドロップ数。
    pub sequence_number: u64,
    pub created_ns: u64,
    pub payload: MonitorPayload,
}

impl MonitorMessage {
    /// `status` ならその中身(照合・テスト用)。
    pub fn status(&self) -> Option<&StatusPayload> {
        match &self.payload {
            MonitorPayload::Status(s) => Some(s),
            _ => None,
        }
    }

    /// `hist_snapshot` ならその中身。
    pub fn hist_snapshot(&self) -> Option<&HistSnapshotPayload> {
        match &self.payload {
            MonitorPayload::HistSnapshot(s) => Some(s),
            _ => None,
        }
    }

    /// `built_event` ならその中身。
    pub fn built_event(&self) -> Option<&BuiltEventPayload> {
        match &self.payload {
            MonitorPayload::BuiltEvent(e) => Some(e),
            _ => None,
        }
    }

    /// `slow` ならその中身(SPEC §2.3 v1.31)。
    pub fn slow(&self) -> Option<&crate::slowctl::SlowPayload> {
        match &self.payload {
            MonitorPayload::Slow(s) => Some(s),
            _ => None,
        }
    }
}

/// モニタ PUB の 1 通が読めなかった理由。
#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("monitor PUB message ({len} B) is not a SPEC §2.2 Data envelope with a `kind` payload: {source}")]
    Envelope {
        len: usize,
        #[source]
        source: rmp_serde::decode::Error,
    },

    #[error("monitor PUB message is not Data (EndOfStream/Heartbeat is not part of SPEC §5.3)")]
    NotData,

    #[error("monitor PUB payload kind={kind} does not parse: {source}")]
    Payload {
        kind: String,
        #[source]
        source: rmp_serde::decode::Error,
    },
}

/// モニタ PUB の 1 通を復号する(SPEC §5.3)。
///
/// `kind` を先に読んでから本体を型付きで読み直す 2 段構え(map 形式の自己記述ワイヤに
/// 対する素直な実装)。未知 kind はエラーにせず [`MonitorPayload::Unknown`] で返す。
pub fn decode_message(raw: &[u8]) -> Result<MonitorMessage, WireError> {
    let probe: crate::msg::Message<KindProbe> =
        rmp_serde::from_slice(raw).map_err(|source| WireError::Envelope {
            len: raw.len(),
            source,
        })?;
    let crate::msg::Message::Data(batch) = probe else {
        return Err(WireError::NotData);
    };
    let kind = batch.payload.kind;
    let payload = match kind.as_str() {
        "status" => MonitorPayload::Status(Box::new(payload_of(raw, &kind)?)),
        "hist_snapshot" => MonitorPayload::HistSnapshot(Box::new(payload_of(raw, &kind)?)),
        "built_event" => MonitorPayload::BuiltEvent(Box::new(payload_of(raw, &kind)?)),
        crate::slowctl::SLOW_KIND => MonitorPayload::Slow(Box::new(payload_of(raw, &kind)?)),
        _ => MonitorPayload::Unknown(kind),
    };
    Ok(MonitorMessage {
        source_id: batch.source_id,
        run_number: batch.run_number,
        sequence_number: batch.sequence_number,
        created_ns: batch.created_ns,
        payload,
    })
}

fn payload_of<T: DeserializeOwned>(raw: &[u8], kind: &str) -> Result<T, WireError> {
    let message: crate::msg::Message<T> =
        rmp_serde::from_slice(raw).map_err(|source| WireError::Payload {
            kind: kind.to_string(),
            source,
        })?;
    match message {
        crate::msg::Message::Data(batch) => Ok(batch.payload),
        _ => Err(WireError::NotData),
    }
}

// =====================================================================
// ギャップ計数(SPEC §5.4「モニタ取りこぼし数」)
// =====================================================================

/// モニタ PUB の `sequence_number` の飛びを数える。
///
/// PUB リンクの seq は **run リセット無しの単調増加**(SPEC §5.3 v1.10)なので、
/// 「前回 + 1」以外は取りこぼし。送り手(root-sink)が再起動して seq が巻き戻った場合は
/// **ギャップではなく再起動**として数え直す(でっちあげの巨大ギャップを出さない)。
#[derive(Debug, Default)]
pub struct GapTracker {
    last: Option<u64>,
    gaps: u64,
    restarts: u64,
}

impl GapTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// 1 通観測する。返り値 = **この 1 通で新たに判明した取りこぼし数**。
    pub fn observe(&mut self, sequence_number: u64) -> u64 {
        let missed = match self.last {
            None => 0,
            Some(prev) if sequence_number > prev => sequence_number - prev - 1,
            Some(_) => {
                // 巻き戻り = 送り手の再起動(seq は単調のはずなので)。
                self.restarts += 1;
                0
            }
        };
        self.last = Some(sequence_number);
        self.gaps += missed;
        missed
    }

    /// 累積の取りこぼし数(= `monitor_gaps`)。
    pub fn gaps(&self) -> u64 {
        self.gaps
    }

    /// 送り手の seq 巻き戻り(再起動)を観測した回数。
    pub fn restarts(&self) -> u64 {
        self.restarts
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::monitor::test_util::*;

    #[test]
    fn gap_tracker_counts_missing_sequence_numbers_only() {
        let mut tracker = GapTracker::new();
        // 初回は基準点(ギャップではない)。
        assert_eq!(tracker.observe(100), 0);
        // 連続
        assert_eq!(tracker.observe(101), 0);
        assert_eq!(tracker.observe(102), 0);
        assert_eq!(tracker.gaps(), 0);
        // 飛び: 103,104 が落ちた = 2
        assert_eq!(tracker.observe(105), 2);
        assert_eq!(tracker.gaps(), 2);
        // また飛び: 106..109 が落ちた = 4(累積 6)
        assert_eq!(tracker.observe(110), 4);
        assert_eq!(tracker.gaps(), 6);
        // 巻き戻り(root-sink 再起動)はギャップにしない
        assert_eq!(tracker.observe(0), 0);
        assert_eq!(tracker.gaps(), 6);
        assert_eq!(tracker.restarts(), 1);
        assert_eq!(tracker.observe(1), 0, "再起動後は新しい系列として続く");
    }

    #[test]
    fn decode_message_reads_the_three_kinds_from_the_named_wire() {
        let status = status_payload();
        let raw = envelope(
            101,
            7,
            42,
            1_755_000_000_000_000_000,
            &rmp_serde::to_vec_named(&status).unwrap(),
        );
        let message = decode_message(&raw).unwrap();
        assert_eq!(message.source_id, 101, "SPEC §3.2: root-sink PUB = 101");
        assert_eq!(message.run_number, 7);
        assert_eq!(message.sequence_number, 42);
        assert_eq!(message.created_ns, 1_755_000_000_000_000_000);
        assert_eq!(message.status(), Some(&status));

        let snapshot = HistSnapshotPayload {
            kind: "hist_snapshot".to_string(),
            run: 7,
            hists: vec![hist(1, "StripTimeU", 2, 2, &[1.0, 2.0, 3.0, 4.0])],
        };
        let raw = envelope(101, 7, 43, 1, &rmp_serde::to_vec_named(&snapshot).unwrap());
        let message = decode_message(&raw).unwrap();
        assert_eq!(message.hist_snapshot(), Some(&snapshot));
        assert_eq!(
            message.hist_snapshot().unwrap().hists[0].values(),
            vec![1.0, 2.0, 3.0, 4.0]
        );

        let event = built_event(7, 5, false, vec![fragment(0, 0, &[(1, 2, 3, 4)])]);
        let raw = envelope(101, 7, 44, 2, &rmp_serde::to_vec_named(&event).unwrap());
        let message = decode_message(&raw).unwrap();
        assert_eq!(message.built_event(), Some(&event));
    }

    #[test]
    fn decode_message_keeps_unknown_kinds_for_forward_compatibility() {
        #[derive(Serialize)]
        struct Future {
            kind: String,
            answer: u32,
        }
        let payload = Future {
            kind: "psu_reading".to_string(),
            answer: 42,
        };
        let raw = envelope(101, 3, 7, 9, &rmp_serde::to_vec_named(&payload).unwrap());
        let message = decode_message(&raw).unwrap();
        assert_eq!(message.sequence_number, 7, "エンベロープは読める");
        assert_eq!(
            message.payload,
            MonitorPayload::Unknown("psu_reading".to_string()),
            "未知 kind はエラーにせず名前を残す(数えて無視する材料)"
        );
    }

    #[test]
    fn decode_message_rejects_garbage_and_non_data_messages() {
        assert!(matches!(
            decode_message(&[0x00, 0x01, 0x02]),
            Err(WireError::Envelope { .. })
        ));
        let eos: crate::msg::Message<crate::msg::Fragments> = crate::msg::Message::EndOfStream {
            source_id: 101,
            run_number: 7,
        };
        assert!(matches!(
            decode_message(&eos.to_msgpack().unwrap()),
            Err(WireError::NotData)
        ));
    }
}
