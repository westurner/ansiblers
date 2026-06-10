//! Module result caching (Phase 6, Weeks 37-38).
//!
//! [`CachingModuleRegistry`] wraps a [`ModuleRegistry`] and caches results for
//! idempotent, read-only modules (e.g. `stat`, `setup`, `find`) so that
//! repeated invocations within a playbook run skip the underlying work.
//!
//! ## Storage backends
//!
//! | Backend | Description |
//! |---------|-------------|
//! | [`InMemoryCache`] | `HashMap` — cleared when the registry is dropped |
//! | [`SqliteCache`] | SQLite via `rusqlite` — persists across runs |
//!
//! ## Cache key
//!
//! Keys are derived from `(module_name, host, sha256(args_json))` so that two
//! tasks calling the same module with the same arguments on the same host
//! receive the same result without re-executing.
//!
//! ## Usage
//!
//! ```rust,no_run
//! use ansiblers_modules::{ModuleRegistry, CachingModuleRegistry, InMemoryCache};
//! use std::sync::Arc;
//!
//! let inner = ModuleRegistry::with_defaults();
//! let cache = Arc::new(InMemoryCache::default());
//! let cached = CachingModuleRegistry::new(inner, cache);
//!
//! // The registry can be used exactly like the standard one:
//! // cached.invoke("stat", &args, "host1", &mut ctx)?;
//! ```

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex};

use ansiblers_core::{ExecutionContext, TaskResult};
use anyhow::Result;
use serde_json;
use tracing::{debug, trace};

use crate::registry::{ModuleArgs, ModuleInvoker, ModuleRegistry};

// ---------------------------------------------------------------------------
// CacheKey
// ---------------------------------------------------------------------------

/// Opaque key derived from (module, host, args_hash).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    module: String,
    host: String,
    args_hash: u64,
}

impl CacheKey {
    pub fn new(module: &str, host: &str, args: &ModuleArgs) -> Self {
        let serialised = serde_json::to_string(&args.args).unwrap_or_default();
        let mut hasher = DefaultHasher::new();
        serialised.hash(&mut hasher);
        Self {
            module: module.to_string(),
            host: host.to_string(),
            args_hash: hasher.finish(),
        }
    }
}

// ---------------------------------------------------------------------------
// ModuleResultCache trait
// ---------------------------------------------------------------------------

/// Trait for module result cache backends.
pub trait ModuleResultCache: Send + Sync {
    /// Look up a cached result.  Returns `None` on miss.
    fn get(&self, key: &CacheKey) -> Option<TaskResult>;
    /// Store a result.
    fn put(&self, key: CacheKey, result: TaskResult);
    /// Invalidate all entries for `host`.
    fn invalidate_host(&self, host: &str);
    /// Clear the entire cache.
    fn clear(&self);
    /// Return the number of cached entries.
    fn len(&self) -> usize;
    /// Return `true` if the cache is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ---------------------------------------------------------------------------
// InMemoryCache
// ---------------------------------------------------------------------------

/// In-memory LRU-like cache backed by a plain `HashMap`.
///
/// The cache is not bounded — for large playbooks consider [`SqliteCache`]
/// or an LRU wrapper.  This backend is ideal for unit tests and short-lived
/// playbook runs.
#[derive(Debug, Default)]
pub struct InMemoryCache {
    store: Mutex<HashMap<CacheKey, TaskResult>>,
}

impl ModuleResultCache for InMemoryCache {
    fn get(&self, key: &CacheKey) -> Option<TaskResult> {
        let guard = self.store.lock().unwrap();
        let result = guard.get(key).cloned();
        if result.is_some() {
            trace!(module = key.module, host = key.host, "cache HIT");
        }
        result
    }

    fn put(&self, key: CacheKey, result: TaskResult) {
        trace!(module = key.module, host = key.host, "cache PUT");
        self.store.lock().unwrap().insert(key, result);
    }

    fn invalidate_host(&self, host: &str) {
        let mut guard = self.store.lock().unwrap();
        guard.retain(|k, _| k.host != host);
    }

    fn clear(&self) {
        self.store.lock().unwrap().clear();
    }

    fn len(&self) -> usize {
        self.store.lock().unwrap().len()
    }
}

// ---------------------------------------------------------------------------
// SqliteCache
// ---------------------------------------------------------------------------

/// Persistent cache backed by SQLite (via `rusqlite`).
///
/// Results are serialised as JSON and stored in a `module_cache` table.
/// The database is created at `path`; use `:memory:` for an in-process DB.
pub struct SqliteCache {
    conn: Mutex<rusqlite::Connection>,
}

impl SqliteCache {
    /// Open (or create) a SQLite cache at `path`.
    /// Pass `":memory:"` for a temporary in-process database.
    pub fn open(path: &str) -> Result<Self> {
        let conn = rusqlite::Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS module_cache (
                module     TEXT NOT NULL,
                host       TEXT NOT NULL,
                args_hash  INTEGER NOT NULL,
                result_json TEXT NOT NULL,
                created_at REAL NOT NULL DEFAULT (unixepoch('now', 'subsec')),
                PRIMARY KEY (module, host, args_hash)
            );",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
}

impl ModuleResultCache for SqliteCache {
    fn get(&self, key: &CacheKey) -> Option<TaskResult> {
        let conn = self.conn.lock().unwrap();
        let result: Option<String> = conn
            .query_row(
                "SELECT result_json FROM module_cache
                 WHERE module = ?1 AND host = ?2 AND args_hash = ?3",
                rusqlite::params![key.module, key.host, key.args_hash as i64],
                |row| row.get(0),
            )
            .ok();

        if let Some(json) = result {
            trace!(module = key.module, host = key.host, "sqlite cache HIT");
            serde_json::from_str(&json).ok()
        } else {
            None
        }
    }

    fn put(&self, key: CacheKey, result: TaskResult) {
        let Ok(json) = serde_json::to_string(&result) else {
            return;
        };
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute(
            "INSERT OR REPLACE INTO module_cache (module, host, args_hash, result_json)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![key.module, key.host, key.args_hash as i64, json],
        );
        trace!(module = key.module, host = key.host, "sqlite cache PUT");
    }

    fn invalidate_host(&self, host: &str) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute(
            "DELETE FROM module_cache WHERE host = ?1",
            rusqlite::params![host],
        );
    }

    fn clear(&self) {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute("DELETE FROM module_cache", []);
    }

    fn len(&self) -> usize {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM module_cache", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap_or(0) as usize
    }
}

// ---------------------------------------------------------------------------
// CACHEABLE_MODULES — the set of modules whose results may be cached
// ---------------------------------------------------------------------------

/// Modules that produce deterministic, read-only results and are safe to cache.
///
/// Only `stat`, `setup`, `find`, and `git` (when not updating) are included by
/// default.  Mutating modules (`shell`, `command`, `file`, `copy`, …) are
/// deliberately excluded.
pub const CACHEABLE_MODULES: &[&str] = &["stat", "setup", "gather_facts", "find"];

fn is_cacheable(module: &str) -> bool {
    CACHEABLE_MODULES.contains(&module)
}

// ---------------------------------------------------------------------------
// CachingModuleRegistry
// ---------------------------------------------------------------------------

/// A [`ModuleRegistry`]-compatible wrapper that transparently caches results
/// for cacheable modules.
///
/// # Cache invalidation
///
/// The cache is **not** automatically invalidated between plays.  Call
/// [`invalidate_host`](Self::invalidate_host) to drop stale facts for a host
/// (e.g. after `setup` re-runs).
///
/// # Example
///
/// ```rust,no_run
/// use ansiblers_modules::{ModuleRegistry, CachingModuleRegistry, InMemoryCache};
/// use std::sync::Arc;
///
/// let inner = ModuleRegistry::with_defaults();
/// let cache = Arc::new(InMemoryCache::default());
/// let cached = CachingModuleRegistry::new(inner, cache);
/// ```
pub struct CachingModuleRegistry {
    inner: ModuleRegistry,
    cache: Arc<dyn ModuleResultCache>,
    /// Counter: total cache lookups.
    pub lookups: Mutex<u64>,
    /// Counter: cache hits.
    pub hits: Mutex<u64>,
}

impl CachingModuleRegistry {
    /// Wrap `inner` with the given `cache` backend.
    pub fn new(inner: ModuleRegistry, cache: Arc<dyn ModuleResultCache>) -> Self {
        Self {
            inner,
            cache,
            lookups: Mutex::new(0),
            hits: Mutex::new(0),
        }
    }

    /// Return the cache hit ratio (0.0–1.0).
    pub fn hit_ratio(&self) -> f64 {
        let total = *self.lookups.lock().unwrap() as f64;
        let hits = *self.hits.lock().unwrap() as f64;
        if total == 0.0 {
            0.0
        } else {
            hits / total
        }
    }

    /// Invoke a module, using the cache for idempotent modules.
    pub fn invoke(
        &self,
        module: &str,
        args: &ModuleArgs,
        host: &str,
        ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        if is_cacheable(module) {
            let key = CacheKey::new(module, host, args);
            *self.lookups.lock().unwrap() += 1;

            if let Some(cached) = self.cache.get(&key) {
                debug!(module, host, "cache hit — skipping module invocation");
                *self.hits.lock().unwrap() += 1;
                return Ok(cached);
            }

            let result = self.inner.invoke(module, args, host, ctx)?;
            self.cache.put(key, result.clone());
            return Ok(result);
        }

        self.inner.invoke(module, args, host, ctx)
    }

    /// Drop all cached results for `host`.
    pub fn invalidate_host(&self, host: &str) {
        self.cache.invalidate_host(host);
    }

    /// Drop all cached results.
    pub fn clear_cache(&self) {
        self.cache.clear();
    }

    /// Return the underlying registry (for registration of new modules).
    pub fn inner_mut(&mut self) -> &mut ModuleRegistry {
        &mut self.inner
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ansiblers_core::{Inventory, TaskStatus};
    use rstest::rstest;
    use std::sync::Arc;

    fn make_args(module: &str) -> ModuleArgs {
        let mut args = HashMap::new();
        args.insert(
            "_raw_params".to_string(),
            ansiblers_core::Value::String("/tmp".to_string()),
        );
        ModuleArgs {
            args,
            task_name: Some(module.to_string()),
        }
    }

    fn make_ctx() -> ExecutionContext {
        ExecutionContext::new(Arc::new(Inventory::default()), HashMap::new())
    }

    // -----------------------------------------------------------------------
    // CacheKey
    // -----------------------------------------------------------------------

    #[test]
    fn cache_key_same_args_same_hash() {
        let args1 = make_args("stat");
        let args2 = make_args("stat");
        assert_eq!(
            CacheKey::new("stat", "host1", &args1),
            CacheKey::new("stat", "host1", &args2)
        );
    }

    #[test]
    fn cache_key_different_hosts_differ() {
        let args = make_args("stat");
        assert_ne!(
            CacheKey::new("stat", "host1", &args),
            CacheKey::new("stat", "host2", &args)
        );
    }

    #[test]
    fn cache_key_different_modules_differ() {
        let args = make_args("stat");
        assert_ne!(
            CacheKey::new("stat", "h", &args),
            CacheKey::new("find", "h", &args)
        );
    }

    // -----------------------------------------------------------------------
    // InMemoryCache
    // -----------------------------------------------------------------------

    #[test]
    fn in_memory_cache_miss_then_hit() {
        let cache = InMemoryCache::default();
        let key = CacheKey::new("stat", "h", &make_args("stat"));
        assert!(cache.get(&key).is_none());

        let result = TaskResult::ok("h");
        cache.put(key.clone(), result.clone());
        assert!(cache.get(&key).is_some());
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn in_memory_cache_invalidate_host() {
        let cache = InMemoryCache::default();
        let k1 = CacheKey::new("stat", "host1", &make_args("stat"));
        let k2 = CacheKey::new("stat", "host2", &make_args("stat"));
        cache.put(k1.clone(), TaskResult::ok("host1"));
        cache.put(k2.clone(), TaskResult::ok("host2"));
        assert_eq!(cache.len(), 2);

        cache.invalidate_host("host1");
        assert_eq!(cache.len(), 1);
        assert!(cache.get(&k1).is_none());
        assert!(cache.get(&k2).is_some());
    }

    #[test]
    fn in_memory_cache_clear() {
        let cache = InMemoryCache::default();
        for i in 0..5 {
            let k = CacheKey {
                module: "stat".to_string(),
                host: format!("h{i}"),
                args_hash: i,
            };
            cache.put(k, TaskResult::ok(format!("h{i}")));
        }
        assert_eq!(cache.len(), 5);
        cache.clear();
        assert!(cache.is_empty());
    }

    // -----------------------------------------------------------------------
    // SqliteCache
    // -----------------------------------------------------------------------

    #[test]
    fn sqlite_cache_roundtrip() {
        let cache = SqliteCache::open(":memory:").unwrap();
        let key = CacheKey::new("setup", "dbhost", &make_args("setup"));
        assert!(cache.get(&key).is_none());

        let result = TaskResult::ok("dbhost");
        cache.put(key.clone(), result);
        let retrieved = cache.get(&key).unwrap();
        assert_eq!(retrieved.status, TaskStatus::Ok);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn sqlite_cache_invalidate_host() {
        let cache = SqliteCache::open(":memory:").unwrap();
        let k1 = CacheKey::new("stat", "s1", &make_args("stat"));
        let k2 = CacheKey::new("stat", "s2", &make_args("stat"));
        cache.put(k1.clone(), TaskResult::ok("s1"));
        cache.put(k2.clone(), TaskResult::ok("s2"));

        cache.invalidate_host("s1");
        assert!(cache.get(&k1).is_none());
        assert!(cache.get(&k2).is_some());
    }

    // -----------------------------------------------------------------------
    // CachingModuleRegistry
    // -----------------------------------------------------------------------

    #[test]
    fn caching_registry_stat_is_cached() {
        let inner = ModuleRegistry::with_defaults();
        let cache: Arc<dyn ModuleResultCache> = Arc::new(InMemoryCache::default());
        let registry = CachingModuleRegistry::new(inner, Arc::clone(&cache));
        let mut ctx = make_ctx();

        let args = ModuleArgs {
            args: {
                let mut m = HashMap::new();
                m.insert(
                    "path".to_string(),
                    ansiblers_core::Value::String("/tmp".to_string()),
                );
                m
            },
            task_name: Some("stat".to_string()),
        };

        // First call — should execute the module and store in cache.
        let _r1 = registry
            .invoke("stat", &args, "localhost", &mut ctx)
            .unwrap();
        assert_eq!(cache.len(), 1);
        assert_eq!(*registry.lookups.lock().unwrap(), 1);
        assert_eq!(*registry.hits.lock().unwrap(), 0);

        // Second call — should hit the cache.
        let _r2 = registry
            .invoke("stat", &args, "localhost", &mut ctx)
            .unwrap();
        assert_eq!(cache.len(), 1); // still 1 entry
        assert_eq!(*registry.lookups.lock().unwrap(), 2);
        assert_eq!(*registry.hits.lock().unwrap(), 1);
        assert_eq!(registry.hit_ratio(), 0.5);
    }

    #[rstest]
    #[case("shell")]
    #[case("command")]
    #[case("file")]
    #[case("copy")]
    #[case("debug")]
    fn non_cacheable_modules_bypass_cache(#[case] module: &str) {
        assert!(!is_cacheable(module));
    }

    #[rstest]
    #[case("stat")]
    #[case("setup")]
    #[case("gather_facts")]
    #[case("find")]
    fn cacheable_modules_are_recognised(#[case] module: &str) {
        assert!(is_cacheable(module));
    }

    #[test]
    fn caching_registry_invalidate_clears_host_entries() {
        let inner = ModuleRegistry::with_defaults();
        let cache: Arc<dyn ModuleResultCache> = Arc::new(InMemoryCache::default());
        let registry = CachingModuleRegistry::new(inner, Arc::clone(&cache));
        let mut ctx = make_ctx();

        let args = ModuleArgs {
            args: {
                let mut m = HashMap::new();
                m.insert(
                    "path".to_string(),
                    ansiblers_core::Value::String("/tmp".to_string()),
                );
                m
            },
            task_name: None,
        };

        registry.invoke("stat", &args, "target1", &mut ctx).unwrap();
        registry.invoke("stat", &args, "target2", &mut ctx).unwrap();
        assert_eq!(cache.len(), 2);

        registry.invalidate_host("target1");
        assert_eq!(cache.len(), 1);
    }
}
