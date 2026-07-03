//! Optional localhost control API (feat/gpu-slicer-wgpu).
//!
//! When the app is launched with `DF_CONTROL_PORT` set, a tiny blocking HTTP
//! server binds `127.0.0.1:$DF_CONTROL_PORT` so external scripts can drive *the
//! running desktop instance* — load meshes, orient/replicate/query the scene,
//! and slice whatever is currently loaded.
//!
//! It is a thin generic command bus: every request becomes a `control:command`
//! Tauri event `{ request_id, op, params }`. The frontend runs the matching
//! scene action and reports back via the `control_command_result` command; the
//! HTTP request then responds with the op's JSON result.
//!
//!   POST /command      body { "op": "<name>", "params": { ... } }
//!   POST /slice/scene  sugar for op "slice" (optionally streams the artifact)
//!   GET  /health
//!
//! Security: loopback only. If `DF_CONTROL_TOKEN` is set, every request must
//! carry a matching `X-Control-Token` header. The server never starts unless
//! `DF_CONTROL_PORT` is present, so default builds are unaffected.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::Emitter;

use crate::DragonFruitAppHandle;

/// How long an HTTP command waits for the frontend to report a result.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(600);

/// Outcome the frontend reports back via `control_command_result`.
#[derive(Clone, Debug, Deserialize)]
pub struct CommandOutcome {
    pub ok: bool,
    /// Arbitrary op-specific JSON returned on success.
    #[serde(default)]
    pub result: Value,
    #[serde(default)]
    pub error: Option<String>,
}

/// Body accepted by `POST /command`.
#[derive(Deserialize)]
struct CommandBody {
    op: String,
    #[serde(default)]
    params: Value,
}

/// Body accepted by `POST /slice/scene` (all fields optional).
#[derive(Default, Deserialize)]
struct SliceSceneBody {
    #[serde(default)]
    output_path: Option<String>,
    /// When true, respond with the raw artifact bytes instead of JSON metadata.
    #[serde(default)]
    download: bool,
}

fn pending() -> &'static Mutex<HashMap<u64, Sender<CommandOutcome>>> {
    static P: OnceLock<Mutex<HashMap<u64, Sender<CommandOutcome>>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_request_id() -> u64 {
    static C: AtomicU64 = AtomicU64::new(1);
    C.fetch_add(1, Ordering::Relaxed)
}

/// Frontend → Rust callback delivering the result of a control command.
#[tauri::command]
pub fn control_command_result(request_id: u64, outcome: CommandOutcome) -> Result<(), String> {
    let sender = {
        let mut map = pending()
            .lock()
            .map_err(|e| format!("control pending lock poisoned: {e}"))?;
        map.remove(&request_id)
    };
    match sender {
        Some(tx) => {
            let _ = tx.send(outcome);
            Ok(())
        }
        None => Err(format!("no pending control request {request_id}")),
    }
}

/// Start the control server if `DF_CONTROL_PORT` is set. No-op otherwise.
pub fn start(app: DragonFruitAppHandle) {
    let Some(port) = std::env::var("DF_CONTROL_PORT")
        .ok()
        .and_then(|v| v.trim().parse::<u16>().ok())
    else {
        return;
    };
    let token = std::env::var("DF_CONTROL_TOKEN")
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());

    let _ = std::thread::Builder::new()
        .name("df-control-server".into())
        .spawn(move || run(app, port, token));
}

/// Dispatch one op to the frontend and block until it reports a result (or we
/// time out). Returns the outcome, or an `Err(message)` on emit/timeout.
fn dispatch(app: &DragonFruitAppHandle, op: &str, params: Value) -> Result<CommandOutcome, String> {
    let request_id = next_request_id();
    let (tx, rx) = channel::<CommandOutcome>();
    pending()
        .lock()
        .expect("control pending lock")
        .insert(request_id, tx);

    if let Err(e) = app.emit(
        "control:command",
        json!({ "request_id": request_id, "op": op, "params": params }),
    ) {
        pending().lock().expect("control pending lock").remove(&request_id);
        return Err(format!("failed to emit control event: {e}"));
    }

    match rx.recv_timeout(COMMAND_TIMEOUT) {
        Ok(outcome) => Ok(outcome),
        Err(_) => {
            pending().lock().expect("control pending lock").remove(&request_id);
            Err("timed out waiting for the frontend (is the window open?)".into())
        }
    }
}

fn run(app: DragonFruitAppHandle, port: u16, token: Option<String>) {
    let addr = format!("127.0.0.1:{port}");
    let server = match tiny_http::Server::http(&addr) {
        Ok(s) => s,
        Err(e) => {
            log::error!("[control] failed to bind {addr}: {e}");
            return;
        }
    };
    log::info!(
        "[control] listening on http://{addr} (token {})",
        if token.is_some() { "required" } else { "disabled" }
    );

    for mut req in server.incoming_requests() {
        // Token gate.
        if let Some(expected) = token.as_deref() {
            let provided = req
                .headers()
                .iter()
                .find(|h| h.field.equiv("X-Control-Token"))
                .map(|h| h.value.as_str().to_string());
            if provided.as_deref() != Some(expected) {
                respond_json(req, 401, &json!({ "error": "invalid or missing X-Control-Token" }));
                continue;
            }
        }

        let method = req.method().as_str().to_string();
        let path = req.url().split('?').next().unwrap_or("").to_string();

        match (method.as_str(), path.as_str()) {
            ("GET", "/health") => respond_json(
                req,
                200,
                &json!({
                    "status": "ok",
                    "engine_version": dragonfruit_slicing_engine::ENGINE_VERSION,
                    "gpu_feature": cfg!(feature = "gpu"),
                    "slice_backend_env": std::env::var("DF_SLICE_BACKEND").unwrap_or_default(),
                }),
            ),
            ("POST", "/command") => handle_command(&app, req),
            ("POST", "/slice/scene") => handle_slice_scene(&app, req),
            _ => respond_json(req, 404, &json!({ "error": "not found" })),
        }
    }
}

fn read_body(req: &mut tiny_http::Request) -> String {
    let mut body = String::new();
    use std::io::Read;
    let _ = req.as_reader().read_to_string(&mut body);
    body
}

fn handle_command(app: &DragonFruitAppHandle, mut req: tiny_http::Request) {
    let body = read_body(&mut req);
    let parsed: CommandBody = match serde_json::from_str(&body) {
        Ok(b) => b,
        Err(e) => {
            respond_json(req, 400, &json!({ "error": format!("bad JSON body: {e}") }));
            return;
        }
    };

    match dispatch(app, &parsed.op, parsed.params) {
        Ok(outcome) if outcome.ok => {
            respond_json(req, 200, &json!({ "ok": true, "result": outcome.result }))
        }
        Ok(outcome) => respond_json(
            req,
            422,
            &json!({ "ok": false, "error": outcome.error.unwrap_or_else(|| "command failed".into()) }),
        ),
        Err(e) => respond_json(req, 504, &json!({ "error": e })),
    }
}

fn handle_slice_scene(app: &DragonFruitAppHandle, mut req: tiny_http::Request) {
    let body = read_body(&mut req);
    let parsed: SliceSceneBody = if body.trim().is_empty() {
        SliceSceneBody::default()
    } else {
        match serde_json::from_str(&body) {
            Ok(b) => b,
            Err(e) => {
                respond_json(req, 400, &json!({ "error": format!("bad JSON body: {e}") }));
                return;
            }
        }
    };

    let output_path = parsed
        .output_path
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| crate::temp_artifact_path("print").to_string_lossy().to_string());

    let outcome = match dispatch(app, "slice", json!({ "output_path": output_path })) {
        Ok(o) => o,
        Err(e) => {
            respond_json(req, 504, &json!({ "error": e }));
            return;
        }
    };

    if !outcome.ok {
        respond_json(
            req,
            422,
            &json!({ "error": outcome.error.unwrap_or_else(|| "slice failed".into()) }),
        );
        return;
    }

    let path = outcome
        .result
        .get("output_path")
        .and_then(|v| v.as_str())
        .unwrap_or(&output_path)
        .to_string();

    if parsed.download {
        match std::fs::read(&path) {
            Ok(bytes) => {
                let header = tiny_http::Header::from_bytes(
                    &b"Content-Type"[..],
                    &b"application/octet-stream"[..],
                )
                .expect("valid header");
                let _ = req.respond(tiny_http::Response::from_data(bytes).with_header(header));
            }
            Err(e) => {
                respond_json(req, 500, &json!({ "error": format!("failed reading artifact: {e}") }))
            }
        }
        return;
    }

    respond_json(req, 200, &json!({ "ok": true, "result": outcome.result }));
}

fn respond_json(req: tiny_http::Request, status: u16, body: &Value) {
    let text = body.to_string();
    let header = tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
        .expect("valid header");
    let response = tiny_http::Response::from_string(text)
        .with_status_code(status)
        .with_header(header);
    let _ = req.respond(response);
}
