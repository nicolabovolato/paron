use std::collections::HashSet;
use std::sync::Arc;

use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse as _, Response};

use super::error::ApiError;
use super::state::Deps;

pub struct PublicOrigins {
    canonical: url::Origin,
    accepted: HashSet<url::Origin>,
}

impl PublicOrigins {
    pub fn new(canonical: url::Origin, others: impl IntoIterator<Item = url::Origin>) -> Self {
        Self {
            accepted: std::iter::once(canonical.clone()).chain(others).collect(),
            canonical,
        }
    }

    pub fn canonical(&self) -> &url::Origin {
        &self.canonical
    }

    fn matching(&self, parts: &Parts) -> Option<&url::Origin> {
        let authority = parts
            .uri
            .authority()
            .map(|authority| authority.as_str().to_owned())
            .or_else(|| {
                parts
                    .headers
                    .get(header::HOST)
                    .and_then(|host| host.to_str().ok())
                    .map(str::to_owned)
            })?
            .parse::<axum::http::uri::Authority>()
            .ok()?;

        self.accepted.iter().find(|accepted| {
            let url::Origin::Tuple(scheme, _, _) = accepted else {
                return false;
            };

            url::Url::parse(&format!("{scheme}://{authority}"))
                .is_ok_and(|url| url.origin() == **accepted)
        })
    }
}

pub async fn match_public_origin(
    State(origins): State<Arc<PublicOrigins>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let (mut parts, body) = request.into_parts();

    if let Some(origin) = origins.matching(&parts).cloned() {
        parts.extensions.insert(RequestOrigin(origin));

        return next
            .run(axum::extract::Request::from_parts(parts, body))
            .await;
    }

    ApiError::detail(StatusCode::MISDIRECTED_REQUEST, "unknown host").into_response()
}

#[derive(Clone)]
pub struct RequestOrigin(pub url::Origin);

impl FromRequestParts<Deps> for RequestOrigin {
    type Rejection = ApiError;

    fn from_request_parts(
        parts: &mut Parts,
        _: &Deps,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        std::future::ready(
            parts
                .extensions
                .get::<Self>()
                .cloned()
                .ok_or_else(|| ApiError::internal("the host of the request was not checked")),
        )
    }
}
