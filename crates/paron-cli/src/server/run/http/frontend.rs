use axum::http::HeaderValue;
#[cfg(debug_assertions)]
use js_frontend_embed::DevServer;
use tower_http::set_header::SetResponseHeaderLayer;

#[cfg(debug_assertions)]
use crate::Result;

#[js_frontend_embed::embed]
struct Bundle;

pub struct Frontend {
    #[cfg(debug_assertions)]
    dev_server: Option<DevServer>,
}

impl Frontend {
    #[cfg_attr(debug_assertions, allow(dead_code))]
    pub fn embedded() -> Self {
        Self {
            #[cfg(debug_assertions)]
            dev_server: None,
        }
    }

    #[cfg(debug_assertions)]
    pub fn start_dev_server() -> Result<Self> {
        let dev_server = js_frontend_embed::dev(Bundle).spawn()?;

        Ok(Self {
            dev_server: Some(dev_server),
        })
    }

    #[cfg_attr(not(debug_assertions), allow(clippy::unused_self))]
    pub fn into_router(self) -> axum::Router {
        #[cfg(debug_assertions)]
        let service = match self.dev_server {
            Some(dev_server) => js_frontend_embed::proxying(dev_server),
            None => Bundle::service(),
        };

        #[cfg(not(debug_assertions))]
        let service = Bundle::service();

        axum::Router::new()
            .fallback_service(service)
            .layer(SetResponseHeaderLayer::overriding(
                axum::http::header::REFERRER_POLICY,
                axum::http::HeaderValue::from_static("strict-origin-when-cross-origin"),
            ))
            .layer(SetResponseHeaderLayer::overriding(
                axum::http::header::CONTENT_SECURITY_POLICY,
                content_security_policy(),
            ))
    }
}

/// Format: ("directive", "value", "additional dev only value")
const CSP_DIRECTIVES: [(&str, &str, &str); 10] = [
    ("default-src", "'self'", ""),
    ("img-src", "'self' data:", ""),
    ("font-src", "'self'", ""),
    ("object-src", "'none'", ""),
    ("base-uri", "'self'", ""),
    ("form-action", "'self'", ""),
    ("frame-ancestors", "'none'", ""),
    ("script-src", "'self'", "'unsafe-inline' 'unsafe-eval'"),
    ("style-src", "'self'", "'unsafe-inline'"),
    ("connect-src", "'self'", "ws: wss:"),
];

#[expect(
    clippy::expect_used,
    reason = "the directives are constants made of valid header text"
)]
fn content_security_policy() -> HeaderValue {
    let policy = CSP_DIRECTIVES
        .iter()
        .map(|(directive, sources, dev_only_sources)| {
            if cfg!(debug_assertions) {
                return format!("{directive} {sources} {dev_only_sources}");
            }

            format!("{directive} {sources}")
        })
        .collect::<Vec<_>>()
        .join("; ");

    HeaderValue::from_str(&policy).expect("could not build content security policy header")
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::extract::Request;
    use axum::http::header;
    use tower::ServiceExt as _;

    use super::*;

    #[test]
    fn content_security_policy_is_relaxed_only_in_a_development_build() {
        let policy = content_security_policy();
        let policy = policy.to_str().unwrap();

        for (directive, sources, _) in CSP_DIRECTIVES {
            assert!(
                policy.contains(&format!("{directive} {sources}")),
                "{directive} is missing from {policy}"
            );
        }

        let is_relaxed = CSP_DIRECTIVES.iter().any(|(_, _, in_development)| {
            !in_development.is_empty() && policy.contains(in_development)
        });

        assert_eq!(
            is_relaxed,
            cfg!(debug_assertions),
            "a deployed build allows only what it is deployed with: {policy}"
        );
    }

    #[test]
    fn bundle_entry_document_carries_nothing_inline() {
        let entry = js_frontend_embed::file::<Bundle>(js_frontend_embed::ENTRY)
            .expect("the bundle has an entry");
        let document = String::from_utf8_lossy(&entry);

        assert!(
            !document.contains("<style"),
            "the entry document carries an inline style, which `content_security_policy` forbids \
             in a deployment"
        );

        for (tag_start, _) in document.match_indices("<script") {
            let script_tag =
                &document[tag_start..][..document[tag_start..].find('>').expect("a tag is closed")];

            assert!(
                script_tag.contains("src="),
                "the entry document carries an inline script, which `content_security_policy` \
                 forbids in a deployment: {script_tag}>"
            );
        }
    }

    #[test]
    fn bundle_serves_no_dotfiles() {
        for name in js_frontend_embed::served::<Bundle>() {
            assert!(
                !name.starts_with('.'),
                "{name} is embedded and should not be"
            );
        }
    }

    #[tokio::test]
    async fn into_router_entry_document_with_brotli_accepted_is_sent_compressed() {
        let request = Request::builder()
            .uri("/")
            .header(header::ACCEPT_ENCODING, "br, gzip")
            .body(Body::empty())
            .unwrap();

        let response = Frontend::embedded()
            .into_router()
            .oneshot(request)
            .await
            .unwrap();

        assert_eq!(response.headers()[header::CONTENT_ENCODING], "br");
        let entry_file = js_frontend_embed::file::<Bundle>(js_frontend_embed::ENTRY).unwrap();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(
            body.len() < entry_file.len(),
            "sent {} of {}",
            body.len(),
            entry_file.len()
        );
    }

    #[tokio::test]
    async fn into_router_entry_document_with_identity_accepted_is_the_bundled_file() {
        let request = Request::builder()
            .uri("/")
            .header(header::ACCEPT_ENCODING, "identity")
            .body(Body::empty())
            .unwrap();

        let response = Frontend::embedded()
            .into_router()
            .oneshot(request)
            .await
            .unwrap();

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            body,
            js_frontend_embed::file::<Bundle>(js_frontend_embed::ENTRY).unwrap()
        );
        assert!(body.starts_with(b"<!doctype html"), "not the document");
    }

    #[tokio::test]
    async fn into_router_fingerprinted_asset_is_cached_forever() {
        let name = Bundle::fingerprinted()
            .next()
            .expect("no manifest was read: `build.manifest` in vite.config.ts");
        let request = Request::builder()
            .uri(format!("/{name}"))
            .body(Body::empty())
            .unwrap();

        let response = Frontend::embedded()
            .into_router()
            .oneshot(request)
            .await
            .unwrap();

        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "public, max-age=31536000, immutable"
        );
    }
}
