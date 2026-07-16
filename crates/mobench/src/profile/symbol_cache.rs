//! Per-capture memoization for expensive native symbol lookups.
//!
//! Resolved and unresolved addresses are both cached. Native profiles often
//! repeat the same frame across many samples, so negative caching matters as
//! much as successful symbolization.

use std::collections::BTreeMap;

#[derive(Debug)]
pub(super) struct ResolutionCache<K, V> {
    entries: BTreeMap<K, Option<V>>,
}

impl<K, V> Default for ResolutionCache<K, V> {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }
}

impl<K, V> ResolutionCache<K, V>
where
    K: Ord,
    V: Clone,
{
    pub(super) fn get_or_resolve(
        &mut self,
        key: K,
        resolve: impl FnOnce() -> Option<V>,
    ) -> Option<V> {
        if let Some(cached) = self.entries.get(&key) {
            return cached.clone();
        }
        let resolved = resolve();
        self.entries.insert(key, resolved.clone());
        resolved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caches_successes_and_misses() {
        let mut cache = ResolutionCache::<u64, String>::default();
        let mut calls = 0;

        assert_eq!(
            cache.get_or_resolve(1, || {
                calls += 1;
                Some("symbol".to_owned())
            }),
            Some("symbol".to_owned())
        );
        assert_eq!(
            cache.get_or_resolve(1, || {
                calls += 1;
                Some("different".to_owned())
            }),
            Some("symbol".to_owned())
        );
        assert_eq!(
            cache.get_or_resolve(2, || {
                calls += 1;
                None
            }),
            None
        );
        assert_eq!(
            cache.get_or_resolve(2, || {
                calls += 1;
                Some("late".to_owned())
            }),
            None
        );
        assert_eq!(calls, 2);
    }
}
