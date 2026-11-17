//! Exercise the production signal adapter against a deterministic native driver.
#![allow(dead_code, non_camel_case_types)]

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::ffi::c_void;

#[path = "../../../src/nvim/event/signal.rs"]
mod signal;

mod event {
    pub mod owned {
        pub use event_lifetimes_tests::SharedCallbackHandle;
    }

    pub mod multiqueue {
        #[derive(Clone, Copy)]
        pub struct MultiQueue(u32);

        impl MultiQueue {
            pub const NULL: Self = Self(0);
            pub const FAST: Self = Self(1);

            pub fn opt(self) -> Option<Self> {
                (self.0 != 0).then_some(self)
            }
        }

        #[derive(Debug, PartialEq, Eq)]
        pub enum Event {
            Signal { signum: i32, is_tui: bool },
        }

        pub fn multiqueue_put_event(_: MultiQueue, event: Event) {
            crate::DRIVER.with(|driver| driver.borrow_mut().events.push(event));
        }
    }

    pub mod r#loop {
        use super::super::*;

        pub type uv_signal_t = c_void;
        pub type uv_handle_t = c_void;
        pub type SignalCallback = unsafe extern "C" fn(*mut uv_signal_t, i32);
        pub type CloseCallback = unsafe extern "C" fn(*mut uv_handle_t);
        pub struct Loop;
        pub static LOOP: Loop = Loop;

        impl Loop {
            pub fn uv_ptr(&self) -> *mut c_void {
                std::ptr::null_mut()
            }

            pub fn register_handle(&self, handle: *mut c_void, close: unsafe fn(*mut c_void)) {
                DRIVER.with(|driver| {
                    let mut driver = driver.borrow_mut();
                    assert!(driver.native.contains_key(&handle.addr()));
                    assert!(driver.owned.insert(handle.addr(), close).is_none());
                });
            }

            pub fn unregister_handle(&self, handle: *mut c_void) {
                DRIVER.with(|driver| {
                    assert!(driver.borrow_mut().owned.remove(&handle.addr()).is_some());
                });
            }
        }

        pub unsafe fn uv_handle_size(kind: i32) -> usize {
            assert_eq!(kind, 16);
            40 * size_of::<u64>()
        }

        pub unsafe fn uv_signal_init(_: *mut c_void, handle: *mut uv_signal_t) -> i32 {
            DRIVER.with(|driver| {
                let mut driver = driver.borrow_mut();
                if let Some(error) = driver.init_error.take() {
                    return error;
                }
                assert!(
                    driver
                        .native
                        .insert(
                            handle.addr(),
                            NativeSignal {
                                handle,
                                callback: None,
                                signum: 0,
                                active: false,
                                close: None,
                            },
                        )
                        .is_none()
                );
                0
            })
        }

        pub unsafe fn uv_signal_start(
            handle: *mut uv_signal_t,
            callback: Option<SignalCallback>,
            signum: i32,
        ) -> i32 {
            DRIVER.with(|driver| {
                let mut driver = driver.borrow_mut();
                driver.starts += 1;
                assert!(driver.owned.contains_key(&handle.addr()));
                let native = driver.native.get_mut(&handle.addr()).unwrap();
                assert!(native.close.is_none());
                if signum <= 0 {
                    return crate::os::fs::UV_EINVAL;
                }
                native.callback = callback;
                native.signum = signum;
                native.active = true;
                unsafe { native_write(handle) };
                0
            })
        }

        pub unsafe fn uv_signal_stop(handle: *mut uv_signal_t) -> i32 {
            DRIVER.with(|driver| {
                let mut driver = driver.borrow_mut();
                driver.stops += 1;
                let native = driver.native.get_mut(&handle.addr()).unwrap();
                assert!(native.close.is_none());
                native.active = false;
                unsafe { native_write(handle) };
            });
            0
        }

        pub unsafe fn uv_close(handle: *mut uv_handle_t, callback: Option<CloseCallback>) {
            DRIVER.with(|driver| {
                let mut driver = driver.borrow_mut();
                driver.closes += 1;
                let native = driver.native.get_mut(&handle.addr()).unwrap();
                assert!(native.close.is_none());
                native.active = false;
                native.close = Some(callback.unwrap());
                unsafe { native_write(handle) };
            });
        }
    }
}

mod os {
    pub mod fs {
        pub const UV_EINVAL: i32 = -22;
    }

    pub mod signal {
        pub fn on_signal(signum: i32) {
            crate::dispatch(crate::Delivery::Editor(signum));
        }
    }
}

mod ui_client {
    pub fn with_tui_mut(callback: fn(&mut ())) {
        callback(&mut ());
    }
}

mod tui {
    pub mod callbacks {
        pub fn sigwinch_cb(_: &mut ()) {
            crate::dispatch(crate::Delivery::Tui);
        }
    }

    pub use callbacks as tui;
}

#[derive(Clone, Copy)]
struct NativeSignal {
    handle: *mut c_void,
    callback: Option<event::r#loop::SignalCallback>,
    signum: i32,
    active: bool,
    close: Option<event::r#loop::CloseCallback>,
}

#[derive(Debug, PartialEq, Eq)]
enum Delivery {
    Editor(i32),
    Tui,
}

#[derive(Default)]
struct Driver {
    native: BTreeMap<usize, NativeSignal>,
    owned: BTreeMap<usize, unsafe fn(*mut c_void)>,
    events: Vec<event::multiqueue::Event>,
    delivered: Vec<Delivery>,
    init_error: Option<i32>,
    starts: usize,
    stops: usize,
    closes: usize,
}

thread_local! {
    static DRIVER: RefCell<Driver> = RefCell::new(Driver::default());
    static DISPATCH_HOOK: Cell<Option<fn()>> = const { Cell::new(None) };
    static OWNER: RefCell<Option<signal::SignalWatcher>> = const { RefCell::new(None) };
}

unsafe fn native_write(handle: *mut c_void) {
    unsafe {
        handle
            .byte_add(size_of::<*const c_void>())
            .cast::<u8>()
            .write(2);
    }
}

fn emit(signum: i32) {
    let callbacks: Vec<_> = DRIVER.with(|driver| {
        driver
            .borrow()
            .native
            .values()
            .filter(|native| native.active && native.signum == signum)
            .copied()
            .collect()
    });
    for callback in callbacks {
        let active = DRIVER.with(|driver| {
            driver
                .borrow()
                .native
                .get(&callback.handle.addr())
                .is_some_and(|native| native.active && native.signum == signum)
        });
        if active {
            unsafe { callback.callback.unwrap()(callback.handle, signum) };
        }
    }
}

fn finish_closes() {
    loop {
        let closing = DRIVER.with(|driver| {
            let mut driver = driver.borrow_mut();
            let key = driver
                .native
                .iter()
                .find_map(|(&key, native)| native.close.is_some().then_some(key));
            key.and_then(|key| driver.native.remove(&key))
        });
        let Some(closing) = closing else {
            return;
        };
        unsafe {
            native_write(closing.handle);
            closing.close.unwrap()(closing.handle);
        }
    }
}

fn force_close() {
    let callbacks: Vec<_> = DRIVER.with(|driver| {
        let driver = driver.borrow();
        driver
            .native
            .values()
            .filter(|native| native.close.is_none())
            .map(|native| (native.handle, driver.owned[&native.handle.addr()]))
            .collect()
    });
    for (handle, close) in callbacks {
        unsafe { close(handle) };
    }
    finish_closes();
}

fn dispatch(delivery: Delivery) {
    DRIVER.with(|driver| driver.borrow_mut().delivered.push(delivery));
    if let Some(hook) = DISPATCH_HOOK.take() {
        hook();
    }
}

fn dispatch_queued() {
    let events = DRIVER.with(|driver| std::mem::take(&mut driver.borrow_mut().events));
    for event in events {
        let event::multiqueue::Event::Signal { signum, is_tui } = event;
        signal::signal_event(signum, is_tui);
    }
}

fn assert_no_native_handles() {
    DRIVER.with(|driver| {
        let driver = driver.borrow();
        assert!(driver.native.is_empty());
        assert!(driver.owned.is_empty());
    });
}

use event::r#loop::LOOP;
use event::multiqueue::{Event, MultiQueue};
use signal::{SignalTarget, SignalWatcher};

#[test]
fn live_dispatch_supports_stop_and_restart() {
    let watcher = SignalWatcher::new(&LOOP, MultiQueue::FAST, SignalTarget::Editor).unwrap();
    assert_eq!(watcher.start(10), Ok(()));
    emit(10);
    watcher.stop();
    emit(10);
    assert_eq!(watcher.start(28), Ok(()));
    emit(28);
    dispatch_queued();
    DRIVER.with(|driver| {
        assert_eq!(
            driver.borrow().delivered,
            vec![Delivery::Editor(10), Delivery::Editor(28)]
        );
    });
    drop(watcher);
    finish_closes();
    assert_no_native_handles();
}

#[test]
fn moved_owner_and_copied_tui_event_outlive_native_close() {
    let watcher = SignalWatcher::new(&LOOP, MultiQueue::FAST, SignalTarget::Tui).unwrap();
    let moved = vec![watcher];
    assert_eq!(moved[0].start(28), Ok(()));
    emit(28);
    drop(moved);
    finish_closes();
    assert_no_native_handles();
    DRIVER.with(|driver| {
        assert_eq!(
            driver.borrow().events,
            vec![Event::Signal {
                signum: 28,
                is_tui: true,
            }]
        );
    });
    dispatch_queued();
    DRIVER.with(|driver| assert_eq!(driver.borrow().delivered, vec![Delivery::Tui]));
}

#[test]
fn initialization_failure_never_registers_or_closes_storage() {
    DRIVER.with(|driver| driver.borrow_mut().init_error = Some(-12));
    assert!(matches!(
        SignalWatcher::new(&LOOP, MultiQueue::FAST, SignalTarget::Editor),
        Err(-12)
    ));
    assert_no_native_handles();
    DRIVER.with(|driver| assert_eq!(driver.borrow().closes, 0));
}

#[test]
fn unsupported_start_does_not_destroy_the_initialized_watcher() {
    let watcher = SignalWatcher::new(&LOOP, MultiQueue::FAST, SignalTarget::Editor).unwrap();
    assert_eq!(watcher.start(-1), Err(os::fs::UV_EINVAL));
    assert_eq!(watcher.start(10), Ok(()));
    emit(10);
    dispatch_queued();
    DRIVER.with(|driver| assert_eq!(driver.borrow().delivered, vec![Delivery::Editor(10)]));
    drop(watcher);
    finish_closes();
    assert_no_native_handles();
}

#[test]
fn immediate_callback_retains_storage_through_reentrant_close() {
    fn close_owner() {
        let owner = OWNER.with(|owner| owner.borrow_mut().take());
        drop(owner);
        finish_closes();
        assert_no_native_handles();
    }

    let watcher = SignalWatcher::new(&LOOP, MultiQueue::NULL, SignalTarget::Editor).unwrap();
    assert_eq!(watcher.start(10), Ok(()));
    OWNER.with(|owner| *owner.borrow_mut() = Some(watcher));
    DISPATCH_HOOK.set(Some(close_owner));
    emit(10);
    DRIVER.with(|driver| assert_eq!(driver.borrow().delivered, vec![Delivery::Editor(10)]));
    assert!(OWNER.with(|owner| owner.borrow().is_none()));
}

#[test]
fn forced_shutdown_and_repeated_close_do_not_reuse_native_storage() {
    let watcher = SignalWatcher::new(&LOOP, MultiQueue::FAST, SignalTarget::Editor).unwrap();
    assert_eq!(watcher.start(10), Ok(()));
    force_close();
    let calls = DRIVER.with(|driver| {
        let driver = driver.borrow();
        (driver.starts, driver.stops, driver.closes)
    });
    watcher.stop();
    watcher.close();
    assert_eq!(watcher.start(10), Err(os::fs::UV_EINVAL));
    drop(watcher);
    DRIVER.with(|driver| {
        let driver = driver.borrow();
        assert_eq!((driver.starts, driver.stops, driver.closes), calls);
    });
    assert_no_native_handles();
}
