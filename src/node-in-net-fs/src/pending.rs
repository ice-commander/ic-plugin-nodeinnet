use nodeinnet_protocol::P2pMessage;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use tokio::sync::mpsc;
use uuid::Uuid;

pub fn get_pending_requests() -> &'static Mutex<HashMap<Uuid, mpsc::UnboundedSender<P2pMessage>>> {
    static PENDING: OnceLock<Mutex<HashMap<Uuid, mpsc::UnboundedSender<P2pMessage>>>> =
        OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

pub struct PendingGuard(pub Uuid);

impl Drop for PendingGuard {
    fn drop(&mut self) {
        if let Ok(mut pending) = get_pending_requests().lock() {
            pending.remove(&self.0);
        }
    }
}

pub fn feed_response(msg: P2pMessage) -> Option<P2pMessage> {
    use P2pMessage as M;
    let id = match &msg {
        M::EntriesResponse { request_id, .. }
        | M::MetadataResponse { request_id, .. }
        | M::CreateDirectoryResponse { request_id, .. }
        | M::DeleteEntryResponse { request_id, .. }
        | M::RenameEntryResponse { request_id, .. }
        | M::SetPermissionsResponse { request_id, .. } => *request_id,
        M::FileTransferResponse { transfer_id, .. }
        | M::FileTransferComplete { transfer_id, .. } => *transfer_id,
        _ => return Some(msg),
    };
    deliver(id, msg)
}

pub fn deliver(id: Uuid, msg: P2pMessage) -> Option<P2pMessage> {
    let waiting = get_pending_requests()
        .lock()
        .ok()
        .and_then(|held| held.get(&id).cloned());
    match waiting {
        Some(tx) => {
            let _ = tx.send(msg);
            None
        }
        None => Some(msg),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_reaches_the_call_that_is_waiting_for_it() {
        let id = Uuid::new_v4();
        let (tx, mut rx) = mpsc::unbounded_channel();
        get_pending_requests().lock().unwrap().insert(id, tx);
        let _guard = PendingGuard(id);

        assert!(deliver(id, P2pMessage::Ping(7)).is_none());
        assert!(matches!(rx.try_recv(), Ok(P2pMessage::Ping(7))));
    }

    #[test]
    fn a_reply_nobody_waits_for_is_handed_back_rather_than_dropped() {
        let stray = deliver(Uuid::new_v4(), P2pMessage::Ping(7));
        assert!(matches!(stray, Some(P2pMessage::Ping(7))));
    }

    #[test]
    fn the_guard_takes_the_waiting_call_out_when_it_falls_out_of_scope() {
        let id = Uuid::new_v4();
        let (tx, _rx) = mpsc::unbounded_channel();
        get_pending_requests().lock().unwrap().insert(id, tx);
        {
            let _guard = PendingGuard(id);
            assert!(get_pending_requests().lock().unwrap().contains_key(&id));
        }
        assert!(!get_pending_requests().lock().unwrap().contains_key(&id));
    }
}
