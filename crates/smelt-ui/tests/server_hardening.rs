//! Network-hardening tests for the `smelt ui` server.
//!
//! Covers two independent guarantees:
//! - CORS is restricted to the server's own origin (no `permissive()` wildcard).
//! - Binding to a non-loopback host requires an explicit opt-in.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::Router;
use smelt_ui::server::{build_cors_layer, validate_bind_host};
use tower::ServiceExt;

fn app_with_cors(host: &str, port: u16) -> Router {
    Router::new()
        .route("/ping", get(|| async { "pong" }))
        .layer(build_cors_layer(host, port))
}

#[tokio::test]
async fn cors_rejects_cross_origin() {
    let app = app_with_cors("127.0.0.1", 3000);

    // Cross-origin request: no Access-Control-Allow-Origin should be echoed back.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/ping")
                .header("Origin", "http://evil.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response
            .headers()
            .get("access-control-allow-origin")
            .is_none(),
        "cross-origin request must not receive an Access-Control-Allow-Origin header"
    );

    // Same-origin request: the server's own origin is allowed.
    let response = app
        .oneshot(
            Request::builder()
                .uri("/ping")
                .header("Origin", "http://127.0.0.1:3000")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let allowed = response
        .headers()
        .get("access-control-allow-origin")
        .expect("same-origin request must receive an Access-Control-Allow-Origin header");
    assert_eq!(allowed, "http://127.0.0.1:3000");
}

#[test]
fn non_loopback_bind_requires_opt_in() {
    let err = validate_bind_host("0.0.0.0", false)
        .expect_err("non-loopback bind without allow_remote must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("--allow-remote"),
        "error message should name the --allow-remote flag, got: {msg}"
    );

    validate_bind_host("0.0.0.0", true)
        .expect("non-loopback bind with allow_remote=true should proceed");

    // Loopback hosts are always fine, opt-in or not.
    validate_bind_host("127.0.0.1", false).expect("loopback bind should never require opt-in");
    validate_bind_host("localhost", false).expect("loopback bind should never require opt-in");
}
