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

// --- Phones that drop their connection (switching apps to text someone) ---
// With bypass_auth the token is the username, so "alice" twice is one person.

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn ws_connect(addr: std::net::SocketAddr) -> Ws {
    tokio_tungstenite::connect_async(format!("ws://{addr}/ws")).await.unwrap().0
}

async fn ws_send(ws: &mut Ws, msg: serde_json::Value) {
    ws.send(Message::Text(msg.to_string())).await.unwrap();
}

/// Next message, or panic after 2 s (a missing message should fail, not hang).
async fn ws_next(ws: &mut Ws) -> serde_json::Value {
    let msg = tokio::time::timeout(std::time::Duration::from_secs(2), ws.next())
        .await
        .expect("timed out waiting for a message")
        .unwrap()
        .unwrap();
    serde_json::from_str(&msg.into_text().unwrap()).unwrap()
}

fn create_msg(token: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "create_room", "room_id": "phone123", "abs_token": token,
        "item_id": "item1", "item_title": "Test Book", "item_author": "Test Author", "library_id": "lib1"
    })
}

fn join_msg(token: &str) -> serde_json::Value {
    serde_json::json!({ "type": "join", "room_id": "phone123", "abs_token": token })
}

#[tokio::test]
async fn websocket_same_person_twice_newest_connection_wins() {
    let addr = spawn_test_server().await;
    let mut host = ws_connect(addr).await;
    ws_send(&mut host, create_msg("alice")).await;
    assert_eq!(ws_next(&mut host).await["type"], "room_state");

    let mut tab1 = ws_connect(addr).await;
    ws_send(&mut tab1, join_msg("bob")).await;
    assert_eq!(ws_next(&mut tab1).await["type"], "room_state");
    assert_eq!(ws_next(&mut host).await["type"], "participant_joined");

    // Bob opens the link again (a second tab, or the page reconnecting before
    // the server noticed the old connection was gone). This used to be "Username taken".
    let mut tab2 = ws_connect(addr).await;
    ws_send(&mut tab2, join_msg("bob")).await;
    let state = ws_next(&mut tab2).await;
    assert_eq!(state["type"], "room_state");
    let names: Vec<&str> = state["participants"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["alice", "bob"]);
    assert_eq!(ws_next(&mut tab1).await["type"], "replaced");

    // The host sees nothing change; the next thing it gets is its own pong.
    ws_send(&mut host, serde_json::json!({"type": "ping", "sent_at": 1})).await;
    assert_eq!(ws_next(&mut host).await["type"], "pong");
}

#[tokio::test]
async fn websocket_room_survives_the_host_switching_apps() {
    let addr = spawn_test_server().await;
    let mut host = ws_connect(addr).await;
    ws_send(&mut host, create_msg("alice")).await;
    assert_eq!(ws_next(&mut host).await["type"], "room_state");

    // The host's phone drops the connection (Chrome went to the background).
    host.close(None).await.unwrap();
    drop(host);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // The guest opens the link meanwhile: the room is there, host shown as away.
    // This used to be "Room not found".
    let mut guest = ws_connect(addr).await;
    ws_send(&mut guest, join_msg("bob")).await;
    let state = ws_next(&mut guest).await;
    assert_eq!(state["type"], "room_state");
    let alice = &state["participants"][0];
    assert_eq!((alice["name"].as_str(), alice["is_host"].as_bool(), alice["away"].as_bool()), (Some("alice"), Some(true), Some(true)));

    // The host's page reconnects and sends create_room again, as Room.vue does.
    let mut host2 = ws_connect(addr).await;
    ws_send(&mut host2, create_msg("alice")).await;
    let back = ws_next(&mut host2).await;
    assert_eq!(back["type"], "room_state");
    assert_eq!(back["participants"].as_array().unwrap().len(), 2);
    assert_eq!(back["participants"][0]["is_host"], true);
    assert_eq!(back["participants"][0]["away"], false);
    let to_guest = ws_next(&mut guest).await;
    assert_eq!((to_guest["type"].as_str(), to_guest["name"].as_str()), (Some("participant_back"), Some("alice")));
}

#[tokio::test]
async fn websocket_leave_closes_an_empty_room_at_once() {
    let addr = spawn_test_server().await;
    let mut host = ws_connect(addr).await;
    ws_send(&mut host, create_msg("alice")).await;
    assert_eq!(ws_next(&mut host).await["type"], "room_state");

    ws_send(&mut host, serde_json::json!({"type": "leave"})).await;
    ws_send(&mut host, serde_json::json!({"type": "ping", "sent_at": 2})).await;
    assert_eq!(ws_next(&mut host).await["type"], "pong"); // leave was handled

    let mut guest = ws_connect(addr).await;
    ws_send(&mut guest, join_msg("bob")).await;
    let reply = ws_next(&mut guest).await;
    assert_eq!(reply["type"], "room_not_found");
}
