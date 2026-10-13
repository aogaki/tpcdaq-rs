use super::*;

// ---------------------------------------------------------------------
// HV / LV の ch ごとの ON/OFF(TODO/124 レーン B、SPEC §8.1 / §9.2 / §11 v1.42)
// ---------------------------------------------------------------------

/// slowctl の power REP への REQ の待ち時間(124 §2)。slowctl 側の上限(機器での実行 5 s)
/// より長く取り、slowctl が自分の `"timeout"` を返す余地を残す。
pub(super) const SLOW_CMD_TIMEOUT: Duration = Duration::from_secs(10);

/// audit の `action`(SPEC §9.2 v1.42)。
pub(super) const SLOW_POWER_ACTION: &str = "slow/power";
pub(super) const SLOW_CMD_NOT_CONFIGURED: &str = "slow_cmd is not configured";
pub(super) const HV_ON_REFUSED: &str = "HV ON is allowed only while no run is active";
pub(super) const LV_OFF_REFUSED: &str =
    "LV OFF is refused while a run is active — stop the run first";
pub(super) const POWER_IN_FLIGHT: &str = "another power command is in flight";
pub(super) const RUN_START_POWER_IN_FLIGHT: &str =
    "slow-control power command in flight — retry in a few seconds";
/// 504 の文言に必ず入れる(Fable B-3)。送った後に返事が無い = 機器が動いたかは分からない。
pub(super) const STATE_UNKNOWN: &str = "state unknown — check the table";

/// 規則(SPEC §11 v1.42、ユーザー裁定 2026-10-12): HV ON(ランプアップ中の偽事象)と
/// LV OFF(AsAd の電源を落とす)は相 `Idle` のときだけ。HV OFF と LV ON はいつでも可。
/// Starting / Stopping も Running と同じく「run が動いている」側に数える(Fable B-2)。
pub(super) fn power_rule(device_type: &str, on: bool, phase: Phase) -> Result<(), &'static str> {
    if phase == Phase::Idle {
        return Ok(());
    }
    match (device_type, on) {
        ("hivolta", true) => Err(HV_ON_REFUSED),
        ("hmp2020", false) => Err(LV_OFF_REFUSED),
        _ => Ok(()),
    }
}

pub(super) fn on_off(on: bool) -> &'static str {
    if on {
        "ON"
    } else {
        "OFF"
    }
}

/// `POST /api/slow/power {token, device, ch, on}`(SPEC §8.1 v1.42)。
///
/// device / ch / on は `Value` のまま受けて手で検証する(`RunNextRequest` と同じ理由 ——
/// 型違いを `Json` extractor が先に弾くと、400 と audit の 1 行(Fable B-4)を出せない)。
/// token の省略も同じく 401 + audit にする。
#[derive(Debug, Deserialize)]
pub(super) struct SlowPowerRequest {
    #[serde(default)]
    token: String,
    #[serde(default)]
    device: Value,
    #[serde(default)]
    ch: Value,
    #[serde(default)]
    on: Value,
}

/// 検証済みの power コマンド。
#[derive(Debug)]
pub(super) struct PowerCommand {
    device: String,
    device_type: String,
    ch: u8,
    on: bool,
}

/// 400 の判定(124 §2): device が `[[slowctl.device]]` に在り、type が hivolta / hmp2020、
/// ch がその型の範囲(hivolta 0–7、hmp2020 1–2)、on が真偽値。
pub(super) fn validate_power_request(
    devices: &[(String, String)],
    request: &SlowPowerRequest,
) -> Result<PowerCommand, String> {
    let Some(device) = request.device.as_str() else {
        return Err(format!(
            "device must be a slow-control device name, got {}",
            request.device
        ));
    };
    let Some((_, device_type)) = devices.iter().find(|(name, _)| name == device) else {
        return Err(format!(
            "unknown slow-control device {device:?} (not in [[slowctl.device]])"
        ));
    };
    let channels = match device_type.as_str() {
        "hivolta" => 0..=7u8,
        "hmp2020" => 1..=2u8,
        other => {
            return Err(format!(
                "device {device:?} is a {other} — only hivolta / hmp2020 have a power switch"
            ))
        }
    };
    let Some(ch) = request
        .ch
        .as_u64()
        .and_then(|ch| u8::try_from(ch).ok())
        .filter(|ch| channels.contains(ch))
    else {
        return Err(format!(
            "ch must be an integer in {}–{} for {device_type}, got {}",
            channels.start(),
            channels.end(),
            request.ch
        ));
    };
    let Some(on) = request.on.as_bool() else {
        return Err(format!("on must be true or false, got {}", request.on));
    };
    Ok(PowerCommand {
        device: device.to_string(),
        device_type: device_type.clone(),
        ch,
        on,
    })
}

/// audit `slow/power` の `params`(SPEC §9.2 v1.42: `{device, ch, on, phase, before, after}`)。
/// device / ch / on は**受け取ったまま**(400 の行にも何が来たかを残す)。
pub(super) fn power_audit_params(
    request: &SlowPowerRequest,
    phase: Phase,
    before: &Value,
    after: &Value,
) -> Value {
    json!({
        "device": request.device,
        "ch": request.ch,
        "on": request.on,
        "phase": phase.to_string(),
        "before": before,
        "after": after,
    })
}

/// slowctl に届かなかった / 返事が読めなかった理由。
#[derive(Debug)]
pub(super) enum SlowCmdFailure {
    /// 送ったが期限内に返事が無い(audit の error は `"timeout"` —— Fable B-3)。
    Timeout,
    /// ソケット・接続・送信の失敗。
    Unreachable(String),
    /// 返事が JSON でない。
    BadReply(String),
}

/// slowctl へ power の REQ を 1 往復(124 §1 の要求形)。ソケットは呼び出し毎に作って捨てる
/// (`ZmqTransport::ecc` と同じ —— REQ の状態機械を持ち越さない)。linger 0。
pub(super) fn slow_power_request(
    context: &zmq::Context,
    endpoint: &str,
    command: &PowerCommand,
    timeout: Duration,
) -> Result<Value, SlowCmdFailure> {
    let timeout_ms = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
    let fail = |what: &str, e: zmq::Error| {
        SlowCmdFailure::Unreachable(format!("slowctl {endpoint}: {what}: {e}"))
    };
    let socket = context
        .socket(zmq::REQ)
        .map_err(|e| fail("cannot create REQ socket", e))?;
    socket
        .set_rcvtimeo(timeout_ms)
        .map_err(|e| fail("cannot set rcvtimeo", e))?;
    socket
        .set_sndtimeo(timeout_ms)
        .map_err(|e| fail("cannot set sndtimeo", e))?;
    socket
        .set_linger(0)
        .map_err(|e| fail("cannot set linger", e))?;
    socket
        .connect(endpoint)
        .map_err(|e| fail("cannot connect", e))?;

    let payload = json!({
        "op": "power",
        "device": command.device,
        "ch": command.ch,
        "on": command.on,
    });
    socket
        .send(payload.to_string().as_bytes(), 0)
        .map_err(|e| fail("send failed", e))?;
    let reply = match socket.recv_bytes(0) {
        Ok(reply) => reply,
        Err(zmq::Error::EAGAIN) => return Err(SlowCmdFailure::Timeout),
        Err(e) => return Err(fail("recv failed", e)),
    };
    serde_json::from_slice(&reply).map_err(|e| {
        SlowCmdFailure::BadReply(format!(
            "slowctl reply is not JSON: {e} ({})",
            String::from_utf8_lossy(&reply)
        ))
    })
}

/// HV / LV の ch ごとの ON/OFF(SPEC §8.1 / §11 v1.42、TODO/124 + Fable B 1〜4)。
///
/// 判定の順は `put_params` と揃えた: authorize(401)→ `slow_cmd` の有無(503)→ 入力(400)→
/// 短いロックの中で「印 → 規則」(409)→ 印を立ててロックを放し、REQ はブロッキングタスクで
/// (200 / 502 / 504)。**どの経路でも audit `slow/power` を 1 行**書く(Fable B-4)。
pub(super) async fn post_slow_power(
    State(controller): State<Arc<Controller>>,
    Json(request): Json<SlowPowerRequest>,
) -> Reply {
    let phase_now = lock(&controller.run).phase;
    let operator = match controller.authorize(&request.token) {
        Ok(operator) => operator,
        Err(why) => {
            let params = power_audit_params(&request, phase_now, &Value::Null, &Value::Null);
            return unauthorized(&controller, SLOW_POWER_ACTION, params, why);
        }
    };
    let refuse = |code: StatusCode, phase: Phase, why: String| {
        let params = power_audit_params(&request, phase, &Value::Null, &Value::Null);
        controller.audit(
            SLOW_POWER_ACTION,
            params,
            &operator,
            false,
            Some(why.clone()),
        );
        warn!(%code, device = %request.device, ch = %request.ch, on = %request.on, %phase, why,
              "slow/power refused");
        err_json(code, why)
    };

    let Some(endpoint) = controller.params.slow_cmd.clone() else {
        return refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            phase_now,
            SLOW_CMD_NOT_CONFIGURED.to_string(),
        );
    };
    let command = match validate_power_request(&controller.params.slow_devices, &request) {
        Ok(command) => command,
        Err(why) => return refuse(StatusCode::BAD_REQUEST, phase_now, why),
    };

    // Fable B-2: 「同じロック区間で検査と送信」は取らない(std Mutex を REQ の 10 s のあいだ
    // 握れない)。run/start の `Starting` と同じく、短いロックで判定して印を立てる。
    // audit はロックを手放してから(logbook のロックと入れ子にしない)。
    let (phase, refused) = {
        let mut run_state = lock(&controller.run);
        let phase = run_state.phase;
        let refused = match &run_state.power_inflight {
            Some(inflight) => Some(format!(
                "{POWER_IN_FLIGHT} ({} ch {} {} by {})",
                inflight.device,
                inflight.ch,
                on_off(inflight.on),
                inflight.operator
            )),
            None => power_rule(&command.device_type, command.on, phase)
                .err()
                .map(str::to_string),
        };
        if refused.is_none() {
            run_state.power_inflight = Some(PowerInflight {
                device: command.device.clone(),
                ch: command.ch,
                on: command.on,
                operator: operator.clone(),
            });
        }
        (phase, refused)
    };
    if let Some(why) = refused {
        return refuse(StatusCode::CONFLICT, phase, why);
    }
    info!(
        device = command.device,
        ch = command.ch,
        on = command.on,
        %phase,
        operator,
        "slow/power: sending to slowctl"
    );

    // ブロッキングタスクが panic しても印を立てたままにしない(run/start の復旧と同じ形)。
    let recovery = Arc::clone(&controller);
    let recovery_params = power_audit_params(&request, phase, &Value::Null, &Value::Null);
    let recovery_operator = operator.clone();
    let reply = tokio::task::spawn_blocking(move || {
        let outcome = slow_power_request(
            &controller.transport.context,
            &endpoint,
            &command,
            SLOW_CMD_TIMEOUT,
        );
        lock(&controller.run).power_inflight = None;
        finish_slow_power(&controller, &request, phase, &operator, &command, outcome)
    })
    .await;
    match reply {
        Ok(reply) => reply,
        Err(e) => {
            lock(&recovery.run).power_inflight = None;
            let why = format!("slow/power task: {e}");
            recovery.audit(
                SLOW_POWER_ACTION,
                recovery_params,
                &recovery_operator,
                false,
                Some(why.clone()),
            );
            error!(error = %e, "slow/power task died — power_inflight cleared");
            err_json(StatusCode::INTERNAL_SERVER_ERROR, why)
        }
    }
}

/// slowctl の返事(または不達)を REST の応答 + audit 1 行にする(124 §2、Fable B-3)。
///
/// * `ok:true` → 200、slowctl の応答**そのまま**。
/// * `ok:false` + `error` → 502、slowctl の応答そのまま(error 文言・before を含む)。
/// * 形の崩れた返事 → 502(何が来たかを文言に入れる)。
/// * 返事が無い / 届かない → 504、文言に「state unknown — check the table」。
pub(super) fn finish_slow_power(
    controller: &Controller,
    request: &SlowPowerRequest,
    phase: Phase,
    operator: &str,
    command: &PowerCommand,
    outcome: Result<Value, SlowCmdFailure>,
) -> Reply {
    let (code, body, before, after, error) = match outcome {
        Ok(reply) => {
            let before = reply.get("before").cloned().unwrap_or(Value::Null);
            let after = reply.get("after").cloned().unwrap_or(Value::Null);
            let ok = reply.get("ok").and_then(Value::as_bool);
            let error = reply
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_string);
            match (ok, error) {
                (Some(true), _) => (StatusCode::OK, reply, before, after, None),
                (Some(false), Some(error)) => {
                    (StatusCode::BAD_GATEWAY, reply, before, after, Some(error))
                }
                _ => {
                    let why = format!("slowctl reply is not {{ok, error, ...}}: {reply}");
                    let body = json!({ "error": why });
                    (StatusCode::BAD_GATEWAY, body, before, after, Some(why))
                }
            }
        }
        Err(SlowCmdFailure::BadReply(why)) => (
            StatusCode::BAD_GATEWAY,
            json!({ "error": why }),
            Value::Null,
            Value::Null,
            Some(why),
        ),
        Err(SlowCmdFailure::Timeout) => (
            StatusCode::GATEWAY_TIMEOUT,
            json!({ "error": format!(
                "slowctl did not reply within {} s — {STATE_UNKNOWN}",
                SLOW_CMD_TIMEOUT.as_secs()
            ) }),
            Value::Null,
            Value::Null,
            Some("timeout".to_string()),
        ),
        Err(SlowCmdFailure::Unreachable(why)) => (
            StatusCode::GATEWAY_TIMEOUT,
            json!({ "error": format!("{why} — {STATE_UNKNOWN}") }),
            Value::Null,
            Value::Null,
            Some(why),
        ),
    };

    let ok = code == StatusCode::OK;
    controller.audit(
        SLOW_POWER_ACTION,
        power_audit_params(request, phase, &before, &after),
        operator,
        ok,
        error.clone(),
    );
    if ok {
        info!(device = command.device, ch = command.ch, on = command.on,
              %before, %after, "slow/power done");
    } else {
        warn!(%code, device = command.device, ch = command.ch, on = command.on,
              %before, %after, error = error.as_deref().unwrap_or(""), "slow/power failed");
    }
    (code, Json(body))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::controller::test_support::*;
    // -----------------------------------------------------------------
    // slow/power(TODO/124 レーン B、SPEC §11 v1.42)
    // -----------------------------------------------------------------

    /// 規則の全表(4 相 × HV/LV × ON/OFF = 16 通り): HV ON と LV OFF は Idle だけ、
    /// HV OFF と LV ON は常時(Starting / Stopping も Running と同じく「run が動いている」側)。
    #[test]
    fn the_power_rule_allows_hv_on_and_lv_off_only_while_idle() {
        for phase in [
            Phase::Idle,
            Phase::Starting,
            Phase::Running,
            Phase::Stopping,
        ] {
            let idle = phase == Phase::Idle;
            assert_eq!(
                power_rule("hivolta", true, phase).is_ok(),
                idle,
                "HV ON @ {phase}"
            );
            assert_eq!(
                power_rule("hmp2020", false, phase).is_ok(),
                idle,
                "LV OFF @ {phase}"
            );
            assert_eq!(
                power_rule("hivolta", false, phase),
                Ok(()),
                "HV OFF @ {phase}"
            );
            assert_eq!(
                power_rule("hmp2020", true, phase),
                Ok(()),
                "LV ON @ {phase}"
            );
        }
        assert_eq!(
            power_rule("hivolta", true, Phase::Starting),
            Err("HV ON is allowed only while no run is active")
        );
        assert_eq!(
            power_rule("hmp2020", false, Phase::Stopping),
            Err("LV OFF is refused while a run is active — stop the run first")
        );
    }

    /// Fable B テスト「HV ON は Starting / Stopping でも 409」。統合テストでは Starting /
    /// Stopping の窓を確実には作れないので、相を直接置いてハンドラを叩く。規則で断るので
    /// slowctl へは送らない(`slow_cmd` は届かない口のままでよい)。409 も audit に 1 行ずつ。
    #[tokio::test(flavor = "multi_thread")]
    async fn hv_on_and_lv_off_are_409_while_starting_or_stopping_and_each_is_audited() {
        let mut params = params_with(&[]);
        params.output_root =
            std::env::temp_dir().join(format!("tpcdaq-controller-power-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&params.output_root);
        params.slow_cmd = Some("tcp://127.0.0.1:1".to_string());
        params.slow_devices = vec![
            ("hv".to_string(), "hivolta".to_string()),
            ("lv".to_string(), "hmp2020".to_string()),
        ];
        let controller = Arc::new(Controller::new(params).unwrap());
        let (token, _) = controller.acquire("aogaki");

        for phase in [Phase::Starting, Phase::Stopping] {
            lock(&controller.run).phase = phase;
            for (device, ch, on, why) in [
                (
                    "hv",
                    3,
                    true,
                    "HV ON is allowed only while no run is active",
                ),
                (
                    "lv",
                    2,
                    false,
                    "LV OFF is refused while a run is active — stop the run first",
                ),
            ] {
                let request: SlowPowerRequest = serde_json::from_value(
                    json!({"token": token, "device": device, "ch": ch, "on": on}),
                )
                .unwrap();
                let (status, Json(body)) =
                    post_slow_power(State(Arc::clone(&controller)), Json(request)).await;
                assert_eq!(status, StatusCode::CONFLICT, "{device} {on} @ {phase}");
                assert_eq!(body["error"], json!(why));
            }
        }
        assert!(
            lock(&controller.run).power_inflight.is_none(),
            "規則で断ったら印は立てない"
        );

        let audits: Vec<Value> = crate::logbook::read_since(&controller.logbook_path, 0)
            .unwrap()
            .records
            .into_iter()
            .filter_map(|line| match line.record {
                LogbookRecord::Audit {
                    action,
                    params,
                    ok,
                    error,
                    ..
                } if action == "slow/power" => {
                    Some(json!({"params": params, "ok": ok, "error": error}))
                }
                _ => None,
            })
            .collect();
        // 手計算: 2 相 × 2 操作 = 4 行。
        assert_eq!(audits.len(), 4, "{audits:?}");
        assert_eq!(
            audits[0],
            json!({
                "params": {"device": "hv", "ch": 3, "on": true, "phase": "Starting",
                           "before": null, "after": null},
                "ok": false,
                "error": "HV ON is allowed only while no run is active",
            })
        );
        assert_eq!(audits[3]["params"]["phase"], json!("Stopping"));
        assert_eq!(audits[3]["params"]["device"], json!("lv"));

        let _ = std::fs::remove_dir_all(&controller.params.output_root);
    }
}
