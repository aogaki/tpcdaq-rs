use super::*;

// ---------------------------------------------------------------------
// パラメータ JSON REST(078 レーン B §10、SPEC §3.3)
// ---------------------------------------------------------------------

/// GET/PUT `/api/params` と PUT の 400/503 応答が共有する「ecc_config_dir 未設定 = 503」。
/// **黙って無効にしない**(CLAUDE.md)—— 発注書 §10 のとおり `{"error": "..."}` は固定文言。
pub(super) const ECC_CONFIG_DIR_NOT_CONFIGURED: &str = "ecc_config_dir is not configured";

/// GET/PUT `/api/params` の共通の応答本体(078 §10「GET と同じ形を返す」)。
pub(super) fn params_body(
    ecc_config_dir: &Path,
    current: &CurrentParams,
    rendered: &crate::params::Rendered,
) -> Value {
    let configure_path = ecc_config_dir.join(&rendered.configure_file_name);
    let prepare_path = ecc_config_dir.join(&rendered.prepare_file_name);
    let params_value: Value = serde_json::from_str(&current.text).unwrap_or(Value::Null);
    json!({
        "id": rendered.config_id,
        "sha256": current.sha256,
        "exceptions": current.params.exceptions.len(),
        "params": params_value,
        "files": {
            "configure": configure_path.display().to_string(),
            "prepare": prepare_path.display().to_string(),
        },
    })
}

/// `GET /api/params`(閲覧系 = 認証なし)。current.json が無ければ 404、
/// ecc_config_dir が無ければ 503(078 §10)。
pub(super) async fn get_params(State(controller): State<Arc<Controller>>) -> Reply {
    let reply = tokio::task::spawn_blocking(move || {
        let Some(ecc_config_dir) = controller.params.ecc_config_dir.clone() else {
            return err_json(
                StatusCode::SERVICE_UNAVAILABLE,
                ECC_CONFIG_DIR_NOT_CONFIGURED,
            );
        };
        let current = match controller.current_params() {
            Ok(Some(current)) => current,
            Ok(None) => return err_json(StatusCode::NOT_FOUND, "no params configured"),
            Err(e) => return err_json(StatusCode::INTERNAL_SERVER_ERROR, e),
        };
        match controller.render_with_template_dir(&ecc_config_dir, &current.params) {
            // GET は「保存済みの current.json をそのまま映す」だけの閲覧系なので、
            // render に失敗するのは current.json の PUT 後にテンプレートが消えた等の
            // サーバ側の不整合 —— 400(このリクエストの入力の問題)ではなく 500。
            Ok(rendered) => ok_json(params_body(&ecc_config_dir, &current, &rendered)),
            Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, e),
        }
    })
    .await;
    match reply {
        Ok(reply) => reply,
        Err(e) => err_json(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("params task: {e}"),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct ParamsPutRequest {
    token: String,
    params: Value,
}

/// `PUT /api/params { token, params }`(078 §10)。検証順序: authorize(401)→
/// ecc_config_dir の有無(503)→ JSON のスキーマ検証(400)→ 相が Idle か(409)→
/// テンプレート読込 + render(400)→ 書き込み。**同じ JSON → 同じファイル**
/// (既存の configure/prepare/`<id>.json` は上書きしない)。
pub(super) async fn put_params(
    State(controller): State<Arc<Controller>>,
    Json(request): Json<ParamsPutRequest>,
) -> Reply {
    let operator = match controller.authorize(&request.token) {
        Ok(operator) => operator,
        Err(why) => return unauthorized(&controller, "params/put", json!({}), why),
    };

    let Some(ecc_config_dir) = controller.params.ecc_config_dir.clone() else {
        controller.audit(
            "params/put",
            json!({}),
            &operator,
            false,
            Some(ECC_CONFIG_DIR_NOT_CONFIGURED.to_string()),
        );
        return err_json(
            StatusCode::SERVICE_UNAVAILABLE,
            ECC_CONFIG_DIR_NOT_CONFIGURED,
        );
    };

    // axum の `Json` extractor は本文を `Value` として parse 済みなので、受け取ったバイト列
    // そのものはもう無い。`Value` を再直列化した**この文字列**を正本として扱う
    // (`serde_json::Map` は `preserve_order` 機能を有効にしていないので `BTreeMap` 裏付け =
    // キーは常に辞書順。空白の揺れは吸収され、同じ内容の JSON は常に同じバイト列になる ——
    // これは §3-3「同じ JSON → 同じファイル」を REST 越しでも保つのに必要)。
    let text = request.params.to_string();
    let params = match crate::params::Params::from_json_str(&text) {
        Ok(params) => params,
        Err(e) => {
            let why = e.to_string();
            controller.audit("params/put", json!({}), &operator, false, Some(why.clone()));
            return err_json(StatusCode::BAD_REQUEST, why);
        }
    };

    let reply = tokio::task::spawn_blocking(move || {
        // 113 §2-3: **相の確認と書き込みを同じロック区間に**置く。確認の後でロックを手放すと、
        // その隙に別クライアントの `run/start` が相を Starting にして current.json を読み、
        // 書きかけ / 直後に差し替わる params で run が始まりうる。ロックを握っている間は
        // start が相を取れないので、「PUT が先に済む」か「PUT が 409」のどちらかに決まる。
        // audit はロックを手放してから(logbook のロックと入れ子にしない)。
        let saved = {
            let run_state = lock(&controller.run);
            if run_state.phase != Phase::Idle {
                Err((
                    StatusCode::CONFLICT,
                    format!(
                        "controller is {} — not Idle (次 run から効くという半端は作らない)",
                        run_state.phase
                    ),
                ))
            } else {
                save_params(&controller, &ecc_config_dir, &params, &text)
            }
        };
        let rendered = match saved {
            Ok(rendered) => rendered,
            Err((code, why)) => {
                controller.audit("params/put", json!({}), &operator, false, Some(why.clone()));
                return err_json(code, why);
            }
        };

        let current = CurrentParams {
            sha256: sha256_hex(&text),
            text,
            params,
        };
        let exceptions = current.params.exceptions.len();
        controller.audit(
            "params/put",
            json!({"id": rendered.config_id, "exceptions": exceptions}),
            &operator,
            true,
            None,
        );
        info!(id = rendered.config_id, exceptions, "params saved");
        ok_json(params_body(&ecc_config_dir, &current, &rendered))
    })
    .await;
    match reply {
        Ok(reply) => reply,
        Err(e) => err_json(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("params/put task: {e}"),
        ),
    }
}

/// [`put_params`] の render + 書き込み(呼び手が `controller.run` のロックを握った状態で呼ぶ)。
/// 失敗は (HTTP コード, 理由): テンプレート/render の不備は 400、書けないのは 500。
pub(super) fn save_params(
    controller: &Controller,
    ecc_config_dir: &Path,
    params: &crate::params::Params,
    text: &str,
) -> Result<crate::params::Rendered, (StatusCode, String)> {
    let rendered = controller
        .render_with_template_dir(ecc_config_dir, params)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let cannot_write = |path: &Path, e: std::io::Error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("cannot write {}: {e}", path.display()),
        )
    };

    // §3-3: 生成物は content-addressed(`<id>` = stem + JSON の sha256 先頭 8 桁)なので、
    // 既存なら書かない(同じ JSON の再 PUT でタイムスタンプが動かない)。
    for file_name in [&rendered.configure_file_name, &rendered.prepare_file_name] {
        let path = ecc_config_dir.join(file_name);
        if !path.exists() {
            std::fs::write(&path, &rendered.xml).map_err(|e| cannot_write(&path, e))?;
        }
    }

    let params_dir = controller.params_dir();
    std::fs::create_dir_all(&params_dir).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("cannot create {}: {e}", params_dir.display()),
        )
    })?;
    // `current.json` は「今の設定」なので常に上書きする(§3-3 の content-addressed 規則は
    // configure/prepare/`<id>.json` だけの話 —— これは固定名で「最新」を指す)。
    // 113 §2-3: 一時ファイルに書いて rename で差し替える(同じファイルシステム内の rename は
    // アトミック)—— 読み手(run/start・GET /api/params・status)が書きかけを見ない。
    let current_path = params_dir.join("current.json");
    let tmp_path = params_dir.join("current.json.tmp");
    std::fs::write(&tmp_path, text).map_err(|e| cannot_write(&tmp_path, e))?;
    std::fs::rename(&tmp_path, &current_path).map_err(|e| cannot_write(&current_path, e))?;

    let id_path = params_dir.join(format!("{}.json", rendered.config_id));
    if !id_path.exists() {
        std::fs::write(&id_path, text).map_err(|e| cannot_write(&id_path, e))?;
    }
    Ok(rendered)
}

#[derive(Debug, Deserialize)]
pub(super) struct ParamsRenderQuery {
    template: Option<String>,
}

/// `GET /api/params/render?template=<name>`(dry run、閲覧系 = 認証なし)。
/// current.json を(省略時はその `template`、指定時はそのテンプレート名で)render して
/// **書かずに**返す(UI の diff 表示用、078 §10)。
pub(super) async fn get_params_render(
    State(controller): State<Arc<Controller>>,
    Query(query): Query<ParamsRenderQuery>,
) -> Reply {
    let reply = tokio::task::spawn_blocking(move || {
        let Some(ecc_config_dir) = controller.params.ecc_config_dir.clone() else {
            return err_json(
                StatusCode::SERVICE_UNAVAILABLE,
                ECC_CONFIG_DIR_NOT_CONFIGURED,
            );
        };
        let current = match controller.current_params() {
            Ok(Some(current)) => current,
            Ok(None) => return err_json(StatusCode::NOT_FOUND, "no params configured"),
            Err(e) => return err_json(StatusCode::INTERNAL_SERVER_ERROR, e),
        };
        let mut params = current.params.clone();
        if let Some(template) = &query.template {
            params.template = template.clone();
        }
        match controller.render_with_template_dir(&ecc_config_dir, &params) {
            Ok(rendered) => ok_json(json!({
                "id": rendered.config_id,
                "exceptions": params.exceptions.len(),
                "configure_xcfg": rendered.xml,
            })),
            // dry run の入力(current.json + クエリの template)が原因で失敗しているので 400。
            Err(e) => err_json(StatusCode::BAD_REQUEST, e),
        }
    })
    .await;
    match reply {
        Ok(reply) => reply,
        Err(e) => err_json(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("params/render task: {e}"),
        ),
    }
}

/// 生成物 `<stem>__<hash8>.xcfg`(078 §3-3)か。`__` の後ろが 16 進 8 桁のときだけ。
pub(super) fn is_generated_xcfg(name: &str) -> bool {
    let stem = name.strip_suffix(".xcfg").unwrap_or(name);
    match stem.rsplit_once("__") {
        Some((_, hash)) => hash.len() == 8 && hash.chars().all(|c| c.is_ascii_hexdigit()),
        None => false,
    }
}

/// `GET /api/params/templates`(閲覧系 = 認証なし、TODO/111 §5.0)。
/// `ecc_config_dir` 直下の `configure-*.xcfg`(生成物を除く)のファイル名を辞書順。
pub(super) async fn get_params_templates(State(controller): State<Arc<Controller>>) -> Reply {
    let reply = tokio::task::spawn_blocking(move || {
        let Some(ecc_config_dir) = controller.params.ecc_config_dir.clone() else {
            return err_json(
                StatusCode::SERVICE_UNAVAILABLE,
                ECC_CONFIG_DIR_NOT_CONFIGURED,
            );
        };
        let entries = match std::fs::read_dir(&ecc_config_dir) {
            Ok(entries) => entries,
            Err(e) => {
                return err_json(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("cannot read {}: {e}", ecc_config_dir.display()),
                )
            }
        };
        let mut templates: Vec<String> = Vec::new();
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    return err_json(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("cannot read {}: {e}", ecc_config_dir.display()),
                    )
                }
            };
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("configure-")
                && name.ends_with(".xcfg")
                && !is_generated_xcfg(&name)
                && entry.path().is_file()
            {
                templates.push(name);
            }
        }
        templates.sort();
        ok_json(json!({"templates": templates}))
    })
    .await;
    match reply {
        Ok(reply) => reply,
        Err(e) => err_json(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("params/templates task: {e}"),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct ParamsDefaultsQuery {
    template: Option<String>,
}

/// `GET /api/params/defaults?template=<name>`(閲覧系 = 認証なし、TODO/111 §5.0)。
/// テンプレート xcfg からノブの値を抽出して返す(UI の「Load defaults from template」)。
/// 400 = 名前が不正 / `ecc_config_dir` の外 / xcfg として読めない、404 = 無い、503 = 未設定。
pub(super) async fn get_params_defaults(
    State(controller): State<Arc<Controller>>,
    Query(query): Query<ParamsDefaultsQuery>,
) -> Reply {
    let reply = tokio::task::spawn_blocking(move || {
        let Some(ecc_config_dir) = controller.params.ecc_config_dir.clone() else {
            return err_json(
                StatusCode::SERVICE_UNAVAILABLE,
                ECC_CONFIG_DIR_NOT_CONFIGURED,
            );
        };
        let name = query.template.unwrap_or_default();
        if name.is_empty() {
            return err_json(StatusCode::BAD_REQUEST, "template is required");
        }
        if name.contains('/') || name.contains('\\') || name.contains("..") {
            return err_json(
                StatusCode::BAD_REQUEST,
                format!("template {name:?} must be a file name in ecc_config_dir"),
            );
        }
        if !name.ends_with(".xcfg") {
            return err_json(
                StatusCode::BAD_REQUEST,
                format!("template {name:?} is not an .xcfg file"),
            );
        }
        let path = ecc_config_dir.join(&name);
        if !path.exists() {
            return err_json(
                StatusCode::NOT_FOUND,
                format!("template {name} not found in {}", ecc_config_dir.display()),
            );
        }
        // シンボリックリンク等でディレクトリの外へ出るものは拒否する(canonicalize してから比べる)。
        let inside = match (path.canonicalize(), ecc_config_dir.canonicalize()) {
            (Ok(file), Ok(dir)) => file.starts_with(dir),
            (Err(e), _) | (_, Err(e)) => {
                return err_json(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("cannot resolve {}: {e}", path.display()),
                )
            }
        };
        if !inside {
            return err_json(
                StatusCode::BAD_REQUEST,
                format!("template {name} resolves outside ecc_config_dir"),
            );
        }
        let xml = match std::fs::read_to_string(&path) {
            Ok(xml) => xml,
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                return err_json(
                    StatusCode::BAD_REQUEST,
                    format!("template {name} is not UTF-8 text: {e}"),
                )
            }
            Err(e) => {
                return err_json(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("cannot read {}: {e}", path.display()),
                )
            }
        };
        match crate::params::extract(&name, &xml) {
            Ok(extracted) => ok_json(extracted.to_json()),
            Err(e) => err_json(StatusCode::BAD_REQUEST, e.to_string()),
        }
    })
    .await;
    match reply {
        Ok(reply) => reply,
        Err(e) => err_json(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("params/defaults task: {e}"),
        ),
    }
}
