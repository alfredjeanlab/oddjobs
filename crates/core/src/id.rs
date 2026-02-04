// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! ID generation abstractions

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Trait for truncating identifiers to a short prefix.
pub trait ShortId {
    /// Returns a string slice truncated to at most `n` characters.
    fn short(&self, n: usize) -> &str;
}

impl ShortId for str {
    fn short(&self, n: usize) -> &str {
        if self.len() <= n {
            self
        } else {
            &self[..n]
        }
    }
}

/// Define a newtype ID wrapper around `String`.
///
/// Generates `new()`, `as_str()`, `short()`, `Display`, `From<String>`, `From<&str>`,
/// `PartialEq<str>`, `PartialEq<&str>`, and `Borrow<str>` implementations.
///
/// ```ignore
/// define_id! {
///     /// Doc comment for the ID type.
///     pub struct MyId;
/// }
///
/// // With extra derives (e.g. Default):
/// define_id! {
///     #[derive(Default)]
///     pub struct MyDefaultId;
/// }
/// ```
#[macro_export]
macro_rules! define_id {
    (
        $(#[$meta:meta])*
        pub struct $name:ident;
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
        pub struct $name(pub String);

        impl $name {
            pub fn new(id: impl Into<String>) -> Self {
                Self(id.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Returns a string slice truncated to at most `n` characters.
            pub fn short(&self, n: usize) -> &str {
                if self.0.len() <= n {
                    &self.0
                } else {
                    &self.0[..n]
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_string())
            }
        }

        impl PartialEq<str> for $name {
            fn eq(&self, other: &str) -> bool {
                self.0 == other
            }
        }

        impl PartialEq<&str> for $name {
            fn eq(&self, other: &&str) -> bool {
                self.0 == *other
            }
        }

        impl std::borrow::Borrow<str> for $name {
            fn borrow(&self) -> &str {
                &self.0
            }
        }
    };
}

/// Generates unique identifiers
pub trait IdGen: Clone + Send + Sync {
    fn next(&self) -> String;
}

/// UUID-based ID generator for production use
#[derive(Clone, Default)]
pub struct UuidIdGen;

impl IdGen for UuidIdGen {
    fn next(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }
}

/// Sequential ID generator for testing
#[derive(Clone)]
pub struct SequentialIdGen {
    prefix: String,
    counter: Arc<AtomicU64>,
}

impl SequentialIdGen {
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            counter: Arc::new(AtomicU64::new(1)),
        }
    }
}

impl Default for SequentialIdGen {
    fn default() -> Self {
        Self::new("id")
    }
}

impl IdGen for SequentialIdGen {
    fn next(&self) -> String {
        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        format!("{}-{}", self.prefix, n)
    }
}

#[cfg(test)]
#[path = "id_tests.rs"]
mod tests;
