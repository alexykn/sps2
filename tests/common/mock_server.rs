//! Simplified mock HTTP server infrastructure for testing download scenarios
//!
//! Provides configurable HTTP servers that can simulate various network
//! conditions and server behaviors for comprehensive download testing.

use httpmock::prelude::*;
use httpmock::Mock;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Configuration for mock server behavior
#[derive(Debug, Clone)]
#[allow(dead_code)] // Test infrastructure - not all fields used yet
pub struct MockServerConfig {
    /// Bandwidth throttling in bytes per second (None = unlimited)
    pub bandwidth_limit: Option<u64>,
    /// Simulate connection timeouts after this duration
    pub timeout_after: Option<Duration>,
    /// Probability of connection interruption (0.0 to 1.0)
    pub interrupt_probability: f64,
    /// Fixed delay before responding
    pub response_delay: Option<Duration>,
    /// Support for resumable downloads via Range requests
    pub supports_resume: bool,
    /// Maximum file size to serve
    pub max_file_size: Option<u64>,
}

impl Default for MockServerConfig {
    fn default() -> Self {
        Self {
            bandwidth_limit: None,
            timeout_after: None,
            interrupt_probability: 0.0,
            response_delay: None,
            supports_resume: true,
            max_file_size: None,
        }
    }
}

/// Mock HTTP server with configurable network conditions
pub struct ConfigurableMockServer {
    server: MockServer,
    config: MockServerConfig,
    file_registry: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl ConfigurableMockServer {
    /// Create a new configurable mock server
    pub fn new(config: MockServerConfig) -> Self {
        let server = MockServer::start();
        Self {
            server,
            config,
            file_registry: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Create a mock server with default configuration
    pub fn with_defaults() -> Self {
        Self::new(MockServerConfig::default())
    }

    /// Register a file to be served by the mock server
    pub fn register_file(&self, path: &str, content: Vec<u8>) {
        let mut registry = self.file_registry.lock().unwrap();
        registry.insert(path.to_string(), content);
    }

    /// Get the base URL of the mock server
    pub fn url(&self, path: &str) -> String {
        self.server.url(path)
    }

    /// Create a mock endpoint for file downloads with realistic behavior
    pub fn mock_file_download(&self, path: &str) -> Mock {
        let content = {
            let registry = self.file_registry.lock().unwrap();
            registry.get(path).cloned().unwrap_or_default()
        };

        let path_owned = path.to_string();

        self.server.mock(|when, then| {
            when.method(GET).path(&path_owned);

            then.status(200)
                .header("content-length", content.len().to_string())
                .header("accept-ranges", "bytes")
                .header("etag", format!("\"{}\"", blake3::hash(&content).to_hex()))
                .body(content);
        })
    }

    /// Create a mock endpoint that supports resumable downloads
    /// Note: This is a simplified implementation due to httpmock 0.7.0 limitations
    /// Real Range request handling would require a stateful server
    #[allow(dead_code)] // Test infrastructure - not used yet
    pub fn mock_resumable_download(&self, path: &str) -> Mock {
        let content = {
            let registry = self.file_registry.lock().unwrap();
            registry.get(path).cloned().unwrap_or_default()
        };

        let path_owned = path.to_string();

        // Due to httpmock 0.7.0 limitations, we can't easily detect Range headers
        // or maintain state between requests. For now, create a mock that simulates
        // a partial content response for any request (assuming it's a resume request)
        self.server.mock(|when, then| {
            when.method(GET).path(&path_owned);

            // Simulate partial content response (as if resuming from middle of file)
            // This is a simplified approach for testing purposes
            let start_pos = content.len() / 2; // Resume from middle
            let partial_content = &content[start_pos..];

            then.status(206) // Partial Content (required for resume validation)
                .header("content-length", partial_content.len().to_string())
                .header(
                    "content-range",
                    format!(
                        "bytes {}-{}/{}",
                        start_pos,
                        content.len() - 1,
                        content.len()
                    ),
                )
                .header("accept-ranges", "bytes")
                .body(partial_content);
        })
    }

    /// Create a mock endpoint with bandwidth throttling (simplified)
    /// Note: This is a placeholder - real throttling is implemented via `ThrottledHttpServer`
    #[allow(dead_code)] // Test infrastructure - not used yet
    pub fn mock_throttled_download(&self, path: &str, _bytes_per_second: u64) -> Mock {
        let content = {
            let registry = self.file_registry.lock().unwrap();
            registry.get(path).cloned().unwrap_or_default()
        };

        let path_owned = path.to_string();

        self.server.mock(|when, then| {
            when.method(GET).path(&path_owned);

            // NOTE: httpmock doesn't support streaming with delays between chunks.
            // For real throttling tests, use `ThrottledHttpServer` instead.
            // This is just a placeholder that adds a fixed delay.
            std::thread::sleep(Duration::from_millis(100));

            then.status(200)
                .header("content-length", content.len().to_string())
                .header("accept-ranges", "bytes")
                .body(content);
        })
    }

    /// Create a mock endpoint that fails a certain number of times before succeeding
    ///
    /// Note: This implementation creates a single mock that always succeeds.
    /// For proper retry testing, use a custom server or test the retry logic
    /// with actual failures in a more controlled manner.
    pub fn mock_failing_download(&self, path: &str, _fail_count: u32) -> Mock {
        let content = {
            let registry = self.file_registry.lock().unwrap();
            registry.get(path).cloned().unwrap_or_default()
        };

        // For now, just create a successful mock
        // TODO: Implement proper stateful behavior when httpmock supports it
        // or when we move to a custom test server
        self.server.mock(|when, then| {
            when.method(GET).path(path);
            then.status(200)
                .header("content-length", content.len().to_string())
                .header("accept-ranges", "bytes")
                .body(content);
        })
    }

    /// Create a mock endpoint for signature files
    #[allow(dead_code)] // Test infrastructure - not used yet
    pub fn mock_signature_download(&self, path: &str, signature_content: &str) -> Mock {
        let path_owned = path.to_string();
        let signature_owned = signature_content.to_string();

        self.server.mock(|when, then| {
            when.method(GET).path(&path_owned);
            then.status(200)
                .header("content-type", "text/plain")
                .header("content-length", signature_owned.len().to_string())
                .body(signature_owned);
        })
    }

    /// Create a mock endpoint that returns 404 Not Found
    #[allow(dead_code)] // Test infrastructure - not used yet
    pub fn mock_not_found(&self, path: &str) -> Mock {
        let path_owned = path.to_string();
        self.server.mock(|when, then| {
            when.method(GET).path(&path_owned);
            then.status(404).body("Not Found");
        })
    }

    /// Get the number of times a path has been requested
    /// Note: This is a simplified implementation. For accurate counting,
    /// tests should use the `Mock.hits()` method on the returned mock directly.
    #[allow(dead_code, clippy::unused_self)]
    pub fn get_request_count(&self, _path: &str) -> u32 {
        // This method is kept for API compatibility but cannot provide
        // accurate counts without storing mock references.
        // Tests should use mock.hits() directly instead.
        0
    }

    /// Reset request counters
    /// Note: This method is kept for API compatibility but has no effect
    /// since request counting is handled by individual Mock objects.
    #[allow(dead_code, clippy::unused_self)]
    pub fn reset_counters(&self) {
        // No-op: httpmock handles request counting per mock
    }

    /// Get server statistics as JSON
    #[allow(dead_code)] // Test infrastructure - not used yet
    pub fn get_stats(&self) -> Value {
        json!({
            "request_counts": {},
            "total_requests": 0,
            "config": {
                "bandwidth_limit": self.config.bandwidth_limit,
                "supports_resume": self.config.supports_resume,
                "interrupt_probability": self.config.interrupt_probability
            }
        })
    }
}

/// Network condition presets for common testing scenarios
#[allow(dead_code)] // Test infrastructure - not used yet
pub struct NetworkConditions;

impl NetworkConditions {
    /// Slow connection (56k modem)
    #[allow(dead_code)] // Test infrastructure - not used yet
    pub fn slow_connection() -> MockServerConfig {
        MockServerConfig {
            bandwidth_limit: Some(7000), // 56k bits/s = ~7KB/s
            response_delay: Some(Duration::from_millis(500)),
            ..Default::default()
        }
    }

    /// Unstable connection with interruptions
    #[allow(dead_code)] // Test infrastructure - not used yet
    pub fn unstable_connection() -> MockServerConfig {
        MockServerConfig {
            interrupt_probability: 0.1, // 10% chance of interruption
            response_delay: Some(Duration::from_millis(100)),
            ..Default::default()
        }
    }

    /// High latency connection
    #[allow(dead_code)] // Test infrastructure - not used yet
    pub fn high_latency() -> MockServerConfig {
        MockServerConfig {
            response_delay: Some(Duration::from_secs(2)),
            ..Default::default()
        }
    }

    /// Server that doesn't support resumable downloads
    #[allow(dead_code)] // Test infrastructure - not used yet
    pub fn no_resume_support() -> MockServerConfig {
        MockServerConfig {
            supports_resume: false,
            ..Default::default()
        }
    }

    /// Server with strict file size limits
    #[allow(dead_code)] // Test infrastructure - not used yet
    pub fn size_limited(max_size: u64) -> MockServerConfig {
        MockServerConfig {
            max_file_size: Some(max_size),
            ..Default::default()
        }
    }
}

/// A simple HTTP server that can throttle bandwidth for testing download speeds
#[allow(dead_code)] // Used in download_tests.rs
pub struct ThrottledHttpServer {
    addr: SocketAddr,
    #[allow(dead_code)] // Kept for potential future use
    content: Arc<Vec<u8>>,
    bytes_per_second: u64,
    _handle: tokio::task::JoinHandle<()>,
}

#[allow(dead_code)] // Used in download_tests.rs, but not all methods are used yet
impl ThrottledHttpServer {
    /// Create a new throttled HTTP server
    ///
    /// # Errors
    ///
    /// Returns an error if the server cannot bind to a port.
    pub async fn new(
        content: Vec<u8>,
        bytes_per_second: u64,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let content = Arc::new(content);
        let content_clone = content.clone();

        let handle = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let content = content_clone.clone();
                let bytes_per_second = bytes_per_second;

                tokio::spawn(async move {
                    let mut buffer = [0; 1024];
                    if stream.read(&mut buffer).await.is_ok() {
                        // Parse request to get path (simple parsing)
                        let _request = String::from_utf8_lossy(&buffer);

                        // Send HTTP response headers
                        let response_headers = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                            content.len()
                        );

                        if stream.write_all(response_headers.as_bytes()).await.is_ok() {
                            // Send content with throttling
                            let _ = Self::send_throttled_content(
                                &mut stream,
                                &content,
                                bytes_per_second,
                            )
                            .await;
                        }
                    }
                });
            }
        });

        Ok(Self {
            addr,
            content,
            bytes_per_second,
            _handle: handle,
        })
    }

    async fn send_throttled_content(
        stream: &mut tokio::net::TcpStream,
        content: &[u8],
        bytes_per_second: u64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let chunk_size = std::cmp::min(8192, bytes_per_second / 10); // Send chunks 10 times per second
        #[allow(clippy::cast_possible_truncation)]
        // chunk_size is limited by bytes_per_second/10 which is reasonable
        let chunk_size = std::cmp::max(1024, chunk_size) as usize; // Minimum 1KB chunks
        let delay_between_chunks =
            Duration::from_millis(1000 * chunk_size as u64 / bytes_per_second);

        let mut sent = 0;
        while sent < content.len() {
            let end = std::cmp::min(sent + chunk_size, content.len());
            let chunk = &content[sent..end];

            stream.write_all(chunk).await?;
            stream.flush().await?;

            sent = end;

            // Don't delay after the last chunk
            if sent < content.len() {
                tokio::time::sleep(delay_between_chunks).await;
            }
        }

        Ok(())
    }

    /// Get the URL for this server
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Get the content being served
    #[allow(dead_code)] // Kept for API completeness
    pub fn content(&self) -> &[u8] {
        &self.content
    }

    /// Get the configured bandwidth limit
    pub fn bytes_per_second(&self) -> u64 {
        self.bytes_per_second
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_server_creation() {
        let server = ConfigurableMockServer::with_defaults();
        assert!(server.url("").starts_with("http://"));
    }

    #[tokio::test]
    async fn test_file_registration() {
        let server = ConfigurableMockServer::with_defaults();
        let test_data = b"Hello, World!".to_vec();

        server.register_file("/test.txt", test_data.clone());

        let mock = server.mock_file_download("/test.txt");

        // Test the actual download
        let client = reqwest::Client::new();
        let response = client.get(server.url("/test.txt")).send().await.unwrap();

        assert_eq!(response.status(), 200);
        let body = response.bytes().await.unwrap();
        assert_eq!(body.as_ref(), test_data.as_slice());

        mock.assert();
    }

    #[tokio::test]
    async fn test_request_counting() {
        let server = ConfigurableMockServer::with_defaults();
        let test_data = b"test data".to_vec();

        server.register_file("/count-test.txt", test_data.clone());

        // Create a mock for the requests
        let mock = server.server.mock(|when, then| {
            when.method(GET).path("/count-test.txt");
            then.status(200).body(test_data);
        });

        let client = reqwest::Client::new();

        // Make multiple requests
        for _ in 0..3 {
            let _response = client
                .get(server.url("/count-test.txt"))
                .send()
                .await
                .unwrap();
        }

        // Use mock.hits() directly for accurate request counting
        assert_eq!(mock.hits(), 3);

        // Use assert_hits instead of expect
        mock.assert_hits(3);
    }

    #[tokio::test]
    async fn test_sequential_failing_download() {
        let server = ConfigurableMockServer::with_defaults();
        let test_data = b"eventual success".to_vec();

        server.register_file("/sequential-test.txt", test_data.clone());

        let client = reqwest::Client::new();

        // Let's test the mock behavior step by step

        // Create just one failing mock first
        let mut fail_mock = server.server.mock(|when, then| {
            when.method(GET).path("/sequential-test.txt");
            then.status(500).body("Internal Server Error");
        });

        // First request should fail
        let response1 = client
            .get(server.url("/sequential-test.txt"))
            .send()
            .await
            .unwrap();
        println!("First request status: {}", response1.status());
        assert_eq!(response1.status(), 500);

        // Now delete the failing mock and create a success mock
        fail_mock.delete();

        let success_mock = server.server.mock(|when, then| {
            when.method(GET).path("/sequential-test.txt");
            then.status(200)
                .header("content-length", test_data.len().to_string())
                .header("accept-ranges", "bytes")
                .body(&test_data);
        });

        // Next request should succeed
        let response2 = client
            .get(server.url("/sequential-test.txt"))
            .send()
            .await
            .unwrap();
        println!("Second request status: {}", response2.status());
        assert_eq!(response2.status(), 200);

        success_mock.assert();
    }

    #[tokio::test]
    async fn test_mock_failing_download_method() {
        let server = ConfigurableMockServer::with_defaults();
        let test_data = b"mock test data".to_vec();

        server.register_file("/mock-failing-test.txt", test_data.clone());

        // Test the mock_failing_download method directly
        let mock = server.mock_failing_download("/mock-failing-test.txt", 2);

        let client = reqwest::Client::new();

        // Make a single request to test the current implementation
        let response = client
            .get(server.url("/mock-failing-test.txt"))
            .send()
            .await
            .unwrap();
        println!("Request status: {}", response.status());
        assert_eq!(response.status(), 200);

        mock.assert();
    }

    #[tokio::test]
    async fn test_failing_download() {
        let server = ConfigurableMockServer::with_defaults();
        let test_data = b"eventual success".to_vec();

        server.register_file("/fail-test.txt", test_data.clone());

        let client = reqwest::Client::new();

        // Test just the failure part - create a mock that always fails
        let mut fail_mock = server.server.mock(|when, then| {
            when.method(GET).path("/fail-test.txt");
            then.status(500).body("Internal Server Error");
        });

        // Make a request that should fail
        let response = client
            .get(server.url("/fail-test.txt"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 500);

        // Verify the fail mock was called
        fail_mock.assert_hits(1);

        // Now test success - delete the fail mock and create a success mock
        fail_mock.delete();

        let success_mock = server.server.mock(|when, then| {
            when.method(GET).path("/fail-test.txt");
            then.status(200)
                .header("content-length", test_data.len().to_string())
                .header("accept-ranges", "bytes")
                .body(test_data.clone());
        });

        // This request should succeed
        let response = client
            .get(server.url("/fail-test.txt"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = response.bytes().await.unwrap();
        assert_eq!(body.as_ref(), test_data.as_slice());

        success_mock.assert_hits(1);
    }
}
