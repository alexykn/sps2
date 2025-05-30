//! HTTP client with connection pooling and retry logic

use futures::StreamExt;
use reqwest::{Client, Response, StatusCode};
use spsv2_errors::{Error, NetworkError};
use std::time::Duration;

/// Download progress information
#[derive(Debug, Clone)]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: u64,
}

/// Network client configuration
#[derive(Debug, Clone)]
pub struct NetConfig {
    pub timeout: Duration,
    pub connect_timeout: Duration,
    pub pool_idle_timeout: Duration,
    pub pool_max_idle_per_host: usize,
    pub retry_count: u32,
    pub retry_delay: Duration,
    pub user_agent: String,
}

impl Default for NetConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(300), // 5 minutes for large downloads
            connect_timeout: Duration::from_secs(30),
            pool_idle_timeout: Duration::from_secs(90),
            pool_max_idle_per_host: 10,
            retry_count: 3,
            retry_delay: Duration::from_secs(1),
            user_agent: format!("spsv2/{}", env!("CARGO_PKG_VERSION")),
        }
    }
}

/// HTTP client wrapper with retry logic
#[derive(Clone)]
pub struct NetClient {
    client: Client,
    config: NetConfig,
}

impl NetClient {
    /// Create a new network client
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP client cannot be created due to invalid configuration
    /// or if the underlying reqwest client fails to initialize.
    pub fn new(config: NetConfig) -> Result<Self, Error> {
        let client = Client::builder()
            .timeout(config.timeout)
            .connect_timeout(config.connect_timeout)
            .pool_idle_timeout(config.pool_idle_timeout)
            .pool_max_idle_per_host(config.pool_max_idle_per_host)
            .user_agent(&config.user_agent)
            .build()
            .map_err(|e| NetworkError::ConnectionRefused(e.to_string()))?;

        Ok(Self { client, config })
    }

    /// Create with default configuration
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP client cannot be created with default settings.
    pub fn with_defaults() -> Result<Self, Error> {
        Self::new(NetConfig::default())
    }

    /// Execute a GET request with retries
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails after all retry attempts, including
    /// network timeouts, connection failures, or server errors.
    pub async fn get(&self, url: &str) -> Result<Response, Error> {
        self.retry_request(|| self.client.get(url).send()).await
    }

    /// Execute a HEAD request with retries
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails after all retry attempts, including
    /// network timeouts, connection failures, or server errors.
    pub async fn head(&self, url: &str) -> Result<Response, Error> {
        self.retry_request(|| self.client.head(url).send()).await
    }

    /// Download file with progress callback
    ///
    /// # Errors
    ///
    /// Returns an error if the download fails, the file cannot be created,
    /// or if there are I/O errors while writing the downloaded content.
    pub async fn download_file_with_progress<F>(
        &self,
        url: &str,
        dest: &std::path::Path,
        progress_callback: F,
    ) -> Result<(), Error>
    where
        F: Fn(DownloadProgress),
    {
        let response = self.get(url).await?;
        let total_size = response.content_length().unwrap_or(0);

        let mut file = tokio::fs::File::create(dest).await?;
        let mut stream = response.bytes_stream();
        let mut downloaded = 0u64;

        while let Some(chunk) = stream.next().await {
            let chunk =
                chunk.map_err(|e| spsv2_errors::NetworkError::DownloadFailed(e.to_string()))?;
            tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await?;

            downloaded += chunk.len() as u64;
            progress_callback(DownloadProgress {
                downloaded,
                total: total_size,
            });
        }

        Ok(())
    }

    /// Execute a request with retries
    async fn retry_request<F, Fut>(&self, mut f: F) -> Result<Response, Error>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<Response, reqwest::Error>>,
    {
        let mut last_error = None;

        for attempt in 0..=self.config.retry_count {
            if attempt > 0 {
                tokio::time::sleep(self.config.retry_delay * attempt).await;
            }

            match f().await {
                Ok(response) => {
                    // Check for rate limiting
                    if response.status() == StatusCode::TOO_MANY_REQUESTS {
                        if let Some(retry_after) = response
                            .headers()
                            .get("retry-after")
                            .and_then(|v| v.to_str().ok())
                            .and_then(|s| s.parse::<u64>().ok())
                        {
                            return Err(NetworkError::RateLimited {
                                seconds: retry_after,
                            }
                            .into());
                        }
                    }

                    return Ok(response);
                }
                Err(e) => {
                    last_error = Some(e);

                    // Don't retry on certain errors
                    if !Self::should_retry(last_error.as_ref().unwrap()) {
                        break;
                    }
                }
            }
        }

        // Convert the last error
        match last_error {
            Some(e) if e.is_timeout() => Err(NetworkError::Timeout {
                url: e
                    .url()
                    .map(std::string::ToString::to_string)
                    .unwrap_or_default(),
            }
            .into()),
            Some(e) if e.is_connect() => Err(NetworkError::ConnectionRefused(e.to_string()).into()),
            Some(e) => Err(NetworkError::DownloadFailed(e.to_string()).into()),
            None => Err(NetworkError::DownloadFailed("Unknown error".to_string()).into()),
        }
    }

    /// Determine if an error should be retried
    fn should_retry(error: &reqwest::Error) -> bool {
        // Retry on timeout, connection errors, and server errors
        error.is_timeout()
            || error.is_connect()
            || error.status().is_none_or(|s| s.is_server_error())
    }

    /// Get the underlying reqwest client for advanced usage
    #[must_use]
    pub fn inner(&self) -> &Client {
        &self.client
    }
}
