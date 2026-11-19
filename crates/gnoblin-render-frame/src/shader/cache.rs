//! The compiled-shader cache every renderer keeps its user shader programs in.
//!
//! The key is the hash of the source ([`ShaderKey`]): two passes with the same text share one program whatever their params are.
//! The bound is [`SHADER_CACHE_ENTRIES`] entries, least recently used out first, so a shell that writes a new source every frame
//! cannot grow the renderer's GPU memory without end. The cache holds whatever a renderer compiles to (a GL program, a Vulkan
//! pipeline) or a failure (`Result<Program, ShaderError>`, so a broken source is compiled once and not once a frame); it does
//! not name a GPU API, and it hands back what it evicts so the renderer can free the object.

use std::collections::VecDeque;

use gnoblin_render::ShaderKey;

/// The most compiled shaders a renderer holds.
pub const SHADER_CACHE_ENTRIES: usize = 64;

/// A least-recently-used cache of at most [`SHADER_CACHE_ENTRIES`] entries by [`ShaderKey`].
///
/// With a bound of 64 a linear scan of the queue is cheaper than a hash map and keeps the recency order in the one structure.
#[derive(Debug)]
pub struct ShaderCache<V> {
    /// The entries, least recently used first.
    entries: VecDeque<(ShaderKey, V)>,
}

impl<V> Default for ShaderCache<V> {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
        }
    }
}

impl<V> ShaderCache<V> {
    /// An empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of entries held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether `key` is held, without counting as a use.
    #[must_use]
    pub fn contains(&self, key: ShaderKey) -> bool {
        self.entries.iter().any(|(held, _)| *held == key)
    }

    /// The entry for `key`, which becomes the most recently used.
    pub fn get(&mut self, key: ShaderKey) -> Option<&mut V> {
        let at = self.entries.iter().position(|(held, _)| *held == key)?;
        let entry = self.entries.remove(at)?;
        self.entries.push_back(entry);
        self.entries.back_mut().map(|(_, value)| value)
    }

    /// Stores `value` for `key` as the most recently used entry, and returns what left the cache for it: the entry the key
    /// replaced, or else the least recently used one when the cache was full. The renderer frees what comes back.
    pub fn insert(&mut self, key: ShaderKey, value: V) -> Option<(ShaderKey, V)> {
        let replaced = self
            .entries
            .iter()
            .position(|(held, _)| *held == key)
            .and_then(|at| self.entries.remove(at));
        let evicted = if replaced.is_none() && self.entries.len() >= SHADER_CACHE_ENTRIES {
            self.entries.pop_front()
        } else {
            replaced
        };
        self.entries.push_back((key, value));
        evicted
    }

    /// Takes the entry for `key` out of the cache.
    pub fn remove(&mut self, key: ShaderKey) -> Option<V> {
        let at = self.entries.iter().position(|(held, _)| *held == key)?;
        self.entries.remove(at).map(|(_, value)| value)
    }

    /// Empties the cache and hands every entry back, for a renderer that lost its device or is shutting down.
    pub fn drain(&mut self) -> impl Iterator<Item = (ShaderKey, V)> + '_ {
        self.entries.drain(..)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: usize) -> ShaderKey {
        ShaderKey::of(&format!("fn effect{n}() {{}}"))
    }

    #[test]
    fn a_full_cache_drops_the_least_recently_used_entry() {
        let mut cache = ShaderCache::new();
        for n in 0..SHADER_CACHE_ENTRIES {
            assert!(cache.insert(key(n), n).is_none(), "room for {SHADER_CACHE_ENTRIES}");
        }
        assert_eq!(cache.len(), SHADER_CACHE_ENTRIES);
        // Using the oldest entry makes the next one the oldest.
        assert_eq!(cache.get(key(0)).copied(), Some(0));
        let evicted = cache.insert(key(1000), 1000).expect("the cache was full");
        assert_eq!(evicted, (key(1), 1));
        assert_eq!(cache.len(), SHADER_CACHE_ENTRIES);
        assert!(cache.contains(key(0)) && cache.contains(key(1000)));
        assert!(!cache.contains(key(1)));
    }

    #[test]
    fn a_key_stored_again_replaces_its_entry_and_hands_the_old_one_back() {
        let mut cache = ShaderCache::new();
        cache.insert(key(1), "old");
        cache.insert(key(2), "other");
        assert_eq!(cache.insert(key(1), "new"), Some((key(1), "old")));
        assert_eq!(cache.len(), 2, "no entry was added");
        assert_eq!(cache.get(key(1)).copied(), Some("new"));
    }

    #[test]
    fn drain_empties_the_cache_and_returns_everything() {
        let mut cache = ShaderCache::new();
        cache.insert(key(1), 1);
        cache.insert(key(2), 2);
        let values: Vec<i32> = cache.drain().map(|(_, value)| value).collect();
        assert_eq!(values, [1, 2]);
        assert!(cache.is_empty());
        assert_eq!(cache.remove(key(1)), None);
    }
}
