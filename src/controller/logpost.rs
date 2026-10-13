use super::*;

// ---------------------------------------------------------------------
// ログ投稿 PULL(SPEC §2.3 LogPost / §3.2)
// ---------------------------------------------------------------------

/// LogPost を受けてログブックへ流すスレッド。
///
/// ペイロードは「JSONL 1 レコード分の JSON 文字列」(§2.3)。`ts` / `seq` は controller が
/// 付ける(§9.1)ので、投稿側は付けない。読めなかった投稿は捨てるが**数えて警告する**。
pub(super) fn spawn_log_post_thread(
    bind: String,
    logbook: Arc<Mutex<LogbookWriter>>,
    counters: Arc<LogPostCounters>,
    slow_latest: Arc<Mutex<SlowLatest>>,
    stop: Arc<AtomicBool>,
) -> Result<(std::thread::JoinHandle<()>, String), ControllerError> {
    let context = zmq::Context::new();
    let socket = context
        .socket(zmq::PULL)
        .map_err(|source| ControllerError::LogPull {
            bind: bind.clone(),
            source,
        })?;
    crate::zmq_helper::apply_pull_hwm(&socket).map_err(|source| ControllerError::LogPull {
        bind: bind.clone(),
        source,
    })?;
    socket
        .set_rcvtimeo(LOG_PULL_POLL_MS)
        .map_err(|source| ControllerError::LogPull {
            bind: bind.clone(),
            source,
        })?;
    socket
        .bind(&bind)
        .map_err(|source| ControllerError::LogPull {
            bind: bind.clone(),
            source,
        })?;
    let endpoint = match socket.get_last_endpoint() {
        Ok(Ok(endpoint)) => endpoint,
        _ => bind.clone(),
    };
    info!(%endpoint, "log post PULL bound");

    let handle = std::thread::Builder::new()
        .name("controller-logpost".to_string())
        .spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let message = match socket.recv_bytes(0) {
                    Ok(message) => message,
                    Err(zmq::Error::EAGAIN) => continue,
                    Err(e) => {
                        error!(error = %e, "log post recv failed");
                        continue;
                    }
                };
                match serde_json::from_slice::<LogbookRecord>(&message) {
                    Ok(record) => {
                        // SPEC §9.2 v1.31: slowctl の VALUE/ALARM/RECOVER は run_start /
                        // run_stop に載せる直近値でもある。ログブックへ書く前に拾う
                        // (書き込みが失敗しても「今の圧力」は失わない)。
                        if matches!(record, LogbookRecord::Slow { .. }) {
                            let ts = crate::logbook::now_rfc3339_millis();
                            apply_slow_record(&mut lock(&slow_latest), &record, &ts);
                        }
                        // TODO/121 §B: 定期値 VALUE は直近値の更新だけ(時系列は別の保存先)。
                        // IDENT / ALARM / RECOVER / ERROR はまれで安全上の意味があるので書く。
                        if matches!(&record, LogbookRecord::Slow { event, .. } if event == "VALUE")
                        {
                            counters
                                .slow_values_not_logged
                                .fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                        let mut writer = lock(&logbook);
                        match writer.append(record) {
                            Ok(_) => {
                                counters.accepted.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(e) => {
                                counters.rejected.fetch_add(1, Ordering::Relaxed);
                                error!(error = %e, "cannot append a log post");
                            }
                        }
                    }
                    Err(e) => {
                        counters.rejected.fetch_add(1, Ordering::Relaxed);
                        warn!(error = %e, payload = %String::from_utf8_lossy(&message),
                              "log post is not a logbook record — dropped");
                    }
                }
            }
            info!("log post task stopped");
        })
        .map_err(|source| ControllerError::Io {
            path: PathBuf::from("controller-logpost"),
            source,
        })?;
    Ok((handle, endpoint))
}
