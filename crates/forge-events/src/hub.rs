//! Routes events to topic subscriptions.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use forge_api::SubscriptionId;
use forge_api::events::{Event, EventNotification, Topic};

/// Receives notifications for one subscription. Returns `false` once the receiver is gone,
/// and the hub drops the subscription.
pub type Subscriber = Box<dyn Fn(EventNotification) -> bool + Send + Sync>;

struct Subscription {
    topics: HashSet<Topic>,
    deliver: Subscriber,
}

/// All live subscriptions in the daemon.
#[derive(Default)]
pub struct Hub {
    subs: Mutex<HashMap<SubscriptionId, Subscription>>,
}

impl Hub {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn subscribe(
        &self,
        topics: impl IntoIterator<Item = Topic>,
        deliver: Subscriber,
    ) -> SubscriptionId {
        let id = SubscriptionId::new();
        let sub = Subscription {
            topics: topics.into_iter().collect(),
            deliver,
        };
        self.lock().insert(id.clone(), sub);
        id
    }

    /// Returns whether the subscription existed.
    pub fn unsubscribe(&self, id: &SubscriptionId) -> bool {
        self.lock().remove(id).is_some()
    }

    /// Deliver `event` to every subscription that includes `topic`.
    pub fn publish(&self, topic: &Topic, event: &Event) {
        let mut subs = self.lock();
        subs.retain(|id, sub| {
            if !sub.topics.contains(topic) {
                return true;
            }
            (sub.deliver)(EventNotification {
                subscription_id: id.clone(),
                topic: topic.clone(),
                seq: event.seq,
                event: event.clone(),
            })
        });
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<SubscriptionId, Subscription>> {
        // A panic while holding the lock leaves the map itself consistent.
        self.subs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_api::SessionId;
    use forge_api::events::{EventPayload, SessionStateBody};
    use forge_api::model::SessionState;
    use std::sync::Arc;

    fn event(session: &SessionId, seq: u64) -> Event {
        Event {
            seq,
            session_id: session.clone(),
            at: forge_api::Timestamp::now(),
            parent: None,
            payload: EventPayload::SessionState(SessionStateBody {
                from: SessionState::Ready,
                to: SessionState::Working,
                reason: None,
            }),
        }
    }

    fn collector() -> (Arc<Mutex<Vec<EventNotification>>>, Subscriber) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        (
            seen,
            Box::new(move |n| {
                sink.lock().unwrap().push(n);
                true
            }),
        )
    }

    #[test]
    fn delivers_only_matching_topics() {
        let hub = Hub::new();
        let session = SessionId::new();
        let (board, board_sub) = collector();
        let (approvals, approvals_sub) = collector();
        let board_id = hub.subscribe([Topic::Board, Topic::Session(session.clone())], board_sub);
        hub.subscribe([Topic::Approvals], approvals_sub);

        hub.publish(&Topic::Board, &event(&session, 1));
        hub.publish(&Topic::Session(session.clone()), &event(&session, 2));

        let board = board.lock().unwrap();
        assert_eq!(board.len(), 2);
        assert_eq!(board[0].subscription_id, board_id);
        assert_eq!(board[1].seq, 2);
        assert!(approvals.lock().unwrap().is_empty());
    }

    #[test]
    fn unsubscribe_and_closed_receivers_remove_subscriptions() {
        let hub = Hub::new();
        let session = SessionId::new();
        let (_, sub) = collector();
        let id = hub.subscribe([Topic::Board], sub);
        hub.subscribe([Topic::Board], Box::new(|_| false));
        assert_eq!(hub.len(), 2);

        hub.publish(&Topic::Board, &event(&session, 1));
        assert_eq!(hub.len(), 1);
        assert!(hub.unsubscribe(&id));
        assert!(!hub.unsubscribe(&id));
        assert!(hub.is_empty());
    }
}
