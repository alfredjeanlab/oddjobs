// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! ID generation abstractions

/// Returns a string slice truncated to at most `n` characters.
pub fn short(s: &str, n: usize) -> &str {
    if s.len() <= n {
        s
    } else {
        &s[..n]
    }
}

/// Define a newtype ID wrapper around `SmolStr` with a type prefix.
///
/// Generates `new()` for random ID generation, `from_string()` for parsing,
/// `as_str()`, `suffix()`, `short()`, `Display`, `From<String>`, `From<&str>`,
/// `PartialEq<str>`, `PartialEq<&str>`, `Borrow<str>`, and `Deref` implementations.
///
/// The ID format is `{prefix}{nanoid}` where:
/// - `prefix`: 3-4 character type indicator (e.g., "job-", "agt-")
/// - `nanoid`: 19 character random ID
/// - Total: 23 characters (exactly fits SmolStr inline capacity)
///
/// ```ignore
/// define_id! {
///     /// Doc comment for the ID type.
///     pub struct JobId("job-");
/// }
/// ```
#[macro_export]
macro_rules! define_id {
    (
        $(#[$meta:meta])*
        pub struct $name:ident($prefix:literal);
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub smol_str::SmolStr);

        impl $name {
            pub const PREFIX: &'static str = $prefix;

            /// Generate a new random ID with the type prefix
            pub fn new() -> Self {
                Self(smol_str::SmolStr::new(&format!(
                    "{}{}",
                    Self::PREFIX,
                    nanoid::nanoid!(19)
                )))
            }

            /// Create ID from existing string (for parsing/deserialization)
            pub fn from_string(id: impl Into<smol_str::SmolStr>) -> Self {
                Self(id.into())
            }

            // NOTE(compat): macro-generated method not used by all ID types
            #[allow(dead_code)]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Get the ID suffix (without prefix)
            pub fn suffix(&self) -> &str {
                self.0.strip_prefix(Self::PREFIX).unwrap_or(&self.0)
            }

            /// Returns a string slice of the suffix truncated to at most `n` characters.
            pub fn short(&self, n: usize) -> &str {
                let suffix = self.suffix();
                let end = std::cmp::min(n, suffix.len());
                &suffix[..end]
            }

            /// Returns true if the ID is an empty string.
            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self::from_string(s)
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self::from_string(s)
            }
        }

        impl From<&String> for $name {
            fn from(s: &String) -> Self {
                Self::from_string(s.clone())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
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

        impl std::ops::Deref for $name {
            type Target = str;

            fn deref(&self) -> &str {
                &self.0
            }
        }
    };
}

#[cfg(test)]
#[path = "id_tests.rs"]
mod tests;
