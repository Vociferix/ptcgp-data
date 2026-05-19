use std::time::Duration;

use anyhow::Result;
use tokio::sync::Semaphore;
use tokio::time::sleep;
use tracing::debug;

const USER_AGENT: &str =
    "ptcgp-scraper/0.1 (https://github.com/user/ptcgp-data; educational/archival)";

/// Minimum gap between successive requests to the same logical host group.
/// Two separate semaphore slots can fire requests simultaneously, so the
/// effective rate ceiling is MAX_CONCURRENT / MIN_DELAY_MS per millisecond.
const MIN_DELAY_MS: u64 = 300;
const MAX_CONCURRENT: usize = 5;

pub struct Client {
    inner: reqwest::Client,
    semaphore: Semaphore,
}

impl Client {
    pub fn new() -> Result<Self> {
        let inner = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .gzip(true)
            .brotli(true)
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            inner,
            semaphore: Semaphore::new(MAX_CONCURRENT),
        })
    }

    pub async fn get_text(&self, url: &str) -> Result<String> {
        let _permit = self.semaphore.acquire().await?;
        sleep(Duration::from_millis(MIN_DELAY_MS)).await;
        debug!("GET {url}");
        let text = self
            .inner
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        Ok(text)
    }

    pub async fn get_json(&self, url: &str) -> Result<serde_json::Value> {
        let text = self.get_text(url).await?;
        Ok(serde_json::from_str(&text)?)
    }
}
