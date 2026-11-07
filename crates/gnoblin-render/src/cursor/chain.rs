//! The cursor chain: the configured providers in order, with a cache and the
//! fallbacks that keep the pointer visible.
//!
//! A lookup resolves a shape in three steps: the cache, then each provider in
//! order (each run under `catch_unwind`), then the built-in arrow. A result is
//! cached only when no provider failed on the way, so a broken theme entry is
//! retried rather than remembered.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use gnoblin_types::Scale;

use super::builtin;
use super::selection;
use super::{CursorFiles, CursorImages, CursorProvider, CursorRequest, Lookup, MAX_PHYSICAL_SIZE};

use crate::log;

/// The physical pixel size for a logical size at `scale`, clamped to
/// [`MAX_PHYSICAL_SIZE`] so no provider is asked to rasterise past it.
#[must_use]
pub fn physical_size(logical: u16, scale: Scale) -> u32 {
    selection::physical_size(logical, scale).min(MAX_PHYSICAL_SIZE)
}

/// One cached answer.
struct CachedCursor {
    theme: String,
    shape: String,
    physical: u32,
    images: Arc<CursorImages>,
    bytes: usize,
}

/// Cached answers, least recently used first, with their total size in bytes.
#[derive(Default)]
struct Cache {
    entries: Vec<CachedCursor>,
    bytes: usize,
}

/// The providers, the theme they draw and the cache of their answers.
pub struct CursorChain {
    files: Arc<dyn CursorFiles>,
    theme: String,
    providers: Vec<Box<dyn CursorProvider>>,
    byte_cap: usize,
    cache: Mutex<Cache>,
}

impl CursorChain {
    /// A chain over `providers` drawing `theme` from `files`, caching at most
    /// `byte_cap` bytes of images. A theme that no directory holds falls back
    /// to the default theme, and the miss is logged once, here.
    #[must_use]
    pub fn new(
        files: Arc<dyn CursorFiles>,
        theme: &str,
        providers: Vec<Box<dyn CursorProvider>>,
        byte_cap: usize,
    ) -> Self {
        let theme = if files.theme_exists(theme) {
            theme.to_owned()
        } else {
            log::warn!(
                theme,
                fallback = selection::DEFAULT_CURSOR_THEME,
                "cursor theme not found; using the default theme"
            );
            selection::DEFAULT_CURSOR_THEME.to_owned()
        };
        Self {
            files,
            theme,
            providers,
            byte_cap,
            cache: Mutex::new(Cache::default()),
        }
    }

    /// The images for `shape` at a logical size and output scale. Never fails:
    /// the built-in arrow answers when no provider does.
    #[must_use]
    pub fn lookup(&self, shape: &str, logical: u16, scale: Scale) -> Arc<CursorImages> {
        let physical = physical_size(logical, scale);
        if let Some(hit) = self.cached(shape, physical) {
            return hit;
        }
        let (images, clean) = self.resolve(shape, physical);
        if clean {
            self.remember(shape, physical, &images);
        }
        images
    }

    /// Asks each provider in order. The flag is false when a provider failed or
    /// panicked on the way, which keeps the answer out of the cache.
    fn resolve(&self, shape: &str, physical: u32) -> (Arc<CursorImages>, bool) {
        let request = CursorRequest {
            theme: &self.theme,
            shape,
            physical,
        };
        let mut clean = true;
        for provider in &self.providers {
            let kind = provider.kind().name();
            let outcome = catch_unwind(AssertUnwindSafe(|| provider.lookup(&request, self.files.as_ref())));
            match outcome {
                Ok(Lookup::Found(images)) => return (Arc::new(images), clean),
                Ok(Lookup::Miss) => {}
                Ok(Lookup::Failed(reason)) => {
                    clean = false;
                    log::warn!(provider = kind, shape, reason = %reason, "cursor provider failed");
                }
                Err(_) => {
                    clean = false;
                    log::warn!(provider = kind, shape, "cursor provider panicked");
                }
            }
        }
        (Arc::new(builtin::arrow(physical)), clean)
    }

    /// The cached answer for a key, moved to the most recently used end.
    fn cached(&self, shape: &str, physical: u32) -> Option<Arc<CursorImages>> {
        let mut cache = self.lock();
        let index = self.position(&cache, shape, physical)?;
        let entry = cache.entries.remove(index);
        let images = Arc::clone(&entry.images);
        cache.entries.push(entry);
        Some(images)
    }

    /// Caches an answer, then evicts the least recently used entries until the
    /// cache is within its byte cap. An answer larger than the cap is not kept.
    fn remember(&self, shape: &str, physical: u32, images: &Arc<CursorImages>) {
        let bytes = images.byte_len();
        if bytes > self.byte_cap {
            return;
        }
        let mut cache = self.lock();
        if let Some(index) = self.position(&cache, shape, physical) {
            let stale = cache.entries.remove(index);
            cache.bytes -= stale.bytes;
        }
        cache.entries.push(CachedCursor {
            theme: self.theme.clone(),
            shape: shape.to_owned(),
            physical,
            images: Arc::clone(images),
            bytes,
        });
        cache.bytes += bytes;
        while cache.bytes > self.byte_cap {
            let evicted = cache.entries.remove(0);
            cache.bytes -= evicted.bytes;
        }
    }

    /// Finds the slot of a cached image for this exact shape, physical size and
    /// theme. A linear scan over the entries, matching on all three fields that
    /// identify an image.
    fn position(&self, cache: &Cache, shape: &str, physical: u32) -> Option<usize> {
        cache
            .entries
            .iter()
            .position(|entry| entry.physical == physical && entry.shape == shape && entry.theme == self.theme)
    }

    /// The cache lock. A poisoned lock still holds consistent entries, because
    /// no provider runs while it is held, so the poison is ignored.
    fn lock(&self) -> MutexGuard<'_, Cache> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::{CursorFrame, MemoryFiles, ProviderKind, SearchPath};

    /// A provider whose answer is a plain function of the request.
    struct Scripted(fn(&CursorRequest<'_>) -> Lookup);

    impl CursorProvider for Scripted {
        fn kind(&self) -> ProviderKind {
            ProviderKind::Xcursor
        }

        fn lookup(&self, request: &CursorRequest<'_>, _files: &dyn CursorFiles) -> Lookup {
            (self.0)(request)
        }
    }

    /// A solid square of `edge` pixels, so a test can tell which provider answered.
    fn marked(edge: u32) -> CursorImages {
        let frame = CursorFrame::new(edge, edge, vec![0xff00_0000; (edge * edge) as usize], 0)
            .expect("a solid square is a valid frame");
        CursorImages::new((0, 0), vec![frame]).expect("one frame is a valid cursor")
    }

    fn files() -> Arc<dyn CursorFiles> {
        Arc::new(MemoryFiles::new().with_file(SearchPath::Xcursor, "default/cursors/arrow", vec![1]))
    }

    const BIG_CAP: usize = 4 << 20;

    #[test]
    fn a_missing_theme_uses_the_default_theme() {
        let provider = Scripted(|request| {
            if request.theme == "default" {
                Lookup::Found(marked(7))
            } else {
                Lookup::Miss
            }
        });
        let chain = CursorChain::new(files(), "Missing", vec![Box::new(provider)], BIG_CAP);
        let images = chain.lookup("arrow", 24, Scale::IDENTITY);
        assert_eq!(images.width(), 7, "the default theme answered, not the arrow");
    }

    #[test]
    fn a_failed_provider_hands_the_shape_to_the_next_one() {
        let chain = CursorChain::new(
            files(),
            "default",
            vec![
                Box::new(Scripted(|_| Lookup::Failed("bad file".to_owned()))),
                Box::new(Scripted(|_| Lookup::Found(marked(5)))),
            ],
            BIG_CAP,
        );
        assert_eq!(chain.lookup("arrow", 24, Scale::IDENTITY).width(), 5);
    }

    #[test]
    fn a_panicking_provider_is_a_failure() {
        let chain = CursorChain::new(
            files(),
            "default",
            vec![
                Box::new(Scripted(|_| panic!("hostile theme"))),
                Box::new(Scripted(|_| Lookup::Found(marked(6)))),
            ],
            BIG_CAP,
        );
        assert_eq!(chain.lookup("arrow", 24, Scale::IDENTITY).width(), 6);
    }

    #[test]
    fn with_no_provider_the_built_in_arrow_answers() {
        let chain = CursorChain::new(files(), "default", Vec::new(), BIG_CAP);
        let images = chain.lookup("arrow", 24, Scale::IDENTITY);
        assert_eq!(*images, builtin::arrow(24));
    }

    #[test]
    fn the_cache_is_keyed_by_size_and_scale() {
        let chain = CursorChain::new(files(), "default", Vec::new(), BIG_CAP);
        let at_one = chain.lookup("arrow", 24, Scale::IDENTITY);
        let at_two = chain.lookup("arrow", 24, Scale::integral(2));
        assert_eq!(at_one.width(), 24);
        assert_eq!(at_two.width(), 48, "the 2x scale is a different cache entry");
        assert!(Arc::ptr_eq(&at_one, &chain.lookup("arrow", 24, Scale::IDENTITY)));
        assert!(Arc::ptr_eq(&at_two, &chain.lookup("arrow", 24, Scale::integral(2))));
    }

    #[test]
    fn the_cache_evicts_the_least_recently_used() {
        // Each 24 pixel arrow is 24 * 24 * 4 bytes, so the cap holds exactly two.
        let one_arrow = 24 * 24 * 4;
        let chain = CursorChain::new(files(), "default", Vec::new(), 2 * one_arrow);
        let a_first = chain.lookup("a", 24, Scale::IDENTITY);
        let b_first = chain.lookup("b", 24, Scale::IDENTITY);
        assert!(Arc::ptr_eq(&a_first, &chain.lookup("a", 24, Scale::IDENTITY)));
        // Touching `a` made `b` the least recently used, so `c` evicts `b`.
        let _ = chain.lookup("c", 24, Scale::IDENTITY);
        assert!(Arc::ptr_eq(&a_first, &chain.lookup("a", 24, Scale::IDENTITY)));
        assert!(!Arc::ptr_eq(&b_first, &chain.lookup("b", 24, Scale::IDENTITY)));
    }

    #[test]
    fn the_physical_size_is_clamped_before_lookup() {
        let chain = CursorChain::new(files(), "default", Vec::new(), 4 << 20);
        let max_logical = crate::cursor::selection::MAX_CURSOR_SIZE;
        let images = chain.lookup("arrow", max_logical, Scale::integral(4));
        assert_eq!(images.width(), MAX_PHYSICAL_SIZE);
    }

    #[test]
    fn an_unknown_provider_name_is_not_a_kind() {
        assert_eq!(ProviderKind::from_name("hyprcursor-plus"), None);
    }
}
