// Copyright 2017 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use std::env;
use std::ffi::OsString;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

// Ensures we don't race on the env vars. This is only also important in tests,
// where we have multiple copies of the RLS running in the same process.
lazy_static! {
    static ref ENV_LOCK: Arc<EnvironmentLock> = Arc::new(EnvironmentLock::new());
}

/// An RAII helper to set and reset the env vars.
/// Requires supplying an external lock guard to guarantee env var consistency across multiple threads.
pub struct Environment<'a> {
    old_vars: HashMap<String, Option<OsString>>,
    _guard: MutexGuard<'a, ()>,
}

impl<'a> Environment<'a> {
    pub fn push_with_lock(envs: &HashMap<String, Option<OsString>>, lock: MutexGuard<'a, ()>) -> Environment<'a> {
        let mut result = Environment {
            old_vars: HashMap::new(),
            _guard: lock,
        };

        for (k, v) in envs {
            result.push_var(k, v);
        }
        result
    }

    pub fn push_var(&mut self, key: &str, value: &Option<OsString>) {
        self.old_vars.insert(key.to_owned(), env::var_os(key));
        match *value {
            Some(ref v) => env::set_var(key, v),
            None => env::remove_var(key),
        }
    }
}

impl<'a> Drop for Environment<'a> {
    fn drop(&mut self) {
        for (k, v) in &self.old_vars {
            match *v {
                Some(ref v) => env::set_var(k, v),
                None => env::remove_var(k),
            }
        }
    }
}

/// Implements a double mutex with a not-so-strict lock order guarantee, that can be used to guard
/// environment variables and guarantee consistency across multiple threads. Since environment
/// is a global, shared resource with a static lifetime, the `EnvironmentLock` is effectively
/// a singleton - a global, static instance.
///
/// It uses two locks instead of one, because RLS, while executing a Cargo build routine, not only
/// needs to guarantee consistent env vars across the Cargo invocation, but also, while holding it,
/// it needs to provide a more fine-grained way to synchronize env vars across different inner
/// compiler invocations, for which Cargo sets specific env vars.
/// To enforce proper env var guarantees, regular rustc and Cargo build routines must first acquire
/// the first, outer lock. Only then, if needed, nested rustc calls inside Cargo routine can
/// acquire the second, inner lock.
/// We're using linked Cargo and rustc to optimize serialization and IPC overhead, which means
/// we don't spawn different processes, hence why we share a single environment and need to provide
/// synchronized access to it.
pub struct EnvironmentLock {
    outer: Mutex<()>,
    inner: Mutex<()>,
}

/// Helper type that provides a unified way to access both outer and inner types of
/// `EnvironmentLock` lock interfaces.
pub enum EnvironmentLockFacade {
    Outer(Arc<EnvironmentLock>),
    Inner(InnerLock),
}

impl<'a> EnvironmentLockFacade {
    /// Retrieves access to an underlying, corresponding `Mutex` lock of `EnvironmentLock` and
    /// additionally returns `InnerLock` if the underlying lock is an `OuterLock`.
    pub fn lock(&self) -> (MutexGuard<'a, ()>, Option<InnerLock>) {
        match *self {
            EnvironmentLockFacade::Outer(ref lock) => {
                let (guard, inner) = lock.lock();
                (guard, Some(inner))
            },
            EnvironmentLockFacade::Inner(ref lock) => (lock.lock(), None),
        }
    }
}

impl<'a> EnvironmentLock {
    fn new() -> EnvironmentLock {
        EnvironmentLock {
            outer: Mutex::new(()),
            inner: Mutex::new(()),
        }
    }

    /// Retrieves a pointer to the single, static instance of an `EnvironmentLock`.
    pub fn get() -> Arc<EnvironmentLock> { ENV_LOCK.clone() }


    /// Acquires the first, outer lock and additionally return `InnerLock` interface, through which
    /// user can access the second, inner lock. Does not enforce any guarantees regarding order of
    /// locking, since `InnerLock` can be copied outside 'a lifetime and locked there.
    pub fn lock(&self) -> (MutexGuard<'a, ()>, InnerLock) {
        (ENV_LOCK.outer.lock().unwrap(), InnerLock{ })
    }

    /// Constructs a corresponding `EnvironmentLockFacade` value, erasing specific type of the lock.
    pub fn as_facade(&self) -> EnvironmentLockFacade {
        EnvironmentLockFacade::Outer(ENV_LOCK.clone())
    }
}

/// Acts as an interface through which user can acquire the second, inner lock of `EnvironmentLock`.
pub struct InnerLock;

impl<'a> InnerLock {
    /// Acquires the second, inner environment lock.
    pub fn lock(&self) -> MutexGuard<'a, ()> {
        ENV_LOCK.inner.lock().unwrap()
    }

    /// Constructs a corresponding `EnvironmentLockFacade` value, erasing specific type of the lock.
    pub fn as_facade(&self) -> EnvironmentLockFacade {
        EnvironmentLockFacade::Inner(Self { })
    }
}
