//! Fail-closed HTTP egress for provider traffic.
//!
//! Cloud destinations use an exact hostname allowlist, resolve once, reject
//! every non-public answer, and pin the approved socket addresses into the
//! reqwest client. Redirects and ambient proxy variables are disabled so a
//! provider response or host environment cannot move a credential-bearing
//! request to a second destination.

use anyhow::{anyhow, bail, Context, Result};
use reqwest::{redirect::Policy, Client, Url};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

pub struct ValidatedClient {
    pub client: Client,
    pub url: Url,
}

pub async fn cloud_client(
    raw_url: &str,
    allowed_hosts: &[&str],
    connect_timeout: Duration,
    request_timeout: Duration,
) -> Result<ValidatedClient> {
    let url = parse_target(raw_url)?;
    if url.scheme() != "https" {
        bail!("cloud provider egress requires HTTPS");
    }
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("provider URL has no hostname"))?;
    if !allowed_hosts
        .iter()
        .any(|allowed| host.eq_ignore_ascii_case(allowed))
    {
        bail!("provider hostname '{host}' is not allowlisted");
    }
    if url.port_or_known_default() != Some(443) {
        bail!("cloud provider egress requires port 443");
    }

    let addresses = resolve_once(host, 443).await?;
    validate_public_answers(host, &addresses)?;
    let client = pinned_client(host, &addresses, connect_timeout, request_timeout)?;
    Ok(ValidatedClient { client, url })
}

pub async fn loopback_client(
    raw_url: &str,
    connect_timeout: Duration,
    request_timeout: Duration,
) -> Result<ValidatedClient> {
    let url = parse_target(raw_url)?;
    if !matches!(url.scheme(), "http" | "https") {
        bail!("local provider URL must use HTTP or HTTPS");
    }
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("local provider URL has no hostname"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| anyhow!("local provider URL has no port"))?;
    let addresses = resolve_once(host, port).await?;
    if addresses.is_empty() || addresses.iter().any(|address| !address.ip().is_loopback()) {
        bail!("local provider '{host}' did not resolve exclusively to loopback");
    }
    let client = pinned_client(host, &addresses, connect_timeout, request_timeout)?;
    Ok(ValidatedClient { client, url })
}

fn parse_target(raw_url: &str) -> Result<Url> {
    let url = Url::parse(raw_url).context("invalid provider URL")?;
    if !url.username().is_empty() || url.password().is_some() {
        bail!("provider URL must not contain user information");
    }
    if url.fragment().is_some() {
        bail!("provider URL must not contain a fragment");
    }
    Ok(url)
}

async fn resolve_once(host: &str, port: u16) -> Result<Vec<SocketAddr>> {
    let mut addresses: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .with_context(|| format!("could not resolve provider hostname '{host}'"))?
        .collect();
    addresses.sort_unstable();
    addresses.dedup();
    if addresses.is_empty() {
        bail!("provider hostname '{host}' resolved to no addresses");
    }
    Ok(addresses)
}

fn pinned_client(
    host: &str,
    addresses: &[SocketAddr],
    connect_timeout: Duration,
    request_timeout: Duration,
) -> Result<Client> {
    Client::builder()
        .redirect(Policy::none())
        .no_proxy()
        .resolve_to_addrs(host, addresses)
        .connect_timeout(connect_timeout)
        .timeout(request_timeout)
        .build()
        .context("could not build the fail-closed provider client")
}

fn validate_public_answers(host: &str, addresses: &[SocketAddr]) -> Result<()> {
    if addresses.iter().any(|address| !is_public(address.ip())) {
        bail!("provider hostname '{host}' resolved to a non-public address");
    }
    Ok(())
}

fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => public_v4(ip),
        IpAddr::V6(ip) => public_v6(ip),
    }
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
        || ip.is_multicast()
        || a == 0
        || a >= 240
        || a == 100 && (64..=127).contains(&b)
        || a == 192 && b == 0 && c == 0
        || a == 198 && (b == 18 || b == 19))
}

fn public_v6(ip: Ipv6Addr) -> bool {
    let segments = ip.segments();
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] == 0x2001 && segments[1] == 0x0db8)
        || ip.to_ipv4_mapped().is_some_and(|mapped| !public_v4(mapped)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn socket(ip: &str) -> SocketAddr {
        SocketAddr::new(ip.parse().unwrap(), 443)
    }

    #[test]
    fn rejects_private_reserved_and_metadata_addresses() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fe80::1",
            "fc00::1",
            "2001:db8::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "accepted {ip}");
        }
        assert!(is_public("8.8.8.8".parse().unwrap()));
        assert!(is_public("2606:4700:4700::1111".parse().unwrap()));
    }

    #[test]
    fn one_private_dns_answer_fails_the_whole_destination() {
        let answers = vec![socket("8.8.8.8"), socket("169.254.169.254")];
        assert!(validate_public_answers("api.example", &answers).is_err());
    }

    #[tokio::test]
    async fn cloud_policy_rejects_scheme_host_port_and_userinfo_before_dns() {
        let timeout = Duration::from_secs(1);
        for url in [
            "http://api.anthropic.com/v1/messages",
            "https://api.anthropic.com:8443/v1/messages",
            "https://api.anthropic.com.evil.example/v1/messages",
            "https://user:pass@api.anthropic.com/v1/messages",
            "https://127.0.0.1/v1/messages",
        ] {
            assert!(
                cloud_client(url, &["api.anthropic.com"], timeout, timeout)
                    .await
                    .is_err(),
                "accepted {url}"
            );
        }
    }

    #[tokio::test]
    async fn loopback_policy_rejects_wildcard_and_private_lan() {
        let timeout = Duration::from_secs(1);
        assert!(loopback_client("http://0.0.0.0:11434", timeout, timeout)
            .await
            .is_err());
        assert!(
            loopback_client("http://192.168.1.10:11434", timeout, timeout)
                .await
                .is_err()
        );
        assert!(loopback_client("http://127.0.0.1:11434", timeout, timeout)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn hardened_client_never_follows_redirects() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await.unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: http://169.254.169.254/latest/meta-data\r\nContent-Length: 0\r\n\r\n",
                )
                .await
                .unwrap();
        });
        let target = format!("http://127.0.0.1:{}/start", address.port());
        let validated = loopback_client(&target, Duration::from_secs(1), Duration::from_secs(1))
            .await
            .unwrap();
        let response = validated.client.get(validated.url).send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::FOUND);
        server.await.unwrap();
    }
}
