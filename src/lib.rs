use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::{
    Router,
    extract::Request,
    http::{HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use tower_governor::GovernorLayer;
use tower_governor::errors::GovernorError;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::key_extractor::KeyExtractor;
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::db::DbPool;

pub mod api;
pub mod db;
pub mod error;
pub mod models;
pub mod schema;
pub mod secrets;
pub mod services;

/// Test/seed secrets, excluded from CodeQL analysis via `paths-ignore`.
/// Not gated on `cfg(test)` so `src/bin/seed.rs` can also use them.
pub mod test_secrets;

/// Rate-limit deployment topology.
///
/// - `Direct` — only our server: rate limit on the direct peer IP.
///   Forwarded headers (`X-Forwarded-For`, `X-Real-IP`) are IGNORED so
///   a client cannot spoof a fresh bucket per request.
/// - `Proxy` — behind a reverse proxy (nginx etc.) that appends the real
///   client IP to `X-Forwarded-For`: use the RIGHTMOST entry (the one our
///   own proxy added, immune to client-supplied leftmost spoofing), then
///   `X-Real-IP`, then the direct peer as last resort.
/// - `Disabled` — reverse proxy handles rate limiting, none on our side.
///
/// The rate-limit budget itself is tunable via env vars (when not
/// `Disabled`): `RATE_LIMIT_PER_SECOND` (default `12`) is the replenish
/// interval in seconds per slot, and `RATE_LIMIT_BURST_SIZE` (default `5`)
/// is the maximum burst. Invalid/unset values fall back to the defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitMode {
    Direct,
    Proxy,
    Disabled,
}

impl RateLimitMode {
    /// Parse and validate the `RATE_LIMIT_MODE` env var, failing fast on
    /// unknown values instead of silently picking the wrong topology.
    pub fn from_env() -> Self {
        let raw = std::env::var("RATE_LIMIT_MODE").unwrap_or_else(|_| "direct".to_string());
        match raw.trim().to_ascii_lowercase().as_str() {
            "direct" => RateLimitMode::Direct,
            "proxy" => RateLimitMode::Proxy,
            "disabled" => RateLimitMode::Disabled,
            other => panic!("invalid RATE_LIMIT_MODE {other:?}: expected direct|proxy|disabled"),
        }
    }
}

/// Rate-limiting key extractor that resolves the real client IP based on
/// the deployment topology.
///
/// In `Direct` mode only the peer IP from `ConnectInfo<SocketAddr>` is used —
/// `X-Forwarded-For` / `X-Real-IP` are never read, so a client talking
/// directly to the server cannot rotate spoofed IPs to get a fresh bucket.
/// In `Proxy` mode the RIGHTMOST `X-Forwarded-For` entry is preferred (the
/// one appended by our own reverse proxy), then `X-Real-IP`, then the peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecureClientIpKeyExtractor {
    pub(crate) mode: RateLimitMode,
}

impl KeyExtractor for SecureClientIpKeyExtractor {
    type Key = IpAddr;

    fn extract<T>(&self, req: &Request<T>) -> Result<Self::Key, GovernorError> {
        match self.mode {
            RateLimitMode::Direct => {
                // Only the direct peer IP — never trust forwarded headers
                let ci = req
                    .extensions()
                    .get::<axum::extract::ConnectInfo<SocketAddr>>()
                    .ok_or(GovernorError::UnableToExtractKey)?;
                Ok(ci.0.ip())
            }
            RateLimitMode::Proxy => {
                // Get the direct peer IP first — forwarded headers are only
                // trusted when the peer is a configured reverse proxy
                let ci = req
                    .extensions()
                    .get::<axum::extract::ConnectInfo<SocketAddr>>()
                    .ok_or(GovernorError::UnableToExtractKey)?;
                let peer_ip = ci.0.ip();

                if forwarded_headers_trusted(peer_ip, &trusted_proxies()) {
                    // Rightmost XFF (appended by our own trusted proxy) first
                    if let Ok(ip) = client_ip::rightmost_x_forwarded_for(req.headers()) {
                        return Ok(ip);
                    }
                    // Then X-Real-IP (set by nginx and similar proxies)
                    if let Some(ip) = x_real_ip(req.headers()) {
                        return Ok(ip);
                    }
                }
                // Fall back to the direct peer when headers are untrusted
                Ok(peer_ip)
            }
            RateLimitMode::Disabled => {
                unreachable!("disabled mode never constructs an extractor")
            }
        }
    }
}

/// Extract the client IP from the `X-Real-IP` header (set by nginx and
/// similar reverse proxies), returning `None` if absent or malformed.
fn x_real_ip(headers: &axum::http::HeaderMap) -> Option<IpAddr> {
    headers
        .get("x-real-ip")
        .and_then(|hv| hv.to_str().ok())
        .and_then(|s| s.trim().parse::<IpAddr>().ok())
}

/// A trusted reverse-proxy entry: either a plain IP or a CIDR range
/// (e.g. `10.0.0.0/8`). Used to decide whether the rightmost
/// `X-Forwarded-For` entry may be trusted for a request.
#[derive(Debug, Clone)]
pub enum TrustedProxy {
    Ip(IpAddr),
    Cidr { network: IpAddr, prefix_len: u8 },
}

impl TrustedProxy {
    /// Parse a single `TRUSTED_PROXIES` entry — either a plain IP or
    /// CIDR notation (`network/prefix`). Returns `None` on malformed input.
    fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if let Some((network_str, prefix_str)) = s.split_once('/') {
            let network = network_str.parse::<IpAddr>().ok()?;
            let prefix_len: u8 = prefix_str.parse().ok()?;
            match network {
                IpAddr::V4(_) if prefix_len > 32 => None,
                IpAddr::V6(_) if prefix_len > 128 => None,
                _ => Some(TrustedProxy::Cidr {
                    network,
                    prefix_len,
                }),
            }
        } else {
            let ip = s.parse::<IpAddr>().ok()?;
            Some(TrustedProxy::Ip(ip))
        }
    }

    /// Returns `true` when `ip` falls within this entry (IP equality for
    /// plain IPs, CIDR containment for ranges).
    fn contains(&self, ip: IpAddr) -> bool {
        match self {
            TrustedProxy::Ip(proxy_ip) => proxy_ip == &ip,
            TrustedProxy::Cidr {
                network,
                prefix_len,
            } => match (network, ip) {
                (IpAddr::V4(net), IpAddr::V4(addr)) => {
                    if *prefix_len == 0 {
                        return true;
                    }
                    let mask = u32::MAX << (32 - *prefix_len);
                    (u32::from(*net) & mask) == (u32::from(addr) & mask)
                }
                (IpAddr::V6(net), IpAddr::V6(addr)) => {
                    if *prefix_len == 0 {
                        return true;
                    }
                    let mask = u128::MAX << (128 - *prefix_len);
                    (u128::from(*net) & mask) == (u128::from(addr) & mask)
                }
                _ => false,
            },
        }
    }
}

/// Parse a `u32` from the named env var, falling back to `default` when the
/// var is unset, not a valid `u32`, or zero. A zero value would panic later
/// in `GovernorConfigBuilder` (per-second/burst size must be positive).
fn env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.trim().parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

/// Returns the list of trusted reverse-proxy entries from the
/// `TRUSTED_PROXIES` env var (comma-separated). Empty/unset ⇒ none.
fn trusted_proxies() -> Vec<TrustedProxy> {
    std::env::var("TRUSTED_PROXIES")
        .unwrap_or_default()
        .split(',')
        .filter_map(TrustedProxy::parse)
        .collect()
}

/// Returns `true` when the rightmost XFF / X-Real-IP may be trusted for
/// this request:
/// - `TRUSTED_PROXIES` is unset/empty ⇒ **do not trust any forwarded
///   headers**; use only the direct peer IP (secure default — spoofing
///   a fresh bucket via `X-Forwarded-For` is impossible)
/// - `TRUSTED_PROXIES` is set ⇒ trust only when the request's direct peer
///   is one of the listed proxies (plain IP or CIDR range)
fn forwarded_headers_trusted(direct_peer: IpAddr, trusted: &[TrustedProxy]) -> bool {
    trusted.iter().any(|t| t.contains(direct_peer))
}

/// The shared application state injected into all Axum handlers.
#[derive(Clone)]
pub struct AppState {
    pub pool: DbPool,
    pub image_storage_path: std::path::PathBuf,
}

/// Validates the JWT from the Authorization header and inserts the
/// user id into request extensions. Returns 401 with the standard
/// error envelope on any failure.
pub async fn auth_middleware(mut req: Request, next: Next) -> Response {
    let auth_header = req.headers().get(axum::http::header::AUTHORIZATION);

    if let Some(auth_value) = auth_header
        && let Ok(auth_str) = auth_value.to_str()
        && auth_str.starts_with("Bearer ")
    {
        let token = &auth_str[7..];
        let secret = services::auth::jwt_secret();
        return match services::auth::verify_access_token(token, secret) {
            Ok(user_id) => {
                req.extensions_mut().insert(user_id);
                next.run(req).await
            }
            Err(_) => unauthorized_envelope(),
        };
    }

    unauthorized_envelope()
}

/// Builds the standard 401 error envelope response (matching the
/// shape produced by `AppError::Unauthorized` in `src/error.rs`).
fn unauthorized_envelope() -> Response {
    use serde_json::json;

    let body = json!({
        "error": {
            "code": "UNAUTHORIZED",
            "message": "invalid or expired token"
        }
    });
    (StatusCode::UNAUTHORIZED, axum::Json(body)).into_response()
}

pub fn app(state: AppState) -> Router {
    // Public auth routes — rate limited via tower_governor (set
    // RATE_LIMIT_MODE=disabled in tests so oneshot() requests don't
    // need ConnectInfo)
    let mode = RateLimitMode::from_env();
    let governor_enabled = mode != RateLimitMode::Disabled;

    let public = if governor_enabled {
        let per_second = env_u32("RATE_LIMIT_PER_SECOND", 12);
        let burst_size = env_u32("RATE_LIMIT_BURST_SIZE", 5);
        let governor_conf = Arc::new(
            GovernorConfigBuilder::default()
                .per_second(u64::from(per_second)) // default: 1 slot per 12s = 5/min sustained
                .burst_size(burst_size) // default: burst of 5
                .key_extractor(SecureClientIpKeyExtractor { mode })
                .finish()
                .unwrap(),
        );
        let mut layer = GovernorLayer::new(governor_conf);
        layer = layer.error_handler(|e: GovernorError| {
            use serde_json::json;
            let (status, code, message) = match &e {
                GovernorError::TooManyRequests { wait_time, .. } => (
                    StatusCode::TOO_MANY_REQUESTS,
                    "TOO_MANY_REQUESTS",
                    format!("rate limit exceeded, retry in {wait_time}s"),
                ),
                GovernorError::UnableToExtractKey => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL_SERVER_ERROR",
                    "unable to extract rate limit key".to_string(),
                ),
                GovernorError::Other { code, msg, .. } => (
                    *code,
                    "INTERNAL_SERVER_ERROR",
                    msg.clone().unwrap_or_else(|| "unknown error".to_string()),
                ),
            };
            let body = json!({
                "error": {
                    "code": code,
                    "message": message
                }
            });
            (status, axum::Json(body)).into_response()
        });
        api::rest::auth::public_router().layer(layer)
    } else {
        api::rest::auth::public_router()
    };
    // Protected auth routes — with auth middleware
    let protected_auth =
        api::rest::auth::protected_router().route_layer(middleware::from_fn(auth_middleware));
    // Rest of the API — with auth middleware
    let rest = api::rest::router().route_layer(middleware::from_fn(auth_middleware));

    // Images: public GET (no auth), protected POST/DELETE (auth + owner check)
    let images = api::rest::images::public_router().merge(
        api::rest::images::protected_router().route_layer(middleware::from_fn(auth_middleware)),
    );

    Router::new()
        .nest("/api/v1/auth", public.merge(protected_auth))
        .nest("/api/v1/images", images)
        .nest("/api/v1", rest)
        .layer(cors_layer())
        .with_state(state)
}

/// CORS: allow only the configured origins (default localhost:5173 for
/// dev), only the methods the API uses, and only the headers the API
/// needs. Authorization must be listed explicitly (it is NOT covered
/// by `*` per the Fetch spec).
fn cors_layer() -> CorsLayer {
    let origins = std::env::var("CORS_ALLOWED_ORIGINS")
        .unwrap_or_else(|_| "http://localhost:5173".to_string());

    let origin_list: Vec<HeaderValue> = origins
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .filter_map(|s| match HeaderValue::from_str(&s) {
            Ok(hv) => Some(hv),
            Err(_) => {
                eprintln!("warning: skipping invalid CORS origin: {s:?}");
                None
            }
        })
        .collect();

    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origin_list))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([
            axum::http::header::AUTHORIZATION,
            axum::http::header::CONTENT_TYPE,
        ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn test_forwarded_headers_trusted() {
        let proxy = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let client = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 1));

        // TRUSTED_PROXIES unset/empty: do NOT trust forwarded headers
        assert!(!forwarded_headers_trusted(proxy, &[]));
        assert!(!forwarded_headers_trusted(client, &[]));

        // TRUSTED_PROXIES set to the proxy: trust when peer is the proxy
        assert!(forwarded_headers_trusted(proxy, &[TrustedProxy::Ip(proxy)]));

        // Direct client (not a trusted proxy): don't trust
        assert!(!forwarded_headers_trusted(
            client,
            &[TrustedProxy::Ip(proxy)]
        ));
    }

    #[test]
    fn test_env_u32_rejects_zero() {
        const VAR: &str = "FOODLY_TEST_ENV_U32";
        // SAFETY: unique variable name owned by this test; no other test reads it.
        unsafe { std::env::set_var(VAR, "0") };
        assert_eq!(env_u32(VAR, 12), 12);
        unsafe { std::env::set_var(VAR, "7") };
        assert_eq!(env_u32(VAR, 12), 7);
        unsafe { std::env::remove_var(VAR) };
        assert_eq!(env_u32(VAR, 12), 12);
    }

    #[test]
    fn test_cidr_contains() {
        // 10.0.0.0/8 covers all 10.x.x.x addresses
        let cidr = TrustedProxy::Cidr {
            network: IpAddr::V4(Ipv4Addr::new(10, 0, 0, 0)),
            prefix_len: 8,
        };
        assert!(cidr.contains(IpAddr::V4(Ipv4Addr::new(10, 255, 255, 255))));
        assert!(cidr.contains(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(!cidr.contains(IpAddr::V4(Ipv4Addr::new(11, 0, 0, 1))));
        assert!(!cidr.contains(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));

        // /32 is an exact IP match
        let exact = TrustedProxy::Cidr {
            network: IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5)),
            prefix_len: 32,
        };
        assert!(exact.contains(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5))));
        assert!(!exact.contains(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 6))));
    }
}
