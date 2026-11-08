//! Atomic pointer updates compare storage and offset, including fresh aliases.
#![no_std]
#![no_main]
#![deny(unsafe_code)]
use core::ptr::null_mut;
use core::sync::atomic::{AtomicPtr, Ordering};
use mires_js::{js_export, throw_value, JsValue};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    throw_value(JsValue::erase("atomic pointer check failed"))
}

#[js_export]
pub fn checks() -> i32 {
    let mut values = [11_u32, 22, 33];
    let base = values.as_mut_ptr();
    let cell = AtomicPtr::new(null_mut());
    let mut checks = 0;
    for success in [
        Ordering::Relaxed,
        Ordering::Acquire,
        Ordering::Release,
        Ordering::AcqRel,
        Ordering::SeqCst,
    ] {
        for failure in [Ordering::Relaxed, Ordering::Acquire, Ordering::SeqCst] {
            cell.store(null_mut(), Ordering::Release);
            assert_eq!(
                cell.compare_exchange(null_mut(), base, success, failure),
                Ok(null_mut())
            );
            assert_eq!(cell.load(Ordering::Acquire), base);
            assert_eq!(
                cell.compare_exchange(base.wrapping_add(1), null_mut(), success, failure),
                Err(base)
            );
            assert_eq!(cell.load(Ordering::Acquire), base);
            assert_eq!(
                cell.compare_exchange(
                    base.wrapping_add(2).wrapping_sub(2),
                    base.wrapping_add(1),
                    success,
                    failure
                ),
                Ok(base)
            );
            assert_eq!(
                cell.compare_exchange(base, null_mut(), success, failure),
                Err(base.wrapping_add(1))
            );
            assert_eq!(
                cell.swap(base.wrapping_add(2), Ordering::AcqRel),
                base.wrapping_add(1)
            );
            loop {
                match cell.compare_exchange_weak(base.wrapping_add(2), null_mut(), success, failure)
                {
                    Ok(old) => {
                        assert_eq!(old, base.wrapping_add(2));
                        break;
                    }
                    Err(old) => assert_eq!(old, base.wrapping_add(2)),
                }
            }
            assert!(cell.load(Ordering::SeqCst).is_null());
            checks += 9;
        }
    }
    checks
}
