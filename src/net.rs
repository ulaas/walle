use std::{
    sync::OnceLock,
    time::{Duration, Instant},
};

use anyhow::{Result, bail};
use tokio::sync::Mutex;

pub const USER_AGENT: &str = concat!("walle/", env!("CARGO_PKG_VERSION"), " (wallpaper browser)");

/// The shared HTTP client: clones share one connection pool, so requests from all the
/// provider objects can reuse connections.
pub fn client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .user_agent(USER_AGENT)
                // Per read rather than total, so that big images on slow links can finish.
                .read_timeout(Duration::from_secs(30))
                .connect_timeout(Duration::from_secs(15))
                .build()
                .expect("http client")
        })
        .clone()
}

/// Token bucket: `capacity` requests, refilled at `capacity / per` .
pub struct RateLimiter {
    capacity: f64,
    refill_per_sec: f64,
    state: Mutex<(f64, Instant)>,
}

impl RateLimiter {
    pub fn new(capacity: u32, per: Duration) -> Self {
        Self {
            capacity: capacity as f64,
            refill_per_sec: capacity as f64 / per.as_secs_f64(),
            state: Mutex::new((capacity as f64, Instant::now())),
        }
    }

    pub async fn acquire(&self) {
        loop {
            let wait = {
                let mut st = self.state.lock().await;
                let now = Instant::now();
                let elapsed = now.duration_since(st.1).as_secs_f64();
                st.0 = (st.0 + elapsed * self.refill_per_sec).min(self.capacity);
                st.1 = now;
                if st.0 >= 1.0 {
                    st.0 -= 1.0;
                    return;
                }
                (1.0 - st.0) / self.refill_per_sec
            };
            tokio::time::sleep(Duration::from_secs_f64(wait)).await;
        }
    }
}

/// GET returning bytes, with one retry on 429 (honouring Retry-After when present).
pub async fn get_bytes(client: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    get_bytes_with(client, url, &[]).await
}

pub async fn get_bytes_with(client: &reqwest::Client, url: &str, headers: &[(&str, &str)]) -> Result<Vec<u8>> {
    for attempt in 0..2 {
        let mut req = client.get(url);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let resp = req.send().await.map_err(|e| e.without_url())?;
        let status = resp.status();
        if status.as_u16() == 429 && attempt == 0 {
            let secs = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(5);
            tokio::time::sleep(Duration::from_secs(secs.min(60))).await;
            continue;
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            let body: String = body.chars().take(200).collect();
            bail!("HTTP {status}: {}", body.trim());
        }
        return Ok(resp.bytes().await.map_err(|e| e.without_url())?.to_vec());
    }
    bail!("rate limited by {}", url.split('?').next().unwrap_or(url))
}

pub async fn get_json<T: serde::de::DeserializeOwned>(client: &reqwest::Client, url: &str) -> Result<T> {
    get_json_with(client, url, &[]).await
}

pub async fn get_json_with<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
    headers: &[(&str, &str)],
) -> Result<T> {
    let bytes = get_bytes_with(client, url, headers).await?;
    let shown = url.split('?').next().unwrap_or(url);
    serde_json::from_slice(&bytes).map_err(|e| anyhow::anyhow!("bad JSON from {shown}: {e}"))
}

/// Make a string safe for use as a file name.
pub fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '_' })
        .collect()
}

/// File name for a cache entry. The hash keeps names that sanitize alike apart
/// (`a b.jpg` and `a_b.jpg`) and the length within what file systems accept.
pub fn cache_file_name(key: &str) -> String {
    let readable: String = sanitize(key).chars().take(80).collect();
    format!("{readable}-{:016x}", fnv64(key))
}

/// Stable 64-bit FNV-1a hash (std's hasher is not stable across releases).
pub fn fnv64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_names_do_not_collide_and_stay_short() {
        assert_ne!(cache_file_name("thumb:local:a b.jpg"), cache_file_name("thumb:local:a_b.jpg"));
        assert_eq!(cache_file_name("x"), cache_file_name("x"));
        assert!(cache_file_name(&"long/".repeat(200)).len() < 120);
    }
}
