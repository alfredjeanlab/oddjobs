// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! ID generation abstractions

/// Maximum byte length for an inline ID.
///
/// All generated IDs are exactly 23 bytes (4-char prefix + 19-char nanoid).
/// `from_string` accepts shorter IDs but debug-asserts they fit.
pub const ID_MAX_LEN: usize = 23;

/// Returns a string slice truncated to at most `n` characters.
pub fn short(s: &str, n: usize) -> &str {
    if s.len() <= n {
        s
    } else {
        &s[..n]
    }
}

/// Fixed-size inline ID buffer. Always ≤ 23 ASCII bytes, `Copy`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct IdBuf {
    len: u8,
    buf: [u8; ID_MAX_LEN],
}

impl std::hash::Hash for IdBuf {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Hash only the active bytes so it matches str::hash,
        // which is required for Borrow<str> HashMap lookups.
        self.as_str().hash(state);
    }
}

impl IdBuf {
    pub const fn empty() -> Self {
        Self { len: 0, buf: [0; ID_MAX_LEN] }
    }

    pub fn new(s: &str) -> Self {
        debug_assert!(
            s.len() <= ID_MAX_LEN,
            "ID exceeds {} bytes ({} bytes): {:?}",
            ID_MAX_LEN,
            s.len(),
            s,
        );
        let len = s.len().min(ID_MAX_LEN);
        let mut buf = [0u8; ID_MAX_LEN];
        buf[..len].copy_from_slice(&s.as_bytes()[..len]);
        Self { len: len as u8, buf }
    }

    pub fn as_str(&self) -> &str {
        // Invariant: only constructed from &str, always valid UTF-8.
        // Panics if invariant is violated (should never happen).
        match std::str::from_utf8(&self.buf[..self.len as usize]) {
            Ok(s) => s,
            Err(_) => unreachable!("IdBuf constructed from non-UTF-8"),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl std::borrow::Borrow<str> for IdBuf {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Debug for IdBuf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

impl std::fmt::Display for IdBuf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl serde::Serialize for IdBuf {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for IdBuf {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = <&str>::deserialize(deserializer)?;
        if s.len() > ID_MAX_LEN {
            return Err(serde::de::Error::custom(format!(
                "ID exceeds {} bytes: {:?}",
                ID_MAX_LEN, s
            )));
        }
        Ok(IdBuf::new(s))
    }
}

/// Define a newtype ID wrapper around [`IdBuf`] with a type prefix.
///
/// Generates `new()` for random ID generation, `from_string()` for parsing,
/// `as_str()`, `suffix()`, `short()`, `Display`, `From<String>`, `From<&str>`,
/// `PartialEq<str>`, `PartialEq<&str>`, `Borrow<str>`, and `Deref` implementations.
///
/// The ID format is `{prefix}{nanoid}` where:
/// - `prefix`: 4 character type indicator (e.g., "job-", "agt-")
/// - `nanoid`: 19 character random ID
/// - Total: 23 characters (exactly fits [`IdBuf`] capacity)
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
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub $crate::id::IdBuf);

        impl $name {
            pub const PREFIX: &'static str = $prefix;

            /// Generate a new random ID with the type prefix
            pub fn new() -> Self {
                Self($crate::id::IdBuf::new(&format!(
                    "{}{}",
                    Self::PREFIX,
                    nanoid::nanoid!(19)
                )))
            }

            /// Create ID from existing string (for parsing/deserialization)
            pub fn from_string(id: impl AsRef<str>) -> Self {
                Self($crate::id::IdBuf::new(id.as_ref()))
            }

            // NOTE(compat): macro-generated method not used by all ID types
            #[allow(dead_code)]
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }

            /// Get the ID suffix (without prefix)
            pub fn suffix(&self) -> &str {
                self.0.as_str().strip_prefix(Self::PREFIX).unwrap_or(self.0.as_str())
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
                write!(f, "{}", self.0.as_str())
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
                Self::from_string(s)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.0.as_str()
            }
        }

        impl PartialEq<str> for $name {
            fn eq(&self, other: &str) -> bool {
                self.0.as_str() == other
            }
        }

        impl PartialEq<&str> for $name {
            fn eq(&self, other: &&str) -> bool {
                self.0.as_str() == *other
            }
        }

        impl std::borrow::Borrow<str> for $name {
            fn borrow(&self) -> &str {
                self.0.as_str()
            }
        }

        impl std::ops::Deref for $name {
            type Target = str;

            fn deref(&self) -> &str {
                self.0.as_str()
            }
        }
    };
}

#[cfg(test)]
#[path = "id_tests.rs"]
mod tests;
