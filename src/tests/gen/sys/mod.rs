// This file is @generated by portable-atomic-internal-codegen
// (gen function at tools/codegen/src/ffi.rs).
// It is not intended for manual editing.

#![cfg_attr(rustfmt, rustfmt::skip)]
#![allow(
    dead_code,
    non_camel_case_types,
    unreachable_pub,
    unused_imports,
    clippy::unreadable_literal,
)]
#[cfg(
    all(
        target_arch = "aarch64",
        target_os = "linux",
        target_env = "gnu",
        target_pointer_width = "64"
    )
)]
mod aarch64_linux_gnu;
#[cfg(
    all(
        target_arch = "aarch64",
        target_os = "linux",
        target_env = "gnu",
        target_pointer_width = "64"
    )
)]
pub(crate) use aarch64_linux_gnu::*;
#[cfg(
    all(
        target_arch = "aarch64",
        target_os = "linux",
        target_env = "gnu",
        target_pointer_width = "32"
    )
)]
mod aarch64_linux_gnu_ilp32;
#[cfg(
    all(
        target_arch = "aarch64",
        target_os = "linux",
        target_env = "gnu",
        target_pointer_width = "32"
    )
)]
pub(crate) use aarch64_linux_gnu_ilp32::*;
#[cfg(all(target_arch = "aarch64", target_os = "android"))]
mod aarch64_linux_android;
#[cfg(all(target_arch = "aarch64", target_os = "android"))]
pub(crate) use aarch64_linux_android::*;
#[cfg(all(target_arch = "aarch64", target_os = "openbsd"))]
mod aarch64_openbsd;
#[cfg(all(target_arch = "aarch64", target_os = "openbsd"))]
pub(crate) use aarch64_openbsd::*;
