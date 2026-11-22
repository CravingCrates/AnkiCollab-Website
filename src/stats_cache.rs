use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::watch;

use crate::structs::{
    DeckBaseStatsInfo, DeckStatsInfo, NoteStatsInfo, NotetypeStatsInfo, RetentionBucket,
    RetentionTrendPoint, TagStatsInfo,
};

/// Default time-to-live for cached deck statistics payloads (15 minutes).
/// Well within the 24-hour refresh and privacy cache window.
pub const DEFAULT_TTL: Duration = Duration::from_mins(15);

/// Maximum number of decks cached concurrently in memory.
pub const DEFAULT_MAX_ENTRIES: usize = 64;

#[derive(Debug, Clone)]
pub struct DeckStatsPayload {
    pub base: DeckBaseStatsInfo,
    pub decks: Vec<DeckStatsInfo>,
    pub notes: Vec<NoteStatsInfo>,
    pub retention_buckets: Vec<RetentionBucket>,
    pub retention_trend: Vec<RetentionTrendPoint>,
    pub tag_stats: Vec<TagStatsInfo>,
    pub notetype_stats: Vec<NotetypeStatsInfo>,
}

type SharedLoadResult = Result<Arc<DeckStatsPayload>, String>;

struct CacheEntry {
    payload: Arc<DeckStatsPayload>,
    cached_at: Instant,
    last_accessed: Instant,
}

struct InFlightEntry {
    id: u64,
    stale: bool,
    rx: watch::Receiver<Option<SharedLoadResult>>,
}

struct StatsCacheInner {
    next_flight_id: u64,
    entries: HashMap<i64, CacheEntry>,
    in_flight: HashMap<i64, InFlightEntry>,
}

/// Bounded TTL cache with per-deck singleflight coalescing for computed statistics pages.
pub struct StatsCache {
    ttl: Duration,
    max_entries: usize,
    inner: Mutex<StatsCacheInner>,
}

impl fmt::Debug for StatsCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let inner = self.lock_inner();
        f.debug_struct("StatsCache")
            .field("ttl", &self.ttl)
            .field("max_entries", &self.max_entries)
            .field("entries", &inner.entries.len())
            .field("in_flight", &inner.in_flight.len())
            .finish_non_exhaustive()
    }
}

impl Default for StatsCache {
    fn default() -> Self {
        Self::new()
    }
}

struct InFlightGuard<'a> {
    cache: &'a StatsCache,
    deck_id: i64,
    flight_id: u64,
    tx: Option<watch::Sender<Option<SharedLoadResult>>>,
}

impl Drop for InFlightGuard<'_> {
    fn drop(&mut self) {
        if let Some(tx) = self.tx.take() {
            {
                let mut inner = self.cache.lock_inner();
                if inner
                    .in_flight
                    .get(&self.deck_id)
                    .is_some_and(|flight| flight.id == self.flight_id)
                {
                    inner.in_flight.remove(&self.deck_id);
                }
            }
            let _ = tx.send(Some(Err("Statistics load was cancelled".to_string())));
        }
    }
}

impl StatsCache {
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(DEFAULT_TTL, DEFAULT_MAX_ENTRIES)
    }

    #[must_use]
    pub fn with_config(ttl: Duration, max_entries: usize) -> Self {
        Self {
            ttl,
            max_entries,
            inner: Mutex::new(StatsCacheInner {
                next_flight_id: 0,
                entries: HashMap::new(),
                in_flight: HashMap::new(),
            }),
        }
    }

    fn lock_inner(&self) -> std::sync::MutexGuard<'_, StatsCacheInner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn prune_expired_locked(&self, inner: &mut StatsCacheInner, now: Instant) {
        inner
            .entries
            .retain(|_, entry| now.saturating_duration_since(entry.cached_at) < self.ttl);
    }

    fn insert_evicting_locked(
        &self,
        inner: &mut StatsCacheInner,
        deck_id: i64,
        payload: Arc<DeckStatsPayload>,
        now: Instant,
    ) {
        if self.max_entries == 0 || self.ttl.is_zero() {
            return;
        }
        self.prune_expired_locked(inner, now);

        while inner.entries.len() >= self.max_entries && !inner.entries.contains_key(&deck_id) {
            let lru_key = inner
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_accessed)
                .map(|(key, _)| *key);
            if let Some(evict_id) = lru_key {
                inner.entries.remove(&evict_id);
            } else {
                break;
            }
        }

        inner.entries.insert(
            deck_id,
            CacheEntry {
                payload,
                cached_at: now,
                last_accessed: now,
            },
        );
    }

    /// Return a fresh cached payload for `deck_id`, updating its last-access timestamp.
    pub fn get(&self, deck_id: i64) -> Option<Arc<DeckStatsPayload>> {
        let now = Instant::now();
        let mut inner = self.lock_inner();
        let is_fresh = inner
            .entries
            .get(&deck_id)
            .is_some_and(|entry| now.saturating_duration_since(entry.cached_at) < self.ttl);

        if is_fresh {
            if let Some(entry) = inner.entries.get_mut(&deck_id) {
                entry.last_accessed = now;
                return Some(Arc::clone(&entry.payload));
            }
        }

        inner.entries.remove(&deck_id);
        None
    }

    /// Return a cached payload or invoke `loader` once per deck concurrently (singleflight).
    pub async fn get_or_load<F, Fut, E>(&self, deck_id: i64, loader: F) -> SharedLoadResult
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<DeckStatsPayload, E>>,
        E: fmt::Display,
    {
        enum Action {
            Hit(Arc<DeckStatsPayload>),
            Wait(watch::Receiver<Option<SharedLoadResult>>),
            Lead(u64, watch::Sender<Option<SharedLoadResult>>),
        }

        let action = {
            let now = Instant::now();
            let mut inner = self.lock_inner();
            let is_fresh = inner
                .entries
                .get(&deck_id)
                .is_some_and(|entry| now.saturating_duration_since(entry.cached_at) < self.ttl);

            if is_fresh {
                let entry = inner
                    .entries
                    .get_mut(&deck_id)
                    .expect("verified fresh entry");
                entry.last_accessed = now;
                Action::Hit(Arc::clone(&entry.payload))
            } else {
                inner.entries.remove(&deck_id);
                if let Some(flight) = inner.in_flight.get(&deck_id) {
                    Action::Wait(flight.rx.clone())
                } else {
                    inner.next_flight_id = inner.next_flight_id.wrapping_add(1);
                    let flight_id = inner.next_flight_id;
                    let (tx, rx) = watch::channel(None);
                    inner.in_flight.insert(
                        deck_id,
                        InFlightEntry {
                            id: flight_id,
                            stale: false,
                            rx,
                        },
                    );
                    Action::Lead(flight_id, tx)
                }
            }
        };

        let (flight_id, tx) = match action {
            Action::Hit(payload) => return Ok(payload),
            Action::Wait(mut rx) => {
                return match rx.wait_for(Option::is_some).await {
                    Ok(guard) => guard
                        .as_ref()
                        .cloned()
                        .unwrap_or_else(|| Err("Statistics load interrupted".to_string())),
                    Err(_) => Err("Statistics load interrupted".to_string()),
                };
            }
            Action::Lead(flight_id, tx) => (flight_id, tx),
        };

        let mut guard = InFlightGuard {
            cache: self,
            deck_id,
            flight_id,
            tx: Some(tx),
        };

        let outcome = loader().await;
        let result: SharedLoadResult = match outcome {
            Ok(payload) => {
                let arc = Arc::new(payload);
                let now = Instant::now();
                let mut inner = self.lock_inner();
                let was_stale = inner
                    .in_flight
                    .get(&deck_id)
                    .filter(|flight| flight.id == flight_id)
                    .is_some_and(|flight| flight.stale);
                if inner
                    .in_flight
                    .get(&deck_id)
                    .is_some_and(|flight| flight.id == flight_id)
                {
                    inner.in_flight.remove(&deck_id);
                }
                if !was_stale {
                    self.insert_evicting_locked(&mut inner, deck_id, Arc::clone(&arc), now);
                }
                Ok(arc)
            }
            Err(err) => {
                let mut inner = self.lock_inner();
                if inner
                    .in_flight
                    .get(&deck_id)
                    .is_some_and(|flight| flight.id == flight_id)
                {
                    inner.in_flight.remove(&deck_id);
                }
                Err(err.to_string())
            }
        };

        if let Some(sender) = guard.tx.take() {
            let _ = sender.send(Some(result.clone()));
        }
        result
    }

    pub fn invalidate_deck(&self, deck_id: i64) {
        let mut inner = self.lock_inner();
        inner.entries.remove(&deck_id);
        if let Some(flight) = inner.in_flight.get_mut(&deck_id) {
            flight.stale = true;
        }
    }

    pub fn invalidate_all(&self) {
        let mut inner = self.lock_inner();
        inner.entries.clear();
        for flight in inner.in_flight.values_mut() {
            flight.stale = true;
        }
    }

    pub fn prune_expired(&self) {
        let mut inner = self.lock_inner();
        self.prune_expired_locked(&mut inner, Instant::now());
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.lock_inner().entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Notify;

    fn sample_payload(note_count: i32) -> DeckStatsPayload {
        DeckStatsPayload {
            base: DeckBaseStatsInfo {
                note_count,
                total_reviewed_notes: 100,
                coverage_pct: 50.0,
                lapses_avg: 1.5,
                reps_avg: 6.0,
                retention_avg: 85.0,
                contributor_count: 10,
                median_sample_size: 8.0,
                leech_count: 2,
                recently_revised_struggling: 1,
                trend_delta_30d: Some(1.2),
            },
            decks: Vec::new(),
            notes: Vec::new(),
            retention_buckets: Vec::new(),
            retention_trend: Vec::new(),
            tag_stats: Vec::new(),
            notetype_stats: Vec::new(),
        }
    }

    #[tokio::test]
    async fn cache_hit_returns_payload_without_reloading() {
        let cache = StatsCache::with_config(Duration::from_mins(5), 8);
        let calls = AtomicUsize::new(0);

        let first = cache
            .get_or_load(1, || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok::<_, &str>(sample_payload(42))
            })
            .await
            .unwrap();
        let second = cache
            .get_or_load(1, || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok::<_, &str>(sample_payload(99))
            })
            .await
            .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(first.base.note_count, 42);
        assert_eq!(second.base.note_count, 42);
        assert!(Arc::ptr_eq(&first, &second));
        assert!(cache.get(1).is_some());
    }

    #[tokio::test]
    async fn ttl_expiry_reloads_entry() {
        let cache = StatsCache::with_config(Duration::from_millis(25), 8);

        let first = cache
            .get_or_load(7, || async { Ok::<_, &str>(sample_payload(10)) })
            .await
            .unwrap();
        assert_eq!(first.base.note_count, 10);

        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(cache.get(7).is_none());

        let reloaded = cache
            .get_or_load(7, || async { Ok::<_, &str>(sample_payload(20)) })
            .await
            .unwrap();
        assert_eq!(reloaded.base.note_count, 20);
    }

    #[tokio::test]
    async fn capacity_evicts_least_recently_accessed_entry() {
        let cache = StatsCache::with_config(Duration::from_mins(5), 2);

        cache
            .get_or_load(1, || async { Ok::<_, &str>(sample_payload(1)) })
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(5)).await;
        cache
            .get_or_load(2, || async { Ok::<_, &str>(sample_payload(2)) })
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(5)).await;

        // Touch deck 1 so deck 2 becomes least recently accessed.
        assert!(cache.get(1).is_some());

        cache
            .get_or_load(3, || async { Ok::<_, &str>(sample_payload(3)) })
            .await
            .unwrap();

        assert_eq!(cache.len(), 2);
        assert!(cache.get(1).is_some());
        assert!(cache.get(2).is_none());
        assert!(cache.get(3).is_some());
    }

    #[tokio::test]
    async fn singleflight_coalesces_concurrent_requests_and_respects_invalidation() {
        let cache = Arc::new(StatsCache::with_config(Duration::from_mins(5), 8));
        let calls = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new(Notify::new());
        let started = Arc::new(Notify::new());

        let c1 = Arc::clone(&cache);
        let calls1 = Arc::clone(&calls);
        let gate1 = Arc::clone(&gate);
        let started1 = Arc::clone(&started);
        let t1 = tokio::spawn(async move {
            c1.get_or_load(55, || async move {
                calls1.fetch_add(1, Ordering::SeqCst);
                started1.notify_one();
                gate1.notified().await;
                Ok::<_, &str>(sample_payload(55))
            })
            .await
        });

        started.notified().await;

        let c2 = Arc::clone(&cache);
        let calls2 = Arc::clone(&calls);
        let t2 = tokio::spawn(async move {
            c2.get_or_load(55, || async move {
                calls2.fetch_add(1, Ordering::SeqCst);
                Ok::<_, &str>(sample_payload(999))
            })
            .await
        });

        // Give waiter a moment to subscribe before invalidating and releasing.
        tokio::task::yield_now().await;
        cache.invalidate_all();
        gate.notify_waiters();

        let r1 = t1.await.unwrap().unwrap();
        let r2 = t2.await.unwrap().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(Arc::ptr_eq(&r1, &r2));
        // Because invalidate_all ran while the flight was active, the stale payload was not cached.
        assert!(cache.get(55).is_none());
    }

    #[tokio::test]
    async fn dropping_cancelled_leader_unblocks_waiter_and_cleans_in_flight() {
        let cache = Arc::new(StatsCache::with_config(Duration::from_mins(5), 8));
        let started = Arc::new(Notify::new());
        let gate = Arc::new(Notify::new());

        let c1 = Arc::clone(&cache);
        let s1 = Arc::clone(&started);
        let g1 = Arc::clone(&gate);
        let leader = tokio::spawn(async move {
            c1.get_or_load(88, || async move {
                s1.notify_one();
                g1.notified().await;
                Ok::<_, &str>(sample_payload(88))
            })
            .await
        });

        started.notified().await;

        let c2 = Arc::clone(&cache);
        let waiter = tokio::spawn(async move {
            c2.get_or_load(88, || async { Ok::<_, &str>(sample_payload(1)) })
                .await
        });
        tokio::task::yield_now().await;

        leader.abort();
        let _ = leader.await;

        let waiter_res = waiter.await.unwrap();
        assert!(waiter_res.is_err());

        let recovered = cache
            .get_or_load(88, || async { Ok::<_, &str>(sample_payload(123)) })
            .await
            .unwrap();
        assert_eq!(recovered.base.note_count, 123);
    }
}
