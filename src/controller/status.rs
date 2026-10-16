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

    fn metric_u64(&self, key: &str) -> Option<u64> {
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
    }
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
