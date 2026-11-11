use abs_party::{build_app, config::Config};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use tower::ServiceExt;

fn test_config() -> Config {
    Config::for_test()
}

// --- HTTP endpoint tests ---

#[tokio::test]
async fn health_returns_ok() {
    let app = build_app(test_config());
    let resp = app
        .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn config_endpoint_returns_abs_base_url() {
    let mut cfg = test_config();
    cfg.abs_base_url = "https://my.abs.example".into();

    let app = build_app(cfg);
    let resp = app
        .oneshot(Request::builder().uri("/api/config").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    let body = http_body_util::BodyExt::collect(resp.into_body())
        .await
        .unwrap()
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["abs_base_url"], "https://my.abs.example");
}

#[tokio::test]
async fn unknown_route_returns_404_or_fallback() {
    let app = build_app(test_config());
    let resp = app
        .oneshot(Request::builder().uri("/nonexistent-path").body(Body::empty()).unwrap())
        .await
        .unwrap();
    // Static file fallback returns 404 when static dir doesn't exist
    assert!(resp.status().is_client_error() || resp.status().is_success());
}

// --- WebSocket tests ---

async fn spawn_test_server() -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = build_app(test_config());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

#[tokio::test]
async fn websocket_ping_pong() {
    let addr = spawn_test_server().await;
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .expect("WS connect failed");

    ws.send(Message::Text(r#"{"type":"ping","sent_at":99999}"#.into()))
        .await
        .unwrap();

    let msg = ws.next().await.unwrap().unwrap();
    let text = msg.into_text().unwrap();
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["type"], "pong");
    assert_eq!(json["sent_at"], 99999);
}

#[tokio::test]
async fn websocket_invalid_message_returns_error() {
    let addr = spawn_test_server().await;
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .unwrap();

    ws.send(Message::Text(r#"{"type":"not_a_real_type"}"#.into()))
        .await
        .unwrap();

    let msg = ws.next().await.unwrap().unwrap();
    let json: serde_json::Value = serde_json::from_str(&msg.into_text().unwrap()).unwrap();
    assert_eq!(json["type"], "error");
}

#[tokio::test]
async fn websocket_create_and_join_room() {
    let addr = spawn_test_server().await;

    // Host creates a room
    let (mut host_ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .unwrap();

    host_ws
        .send(Message::Text(
            serde_json::json!({
                "type": "create_room",
                "room_id": "abcd1234",
                "abs_token": "alice",
                "item_id": "item1",
                "item_title": "Test Book",
                "item_author": "Test Author",
                "library_id": "lib1"
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();

    let state_msg = host_ws.next().await.unwrap().unwrap();
    let state: serde_json::Value = serde_json::from_str(&state_msg.into_text().unwrap()).unwrap();
    assert_eq!(state["type"], "room_state");
    let room_id = state["room_id"].as_str().unwrap().to_string();
    assert_eq!(room_id, "abcd1234");
    assert_eq!(state["item"]["title"], "Test Book");

    // Guest joins the room
    let (mut guest_ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .unwrap();

    guest_ws
        .send(Message::Text(
            serde_json::json!({
                "type": "join",
                "room_id": room_id,
                "abs_token": "bob"
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();

    // Guest receives room_state
    let guest_msg = guest_ws.next().await.unwrap().unwrap();
    let guest_state: serde_json::Value =
        serde_json::from_str(&guest_msg.into_text().unwrap()).unwrap();
    assert_eq!(guest_state["type"], "room_state");

    // Host receives participant_joined notification
    let host_notif = host_ws.next().await.unwrap().unwrap();
    let notif: serde_json::Value =
        serde_json::from_str(&host_notif.into_text().unwrap()).unwrap();
    assert_eq!(notif["type"], "participant_joined");
    assert_eq!(notif["name"], "bob");
}

#[tokio::test]
async fn websocket_play_intent_and_ready_triggers_play() {
    let addr = spawn_test_server().await;

    let (mut host_ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .unwrap();
    host_ws
        .send(Message::Text(
            serde_json::json!({"type":"create_room","room_id":"abcd1234","abs_token":"alice","item_id":"i","item_title":"B","item_author":"A","library_id":"l"})
            .to_string().into(),
        ))
        .await
        .unwrap();
    let state: serde_json::Value =
        serde_json::from_str(&host_ws.next().await.unwrap().unwrap().into_text().unwrap()).unwrap();
    let room_id = state["room_id"].as_str().unwrap().to_string();

    let (mut guest_ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .unwrap();
    guest_ws
        .send(Message::Text(
            serde_json::json!({"type":"join","room_id":room_id,"abs_token":"bob"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let _ = guest_ws.next().await; // drain room_state
    let _ = host_ws.next().await; // drain participant_joined

    // Host sends play_intent
    host_ws
        .send(Message::Text(
            serde_json::json!({"type":"play_intent","position":75.5}).to_string().into(),
        ))
        .await
        .unwrap();

    // Both host and guest receive play_intent
    let host_intent: serde_json::Value =
        serde_json::from_str(&host_ws.next().await.unwrap().unwrap().into_text().unwrap()).unwrap();
    assert_eq!(host_intent["type"], "play_intent");
    let guest_intent: serde_json::Value =
        serde_json::from_str(&guest_ws.next().await.unwrap().unwrap().into_text().unwrap()).unwrap();
    assert_eq!(guest_intent["type"], "play_intent");
    assert!((guest_intent["position"].as_f64().unwrap() - 75.5).abs() < f64::EPSILON);

    // Both send ready
    host_ws
        .send(Message::Text(serde_json::json!({"type":"ready"}).to_string().into()))
        .await
        .unwrap();
    guest_ws
        .send(Message::Text(serde_json::json!({"type":"ready"}).to_string().into()))
        .await
        .unwrap();

    // Both should receive play
    let host_play: serde_json::Value =
        serde_json::from_str(&host_ws.next().await.unwrap().unwrap().into_text().unwrap()).unwrap();
    assert_eq!(host_play["type"], "play");
    let guest_play: serde_json::Value =
        serde_json::from_str(&guest_ws.next().await.unwrap().unwrap().into_text().unwrap()).unwrap();
    assert_eq!(guest_play["type"], "play");
    assert!((guest_play["position"].as_f64().unwrap() - 75.5).abs() < f64::EPSILON);
}

// --- Podcast episodes ---

#[tokio::test]
async fn websocket_episode_room_tells_guests_the_episode() {
    let addr = spawn_test_server().await;

    let (mut host_ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws")).await.unwrap();
    host_ws
        .send(Message::Text(
            serde_json::json!({
                "type": "create_room",
                "room_id": "pod12345",
                "abs_token": "alice",
                "item_id": "li_podcast1",
                "item_title": "Episode 7",
                "item_author": "Some Show",
                "library_id": "lib_pods",
                "episode_id": "ep_seven"
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    let state: serde_json::Value =
        serde_json::from_str(&host_ws.next().await.unwrap().unwrap().into_text().unwrap()).unwrap();
    assert_eq!(state["type"], "room_state");
    assert_eq!(state["item"]["episode_id"], "ep_seven");

    let (mut guest_ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws")).await.unwrap();
    guest_ws
        .send(Message::Text(
            serde_json::json!({ "type": "join", "room_id": "pod12345", "abs_token": "bob" }).to_string().into(),
        ))
        .await
        .unwrap();
    let guest_state: serde_json::Value =
        serde_json::from_str(&guest_ws.next().await.unwrap().unwrap().into_text().unwrap()).unwrap();
    assert_eq!(guest_state["item"]["id"], "li_podcast1");
    assert_eq!(guest_state["item"]["episode_id"], "ep_seven");
}

#[tokio::test]
async fn websocket_create_room_refuses_ids_that_are_not_plain() {
    let addr = spawn_test_server().await;
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws")).await.unwrap();

    for (item_id, episode_id) in [("../users", None), ("li_1", Some("ep/../../users")), ("li_1", Some("%2e%2e"))] {
        ws.send(Message::Text(
            serde_json::json!({
                "type": "create_room",
                "room_id": "bad12345",
                "abs_token": "alice",
                "item_id": item_id,
                "item_title": "T",
                "item_author": "A",
                "library_id": "lib_1",
                "episode_id": episode_id
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
        let reply: serde_json::Value =
            serde_json::from_str(&ws.next().await.unwrap().unwrap().into_text().unwrap()).unwrap();
        assert_eq!(reply["type"], "error", "{item_id} / {episode_id:?} should be refused");
    }
}

async fn proxy_status(method: &str, uri: &str) -> StatusCode {
    build_app(test_config())
        .oneshot(Request::builder().method(method).uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn proxy_lets_episode_paths_through_to_the_token_check() {
    // No Bearer token: an allowed path stops at 401, a refused one at 403.
    assert_eq!(proxy_status("POST", "/api/abs-proxy/api/items/li_1/play/ep_1").await, StatusCode::UNAUTHORIZED);
    assert_eq!(proxy_status("GET", "/api/abs-proxy/api/libraries/lib_1/recent-episodes?limit=40&page=0").await, StatusCode::UNAUTHORIZED);
    assert_eq!(proxy_status("GET", "/api/abs-proxy/api/me/progress/li_1/ep_1").await, StatusCode::UNAUTHORIZED);
    assert_eq!(proxy_status("POST", "/api/abs-proxy/api/items/li_1/play/ep_1/x").await, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn proxy_refuses_double_encoded_dots() {
    assert_eq!(
        proxy_status("GET", "/api/abs-proxy/api/items/x/%252e%252e/%252e%252e/users").await,
        StatusCode::FORBIDDEN
    );
}

/// A WS server that really checks tokens with an ABS at `abs_base_url`.
async fn spawn_checking_server(abs_base_url: String) -> std::net::SocketAddr {
    let mut cfg = test_config();
    cfg.bypass_auth = false;
    cfg.abs_base_url = abs_base_url;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = build_app(cfg);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

async fn join_reply(addr: std::net::SocketAddr, token: &str) -> serde_json::Value {
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws")).await.unwrap();
    ws.send(Message::Text(
        serde_json::json!({ "type": "join", "room_id": "abc12345", "abs_token": token }).to_string().into(),
    ))
    .await
    .unwrap();
    serde_json::from_str(&ws.next().await.unwrap().unwrap().into_text().unwrap()).unwrap()
}

#[tokio::test]
async fn websocket_tells_a_refused_token_apart_from_abs_being_down() {
    // A stand-in ABS that refuses every token, like one whose key has run out.
    let abs = axum::Router::new().route(
        "/api/me",
        axum::routing::get(|| async { StatusCode::UNAUTHORIZED }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let abs_addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, abs).await.unwrap() });

    let refused = join_reply(spawn_checking_server(format!("http://{abs_addr}")).await, "old-key").await;
    assert_eq!(refused["type"], "auth_failed", "{refused}");

    // Nothing listens on port 9 here: ABS can't be reached, so the token may be fine.
    let down = join_reply(spawn_checking_server("http://127.0.0.1:9".into()).await, "old-key").await;
    assert_eq!(down["type"], "error", "{down}");
    assert!(down["message"].as_str().unwrap().contains("could not be reached"), "{down}");
}
