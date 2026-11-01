#[cfg(debug_assertions)]
mod proxy;

#[cfg(debug_assertions)]
pub use proxy::proxying;

use std::borrow::Cow;
use std::collections::BTreeSet;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::Embed;

/// How long a browser may keep each name in the bundle.
///
/// The directives are fixed deliberately: a content-hashed name is held forever, and everything
/// else is revalidated against an `ETag`, which costs headers rather than the file. What people
/// reach for instead — `s-maxage`, `stale-while-revalidate` — buys one round trip by risking an
/// entry document naming chunks the deploy already deleted, which is a blank page.
#[derive(Debug, Clone)]
pub struct Policy {
    /// The document served for any path that is not a file. Client-side routing needs this.
    entry: String,

    /// Exactly the names the bundler reported emitting with a content hash. Empty holds nothing,
    /// which is the only safe assumption about a name nobody vouched for.
    fingerprinted: BTreeSet<String>,

    // Header values rather than strings: fixed, sent on every response, so a clone of something
    // already valid beats parsing the same constant per request.
    immutable: HeaderValue,

    mutable: HeaderValue,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            entry: crate::ENTRY.to_owned(),
            fingerprinted: BTreeSet::new(),
            immutable: HeaderValue::from_static("public, max-age=31536000, immutable"),
            mutable: HeaderValue::from_static("no-cache"),
        }
    }
}

impl Policy {
    /// Holds the names the bundler said it fingerprinted, which is what
    /// [`embed`](crate::embed) reports.
    ///
    /// Every name handed in is promised to change whenever its contents do, and a browser is told
    /// so for a year. It takes names rather than that macro so a build script can work them out
    /// some other way — what must hold is the promise, not where it came from.
    ///
    /// ```
    /// # use js_frontend_embed::Policy;
    /// let policy = Policy::holding(["assets/index-DkK_wTPT.js"]);
    ///
    /// assert!(policy.fingerprinted().contains("assets/index-DkK_wTPT.js"));
    /// ```
    #[must_use]
    pub fn holding<I>(fingerprinted: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<String>,
    {
        Self {
            // A bundle is keyed without a leading slash and a URL always has one; taken as
            // given, the wrong spelling matches nothing and caching quietly stops happening.
            fingerprinted: fingerprinted
                .into_iter()
                .map(|name| name.into().trim_start_matches('/').to_owned())
                .collect(),
            ..Self::default()
        }
    }

    /// The document answering any path that is not a file, [`crate::ENTRY`] by default.
    ///
    /// [`Build::entry`](crate::Build::entry) names the same document on the build side. Setting
    /// one without the other leaves this serving a document that is not there.
    #[must_use]
    pub fn entry(mut self, entry: &str) -> Self {
        // Without a leading slash, like every other name in a bundle.
        entry.trim_start_matches('/').clone_into(&mut self.entry);
        self
    }

    /// The names being held forever, in case a consumer wants to check the bundler reported any.
    #[must_use]
    pub fn fingerprinted(&self) -> &BTreeSet<String> {
        &self.fingerprinted
    }

    /// The entry document is the exception whatever the manifest says: its name has to stay put
    /// for the application to be reachable, so holding it forever pins the whole application
    /// rather than one asset, with no name left to publish a fix under.
    fn cache_for(&self, path: &str) -> &HeaderValue {
        if path != self.entry && self.fingerprinted.contains(path) {
            &self.immutable
        } else {
            &self.mutable
        }
    }
}

/// Serves the bundle `A` embedded in this binary, caching each name for as long as `policy` says.
///
/// The bundle was compressed on its way into the binary, so this picks between sending what is
/// stored and decoding it, by what the request says it takes. Nothing is compressed here: a
/// compression layer on the router still covers the API, and skips these because they arrive with
/// an encoding already named.
pub fn embedded<A: Embed + 'static>(policy: Policy) -> axum::routing::MethodRouter {
    // Shared rather than cloned: axum clones the handler per request, and the policy carries the
    // bundler's whole list of names — a thousand string allocations on a code-split frontend.
    let policy = std::sync::Arc::new(policy);

    // Fixed when the binary was built, so it is asked once here rather than on every request. Not
    // cached inside `compressed` itself: a `static` in a generic function is one static for every
    // type it is called with, so a second bundle in the same process would be answered for by the
    // first.
    let compressed = compressed::<A>();

    // Reading is the only thing you can do to a file already compiled in, so everything else is
    // a 405 with an `Allow` axum fills in. `proxying` stays open to every method: a dev server is
    // a whole server.
    axum::routing::get(move |request: Request<Body>| {
        let policy = std::sync::Arc::clone(&policy);

        async move { answer::<A>(&policy, compressed, &request) }
    })
}

fn answer<A: Embed>(policy: &Policy, compressed: bool, request: &Request<Body>) -> Response {
    let uri = request.uri();
    let found = requested_name(uri)
        .and_then(|name| asset::<A>(policy, compressed, &name, request.headers()));

    if let Some(response) = found {
        return response;
    }

    // Both answers below are to the same URL and differ only by what the request accepts, so a
    // cache that kept one without knowing which `Accept` produced it would hand it to the other.
    let mut response = if wants_the_page(uri, request.headers()) {
        asset::<A>(policy, compressed, &policy.entry, request.headers())
            .unwrap_or_else(|| missing::<A>(&policy.entry))
    } else {
        not_found()
    };

    // Both names, and `insert` rather than `append`: this stands in for whatever `asset` set,
    // and the entry document is reached through here on every page load — so dropping the
    // encoding half would leave a shared cache holding brotli under a key that does not mention
    // it, and handing it to the next client that cannot read it.
    response.headers_mut().insert(
        header::VARY,
        HeaderValue::from_static("Accept, Accept-Encoding"),
    );

    response
}

/// A 404 is one of the few statuses a cache may keep uninvited, and the one given to a `fetch`
/// for a route with nothing behind it is, at the same URL, the page a browser should get.
fn not_found() -> Response {
    (StatusCode::NOT_FOUND, [(header::CACHE_CONTROL, "no-store")]).into_response()
}

fn asset<A: Embed>(
    policy: &Policy,
    compressed: bool,
    path: &str,
    asked_with: &HeaderMap,
) -> Option<Response> {
    let (stored, file) = stored::<A>(compressed, path)?;

    // Two representations of one URL come out of the same stored file, so one validator would
    // answer both — and a browser holding the compressed copy would be told its brotli was still
    // current while asking for something it can read.
    let brotli = stored == Stored::Brotli && takes_brotli(asked_with);
    // Marked only where there is something to tell apart: a file stored as it is has one
    // representation, and marking it would change every validator already in a browser.
    let tag = etag(
        &file.metadata.sha256_hash(),
        stored == Stored::Brotli && !brotli,
    );
    let cache = policy.cache_for(path).clone();

    // Folding the failure into the `None` above would mean "no such file" and send the request to
    // the entry document.
    #[expect(clippy::expect_used)]
    let validator = HeaderValue::from_str(&tag).expect("a quoted digest is a valid header value");

    // The common case for everything served `no-cache`: the entry document on every page load,
    // and every asset the bundler passed through without a hash in its name.
    if already_held(asked_with, &tag) {
        let mut response = Response::new(Body::empty());
        *response.status_mut() = StatusCode::NOT_MODIFIED;
        response.headers_mut().insert(header::ETAG, validator);
        response.headers_mut().insert(header::CACHE_CONTROL, cache);

        // Required here as much as on the answer it stands in for (RFC 9110 §15.4.5). A shared
        // cache revalidating an entry takes what it learns from this response, and one that
        // forgot the name it varies on would go on to hand brotli to a client that said it
        // takes none.
        if stored == Stored::Brotli {
            response
                .headers_mut()
                .insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
        }

        // axum puts a `Content-Length` back from the body's size hint after this returns, so
        // removing it here would change nothing that leaves.
        return Some(response);
    }

    // Kept as the `Cow` it arrives as where it can be: an embedded file borrows from the binary's
    // own data, and `into_owned` would copy it on every request for the life of the process.
    let body = match (stored, brotli) {
        // The common case, and the whole point: the bytes go out as they are stored.
        (Stored::Verbatim | Stored::Brotli, true) | (Stored::Verbatim, false) => {
            Body::from(file.data)
        }
        // A client that takes no encoding. Rare, and decoding is the cheap direction.
        (Stored::Brotli, false) => match unpacked(&file.data) {
            Some(unpacked) => Body::from(unpacked),
            // The file is here and cannot be read, which is this server's fault and nobody
            // else's. Answering `None` would send the request on to the entry document, and a
            // page of HTML would arrive where a script was asked for — with a 200 on it.
            None => return Some(unreadable(path)),
        },
    };

    let mut response = Response::new(body);
    // From the name asked for, not the name stored: `app.js.br` guesses as brotli, and a browser
    // told an application is `application/x-brotli` downloads it. Refusing over an unrepresentable
    // type would come out as a 404 and then as the entry document — HTML in place of what was
    // asked for, which is what `nosniff` prevents.
    let kind = HeaderValue::from_str(mimetype(path).as_ref())
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));

    if brotli {
        response
            .headers_mut()
            .insert(header::CONTENT_ENCODING, HeaderValue::from_static("br"));
    }

    // Set whichever way it was answered: the two representations share a URL, so a cache that
    // kept one without knowing what encoding produced it would hand it to a client that cannot
    // read it.
    if stored == Stored::Brotli {
        response
            .headers_mut()
            .insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
    }

    response.headers_mut().insert(header::CONTENT_TYPE, kind);
    response.headers_mut().insert(header::CACHE_CONTROL, cache);
    response.headers_mut().insert(header::ETAG, validator);
    // A browser left to guess can decide a file is HTML and run what is inside it.
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );

    Some(response)
}

/// One file out of the bundle, by the name it is served under, however it is stored.
///
/// Not `Embed::get`, which takes the storage name and hands back whatever is under it — brotli,
/// for anything that compressed. This is for a consumer that wants to look at what it shipped:
/// a test asserting the entry document carries nothing inline, say.
pub fn file<A: Embed>(name: &str) -> Option<Vec<u8>> {
    let (stored, file) = stored::<A>(compressed::<A>(), name)?;

    match stored {
        Stored::Brotli => unpacked(&file.data),
        Stored::Verbatim => Some(file.data.into_owned()),
    }
}

/// Every name the bundle answers to, as a request would spell it.
///
/// Not `Embed::iter`, which reports how each file is stored: compressed names carry an extension
/// the request does not, and a store says so in a file of its own that is no name at all.
pub fn served<A: Embed>() -> impl Iterator<Item = String> {
    let compressed = compressed::<A>();

    A::iter().filter_map(move |name| {
        if !compressed {
            return Some(name.into_owned());
        }

        if name == crate::COMPRESSED {
            return None;
        }

        // A compressed store holds each name once: in the encoded directory, or outside it. So
        // this is the name with that directory taken off, and every other name as it is.
        Some(name.strip_prefix(ENCODED).unwrap_or(&name).to_owned())
    })
}

/// The encoded directory, as a name in a store is spelled.
const ENCODED: &str = concat!(".br", "/");

/// How a name is held in the binary. The build compresses what compresses and leaves the rest —
/// a png, a woff2 — as it found it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stored {
    Brotli,
    Verbatim,
}

/// Whether this bundle was compressed on its way in.
///
/// Asked of the bundle rather than assumed, because [`Build::compress`](crate::Build::compress)
/// is off by default and a bundle embedded as the bundler wrote it may hold a `.br` file of its
/// own — a dictionary the frontend fetches, a plugin's leftovers. Reading those as an encoding of
/// their neighbour would answer one asset with another.
fn compressed<A: Embed>() -> bool {
    A::get(crate::COMPRESSED).is_some()
}

/// The stored file for a name, compressed if the store holds it that way.
fn stored<A: Embed>(compressed: bool, path: &str) -> Option<(Stored, rust_embed::EmbeddedFile)> {
    if compressed {
        // The store keeps its own work in one directory and its own name for itself. Neither is
        // a name the bundle serves, and answering either would hand out an encoding as though it
        // were a file.
        if path.starts_with(ENCODED) || path == crate::COMPRESSED {
            return None;
        }

        if let Some(file) = A::get(&format!("{ENCODED}{path}")) {
            return Some((Stored::Brotli, file));
        }
    }

    A::get(path).map(|file| (Stored::Verbatim, file))
}

/// Whether the client said it can read brotli. Absent means it did not say, and a client that has
/// not said takes the file itself.
fn takes_brotli(asked_with: &HeaderMap) -> bool {
    // Every line of it: a field that may be a list may also arrive as several lines, and reading
    // only the first would decompress for a client that spelled `br` on the second.
    let entries = asked_with
        .get_all(header::ACCEPT_ENCODING)
        .iter()
        .filter_map(|accepted| accepted.to_str().ok())
        .flat_map(|accepted| accepted.split(','));

    let mut wildcard = None;

    for entry in entries {
        let mut parts = entry.split(';');
        let coding = parts.next().unwrap_or_default().trim();
        let named = coding.eq_ignore_ascii_case("br");

        if !named && coding != "*" {
            continue;
        }

        // `br;q=0` names brotli in order to refuse it, and `*;q=0` refuses everything unnamed.
        let wanted = !parts.any(|parameter| {
            let Some((name, quality)) = parameter.split_once('=') else {
                return false;
            };

            name.trim().eq_ignore_ascii_case("q")
                && quality
                    .trim()
                    .parse::<f32>()
                    .is_ok_and(|quality| quality <= 0.0)
        });

        // Named beats unnamed, whichever came first: `*` is every encoding the request did not
        // mention, so it has nothing to say about one it did (RFC 9110 §12.5.3). Without this,
        // `br;q=0, *` would be read as taking brotli by a client that spelled out that it cannot.
        if named {
            return wanted;
        }

        wildcard = Some(wanted);
    }

    wildcard.unwrap_or(false)
}

/// A file that is embedded and cannot be decoded. Nothing a caller does causes this and nothing
/// they do fixes it, so it is a 500 and not a 404 — and never the entry document, which would be
/// HTML answering a request for a script.
fn unreadable(path: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CACHE_CONTROL, "no-store")],
        format!("{path} is embedded but could not be decoded"),
    )
        .into_response()
}

/// The file behind a stored brotli one, for the client that cannot take it encoded.
fn unpacked(compressed: &[u8]) -> Option<Vec<u8>> {
    let mut unpacked = Vec::new();

    // Only unreachable through a corrupt binary: these bytes were written by the build script
    // that put them here. Answering `None` sends the request to the entry document, which is
    // wrong but is what every other unreadable asset does.
    brotli::BrotliDecompress(&mut &compressed[..], &mut unpacked).ok()?;

    Some(unpacked)
}

/// What the name asked for is, rather than what the stored copy of it is.
fn mimetype(path: &str) -> std::borrow::Cow<'static, str> {
    match mime_guess::from_path(path).first_raw() {
        Some(kind) => Cow::Borrowed(kind),
        None => Cow::Borrowed("application/octet-stream"),
    }
}

/// A validator for one embedded file, from the digest `rust-embed` took at compile time — so
/// this is formatting rather than hashing.
///
/// The digest is of the bytes as stored, so the compressed representation takes it as it is and
/// the decompressed one is marked: same file, same URL, two things a browser must not confuse.
fn etag(digest: &[u8; DIGEST_BYTES], decompressed: bool) -> String {
    use std::fmt::Write as _;

    let mut tag = String::with_capacity(2 + HALF * 2);
    tag.push('"');

    for byte in &digest[..HALF] {
        let _ = write!(tag, "{byte:02x}");
    }

    if decompressed {
        tag.push_str("-id");
    }

    tag.push('"');
    tag
}

/// The SHA-256 `rust-embed` computes for each file.
const DIGEST_BYTES: usize = 32;

/// A validator only separates one version of a URL from the next, so this goes wrong only if a
/// file changes into one agreeing for sixteen whole bytes. The other half would buy nothing and
/// costs thirty-two bytes on every response and every request that carries one back.
const HALF: usize = DIGEST_BYTES / 2;

/// `If-None-Match` carries a list, and a cache may have marked a validator in it weak. Weak is
/// strong enough here: the question is only whether to send the body again.
fn already_held(asked_with: &HeaderMap, tag: &str) -> bool {
    let Some(held) = asked_with.get(header::IF_NONE_MATCH) else {
        return false;
    };

    let Ok(held) = held.to_str() else {
        return false;
    };

    held.split(',').any(|candidate| {
        let candidate = candidate.trim();

        candidate == "*" || candidate.trim_start_matches("W/") == tag
    })
}

/// The name the request is asking for, spelled the way the bundler wrote it. Hashed names never
/// need encoding, but everything copied through from `public/` keeps the name a person gave it,
/// and people name files "company logo.svg".
///
/// `None` for a path that is not UTF-8 once decoded, or that climbs through `..` — worth refusing
/// on its own terms rather than because a lookup happens to miss.
fn requested_name(uri: &Uri) -> Option<Cow<'_, str>> {
    let path = uri.path().trim_start_matches('/');
    let decoded = percent_encoding::percent_decode_str(path)
        .decode_utf8()
        .ok()?;

    (!decoded.split('/').any(|segment| segment == "..")).then_some(decoded)
}

/// Whether a path nothing was embedded under should be answered with the entry document.
///
/// A browser opening a page leads its `Accept` with `text/html`; everything that page pulls in
/// asks for `*/*` or its own type. Asking beats reading the path, which cannot tell
/// `/users/ada.lovelace` from a file. The path is the tiebreak when nothing is stated.
fn wants_the_page(uri: &Uri, asked_with: &HeaderMap) -> bool {
    let Some(accepted) = asked_with
        .get(header::ACCEPT)
        .and_then(|accept| accept.to_str().ok())
    else {
        return !looks_like_a_file(uri);
    };

    accepted.split(',').any(|entry| {
        // Compared whole rather than as a prefix: `text/htmlish` starts the same way.
        let mut parts = entry.split(';');
        let kind = parts.next().unwrap_or_default().trim();

        if !kind.eq_ignore_ascii_case("text/html") {
            return false;
        }

        // `q=0` is a client naming HTML in order to say it will not take any.
        !parts.any(|parameter| {
            let Some((name, quality)) = parameter.split_once('=') else {
                return false;
            };

            name.trim().eq_ignore_ascii_case("q")
                && quality
                    .trim()
                    .parse::<f32>()
                    .is_ok_and(|quality| quality <= 0.0)
        })
    })
}

fn looks_like_a_file(uri: &Uri) -> bool {
    uri.path()
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .contains('.')
}

/// Two mistakes arrive here and are corrected in different places, so this says which: nothing
/// embedded at all is a binary built without its bundle, and a bundle that does not hold this one
/// name is [`Policy::entry`] and the bundler disagreeing about what it is called.
fn missing<A: Embed>(entry: &str) -> Response {
    let said = if A::iter().next().is_some() {
        format!("the bundle in this binary holds no {entry}, which is what `Policy::entry` names\n")
    } else {
        String::from("no frontend was embedded in this binary\n")
    };

    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        said,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_for_returns_immutable_only_for_a_fingerprinted_name() {
        let policy = Policy::holding(["assets/index-DkK_wTPT.js", "assets/png/logo-B1n_snK3.png"]);

        for (name, cache) in [
            ("assets/index-DkK_wTPT.js", &policy.immutable),
            ("assets/png/logo-B1n_snK3.png", &policy.immutable),
            ("favicon.ico", &policy.mutable),
            ("robots.txt", &policy.mutable),
            ("index.html", &policy.mutable),
            ("assets/main.css", &policy.mutable),
            ("assets/logo-B1n_snK3.png", &policy.mutable),
        ] {
            assert_eq!(policy.cache_for(name), cache, "{name}");
        }
    }

    #[test]
    fn cache_for_the_entry_document_returns_mutable_however_it_is_named() {
        for (entry, policy) in [
            (crate::ENTRY, Policy::holding([crate::ENTRY])),
            ("app.html", Policy::holding(["app.html"]).entry("app.html")),
            ("app.html", Policy::holding(["app.html"]).entry("/app.html")),
        ] {
            assert_eq!(policy.cache_for(entry), policy.mutable, "{entry}");
        }
    }

    #[test]
    fn holding_strips_a_leading_slash() {
        let policy = Policy::holding(["/assets/index-DkK_wTPT.js"]);

        assert_eq!(
            policy.fingerprinted(),
            &BTreeSet::from(["assets/index-DkK_wTPT.js".to_owned()])
        );
    }

    #[test]
    fn names_splits_the_published_line() {
        for (published, names) in [
            (
                "assets/a-DkK.js,assets/b-B1n.css",
                &["assets/a-DkK.js", "assets/b-B1n.css"][..],
            ),
            ("", &[][..]),
        ] {
            assert_eq!(
                crate::names(published).collect::<Vec<_>>(),
                names,
                "{published:?}"
            );
        }
    }

    #[test]
    fn etag_quotes_half_the_digest() {
        let tag = etag(&[0xab; DIGEST_BYTES], false);

        assert_eq!(tag, format!("\"{}\"", "ab".repeat(HALF)));
    }

    #[test]
    fn already_held_with_a_matching_validator_returns_true() {
        let tag = etag(&[7; DIGEST_BYTES], false);
        let other_tag = etag(&[8; DIGEST_BYTES], false);

        for if_none_match in [
            tag.clone(),
            format!("W/{tag}"),
            format!("{other_tag}, {tag}"),
            "*".to_owned(),
        ] {
            let headers =
                HeaderMap::from_iter([(header::IF_NONE_MATCH, if_none_match.parse().unwrap())]);

            assert!(already_held(&headers, &tag), "{if_none_match}");
        }
    }

    #[test]
    fn requested_name_climbing_out_of_the_bundle_returns_none() {
        for climbing in [
            "/..",
            "/../secrets",
            "/assets/../../secrets",
            "/%2e%2e/secrets",
            "/..%2fetc/passwd",
            "/..%2f..%2fsecrets",
            "/%2e%2e%2f%2e%2e%2fsecrets",
        ] {
            assert_eq!(
                requested_name(&Uri::from_static(climbing)),
                None,
                "{climbing}"
            );
        }
    }

    // `%c0%af` is an overlong encoding of `/`.
    #[test]
    fn requested_name_of_undecodable_bytes_returns_none() {
        for undecodable in ["/assets/%c0%af", "/%ff%fe"] {
            assert_eq!(
                requested_name(&Uri::from_static(undecodable)),
                None,
                "{undecodable}"
            );
        }
    }

    #[test]
    fn requested_name_decodes_percent_escapes() {
        for (path, name) in [
            ("/assets/index-DkK.js", "assets/index-DkK.js"),
            ("/a%20logo.svg", "a logo.svg"),
            ("/caf%C3%A9.png", "café.png"),
            ("/", ""),
            ("//", ""),
            ("///assets//x.js", "assets//x.js"),
            ("/./x", "./x"),
            ("/%", "%"),
            ("/%2", "%2"),
            ("/%zz", "%zz"),
            ("/%00.js", "\0.js"),
            ("/....//etc/passwd", "....//etc/passwd"),
        ] {
            let uri: Uri = path.parse().unwrap();

            assert_eq!(requested_name(&uri).as_deref(), Some(name), "{path}");
        }
    }

    #[test]
    fn wants_the_page_for_a_dotted_route_returns_true() {
        let route = Uri::from_static("/users/ada.lovelace");
        let headers = HeaderMap::from_iter([(
            header::ACCEPT,
            HeaderValue::from_static(
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            ),
        )]);

        assert!(wants_the_page(&route, &headers));
    }

    #[test]
    fn wants_the_page_reads_accept_the_way_clients_write_it() {
        let route = Uri::from_static("/machines/42");

        for (accept, wanted, why) in [
            ("text/html", true, "the plain form"),
            ("text/html,application/xhtml+xml", true, "a browser's list"),
            ("application/xhtml+xml,text/html;q=0.9", true, "html second"),
            ("text/html; charset=utf-8", true, "with a parameter"),
            ("TEXT/HTML", true, "media types are case-insensitive"),
            ("Text/Html,*/*;q=0.8", true, "and so is a list of them"),
            ("*/*", false, "what a script or a fetch sends"),
            ("image/avif,image/webp,*/*", false, "what an image asks for"),
            ("text/css", false, "a stylesheet is not a page"),
            (
                "text/htmlish",
                false,
                "a longer type that only starts the same",
            ),
            ("", false, "stated and empty is not a preference for html"),
            (
                "application/json, text/html;q=0",
                false,
                "q=0 is a client naming html to refuse it",
            ),
            (
                "text/html;q=0.001",
                true,
                "any other quality is a ranking, and there is nothing here to rank",
            ),
            (
                "text/html;level=0",
                true,
                "level qualifies the html that is asked for, and does not refuse it",
            ),
            (
                "text/html;Q=0",
                false,
                "a parameter name is not case-sensitive",
            ),
        ] {
            let headers = HeaderMap::from_iter([(header::ACCEPT, accept.parse().unwrap())]);

            assert_eq!(
                wants_the_page(&route, &headers),
                wanted,
                "{accept:?}: {why}"
            );
        }
    }

    #[test]
    fn wants_the_page_without_accept_falls_back_to_the_path() {
        for (route, wanted) in [
            ("/machines/42", true),
            ("/", true),
            ("/v1.2/machines", true),
            ("/assets/gone.js", false),
            ("/favicon.ico", false),
        ] {
            assert_eq!(
                wants_the_page(&Uri::from_static(route), &HeaderMap::new()),
                wanted,
                "{route}"
            );
        }
    }
}

#[cfg(test)]
mod embedding {
    use tower::ServiceExt as _;

    use super::*;

    fn embedded_file(
        data: &'static [u8],
        digest: [u8; 32],
        kind: &'static str,
    ) -> rust_embed::EmbeddedFile {
        rust_embed::EmbeddedFile {
            data: Cow::Borrowed(data),
            metadata: rust_embed::Metadata::__rust_embed_new(digest, None, None, kind),
        }
    }

    fn header_value(response: &Response, name: &axum::http::HeaderName) -> String {
        response
            .headers()
            .get(name)
            .unwrap_or_else(|| panic!("no {name} header"))
            .to_str()
            .unwrap()
            .to_owned()
    }

    async fn body(response: Response) -> axum::body::Bytes {
        axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
    }

    struct EmptyBundle;

    impl Embed for EmptyBundle {
        fn get(_: &str) -> Option<rust_embed::EmbeddedFile> {
            None
        }

        fn iter() -> impl Iterator<Item = Cow<'static, str>> + 'static {
            std::iter::empty()
        }
    }

    const FILE_DIGEST: [u8; 32] = [7; 32];

    // A `static`, not a `const`: one test compares addresses, and a `const` has no fixed address.
    static FILE_CONTENTS: &[u8] = b"<!doctype html>";

    const LOGO_NAME: &str = "a logo.svg";

    struct EntryAndLogoBundle;

    impl Embed for EntryAndLogoBundle {
        fn get(path: &str) -> Option<rust_embed::EmbeddedFile> {
            let kind = match path {
                crate::ENTRY => "text/html",
                LOGO_NAME => "image/svg+xml",
                _ => return None,
            };

            Some(embedded_file(FILE_CONTENTS, FILE_DIGEST, kind))
        }

        fn iter() -> impl Iterator<Item = Cow<'static, str>> + 'static {
            [crate::ENTRY, LOGO_NAME].into_iter().map(Cow::Borrowed)
        }
    }

    const JS_SOURCE: &[u8] = b"export const answer = 42;\nexport const question = 'unknown';\n";

    const PNG_SIGNATURE: &[u8] = &[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

    fn brotli_compressed_js() -> &'static [u8] {
        static BROTLI_COMPRESSED_JS: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();

        BROTLI_COMPRESSED_JS.get_or_init(|| {
            let mut compressed = Vec::new();
            brotli::BrotliCompress(
                &mut &JS_SOURCE.repeat(20)[..],
                &mut compressed,
                &brotli::enc::BrotliEncoderParams::default(),
            )
            .unwrap();

            compressed
        })
    }

    /// A store that `Build::compress` filled: `app.js` compressed, `photo.png` as it is, and a
    /// `dict.br` that is an asset and not an encoding.
    struct CompressedBundle;

    impl Embed for CompressedBundle {
        fn get(path: &str) -> Option<rust_embed::EmbeddedFile> {
            let file = match path {
                crate::COMPRESSED => embedded_file(b"", [0; 32], "application/octet-stream"),
                ".br/assets/app.js" => {
                    embedded_file(brotli_compressed_js(), [3; 32], "application/x-brotli")
                }
                "assets/photo.png" => embedded_file(PNG_SIGNATURE, [4; 32], "image/png"),
                "dict.br" => embedded_file(PNG_SIGNATURE, [6; 32], "application/octet-stream"),
                _ => return None,
            };

            Some(file)
        }

        fn iter() -> impl Iterator<Item = Cow<'static, str>> + 'static {
            [
                crate::COMPRESSED,
                ".br/assets/app.js",
                "assets/photo.png",
                "dict.br",
            ]
            .into_iter()
            .map(Cow::Borrowed)
        }
    }

    struct BundleWithNestedIndex;

    impl Embed for BundleWithNestedIndex {
        fn get(path: &str) -> Option<rust_embed::EmbeddedFile> {
            let file = match path {
                crate::ENTRY => {
                    embedded_file(b"<!doctype html><title>shell</title>", [1; 32], "text/html")
                }
                "about/index.html" => {
                    embedded_file(b"<!doctype html><title>about</title>", [2; 32], "text/html")
                }
                _ => return None,
            };

            Some(file)
        }

        fn iter() -> impl Iterator<Item = Cow<'static, str>> + 'static {
            [crate::ENTRY, "about/index.html"]
                .into_iter()
                .map(Cow::Borrowed)
        }
    }

    const DECOMPOSED_NAME: &str = "assets/cafe\u{301}-A1b2C3d4.svg";

    // Not a file on disk: git on macOS composes filenames on checkout, so a fixture file would
    // not keep this spelling.
    struct BundleWithDecomposedName;

    impl Embed for BundleWithDecomposedName {
        fn get(path: &str) -> Option<rust_embed::EmbeddedFile> {
            (path == DECOMPOSED_NAME)
                .then(|| embedded_file(FILE_CONTENTS, FILE_DIGEST, "image/svg+xml"))
        }

        fn iter() -> impl Iterator<Item = Cow<'static, str>> + 'static {
            std::iter::once(Cow::Borrowed(DECOMPOSED_NAME))
        }
    }

    struct BundleWithRenamedEntry;

    impl Embed for BundleWithRenamedEntry {
        fn get(path: &str) -> Option<rust_embed::EmbeddedFile> {
            (path == "app.html").then(|| embedded_file(FILE_CONTENTS, FILE_DIGEST, "text/html"))
        }

        fn iter() -> impl Iterator<Item = Cow<'static, str>> + 'static {
            std::iter::once(Cow::Borrowed("app.html"))
        }
    }

    #[derive(Embed)]
    #[folder = "tests/bundle"]
    #[exclude = ".vite/*"]
    struct BundleOnDisk;

    // `#[exclude = "*.map"]` is the line that the source-map warning of the build tells the user
    // to add.
    #[derive(Embed)]
    #[folder = "tests/bundle"]
    #[exclude = ".vite/*"]
    #[exclude = "*.map"]
    struct BundleOnDiskWithoutSourceMaps;

    const CLIENT_SIDE_ROUTE: &str = "/settings/profile";

    #[tokio::test]
    async fn embedded_of_a_stored_file_by_a_client_taking_brotli_sends_it_as_stored() {
        for accept_encoding in ["gzip, br", "*"] {
            let response = embedded::<CompressedBundle>(Policy::default())
                .oneshot(
                    Request::get("/assets/app.js")
                        .header(header::ACCEPT_ENCODING, accept_encoding)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(
                (
                    response.status(),
                    header_value(&response, &header::CONTENT_ENCODING),
                    header_value(&response, &header::VARY),
                    header_value(&response, &header::CONTENT_TYPE),
                ),
                (
                    StatusCode::OK,
                    "br".to_owned(),
                    "Accept-Encoding".to_owned(),
                    "text/javascript".to_owned()
                ),
                "{accept_encoding}"
            );
            assert_eq!(
                body(response).await,
                brotli_compressed_js(),
                "{accept_encoding}"
            );
        }
    }

    #[tokio::test]
    async fn embedded_of_a_stored_file_by_a_client_taking_nothing_sends_the_file() {
        for accept_encoding in [
            None,
            Some("gzip"),
            Some("br;q=0"),
            Some("identity"),
            Some("*;q=0"),
            Some("brotli"),
        ] {
            let mut request = Request::get("/assets/app.js");
            if let Some(accept_encoding) = accept_encoding {
                request = request.header(header::ACCEPT_ENCODING, accept_encoding);
            }

            let response = embedded::<CompressedBundle>(Policy::default())
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::OK, "{accept_encoding:?}");
            assert_eq!(
                response.headers().get(header::CONTENT_ENCODING),
                None,
                "{accept_encoding:?}"
            );
            assert_eq!(
                body(response).await,
                JS_SOURCE.repeat(20),
                "{accept_encoding:?}"
            );
        }
    }

    #[tokio::test]
    async fn embedded_of_a_stored_file_gives_each_representation_its_own_validator() {
        let compressed_response = embedded::<CompressedBundle>(Policy::default())
            .oneshot(
                Request::get("/assets/app.js")
                    .header(header::ACCEPT_ENCODING, "br")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let compressed_etag = header_value(&compressed_response, &header::ETAG);

        let response = embedded::<CompressedBundle>(Policy::default())
            .oneshot(
                Request::get("/assets/app.js")
                    .header(header::IF_NONE_MATCH, &compressed_etag)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::OK,
            "a validator for the compressed copy answered a request that cannot read it"
        );
    }

    #[tokio::test]
    async fn embedded_of_a_stored_file_with_a_matching_validator_returns_not_modified_and_varies() {
        let first_response = embedded::<CompressedBundle>(Policy::default())
            .oneshot(
                Request::get("/assets/app.js")
                    .header(header::ACCEPT_ENCODING, "br")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let first_etag = header_value(&first_response, &header::ETAG);

        let response = embedded::<CompressedBundle>(Policy::default())
            .oneshot(
                Request::get("/assets/app.js")
                    .header(header::ACCEPT_ENCODING, "br")
                    .header(header::IF_NONE_MATCH, &first_etag)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            (
                response.status(),
                header_value(&response, &header::VARY),
                header_value(&response, &header::ETAG)
            ),
            (
                StatusCode::NOT_MODIFIED,
                "Accept-Encoding".to_owned(),
                first_etag
            )
        );
    }

    #[tokio::test]
    async fn embedded_a_client_side_route_from_a_store_varies_on_accept_and_accept_encoding() {
        let router = embedded::<CompressedBundle>(Policy::default().entry("assets/app.js"));

        let response = router
            .oneshot(
                Request::get("/machines/42")
                    .header(header::ACCEPT, "text/html")
                    .header(header::ACCEPT_ENCODING, "br")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            (
                header_value(&response, &header::CONTENT_ENCODING),
                header_value(&response, &header::VARY)
            ),
            ("br".to_owned(), "Accept, Accept-Encoding".to_owned())
        );
    }

    #[tokio::test]
    async fn embedded_of_the_encoded_directory_returns_not_found() {
        for path in ["/.br/assets/app.js", "/.compressed"] {
            let response = embedded::<CompressedBundle>(Policy::default())
                .oneshot(
                    Request::get(path)
                        .header(header::ACCEPT_ENCODING, "br")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        }
    }

    #[tokio::test]
    async fn embedded_of_an_asset_named_like_an_encoding_returns_the_asset() {
        let router = embedded::<CompressedBundle>(Policy::default());

        let response = router
            .oneshot(
                Request::get("/dict.br")
                    .header(header::ACCEPT_ENCODING, "br")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get(header::CONTENT_ENCODING), None);
        assert_eq!(body(response).await, PNG_SIGNATURE);
    }

    #[tokio::test]
    async fn embedded_of_a_dot_br_file_in_an_uncompressed_bundle_returns_the_file() {
        struct UncompressedBundleWithDotBrFile;

        impl Embed for UncompressedBundleWithDotBrFile {
            fn get(path: &str) -> Option<rust_embed::EmbeddedFile> {
                let file = match path {
                    "data" => embedded_file(b"the file itself", [8; 32], "text/plain"),
                    "data.br" => embedded_file(b"a dictionary", [9; 32], "text/plain"),
                    _ => return None,
                };

                Some(file)
            }

            fn iter() -> impl Iterator<Item = Cow<'static, str>> + 'static {
                ["data", "data.br"].into_iter().map(Cow::Borrowed)
            }
        }

        let router = embedded::<UncompressedBundleWithDotBrFile>(Policy::default());

        let response = router
            .oneshot(
                Request::get("/data.br")
                    .header(header::ACCEPT_ENCODING, "br")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.headers().get(header::CONTENT_ENCODING), None);
        assert_eq!(body(response).await, b"a dictionary".as_slice());
    }

    #[tokio::test]
    async fn embedded_of_a_file_that_cannot_be_decoded_returns_internal_error() {
        struct BundleWithCorruptBrotli;

        impl Embed for BundleWithCorruptBrotli {
            fn get(path: &str) -> Option<rust_embed::EmbeddedFile> {
                let data: &'static [u8] = match path {
                    crate::COMPRESSED => b"",
                    ".br/app.js" => b"this is not brotli",
                    crate::ENTRY => b"<!doctype html>",
                    _ => return None,
                };

                Some(embedded_file(data, [5; 32], "text/html"))
            }

            fn iter() -> impl Iterator<Item = Cow<'static, str>> + 'static {
                [crate::COMPRESSED, ".br/app.js", crate::ENTRY]
                    .into_iter()
                    .map(Cow::Borrowed)
            }
        }

        let router = embedded::<BundleWithCorruptBrotli>(Policy::default());

        let response = router
            .oneshot(
                Request::get("/app.js")
                    .header(header::ACCEPT_ENCODING, "identity")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            (
                response.status(),
                header_value(&response, &header::CACHE_CONTROL)
            ),
            (StatusCode::INTERNAL_SERVER_ERROR, "no-store".to_owned())
        );
    }

    #[tokio::test]
    async fn embedded_of_a_file_stored_as_it_is_returns_it_unencoded() {
        let router = embedded::<CompressedBundle>(Policy::default());

        let response = router
            .oneshot(
                Request::get("/assets/photo.png")
                    .header(header::ACCEPT_ENCODING, "br")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get(header::CONTENT_ENCODING), None);
        assert_eq!(response.headers().get(header::VARY), None);
        assert_eq!(header_value(&response, &header::CONTENT_TYPE), "image/png");
    }

    #[test]
    fn served_returns_the_names_a_request_would_use() {
        let mut names: Vec<String> = served::<CompressedBundle>().collect();

        names.sort();
        assert_eq!(names, ["assets/app.js", "assets/photo.png", "dict.br"]);
    }

    #[tokio::test]
    async fn embedded_of_a_directory_url_returns_the_entry_document() {
        let router = embedded::<BundleWithNestedIndex>(Policy::default());

        let response = router
            .oneshot(
                Request::get("/about/")
                    .header(header::ACCEPT, "text/html")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            body(response).await,
            b"<!doctype html><title>shell</title>".as_slice()
        );
    }

    // macOS finds a file under either Unicode spelling, and under either case. An embedded
    // bundle finds only the exact name.
    #[tokio::test]
    async fn embedded_an_accented_name_returns_only_the_spelling_the_bundler_wrote() {
        let router = embedded::<BundleWithDecomposedName>(Policy::default());

        for (uri, status) in [
            ("/assets/cafe%CC%81-A1b2C3d4.svg", StatusCode::OK),
            ("/assets/caf%C3%A9-A1b2C3d4.svg", StatusCode::NOT_FOUND),
            ("/assets/CAFE%CC%81-A1b2C3d4.svg", StatusCode::NOT_FOUND),
        ] {
            let response = router
                .clone()
                .oneshot(
                    Request::get(uri)
                        .header(header::ACCEPT, "image/svg+xml")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(response.status(), status, "{uri}");
        }
    }

    #[tokio::test]
    async fn embedded_a_bundle_on_disk_returns_each_file_with_its_own_type() {
        let router = embedded::<BundleOnDisk>(Policy::default());

        for (uri, kind) in [
            ("/assets/app-A1b2C3d4.js", "text/javascript"),
            ("/assets/app-B2c3D4e5.css", "text/css"),
            ("/assets/logo-Z9y8X7w6.svg", "image/svg+xml"),
            ("/robots.txt", "text/plain"),
            ("/index.html", "text/html"),
        ] {
            let response = router
                .clone()
                .oneshot(Request::get(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::OK, "{uri}");
            assert!(
                header_value(&response, &header::CONTENT_TYPE).starts_with(kind),
                "{uri}"
            );
        }
    }

    #[test]
    fn embed_excludes_the_bundlers_manifest() {
        let mut names: Vec<_> = BundleOnDisk::iter().collect();

        names.sort();
        assert_eq!(
            names,
            [
                "assets/app-A1b2C3d4.js",
                "assets/app-A1b2C3d4.js.map",
                "assets/app-B2c3D4e5.css",
                "assets/logo-Z9y8X7w6.svg",
                "index.html",
                "robots.txt",
            ]
        );
    }

    #[test]
    fn embed_excludes_source_maps_in_subdirectories() {
        let mut names: Vec<_> = BundleOnDiskWithoutSourceMaps::iter().collect();

        names.sort();
        assert_eq!(
            names,
            [
                "assets/app-A1b2C3d4.js",
                "assets/app-B2c3D4e5.css",
                "assets/logo-Z9y8X7w6.svg",
                "index.html",
                "robots.txt",
            ]
        );
    }

    #[tokio::test]
    async fn embedded_nested_under_a_path_answers_there() {
        let router = axum::Router::new().nest_service(
            "/app",
            embedded::<BundleOnDisk>(Policy::holding(["assets/app-A1b2C3d4.js"])),
        );

        for (uri, kind, cache_control) in [
            (
                "/app/assets/app-A1b2C3d4.js",
                "text/javascript",
                "public, max-age=31536000, immutable",
            ),
            ("/app", "text/html", "no-cache"),
            ("/app/", "text/html", "no-cache"),
            ("/app/settings/deep/link", "text/html", "no-cache"),
        ] {
            let response = router
                .clone()
                .oneshot(
                    Request::get(uri)
                        .header(header::ACCEPT, "text/html,*/*")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::OK, "{uri}");
            assert!(
                header_value(&response, &header::CONTENT_TYPE).starts_with(kind),
                "{uri}"
            );
            assert_eq!(
                header_value(&response, &header::CACHE_CONTROL),
                cache_control,
                "{uri}"
            );
        }
    }

    #[tokio::test]
    async fn embedded_a_client_side_route_returns_the_page_and_varies_on_accept() {
        let router = embedded::<EntryAndLogoBundle>(Policy::default());

        let response = router
            .oneshot(
                Request::get(CLIENT_SIDE_ROUTE)
                    .header(header::ACCEPT, "text/html,application/xhtml+xml")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            (response.status(), header_value(&response, &header::VARY)),
            (StatusCode::OK, "Accept, Accept-Encoding".to_owned())
        );
    }

    #[tokio::test]
    async fn embedded_a_route_without_accepting_html_returns_an_uncacheable_not_found() {
        let router = embedded::<EntryAndLogoBundle>(Policy::default());

        let response = router
            .oneshot(
                Request::get(CLIENT_SIDE_ROUTE)
                    .header(header::ACCEPT, "application/json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            (
                response.status(),
                header_value(&response, &header::VARY),
                header_value(&response, &header::CACHE_CONTROL)
            ),
            (
                StatusCode::NOT_FOUND,
                "Accept, Accept-Encoding".to_owned(),
                "no-store".to_owned()
            )
        );
    }

    #[tokio::test]
    async fn embedded_a_file_by_name_does_not_vary() {
        let router = embedded::<EntryAndLogoBundle>(Policy::default());

        let response = router
            .oneshot(
                Request::get("/a%20logo.svg")
                    .header(header::ACCEPT, "text/html")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get(header::VARY), None);
    }

    #[tokio::test]
    async fn embedded_returns_the_stored_bytes_with_type_nosniff_etag_and_cache_control() {
        let router = embedded::<EntryAndLogoBundle>(Policy::default());

        let response = router
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(
            (
                header_value(&response, &header::CONTENT_TYPE),
                header_value(&response, &header::X_CONTENT_TYPE_OPTIONS),
                header_value(&response, &header::ETAG),
                header_value(&response, &header::CACHE_CONTROL)
            ),
            (
                "text/html".to_owned(),
                "nosniff".to_owned(),
                etag(&FILE_DIGEST, false),
                "no-cache".to_owned()
            )
        );
        assert_eq!(body(response).await, FILE_CONTENTS);
    }

    #[tokio::test]
    async fn embedded_with_a_matching_validator_returns_not_modified_with_no_body() {
        let router = embedded::<EntryAndLogoBundle>(Policy::default());

        let response = router
            .oneshot(
                Request::get("/")
                    .header(header::IF_NONE_MATCH, etag(&FILE_DIGEST, false))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            (
                response.status(),
                header_value(&response, &header::ETAG),
                header_value(&response, &header::CACHE_CONTROL)
            ),
            (
                StatusCode::NOT_MODIFIED,
                etag(&FILE_DIGEST, false),
                "no-cache".to_owned()
            )
        );
        assert_eq!(body(response).await, b"".as_slice());
    }

    #[tokio::test]
    async fn embedded_with_an_outdated_validator_returns_the_file() {
        let router = embedded::<EntryAndLogoBundle>(Policy::default());

        let response = router
            .oneshot(
                Request::get("/")
                    .header(header::IF_NONE_MATCH, "\"gone\"")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn embedded_post_returns_method_not_allowed_with_an_allow_header() {
        let router = embedded::<EmptyBundle>(Policy::default());

        let response = router
            .oneshot(Request::post("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(
            (response.status(), header_value(&response, &header::ALLOW)),
            (StatusCode::METHOD_NOT_ALLOWED, "GET,HEAD".to_owned())
        );
    }

    #[tokio::test]
    async fn embedded_head_is_answered_like_the_get_it_stands_for() {
        let router = embedded::<EntryAndLogoBundle>(Policy::default());
        let get_response = router
            .clone()
            .oneshot(Request::get("/a%20logo.svg").body(Body::empty()).unwrap())
            .await
            .unwrap();

        let head_response = router
            .oneshot(Request::head("/a%20logo.svg").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(head_response.status(), StatusCode::OK);
        assert_eq!(head_response.headers(), get_response.headers());
        assert_eq!(header_value(&head_response, &header::CONTENT_LENGTH), "15");
        assert_eq!(body(head_response).await, b"".as_slice());
    }

    // A copy of the bytes for each request was measured: 1 MB took 16 microseconds, not 1.
    #[tokio::test]
    async fn embedded_hands_over_the_bytes_in_the_binary_without_copying() {
        let router = embedded::<EntryAndLogoBundle>(Policy::default());

        let response = router
            .oneshot(Request::get("/a%20logo.svg").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(body(response).await.as_ptr(), FILE_CONTENTS.as_ptr());
    }

    #[tokio::test]
    async fn embedded_a_file_with_a_query_returns_the_file() {
        let router = embedded::<EntryAndLogoBundle>(Policy::default());

        for uri in [
            "/a%20logo.svg?v=2",
            "/a%20logo.svg?t=1700000000&build=17",
            "/a%20logo.svg?",
        ] {
            let response = router
                .clone()
                .oneshot(Request::get(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::OK, "{uri}");
        }
    }

    #[tokio::test]
    async fn embedded_a_client_side_route_returns_the_renamed_entry_document() {
        let router = embedded::<BundleWithRenamedEntry>(Policy::default().entry("app.html"));

        let response = router
            .oneshot(
                Request::get(CLIENT_SIDE_ROUTE)
                    .header(header::ACCEPT, "text/html")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            (
                response.status(),
                header_value(&response, &header::CONTENT_TYPE)
            ),
            (StatusCode::OK, "text/html".to_owned())
        );
    }

    #[tokio::test]
    async fn embedded_without_the_entry_document_returns_internal_error_naming_it() {
        let router = embedded::<BundleWithRenamedEntry>(Policy::default());

        let response = router
            .oneshot(
                Request::get(CLIENT_SIDE_ROUTE)
                    .header(header::ACCEPT, "text/html")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            body(response).await,
            "the bundle in this binary holds no index.html, which is what `Policy::entry` names\n"
        );
    }

    #[tokio::test]
    async fn embedded_with_nothing_embedded_returns_a_named_internal_error() {
        let router = embedded::<EmptyBundle>(Policy::default());

        let response = router
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            body(response).await,
            "no frontend was embedded in this binary\n"
        );
    }

    #[tokio::test]
    async fn embedded_a_range_request_returns_the_whole_file() {
        let router = embedded::<EntryAndLogoBundle>(Policy::default());

        let response = router
            .oneshot(
                Request::get("/a%20logo.svg")
                    .header(header::RANGE, "bytes=0-3")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get(header::ACCEPT_RANGES), None);
        assert_eq!(body(response).await, FILE_CONTENTS);
    }
}
