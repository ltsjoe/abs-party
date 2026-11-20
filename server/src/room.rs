use std::collections::HashSet;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::models::{ItemInfo, ParticipantInfo, ServerMessage};

/// One person's seat in a room. `id` is the WebSocket connection that holds the
/// seat right now: when the same person reconnects, the seat keeps its place
/// and gets the new connection's id and sender.
pub struct Participant {
    pub id: Uuid,
    pub name: String,
    pub abs_token: String,
    pub abs_username: String,
    pub tx: mpsc::UnboundedSender<ServerMessage>,
    /// True while the connection is gone and the seat is held for them to come
    /// back (a phone that switched apps). Away seats get no messages and are
    /// not waited on.
    pub away: bool,
}

pub struct ReadyCheck {
    pub position: f64,
    pub expected: HashSet<Uuid>,
    pub ready: HashSet<Uuid>,
}

pub struct Room {
    pub id: String,
    pub item: ItemInfo,
    pub position: f64,
    pub playing: bool,
    pub speed: f64,
    pub host_id: Uuid,
    pub participants: Vec<Participant>,
    pub play_started_at: Option<Instant>,
    pub ready_check: Option<ReadyCheck>,
    /// ABS username of whoever opened the room. They get host back when they
    /// return after someone else took it over.
    pub owner: String,
    /// Counts the times the room became empty, so a removal timer started for
    /// one empty spell does nothing if people came back in between.
    pub empty_epoch: u64,
    /// Set once the room is taken out of the room map. A join that found the
    /// room just before that sees this and treats the room as gone.
    pub closed: bool,
}

impl Room {
    pub fn new(id: String, item: ItemInfo, host: Participant) -> Self {
        Self {
            id,
            item,
            position: 0.0,
            playing: false,
            speed: 1.0,
            host_id: host.id,
            owner: host.abs_username.clone(),
            participants: vec![host],
            play_started_at: None,
            ready_check: None,
            empty_epoch: 0,
            closed: false,
        }
    }

    pub fn estimated_position(&self) -> f64 {
        match self.play_started_at {
            Some(t) => self.position + t.elapsed().as_secs_f64() * self.speed,
            None => self.position,
        }
    }

    pub fn pause_now(&mut self) {
        self.position = self.estimated_position();
        self.playing = false;
        self.play_started_at = None;
        self.ready_check = None;
    }

    pub fn participant_infos(&self) -> Vec<ParticipantInfo> {
        self.participants
            .iter()
            .map(|p| ParticipantInfo {
                name: p.name.clone(),
                is_host: p.id == self.host_id,
                away: p.away,
            })
            .collect()
    }

    /// Sends to everyone whose connection is up (away seats have none).
    pub fn broadcast(&self, msg: &ServerMessage) {
        for p in self.participants.iter().filter(|p| !p.away) {
            let _ = p.tx.send(msg.clone());
        }
    }

    pub fn broadcast_except(&self, exclude_id: Uuid, msg: &ServerMessage) {
        for p in self.participants.iter().filter(|p| !p.away) {
            if p.id != exclude_id {
                let _ = p.tx.send(msg.clone());
            }
        }
    }

    pub fn send_to(&self, participant_id: Uuid, msg: ServerMessage) {
        if let Some(p) = self.participants.iter().find(|p| p.id == participant_id) {
            let _ = p.tx.send(msg);
        }
    }

    /// True when `id` holds a seat and its connection is up. Messages from any
    /// other connection (one that was replaced, or one whose seat is away) are
    /// ignored by the hub.
    pub fn is_present(&self, id: Uuid) -> bool {
        self.participants.iter().any(|p| p.id == id && !p.away)
    }

    pub fn remove_participant(&mut self, id: Uuid) -> Option<String> {
        if let Some(pos) = self.participants.iter().position(|p| p.id == id) {
            let name = self.participants.remove(pos).name;
            // Transfer host if needed, to someone who is here before someone away
            if self.host_id == id {
                let next = self
                    .participants
                    .iter()
                    .find(|p| !p.away)
                    .or_else(|| self.participants.first());
                if let Some(next) = next {
                    self.host_id = next.id;
                    let new_host_name = next.name.clone();
                    self.broadcast(&ServerMessage::HostChanged {
                        name: new_host_name,
                    });
                }
            }
            Some(name)
        } else {
            None
        }
    }

    pub fn is_empty(&self) -> bool {
        self.participants.is_empty()
    }

    pub fn is_host(&self, id: Uuid) -> bool {
        self.host_id == id
    }

    pub fn state_message(&self) -> ServerMessage {
        ServerMessage::RoomState {
            room_id: self.id.clone(),
            item: self.item.clone(),
            position: self.estimated_position(),
            playing: self.playing,
            speed: self.speed,
            participants: self.participant_infos(),
        }
    }
}

pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
pub(crate) mod test_helpers {
    use super::*;
    use crate::models::ItemInfo;
    use tokio::sync::mpsc;

    pub fn make_item() -> ItemInfo {
        ItemInfo { id: "item1".into(), library_id: "lib1".into(), episode_id: None, title: "Book".into(), author: "Author".into() }
    }

    pub fn make_participant(name: &str) -> (Participant, mpsc::UnboundedReceiver<ServerMessage>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (
            Participant { id: Uuid::new_v4(), name: name.into(), abs_token: "tok".into(), abs_username: name.into(), tx, away: false },
            rx,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::test_helpers::*;
    use crate::models::ServerMessage;

    #[test]
    fn new_room_sets_host() {
        let (host, _rx) = make_participant("Alice");
        let host_id = host.id;
        let room = Room::new("r1".into(), make_item(), host);
        assert!(room.is_host(host_id));
        assert_eq!(room.participants.len(), 1);
    }

    #[test]
    fn participant_infos_marks_host() {
        let (host, _rx) = make_participant("Alice");
        let room = Room::new("r1".into(), make_item(), host);
        let infos = room.participant_infos();
        assert_eq!(infos.len(), 1);
        assert!(infos[0].is_host);
        assert_eq!(infos[0].name, "Alice");
    }

    #[test]
    fn remove_non_host_participant() {
        let (host, _host_rx) = make_participant("Alice");
        let (guest, _guest_rx) = make_participant("Bob");
        let guest_id = guest.id;
        let host_id = host.id;
        let mut room = Room::new("r1".into(), make_item(), host);
        room.participants.push(guest);

        let name = room.remove_participant(guest_id);
        assert_eq!(name, Some("Bob".into()));
        assert!(room.is_host(host_id)); // host unchanged
        assert_eq!(room.participants.len(), 1);
    }

    #[test]
    fn host_removal_transfers_to_next_participant() {
        let (host, _host_rx) = make_participant("Alice");
        let (guest, mut guest_rx) = make_participant("Bob");
        let host_id = host.id;
        let guest_id = guest.id;
        let mut room = Room::new("r1".into(), make_item(), host);
        room.participants.push(guest);

        room.remove_participant(host_id);

        // Bob should now be host
        assert!(room.is_host(guest_id));
        // Bob should have received a HostChanged message
        let msg = guest_rx.try_recv().unwrap();
        assert!(matches!(msg, ServerMessage::HostChanged { name } if name == "Bob"));
    }

    #[test]
    fn host_passes_to_someone_present_before_someone_away() {
        let (host, _host_rx) = make_participant("Alice");
        let (away, _away_rx) = make_participant("Bob");
        let (here, _here_rx) = make_participant("Carol");
        let host_id = host.id;
        let here_id = here.id;
        let mut room = Room::new("r1".into(), make_item(), host);
        room.participants.push(Participant { away: true, ..away });
        room.participants.push(here);

        room.remove_participant(host_id);

        assert!(room.is_host(here_id));
    }

    #[test]
    fn broadcast_skips_away_seats() {
        let (host, mut host_rx) = make_participant("Alice");
        let (away, mut away_rx) = make_participant("Bob");
        let mut room = Room::new("r1".into(), make_item(), host);
        room.participants.push(Participant { away: true, ..away });

        room.broadcast(&ServerMessage::Pong { sent_at: 1 });

        assert!(host_rx.try_recv().is_ok());
        assert!(away_rx.try_recv().is_err());
    }

    #[test]
    fn is_empty_after_all_leave() {
        let (host, _rx) = make_participant("Alice");
        let host_id = host.id;
        let mut room = Room::new("r1".into(), make_item(), host);
        assert!(!room.is_empty());
        room.remove_participant(host_id);
        assert!(room.is_empty());
    }

    #[test]
    fn state_message_reflects_room_state() {
        let (host, _rx) = make_participant("Alice");
        let mut room = Room::new("r1".into(), make_item(), host);
        room.position = 42.0;
        room.playing = true;
        room.speed = 1.5;

        let msg = room.state_message();
        assert!(matches!(msg, ServerMessage::RoomState { position, playing, speed, .. }
            if (position - 42.0).abs() < f64::EPSILON && playing && (speed - 1.5).abs() < f64::EPSILON));
    }
}
