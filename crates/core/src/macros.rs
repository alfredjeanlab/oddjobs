// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 Alfred Jean LLC

//! Declarative macros for reducing boilerplate.
//!
//! - [`simple_display!`] — `Display` impl mapping enum variants to string literals
//! - [`builder!`] — test builder struct with Default, setters, and `build()`
//! - [`setters!`] — setter methods for production builder/config structs

/// Generate a `Display` impl that maps enum variants to string literals.
///
/// Unit variants match directly; data-carrying variants use `(..)` to ignore fields.
///
/// ```ignore
/// crate::simple_display! {
///     MyEnum {
///         Foo => "foo",
///         Bar(..) => "bar",
///     }
/// }
/// ```
#[macro_export]
macro_rules! simple_display {
    ($enum:ty { $( $variant:ident $(( $($ignore:tt)* ))? => $str:expr ),+ $(,)? }) => {
        impl std::fmt::Display for $enum {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(match self {
                    $( Self::$variant $(( $($ignore)* ))? => $str, )+
                })
            }
        }
    };
}

/// Generate a test builder (struct + Default + setters + build).
///
/// All generated items are gated behind `#[cfg(any(test, feature = "test-support"))]`.
///
/// Field groups:
/// - `into { field: Type = default }` — setter uses `impl Into<Type>`
/// - `set { field: Type = default }` — setter takes `Type` directly
/// - `option { field: Type = default }` — builder field is `Option<Type>`,
///   setter wraps in `Some(v.into())`
/// - `computed { field: Type = expr }` — no builder field or setter;
///   value computed at build time
///
/// ```ignore
/// crate::builder! {
///     pub struct FooBuilder => Foo {
///         into { name: String = "test" }
///         set { count: u32 = 0 }
///         option { label: String = None }
///         computed { created_at: Instant = Instant::now() }
///     }
/// }
/// ```
#[macro_export]
macro_rules! builder {
    (
        pub struct $builder:ident => $target:ident {
            $(into {
                $( $into_field:ident : $into_ty:ty = $into_default:expr ),* $(,)?
            })?
            $(set {
                $( $set_field:ident : $set_ty:ty = $set_default:expr ),* $(,)?
            })?
            $(option {
                $( $opt_field:ident : $opt_ty:ty = $opt_default:expr ),* $(,)?
            })?
            $(computed {
                $( $comp_field:ident : $comp_ty:ty = $comp_expr:expr ),* $(,)?
            })?
        }
    ) => {
        #[cfg(any(test, feature = "test-support"))]
        pub struct $builder {
            $($( $into_field: $into_ty, )*)?
            $($( $set_field: $set_ty, )*)?
            $($( $opt_field: Option<$opt_ty>, )*)?
        }

        #[cfg(any(test, feature = "test-support"))]
        impl Default for $builder {
            fn default() -> Self {
                Self {
                    $($( $into_field: $into_default.into(), )*)?
                    $($( $set_field: $set_default, )*)?
                    $($( $opt_field: $opt_default, )*)?
                }
            }
        }

        #[cfg(any(test, feature = "test-support"))]
        impl $builder {
            $($(
                pub fn $into_field(mut self, v: impl Into<$into_ty>) -> Self {
                    self.$into_field = v.into();
                    self
                }
            )*)?

            $($(
                pub fn $set_field(mut self, v: $set_ty) -> Self {
                    self.$set_field = v;
                    self
                }
            )*)?

            $($(
                pub fn $opt_field(mut self, v: impl Into<$opt_ty>) -> Self {
                    self.$opt_field = Some(v.into());
                    self
                }
            )*)?

            pub fn build(self) -> $target {
                $target {
                    $($( $into_field: self.$into_field, )*)?
                    $($( $set_field: self.$set_field, )*)?
                    $($( $opt_field: self.$opt_field, )*)?
                    $($( $comp_field: $comp_expr, )*)?
                }
            }
        }

        #[cfg(any(test, feature = "test-support"))]
        impl $target {
            /// Create a builder with test defaults.
            pub fn builder() -> $builder {
                $builder::default()
            }
        }
    };
}

/// Generate setter methods inside an existing `impl` block.
///
/// Field groups work the same as [`builder!`] but only generate setter methods.
///
/// ```ignore
/// impl MyConfig {
///     oj_core::setters! {
///         into { name: String }
///         set { count: u32 }
///         option { label: String }
///     }
/// }
/// ```
#[macro_export]
macro_rules! setters {
    (
        $(into {
            $( $into_field:ident : $into_ty:ty ),* $(,)?
        })?
        $(set {
            $( $set_field:ident : $set_ty:ty ),* $(,)?
        })?
        $(option {
            $( $opt_field:ident : $opt_ty:ty ),* $(,)?
        })?
    ) => {
        $($(
            pub fn $into_field(mut self, v: impl Into<$into_ty>) -> Self {
                self.$into_field = v.into();
                self
            }
        )*)?

        $($(
            pub fn $set_field(mut self, v: $set_ty) -> Self {
                self.$set_field = v;
                self
            }
        )*)?

        $($(
            pub fn $opt_field(mut self, v: impl Into<$opt_ty>) -> Self {
                self.$opt_field = Some(v.into());
                self
            }
        )*)?
    };
}
