use super::*;

// ---------------------------------------------------------------------
// コンポーネント状態のスナップショット
// ---------------------------------------------------------------------

/// `GET /api/status` と run_stop の材料。
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentStatus {
    pub name: String,
    pub kind: ComponentKind,
    pub endpoint: String,
    pub reachable: bool,
    pub state: Option<ComponentState>,
    pub run_number: Option<u32>,
    pub metrics: Option<Value>,
    pub error: Option<String>,
}

impl ComponentStatus {
    pub(super) fn from_response(component: &ComponentEndpoint, response: &CommandResponse) -> Self {
        Self {
            name: component.name.clone(),
            kind: component.kind,
            endpoint: component.endpoint.clone(),
            reachable: true,
            state: Some(response.state),
            run_number: response.run_number,
            metrics: response.metrics.clone(),
            error: (!response.success).then(|| response.message.clone()),
        }
    }

    pub(super) fn unreachable(component: &ComponentEndpoint, error: String) -> Self {
        Self {
            name: component.name.clone(),
            kind: component.kind,
            endpoint: component.endpoint.clone(),
            reachable: false,
            state: None,
            run_number: None,
            metrics: None,
            error: Some(error),
        }
    }

    pub(super) fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "endpoint": self.endpoint,
            "reachable": self.reachable,
            "state": self.state.map(|s| s.to_string()),
            "run_number": self.run_number,
            "metrics": self.metrics,
            "error": self.error,
        })
    }

    pub(super) fn metric_u64(&self, key: &str) -> Option<u64> {
        self.metrics
            .as_ref()
            .and_then(|m| m.get(key))
            .and_then(Value::as_u64)
    }
}

/// run_stop の `counters`(SPEC §9.2)を GetStatus 実測から充填する。
///
/// **取れる分だけ入れる**: `events_built` / `events_incomplete` / `late_fragments` は
/// root-sink だけが持つ値で、root-sink は REP を持たない(SPEC §1.3)ので常に `None`。
/// `overflow_frames` / `malformed` は receiver / decoder の GetStatus から実測できるので
/// `Some`(025 で `Counters` が `Option<u64>` 化された — SPEC v1.10 §9.2、016 逸脱③の確定処置。
/// 016 は非 Option で「取得不能」も `0` 埋めしていた)。
pub(super) fn counters_from(params: &ControllerParams, statuses: &[ComponentStatus]) -> Counters {
    let mut frames = BTreeMap::new();
    let mut overflow_frames = 0u64;
    let mut malformed = 0u64;
    // decoder が持っていれば Some(合計)、どの decoder も持たなければ None(0 と区別)。
    let mut acquisition_restarts: Option<u64> = None;
    // ソース別の辞書(TODO/133)。どの decoder も辞書を持たなければ None。
    let mut frames_before_restart: Option<BTreeMap<String, u64>> = None;
    // frameType 別(TODO/158)。どの decoder も持たなければ None(0 と区別)。
    let mut frames_type1: Option<u64> = None;
    let mut frames_type2: Option<u64> = None;
    let mut compact_size_mismatch: Option<u64> = None;
    for status in statuses {
        match status.kind {
            ComponentKind::Receiver { cobo_id } => {
                frames.insert(
                    cobo_id.to_string(),
                    status.metric_u64("frames").unwrap_or(0),
                );
                overflow_frames += status.metric_u64("overflow_frames").unwrap_or(0);
            }
            ComponentKind::Decoder => {
                malformed += status.metric_u64("malformed").unwrap_or(0);
                if let Some(n) = status.metric_u64("frames_type1") {
                    *frames_type1.get_or_insert(0) += n;
                }
                if let Some(n) = status.metric_u64("frames_type2") {
                    *frames_type2.get_or_insert(0) += n;
                }
                if let Some(n) = status.metric_u64("compact_size_mismatch") {
                    *compact_size_mismatch.get_or_insert(0) += n;
                }
                if let Some(n) = status.metric_u64("acquisition_restarts") {
                    *acquisition_restarts.get_or_insert(0) += n;
                }
                if let Some(per_source) = status
                    .metrics
                    .as_ref()
                    .and_then(|m| m.get("frames_before_restart"))
                    .and_then(Value::as_object)
                {
                    let merged = frames_before_restart.get_or_insert_with(BTreeMap::new);
                    for (source, n) in per_source {
                        *merged.entry(source.clone()).or_insert(0) += n.as_u64().unwrap_or(0);
                    }
                }
            }
            ComponentKind::GrawWriter => {}
        }
    }
    // 設定にある CoBo が status に居ない(不達)場合も鍵は出す(欠落を隠さない)。
    for cobo in &params.cobos {
        frames.entry(cobo.id.to_string()).or_insert(0);
    }

    // v1.53(TODO/185): リンクごとの送受。**材料が 1 つでも欠ければ `None`**(半端な和や 0 埋めを
    // しない —— 差 `lost_*` が「欠けが無い」に化けるため)。
    let receivers: Vec<&ComponentStatus> = statuses
        .iter()
        .filter(|s| matches!(s.kind, ComponentKind::Receiver { .. }))
        .collect();
    let decoders: Vec<&ComponentStatus> = statuses
        .iter()
        .filter(|s| s.kind == ComponentKind::Decoder)
        .collect();
    let graw_writers: Vec<&ComponentStatus> = statuses
        .iter()
        .filter(|s| s.kind == ComponentKind::GrawWriter)
        .collect();
    let receiver_sum = |key: &str| {
        if receivers.len() < params.cobos.len() {
            return None; // 設定にある receiver が status に居ない
        }
        sum_all(&receivers, |s| s.metric_u64(key))
    };
    let decoder_sum = |key: &str| sum_all(&decoders, |s| s.metric_u64(key));

    let received = receiver_sum("frames");
    let overflowed = receiver_sum("overflow_frames");
    let decoder_link_dropped_frames = receiver_sum("decoder_link_dropped_frames");
    let graw_frames = sum_all(&graw_writers, |s| graw_frames(s.metrics.as_ref()));
    let decoder_frames_in = decoder_sum("frames_in");
    let decoder_abandoned = decoder_sum("batches_abandoned")
        .zip(decoder_sum("eos_abandoned"))
        .map(|(batches, eos)| batches + eos);
    // Σ receiver(frames − overflow_frames)= graw / decoder リンクへ出たはずの本数。差は符号つき。
    let delivered = received
        .zip(overflowed)
        .and_then(|(received, overflowed)| signed_diff(received, overflowed));
    let lost_receiver_to_graw = delivered
        .zip(graw_frames)
        .and_then(|(delivered, graw)| delivered.checked_sub(i64::try_from(graw).ok()?));
    let lost_receiver_to_decoder = delivered
        .zip(decoder_link_dropped_frames)
        .zip(decoder_frames_in)
        .and_then(|((delivered, dropped), frames_in)| {
            delivered
                .checked_sub(i64::try_from(dropped).ok()?)?
                .checked_sub(i64::try_from(frames_in).ok()?)
        });

    Counters {
        events_built: None,
        events_incomplete: None,
        late_fragments: None,
        frames,
        overflow_frames: Some(overflow_frames),
        malformed: Some(malformed),
        acquisition_restarts,
        frames_before_restart: frames_before_restart.map(FramesBeforeRestart::PerSource),
        frames_type1,
        frames_type2,
        compact_size_mismatch,
        decoder_link_dropped_frames,
        decoder_link_dropped_batches: receiver_sum("decoder_link_dropped_batches"),
        decoder_link_dropped_heartbeats: receiver_sum("decoder_link_dropped_heartbeats"),
        graw_frames,
        decoder_frames_in,
        decoder_fragments_out: decoder_sum("fragments_out"),
        decoder_abandoned,
        lost_receiver_to_graw,
        lost_receiver_to_decoder,
        // EOS の顛末が要るので呼び手が [`root_complete`] で埋める(ここでは観測していない)。
        root_complete: None,
    }
}

/// `group` の全員が申告したときだけの和(1 人でも `None` なら `None`)。相手が 1 つも
/// 居なければ(= 観測していない)`None`。
fn sum_all(
    group: &[&ComponentStatus],
    read: impl Fn(&ComponentStatus) -> Option<u64>,
) -> Option<u64> {
    if group.is_empty() {
        return None;
    }
    group.iter().map(|s| read(s)).sum()
}

/// `a − b` を符号つきで(どちらかが i64 に収まらなければ `None`)。
fn signed_diff(a: u64, b: u64) -> Option<i64> {
    i64::try_from(a).ok()?.checked_sub(i64::try_from(b).ok()?)
}

/// graw-writer の metrics から「書いたフレーム数」= `asad[].frames` の合計 + `ctrl_frames`
/// (SPEC §9.2 v1.53 `graw_frames`、TODO/185)。どれか欠ければ `None`。停止第二段の進捗の
/// 観測(`Sequencer::downstream_progress`)と run_stop の `graw_frames` が同じ定義を使う。
pub(super) fn graw_frames(metrics: Option<&Value>) -> Option<u64> {
    let metrics = metrics?;
    let asad = metrics
        .get("asad")?
        .as_array()?
        .iter()
        .map(|entry| entry.get("frames").and_then(Value::as_u64))
        .sum::<Option<u64>>()?;
    let ctrl = metrics.get("ctrl_frames").and_then(Value::as_u64)?;
    Some(asad + ctrl)
}

/// run_stop の `events_built` / `events_incomplete` / `late_fragments`(SPEC §9.2 v1.54、TODO/204 C)。
/// root-sink がその run を閉じた(`idle`)status を観測できた run ではその値(v1.50 以降は run ごと)、
/// 観測できなければ触らない(= [`counters_from`] の `None` のまま)。
pub(super) fn fill_root_sink_counters(counters: &mut Counters, closed: Option<&StatusPayload>) {
    if let Some(closed) = closed {
        counters.events_built = Some(closed.events_built);
        counters.events_incomplete = Some(closed.events_incomplete);
        counters.late_fragments = Some(closed.late_fragments);
    }
}

/// `run_stop.counters.root_complete`(SPEC §1.4-8 v1.53)= `overflow_frames == 0 ∧
/// decoder_link_dropped_frames == 0 ∧ eos_closed ∧ decoder_abandoned == 0`。材料のどれかが
/// `None` なら `None`(TODO/185 §1-C「どれかが null なら null」)。
pub(super) fn root_complete(counters: &Counters, eos_closed: bool) -> Option<bool> {
    let overflow = counters.overflow_frames?;
    let dropped = counters.decoder_link_dropped_frames?;
    let abandoned = counters.decoder_abandoned?;
    Some(overflow == 0 && dropped == 0 && eos_closed && abandoned == 0)
}

/// `GET /api/status` の `stopping.decoder_backlog`(TODO/185)= decoder の `frames_in −
/// fragments_out`。decoder が不達・キーが無ければ `None`。`fragments_out > frames_in` は
/// 起きないはずの食い違いなので `None` にして `notes` に残す(silent にしない)。
pub(super) fn decoder_backlog(
    statuses: &[ComponentStatus],
    notes: &mut Vec<String>,
) -> Option<u64> {
    let decoder = statuses.iter().find(|s| s.kind == ComponentKind::Decoder)?;
    let frames_in = decoder.metric_u64("frames_in")?;
    let fragments_out = decoder.metric_u64("fragments_out")?;
    let backlog = frames_in.checked_sub(fragments_out);
    if backlog.is_none() {
        notes.push(format!(
            "{}: fragments_out {fragments_out} > frames_in {frames_in} — decoder_backlog is null",
            decoder.name
        ));
    }
    backlog
}

/// `GET /api/status` の `stopping.root_sink_backlog`(TODO/204 C、187 ③ の「残り N」)=
/// Σ receiver(`frames − overflow_frames − decoder_link_dropped_frames`)− root-sink の `fragments`
/// = decoder リンクへ出たのに root-sink がまだ受け取っていないフレーム数(ctrl フレームは
/// フラグメントにならないので 1 run あたり数本は残る)。
///
/// どれか取れなければ `None`: 設定にある receiver が居ない・キーが無い・root-sink の status が
/// 無い / `fragments` を出さない古い root-sink / **停止中の run と違う run の status**(別の run の
/// 数を引くと意味の無い値になる)。負になる食い違いは `None` + `notes`(silent にしない)。
pub(super) fn root_sink_backlog(
    params: &ControllerParams,
    statuses: &[ComponentStatus],
    root_sink: Option<&StatusPayload>,
    run: Option<u32>,
    notes: &mut Vec<String>,
) -> Option<u64> {
    let root_sink = root_sink.filter(|status| Some(status.run) == run)?;
    let fragments = root_sink.fragments?;
    let receivers: Vec<&ComponentStatus> = statuses
        .iter()
        .filter(|s| matches!(s.kind, ComponentKind::Receiver { .. }))
        .collect();
    if receivers.is_empty() || receivers.len() < params.cobos.len() {
        return None;
    }
    let mut delivered = 0u64;
    for receiver in receivers {
        let frames = receiver.metric_u64("frames")?;
        let overflow = receiver.metric_u64("overflow_frames")?;
        let dropped = receiver.metric_u64("decoder_link_dropped_frames")?;
        let Some(sent) = frames
            .checked_sub(overflow)
            .and_then(|n| n.checked_sub(dropped))
        else {
            notes.push(format!(
                "{}: frames {frames} < overflow_frames {overflow} + decoder_link_dropped_frames \
                 {dropped} — root_sink_backlog is null",
                receiver.name
            ));
            return None;
        };
        delivered += sent;
    }
    let backlog = delivered.checked_sub(fragments);
    if backlog.is_none() {
        notes.push(format!(
            "root-sink fragments {fragments} > frames sent to the decoder {delivered} — \
             root_sink_backlog is null"
        ));
    }
    backlog
}

/// run_stop の `files`(SPEC §9.2)。graw-writer の metrics の実績をそのまま写す。
pub(super) fn files_from(statuses: &[ComponentStatus]) -> Vec<OutputFile> {
    let mut files = Vec::new();
    for status in statuses {
        if status.kind != ComponentKind::GrawWriter {
            continue;
        }
        let entries = status
            .metrics
            .as_ref()
            .and_then(|m| m.get("files"))
            .and_then(Value::as_array);
        for entry in entries.into_iter().flatten() {
            let path = entry.get("path").and_then(Value::as_str);
            let bytes = entry.get("bytes").and_then(Value::as_u64);
            if let (Some(path), Some(bytes)) = (path, bytes) {
                files.push(OutputFile {
                    path: path.to_string(),
                    bytes,
                });
            }
        }
    }
    files
}
