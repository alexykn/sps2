//! Integration tests for net crate

#[cfg(test)]
mod tests {
    use httpmock::prelude::*;
    use spsv2_events::channel;
    use spsv2_hash::Hash;
    use spsv2_net::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_download_file() {
        let server = MockServer::start();
        let (tx, mut rx) = channel();

        // Mock response
        let content = b"test file content";
        let mock = server.mock(|when, then| {
            when.method(GET).path("/test.txt");
            then.status(200)
                .header("content-length", content.len().to_string())
                .body(content);
        });

        // Setup
        let temp = tempdir().unwrap();
        let dest = temp.path().join("downloaded.txt");
        let client = NetClient::default().unwrap();
        let url = server.url("/test.txt");

        // Download
        let result = download_file(&client, &url, &dest, None, &tx)
            .await
            .unwrap();

        // Verify
        mock.assert();
        assert_eq!(result.size, content.len() as u64);
        assert_eq!(result.hash, Hash::hash(content));

        let downloaded = tokio::fs::read(&dest).await.unwrap();
        assert_eq!(downloaded, content);

        // Check events
        let mut saw_start = false;
        let mut saw_complete = false;

        while let Ok(event) = rx.try_recv() {
            match event {
                spsv2_events::Event::DownloadStarted { .. } => saw_start = true,
                spsv2_events::Event::DownloadCompleted { .. } => saw_complete = true,
                _ => {}
            }
        }

        assert!(saw_start);
        assert!(saw_complete);
    }

    #[tokio::test]
    async fn test_download_with_hash_verification() {
        let server = MockServer::start();
        let (tx, _rx) = channel();

        // Mock response
        let content = b"verified content";
        let expected_hash = Hash::hash(content);

        server.mock(|when, then| {
            when.method(GET).path("/verified.txt");
            then.status(200).body(content);
        });

        // Setup
        let temp = tempdir().unwrap();
        let dest = temp.path().join("verified.txt");
        let client = NetClient::default().unwrap();
        let url = server.url("/verified.txt");

        // Download with correct hash
        let result = download_file(&client, &url, &dest, Some(&expected_hash), &tx)
            .await
            .unwrap();
        assert_eq!(result.hash, expected_hash);

        // Download with wrong hash should fail
        let wrong_hash = Hash::hash(b"different content");
        let dest2 = temp.path().join("wrong.txt");
        let error = download_file(&client, &url, &dest2, Some(&wrong_hash), &tx)
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            spsv2_errors::Error::Network(spsv2_errors::NetworkError::ChecksumMismatch { .. })
        ));
    }

    #[tokio::test]
    async fn test_fetch_text() {
        let server = MockServer::start();
        let (tx, _rx) = channel();

        let content = "Hello, world!";
        server.mock(|when, then| {
            when.method(GET).path("/text");
            then.status(200)
                .header("content-type", "text/plain")
                .body(content);
        });

        let client = NetClient::default().unwrap();
        let url = server.url("/text");

        let text = fetch_text(&client, &url, &tx).await.unwrap();
        assert_eq!(text, content);
    }

    #[tokio::test]
    async fn test_http_error_handling() {
        let server = MockServer::start();
        let (tx, _rx) = channel();

        server.mock(|when, then| {
            when.method(GET).path("/404");
            then.status(404).body("Not Found");
        });

        let client = NetClient::default().unwrap();
        let url = server.url("/404");

        let error = fetch_text(&client, &url, &tx).await.unwrap_err();
        assert!(matches!(
            error,
            spsv2_errors::Error::Network(spsv2_errors::NetworkError::HttpError { status: 404, .. })
        ));
    }

    #[tokio::test]
    async fn test_check_url() {
        let server = MockServer::start();

        server.mock(|when, then| {
            when.method(HEAD).path("/exists");
            then.status(200);
        });

        server.mock(|when, then| {
            when.method(HEAD).path("/missing");
            then.status(404);
        });

        let client = NetClient::default().unwrap();

        assert!(check_url(&client, &server.url("/exists")).await.unwrap());
        assert!(!check_url(&client, &server.url("/missing")).await.unwrap());
    }
}
