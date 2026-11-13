//! Exercises every asset-listing strategy against a mock Immich server.

use immich_core::{Error, ImmichClient, ShareLink};
use serde_json::json;
use wiremock::matchers::{method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "testkey";

async fn client(server: &MockServer) -> ImmichClient {
    ImmichClient::new(ShareLink::parse(&format!("{}/share/{KEY}", server.uri())).unwrap()).unwrap()
}

fn me_with_album(assets: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "id": "link1", "type": "ALBUM", "allowDownload": true, "assets": [],
        "album": { "id": "album1", "albumName": "Family", "assets": assets }
    }))
}

#[tokio::test]
async fn uses_album_endpoint() {
    let s = MockServer::start().await;
    Mock::given(method("GET")).and(path("/api/shared-links/me")).and(query_param("key", KEY))
        .respond_with(me_with_album(json!([]))).mount(&s).await;
    Mock::given(method("GET")).and(path("/api/albums/album1")).and(query_param("key", KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "album1",
            "assets": [
                { "id": "a", "type": "IMAGE", "originalMimeType": "image/jpeg",
                  "exifInfo": { "exifImageWidth": 1920, "exifImageHeight": 1080 } },
                { "id": "b", "type": "VIDEO" }
            ]
        })))
        .mount(&s).await;

    let assets = client(&s).await.list_assets().await.unwrap();
    assert_eq!(assets.len(), 2);
    assert!(assets[0].is_image && !assets[1].is_image);
}

#[tokio::test]
async fn falls_back_to_columnar_timeline() {
    let s = MockServer::start().await;
    Mock::given(path("/api/shared-links/me")).respond_with(me_with_album(json!([]))).mount(&s).await;
    Mock::given(path("/api/albums/album1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "album1" })))
        .mount(&s).await;
    Mock::given(path("/api/timeline/buckets")).and(query_param("albumId", "album1"))
        .and(query_param_is_missing("size"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "timeBucket": "2025-01-01", "count": 2 }, { "timeBucket": "2024-06-01", "count": 1 }
        ])))
        .mount(&s).await;
    Mock::given(path("/api/timeline/bucket")).and(query_param("timeBucket", "2025-01-01"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": ["t1", "t2"], "isImage": [true, true], "ratio": [1.5, 0.66]
        })))
        .mount(&s).await;
    Mock::given(path("/api/timeline/bucket")).and(query_param("timeBucket", "2024-06-01"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": ["t3"], "isImage": [false], "ratio": [1.77]
        })))
        .mount(&s).await;

    let ids: Vec<String> = client(&s).await.list_assets().await.unwrap().into_iter().map(|a| a.id).collect();
    assert_eq!(ids, ["t1", "t2", "t3"]);
}

#[tokio::test]
async fn old_timeline_requires_size_param() {
    let s = MockServer::start().await;
    Mock::given(path("/api/shared-links/me")).respond_with(me_with_album(json!([]))).mount(&s).await;
    Mock::given(path("/api/albums/album1")).respond_with(ResponseTemplate::new(404)).mount(&s).await;
    Mock::given(path("/api/timeline/buckets")).and(query_param("size", "MONTH"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{ "timeBucket": "2023-01-01T00:00:00.000Z", "count": 1 }])))
        .with_priority(1)
        .mount(&s).await;
    Mock::given(path("/api/timeline/buckets"))
        .respond_with(ResponseTemplate::new(400)).with_priority(2).mount(&s).await;
    Mock::given(path("/api/timeline/bucket")).and(query_param("size", "MONTH"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{ "id": "old1", "type": "IMAGE" }])))
        .mount(&s).await;

    let assets = client(&s).await.list_assets().await.unwrap();
    assert_eq!(assets[0].id, "old1");
}

#[tokio::test]
async fn revoked_link_is_unauthorized() {
    let s = MockServer::start().await;
    Mock::given(path("/api/shared-links/me")).respond_with(ResponseTemplate::new(401)).mount(&s).await;
    assert!(matches!(client(&s).await.list_assets().await, Err(Error::Unauthorized)));
}

#[tokio::test]
async fn download_prefers_original_then_preview() {
    let s = MockServer::start().await;
    Mock::given(path("/api/assets/jpg1/original"))
        .respond_with(ResponseTemplate::new(200).insert_header("content-type", "image/jpeg").set_body_bytes(b"ORIG".to_vec()))
        .mount(&s).await;
    // Downloads disabled on the link -> original is forbidden.
    Mock::given(path("/api/assets/nodl/original")).respond_with(ResponseTemplate::new(403)).mount(&s).await;
    Mock::given(path("/api/assets/nodl/thumbnail")).and(query_param("size", "preview"))
        .respond_with(ResponseTemplate::new(200).insert_header("content-type", "image/jpeg").set_body_bytes(b"PREV".to_vec()))
        .mount(&s).await;
    Mock::given(path("/api/assets/heic/thumbnail")).and(query_param("size", "preview"))
        .respond_with(ResponseTemplate::new(200).insert_header("content-type", "image/jpeg").set_body_bytes(b"HEICPREV".to_vec()))
        .mount(&s).await;

    let c = client(&s).await;
    let mk = |id: &str, mime: Option<&str>| immich_core::Asset {
        id: id.into(), is_image: true, ratio: None, mime: mime.map(String::from),
    };
    let img = c.download(&mk("jpg1", Some("image/jpeg")), true).await.unwrap();
    assert_eq!((img.bytes.as_slice(), img.ext), (&b"ORIG"[..], "jpg"));
    let img = c.download(&mk("nodl", None), true).await.unwrap();
    assert_eq!(img.bytes, b"PREV");
    // HEIC never requests the original.
    let img = c.download(&mk("heic", Some("image/heic")), true).await.unwrap();
    assert_eq!(img.bytes, b"HEICPREV");
}
