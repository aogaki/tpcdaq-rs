use super::*;

// ---------------------------------------------------------------------
// run_start の材料(ジオメトリ)
// ---------------------------------------------------------------------

/// `run_start.geometry`(SPEC §9.2)と `expected_fragments` をジオメトリファイルから作る。
///
/// `expected_fragments` = ジオメトリが申告している (cobo, asad) 在庫。root-sink の
/// EventBuilder が使う期待集合と同じ定義(`tools/root_sink/eb_core.hpp`)。
/// [`crate::geometry::Geometry::asad_inventory`](025)を直接呼ぶ — `lookup` を経由しないので
/// `unmapped_hit_count` を汚さない(旧実装は同じ理由で `dump_tsv` の先頭 2 列をパースして
/// いたが、025 で専用アクセサに置き換えた)。
pub(super) fn geometry_record(path: &Path) -> Result<(GeometryRecord, Vec<(u32, u32)>), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let sha256 = format!("{:x}", hasher.finalize());

    let text = String::from_utf8_lossy(&bytes);
    let geometry = crate::geometry::parse(&text);
    let fragments = expected_fragments(&geometry);
    if fragments.is_empty() {
        warn!(path = %path.display(), "geometry declares no (cobo, asad) — expected_fragments is empty");
    }
    Ok((
        GeometryRecord {
            path: path.display().to_string(),
            sha256,
        },
        fragments,
    ))
}

/// 025: [`crate::geometry::Geometry::asad_inventory`] へ委譲するだけの薄いラッパ
/// (呼び出し側 [`geometry_record`] のドキュメントを参照)。
pub(super) fn expected_fragments(geometry: &Geometry) -> Vec<(u32, u32)> {
    geometry.asad_inventory()
}

/// logbook `run_start` の `(config_id, config_ids)`(SPEC §9.2 v1.13)。
///
/// - 3 相同値: `config_id` = その値、`config_ids` は**出さない**
///   (省略 ≠ 空値 —— 025 の counters と同じ nullable 規律)。
/// - 非同値: `config_id` = **configure 相**の id(run のデータ取得条件を代表する相)、
///   `config_ids` に 3 相全部。
pub(super) fn run_start_config_fields(ids: &ConfigIds) -> (String, Option<ConfigIds>) {
    match ids.same_value() {
        Some(id) => (id.to_string(), None),
        None => (ids.configure.clone(), Some(ids.clone())),
    }
}

/// 正式な run 開始 TS(SPEC §6.4 v1.8)。graw-writer のファイル名 TS と同じ書式にして、
/// 将来そのまま採用できるようにしておく(`src/graw_writer.rs` の `local_timestamp`)。
pub(super) fn run_start_timestamp() -> String {
    chrono::Local::now()
        .format("%Y-%m-%dT%H:%M:%S%.3f")
        .to_string()
}

// ---------------------------------------------------------------------
// REST ハンドラ(SPEC §8.1)
// ---------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(super) struct AcquireRequest {
    operator: String,
    passphrase: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct TokenRequest {
    token: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct RunStartRequest {
    token: String,
    #[serde(default)]
    comment: String,
}

/// `POST /api/run/next`(SPEC v1.10 §8.1)。`next_run` はまず `Value` のまま受けて
/// 手でバリデーションする — `u32`/`u64` で直接受けると、型不一致(文字列・小数・負値)を
/// axum の `Json` extractor が本ハンドラより先に弾いてしまい、発注書どおりの
/// 「それ以外は 400」を一律に返せない(extractor 側の rejection は 422 になりうる)。
#[derive(Debug, Deserialize)]
pub(super) struct RunNextRequest {
    token: String,
    next_run: Value,
}

#[derive(Debug, Deserialize)]
pub(super) struct CommentRequest {
    author: String,
    text: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct SinceQuery {
    #[serde(default)]
    since_seq: u64,
}

pub(super) type Reply = (StatusCode, Json<Value>);

pub(super) fn ok_json(body: Value) -> Reply {
    (StatusCode::OK, Json(body))
}

pub(super) fn err_json(code: StatusCode, message: impl Into<String>) -> Reply {
    (code, Json(json!({"error": message.into()})))
}

/// 全コンポーネント + ecc + 自身の相(SPEC §8.1)。閲覧系なので認証は要らない。
pub(super) async fn get_status(State(controller): State<Arc<Controller>>) -> Reply {
    let reply = tokio::task::spawn_blocking(move || {
        let mut transport = controller
            .transport
            .with_command_timeout(controller.params.status_timeout);
        let mut notes = Vec::new();
        let components: Vec<Value> = {
            let mut sequencer = Sequencer::new(&controller.params, &mut transport);
            sequencer
                .collect_status(&mut notes)
                .iter()
                .map(ComponentStatus::to_json)
                .collect()
        };
        // `ecc_error` は **GET の error フラグ**をそのまま載せるだけ(SPEC §8.2 v1.14 =
        // TODO/043)。controller は解釈も加工もしない —— 表示の判断は UI(P4)の仕事。
        // ecc-bridge に届かなかったときは `state` と同じく `Unknown`: 「聞けていない」を
        // 「ECC がエラー無しと申告した(NO_ERR)」に化けさせない。
        let ecc = match transport.ecc(&json!({"action": "status"})) {
            Ok(reply) => json!({
                "ok": reply.ok,
                "state": reply.state,
                "error": reply.error,
                "ecc_error": reply.ecc_error,
            }),
            Err(e) => json!({"ok": false, "state": "Unknown", "error": e, "ecc_error": "Unknown"}),
        };
        // 078 レーン B §10-4: `params <id> · exceptions N` の元(UI はレーン C)。
        // status は必ず 200 を返す既存の姿勢を崩さない —— params 側の不整合は `notes` へ
        // 積んで `params: null` にする(このリクエスト自体は失敗させない)。
        let params_status = params_status_json(&controller, &mut notes);
        let run_state = lock(&controller.run);
        let operator = lock(&controller.session)
            .as_ref()
            .map(|s| s.operator.clone());
        json!({
            "phase": run_state.phase.to_string(),
            "run": run_state.active.as_ref().map(|active| json!({
                "run": active.run,
                "operator": active.operator,
                "comment": active.comment,
                "started_ts": active.started_ts,
                "elapsed_s": active.started_at.elapsed().as_secs_f64(),
            })),
            "operator": operator,
            "components": components,
            "ecc": ecc,
            "params": params_status,
            "log_posts": {
                "accepted": controller.log_posts.accepted.load(Ordering::Relaxed),
                "rejected": controller.log_posts.rejected.load(Ordering::Relaxed),
                "slow_values_not_logged": controller
                    .log_posts
                    .slow_values_not_logged
                    .load(Ordering::Relaxed),
            },
            "notes": notes,
        })
    })
    .await;
    match reply {
        Ok(body) => ok_json(body),
        Err(e) => err_json(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("status task: {e}"),
        ),
    }
}

/// `GET /api/status` の `params: {id, exceptions} | null`(078 レーン B §10-4)。
/// current.json が無い、ecc_config_dir 未設定、render 失敗のいずれも `null` + `notes` へ
/// 一言(silent にしない)。status 応答そのものは落とさない。
pub(super) fn params_status_json(controller: &Controller, notes: &mut Vec<String>) -> Value {
    let current = match controller.current_params() {
        Ok(Some(current)) => current,
        Ok(None) => return Value::Null,
        Err(e) => {
            notes.push(format!("params: {e}"));
            return Value::Null;
        }
    };
    let Some(ecc_config_dir) = &controller.params.ecc_config_dir else {
        notes.push("params: ecc_config_dir is not configured".to_string());
        return Value::Null;
    };
    match controller.render_with_template_dir(ecc_config_dir, &current.params) {
        Ok(rendered) => json!({
            "id": rendered.config_id,
            "exceptions": current.params.exceptions.len(),
        }),
        Err(e) => {
            notes.push(format!("params: {e}"));
            Value::Null
        }
    }
}

/// 操作権の取得(常に横取り可、横取りは監査へ)。
pub(super) async fn post_acquire(
    State(controller): State<Arc<Controller>>,
    Json(request): Json<AcquireRequest>,
) -> Reply {
    if request.passphrase != controller.params.passphrase {
        controller.audit(
            "control/acquire",
            json!({"operator": request.operator}),
            &request.operator,
            false,
            Some("wrong passphrase".to_string()),
        );
        warn!(
            operator = request.operator,
            "control/acquire refused — wrong passphrase"
        );
        return err_json(StatusCode::FORBIDDEN, "wrong passphrase");
    }
    let (token, preempted) = controller.acquire(&request.operator);
    controller.audit(
        "control/acquire",
        json!({"operator": request.operator, "preempted": preempted}),
        &request.operator,
        true,
        None,
    );
    if let Some(previous) = &preempted {
        warn!(
            operator = request.operator,
            previous, "control token taken over"
        );
    }
    ok_json(json!({"token": token, "preempted": preempted}))
}

pub(super) async fn post_release(
    State(controller): State<Arc<Controller>>,
    Json(request): Json<TokenRequest>,
) -> Reply {
    let operator = match controller.authorize(&request.token) {
        Ok(operator) => operator,
        Err(why) => return unauthorized(&controller, "control/release", json!({}), why),
    };
    *lock(&controller.session) = None;
    controller.audit("control/release", json!({}), &operator, true, None);
    ok_json(json!({"released": true}))
}

/// token 無し / 不一致は 401 相当 + 監査(SPEC §8.1)。
pub(super) fn unauthorized(
    controller: &Controller,
    action: &str,
    params: Value,
    why: &str,
) -> Reply {
    controller.audit(
        action,
        params,
        "<unauthorized>",
        false,
        Some(why.to_string()),
    );
    warn!(action, why, "rejected an unauthorized request");
    err_json(StatusCode::UNAUTHORIZED, why)
}

/// run 開始(§1.3 のシーケンス)。
pub(super) async fn post_run_start(
    State(controller): State<Arc<Controller>>,
    Json(request): Json<RunStartRequest>,
) -> Reply {
    let operator = match controller.authorize(&request.token) {
        Ok(operator) => operator,
        Err(why) => {
            return unauthorized(
                &controller,
                "run/start",
                json!({"comment": request.comment}),
                why,
            )
        }
    };

    // 相の占有(Idle からしか始められない = 二重 start を弾く)。
    // 124(Fable B-2): power コマンドの実行中も弾く(HV ON の検査の後に run が始まって、
    // ランプアップ中のデータを取らないように)。同じロック区間で判定する。
    {
        let mut run_state = lock(&controller.run);
        let refused = if run_state.phase != Phase::Idle {
            Some(format!("controller is {} — not Idle", run_state.phase))
        } else if run_state.power_inflight.is_some() {
            Some(RUN_START_POWER_IN_FLIGHT.to_string())
        } else {
            None
        };
        if let Some(why) = refused {
            drop(run_state);
            controller.audit(
                "run/start",
                json!({"comment": request.comment}),
                &operator,
                false,
                Some(why.clone()),
            );
            return err_json(StatusCode::CONFLICT, why);
        }
        run_state.phase = Phase::Starting;
    }

    // ブロッキングタスクが panic しても相を Starting のまま固めない(制御不能にしない)。
    let recovery = Arc::clone(&controller);
    let reply = tokio::task::spawn_blocking(move || {
        let outcome = start_run_blocking(&controller, &operator, &request.comment);
        let mut run_state = lock(&controller.run);
        match outcome {
            Ok((run, ts)) => {
                run_state.phase = Phase::Running;
                run_state.active = Some(ActiveRun {
                    run,
                    operator,
                    comment: request.comment,
                    started_ts: ts,
                    started_at: Instant::now(),
                });
                ok_json(json!({"run": run, "phase": "Running"}))
            }
            Err(detail) => {
                run_state.phase = Phase::Idle;
                run_state.active = None;
                err_json(StatusCode::INTERNAL_SERVER_ERROR, detail)
            }
        }
    })
    .await;
    match reply {
        Ok(reply) => reply,
        Err(e) => {
            let mut run_state = lock(&recovery.run);
            run_state.phase = Phase::Idle;
            run_state.active = None;
            error!(error = %e, "run start task died — back to Idle");
            err_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("run start task: {e}"),
            )
        }
    }
}

/// run 開始の本体(ブロッキング)。成功したら (run 番号, 開始 TS)。
pub(super) fn start_run_blocking(
    controller: &Controller,
    operator: &str,
    comment: &str,
) -> Result<(u32, String), String> {
    // ジオメトリはコンポーネントに触る前に読む(run_start レコードに必須 — SPEC §9.2)。
    let geometry = match geometry_record(&controller.params.geometry_path) {
        Ok(geometry) => geometry,
        Err(detail) => {
            controller.audit(
                "run/start",
                json!({"comment": comment}),
                operator,
                false,
                Some(detail.clone()),
            );
            return Err(detail);
        }
    };

    // 078 レーン B §10-3: current.json があれば prepare/configure の config_id を
    // その `<id>` に差し替える(describe は TOML のまま)。無ければ従来どおり TOML の id
    // (= `params_snapshot` が `None`)。**現在の params が壊れている / render できないのに
    // 黙って TOML の id へ後退はしない** —— run のデータ取得条件が記録と食い違うほうが
    // 静かなフォールバックより危険(CLAUDE.md「silent failure を作らない」)。
    let params_snapshot = match run_start_params_snapshot(controller) {
        Ok(snapshot) => snapshot,
        Err(detail) => {
            controller.audit(
                "run/start",
                json!({"comment": comment}),
                operator,
                false,
                Some(detail.clone()),
            );
            return Err(detail);
        }
    };
    let params_id = params_snapshot
        .as_ref()
        .map(|(_, rendered)| rendered.config_id.clone());
    let params_field = params_snapshot.as_ref().map(|(current, rendered)| {
        json!({
            "id": rendered.config_id,
            "sha256": current.sha256,
            "exceptions": current.params.exceptions.len(),
        })
    });

    let mut params_for_run = controller.params.clone();
    if let Some(id) = &params_id {
        params_for_run.config_ids = ConfigIds {
            describe: params_for_run.config_ids.describe.clone(),
            prepare: id.clone(),
            configure: id.clone(),
        };
    }

    // run 番号採番(015、controller 単一書き手)。
    let run = match crate::state::take_next_run(&controller.state_path) {
        Ok(run) => run,
        Err(e) => {
            let detail = format!("cannot take the next run number: {e}");
            controller.audit(
                "run/start",
                json!({"comment": comment}),
                operator,
                false,
                Some(detail.clone()),
            );
            return Err(detail);
        }
    };

    let run_start_ts = run_start_timestamp();
    let mut transport = controller.transport.clone();
    let result = {
        let mut sequencer = Sequencer::new(&params_for_run, &mut transport);
        sequencer.start_run(run, comment, &run_start_ts)
    };

    // 115: 歩き戻しで閉じた前の run の run_stop を、新しい run の RunStart より**前に**書く
    // (start が後で落ちても前の run は閉じているので書く)。要約は audit へ(null = 閉じた run 無し)。
    let walked_back_run = match &result {
        Ok(report) => report.walked_back_run.as_ref(),
        Err(failure) => failure.walked_back_run.as_deref(),
    };
    let previous_run_stop =
        walked_back_run.map(|closed| append_walked_back_run_stop(controller, closed));

    match result {
        Ok(report) => {
            let (geometry, expected_fragments) = geometry;
            let (config_id, config_ids) = run_start_config_fields(&params_for_run.config_ids);
            // ここから先(run_start をログブックへ書いた後)は **絶対に採番を巻き戻さない** ——
            // 記録に残った run 番号を次の run が撃ち直すことになる(034 = 論点 C)。
            controller.append(LogbookRecord::RunStart {
                actor: ACTOR.to_string(),
                run,
                config_id,
                config_ids,
                geometry,
                cobos: controller.params.cobo_listens(),
                operator: operator.to_string(),
                comment: comment.to_string(),
                expected_fragments,
                slow: controller.slow_snapshot(),
                params: params_field,
            });
            controller.audit(
                "run/start",
                json!({"run": run, "comment": comment, "links": report.links.iter()
                    .map(|l| json!({"cobo": l.cobo_id, "router_ip": l.router_ip, "router_port": l.router_port}))
                    .collect::<Vec<_>>(),
                    // 034: Arm を撃ち直したなら回数と所要を必ず残す(silent 禁止)。
                    "arm_retries": report.arm_retries.iter().map(ArmRetry::to_json).collect::<Vec<_>>(),
                    // 034: 歩き戻しの着地点(`Idle` / `Off` 以外では run は始まらない)。
                    "ecc_state_after_reset": report.ecc_state_after_reset,
                    // 036: **歩き戻しの各段**。実機で歩き戻しが効いているかはここで機械確認する
                    // (2 本目以降が `["status->Ready","breakup->Prepared","reset->Described",
                    //  "reset->Idle"]` になっていること)。
                    "ecc_walk_back": report.ecc_walk_back,
                    // 113: Idle でなかったコンポーネントを Reset で歩き戻した跡(空 = 全員 Idle)。
                    // 空でない = 前の run の停止で Reset が届かなかった、という運用の可視化。
                    "component_walk_back": report.component_walk_back,
                    // 115: 歩き戻しで閉じた前の run の run_stop の要約(`{"run", "notes"}`、
                    // run が null = 番号が決まらず書かなかった)。null = 閉じた run 無し。
                    "previous_run_stop": previous_run_stop,
                    // 078 レーン B §10-3: この run が使った params の `<id>`(無ければ null =
                    // TOML の config_id だけで走った)。
                    "params_id": params_id}),
                operator,
                true,
                None,
            );
            info!(run, ts = run_start_ts, "run started");
            Ok((run, run_start_ts))
        }
        Err(failure) => {
            let detail = failure.to_string();
            // 034 = 論点 C: **run_start をログブックへ書く前**に落ちたので、採番を返す。
            // controller は `next_run` の単一書き手で、この時点の相は `Starting`
            // (`run/next` も次の `run/start` も弾かれる)ため、割り込みが無い。
            let rewound = match crate::state::set_next_run(&controller.state_path, run) {
                Ok(()) => {
                    info!(run, "run start failed — next_run rewound to this number");
                    true
                }
                Err(e) => {
                    error!(run, error = %e, "cannot rewind next_run — the number is spent");
                    false
                }
            };
            controller.audit(
                "run/start",
                json!({
                    "run": run,
                    "comment": comment,
                    "stage": failure.stage,
                    "next_run_rewound": rewound,
                    "previous_run_stop": previous_run_stop,
                    "params_id": params_id,
                }),
                operator,
                false,
                Some(detail.clone()),
            );
            Err(detail)
        }
    }
}

/// 歩き戻しで閉じた前の run の `run_stop` を書く(115、Fable 裁定)。**新しい run の
/// `RunStart` より前に**呼ぶ(start が後で落ちても前の run は閉じているので書く)。
///
/// * 番号が決まらなければ(`closed.run` が `Err`)書かない —— 理由は要約の notes へ。
/// * `reason: "abort:controller-restart"`、`ok: false`。EOS の顛末と counters / files は
///   歩き戻しの材料そのまま。
/// * `duration_s` はログブックの**同じ番号の** `run_start.ts` から。無ければ `null`。
/// * ログブックの**最後の** `run_start` は照合だけに使い、番号が違えば note を残す。
///
/// 戻り値は audit `run/start.previous_run_stop` に載せる要約 `{"run": N | null, "notes": [...]}`
/// (書かなかった理由・照合の食い違いを silent にしない)。
pub(super) fn append_walked_back_run_stop(
    controller: &Controller,
    closed: &WalkedBackRun,
) -> Value {
    let mut notes = closed.notes.clone();
    let run = match &closed.run {
        Ok(run) => *run,
        Err(why) => {
            notes.push(why.clone());
            return json!({"run": null, "notes": notes});
        }
    };

    let records = match crate::logbook::read_since(&controller.logbook_path, 0) {
        Ok(result) => result.records,
        Err(e) => {
            notes.push(format!("cannot read the logbook: {e}"));
            Vec::new()
        }
    };
    let starts: Vec<(u32, &str)> = records
        .iter()
        .filter_map(|line| match &line.record {
            LogbookRecord::RunStart { run, .. } => Some((*run, line.ts.as_str())),
            _ => None,
        })
        .collect();
    if let Some(&(last, _)) = starts.last() {
        if last != run {
            notes.push(format!(
                "the last run_start in the logbook is run {last}, not {run}"
            ));
        }
    }
    let duration_s = match starts.iter().rev().find(|(r, _)| *r == run) {
        None => {
            notes.push(format!(
                "no run_start for run {run} in the logbook — duration_s is null"
            ));
            None
        }
        Some((_, ts)) => {
            let seconds = seconds_since(ts, chrono::Local::now().fixed_offset());
            if seconds.is_none() {
                notes.push(format!(
                    "run_start.ts {ts:?} of run {run} is not RFC3339 — duration_s is null"
                ));
            }
            seconds
        }
    };

    warn!(
        run,
        eos_closed = closed.eos_closed,
        ?notes,
        "writing run_stop for the run the walk-back closed (the controller restarted mid-run)"
    );
    controller.append(LogbookRecord::RunStop {
        actor: ACTOR.to_string(),
        run,
        duration_s,
        ok: false,
        reason: "abort:controller-restart".to_string(),
        forced_eos: closed.forced_eos,
        eos_closed: closed.eos_closed,
        counters: closed.counters.clone(),
        files: closed.files.clone(),
        slow: controller.slow_snapshot(),
    });
    json!({"run": run, "notes": notes})
}

/// RFC3339 の `ts` から `now` までの秒(オフセットが違っても同じ瞬間で比べる)。
/// 読めなければ `None`。
pub(super) fn seconds_since(ts: &str, now: chrono::DateTime<chrono::FixedOffset>) -> Option<f64> {
    let started = chrono::DateTime::parse_from_rfc3339(ts).ok()?;
    Some((now - started).num_milliseconds() as f64 / 1000.0)
}

/// [`start_run_blocking`] の params 部分だけを切り出したもの(078 レーン B §10-3)。
/// current.json が無ければ `Ok(None)`(= 従来どおり TOML の config_id)。
/// current.json はあるが検証/render に失敗したら `Err`(黙って TOML へ後退しない)。
#[allow(clippy::type_complexity)]
pub(super) fn run_start_params_snapshot(
    controller: &Controller,
) -> Result<Option<(CurrentParams, crate::params::Rendered)>, String> {
    let Some(current) = controller.current_params()? else {
        return Ok(None);
    };
    let ecc_config_dir = controller.params.ecc_config_dir.as_ref().ok_or_else(|| {
        "params/current.json exists but ecc_config_dir is not configured — \
         refusing to start a run with a stale config_id"
            .to_string()
    })?;
    let rendered = controller.render_with_template_dir(ecc_config_dir, &current.params)?;
    Ok(Some((current, rendered)))
}

/// run 番号の手動設定(SPEC v1.10 §8.1)。次の `run/start` から有効になる。
///
/// 検証順序は既存の `post_ecc`(未知 action → 400)と揃えた: authorize(401)→ 入力の形
/// (400)→ 相(Phase が Idle か、409)→ 成功。
pub(super) async fn post_run_next(
    State(controller): State<Arc<Controller>>,
    Json(request): Json<RunNextRequest>,
) -> Reply {
    let operator = match controller.authorize(&request.token) {
        Ok(operator) => operator,
        Err(why) => return unauthorized(&controller, "run/next", json!({}), why),
    };

    // 正整数のみ(0・負・非数はすべて 400)。`Value::as_u64` は非負整数の JSON 数値だけ
    // `Some` を返す(小数・負値・文字列・真偽値・null はすべて `None`)。
    let next_run = match request
        .next_run
        .as_u64()
        .filter(|&n| n > 0 && n <= u32::MAX as u64)
    {
        Some(n) => n as u32,
        None => {
            let why = format!(
                "next_run must be a positive integer, got {}",
                request.next_run
            );
            controller.audit(
                "run/next",
                json!({"next_run": request.next_run}),
                &operator,
                false,
                Some(why.clone()),
            );
            return err_json(StatusCode::BAD_REQUEST, why);
        }
    };

    {
        let run_state = lock(&controller.run);
        if run_state.phase != Phase::Idle {
            let why = format!(
                "controller is {} — cannot set next_run while a run is active",
                run_state.phase
            );
            drop(run_state);
            controller.audit(
                "run/next",
                json!({"next_run": next_run}),
                &operator,
                false,
                Some(why.clone()),
            );
            return err_json(StatusCode::CONFLICT, why);
        }
    }

    match crate::state::set_next_run(&controller.state_path, next_run) {
        Ok(()) => {
            controller.audit(
                "run/next",
                json!({"next_run": next_run}),
                &operator,
                true,
                None,
            );
            info!(next_run, "next run number set manually");
            ok_json(json!({"next_run": next_run}))
        }
        Err(e) => {
            let why = format!("cannot persist next_run: {e}");
            controller.audit(
                "run/next",
                json!({"next_run": next_run}),
                &operator,
                false,
                Some(why.clone()),
            );
            err_json(StatusCode::INTERNAL_SERVER_ERROR, why)
        }
    }
}

/// run 停止(§1.3)。コンポーネントが Error に落ちていれば中止経路(v1.6)へ切り替える。
pub(super) async fn post_run_stop(
    State(controller): State<Arc<Controller>>,
    Json(request): Json<TokenRequest>,
) -> Reply {
    let operator = match controller.authorize(&request.token) {
        Ok(operator) => operator,
        Err(why) => return unauthorized(&controller, "run/stop", json!({}), why),
    };

    let active = {
        let mut run_state = lock(&controller.run);
        if run_state.phase != Phase::Running {
            let why = format!("controller is {} — no run to stop", run_state.phase);
            drop(run_state);
            controller.audit("run/stop", json!({}), &operator, false, Some(why.clone()));
            return err_json(StatusCode::CONFLICT, why);
        }
        run_state.phase = Phase::Stopping;
        run_state.active.clone()
    };

    // 停止タスクが panic しても相を Stopping のまま固めない(次の run を出せなくしない)。
    let recovery = Arc::clone(&controller);
    let reply = tokio::task::spawn_blocking(move || {
        let body = stop_run_blocking(&controller, &operator, active);
        let mut run_state = lock(&controller.run);
        run_state.phase = Phase::Idle;
        run_state.active = None;
        ok_json(body)
    })
    .await;
    match reply {
        Ok(reply) => reply,
        Err(e) => {
            let mut run_state = lock(&recovery.run);
            run_state.phase = Phase::Idle;
            run_state.active = None;
            error!(error = %e, "run stop task died — back to Idle");
            err_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("run stop task: {e}"),
            )
        }
    }
}

pub(super) fn stop_run_blocking(
    controller: &Controller,
    operator: &str,
    active: Option<ActiveRun>,
) -> Value {
    let mut transport = controller.transport.clone();

    // 中止判定(SPEC §1.4-3): 走行中のコンポーネントが Error なら異常停止として扱う。
    let mode = {
        let mut notes = Vec::new();
        let mut sequencer = Sequencer::new(&controller.params, &mut transport);
        let statuses = sequencer.collect_status(&mut notes);
        let broken: Vec<&str> = statuses
            .iter()
            .filter(|s| s.state == Some(ComponentState::Error) || !s.reachable)
            .map(|s| s.name.as_str())
            .collect();
        if broken.is_empty() {
            StopMode::Normal
        } else {
            warn!(components = ?broken, "stopping as an abort — component(s) in Error/unreachable");
            StopMode::Abort(format!("component-error:{}", broken.join(",")))
        }
    };

    let report = {
        let mut sequencer = Sequencer::new(&controller.params, &mut transport);
        sequencer.stop_run(mode)
    };

    let run = active.as_ref().map(|a| a.run).unwrap_or(0);
    let duration_s = active
        .as_ref()
        .map(|a| a.started_at.elapsed().as_secs_f64())
        .unwrap_or(0.0);
    controller.append(LogbookRecord::RunStop {
        actor: ACTOR.to_string(),
        run,
        duration_s: Some(duration_s),
        ok: report.ok,
        reason: report.reason.clone(),
        // 033-A: 停止の顛末は **run_stop(唯一の run 台帳)** にも載せる。ここに無いと
        // audit 行と突き合わせないと「どう閉じたか / 閉じ損ねたか」が読めない
        // (SPEC §9.2 v1.12。041 でこの不履行が実 run で確認された)。
        forced_eos: report.forced_eos,
        eos_closed: report.eos_closed,
        counters: report.counters.clone(),
        files: report.files.clone(),
        slow: controller.slow_snapshot(),
    });
    controller.audit(
        "run/stop",
        json!({
            "run": run,
            "reason": report.reason,
            "forced_eos": report.forced_eos,
            "eos_closed": report.eos_closed,
        }),
        operator,
        report.ok,
        (!report.notes.is_empty()).then(|| report.notes.join("; ")),
    );
    info!(
        run,
        reason = report.reason,
        forced_eos = report.forced_eos,
        "run stopped"
    );
    json!({
        "run": run,
        "ok": report.ok,
        "reason": report.reason,
        "forced_eos": report.forced_eos,
        "eos_closed": report.eos_closed,
        "notes": report.notes,
    })
}

/// ECC の段階操作(R6: GET controller と同じ操作感)。
pub(super) async fn post_ecc(
    State(controller): State<Arc<Controller>>,
    UrlPath(action): UrlPath<String>,
    Json(request): Json<TokenRequest>,
) -> Reply {
    const ACTIONS: [&str; 7] = [
        "describe",
        "prepare",
        "configure",
        "start",
        "stop",
        "breakup",
        "reset",
    ];
    let endpoint = format!("ecc/{action}");
    let operator = match controller.authorize(&request.token) {
        Ok(operator) => operator,
        Err(why) => return unauthorized(&controller, &endpoint, json!({}), why),
    };
    if !ACTIONS.contains(&action.as_str()) {
        let why = format!("unknown ecc action: {action}");
        controller.audit(&endpoint, json!({}), &operator, false, Some(why.clone()));
        return err_json(StatusCode::BAD_REQUEST, why);
    }

    let reply = tokio::task::spawn_blocking(move || {
        let mut transport = controller.transport.clone();
        let request_body = match action.as_str() {
            // DataLinkSet は receiver が**実際に bind した**ポートで組む(SPEC §8.2)。
            "configure" => match configure_request_now(&controller, &mut transport) {
                Ok(body) => body,
                Err(why) => {
                    controller.audit(&endpoint, json!({}), &operator, false, Some(why.clone()));
                    return err_json(StatusCode::CONFLICT, why);
                }
            },
            // describe / prepare は**その相の** ConfigId を載せる(SPEC §8.2 v1.13)。
            // start / stop / breakup / reset は id を使わない = 載せない。
            other => match controller.params.config_ids.for_action(other) {
                Some(config_id) => json!({"action": action, "config_id": config_id}),
                None => json!({"action": action}),
            },
        };
        match transport.ecc(&request_body) {
            Ok(reply) => {
                controller.audit(
                    &endpoint,
                    request_body.clone(),
                    &operator,
                    reply.ok,
                    (!reply.ok).then(|| reply.error.clone()),
                );
                ok_json(json!({"ok": reply.ok, "state": reply.state, "error": reply.error}))
            }
            Err(e) => {
                controller.audit(&endpoint, request_body, &operator, false, Some(e.clone()));
                err_json(StatusCode::BAD_GATEWAY, e)
            }
        }
    })
    .await;
    match reply {
        Ok(reply) => reply,
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, format!("ecc task: {e}")),
    }
}

/// いまの receiver の bind 実績から DataLinkSet を組む(段階操作の `configure` 用)。
pub(super) fn configure_request_now(
    controller: &Controller,
    transport: &mut ZmqTransport,
) -> Result<Value, String> {
    let mut sequencer = Sequencer::new(&controller.params, transport);
    let mut links = Vec::new();
    for component in controller.params.components.iter() {
        let ComponentKind::Receiver { cobo_id } = component.kind else {
            continue;
        };
        let response = sequencer.apply(component, &Command::GetStatus)?;
        let link = sequencer
            .router_link(cobo_id, &response)
            .map_err(|e| format!("{}: {e} (Arm the receivers first)", component.name))?;
        links.push(link);
    }
    Ok(sequencer.configure_request(&links))
}

/// ログブックの読み出し(閲覧系 = 認証なし、SPEC §8.1 の二層)。
pub(super) async fn get_logbook(
    State(controller): State<Arc<Controller>>,
    Query(query): Query<SinceQuery>,
) -> Reply {
    let path = controller.logbook_path.clone();
    let reply =
        tokio::task::spawn_blocking(move || crate::logbook::read_since(&path, query.since_seq))
            .await;
    match reply {
        Ok(Ok(result)) => ok_json(json!({
            "records": result.records,
            "tail_corrupt": result.tail_corrupt,
        })),
        Ok(Err(e)) => err_json(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        Err(e) => err_json(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("logbook task: {e}"),
        ),
    }
}

/// コメント投稿(R11)。**状態を変えない**ので token は要求しない(記録は `author` が持つ)。
pub(super) async fn post_comment(
    State(controller): State<Arc<Controller>>,
    Json(request): Json<CommentRequest>,
) -> Reply {
    controller.append(LogbookRecord::Comment {
        actor: "ui".to_string(),
        author: request.author,
        text: request.text,
    });
    ok_json(json!({"appended": true}))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::controller::test_support::*;
    // -----------------------------------------------------------------
    // ジオメトリ由来の run_start 材料(SPEC §9.2)
    // -----------------------------------------------------------------

    /// `expected_fragments` = ジオメトリが申告する (cobo, asad) 在庫。
    /// sha256 は空でない 64 桁の 16 進(ファイルの実ハッシュ)。
    #[test]
    fn geometry_record_reports_the_sha256_and_the_expected_fragments() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/geometry_2cobo_fake.dat");
        let (record, fragments) = geometry_record(&path).unwrap();
        assert_eq!(record.path, path.display().to_string());
        assert_eq!(record.sha256.len(), 64);
        assert!(record.sha256.chars().all(|c| c.is_ascii_hexdigit()));
        // 合成 2-CoBo フィクスチャの申告どおり: CoBo 0 は AsAd 1 枚、CoBo 1 は 2 枚
        // (`grep -v '^#' tests/fixtures/geometry_2cobo_fake.dat | cut -f4,5 | sort -u` の実測)。
        assert_eq!(fragments, vec![(0, 0), (1, 0), (1, 1)]);
    }

    /// 同じファイルなら同じハッシュ、1 バイト違えば違うハッシュ(取り違え検出)。
    #[test]
    fn geometry_sha256_changes_with_the_file_content() {
        let dir = std::env::temp_dir().join(format!("tpcdaq-geo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.dat");
        let b = dir.join("b.dat");
        std::fs::write(&a, "0\t0\t0\t0\tU\t1\t2\n").unwrap();
        std::fs::write(&b, "0\t0\t0\t1\tU\t1\t2\n").unwrap();
        let sha_a = geometry_record(&a).unwrap().0.sha256;
        let sha_b = geometry_record(&b).unwrap().0.sha256;
        assert_eq!(sha_a, geometry_record(&a).unwrap().0.sha256);
        assert_ne!(sha_a, sha_b);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------
    // 115 — 歩き戻しで閉じた前の run の run_stop(台帳側)
    // -----------------------------------------------------------------

    /// 前の run の `run_start`(controller 自身が書いたものと同じ形)。
    fn run_start_of(run: u32) -> LogbookRecord {
        LogbookRecord::RunStart {
            actor: ACTOR.to_string(),
            run,
            config_id: "mock".to_string(),
            config_ids: None,
            geometry: GeometryRecord {
                path: "/tmp/tpcdaq-mock/geometry.dat".to_string(),
                sha256: "ab12".to_string(),
            },
            cobos: vec![CoboListen {
                id: 0,
                listen: "0.0.0.0:46005".to_string(),
            }],
            operator: "aogaki".to_string(),
            comment: "alpha".to_string(),
            expected_fragments: vec![(0, 0), (0, 1)],
            slow: None,
            params: None,
        }
    }

    /// 歩き戻しが返す材料(非対称な値: CoBo 0 = 68 フレーム、graw 2 本)。
    fn walked_back(run: Result<u32, String>) -> WalkedBackRun {
        let mut frames = BTreeMap::new();
        frames.insert("0".to_string(), 68);
        WalkedBackRun {
            run,
            forced_eos: true,
            eos_closed: true,
            counters: Counters {
                events_built: None,
                events_incomplete: None,
                late_fragments: None,
                frames,
                overflow_frames: Some(0),
                malformed: Some(2),
                acquisition_restarts: None,
                frames_before_restart: None,
            },
            files: vec![
                OutputFile {
                    path: "run0058/CoBo0_AsAd0_ts_0000.graw".to_string(),
                    bytes: 6528,
                },
                OutputFile {
                    path: "run0058/CoBo0_AsAd1_ts_0000.graw".to_string(),
                    bytes: 6432,
                },
            ],
            notes: Vec::new(),
        }
    }

    fn logbook_records(controller: &Controller) -> Vec<LogbookRecord> {
        crate::logbook::read_since(&controller.logbook_path, 0)
            .unwrap()
            .records
            .into_iter()
            .map(|line| line.record)
            .collect()
    }

    /// `duration_s` は前の run の `run_start.ts` から測る。オフセットが違っても同じ瞬間で比べる。
    /// 手計算: 08:42:00.000+03:00 = 05:42:00.000Z、now = 05:42:15.250Z → 15.25 s。
    #[test]
    fn seconds_since_measures_across_utc_offsets() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-12T05:42:15.250+00:00").unwrap();
        assert_eq!(
            seconds_since("2026-10-12T08:42:00.000+03:00", now),
            Some(15.25)
        );
        assert_eq!(seconds_since("not a timestamp", now), None);
    }

    /// **115 の本体(台帳側)**: 番号が決まった前の run には run_stop を書く ——
    /// `reason: "abort:controller-restart"`、`ok: false`、EOS の顛末と counters / files は
    /// 歩き戻しの材料そのまま、`duration_s` はその run の run_start からの経過。
    /// 戻り値は audit `run/start.previous_run_stop` に載る要約。
    #[test]
    fn a_walked_back_run_gets_a_run_stop_marked_controller_restart() {
        let controller = test_controller("orphan-stop");
        controller.append(run_start_of(58));

        let summary = append_walked_back_run_stop(&controller, &walked_back(Ok(58)));

        assert_eq!(summary, json!({"run": 58, "notes": []}));
        let records = logbook_records(&controller);
        assert_eq!(records.len(), 2, "{records:?}");
        let LogbookRecord::RunStop {
            run,
            duration_s,
            ok,
            reason,
            forced_eos,
            eos_closed,
            counters,
            files,
            ..
        } = &records[1]
        else {
            panic!("run_stop が書かれていない: {records:?}");
        };
        assert_eq!(*run, 58);
        assert!(!ok);
        assert_eq!(reason, "abort:controller-restart");
        assert!(*forced_eos);
        assert!(*eos_closed);
        let expected = walked_back(Ok(58));
        assert_eq!(counters, &expected.counters);
        assert_eq!(files, &expected.files);
        // run_start を書いた直後なので経過は 0 s 以上・数秒未満。
        let duration = duration_s.expect("run_start があるので duration_s は数値");
        assert!((0.0..10.0).contains(&duration), "{duration}");
        let _ = std::fs::remove_dir_all(&controller.params.output_root);
    }

    /// 番号が決まらない(申告の食い違い)なら run_stop は書かない。理由は要約の notes へ
    /// (audit に残る = silent にしない)。
    #[test]
    fn a_walked_back_run_without_an_agreed_number_writes_no_run_stop() {
        let controller = test_controller("orphan-disagree");
        controller.append(run_start_of(58));
        let why = "components disagree on the previous run number \
                   (receiver0=58, decoder=57) — run_stop not written"
            .to_string();

        let summary = append_walked_back_run_stop(&controller, &walked_back(Err(why.clone())));

        assert_eq!(summary, json!({"run": null, "notes": [why]}));
        let records = logbook_records(&controller);
        assert!(
            !records
                .iter()
                .any(|r| matches!(r, LogbookRecord::RunStop { .. })),
            "{records:?}"
        );
        let _ = std::fs::remove_dir_all(&controller.params.output_root);
    }

    /// ログブックの最後の run_start は**照合だけ**に使う: 番号が違えば note を残し、
    /// 同じ番号の run_start が無ければ `duration_s` は null(この経路に限り nullable)。
    /// run_stop そのものはコンポーネントの申告した番号で書く。
    #[test]
    fn a_walked_back_run_missing_from_the_logbook_gets_a_null_duration_and_notes() {
        let controller = test_controller("orphan-no-start");
        controller.append(run_start_of(57));

        let summary = append_walked_back_run_stop(&controller, &walked_back(Ok(58)));

        assert_eq!(
            summary,
            json!({"run": 58, "notes": [
                "the last run_start in the logbook is run 57, not 58",
                "no run_start for run 58 in the logbook — duration_s is null",
            ]})
        );
        let records = logbook_records(&controller);
        let Some(LogbookRecord::RunStop {
            run, duration_s, ..
        }) = records.last()
        else {
            panic!("run_stop が書かれていない: {records:?}");
        };
        assert_eq!(*run, 58);
        assert_eq!(*duration_s, None);
        let _ = std::fs::remove_dir_all(&controller.params.output_root);
    }
}
