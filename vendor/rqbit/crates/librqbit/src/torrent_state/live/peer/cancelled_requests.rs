use std::{collections::HashMap, hash::Hash};

/// Cancelled requests still require a piece or reject response (BEP 52).
/// Keep exact identities and multiplicity, with a per-connection memory bound.
#[derive(Debug)]
pub(super) struct CancelledRequests<K> {
    requests: HashMap<K, usize>,
    count: usize,
}

impl<K> Default for CancelledRequests<K> {
    fn default() -> Self {
        Self {
            requests: HashMap::new(),
            count: 0,
        }
    }
}

impl<K: Eq + Hash> CancelledRequests<K> {
    pub fn insert(&mut self, request: K) -> bool {
        if self.count == 4096 {
            return false;
        }
        *self.requests.entry(request).or_default() += 1;
        self.count += 1;
        true
    }

    pub fn remove(&mut self, request: &K) -> bool {
        let Some(count) = self.requests.get_mut(request) else {
            return false;
        };
        *count -= 1;
        self.count -= 1;
        if *count == 0 {
            self.requests.remove(request);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::CancelledRequests;

    #[test]
    fn cancellation_never_allows_an_unrelated_response() {
        let mut ledger = CancelledRequests::default();
        assert!(ledger.insert((0, 0, 16384)));
        assert!(!ledger.remove(&(0, 16384, 16384)));
        assert!(!ledger.remove(&(1, 0, 16384)));
        assert!(!ledger.remove(&(0, 0, 1)));
        assert!(ledger.remove(&(0, 0, 16384)));
        assert!(!ledger.remove(&(0, 0, 16384)));
    }

    #[test]
    fn repeated_requests_each_require_their_own_response() {
        let mut ledger = CancelledRequests::default();
        assert!(ledger.insert(7));
        assert!(ledger.insert(7));
        assert!(ledger.remove(&7));
        assert!(ledger.remove(&7));
        assert!(!ledger.remove(&7));
    }

    #[test]
    fn unanswered_requests_are_bounded_and_responses_release_capacity() {
        let mut ledger = CancelledRequests::default();
        for i in 0..4096 {
            assert!(ledger.insert(i));
        }
        assert!(!ledger.insert(5000));
        assert!(!ledger.insert(0));
        assert!(ledger.remove(&1));
        assert!(ledger.insert(5000));
        assert!(!ledger.insert(5001));
    }
}
