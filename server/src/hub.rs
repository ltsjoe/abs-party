use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;
use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::config::Holds;
use crate::models::{ItemInfo, ServerMessage};
use crate::room::{now_millis, Participant, ReadyCheck, Room};

// Rooms live in memory only. Phones drop their WebSocket whenever the browser
// goes to the background (switching to Messages to text the others), so a
// dropped connection does not mean the person left: their seat is held for
// `Holds::seat` and they get it back when the page reconnects. A room nobody
// is in is kept for `Holds::room`. Only Leave gives a seat up at once.

pub type RoomMap = Arc<DashMap<String, Arc<Mutex<Room>>>>;

pub const ROOM_NOT_FOUND: &str = "Room not found — the host hasn't opened the room yet";

const REPLACED: &str = "You opened this room on another page or device, so this page left it.";

pub fn new_room_map() -> RoomMap {
    Arc::new(DashMap::new())
}

fn validate_room_id(id: &str) -> Result<(), String> {
    if id.len() != 8 {
        return Err("Room ID must be exactly 8 characters".into());
    }
    if !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("Room ID may only contain letters and digits".into());
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub enum JoinError {
    /// No open room with that id (never opened, or closed after standing empty).
    NotFound,
    /// The room id itself is malformed.
    Invalid(String),
}

/// Create a new room with the given ID, or join the existing room if the ID is already in use.
/// Returns Ok(true) if a new room was created, Ok(false) if joined an existing room.
pub async fn create_room(
    rooms: &RoomMap,
    room_id: String,
    item: ItemInfo,
    host: Participant,
) -> Result<bool, String> {
    validate_room_id(&room_id)?;
    loop {
        let arc = match rooms.entry(room_id.clone()) {
            Entry::Vacant(v) => {
                v.insert(Arc::new(Mutex::new(Room::new(room_id, item, host))));
                return Ok(true);
            }
            Entry::Occupied(o) => o.get().clone(),
        };
        let mut room = arc.lock().await;
        if room.closed {
            // Closed between the lookup and the lock. It is out of the map
            // already; go round again and open a fresh room under this id.
            drop(room);
            rooms.remove_if(&room_id, |_, v| Arc::ptr_eq(v, &arc));
            continue;
        }
        seat(&mut room, host);
        return Ok(false);
    }
}

pub async fn join_room(
    rooms: &RoomMap,
    room_id: &str,
    participant: Participant,
) -> Result<(), JoinError> {
    validate_room_id(room_id).map_err(JoinError::Invalid)?;
    let arc = rooms.get(room_id).map(|r| r.clone()).ok_or(JoinError::NotFound)?;
    let mut room = arc.lock().await;
    if room.closed {
        return Err(JoinError::NotFound);
    }
    seat(&mut room, participant);
    Ok(())
}

/// Puts `p` in the room. A seat's name is the ABS username, which ABS vouched
/// for when this connection joined, so a seat with the same username is this
/// same person: they get it back. Anyone else is a newcomer.
fn seat(room: &mut Room, p: Participant) {
    match room.participants.iter().position(|s| s.abs_username == p.abs_username) {
        Some(idx) => retake_seat(room, idx, p),
        None => add_newcomer(room, p),
    }
}

/// The same person again: back from a dropped connection, or a newer page or
/// device. The new connection takes the seat (place in the list, host role)
/// and an older connection still on it is told it was replaced. Playback is
/// not paused for the others; the returning page catches up from room_state.
fn retake_seat(room: &mut Room, idx: usize, p: Participant) {
    let (old_id, was_away, name) = {
        let s = &room.participants[idx];
        (s.id, s.away, s.name.clone())
    };
    let new_id = p.id;
    if old_id != new_id && !was_away {
        let _ = room.participants[idx].tx.send(ServerMessage::Replaced { message: REPLACED.into() });
        forget_in_ready_check(room, old_id);
    }
    {
        let s = &mut room.participants[idx];
        s.id = new_id;
        s.tx = p.tx;
        s.abs_token = p.abs_token;
        s.away = false;
    }
    if room.host_id == old_id {
        room.host_id = new_id;
    }
    let host_back = give_owner_host_back(room, new_id, &p.abs_username);
    let state = room.state_message();
    room.send_to(new_id, state);
    if was_away {
        room.broadcast_except(new_id, &ServerMessage::ParticipantBack { name: name.clone() });
    }
    if host_back {
        room.broadcast_except(new_id, &ServerMessage::HostChanged { name });
    }
}

fn add_newcomer(room: &mut Room, p: Participant) {
    let name = p.name.clone();
    let id = p.id;
    let username = p.abs_username.clone();

    // Auto-pause on join so all parties re-sync before resuming
    if room.playing {
        let pos = room.estimated_position();
        room.pause_now();
        room.broadcast(&ServerMessage::Pause {
            position: pos,
            sender_name: format!("{} joined", name),
        });
    } else if let Some(check) = room.ready_check.take() {
        // Cancel any pending ready-check; broadcast Pause so clients clear buffering state
        room.broadcast(&ServerMessage::Pause {
            position: check.position,
            sender_name: format!("{} joined", name),
        });
    }

    // Someone walking into a room that stood empty runs it until its owner is back.
    let first_in = room.is_empty();
    // Push the newcomer first so room_state includes them in the participant list
    room.participants.push(p);
    if first_in {
        room.host_id = id;
    }
    let host_back = give_owner_host_back(room, id, &username);

    let state = room.state_message();
    room.send_to(id, state);
    // Joined before HostChanged, so the others' lists have the name to mark.
    room.broadcast_except(id, &ServerMessage::ParticipantJoined { name: name.clone() });
    if host_back {
        room.broadcast_except(id, &ServerMessage::HostChanged { name });
    }
}

/// Whoever opened the room is its host again when they come back after the
/// role passed to someone else. True when the host changed (the caller
/// tells the others, after they know this person is in the room).
fn give_owner_host_back(room: &mut Room, id: Uuid, username: &str) -> bool {
    if username == room.owner && room.host_id != id {
        room.host_id = id;
        true
    } else {
        false
    }
}

/// Takes `id` off a pending play's wait list. If everyone still on it is
/// ready, the play starts; if nobody is left on it, it is dropped.
fn forget_in_ready_check(room: &mut Room, id: Uuid) {
    let Some(check) = room.ready_check.as_mut() else { return };
    check.expected.remove(&id);
    check.ready.remove(&id);
    if check.expected.is_empty() {
        room.ready_check = None;
    } else if check.expected.is_subset(&check.ready) {
        start_play(room);
    }
}

fn start_play(room: &mut Room) {
    let Some(check) = room.ready_check.take() else { return };
    room.playing = true;
    room.position = check.position;
    room.play_started_at = Some(Instant::now());
    room.broadcast(&ServerMessage::Play {
        position: check.position,
        host_time: now_millis(),
        sender_name: String::new(),
    });
}

pub async fn handle_play_intent(rooms: &RoomMap, room_id: &str, sender_id: Uuid, position: f64) {
    if !position.is_finite() || position < 0.0 { return; }
    if let Some(arc) = rooms.get(room_id).map(|r| r.clone()) {
        let mut room = arc.lock().await;
        if !room.is_present(sender_id) { return; }
        let sender_name = room.participants.iter()
            .find(|p| p.id == sender_id)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        room.pause_now();
        room.position = position;
        // Wait only for people who are here: an away seat can't buffer.
        let expected: HashSet<Uuid> = room.participants.iter().filter(|p| !p.away).map(|p| p.id).collect();
        room.ready_check = Some(ReadyCheck { position, expected, ready: HashSet::new() });
        // Broadcast to ALL participants (including sender) so everyone starts buffering
        room.broadcast(&ServerMessage::PlayIntent { position, sender_name });
    }
}

pub async fn handle_ready(rooms: &RoomMap, room_id: &str, sender_id: Uuid) {
    if let Some(arc) = rooms.get(room_id).map(|r| r.clone()) {
        let mut room = arc.lock().await;
        if !room.is_present(sender_id) { return; }
        let fire_play = match room.ready_check.as_mut() {
            Some(check) => {
                check.ready.insert(sender_id);
                check.expected.is_subset(&check.ready)
            }
            None => false,
        };
        if fire_play {
            start_play(&mut room);
        }
    }
}

pub async fn handle_pause(rooms: &RoomMap, room_id: &str, sender_id: Uuid, position: f64) {
    if !position.is_finite() || position < 0.0 { return; }
    if let Some(arc) = rooms.get(room_id).map(|r| r.clone()) {
        let mut room = arc.lock().await;
        if !room.is_present(sender_id) { return; }
        let sender_name = room.participants.iter()
            .find(|p| p.id == sender_id)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        room.position = position;
        room.playing = false;
        room.play_started_at = None;
        room.ready_check = None;
        let msg = ServerMessage::Pause { position, sender_name };
        room.broadcast_except(sender_id, &msg);
    }
}

pub async fn handle_seek(rooms: &RoomMap, room_id: &str, sender_id: Uuid, position: f64) {
    if !position.is_finite() || position < 0.0 { return; }
    if let Some(arc) = rooms.get(room_id).map(|r| r.clone()) {
        let mut room = arc.lock().await;
        if !room.is_present(sender_id) { return; }
        let sender_name = room.participants.iter()
            .find(|p| p.id == sender_id)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        room.position = position;
        // Keep the clock running if we're playing, otherwise estimated_position freezes here
        room.play_started_at = if room.playing { Some(Instant::now()) } else { None };
        room.ready_check = None;
        let msg = ServerMessage::Seek { position, sender_name };
        room.broadcast_except(sender_id, &msg);
    }
}

pub async fn handle_speed(rooms: &RoomMap, room_id: &str, sender_id: Uuid, rate: f64) {
    if !rate.is_finite() || !(0.25_f64..=4.0_f64).contains(&rate) { return; }
    if let Some(arc) = rooms.get(room_id).map(|r| r.clone()) {
        let mut room = arc.lock().await;
        if !room.is_present(sender_id) || !room.is_host(sender_id) {
            return;
        }
        let sender_name = room.participants.iter()
            .find(|p| p.id == sender_id)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        // Keep estimated_position right across the change of rate
        if room.playing {
            room.position = room.estimated_position();
            room.play_started_at = Some(Instant::now());
        }
        room.speed = rate;
        let msg = ServerMessage::Speed { rate, sender_name };
        room.broadcast_except(sender_id, &msg);
    }
}

/// The person pressed Leave: their seat goes now. If they were the last one
/// in, the room closes at once too.
pub async fn leave_room(rooms: &RoomMap, room_id: &str, participant_id: Uuid, holds: Holds) {
    remove_seat(rooms, room_id, participant_id, holds, true).await;
}

/// The connection closed without a Leave: a phone switching apps or losing
/// signal, a closed tab, a reload. The seat is held for `holds.seat` so the
/// person can come back into it; after that it is given up.
pub async fn disconnect(rooms: &RoomMap, room_id: &str, conn_id: Uuid, holds: Holds) {
    if holds.seat.is_zero() {
        remove_seat(rooms, room_id, conn_id, holds, false).await;
        return;
    }
    let Some(arc) = rooms.get(room_id).map(|r| r.clone()) else { return };
    {
        let mut room = arc.lock().await;
        if room.closed { return; }
        // No seat with this id: the person is already back on a newer connection.
        let Some(seat) = room.participants.iter_mut().find(|p| p.id == conn_id && !p.away) else { return };
        seat.away = true;
        let name = seat.name.clone();
        forget_in_ready_check(&mut room, conn_id);
        room.broadcast(&ServerMessage::ParticipantAway { name });
    }
    let rooms = rooms.clone();
    tokio::spawn(async move {
        tokio::time::sleep(holds.seat).await;
        let mut room = arc.lock().await;
        // Coming back gives the seat the new connection's id, so this finds
        // nothing unless they stayed away the whole time.
        if room.closed || !room.participants.iter().any(|p| p.id == conn_id && p.away) {
            return;
        }
        if let Some(name) = room.remove_participant(conn_id) {
            // No pause: the others have been listening on without them.
            room.broadcast(&ServerMessage::ParticipantLeft { name });
        }
        if room.is_empty() {
            room_emptied(&rooms, &arc, &mut room, holds, false);
        }
    });
}

async fn remove_seat(rooms: &RoomMap, room_id: &str, participant_id: Uuid, holds: Holds, close_now: bool) {
    let Some(arc) = rooms.get(room_id).map(|r| r.clone()) else { return };
    let mut room = arc.lock().await;
    if room.closed { return; }
    let Some(name) = room.remove_participant(participant_id) else { return };
    room.broadcast(&ServerMessage::ParticipantLeft { name: name.clone() });

    if room.is_empty() {
        room_emptied(rooms, &arc, &mut room, holds, close_now);
    } else if room.playing {
        let pos = room.estimated_position();
        room.pause_now();
        room.broadcast(&ServerMessage::Pause {
            position: pos,
            sender_name: format!("{} left", name),
        });
    } else {
        forget_in_ready_check(&mut room, participant_id);
    }
}

/// The last seat is gone. Stop the room's clock, then close the room: at once
/// after a Leave (or with no room hold), otherwise after `holds.room` unless
/// someone came in meanwhile.
fn room_emptied(rooms: &RoomMap, arc: &Arc<Mutex<Room>>, room: &mut Room, holds: Holds, close_now: bool) {
    room.pause_now();
    room.empty_epoch += 1;
    if close_now || holds.room.is_zero() {
        close_room(rooms, arc, room);
        return;
    }
    let epoch = room.empty_epoch;
    let (rooms, arc) = (rooms.clone(), arc.clone());
    tokio::spawn(async move {
        tokio::time::sleep(holds.room).await;
        let mut room = arc.lock().await;
        if !room.closed && room.is_empty() && room.empty_epoch == epoch {
            close_room(&rooms, &arc, &mut room);
        }
    });
}

/// Takes the room out of the map, under its lock. `closed` tells any join that
/// found the room just before this to treat it as gone.
fn close_room(rooms: &RoomMap, arc: &Arc<Mutex<Room>>, room: &mut Room) {
    room.closed = true;
    rooms.remove_if(&room.id, |_, v| Arc::ptr_eq(v, arc));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ServerMessage;
    use crate::room::test_helpers::{make_item, make_participant};
    use std::time::Duration;
    use tokio::sync::mpsc::UnboundedReceiver;

    const HOLDS: Holds = Holds { seat: Duration::from_secs(120), room: Duration::from_secs(1800) };

    fn drain(rx: &mut UnboundedReceiver<ServerMessage>) -> Vec<ServerMessage> {
        let mut out = Vec::new();
        while let Ok(m) = rx.try_recv() {
            out.push(m);
        }
        out
    }

    async fn room_of(rooms: &RoomMap, id: &str) -> Arc<Mutex<Room>> {
        rooms.get(id).map(|r| r.clone()).expect("room should exist")
    }

    /// Alice opens "testroom" and Bob joins; both inboxes start empty.
    async fn alice_and_bob(rooms: &RoomMap) -> (Uuid, UnboundedReceiver<ServerMessage>, Uuid, UnboundedReceiver<ServerMessage>) {
        let (alice, mut alice_rx) = make_participant("Alice");
        let alice_id = alice.id;
        create_room(rooms, "testroom".into(), make_item(), alice).await.unwrap();
        let (bob, mut bob_rx) = make_participant("Bob");
        let bob_id = bob.id;
        join_room(rooms, "testroom", bob).await.unwrap();
        drain(&mut alice_rx);
        drain(&mut bob_rx);
        (alice_id, alice_rx, bob_id, bob_rx)
    }

    async fn start_playing(rooms: &RoomMap, ids: &[Uuid]) {
        handle_play_intent(rooms, "testroom", ids[0], 10.0).await;
        for id in ids {
            handle_ready(rooms, "testroom", *id).await;
        }
    }

    // --- Held seats: a dropped connection is someone coming right back ---

    #[tokio::test(start_paused = true)]
    async fn dropped_seat_is_held_and_others_hear_it_is_away() {
        let rooms = new_room_map();
        let (_alice_id, mut alice_rx, bob_id, _bob_rx) = alice_and_bob(&rooms).await;

        disconnect(&rooms, "testroom", bob_id, HOLDS).await;

        let msgs = drain(&mut alice_rx);
        assert!(matches!(&msgs[..], [ServerMessage::ParticipantAway { name }] if name == "Bob"), "{msgs:?}");
        let arc = room_of(&rooms, "testroom").await;
        let room = arc.lock().await;
        assert_eq!(room.participants.len(), 2);
        assert!(room.participants.iter().any(|p| p.name == "Bob" && p.away));
    }

    #[tokio::test(start_paused = true)]
    async fn coming_back_retakes_the_seat_without_pausing_the_others() {
        let rooms = new_room_map();
        let (alice_id, mut alice_rx, bob_id, mut bob_rx) = alice_and_bob(&rooms).await;
        start_playing(&rooms, &[alice_id, bob_id]).await;
        drain(&mut alice_rx);
        drain(&mut bob_rx);

        // Alice (the host) switches apps, then her page reconnects.
        disconnect(&rooms, "testroom", alice_id, HOLDS).await;
        tokio::time::sleep(Duration::from_secs(30)).await;
        let (alice2, mut alice2_rx) = make_participant("Alice");
        let alice2_id = alice2.id;
        join_room(&rooms, "testroom", alice2).await.unwrap();

        let to_bob = drain(&mut bob_rx);
        assert!(matches!(&to_bob[..], [ServerMessage::ParticipantAway { .. }, ServerMessage::ParticipantBack { name }] if name == "Alice"), "{to_bob:?}");
        match alice2_rx.try_recv().unwrap() {
            ServerMessage::RoomState { playing, participants, .. } => {
                assert!(playing, "the room kept playing");
                assert_eq!(participants.len(), 2, "no second Alice");
                assert!(participants.iter().any(|p| p.name == "Alice" && p.is_host && !p.away));
            }
            other => panic!("expected room_state, got {other:?}"),
        }
        let arc = room_of(&rooms, "testroom").await;
        let room = arc.lock().await;
        assert!(room.is_host(alice2_id));
        assert!(room.playing);
        // Her seat kept its place at the top of the list.
        assert_eq!(room.participants[0].id, alice2_id);
    }

    #[tokio::test(start_paused = true)]
    async fn a_newer_connection_replaces_a_live_one() {
        let rooms = new_room_map();
        let (_alice_id, mut alice_rx, bob_id, mut bob_rx) = alice_and_bob(&rooms).await;

        // Bob opens the link again in another tab while the first is still connected.
        let (bob2, mut bob2_rx) = make_participant("Bob");
        let bob2_id = bob2.id;
        join_room(&rooms, "testroom", bob2).await.unwrap();

        assert!(matches!(drain(&mut bob_rx)[..], [ServerMessage::Replaced { .. }]));
        assert!(matches!(bob2_rx.try_recv().unwrap(), ServerMessage::RoomState { .. }));
        assert!(drain(&mut alice_rx).is_empty(), "Alice sees nothing change");

        // The replaced tab can no longer steer the room, and its close changes nothing.
        handle_pause(&rooms, "testroom", bob_id, 99.0).await;
        disconnect(&rooms, "testroom", bob_id, HOLDS).await;
        assert!(drain(&mut alice_rx).is_empty());
        let arc = room_of(&rooms, "testroom").await;
        let room = arc.lock().await;
        assert_eq!(room.participants.iter().filter(|p| p.name == "Bob").count(), 1);
        assert!(room.is_present(bob2_id));
    }

    #[tokio::test(start_paused = true)]
    async fn held_seat_is_given_up_after_the_hold_without_pausing() {
        let rooms = new_room_map();
        let (alice_id, mut alice_rx, bob_id, mut bob_rx) = alice_and_bob(&rooms).await;
        start_playing(&rooms, &[alice_id, bob_id]).await;
        drain(&mut alice_rx);
        drain(&mut bob_rx);

        disconnect(&rooms, "testroom", alice_id, HOLDS).await;
        tokio::time::sleep(Duration::from_secs(119)).await;
        assert!(matches!(drain(&mut bob_rx)[..], [ServerMessage::ParticipantAway { .. }]));
        tokio::time::sleep(Duration::from_secs(2)).await;

        let msgs = drain(&mut bob_rx);
        assert!(matches!(&msgs[..], [ServerMessage::HostChanged { name }, ServerMessage::ParticipantLeft { .. }] if name == "Bob"), "{msgs:?}");
        let arc = room_of(&rooms, "testroom").await;
        let room = arc.lock().await;
        assert_eq!(room.participants.len(), 1);
        assert!(room.playing, "Bob kept listening");
    }

    #[tokio::test(start_paused = true)]
    async fn coming_back_in_time_cancels_the_give_up() {
        let rooms = new_room_map();
        let (alice_id, _alice_rx, _bob_id, mut bob_rx) = alice_and_bob(&rooms).await;

        disconnect(&rooms, "testroom", alice_id, HOLDS).await;
        tokio::time::sleep(Duration::from_secs(60)).await;
        let (alice2, _alice2_rx) = make_participant("Alice");
        join_room(&rooms, "testroom", alice2).await.unwrap();
        tokio::time::sleep(Duration::from_secs(300)).await;

        let msgs = drain(&mut bob_rx);
        assert!(!msgs.iter().any(|m| matches!(m, ServerMessage::ParticipantLeft { .. })), "{msgs:?}");
        assert_eq!(room_of(&rooms, "testroom").await.lock().await.participants.len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn an_empty_room_is_kept_for_the_room_hold_then_closed() {
        let rooms = new_room_map();
        let (alice, _rx) = make_participant("Alice");
        let alice_id = alice.id;
        create_room(&rooms, "testroom".into(), make_item(), alice).await.unwrap();
        start_playing(&rooms, &[alice_id]).await;

        // The only listener switches apps and stays away.
        disconnect(&rooms, "testroom", alice_id, HOLDS).await;
        tokio::time::sleep(Duration::from_secs(121)).await;
        {
            let arc = room_of(&rooms, "testroom").await;
            let room = arc.lock().await;
            assert!(room.is_empty());
            assert!(!room.playing, "an empty room's clock stops");
        }
        tokio::time::sleep(Duration::from_secs(1800 - 2)).await;
        assert!(rooms.contains_key("testroom"));
        tokio::time::sleep(Duration::from_secs(3)).await;
        assert!(!rooms.contains_key("testroom"));
    }

    #[tokio::test(start_paused = true)]
    async fn walking_into_an_empty_room_makes_you_host_until_the_owner_returns() {
        let rooms = new_room_map();
        let (alice, _rx) = make_participant("Alice");
        let alice_id = alice.id;
        create_room(&rooms, "testroom".into(), make_item(), alice).await.unwrap();
        disconnect(&rooms, "testroom", alice_id, HOLDS).await;
        tokio::time::sleep(Duration::from_secs(121)).await;

        // Bob opens the link while the room stands empty: it is still there.
        let (bob, mut bob_rx) = make_participant("Bob");
        let bob_id = bob.id;
        join_room(&rooms, "testroom", bob).await.unwrap();
        assert!(room_of(&rooms, "testroom").await.lock().await.is_host(bob_id));
        drain(&mut bob_rx);

        // Alice comes back (her held seat is long gone) and gets host back.
        let (alice2, _rx2) = make_participant("Alice");
        let alice2_id = alice2.id;
        create_room(&rooms, "testroom".into(), make_item(), alice2).await.unwrap();
        assert!(room_of(&rooms, "testroom").await.lock().await.is_host(alice2_id));
        let msgs = drain(&mut bob_rx);
        assert!(
            matches!(&msgs[..], [ServerMessage::ParticipantJoined { name: j }, ServerMessage::HostChanged { name: h }] if j == "Alice" && h == "Alice"),
            "{msgs:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_room_someone_came_back_to_is_not_closed_by_the_old_timer() {
        let rooms = new_room_map();
        let (alice, _rx) = make_participant("Alice");
        let alice_id = alice.id;
        create_room(&rooms, "testroom".into(), make_item(), alice).await.unwrap();
        disconnect(&rooms, "testroom", alice_id, HOLDS).await;
        tokio::time::sleep(Duration::from_secs(600)).await; // empty since 120 s

        let (alice2, _rx2) = make_participant("Alice");
        join_room(&rooms, "testroom", alice2).await.unwrap();
        tokio::time::sleep(Duration::from_secs(3600)).await;
        assert!(rooms.contains_key("testroom"));
    }

    #[tokio::test(start_paused = true)]
    async fn play_does_not_wait_for_an_away_seat() {
        let rooms = new_room_map();
        let (alice_id, mut alice_rx, bob_id, _bob_rx) = alice_and_bob(&rooms).await;
        disconnect(&rooms, "testroom", bob_id, HOLDS).await;
        drain(&mut alice_rx);

        handle_play_intent(&rooms, "testroom", alice_id, 5.0).await;
        handle_ready(&rooms, "testroom", alice_id).await;

        let msgs = drain(&mut alice_rx);
        assert!(matches!(&msgs[..], [ServerMessage::PlayIntent { .. }, ServerMessage::Play { .. }]), "{msgs:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn a_drop_during_a_ready_check_lets_the_rest_start() {
        let rooms = new_room_map();
        let (alice_id, mut alice_rx, bob_id, _bob_rx) = alice_and_bob(&rooms).await;
        handle_play_intent(&rooms, "testroom", alice_id, 5.0).await;
        handle_ready(&rooms, "testroom", alice_id).await;
        drain(&mut alice_rx);

        disconnect(&rooms, "testroom", bob_id, HOLDS).await;

        let msgs = drain(&mut alice_rx);
        assert!(msgs.iter().any(|m| matches!(m, ServerMessage::Play { .. })), "{msgs:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn an_away_seat_cannot_steer_the_room() {
        let rooms = new_room_map();
        let (_alice_id, mut alice_rx, bob_id, _bob_rx) = alice_and_bob(&rooms).await;
        disconnect(&rooms, "testroom", bob_id, HOLDS).await;
        drain(&mut alice_rx);

        handle_seek(&rooms, "testroom", bob_id, 500.0).await;
        handle_play_intent(&rooms, "testroom", bob_id, 500.0).await;

        assert!(drain(&mut alice_rx).is_empty());
        assert_eq!(room_of(&rooms, "testroom").await.lock().await.position, 0.0);
    }

    #[tokio::test(start_paused = true)]
    async fn zero_holds_drop_seats_and_rooms_at_once_as_before() {
        let rooms = new_room_map();
        let none = Holds { seat: Duration::ZERO, room: Duration::ZERO };
        let (alice_id, _alice_rx, bob_id, mut bob_rx) = alice_and_bob(&rooms).await;

        disconnect(&rooms, "testroom", alice_id, none).await;
        let msgs = drain(&mut bob_rx);
        assert!(msgs.iter().any(|m| matches!(m, ServerMessage::ParticipantLeft { name } if name == "Alice")), "{msgs:?}");
        disconnect(&rooms, "testroom", bob_id, none).await;
        assert!(!rooms.contains_key("testroom"));
    }

    #[tokio::test]
    async fn a_closed_room_counts_as_gone() {
        let rooms = new_room_map();
        let (alice, _rx) = make_participant("Alice");
        create_room(&rooms, "testroom".into(), make_item(), alice).await.unwrap();
        // As if a join found the room just before it was closed and left in the map.
        room_of(&rooms, "testroom").await.lock().await.closed = true;

        let (bob, _bob_rx) = make_participant("Bob");
        assert_eq!(join_room(&rooms, "testroom", bob).await, Err(JoinError::NotFound));

        let (carol, _carol_rx) = make_participant("Carol");
        let created = create_room(&rooms, "testroom".into(), make_item(), carol).await.unwrap();
        assert!(created, "create_room opens a fresh room in place of a closed one");
        assert!(!room_of(&rooms, "testroom").await.lock().await.closed);
    }

    #[test]
    fn validate_room_id_rejects_short() {
        assert!(validate_room_id("abc").is_err());
    }

    #[test]
    fn validate_room_id_rejects_long() {
        assert!(validate_room_id("abcdefghi").is_err());
    }

    #[test]
    fn validate_room_id_rejects_invalid_chars() {
        assert!(validate_room_id("abcd!@#$").is_err());
        assert!(validate_room_id("abcd efgh").is_err());
    }

    #[test]
    fn validate_room_id_accepts_valid() {
        assert!(validate_room_id("abcd1234").is_ok());
        assert!(validate_room_id("ABCD1234").is_ok());
        assert!(validate_room_id("AbCd1234").is_ok());
    }

    #[tokio::test]
    async fn create_room_inserts_into_map() {
        let rooms = new_room_map();
        let (host, _rx) = make_participant("Alice");
        let created = create_room(&rooms, "testroom".into(), make_item(), host).await.unwrap();
        assert!(created); // new room
        assert!(rooms.contains_key("testroom"));
    }

    #[tokio::test]
    async fn create_room_joins_existing_room() {
        let rooms = new_room_map();
        let (host, _host_rx) = make_participant("Alice");
        create_room(&rooms, "testroom".into(), make_item(), host).await.unwrap();

        let (guest, mut guest_rx) = make_participant("Bob");
        let joined = create_room(&rooms, "testroom".into(), make_item(), guest).await.unwrap();
        assert!(!joined); // joined existing
        // join_room sends room_state to the newcomer
        let msg = guest_rx.recv().await.unwrap();
        assert!(matches!(msg, ServerMessage::RoomState { .. }));
    }

    #[tokio::test]
    async fn create_room_rejects_invalid_id() {
        let rooms = new_room_map();
        let (host, _rx) = make_participant("Alice");
        let result = create_room(&rooms, "bad!id".into(), make_item(), host).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn join_room_sends_state_to_newcomer() {
        let rooms = new_room_map();
        let (host, _host_rx) = make_participant("Alice");
        create_room(&rooms, "testroom".into(), make_item(), host).await.unwrap();

        let (guest, mut guest_rx) = make_participant("Bob");
        join_room(&rooms, "testroom", guest).await.unwrap();

        let msg = guest_rx.recv().await.unwrap();
        assert!(matches!(msg, ServerMessage::RoomState { .. }));
    }

    #[tokio::test]
    async fn join_room_notifies_existing_participants() {
        let rooms = new_room_map();
        let (host, mut host_rx) = make_participant("Alice");
        create_room(&rooms, "testroom".into(), make_item(), host).await.unwrap();

        let (guest, _guest_rx) = make_participant("Bob");
        join_room(&rooms, "testroom", guest).await.unwrap();

        let msg = host_rx.recv().await.unwrap();
        assert!(matches!(msg, ServerMessage::ParticipantJoined { name } if name == "Bob"));
    }

    #[tokio::test]
    async fn join_nonexistent_room_returns_error() {
        let rooms = new_room_map();
        let (guest, _rx) = make_participant("Bob");
        let result = join_room(&rooms, "nonexist", guest).await;
        assert_eq!(result, Err(JoinError::NotFound));
    }

    #[tokio::test]
    async fn join_room_rejects_invalid_id() {
        let rooms = new_room_map();
        let (guest, _rx) = make_participant("Bob");
        let result = join_room(&rooms, "bad!", guest).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn play_intent_broadcasts_to_all_and_ready_fires_play() {
        let rooms = new_room_map();
        let (host, mut host_rx) = make_participant("Alice");
        let host_id = host.id;
        create_room(&rooms, "testroom".into(), make_item(), host).await.unwrap();
        let room_id = "testroom";

        let (guest, mut guest_rx) = make_participant("Bob");
        let guest_id = guest.id;
        join_room(&rooms, room_id, guest).await.unwrap();
        // Room was not playing, so join only sends ParticipantJoined to host
        let _ = host_rx.recv().await; // drain ParticipantJoined
        let _ = guest_rx.recv().await; // drain room_state

        handle_play_intent(&rooms, room_id, host_id, 100.0).await;

        // Both should receive PlayIntent (broadcast to all)
        let h_msg = host_rx.recv().await.unwrap();
        let g_msg = guest_rx.recv().await.unwrap();
        assert!(matches!(h_msg, ServerMessage::PlayIntent { position, .. } if (position - 100.0).abs() < f64::EPSILON));
        assert!(matches!(g_msg, ServerMessage::PlayIntent { .. }));

        // Host sends Ready — not all ready yet (guest hasn't)
        handle_ready(&rooms, room_id, host_id).await;
        assert!(host_rx.try_recv().is_err());
        assert!(guest_rx.try_recv().is_err());

        // Guest sends Ready — now all ready, Play fires to everyone
        handle_ready(&rooms, room_id, guest_id).await;
        let h_play = host_rx.recv().await.unwrap();
        let g_play = guest_rx.recv().await.unwrap();
        assert!(matches!(h_play, ServerMessage::Play { position, .. } if (position - 100.0).abs() < f64::EPSILON));
        assert!(matches!(g_play, ServerMessage::Play { .. }));
    }

    #[tokio::test]
    async fn guest_can_send_play_intent() {
        let rooms = new_room_map();
        let (host, mut host_rx) = make_participant("Alice");
        let host_id = host.id;
        create_room(&rooms, "testroom".into(), make_item(), host).await.unwrap();
        let room_id = "testroom";

        let (guest, mut guest_rx) = make_participant("Bob");
        let guest_id = guest.id;
        join_room(&rooms, room_id, guest).await.unwrap();
        let _ = host_rx.recv().await; // drain ParticipantJoined
        let _ = guest_rx.recv().await; // drain room_state

        handle_play_intent(&rooms, room_id, guest_id, 50.0).await;

        // Both receive PlayIntent
        assert!(matches!(guest_rx.recv().await.unwrap(), ServerMessage::PlayIntent { .. }));
        assert!(matches!(host_rx.recv().await.unwrap(), ServerMessage::PlayIntent { .. }));

        // Both send Ready → all ready → Play fires to both
        handle_ready(&rooms, room_id, guest_id).await;
        handle_ready(&rooms, room_id, host_id).await;

        assert!(matches!(guest_rx.recv().await.unwrap(), ServerMessage::Play { .. }));
        assert!(matches!(host_rx.recv().await.unwrap(), ServerMessage::Play { .. }));
    }

    #[tokio::test]
    async fn leave_room_removes_empty_room() {
        let rooms = new_room_map();
        let (host, _rx) = make_participant("Alice");
        let host_id = host.id;
        create_room(&rooms, "testroom".into(), make_item(), host).await.unwrap();

        leave_room(&rooms, "testroom", host_id, Holds::default()).await;

        // Leave is on purpose, so an empty room closes at once, hold or not.
        assert!(!rooms.contains_key("testroom"));
    }

    #[tokio::test]
    async fn leave_room_notifies_remaining() {
        let rooms = new_room_map();
        let (host, mut host_rx) = make_participant("Alice");
        create_room(&rooms, "testroom".into(), make_item(), host).await.unwrap();
        let room_id = "testroom";

        let (guest, _guest_rx) = make_participant("Bob");
        let guest_id = guest.id;
        join_room(&rooms, room_id, guest).await.unwrap();
        let _ = host_rx.recv().await; // drain joined notification

        leave_room(&rooms, room_id, guest_id, Holds::default()).await;

        let msg = host_rx.recv().await.unwrap();
        assert!(matches!(msg, ServerMessage::ParticipantLeft { name } if name == "Bob"));
    }
}
