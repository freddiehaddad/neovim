//! Shared timer storage and deferred cleanup, independent of the native driver.

use super::owned::SharedCallbackHandle;

pub const TIMER_HANDLE_WORDS: usize = 32;

pub type TimerHandle<T> = SharedCallbackHandle<T, TIMER_HANDLE_WORDS>;

pub struct TimerCloseEvent<T: 'static> {
    timer: TimerHandle<T>,
    cleanup: fn(&T),
}

impl<T: 'static> TimerCloseEvent<T> {
    pub fn new(timer: TimerHandle<T>, cleanup: fn(&T)) -> Self {
        assert!(timer.is_closed(), "native timer close is incomplete");
        Self { timer, cleanup }
    }
}

impl<T: 'static> Drop for TimerCloseEvent<T> {
    fn drop(&mut self) {
        (self.cleanup)(self.timer.data());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    #[test]
    fn callback_owner_survives_native_close_and_handle_moves() {
        struct Payload {
            drops: Rc<Cell<usize>>,
            value: Cell<usize>,
        }

        impl Drop for Payload {
            fn drop(&mut self) {
                self.drops.set(self.drops.get() + 1);
            }
        }

        let drops = Rc::new(Cell::new(0));
        let timer = TimerHandle::new(Payload {
            drops: drops.clone(),
            value: Cell::new(0),
        });
        let pointer = timer.as_ptr();
        unsafe { timer.register() };
        let moved = vec![timer];
        let callback = unsafe { TimerHandle::<Payload>::from_callback(pointer) };
        drop(moved);
        assert_eq!(callback.as_ptr(), pointer);
        callback.data().value.set(1);
        assert!(callback.begin_close());
        assert!(!callback.begin_close());
        unsafe { callback.finish_close() };
        assert!(!callback.is_active());
        assert!(!callback.can_initialize());
        callback.data().value.set(2);
        assert_eq!(drops.get(), 0);
        drop(callback);
        assert_eq!(drops.get(), 1);
    }

    #[test]
    fn cancelled_close_event_keeps_data_alive_while_releasing_registry_owner() {
        struct Payload {
            registry: std::rc::Weak<RefCell<Option<TimerHandle<Payload>>>>,
            cleanups: Rc<Cell<usize>>,
            drops: Rc<Cell<usize>>,
        }

        impl Drop for Payload {
            fn drop(&mut self) {
                self.drops.set(self.drops.get() + 1);
            }
        }

        fn cleanup(data: &Payload) {
            let owner = data.registry.upgrade().unwrap().borrow_mut().take();
            drop(owner);
            assert_eq!(data.drops.get(), 0);
            data.cleanups.set(data.cleanups.get() + 1);
        }

        let registry = Rc::new(RefCell::new(None));
        let cleanups = Rc::new(Cell::new(0));
        let drops = Rc::new(Cell::new(0));
        let timer = TimerHandle::new(Payload {
            registry: Rc::downgrade(&registry),
            cleanups: cleanups.clone(),
            drops: drops.clone(),
        });
        unsafe { timer.register() };
        registry.replace(Some(timer.clone()));
        assert!(timer.begin_close());
        unsafe { timer.finish_close() };
        let event = TimerCloseEvent::new(timer.clone(), cleanup);
        drop(timer);
        assert_eq!(drops.get(), 0);
        drop(event);
        assert!(registry.borrow().is_none());
        assert_eq!(cleanups.get(), 1);
        assert_eq!(drops.get(), 1);
    }
}
