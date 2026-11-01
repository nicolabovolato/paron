//! Uses only the public API, so a missing `pub use` in `lib.rs` is a compile error here.

#![cfg(feature = "serve")]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use js_frontend_embed::{Embed, Policy, embedded};
use tower::ServiceExt as _;

// A plain path, not `$JS_FRONTEND_EMBED_DIST`: no build script runs for this test.
#[derive(Embed)]
#[folder = "tests/bundle"]
#[crate_path = "js_frontend_embed::rust_embed"]
#[exclude = ".vite/*"]
struct BundleOnDisk;

#[tokio::test]
async fn embedded_answers_from_a_bundle_through_the_public_api() {
    let router = embedded::<BundleOnDisk>(Policy::holding([
        "assets/app-A1b2C3d4.js",
        "assets/logo-Z9y8X7w6.svg",
    ]));

    for (uri, accept, status, cache_control) in [
        ("/", "text/html", StatusCode::OK, "no-cache"),
        (
            "/assets/app-A1b2C3d4.js",
            "*/*",
            StatusCode::OK,
            "public, max-age=31536000, immutable",
        ),
        ("/machines/42", "text/html", StatusCode::OK, "no-cache"),
        ("/machines/42", "*/*", StatusCode::NOT_FOUND, "no-store"),
        (
            "/.vite/manifest.json",
            "*/*",
            StatusCode::NOT_FOUND,
            "no-store",
        ),
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::get(uri)
                    .header(header::ACCEPT, accept)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            (
                response.status(),
                response.headers()[header::CACHE_CONTROL].to_str().unwrap()
            ),
            (status, cache_control),
            "{uri} with Accept {accept}"
        );
    }
}
