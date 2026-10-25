//! The Drive client against a fake Google (wiremock): token refresh, retries,
//! error mapping, change feeds, folder setup and resumable uploads.

use std::time::Duration;

use memoir_core::remote::{Area, ProviderError, StorageProvider};
use memoir_gdrive::{DriveConfig, DriveProvider, Folders, OAuthConfig, RetryPolicy};
use serde_json::json;
use wiremock::matchers::{body_bytes, header, method, path, query_param, query_param_contains};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zeroize::Zeroizing;

fn folders() -> Folders {
    Folders {
        root: "root-id".into(),
        objects: "objects-id".into(),
        blobs: "blobs-id".into(),
    }
}

fn config(server: &MockServer) -> DriveConfig {
    let base = server.uri();
    DriveConfig {
        oauth: OAuthConfig {
            client_id: "client".into(),
            client_secret: "secret".into(),
            auth_url: format!("{base}/auth"),
            token_url: format!("{base}/token"),
            revoke_url: format!("{base}/revoke"),
        },
        api_url: format!("{base}/drive/v3"),
        upload_url: format!("{base}/upload/drive/v3"),
        retry: RetryPolicy {
            attempts: 5,
            base: Duration::from_millis(1),
            max: Duration::from_millis(5),
        },
        resumable_threshold: 10,
        chunk_size: 8,
        root_folder: "Memoir".into(),
    }
}

fn provider(server: &MockServer) -> DriveProvider {
    DriveProvider::new(config(server), Zeroizing::new("refresh-token".into()))
        .with_folders(folders())
}

async fn token_ok(server: &MockServer, token: &str, times: u64) {
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": token, "expires_in": 3600
        })))
        .up_to_n_times(times)
        .mount(server)
        .await;
}

fn file_json(id: &str, name: &str, parent: &str) -> serde_json::Value {
    json!({ "id": id, "name": name, "size": "3", "modifiedTime": "2026-10-25T10:00:00.000Z",
            "mimeType": "application/octet-stream", "parents": [parent] })
}

async fn requests_to(server: &MockServer, p: &str) -> usize {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == p)
        .count()
}

#[tokio::test]
async fn refreshes_the_token_once_on_401() {
    let server = MockServer::start().await;
    token_ok(&server, "t1", 1).await;
    token_ok(&server, "t2", 1).await;
    Mock::given(method("GET"))
        .and(path("/drive/v3/files"))
        .and(header("authorization", "Bearer t1"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/drive/v3/files"))
        .and(header("authorization", "Bearer t2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "files": [file_json("a", "x.mmr", "objects-id")]
        })))
        .mount(&server)
        .await;

    let files = provider(&server).list(Area::Objects).await.unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!((files[0].name.as_str(), files[0].size), ("x.mmr", 3));
    assert_eq!(requests_to(&server, "/token").await, 2);
}

#[tokio::test]
async fn retries_transient_failures_then_succeeds() {
    let server = MockServer::start().await;
    token_ok(&server, "t", 10).await;
    Mock::given(method("GET"))
        .and(path("/drive/v3/files"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/drive/v3/files"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "0"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/drive/v3/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "files": [] })))
        .mount(&server)
        .await;

    assert!(
        provider(&server)
            .list(Area::Blobs)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(requests_to(&server, "/drive/v3/files").await, 3);
}

#[tokio::test]
async fn gives_up_after_the_retry_budget() {
    let server = MockServer::start().await;
    token_ok(&server, "t", 10).await;
    Mock::given(method("GET"))
        .and(path("/drive/v3/files"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let err = provider(&server).list(Area::Root).await.unwrap_err();
    assert_eq!(err, ProviderError::Busy);
    assert!(err.is_transient());
    assert_eq!(requests_to(&server, "/drive/v3/files").await, 5);
}

#[tokio::test]
async fn expired_sign_in_is_an_auth_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": "invalid_grant", "error_description": "Token has been expired or revoked."
        })))
        .mount(&server)
        .await;

    assert_eq!(
        provider(&server).list(Area::Root).await.unwrap_err(),
        ProviderError::Auth
    );
}

#[tokio::test]
async fn full_drive_is_reported_as_quota() {
    let server = MockServer::start().await;
    token_ok(&server, "t", 10).await;
    Mock::given(method("POST"))
        .and(path("/upload/drive/v3/files"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({ "error": {
            "errors": [{ "reason": "storageQuotaExceeded" }],
            "message": "The user's Drive storage quota has been exceeded."
        }})))
        .mount(&server)
        .await;

    let err = provider(&server)
        .upload(Area::Objects, "n.r.mmr", b"abc".to_vec())
        .await
        .unwrap_err();
    assert_eq!(err, ProviderError::Quota);
}

#[tokio::test]
async fn small_files_upload_in_one_request_and_download() {
    let server = MockServer::start().await;
    token_ok(&server, "t", 10).await;
    Mock::given(method("POST"))
        .and(path("/upload/drive/v3/files"))
        .and(query_param("uploadType", "multipart"))
        .respond_with(ResponseTemplate::new(200).set_body_json(file_json(
            "new",
            "n.r.mmr",
            "objects-id",
        )))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/drive/v3/files/new"))
        .and(query_param("alt", "media"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"abc".to_vec()))
        .mount(&server)
        .await;

    let p = provider(&server);
    let file = p
        .upload(Area::Objects, "n.r.mmr", b"abc".to_vec())
        .await
        .unwrap();
    assert_eq!((file.id.as_str(), file.area), ("new", Area::Objects));
    assert_eq!(p.download("new").await.unwrap(), b"abc");
    let sent = &server.received_requests().await.unwrap();
    let upload = sent
        .iter()
        .find(|r| r.url.path() == "/upload/drive/v3/files")
        .unwrap();
    let body = String::from_utf8_lossy(&upload.body);
    assert!(body.contains(r#""parents":["objects-id"]"#) && body.contains("abc"));
}

#[tokio::test]
async fn change_feed_is_paged_and_mapped_to_areas() {
    let server = MockServer::start().await;
    token_ok(&server, "t", 10).await;
    Mock::given(method("GET"))
        .and(path("/drive/v3/changes"))
        .and(query_param("pageToken", "c1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "changes": [
                { "fileId": "a", "file": file_json("a", "a.r1.mmr", "objects-id") },
                { "fileId": "g", "file": file_json("g", "g.mmr", "blobs-id") },
                { "fileId": "dir", "file": { "id": "dir", "name": "objects",
                  "mimeType": "application/vnd.google-apps.folder", "parents": ["root-id"] } },
            ],
            "nextPageToken": "c2"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/drive/v3/changes"))
        .and(query_param("pageToken", "c2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "changes": [
                { "fileId": "b", "removed": true },
                { "fileId": "other", "file": file_json("other", "x", "someone-elses-folder") },
                { "fileId": "a", "file": { "id": "a", "trashed": true, "parents": ["objects-id"] } },
            ],
            "newStartPageToken": "c3"
        })))
        .mount(&server)
        .await;

    let changes = provider(&server).changes("c1").await.unwrap();
    assert_eq!(changes.cursor, "c3");
    assert_eq!(
        changes
            .files
            .iter()
            .map(|f| (f.id.as_str(), f.area))
            .collect::<Vec<_>>(),
        [("g", Area::Blobs)]
    );
    assert_eq!(changes.removed, ["b", "a"]);
}

#[tokio::test]
async fn folders_are_found_or_created() {
    let server = MockServer::start().await;
    token_ok(&server, "t", 10).await;
    let lookup = |name: &str, found: serde_json::Value| {
        Mock::given(method("GET"))
            .and(path("/drive/v3/files"))
            .and(query_param_contains("q", format!("name = '{name}'")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "files": found })))
    };
    lookup("Memoir", json!([{ "id": "root1" }]))
        .mount(&server)
        .await;
    lookup("objects", json!([])).mount(&server).await;
    lookup("blobs", json!([{ "id": "blobs1" }]))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/drive/v3/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "objects1" })))
        .expect(1)
        .mount(&server)
        .await;

    let p = DriveProvider::new(config(&server), Zeroizing::new("r".into()));
    let folders = p.ensure_folders().await.unwrap();
    assert_eq!(
        folders,
        Folders {
            root: "root1".into(),
            objects: "objects1".into(),
            blobs: "blobs1".into()
        }
    );
    let created = server.received_requests().await.unwrap();
    let post = created
        .iter()
        .find(|r| r.method.as_str() == "POST" && r.url.path() == "/drive/v3/files")
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&post.body).unwrap();
    assert_eq!(body["parents"], json!(["root1"]));
}

#[tokio::test]
async fn resumable_upload_continues_where_the_server_stopped() {
    let server = MockServer::start().await;
    token_ok(&server, "t", 10).await;
    let data: Vec<u8> = (0u8..20).collect();
    let session = format!("{}/session1", server.uri());

    Mock::given(method("POST"))
        .and(path("/upload/drive/v3/files"))
        .and(query_param("uploadType", "resumable"))
        .respond_with(ResponseTemplate::new(200).insert_header("location", session.as_str()))
        .mount(&server)
        .await;
    // First chunk arrives.
    Mock::given(method("PUT"))
        .and(path("/session1"))
        .and(header("content-range", "bytes 0-7/20"))
        .respond_with(ResponseTemplate::new(308).insert_header("range", "bytes=0-7"))
        .mount(&server)
        .await;
    // Second chunk fails half-way…
    Mock::given(method("PUT"))
        .and(path("/session1"))
        .and(header("content-range", "bytes 8-15/20"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    // …the status check says 12 bytes arrived…
    Mock::given(method("PUT"))
        .and(path("/session1"))
        .and(header("content-range", "bytes */20"))
        .respond_with(ResponseTemplate::new(308).insert_header("range", "bytes=0-11"))
        .mount(&server)
        .await;
    // …so the client continues from byte 12, sending exactly the rest.
    Mock::given(method("PUT"))
        .and(path("/session1"))
        .and(header("content-range", "bytes 12-19/20"))
        .and(body_bytes(data[12..].to_vec()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "big", "name": "big.mmr", "size": "20", "modifiedTime": "2026-10-25T10:00:00Z"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let file = provider(&server)
        .upload(Area::Blobs, "big.mmr", data)
        .await
        .unwrap();
    assert_eq!(
        (file.id.as_str(), file.size, file.area),
        ("big", 20, Area::Blobs)
    );
}
