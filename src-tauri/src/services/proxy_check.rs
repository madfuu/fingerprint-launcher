//! Proxy connectivity test (R6.3, design §5 `testProxy`): one request through the proxy to an
//! IP lookup service, reporting the exit IP and where it is.
//!
//! Only runs when the user asks. Lookups: `https://ipinfo.io/json`, falling back to
//! `http://ip-api.com/json` (free tier is plain HTTP).

use std::sync::Once;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::db::proxies::{Proxy, ProxyScheme};
use crate::error::{AppError, AppResult, ErrorCode};

/// Per lookup request.
pub const TIMEOUT: Duration = Duration::from_secs(10);
/// Both answers are well under 1 KiB. The fallback is plain HTTP through a proxy the launcher
/// does not control, so the body is read only up to this size.
const MAX_BODY_BYTES: usize = 64 * 1024;

const IPINFO_URL: &str = "https://ipinfo.io/json";
const IP_API_URL: &str = "http://ip-api.com/json";

/// Where the exit IP was looked up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum LookupService {
    Ipinfo,
    IpApi,
}

/// Result of `testProxy`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ProxyCheck {
    /// The exit IP address as seen by the lookup service.
    pub ip: String,
    /// ISO 3166-1 alpha-2 code, e.g. `US`.
    pub country: Option<String>,
    pub city: Option<String>,
    /// IANA time zone of the exit IP, e.g. `America/Los_Angeles`.
    pub timezone: Option<String>,
    pub service: LookupService,
    /// Duration of the successful lookup, connection setup included.
    pub latency_ms: u32,
}

/// Looks up the exit IP through `proxy`; `proxy_test_failed` when both services fail.
pub async fn check(proxy: &Proxy) -> AppResult<ProxyCheck> {
    let client = client(proxy)?;
    let first = lookup(&client, IPINFO_URL, LookupService::Ipinfo).await;
    let first_error = match first {
        Ok(found) => return Ok(found),
        Err(err) => err,
    };
    lookup(&client, IP_API_URL, LookupService::IpApi)
        .await
        .map_err(|second_error| {
            AppError::new(
                ErrorCode::ProxyTestFailed,
                format!("ipinfo.io: {first_error}; ip-api.com: {second_error}"),
            )
        })
}

/// The proxy URL for the HTTP client. SOCKS5 uses `socks5h`, which resolves host names on the
/// proxy like Chromium does with `--proxy-server=socks5://...`; a local lookup would test a
/// different path (and leak DNS).
fn client_proxy_url(proxy: &Proxy) -> String {
    let url = proxy.url();
    match proxy.scheme {
        ProxyScheme::Http => url,
        ProxyScheme::Socks5 => url.replacen("socks5://", "socks5h://", 1),
    }
}

fn client(proxy: &Proxy) -> AppResult<reqwest::Client> {
    install_crypto_provider();
    let invalid = |err: reqwest::Error| {
        AppError::invalid_input(format!("cannot use proxy {}: {err}", proxy.url()))
    };
    reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(client_proxy_url(proxy)).map_err(invalid)?)
        .timeout(TIMEOUT)
        // Both services answer directly; a redirect would only come from the proxy.
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("fingerprint-launcher/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(invalid)
}

/// reqwest is built without a bundled crypto provider (`rustls-no-provider`); ring is the one
/// that builds everywhere without extra tools.
fn install_crypto_provider() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // Fails only when a provider is already installed, which is just as good.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

async fn lookup(
    client: &reqwest::Client,
    url: &str,
    service: LookupService,
) -> Result<ProxyCheck, String> {
    let started = Instant::now();
    let mut response = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|err| error_chain(&err))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|err| error_chain(&err))? {
        if body.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(format!(
                "the response is larger than {MAX_BODY_BYTES} bytes"
            ));
        }
        body.extend_from_slice(&chunk);
    }
    let latency_ms = u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX);
    let parsed = match service {
        LookupService::Ipinfo => parse_ipinfo(&body),
        LookupService::IpApi => parse_ip_api(&body),
    }?;
    Ok(ProxyCheck {
        service,
        latency_ms,
        ..parsed
    })
}

/// reqwest's top-level message ("error sending request") hides the cause, e.g. "connection
/// refused" or a SOCKS failure.
fn error_chain(err: &dyn std::error::Error) -> String {
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// `{"ip": "...", "city": "...", "country": "US", "timezone": "...", ...}`
fn parse_ipinfo(body: &[u8]) -> Result<ProxyCheck, String> {
    #[derive(Deserialize)]
    struct Ipinfo {
        ip: Option<String>,
        city: Option<String>,
        country: Option<String>,
        timezone: Option<String>,
    }
    let info: Ipinfo = serde_json::from_slice(body).map_err(|err| err.to_string())?;
    found(
        info.ip,
        info.country,
        info.city,
        info.timezone,
        LookupService::Ipinfo,
    )
}

/// `{"status": "success", "query": "<ip>", "countryCode": "US", "city": "...", "timezone": "..."}`
/// or `{"status": "fail", "message": "..."}`.
fn parse_ip_api(body: &[u8]) -> Result<ProxyCheck, String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct IpApi {
        status: Option<String>,
        message: Option<String>,
        query: Option<String>,
        country_code: Option<String>,
        city: Option<String>,
        timezone: Option<String>,
    }
    let info: IpApi = serde_json::from_slice(body).map_err(|err| err.to_string())?;
    if info.status.as_deref() != Some("success") {
        return Err(info.message.unwrap_or_else(|| "lookup failed".to_owned()));
    }
    found(
        info.query,
        info.country_code,
        info.city,
        info.timezone,
        LookupService::IpApi,
    )
}

fn found(
    ip: Option<String>,
    country: Option<String>,
    city: Option<String>,
    timezone: Option<String>,
    service: LookupService,
) -> Result<ProxyCheck, String> {
    let clean = |v: Option<String>| v.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
    Ok(ProxyCheck {
        ip: clean(ip).ok_or("the response has no IP address")?,
        country: clean(country),
        city: clean(city),
        timezone: clean(timezone),
        service,
        latency_ms: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proxy(scheme: ProxyScheme, host: &str) -> Proxy {
        Proxy {
            id: "p".into(),
            name: None,
            scheme,
            host: host.into(),
            port: 1080,
            note: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn proxy_urls_resolve_names_on_the_proxy_and_bracket_ipv6() {
        assert_eq!(
            client_proxy_url(&proxy(ProxyScheme::Socks5, "proxy.example.com")),
            "socks5h://proxy.example.com:1080"
        );
        assert_eq!(
            client_proxy_url(&proxy(ProxyScheme::Socks5, "::1")),
            "socks5h://[::1]:1080"
        );
        assert_eq!(
            client_proxy_url(&proxy(ProxyScheme::Http, "2001:db8::1")),
            "http://[2001:db8::1]:1080"
        );
    }

    #[test]
    fn builds_a_client_for_every_proxy_form() {
        for p in [
            proxy(ProxyScheme::Socks5, "::1"),
            proxy(ProxyScheme::Http, "10.0.0.1"),
            proxy(ProxyScheme::Socks5, "proxy.example.com"),
        ] {
            client(&p).unwrap();
        }
    }

    #[test]
    fn parses_ipinfo() {
        let body = br#"{"ip":"203.0.113.7","city":"Los Angeles","region":"California",
            "country":"US","loc":"34.05,-118.24","timezone":"America/Los_Angeles",
            "readme":"https://ipinfo.io/missingauth"}"#;
        assert_eq!(
            parse_ipinfo(body).unwrap(),
            ProxyCheck {
                ip: "203.0.113.7".into(),
                country: Some("US".into()),
                city: Some("Los Angeles".into()),
                timezone: Some("America/Los_Angeles".into()),
                service: LookupService::Ipinfo,
                latency_ms: 0,
            }
        );
        // Only the IP is required.
        let minimal = parse_ipinfo(br#"{"ip":"2001:db8::7","city":""}"#).unwrap();
        assert_eq!((minimal.ip.as_str(), minimal.city), ("2001:db8::7", None));
        assert!(parse_ipinfo(br#"{"error":{"title":"Rate limit exceeded"}}"#).is_err());
        assert!(parse_ipinfo(b"<html>").is_err());
    }

    #[test]
    fn parses_ip_api() {
        let body = br#"{"status":"success","country":"Japan","countryCode":"JP",
            "city":"Tokyo","timezone":"Asia/Tokyo","query":"198.51.100.4"}"#;
        assert_eq!(
            parse_ip_api(body).unwrap(),
            ProxyCheck {
                ip: "198.51.100.4".into(),
                country: Some("JP".into()),
                city: Some("Tokyo".into()),
                timezone: Some("Asia/Tokyo".into()),
                service: LookupService::IpApi,
                latency_ms: 0,
            }
        );
        assert_eq!(
            parse_ip_api(br#"{"status":"fail","message":"reserved range","query":"10.0.0.1"}"#)
                .unwrap_err(),
            "reserved range"
        );
    }

    /// A local HTTP proxy that refuses `CONNECT` (so the HTTPS lookup fails) and answers every
    /// plain request with `body`. Returns the proxy entry pointing at it.
    async fn fake_proxy(body: Vec<u8>) -> Proxy {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let body = body.clone();
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut buf = [0; 1024];
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        match socket.read(&mut buf).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => request.extend_from_slice(&buf[..n]),
                        }
                    }
                    let response = if request.starts_with(b"CONNECT ") {
                        b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_vec()
                    } else {
                        let mut head = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                             Content-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .into_bytes();
                        head.extend_from_slice(&body);
                        head
                    };
                    let _ = socket.write_all(&response).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        Proxy {
            port,
            ..proxy(ProxyScheme::Http, "127.0.0.1")
        }
    }

    #[tokio::test]
    async fn falls_back_to_the_second_service() {
        let proxy = fake_proxy(
            br#"{"status":"success","countryCode":"JP","city":"Tokyo","timezone":"Asia/Tokyo","query":"198.51.100.4"}"#
                .to_vec(),
        )
        .await;
        let found = check(&proxy).await.unwrap();
        assert_eq!(found.service, LookupService::IpApi);
        assert_eq!(found.ip, "198.51.100.4");
        assert_eq!(found.country.as_deref(), Some("JP"));
    }

    #[tokio::test]
    async fn refuses_oversized_answers() {
        let proxy = fake_proxy(vec![b' '; MAX_BODY_BYTES + 1]).await;
        let err = check(&proxy).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::ProxyTestFailed);
        assert!(err.message.contains("ipinfo.io: "), "{}", err.message);
        assert!(err.message.contains("larger than"), "{}", err.message);
    }

    #[test]
    fn serializes_for_the_bridge() {
        let check = ProxyCheck {
            ip: "1.2.3.4".into(),
            country: None,
            city: None,
            timezone: Some("UTC".into()),
            service: LookupService::IpApi,
            latency_ms: 12,
        };
        assert_eq!(
            serde_json::to_value(check).unwrap(),
            serde_json::json!({
                "ip": "1.2.3.4", "country": null, "city": null, "timezone": "UTC",
                "service": "ip_api", "latencyMs": 12,
            })
        );
    }
}
