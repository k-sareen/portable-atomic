// Atomic{I,U}128 implementation for x86_64 using CMPXCHG16B (DWCAS).
//
// Refs:
// - x86 and amd64 instruction reference https://www.felixcloutier.com/x86
// - atomic-maybe-uninit https://github.com/taiki-e/atomic-maybe-uninit
//
// Generated asm:
// - x86_64 (+cmpxchg16b) https://godbolt.org/z/44xdG776a

include!("macros.rs");

#[cfg(any(
    test,
    not(any(target_feature = "cmpxchg16b", portable_atomic_target_feature = "cmpxchg16b")),
))]
#[path = "../fallback/outline_atomics.rs"]
mod fallback;

#[path = "detect/x86_64.rs"]
mod detect;

#[cfg(not(portable_atomic_no_asm))]
use core::arch::asm;
use core::sync::atomic::Ordering;

#[allow(unused_macros)]
#[cfg(target_pointer_width = "32")]
macro_rules! ptr_modifier {
    () => {
        ":e"
    };
}
#[allow(unused_macros)]
#[cfg(target_pointer_width = "64")]
macro_rules! ptr_modifier {
    () => {
        ""
    };
}

/// A 128-bit value represented as a pair of 64-bit values.
// This type is #[repr(C)], both fields have the same in-memory representation
// and are plain old datatypes, so access to the fields is always safe.
#[derive(Clone, Copy)]
#[repr(C)]
union U128 {
    whole: u128,
    pair: Pair,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct Pair {
    lo: u64,
    hi: u64,
}

#[cfg_attr(
    not(any(target_feature = "cmpxchg16b", portable_atomic_target_feature = "cmpxchg16b")),
    target_feature(enable = "cmpxchg16b")
)]
#[cfg_attr(
    any(target_feature = "cmpxchg16b", portable_atomic_target_feature = "cmpxchg16b"),
    inline
)]
#[cfg_attr(
    not(any(target_feature = "cmpxchg16b", portable_atomic_target_feature = "cmpxchg16b")),
    inline(never)
)]
unsafe fn _cmpxchg16b(
    dst: *mut u128,
    old: u128,
    new: u128,
    success: Ordering,
    failure: Ordering,
) -> (u128, bool) {
    debug_assert!(dst as usize % 16 == 0);

    // Miri and Sanitizer do not support inline assembly.
    #[cfg(any(miri, portable_atomic_sanitize_thread))]
    // SAFETY: the caller must guarantee that `dst` is valid for both writes and
    // reads, 16-byte aligned (required by CMPXCHG16B), that there are no
    // concurrent non-atomic operations, and that the CPU supports CMPXCHG16B.
    unsafe {
        let res = core::arch::x86_64::cmpxchg16b(dst, old, new, success, failure);
        (res, res == old)
    }
    #[cfg(not(any(miri, portable_atomic_sanitize_thread)))]
    // SAFETY: the caller must guarantee that `dst` is valid for both writes and
    // reads, 16-byte aligned (required by CMPXCHG16B), that there are no
    // concurrent non-atomic operations, and that the CPU supports CMPXCHG16B.
    //
    // If the value at `dst` (destination operand) and rdx:rax are equal, the
    // 128-bit value in rcx:rbx is stored in the `dst`, otherwise the value at
    // `dst` is loaded to rdx:rax.
    //
    // The ZF flag is set if the value at `dst` and rdx:rax are equal,
    // otherwise it is cleared. Other flags are unaffected.
    //
    // Refs: https://www.felixcloutier.com/x86/cmpxchg8b:cmpxchg16b
    unsafe {
        // cmpxchg16b is always SeqCst.
        let _ = (success, failure);
        let r: u8;
        let old = U128 { whole: old };
        let new = U128 { whole: new };
        let (prev_lo, prev_hi);
        macro_rules! cmpxchg16b {
            ($rdi:tt) => {
                asm!(
                    // rbx is reserved by LLVM
                    "xchg {rbx_tmp}, rbx",
                    concat!("lock cmpxchg16b xmmword ptr [", $rdi, "]"),
                    "sete r8b",
                    // restore rbx
                    "mov rbx, {rbx_tmp}",
                    rbx_tmp = inout(reg) new.pair.lo => _,
                    in("rcx") new.pair.hi,
                    inout("rax") old.pair.lo => prev_lo,
                    inout("rdx") old.pair.hi => prev_hi,
                    in($rdi) dst,
                    out("r8b") r,
                    // Do not use `preserves_flags` because CMPXCHG16B modifies the ZF flag.
                    options(nostack),
                )
            };
        }
        #[cfg(target_pointer_width = "32")]
        cmpxchg16b!("edi");
        #[cfg(target_pointer_width = "64")]
        cmpxchg16b!("rdi");
        (U128 { pair: Pair { lo: prev_lo, hi: prev_hi } }.whole, r != 0)
    }
}

// 128-bit atomic load by two 64-bit atomic loads.
//
// See atomic_update for details.
#[cfg(any(
    test,
    not(any(target_feature = "cmpxchg16b", portable_atomic_target_feature = "cmpxchg16b")),
    any(miri, portable_atomic_sanitize_thread),
))]
#[inline]
unsafe fn byte_wise_atomic_load(src: *mut u128) -> u128 {
    debug_assert!(src as usize % 16 == 0);

    // Miri and Sanitizer do not support inline assembly.
    #[cfg(any(miri, portable_atomic_sanitize_thread))]
    // SAFETY: the caller must uphold the safety contract.
    unsafe {
        atomic_load(src, Ordering::Relaxed)
    }
    #[cfg(not(any(miri, portable_atomic_sanitize_thread)))]
    // SAFETY: the caller must uphold the safety contract.
    unsafe {
        let (prev_lo, prev_hi);
        asm!(
            concat!("mov {prev_lo}, qword ptr [{src", ptr_modifier!(), "}]"),
            concat!("mov {prev_hi}, qword ptr [{src", ptr_modifier!(), "} + 8]"),
            src = in(reg) src,
            prev_lo = out(reg) prev_lo,
            prev_hi = out(reg) prev_hi,
            options(nostack, preserves_flags, readonly),
        );
        U128 { pair: Pair { lo: prev_lo, hi: prev_hi } }.whole
    }
}

// VMOVDQA is atomic on Intel and AMD CPUs with AVX.
// See https://gcc.gnu.org/bugzilla//show_bug.cgi?id=104688 for details.
//
// Refs: https://www.felixcloutier.com/x86/movdqa:vmovdqa32:vmovdqa64
//
// Do not use vector registers on targets such as x86_64-unknown-none unless SSE is explicitly enabled.
// https://doc.rust-lang.org/nightly/rustc/platform-support/x86_64-unknown-none.html
#[cfg(target_feature = "sse")]
#[target_feature(enable = "avx")]
#[inline]
unsafe fn _atomic_load_vmovdqa(src: *mut u128, _order: Ordering) -> u128 {
    debug_assert!(src as usize % 16 == 0);

    // SAFETY: the caller must uphold the safety contract.
    unsafe {
        let out: core::arch::x86_64::__m128;
        asm!(
            concat!("vmovdqa {out}, xmmword ptr [{src", ptr_modifier!(), "}]"),
            src = in(reg) src,
            out = out(xmm_reg) out,
            options(nostack, preserves_flags),
        );
        core::mem::transmute(out)
    }
}
#[cfg(target_feature = "sse")]
#[target_feature(enable = "avx")]
#[inline]
unsafe fn _atomic_store_vmovdqa(dst: *mut u128, val: u128, order: Ordering) {
    debug_assert!(dst as usize % 16 == 0);

    // SAFETY: the caller must uphold the safety contract.
    unsafe {
        let val: core::arch::x86_64::__m128 = core::mem::transmute(val);
        match order {
            // Relaxed and Release stores are equivalent.
            Ordering::Relaxed | Ordering::Release => {
                asm!(
                    concat!("vmovdqa xmmword ptr [{dst", ptr_modifier!(), "}], {val}"),
                    dst = in(reg) dst,
                    val = in(xmm_reg) val,
                    options(nostack, preserves_flags),
                );
            }
            Ordering::SeqCst => {
                asm!(
                    concat!("vmovdqa xmmword ptr [{dst", ptr_modifier!(), "}], {val}"),
                    "mfence",
                    dst = in(reg) dst,
                    val = in(xmm_reg) val,
                    options(nostack, preserves_flags),
                );
            }
            _ => unreachable!("{:?}", order),
        }
    }
}

#[inline]
unsafe fn atomic_load(src: *mut u128, order: Ordering) -> u128 {
    // Do not use vector registers on targets such as x86_64-unknown-none unless SSE is explicitly enabled.
    // https://doc.rust-lang.org/nightly/rustc/platform-support/x86_64-unknown-none.html
    // SGX doesn't support CPUID.
    // Miri and Sanitizer do not support inline assembly.
    #[cfg(any(
        not(target_feature = "sse"),
        portable_atomic_no_outline_atomics,
        target_env = "sgx",
        miri,
        portable_atomic_sanitize_thread,
    ))]
    // SAFETY: the caller must uphold the safety contract.
    unsafe {
        _atomic_load_cmpxchg16b(src, order)
    }
    #[cfg(not(any(
        not(target_feature = "sse"),
        portable_atomic_no_outline_atomics,
        target_env = "sgx",
        miri,
        portable_atomic_sanitize_thread,
    )))]
    // SAFETY: the caller must uphold the safety contract.
    unsafe {
        ifunc!(unsafe fn(src: *mut u128, order: Ordering) -> u128 {
            // Check CMPXCHG16B anyway to prevent mixing atomic and non-atomic access.
            let cpuid = detect::detect();
            if cpuid.has_cmpxchg16b() && cpuid.has_vmovdqa_atomic() {
                _atomic_load_vmovdqa
            } else {
                _atomic_load_cmpxchg16b
            }
        })
    }
}
#[inline]
unsafe fn _atomic_load_cmpxchg16b(src: *mut u128, order: Ordering) -> u128 {
    let fail_order = crate::utils::strongest_failure_ordering(order);
    // SAFETY: the caller must uphold the safety contract.
    unsafe {
        match atomic_compare_exchange(src, 0, 0, order, fail_order) {
            Ok(v) | Err(v) => v,
        }
    }
}

#[inline]
unsafe fn atomic_store(dst: *mut u128, val: u128, order: Ordering) {
    // Do not use vector registers on targets such as x86_64-unknown-none unless SSE is explicitly enabled.
    // https://doc.rust-lang.org/nightly/rustc/platform-support/x86_64-unknown-none.html
    // SGX doesn't support CPUID.
    // Miri and Sanitizer do not support inline assembly.
    #[cfg(any(
        not(target_feature = "sse"),
        portable_atomic_no_outline_atomics,
        target_env = "sgx",
        miri,
        portable_atomic_sanitize_thread,
    ))]
    // SAFETY: the caller must uphold the safety contract.
    unsafe {
        _atomic_store_cmpxchg16b(dst, val, order);
    }
    #[cfg(not(any(
        not(target_feature = "sse"),
        portable_atomic_no_outline_atomics,
        target_env = "sgx",
        miri,
        portable_atomic_sanitize_thread,
    )))]
    // SAFETY: the caller must uphold the safety contract.
    unsafe {
        match order {
            // Relaxed and Release stores are equivalent in all implementations
            // that may be called here (vmovdqa, asm-based cmpxchg16b, and fallback).
            // Due to cfg, core::arch's cmpxchg16b will never called here.
            Ordering::Relaxed | Ordering::Release => {
                ifunc!(unsafe fn(dst: *mut u128, val: u128) {
                    // Check CMPXCHG16B anyway to prevent mixing atomic and non-atomic access.
                    let cpuid = detect::detect();
                    if cpuid.has_cmpxchg16b() && cpuid.has_vmovdqa_atomic() {
                        _atomic_store_vmovdqa_relaxed
                    } else {
                        _atomic_store_cmpxchg16b_relaxed
                    }
                });
            }
            Ordering::SeqCst => {
                ifunc!(unsafe fn(dst: *mut u128, val: u128) {
                    // Check CMPXCHG16B anyway to prevent mixing atomic and non-atomic access.
                    let cpuid = detect::detect();
                    if cpuid.has_cmpxchg16b() && cpuid.has_vmovdqa_atomic() {
                        _atomic_store_vmovdqa_seqcst
                    } else {
                        _atomic_store_cmpxchg16b_seqcst
                    }
                });
            }
            _ => unreachable!("{:?}", order),
        }
    }
}
#[cfg(not(portable_atomic_no_outline_atomics))]
fn_alias! {
    #[cfg(target_feature = "sse")]
    #[target_feature(enable = "avx")]
    unsafe fn(dst: *mut u128, val: u128);
    _atomic_store_vmovdqa_relaxed = _atomic_store_vmovdqa(Ordering::Relaxed);
    _atomic_store_vmovdqa_seqcst = _atomic_store_vmovdqa(Ordering::SeqCst);
}
#[cfg(not(portable_atomic_no_outline_atomics))]
fn_alias! {
    unsafe fn(dst: *mut u128, val: u128);
    _atomic_store_cmpxchg16b_relaxed = _atomic_store_cmpxchg16b(Ordering::Relaxed);
    _atomic_store_cmpxchg16b_seqcst = _atomic_store_cmpxchg16b(Ordering::SeqCst);
}
#[inline]
unsafe fn _atomic_store_cmpxchg16b(dst: *mut u128, val: u128, order: Ordering) {
    // SAFETY: the caller must uphold the safety contract.
    unsafe {
        atomic_swap(dst, val, order);
    }
}

#[inline]
unsafe fn atomic_compare_exchange(
    dst: *mut u128,
    old: u128,
    new: u128,
    success: Ordering,
    failure: Ordering,
) -> Result<u128, u128> {
    let success = crate::utils::upgrade_success_ordering(success, failure);
    #[cfg(any(target_feature = "cmpxchg16b", portable_atomic_target_feature = "cmpxchg16b"))]
    // SAFETY: the caller must guarantee that `dst` is valid for both writes and
    // reads, 16-byte aligned, that there are no concurrent non-atomic operations,
    // and cfg guarantees that CMPXCHG16B is statically available.
    let (res, ok) = unsafe { _cmpxchg16b(dst, old, new, success, failure) };
    #[cfg(not(any(target_feature = "cmpxchg16b", portable_atomic_target_feature = "cmpxchg16b")))]
    let (res, ok) = {
        // SAFETY: the caller must guarantee that `dst` is valid for both writes and
        // reads, 16-byte aligned, and that there are no different kinds of concurrent accesses.
        unsafe {
            ifunc!(unsafe fn(
                dst: *mut u128, old: u128, new: u128, success: Ordering, failure: Ordering
            ) -> (u128, bool) {
                if detect::has_cmpxchg16b() {
                    _cmpxchg16b
                } else {
                    fallback::atomic_compare_exchange
                }
            })
        }
    };
    if ok {
        Ok(res)
    } else {
        Err(res)
    }
}

use atomic_compare_exchange as atomic_compare_exchange_weak;

#[cfg(any(
    not(any(target_feature = "cmpxchg16b", portable_atomic_target_feature = "cmpxchg16b")),
    any(miri, portable_atomic_sanitize_thread),
))]
#[inline(always)]
unsafe fn atomic_update<F>(dst: *mut u128, order: Ordering, mut f: F) -> u128
where
    F: FnMut(u128) -> u128,
{
    // SAFETY: the caller must uphold the safety contract.
    unsafe {
        // This is based on the code generated for the first load in DW RMWs by LLVM,
        // but it is interesting that they generate code that does mixed-sized atomic access.
        //
        // This is not single-copy atomic reads, but this is ok because subsequent
        // CAS will check for consistency.
        //
        // byte_wise_atomic_load works the same way as seqlock's byte-wise atomic memcpy,
        // so it works well even when atomic_compare_exchange_weak calls global lock-based fallback.
        //
        // Note that the C++20 memory model does not allow mixed-sized atomic access,
        // so we must use inline assembly to implement byte_wise_atomic_load.
        // (i.e., byte-wise atomic based on the standard library's atomic types
        // cannot be used here). Since fallback's byte-wise atomic memcpy is per
        // 64-bit on x86_64 (even on x32 ABI), it's okay to use it together with this.
        let mut old = byte_wise_atomic_load(dst);
        loop {
            let next = f(old);
            // This is a private function and all instances of `f` only operate on the value
            // loaded, so there is no need to synchronize the first load/failed CAS.
            match atomic_compare_exchange_weak(dst, old, next, order, Ordering::Relaxed) {
                Ok(x) => return x,
                Err(x) => old = x,
            }
        }
    }
}

// Miri and Sanitizer do not support inline assembly.
#[cfg(not(any(
    not(any(target_feature = "cmpxchg16b", portable_atomic_target_feature = "cmpxchg16b")),
    any(miri, portable_atomic_sanitize_thread),
)))]
#[inline]
unsafe fn atomic_swap(dst: *mut u128, val: u128, order: Ordering) -> u128 {
    debug_assert!(dst as usize % 16 == 0);

    // SAFETY: the caller must guarantee that `dst` is valid for both writes and
    // reads, 16-byte aligned, and that there are no concurrent non-atomic operations.
    // cfg guarantees that the CPU supports CMPXCHG16B.
    //
    // See _cmpxchg16b for more.
    //
    // We could use atomic_update here, but using an inline assembly allows omitting
    // the storing/comparing of condition flags and reducing uses of xchg/mov to handle rbx.
    //
    // Do not use atomic_rmw_cas_3 because it needs extra MOV to implement swap.
    unsafe {
        // atomic swap is always SeqCst.
        let _ = order;
        let val = U128 { whole: val };
        let (mut prev_lo, mut prev_hi);
        macro_rules! cmpxchg16b {
            ($rdi:tt) => {
                asm!(
                    // rbx is reserved by LLVM
                    "xchg {rbx_tmp}, rbx",
                    // See atomic_update
                    concat!("mov rax, qword ptr [", $rdi, "]"),
                    concat!("mov rdx, qword ptr [", $rdi, " + 8]"),
                    "2:",
                        concat!("lock cmpxchg16b xmmword ptr [", $rdi, "]"),
                        "jne 2b",
                    // restore rbx
                    "mov rbx, {rbx_tmp}",
                    rbx_tmp = inout(reg) val.pair.lo => _,
                    in("rcx") val.pair.hi,
                    out("rax") prev_lo,
                    out("rdx") prev_hi,
                    in($rdi) dst,
                    // Do not use `preserves_flags` because CMPXCHG16B modifies the ZF flag.
                    options(nostack),
                )
            };
        }
        #[cfg(target_pointer_width = "32")]
        cmpxchg16b!("edi");
        #[cfg(target_pointer_width = "64")]
        cmpxchg16b!("rdi");
        U128 { pair: Pair { lo: prev_lo, hi: prev_hi } }.whole
    }
}

/// Atomic RMW by CAS loop (3 arguments)
/// `unsafe fn(dst: *mut u128, val: u128, order: Ordering) -> u128;`
///
/// `$op` can use the following registers:
/// - rsi/r8 pair: val argument (read-only for `$op`)
/// - rax/rdx pair: previous value loaded (read-only for `$op`)
/// - rbx/rcx pair: new value that will to stored
// We could use atomic_update here, but using an inline assembly allows omitting
// the storing/comparing of condition flags and reducing uses of xchg/mov to handle rbx.
#[rustfmt::skip] // buggy macro formatting
macro_rules! atomic_rmw_cas_3 {
    ($name:ident, $($op:tt)*) => {
        // Miri and Sanitizer do not support inline assembly.
        #[cfg(not(any(
            not(any(target_feature = "cmpxchg16b", portable_atomic_target_feature = "cmpxchg16b")),
            any(miri, portable_atomic_sanitize_thread),
        )))]
        #[inline]
        unsafe fn $name(dst: *mut u128, val: u128, _order: Ordering) -> u128 {
            debug_assert!(dst as usize % 16 == 0);
            // SAFETY: the caller must guarantee that `dst` is valid for both writes and
            // reads, 16-byte aligned, and that there are no concurrent non-atomic operations.
            // cfg guarantees that the CPU supports CMPXCHG16B.
            //
            // See _cmpxchg16b for more.
            unsafe {
                // atomic swap is always SeqCst.
                let val = U128 { whole: val };
                let (mut prev_lo, mut prev_hi);
                macro_rules! cmpxchg16b {
                    ($rdi:tt) => {
                        asm!(
                            // rbx is reserved by LLVM
                            "mov {rbx_tmp}, rbx",
                            // See atomic_update
                            concat!("mov rax, qword ptr [", $rdi, "]"),
                            concat!("mov rdx, qword ptr [", $rdi, " + 8]"),
                            "2:",
                                $($op)*
                                concat!("lock cmpxchg16b xmmword ptr [", $rdi, "]"),
                                "jne 2b",
                            // restore rbx
                            "mov rbx, {rbx_tmp}",
                            rbx_tmp = out(reg) _,
                            out("rcx") _,
                            out("rax") prev_lo,
                            out("rdx") prev_hi,
                            in($rdi) dst,
                            in("rsi") val.pair.lo,
                            in("r8") val.pair.hi,
                            // Do not use `preserves_flags` because CMPXCHG16B modifies the ZF flag.
                            options(nostack),
                        )
                    };
                }
                #[cfg(target_pointer_width = "32")]
                cmpxchg16b!("edi");
                #[cfg(target_pointer_width = "64")]
                cmpxchg16b!("rdi");
                U128 { pair: Pair { lo: prev_lo, hi: prev_hi } }.whole
            }
        }
    };
}
/// Atomic RMW by CAS loop (2 arguments)
/// `unsafe fn(dst: *mut u128, order: Ordering) -> u128;`
///
/// `$op` can use the following registers:
/// - rax/rdx pair: previous value loaded (read-only for `$op`)
/// - rbx/rcx pair: new value that will to stored
// We could use atomic_update here, but using an inline assembly allows omitting
// the storing/comparing of condition flags and reducing uses of xchg/mov to handle rbx.
#[rustfmt::skip] // buggy macro formatting
macro_rules! atomic_rmw_cas_2 {
    ($name:ident, $($op:tt)*) => {
        // Miri and Sanitizer do not support inline assembly.
        #[cfg(not(any(
            not(any(target_feature = "cmpxchg16b", portable_atomic_target_feature = "cmpxchg16b")),
            any(miri, portable_atomic_sanitize_thread),
        )))]
        #[inline]
        unsafe fn $name(dst: *mut u128, _order: Ordering) -> u128 {
            debug_assert!(dst as usize % 16 == 0);
            // SAFETY: the caller must guarantee that `dst` is valid for both writes and
            // reads, 16-byte aligned, and that there are no concurrent non-atomic operations.
            // cfg guarantees that the CPU supports CMPXCHG16B.
            //
            // See _cmpxchg16b for more.
            unsafe {
                // atomic swap is always SeqCst.
                let (mut prev_lo, mut prev_hi);
                macro_rules! cmpxchg16b {
                    ($rdi:tt) => {
                        asm!(
                            // rbx is reserved by LLVM
                            "mov {rbx_tmp}, rbx",
                            // See atomic_update
                            concat!("mov rax, qword ptr [", $rdi, "]"),
                            concat!("mov rdx, qword ptr [", $rdi, " + 8]"),
                            "2:",
                                $($op)*
                                concat!("lock cmpxchg16b xmmword ptr [", $rdi, "]"),
                                "jne 2b",
                            // restore rbx
                            "mov rbx, {rbx_tmp}",
                            rbx_tmp = out(reg) _,
                            out("rcx") _,
                            out("rax") prev_lo,
                            out("rdx") prev_hi,
                            in($rdi) dst,
                            // Do not use `preserves_flags` because CMPXCHG16B modifies the ZF flag.
                            options(nostack),
                        )
                    };
                }
                #[cfg(target_pointer_width = "32")]
                cmpxchg16b!("edi");
                #[cfg(target_pointer_width = "64")]
                cmpxchg16b!("rdi");
                U128 { pair: Pair { lo: prev_lo, hi: prev_hi } }.whole
            }
        }
    };
}

atomic_rmw_cas_3! {
    atomic_add,
    "mov rbx, rax",
    "add rbx, rsi",
    "mov rcx, rdx",
    "adc rcx, r8",
}
atomic_rmw_cas_3! {
    atomic_sub,
    "mov rbx, rax",
    "sub rbx, rsi",
    "mov rcx, rdx",
    "sbb rcx, r8",
}
atomic_rmw_cas_3! {
    atomic_and,
    "mov rbx, rax",
    "and rbx, rsi",
    "mov rcx, rdx",
    "and rcx, r8",
}
atomic_rmw_cas_3! {
    atomic_nand,
    "mov rbx, rax",
    "and rbx, rsi",
    "not rbx",
    "mov rcx, rdx",
    "and rcx, r8",
    "not rcx",
}
atomic_rmw_cas_3! {
    atomic_or,
    "mov rbx, rax",
    "or rbx, rsi",
    "mov rcx, rdx",
    "or rcx, r8",
}
atomic_rmw_cas_3! {
    atomic_xor,
    "mov rbx, rax",
    "xor rbx, rsi",
    "mov rcx, rdx",
    "xor rcx, r8",
}

atomic_rmw_cas_2! {
    atomic_not,
    "mov rbx, rax",
    "not rbx",
    "mov rcx, rdx",
    "not rcx",
}
atomic_rmw_cas_2! {
    atomic_neg,
    "mov rbx, rax",
    "neg rbx",
    "mov rcx, 0",
    "sbb rcx, rdx",
}

atomic_rmw_cas_3! {
    atomic_max,
    "cmp rsi, rax",
    "mov rcx, r8",
    "sbb rcx, rdx",
    "mov rcx, r8",
    "cmovl rcx, rdx",
    "mov rbx, rsi",
    "cmovl rbx, rax",
}
atomic_rmw_cas_3! {
    atomic_umax,
    "cmp rsi, rax",
    "mov rcx, r8",
    "sbb rcx, rdx",
    "mov rcx, r8",
    "cmovb rcx, rdx",
    "mov rbx, rsi",
    "cmovb rbx, rax",
}
atomic_rmw_cas_3! {
    atomic_min,
    "cmp rsi, rax",
    "mov rcx, r8",
    "sbb rcx, rdx",
    "mov rcx, r8",
    "cmovge rcx, rdx",
    "mov rbx, rsi",
    "cmovge rbx, rax",
}
atomic_rmw_cas_3! {
    atomic_umin,
    "cmp rsi, rax",
    "mov rcx, r8",
    "sbb rcx, rdx",
    "mov rcx, r8",
    "cmovae rcx, rdx",
    "mov rbx, rsi",
    "cmovae rbx, rax",
}

// Miri and Sanitizer do not support inline assembly.
#[cfg(any(
    not(any(target_feature = "cmpxchg16b", portable_atomic_target_feature = "cmpxchg16b")),
    any(miri, portable_atomic_sanitize_thread),
))]
atomic_rmw_by_atomic_update!();

#[inline]
fn is_lock_free() -> bool {
    detect::has_cmpxchg16b()
}
#[inline]
const fn is_always_lock_free() -> bool {
    cfg!(any(target_feature = "cmpxchg16b", portable_atomic_target_feature = "cmpxchg16b"))
}

atomic128!(AtomicI128, i128, atomic_max, atomic_min);
atomic128!(AtomicU128, u128, atomic_umax, atomic_umin);

#[allow(clippy::undocumented_unsafe_blocks, clippy::wildcard_imports)]
#[cfg(test)]
mod tests {
    use super::*;

    test_atomic_int!(i128);
    test_atomic_int!(u128);

    #[test]
    fn test() {
        // Miri doesn't support inline assembly used in is_x86_feature_detected
        #[cfg(not(miri))]
        {
            assert!(std::is_x86_feature_detected!("cmpxchg16b"));
        }
        assert!(AtomicI128::is_lock_free());
        assert!(AtomicU128::is_lock_free());
    }

    #[cfg(any(target_feature = "cmpxchg16b", portable_atomic_target_feature = "cmpxchg16b"))]
    mod quickcheck {
        use core::cell::UnsafeCell;

        use test_helper::Align16;

        use super::super::*;

        ::quickcheck::quickcheck! {
            fn test(x: u128, y: u128, z: u128) -> bool {
                // Miri doesn't support inline assembly used in is_x86_feature_detected
                #[cfg(not(miri))]
                {
                    assert!(std::is_x86_feature_detected!("cmpxchg16b"));
                }
                unsafe {
                    let a = Align16(UnsafeCell::new(x));
                    let (res, ok) = _cmpxchg16b(a.get(), y, z, Ordering::SeqCst, Ordering::SeqCst);
                    if x == y {
                        assert!(ok);
                        assert_eq!(res, x);
                        assert_eq!(*a.get(), z);
                    } else {
                        assert!(!ok);
                        assert_eq!(res, x);
                        assert_eq!(*a.get(), x);
                    }
                }
                true
            }
        }
    }
}

#[allow(clippy::undocumented_unsafe_blocks, clippy::wildcard_imports)]
#[cfg(test)]
mod tests_no_cmpxchg16b {
    use super::*;

    #[inline(never)]
    unsafe fn cmpxchg16b(
        dst: *mut u128,
        old: u128,
        new: u128,
        success: Ordering,
        failure: Ordering,
    ) -> (u128, bool) {
        unsafe { fallback::atomic_compare_exchange(dst, old, new, success, failure) }
    }
    #[inline]
    unsafe fn byte_wise_atomic_load(src: *mut u128) -> u128 {
        debug_assert!(src as usize % 16 == 0);

        // Miri and Sanitizer do not support inline assembly.
        #[cfg(any(miri, portable_atomic_sanitize_thread))]
        unsafe {
            atomic_load(src, Ordering::Relaxed)
        }
        #[cfg(not(any(miri, portable_atomic_sanitize_thread)))]
        unsafe {
            super::byte_wise_atomic_load(src)
        }
    }

    #[inline(never)]
    unsafe fn atomic_load(src: *mut u128, order: Ordering) -> u128 {
        let fail_order = crate::utils::strongest_failure_ordering(order);
        unsafe {
            match atomic_compare_exchange(src, 0, 0, order, fail_order) {
                Ok(v) | Err(v) => v,
            }
        }
    }

    #[inline(never)]
    unsafe fn atomic_store(dst: *mut u128, val: u128, order: Ordering) {
        unsafe {
            atomic_swap(dst, val, order);
        }
    }

    #[inline]
    unsafe fn atomic_compare_exchange(
        dst: *mut u128,
        old: u128,
        new: u128,
        success: Ordering,
        failure: Ordering,
    ) -> Result<u128, u128> {
        let success = crate::utils::upgrade_success_ordering(success, failure);
        let (res, ok) = unsafe { cmpxchg16b(dst, old, new, success, failure) };
        if ok {
            Ok(res)
        } else {
            Err(res)
        }
    }

    use atomic_compare_exchange as atomic_compare_exchange_weak;

    #[inline(always)]
    unsafe fn atomic_update<F>(dst: *mut u128, order: Ordering, mut f: F) -> u128
    where
        F: FnMut(u128) -> u128,
    {
        unsafe {
            let mut old = byte_wise_atomic_load(dst);
            loop {
                let next = f(old);
                match atomic_compare_exchange_weak(dst, old, next, order, Ordering::Relaxed) {
                    Ok(x) => return x,
                    Err(x) => old = x,
                }
            }
        }
    }

    atomic_rmw_by_atomic_update!();

    #[inline]
    const fn is_always_lock_free() -> bool {
        false
    }
    use is_always_lock_free as is_lock_free;

    atomic128!(AtomicI128, i128, atomic_max, atomic_min);
    atomic128!(AtomicU128, u128, atomic_umax, atomic_umin);

    // Do not put this in the nested tests module due to glob imports refer to super::super::Atomic*.
    test_atomic_int!(i128);
    test_atomic_int!(u128);
}
