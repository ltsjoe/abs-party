use serde::{Deserialize, Serialize};
use uuid::Uuid;

// Messages from client → server
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Join {
        room_id: Option<String>,
        abs_token: String,
    },
    CreateRoom {
        room_id: String,
        abs_token: String,
        item_id: String,
        item_title: String,
        item_author: String,
        library_id: String,
        /// Set when the room plays one podcast episode; absent for a book.
        #[serde(default)]
        episode_id: Option<String>,
    },
    PlayIntent {
        position: f64,
    },
    Ready,
    Pause {
        position: f64,
    },
    Seek {
        position: f64,
    },
    Speed {
        rate: f64,
    },
    Ping {
        sent_at: u64,
    },
}

// Messages from server → client
#[derive(Debug, Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    RoomState {
        room_id: String,
        item: ItemInfo,
        position: f64,
        playing: bool,
        speed: f64,
        participants: Vec<ParticipantInfo>,
    },
    Play {
        position: f64,
        host_time: u64,
        sender_name: String,
    },
    Pause {
        position: f64,
        sender_name: String,
    },
    Seek {
        position: f64,
        sender_name: String,
    },
    Speed {
        rate: f64,
        sender_name: String,
    },
    PlayIntent {
        position: f64,
        sender_name: String,
    },
    Sync {
        position: f64,
    },
    Pong {
        sent_at: u64,
    },
    ParticipantJoined {
        name: String,
    },
    ParticipantLeft {
        name: String,
    },
    HostChanged {
        name: String,
    },
    Error {
        message: String,
    },
    /// ABS refused the browser's token (it expired, or was deleted). The page
    /// clears it and asks the person to sign in again.
    AuthFailed {
        message: String,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ItemInfo {
    pub id: String,
    pub library_id: String,
    /// The podcast episode this room plays (an ABS episode id), or None for a
    /// book. For an episode, `title` is the episode's and `author` the podcast's.
    #[serde(default)]
    pub episode_id: Option<String>,
    pub title: String,
    pub author: String,
}

/// True for something shaped like an ABS id: a UUID (current ABS) or an old
/// "li_..."/"ep_..." id. A room's ids come from its host and end up in every
/// guest's relay calls (`api/items/<id>/play/<episode id>`), so only plain id
/// characters pass: no `/`, `.` or `%` that could point a guest's own token
/// at some other ABS path.
pub fn is_abs_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[derive(Debug, Serialize, Clone)]
pub struct ParticipantInfo {
    pub name: String,
    pub is_host: bool,
}

// ABS API response types
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AbsLoginResponse {
    pub user: AbsUser,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AbsUser {
    pub id: String,
    pub username: String,
    pub token: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AbsMeResponse {
    pub id: String,
    pub username: String,
}

/// One entry of ABS's admin-only GET /api/users.
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AbsUserSummary {
    pub id: String,
    pub username: String,
    /// "root", "admin", "user" or "guest".
    #[serde(rename = "type")]
    pub user_type: String,
    pub is_active: bool,
}

/// An ABS API key as listed by GET /api/api-keys (the secret itself is never listed).
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AbsApiKey {
    pub id: String,
    pub name: String,
    pub user_id: Option<String>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// The reply to POST /api/api-keys: the only time ABS shows the key's secret.
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AbsApiKeyCreated {
    pub id: String,
    /// The secret, a JWT that works as a Bearer token.
    pub api_key: String,
}

pub type RoomId = String;
pub type ParticipantId = Uuid;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_message_join_deserializes() {
        let json = r#"{"type":"join","room_id":"abc123","abs_token":"tok"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, ClientMessage::Join { room_id: Some(_), .. }));
    }

    #[test]
    fn client_message_create_room_deserializes() {
        let json = r#"{"type":"create_room","room_id":"abcd1234","abs_token":"t","item_id":"i1","item_title":"Book","item_author":"Auth","library_id":"l1"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, ClientMessage::CreateRoom { .. }));
    }

    #[test]
    fn client_message_ping_deserializes() {
        let json = r#"{"type":"ping","sent_at":1234567890}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, ClientMessage::Ping { sent_at: 1234567890 }));
    }

    #[test]
    fn server_message_pong_serializes() {
        let msg = ServerMessage::Pong { sent_at: 42 };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"pong""#));
        assert!(json.contains(r#""sent_at":42"#));
    }

    #[test]
    fn server_message_play_serializes() {
        let msg = ServerMessage::Play { position: 123.4, host_time: 1000, sender_name: "Alice".into() };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"play""#));
        assert!(json.contains(r#""position":123.4"#));
    }

    #[test]
    fn server_message_error_serializes() {
        let msg = ServerMessage::Error { message: "oops".into() };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"error""#));
        assert!(json.contains(r#""message":"oops""#));
    }

    #[test]
    fn server_message_auth_failed_serializes() {
        let msg = ServerMessage::AuthFailed { message: "again".into() };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"auth_failed""#));
    }

    #[test]
    fn item_info_roundtrips() {
        let item = ItemInfo { id: "i".into(), library_id: "l".into(), episode_id: None, title: "T".into(), author: "A".into() };
        let json = serde_json::to_string(&item).unwrap();
        let back: ItemInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "i");
        assert_eq!(back.title, "T");
        assert_eq!(back.episode_id, None);
    }

    #[test]
    fn create_room_without_episode_is_a_book() {
        let json = r#"{"type":"create_room","room_id":"abcd1234","abs_token":"t","item_id":"i1","item_title":"Book","item_author":"Auth","library_id":"l1"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, ClientMessage::CreateRoom { episode_id: None, .. }));
    }

    #[test]
    fn create_room_with_episode_deserializes() {
        let json = r#"{"type":"create_room","room_id":"abcd1234","abs_token":"t","item_id":"i1","item_title":"Ep 1","item_author":"Show","library_id":"l1","episode_id":"ep_1"}"#;
        let msg: ClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            ClientMessage::CreateRoom { episode_id, .. } => assert_eq!(episode_id.as_deref(), Some("ep_1")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn abs_ids_accept_uuids_and_old_ids_only() {
        assert!(is_abs_id("0b1c2d3e-aaaa-4bbb-8ccc-1234567890ab"));
        assert!(is_abs_id("li_8gch9ve09orgn4fdz8"));
        assert!(is_abs_id("ep_lh6ko39pumnrma3dhv"));
        let too_long = "x".repeat(65);
        for bad in ["", "..", "a/b", "a.b", "%2e%2e", "a b", "a?b", too_long.as_str()] {
            assert!(!is_abs_id(bad), "{bad:?} should be refused");
        }
    }
}
