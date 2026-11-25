//! Optional local web UI for the reconciliation toolkit.
//!
//! This module is a thin shell, exactly like the CLI: it parses incoming
//! ledger text with [`parse_records`], runs the **same** library functions
//! (`summarize`, `validate_all`, `reconcile`) and serializes their results.
//! No validation or accounting logic lives here — a web user must get the
//! identical answer a CLI user gets from the same bytes.
//!
//! Security / scope notes:
//! - The listener binds `127.0.0.1` only. There is no auth, no persistence and
//!   no CORS: this is a local tool, not a service.
//! - The page is embedded at compile time (`include_str!`) and has no external
//!   CSS/JS/fonts, so it works offline. The browser renders every dynamic
//!   value with `textContent`; the HTML endpoints reuse `report`'s escaping.

use axum::body::Bytes;
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::diff::{reconcile as reconcile_ledgers, LedgerDiff};
use crate::ingest::parse_records;
use crate::reconcile::summarize;
use crate::report::{render_diff, render_full_report, Format};
use crate::sample_records;
use crate::validate::validate_all;

/// The single self-contained page, embedded so there is nothing to serve from
/// disk and nothing to fetch from the network at runtime.
const INDEX_HTML: &str = include_str!("web/index.html");

/// The axum router. Public so integration tests can drive it in-process with
/// `tower::ServiceExt::oneshot` without binding a socket.
pub fn router() -> Router {
    Router::new()
        .route("/", get(index))
        .route("/api/sample", get(sample))
        .route("/api/report", post(report_json))
        .route("/api/report/html", post(report_html))
        .route("/api/reconcile", post(reconcile_json))
        .route("/api/reconcile/html", post(reconcile_html))
}

/// Start the local-only server.
///
/// `port = 0` lets the OS pick a free port; the chosen address is printed to
/// stderr so a wrapper can discover it. No `--host` flag exists on purpose:
/// exposing this unauthenticated UI on a LAN would be a footgun.
///
/// # Errors
/// Returns an error if the loopback port cannot be bound or the server fails.
pub async fn serve(port: u16) -> anyhow::Result<()> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|err| anyhow::anyhow!("failed to bind {addr}: {err}"))?;
    eprintln!(
        "kite-x402-reconciler web UI: http://{}  (Ctrl-C to stop)",
        listener.local_addr()?
    );
    axum::serve(listener, router())
        .await
        .map_err(|err| anyhow::anyhow!("web server failed: {err}"))
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn sample() -> Json<Vec<crate::model::SettlementRecord>> {
    Json(sample_records())
}

/// JSON envelope for the single-ledger endpoint. Keeps summary and validation
/// in one object — two concatenated JSON documents would be unparseable.
#[derive(Serialize)]
struct ReportPayload {
    summary: crate::reconcile::Summary,
    validation: crate::validate::ValidationReport,
}

#[derive(Deserialize)]
struct ReconcileRequest {
    local: String,
    facilitator: String,
}

/// A client error that must be reported as HTTP 400 rather than a 500: bad
/// ledger text is expected input, not a server fault.
#[derive(Debug)]
struct BadRequest(String);

impl IntoResponse for BadRequest {
    fn into_response(self) -> Response {
        let body = serde_json::json!({ "error": self.0 }).to_string();
        (
            StatusCode::BAD_REQUEST,
            [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
            body,
        )
            .into_response()
    }
}

fn text_of(body: &Bytes) -> Result<String, BadRequest> {
    String::from_utf8(body.to_vec())
        .map_err(|_| BadRequest("request body is not valid UTF-8".to_owned()))
}

fn build_report(text: &str) -> Result<ReportPayload, BadRequest> {
    let records = parse_records(text)
        .map_err(|err| BadRequest(format!("failed to parse ledger: {err:#}")))?;
    Ok(ReportPayload {
        summary: summarize(&records),
        validation: validate_all(&records),
    })
}

fn build_diff(text: &str) -> Result<LedgerDiff, BadRequest> {
    let request: ReconcileRequest = serde_json::from_str(text)
        .map_err(|err| BadRequest(format!("invalid reconcile request: {err}")))?;
    let local = parse_records(&request.local)
        .map_err(|err| BadRequest(format!("failed to parse local ledger: {err:#}")))?;
    let facilitator = parse_records(&request.facilitator)
        .map_err(|err| BadRequest(format!("failed to parse facilitator ledger: {err:#}")))?;
    Ok(reconcile_ledgers(&local, &facilitator))
}

async fn report_json(body: Bytes) -> Result<Json<Value>, BadRequest> {
    let payload = build_report(&text_of(&body)?)?;
    let value = serde_json::to_value(payload)
        .map_err(|err| BadRequest(format!("failed to serialize report: {err}")))?;
    Ok(Json(value))
}

async fn report_html(body: Bytes) -> Result<Html<String>, BadRequest> {
    let payload = build_report(&text_of(&body)?)?;
    Ok(Html(render_full_report(
        &payload.summary,
        &payload.validation,
        Format::Html,
    )))
}

async fn reconcile_json(body: Bytes) -> Result<Json<LedgerDiff>, BadRequest> {
    Ok(Json(build_diff(&text_of(&body)?)?))
}

async fn reconcile_html(body: Bytes) -> Result<Html<String>, BadRequest> {
    Ok(Html(render_diff(
        &build_diff(&text_of(&body)?)?,
        Format::Html,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn report_matches_library_functions() {
        let ledger = json!(sample_records()).to_string();
        let payload = build_report(&ledger).expect("sample ledger parses");
        assert_eq!(payload.summary.total.count, 4);
        assert_eq!(payload.validation.errors, 2);
    }

    #[test]
    fn reconcile_matches_library_functions() {
        let ledger = json!(sample_records()).to_string();
        let request = json!({ "local": ledger, "facilitator": ledger }).to_string();
        let diff = build_diff(&request).expect("matching ledgers parse");
        assert_eq!(diff.errors(), 0);
        assert_eq!(diff.matched, 3);
    }

    #[test]
    fn bad_ledger_is_a_client_error() {
        assert!(build_report("not json").is_err());
        assert!(build_diff("not json").is_err());
    }

    #[test]
    fn page_is_self_contained() {
        assert!(INDEX_HTML.contains("<script>"));
        assert!(!INDEX_HTML.contains("src=\"http"));
        assert!(!INDEX_HTML.contains("href=\"http"));
        assert!(!INDEX_HTML.contains("innerHTML"));
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime builds")
    }

    #[test]
    fn json_endpoint_matches_library_result() {
        let ledger = Bytes::from(json!(sample_records()).to_string());
        let value = runtime()
            .block_on(report_json(ledger))
            .expect("valid ledger");
        assert_eq!(value.0["summary"]["total"]["count"], 4);
        assert_eq!(value.0["validation"]["errors"], 2);
    }

    #[test]
    fn html_endpoint_escapes_a_hostile_field() {
        let mut records = sample_records();
        records[0].network = "eip155:1<script>alert(1)</script>".into();
        let ledger = Bytes::from(serde_json::to_string(&records).expect("serializes"));
        let html = runtime()
            .block_on(report_html(ledger))
            .expect("valid ledger")
            .0;
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(
            !html.contains("<script>alert(1)</script>"),
            "raw script leaked"
        );
        assert!(html.contains("&lt;script&gt;"), "escaped form missing");
    }

    #[test]
    fn reconcile_endpoint_handles_two_ledgers() {
        let ledger = json!(sample_records()).to_string();
        let body = Bytes::from(json!({ "local": ledger, "facilitator": ledger }).to_string());
        let diff = runtime()
            .block_on(reconcile_json(body))
            .expect("matching ledgers");
        assert_eq!(diff.0.errors(), 0);
        assert_eq!(diff.0.matched, 3);
    }

    #[test]
    fn malformed_input_is_rejected_as_bad_request() {
        let body = Bytes::from_static(b"not json");
        assert!(runtime().block_on(report_json(body)).is_err());
    }
}
