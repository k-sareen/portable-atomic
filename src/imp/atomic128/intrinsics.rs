// Atomic{I,U}128 implementation without inline assembly.
//
// Note: This module is currently only enabled on Miri and ThreadSanitizer which
// do not support inline assembly.
//
// This uses `core::arch::x86_64::cmpxchg16b` on x86_64 and
// `core::intrinsics::atomic_*` on aarch64, powerpc64, and s390x.
//
// See README.md of this directory for performance comparison with the
// implementation with inline assembly.
//
// Note:
// - This currently always needs nightly compilers. On x86_64, the stabilization
//   of `core::arch::x86_64::cmpxchg16b` has been recently merged to stdarch:
//   https://github.com/rust-lang/stdarch/pull/1358
// - On powerpc64, this requires LLVM 15+ and pwr8+ (quadword-atomics LLVM target feature):
//   https://github.com/llvm/llvm-project/commit/549e118e93c666914a1045fde38a2cac33e1e445
// - On aarch64 big-endian, LLVM (as of 15) generates broken code.
//   (on cfg(miri)/cfg(sanitize) it is fine though)
// - On s390x, LLVM (as of 16) generates libcalls for operations other than load/store/cmpxchg:
//   https://godbolt.org/z/5a5T4hxMh
// - On powerpc64, LLVM (as of 16) doesn't support 128-bit atomic min/max:
//   https://godbolt.org/z/3rebKcbdf
//
// Refs: https://github.com/rust-lang/rust/blob/1.69.0/library/core/src/sync/atomic.rs

include!("macros.rs");

#[allow(dead_code)] // we only use compare_exchange
#[cfg(target_arch = "x86_64")]
#[cfg(not(target_feature = "cmpxchg16b"))]
#[path = "../fallback/outline_atomics.rs"]
mod fallback;

#[cfg(target_arch = "x86_64")]
#[cfg(not(target_feature = "cmpxchg16b"))]
#[path = "detect/x86_64.rs"]
mod detect;

use core::sync::atomic::Ordering;
#[cfg(not(target_arch = "x86_64"))]
use core::{
    intrinsics,
    sync::atomic::Ordering::{AcqRel, Acquire, Relaxed, Release, SeqCst},
};

#[inline]
unsafe fn atomic_load(src: *mut u128, order: Ordering) -> u128 {
    #[cfg(target_arch = "x86_64")]
    // SAFETY: the caller must uphold the safety contract.
    unsafe {
        let fail_order = crate::utils::strongest_failure_ordering(order);
        match atomic_compare_exchange(src, 0, 0, order, fail_order) {
            Ok(v) | Err(v) => v,
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    // SAFETY: the caller must uphold the safety contract.
    unsafe {
        match order {
            Acquire => intrinsics::atomic_load_acquire(src),
            Relaxed => intrinsics::atomic_load_relaxed(src),
            SeqCst => intrinsics::atomic_load_seqcst(src),
            _ => unreachable!("{:?}", order),
        }
    }
}

#[inline]
unsafe fn atomic_store(dst: *mut u128, val: u128, order: Ordering) {
    #[cfg(target_arch = "x86_64")]
    // SAFETY: the caller must uphold the safety contract.
    unsafe {
        atomic_swap(dst, val, order);
    }
    #[cfg(not(target_arch = "x86_64"))]
    // SAFETY: the caller must uphold the safety contract.
    unsafe {
        match order {
            Release => intrinsics::atomic_store_release(dst, val),
            Relaxed => intrinsics::atomic_store_relaxed(dst, val),
            SeqCst => intrinsics::atomic_store_seqcst(dst, val),
            _ => unreachable!("{:?}", order),
        }
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
    #[cfg(target_arch = "x86_64")]
    let (val, ok) = {
        #[cfg_attr(not(target_feature = "cmpxchg16b"), target_feature(enable = "cmpxchg16b"))]
        #[cfg_attr(target_feature = "cmpxchg16b", inline)]
        #[cfg_attr(not(target_feature = "cmpxchg16b"), inline(never))]
        unsafe fn cmpxchg16b(
            dst: *mut u128,
            old: u128,
            new: u128,
            success: Ordering,
            failure: Ordering,
        ) -> (u128, bool) {
            debug_assert!(dst as usize % 16 == 0);
            #[cfg(not(target_feature = "cmpxchg16b"))]
            {
                debug_assert!(detect::detect().has_cmpxchg16b());
            }
            // SAFETY: the caller must guarantee that `dst` is valid for both writes and
            // reads, 16-byte aligned (required by CMPXCHG16B), that there are no
            // concurrent non-atomic operations, and that the CPU supports CMPXCHG16B.
            let res = unsafe { core::arch::x86_64::cmpxchg16b(dst, old, new, success, failure) };
            (res, res == old)
        }
        let success = crate::utils::upgrade_success_ordering(success, failure);
        #[cfg(target_feature = "cmpxchg16b")]
        // SAFETY: the caller must guarantee that `dst` is valid for both writes and
        // reads, 16-byte aligned, that there are no concurrent non-atomic operations,
        // and cfg guarantees that CMPXCHG16B is available at compile-time.
        unsafe {
            cmpxchg16b(dst, old, new, success, failure)
        }
        #[cfg(not(target_feature = "cmpxchg16b"))]
        // SAFETY: the caller must guarantee that `dst` is valid for both writes and
        // reads, 16-byte aligned, and that there are no different kinds of concurrent accesses.
        unsafe {
            ifunc!(unsafe fn(
                dst: *mut u128, old: u128, new: u128, success: Ordering, failure: Ordering
            ) -> (u128, bool) {
                if detect::detect().has_cmpxchg16b() {
                    cmpxchg16b
                } else {
                    fallback::atomic_compare_exchange
                }
            })
        }
    };
    #[cfg(not(target_arch = "x86_64"))]
    // SAFETY: the caller must uphold the safety contract.
    let (val, ok) = unsafe {
        match (success, failure) {
            (Relaxed, Relaxed) => intrinsics::atomic_cxchg_relaxed_relaxed(dst, old, new),
            (Relaxed, Acquire) => intrinsics::atomic_cxchg_relaxed_acquire(dst, old, new),
            (Relaxed, SeqCst) => intrinsics::atomic_cxchg_relaxed_seqcst(dst, old, new),
            (Acquire, Relaxed) => intrinsics::atomic_cxchg_acquire_relaxed(dst, old, new),
            (Acquire, Acquire) => intrinsics::atomic_cxchg_acquire_acquire(dst, old, new),
            (Acquire, SeqCst) => intrinsics::atomic_cxchg_acquire_seqcst(dst, old, new),
            (Release, Relaxed) => intrinsics::atomic_cxchg_release_relaxed(dst, old, new),
            (Release, Acquire) => intrinsics::atomic_cxchg_release_acquire(dst, old, new),
            (Release, SeqCst) => intrinsics::atomic_cxchg_release_seqcst(dst, old, new),
            (AcqRel, Relaxed) => intrinsics::atomic_cxchg_acqrel_relaxed(dst, old, new),
            (AcqRel, Acquire) => intrinsics::atomic_cxchg_acqrel_acquire(dst, old, new),
            (AcqRel, SeqCst) => intrinsics::atomic_cxchg_acqrel_seqcst(dst, old, new),
            (SeqCst, Relaxed) => intrinsics::atomic_cxchg_seqcst_relaxed(dst, old, new),
            (SeqCst, Acquire) => intrinsics::atomic_cxchg_seqcst_acquire(dst, old, new),
            (SeqCst, SeqCst) => intrinsics::atomic_cxchg_seqcst_seqcst(dst, old, new),
            _ => unreachable!("{:?}, {:?}", success, failure),
        }
    };
    if ok {
        Ok(val)
    } else {
        Err(val)
    }
}

#[cfg(target_arch = "x86_64")]
use atomic_compare_exchange as atomic_compare_exchange_weak;
#[cfg(not(target_arch = "x86_64"))]
#[inline]
unsafe fn atomic_compare_exchange_weak(
    dst: *mut u128,
    old: u128,
    new: u128,
    success: Ordering,
    failure: Ordering,
) -> Result<u128, u128> {
    // SAFETY: the caller must uphold the safety contract.
    let (val, ok) = unsafe {
        match (success, failure) {
            (Relaxed, Relaxed) => intrinsics::atomic_cxchgweak_relaxed_relaxed(dst, old, new),
            (Relaxed, Acquire) => intrinsics::atomic_cxchgweak_relaxed_acquire(dst, old, new),
            (Relaxed, SeqCst) => intrinsics::atomic_cxchgweak_relaxed_seqcst(dst, old, new),
            (Acquire, Relaxed) => intrinsics::atomic_cxchgweak_acquire_relaxed(dst, old, new),
            (Acquire, Acquire) => intrinsics::atomic_cxchgweak_acquire_acquire(dst, old, new),
            (Acquire, SeqCst) => intrinsics::atomic_cxchgweak_acquire_seqcst(dst, old, new),
            (Release, Relaxed) => intrinsics::atomic_cxchgweak_release_relaxed(dst, old, new),
            (Release, Acquire) => intrinsics::atomic_cxchgweak_release_acquire(dst, old, new),
            (Release, SeqCst) => intrinsics::atomic_cxchgweak_release_seqcst(dst, old, new),
            (AcqRel, Relaxed) => intrinsics::atomic_cxchgweak_acqrel_relaxed(dst, old, new),
            (AcqRel, Acquire) => intrinsics::atomic_cxchgweak_acqrel_acquire(dst, old, new),
            (AcqRel, SeqCst) => intrinsics::atomic_cxchgweak_acqrel_seqcst(dst, old, new),
            (SeqCst, Relaxed) => intrinsics::atomic_cxchgweak_seqcst_relaxed(dst, old, new),
            (SeqCst, Acquire) => intrinsics::atomic_cxchgweak_seqcst_acquire(dst, old, new),
            (SeqCst, SeqCst) => intrinsics::atomic_cxchgweak_seqcst_seqcst(dst, old, new),
            _ => unreachable!("{:?}, {:?}", success, failure),
        }
    };
    if ok {
        Ok(val)
    } else {
        Err(val)
    }
}

#[inline(always)]
unsafe fn atomic_update<F>(dst: *mut u128, order: Ordering, mut f: F) -> u128
where
    F: FnMut(u128) -> u128,
{
    // SAFETY: the caller must uphold the safety contract.
    unsafe {
        // This is a private function and all instances of `f` only operate on the value
        // loaded, so there is no need to synchronize the first load/failed CAS.
        let mut old = atomic_load(dst, Ordering::Relaxed);
        loop {
            let next = f(old);
            match atomic_compare_exchange_weak(dst, old, next, order, Ordering::Relaxed) {
                Ok(x) => return x,
                Err(x) => old = x,
            }
        }
    }
}

// On x86_64, we use core::arch::x86_64::cmpxchg16b instead of core::intrinsics.
// On s390x, LLVM (as of 16) generates libcalls for operations other than load/store/cmpxchg: https://godbolt.org/z/5a5T4hxMh
#[cfg(any(target_arch = "x86_64", target_arch = "s390x"))]
atomic_rmw_by_atomic_update!();
// On powerpc64, LLVM (as of 16) doesn't support 128-bit atomic min/max: https://godbolt.org/z/3rebKcbdf
#[cfg(target_arch = "powerpc64")]
atomic_rmw_by_atomic_update!(cmp);

#[cfg(not(any(target_arch = "x86_64", target_arch = "s390x")))]
#[inline]
unsafe fn atomic_swap(dst: *mut u128, val: u128, order: Ordering) -> u128 {
    // SAFETY: the caller must uphold the safety contract.
    unsafe {
        match order {
            Acquire => intrinsics::atomic_xchg_acquire(dst, val),
            Release => intrinsics::atomic_xchg_release(dst, val),
            AcqRel => intrinsics::atomic_xchg_acqrel(dst, val),
            Relaxed => intrinsics::atomic_xchg_relaxed(dst, val),
            SeqCst => intrinsics::atomic_xchg_seqcst(dst, val),
            _ => unreachable!("{:?}", order),
        }
    }
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "s390x")))]
#[inline]
unsafe fn atomic_add(dst: *mut u128, val: u128, order: Ordering) -> u128 {
    // SAFETY: the caller must uphold the safety contract.
    unsafe {
        match order {
            Acquire => intrinsics::atomic_xadd_acquire(dst, val),
            Release => intrinsics::atomic_xadd_release(dst, val),
            AcqRel => intrinsics::atomic_xadd_acqrel(dst, val),
            Relaxed => intrinsics::atomic_xadd_relaxed(dst, val),
            SeqCst => intrinsics::atomic_xadd_seqcst(dst, val),
            _ => unreachable!("{:?}", order),
        }
    }
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "s390x")))]
#[inline]
unsafe fn atomic_sub(dst: *mut u128, val: u128, order: Ordering) -> u128 {
    // SAFETY: the caller must uphold the safety contract.
    unsafe {
        match order {
            Acquire => intrinsics::atomic_xsub_acquire(dst, val),
            Release => intrinsics::atomic_xsub_release(dst, val),
            AcqRel => intrinsics::atomic_xsub_acqrel(dst, val),
            Relaxed => intrinsics::atomic_xsub_relaxed(dst, val),
            SeqCst => intrinsics::atomic_xsub_seqcst(dst, val),
            _ => unreachable!("{:?}", order),
        }
    }
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "s390x")))]
#[inline]
unsafe fn atomic_and(dst: *mut u128, val: u128, order: Ordering) -> u128 {
    // SAFETY: the caller must uphold the safety contract.
    unsafe {
        match order {
            Acquire => intrinsics::atomic_and_acquire(dst, val),
            Release => intrinsics::atomic_and_release(dst, val),
            AcqRel => intrinsics::atomic_and_acqrel(dst, val),
            Relaxed => intrinsics::atomic_and_relaxed(dst, val),
            SeqCst => intrinsics::atomic_and_seqcst(dst, val),
            _ => unreachable!("{:?}", order),
        }
    }
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "s390x")))]
#[inline]
unsafe fn atomic_nand(dst: *mut u128, val: u128, order: Ordering) -> u128 {
    // SAFETY: the caller must uphold the safety contract.
    unsafe {
        match order {
            Acquire => intrinsics::atomic_nand_acquire(dst, val),
            Release => intrinsics::atomic_nand_release(dst, val),
            AcqRel => intrinsics::atomic_nand_acqrel(dst, val),
            Relaxed => intrinsics::atomic_nand_relaxed(dst, val),
            SeqCst => intrinsics::atomic_nand_seqcst(dst, val),
            _ => unreachable!("{:?}", order),
        }
    }
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "s390x")))]
#[inline]
unsafe fn atomic_or(dst: *mut u128, val: u128, order: Ordering) -> u128 {
    // SAFETY: the caller must uphold the safety contract.
    unsafe {
        match order {
            Acquire => intrinsics::atomic_or_acquire(dst, val),
            Release => intrinsics::atomic_or_release(dst, val),
            AcqRel => intrinsics::atomic_or_acqrel(dst, val),
            Relaxed => intrinsics::atomic_or_relaxed(dst, val),
            SeqCst => intrinsics::atomic_or_seqcst(dst, val),
            _ => unreachable!("{:?}", order),
        }
    }
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "s390x")))]
#[inline]
unsafe fn atomic_xor(dst: *mut u128, val: u128, order: Ordering) -> u128 {
    // SAFETY: the caller must uphold the safety contract.
    unsafe {
        match order {
            Acquire => intrinsics::atomic_xor_acquire(dst, val),
            Release => intrinsics::atomic_xor_release(dst, val),
            AcqRel => intrinsics::atomic_xor_acqrel(dst, val),
            Relaxed => intrinsics::atomic_xor_relaxed(dst, val),
            SeqCst => intrinsics::atomic_xor_seqcst(dst, val),
            _ => unreachable!("{:?}", order),
        }
    }
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "powerpc64", target_arch = "s390x")))]
#[inline]
unsafe fn atomic_max(dst: *mut u128, val: u128, order: Ordering) -> i128 {
    // SAFETY: the caller must uphold the safety contract.
    unsafe {
        match order {
            Acquire => intrinsics::atomic_max_acquire(dst.cast::<i128>(), val as i128),
            Release => intrinsics::atomic_max_release(dst.cast::<i128>(), val as i128),
            AcqRel => intrinsics::atomic_max_acqrel(dst.cast::<i128>(), val as i128),
            Relaxed => intrinsics::atomic_max_relaxed(dst.cast::<i128>(), val as i128),
            SeqCst => intrinsics::atomic_max_seqcst(dst.cast::<i128>(), val as i128),
            _ => unreachable!("{:?}", order),
        }
    }
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "powerpc64", target_arch = "s390x")))]
#[inline]
unsafe fn atomic_min(dst: *mut u128, val: u128, order: Ordering) -> i128 {
    // SAFETY: the caller must uphold the safety contract.
    unsafe {
        match order {
            Acquire => intrinsics::atomic_min_acquire(dst.cast::<i128>(), val as i128),
            Release => intrinsics::atomic_min_release(dst.cast::<i128>(), val as i128),
            AcqRel => intrinsics::atomic_min_acqrel(dst.cast::<i128>(), val as i128),
            Relaxed => intrinsics::atomic_min_relaxed(dst.cast::<i128>(), val as i128),
            SeqCst => intrinsics::atomic_min_seqcst(dst.cast::<i128>(), val as i128),
            _ => unreachable!("{:?}", order),
        }
    }
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "powerpc64", target_arch = "s390x")))]
#[inline]
unsafe fn atomic_umax(dst: *mut u128, val: u128, order: Ordering) -> u128 {
    // SAFETY: the caller must uphold the safety contract.
    unsafe {
        match order {
            Acquire => intrinsics::atomic_umax_acquire(dst, val),
            Release => intrinsics::atomic_umax_release(dst, val),
            AcqRel => intrinsics::atomic_umax_acqrel(dst, val),
            Relaxed => intrinsics::atomic_umax_relaxed(dst, val),
            SeqCst => intrinsics::atomic_umax_seqcst(dst, val),
            _ => unreachable!("{:?}", order),
        }
    }
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "powerpc64", target_arch = "s390x")))]
#[inline]
unsafe fn atomic_umin(dst: *mut u128, val: u128, order: Ordering) -> u128 {
    // SAFETY: the caller must uphold the safety contract.
    unsafe {
        match order {
            Acquire => intrinsics::atomic_umin_acquire(dst, val),
            Release => intrinsics::atomic_umin_release(dst, val),
            AcqRel => intrinsics::atomic_umin_acqrel(dst, val),
            Relaxed => intrinsics::atomic_umin_relaxed(dst, val),
            SeqCst => intrinsics::atomic_umin_seqcst(dst, val),
            _ => unreachable!("{:?}", order),
        }
    }
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "s390x")))]
#[inline]
unsafe fn atomic_not(dst: *mut u128, order: Ordering) -> u128 {
    // SAFETY: the caller must uphold the safety contract.
    unsafe { atomic_xor(dst, core::u128::MAX, order) }
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "s390x")))]
#[inline]
unsafe fn atomic_neg(dst: *mut u128, order: Ordering) -> u128 {
    // SAFETY: the caller must uphold the safety contract.
    unsafe { atomic_update(dst, order, u128::wrapping_neg) }
}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
const fn is_lock_free() -> bool {
    IS_ALWAYS_LOCK_FREE
}
#[cfg(not(target_arch = "x86_64"))]
const IS_ALWAYS_LOCK_FREE: bool = true;

#[cfg(target_arch = "x86_64")]
#[inline]
fn is_lock_free() -> bool {
    #[cfg(target_feature = "cmpxchg16b")]
    {
        // CMPXCHG16B is available at compile-time.
        true
    }
    #[cfg(not(target_feature = "cmpxchg16b"))]
    {
        detect::detect().has_cmpxchg16b()
    }
}
#[cfg(target_arch = "x86_64")]
const IS_ALWAYS_LOCK_FREE: bool = cfg!(target_feature = "cmpxchg16b");

atomic128!(AtomicI128, i128, atomic_max, atomic_min);
atomic128!(AtomicU128, u128, atomic_umax, atomic_umin);

#[cfg(test)]
mod tests {
    use super::*;

    test_atomic_int!(i128);
    test_atomic_int!(u128);

    // load/store/swap implementation is not affected by signedness, so it is
    // enough to test only unsigned types.
    stress_test_load_store!(u128);
    stress_test_load_swap!(u128);
}
