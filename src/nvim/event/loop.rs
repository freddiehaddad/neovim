//! Event loop wrapper around `uv_loop_t` (`src/nvim/event/loop.c`).

use crate::event::multiqueue::{
    Event, MultiQueue, multiqueue_free, multiqueue_move_events, multiqueue_new,
    multiqueue_new_child, multiqueue_process_events, multiqueue_purge_events,
    multiqueue_put_event, multiqueue_size,
};
use crate::log::{LOGLVL_ERR, log_uv_handles, logmsg};
use crate::os::time::os_hrtime;
use std::ffi::{c_int, c_void};
use std::ptr;

pub(crate) const UV_RUN_DEFAULT: c_int = 0;
pub(crate) const UV_RUN_ONCE: c_int = 1;
pub(crate) const UV_RUN_NOWAIT: c_int = 2;
pub(crate) const UV_REQ: c_int = 1;
pub(crate) const UV_TTY_MODE_NORMAL: c_int = 0;

#[cfg(windows)]
const UV_EBUSY: c_int = -4082;
#[cfg(not(windows))]
const UV_EBUSY: c_int = -16;

pub(crate) const NS_PER_SEC: u64 = 1_000_000_000;

pub type uv_loop_t = c_void;
pub type uv_handle_t = c_void;
pub type uv_signal_t = c_void;
pub type uv_timer_t = c_void;
pub type uv_async_t = c_void;
pub type uv_mutex_t = c_void;
pub type uv_connect_t = c_void;
pub type uv_fs_t = c_void;
#[cfg(windows)]
pub type uv_uid_t = u8;
#[cfg(not(windows))]
pub type uv_uid_t = u32;
#[cfg(windows)]
pub type uv_gid_t = u8;
#[cfg(not(windows))]
pub type uv_gid_t = u32;
pub type uv_thread_t = usize;

pub type uv_async_cb = Option<unsafe extern "C" fn(handle: *mut uv_async_t)>;
pub type uv_timer_cb = Option<unsafe extern "C" fn(handle: *mut uv_timer_t)>;
pub type uv_close_cb = Option<unsafe extern "C" fn(handle: *mut uv_handle_t)>;
pub type uv_walk_cb = Option<unsafe extern "C" fn(handle: *mut uv_handle_t, arg: *mut c_void)>;
pub type uv_signal_cb = Option<unsafe extern "C" fn(handle: *mut uv_signal_t, signum: c_int)>;
pub type uv_connect_cb = Option<unsafe extern "C" fn(req: *mut c_void, status: c_int)>;
pub type uv_write_cb = Option<unsafe extern "C" fn(req: *mut c_void, status: c_int)>;
pub type uv_fs_cb = Option<unsafe extern "C" fn(req: *mut uv_fs_t)>;
pub type uv_alloc_cb = Option<
    unsafe extern "C" fn(
        handle: *mut uv_handle_t,
        suggested_size: usize,
        buf: *mut crate::event::wstream::uv_buf_t,
    ),
>;
pub type uv_read_cb = Option<
    unsafe extern "C" fn(
        stream: *mut crate::event::stream::uv_stream_t,
        nread: isize,
        buf: *const crate::event::wstream::uv_buf_t,
    ),
>;
pub type uv_idle_cb = Option<unsafe extern "C" fn(handle: *mut crate::event::stream::uv_idle_t)>;
pub type uv_connection_cb =
    Option<unsafe extern "C" fn(server: *mut crate::event::stream::uv_stream_t, status: c_int)>;
#[cfg(windows)]
pub type uv_thread_cb = Option<unsafe extern "C" fn(arg: *mut c_void)>;

#[repr(C)]
pub struct uv_env_item_t {
    pub name: *mut std::ffi::c_char,
    pub value: *mut std::ffi::c_char,
}

#[repr(C)]
pub struct UvTimeval64 {
    pub tv_sec: i64,
    pub tv_usec: i32,
}

#[repr(C)]
pub struct UvTimespec64 {
    pub tv_sec: i64,
    pub tv_nsec: i32,
}

#[repr(C)]
pub struct Loop {
    pub uv: [u64; 192],
    pub events: *mut MultiQueue,
    pub thread_events: *mut MultiQueue,
    pub fast_events: *mut MultiQueue,
    pub children: Vec<*mut crate::event::proc::Proc>,
    pub children_watcher: [u64; 40],
    pub children_kill_timer: [u64; 32],
    pub poll_timer: [u64; 32],
    pub exit_delay_timer: [u64; 32],
    pub async_: [u64; 32],
    pub mutex: [u64; 16],
    pub recursive: c_int,
    pub closing: bool,
}

impl Loop {
    pub const fn new() -> Self {
        Self {
            uv: [0; 192],
            events: ptr::null_mut(),
            thread_events: ptr::null_mut(),
            fast_events: ptr::null_mut(),
            children: Vec::new(),
            children_watcher: [0; 40],
            children_kill_timer: [0; 32],
            poll_timer: [0; 32],
            exit_delay_timer: [0; 32],
            async_: [0; 32],
            mutex: [0; 16],
            recursive: 0,
            closing: false,
        }
    }
}

unsafe extern "C" {
    pub fn uv_loop_size() -> usize;
    pub fn uv_handle_size(h_type: c_int) -> usize;
    pub fn uv_req_size(r_type: c_int) -> usize;
    pub fn uv_handle_get_loop(handle: *const uv_handle_t) -> *mut uv_loop_t;
    pub fn uv_handle_get_type(handle: *const uv_handle_t) -> c_int;
    pub fn uv_guess_handle(file: c_int) -> c_int;
    pub fn uv_strerror(err: c_int) -> *const std::ffi::c_char;
    pub fn uv_err_name(err: c_int) -> *const std::ffi::c_char;
    pub fn uv_translate_sys_error(sys_errno: c_int) -> c_int;

    pub fn uv_loop_init(loop_: *mut uv_loop_t) -> c_int;
    pub fn uv_loop_close(loop_: *mut uv_loop_t) -> c_int;
    pub fn uv_run(loop_: *mut uv_loop_t, mode: c_int) -> c_int;
    pub fn uv_stop(loop_: *mut uv_loop_t);
    pub fn uv_walk(loop_: *mut uv_loop_t, walk_cb: uv_walk_cb, arg: *mut c_void);

    pub fn uv_mutex_init(handle: *mut uv_mutex_t) -> c_int;
    pub fn uv_mutex_init_recursive(handle: *mut c_void) -> c_int;
    pub fn uv_mutex_destroy(handle: *mut uv_mutex_t);
    pub fn uv_mutex_lock(handle: *mut uv_mutex_t);
    pub fn uv_mutex_unlock(handle: *mut uv_mutex_t);

    pub fn uv_async_init(
        loop_: *mut uv_loop_t,
        async_: *mut uv_async_t,
        async_cb: uv_async_cb,
    ) -> c_int;
    pub fn uv_async_send(async_: *mut uv_async_t) -> c_int;

    pub fn uv_signal_init(loop_: *mut uv_loop_t, handle: *mut uv_signal_t) -> c_int;

    pub fn uv_timer_init(loop_: *mut uv_loop_t, handle: *mut uv_timer_t) -> c_int;
    pub fn uv_timer_start(
        handle: *mut uv_timer_t,
        cb: uv_timer_cb,
        timeout: u64,
        repeat: u64,
    ) -> c_int;
    pub fn uv_timer_stop(handle: *mut uv_timer_t) -> c_int;
    pub fn uv_timer_get_due_in(handle: *const uv_timer_t) -> u64;

    pub fn uv_is_closing(handle: *const uv_handle_t) -> c_int;
    pub fn uv_close(handle: *mut uv_handle_t, close_cb: uv_close_cb);
    pub fn uv_unref(handle: *mut uv_handle_t);
    pub fn uv_recv_buffer_size(handle: *mut uv_handle_t, value: *mut c_int) -> c_int;

    pub fn uv_pipe(fds: *mut c_int, read_flags: c_int, write_flags: c_int) -> c_int;
    pub fn uv_pipe_init(loop_: *mut uv_loop_t, handle: *mut c_void, ipc: c_int) -> c_int;
    pub fn uv_pipe_open(handle: *mut c_void, file: c_int) -> c_int;
    pub fn uv_pipe_bind2(
        handle: *mut crate::event::stream::uv_pipe_t,
        name: *const std::ffi::c_char,
        namelen: usize,
        flags: std::ffi::c_uint,
    ) -> c_int;
    pub fn uv_pipe_connect(
        req: *mut c_void,
        handle: *mut c_void,
        name: *const std::ffi::c_char,
        cb: uv_connect_cb,
    );
    pub fn uv_tty_init(loop_: *mut uv_loop_t, tty: *mut c_void, fd: c_int, unused: c_int) -> c_int;
    pub fn uv_tty_set_mode(tty: *mut c_void, mode: c_int) -> c_int;
    pub fn uv_tty_reset_mode() -> c_int;
    pub fn uv_tty_get_winsize(
        tty: *mut crate::event::stream::uv_tty_t,
        width: *mut c_int,
        height: *mut c_int,
    ) -> c_int;
    #[cfg(windows)]
    pub fn uv_tty_set_vterm_state(state: c_int);
    pub fn uv_write(
        req: *mut c_void,
        handle: *mut c_void,
        bufs: *const crate::event::wstream::uv_buf_t,
        nbufs: std::ffi::c_uint,
        cb: uv_write_cb,
    ) -> c_int;
    pub fn uv_read_start(
        stream: *mut crate::event::stream::uv_stream_t,
        alloc_cb: uv_alloc_cb,
        read_cb: uv_read_cb,
    ) -> c_int;
    pub fn uv_read_stop(stream: *mut crate::event::stream::uv_stream_t) -> c_int;
    pub fn uv_stream_set_blocking(
        handle: *mut crate::event::stream::uv_stream_t,
        blocking: c_int,
    ) -> c_int;
    pub fn uv_stream_get_write_queue_size(
        stream: *const crate::event::stream::uv_stream_t,
    ) -> usize;
    pub fn uv_idle_init(loop_: *mut uv_loop_t, idle: *mut crate::event::stream::uv_idle_t)
    -> c_int;
    pub fn uv_idle_start(idle: *mut crate::event::stream::uv_idle_t, cb: uv_idle_cb) -> c_int;
    pub fn uv_idle_stop(idle: *mut crate::event::stream::uv_idle_t) -> c_int;
    pub fn uv_fs_read(
        loop_: *mut uv_loop_t,
        req: *mut uv_fs_t,
        file: c_int,
        bufs: *const crate::event::wstream::uv_buf_t,
        nbufs: std::ffi::c_uint,
        offset: i64,
        cb: uv_fs_cb,
    ) -> c_int;
    pub fn uv_fs_write(
        loop_: *mut uv_loop_t,
        req: *mut uv_fs_t,
        file: c_int,
        bufs: *const crate::event::wstream::uv_buf_t,
        nbufs: std::ffi::c_uint,
        offset: i64,
        cb: uv_fs_cb,
    ) -> c_int;
    pub fn uv_fs_req_cleanup(req: *mut c_void);
    pub fn uv_fs_get_result(req: *const c_void) -> isize;
    pub fn uv_chdir(dir: *const std::ffi::c_char) -> c_int;
    pub fn uv_cwd(buffer: *mut std::ffi::c_char, size: *mut usize) -> c_int;
    pub fn uv_kill(pid: c_int, signum: c_int) -> c_int;
    pub fn uv_signal_start(
        handle: *mut uv_signal_t,
        signal_cb: uv_signal_cb,
        signum: c_int,
    ) -> c_int;
    pub fn uv_signal_stop(handle: *mut uv_signal_t) -> c_int;
    pub fn uv_dlopen(filename: *const std::ffi::c_char, lib: *mut crate::os::dl::UvLib) -> c_int;
    pub fn uv_dlclose(lib: *mut crate::os::dl::UvLib);
    pub fn uv_dlsym(
        lib: *mut crate::os::dl::UvLib,
        name: *const std::ffi::c_char,
        ptr: *mut *mut c_void,
    ) -> c_int;
    pub fn uv_dlerror(lib: *const crate::os::dl::UvLib) -> *const std::ffi::c_char;
    pub fn uv_getaddrinfo(
        loop_: *mut uv_loop_t,
        req: *mut crate::event::socket::uv_getaddrinfo_t,
        getaddrinfo_cb: *mut c_void,
        node: *const std::ffi::c_char,
        service: *const std::ffi::c_char,
        hints: *const crate::event::socket::addrinfo,
    ) -> c_int;
    pub fn uv_freeaddrinfo(ai: *mut c_void);
    pub fn uv_tcp_init(loop_: *mut uv_loop_t, handle: *mut crate::event::socket::uv_tcp_t)
    -> c_int;
    pub fn uv_tcp_nodelay(handle: *mut crate::event::socket::uv_tcp_t, enable: c_int) -> c_int;
    pub fn uv_tcp_bind(
        handle: *mut crate::event::socket::uv_tcp_t,
        addr: *const c_void,
        flags: std::ffi::c_uint,
    ) -> c_int;
    pub fn uv_tcp_getsockname(
        handle: *const crate::event::socket::uv_tcp_t,
        name: *mut c_void,
        namelen: *mut c_int,
    ) -> c_int;
    pub fn uv_tcp_connect(
        req: *mut uv_connect_t,
        handle: *mut crate::event::socket::uv_tcp_t,
        addr: *const c_void,
        cb: uv_connect_cb,
    ) -> c_int;
    pub fn uv_listen(
        stream: *mut crate::event::stream::uv_stream_t,
        backlog: c_int,
        cb: uv_connection_cb,
    ) -> c_int;
    pub fn uv_accept(
        server: *mut crate::event::stream::uv_stream_t,
        client: *mut crate::event::stream::uv_stream_t,
    ) -> c_int;
    pub fn uv_spawn(
        loop_: *mut uv_loop_t,
        handle: *mut crate::event::libuv_proc::uv_process_t,
        options: *const crate::event::libuv_proc::uv_process_options_t,
    ) -> c_int;
    pub fn uv_process_get_pid(handle: *const crate::event::libuv_proc::uv_process_t) -> c_int;
    pub fn uv_os_environ(envitems: *mut *mut uv_env_item_t, count: *mut c_int) -> c_int;
    pub fn uv_os_free_environ(envitems: *mut uv_env_item_t, count: c_int);
    pub fn uv_os_getenv(
        name: *const std::ffi::c_char,
        buffer: *mut std::ffi::c_char,
        size: *mut usize,
    ) -> c_int;
    pub fn uv_os_setenv(name: *const std::ffi::c_char, value: *const std::ffi::c_char) -> c_int;
    pub fn uv_os_unsetenv(name: *const std::ffi::c_char) -> c_int;
    pub fn uv_os_homedir(buffer: *mut std::ffi::c_char, size: *mut usize) -> c_int;
    pub fn uv_os_getpid() -> c_int;
    pub fn uv_random(
        loop_: *mut c_void,
        req: *mut c_void,
        buf: *mut c_void,
        buflen: usize,
        flags: u32,
        cb: *const c_void,
    ) -> c_int;
    pub fn uv_print_all_handles(loop_: *mut c_void, stream: *mut crate::fileio::FILE);
    pub fn uv_gettimeofday(tv: *mut UvTimeval64) -> c_int;
    pub fn uv_uptime(uptime: *mut f64) -> c_int;
    pub fn uv_sleep(msec: std::ffi::c_uint);
    pub fn uv_hrtime() -> u64;
    pub fn uv_clock_gettime(clock_id: c_int, ts: *mut UvTimespec64) -> c_int;
    pub fn uv_now(loop_ptr: *const c_void) -> u64;
    pub fn uv_thread_self() -> uv_thread_t;
    pub fn uv_thread_equal(t1: *const uv_thread_t, t2: *const uv_thread_t) -> c_int;
    #[cfg(windows)]
    pub fn uv_thread_create(tid: *mut uv_thread_t, entry: uv_thread_cb, arg: *mut c_void) -> c_int;
    #[cfg(windows)]
    pub fn uv_thread_detach(tid: *mut uv_thread_t) -> c_int;
    #[cfg(unix)]
    pub fn uv_disable_stdio_inheritance();

    pub fn uv_exepath(buffer: *mut std::ffi::c_char, size: *mut usize) -> c_int;
    pub fn uv_fs_get_ptr(req: *const crate::os::fs::uv_fs_buf_t) -> *mut c_void;
    pub fn uv_fs_get_path(req: *const crate::os::fs::uv_fs_buf_t) -> *const std::ffi::c_char;
    pub fn uv_fs_get_statbuf(req: *mut crate::os::fs::uv_fs_buf_t)
    -> *mut crate::os::fs::uv_stat_t;
    pub fn uv_fs_stat(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        path: *const std::ffi::c_char,
        cb: *const c_void,
    ) -> c_int;
    pub fn uv_fs_fstat(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        file: c_int,
        cb: *const c_void,
    ) -> c_int;
    pub fn uv_fs_lstat(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        path: *const std::ffi::c_char,
        cb: *const c_void,
    ) -> c_int;
    pub fn uv_fs_access(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        path: *const std::ffi::c_char,
        mode: c_int,
        cb: *const c_void,
    ) -> c_int;
    pub fn uv_fs_open(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        path: *const std::ffi::c_char,
        flags: c_int,
        mode: c_int,
        cb: *const c_void,
    ) -> c_int;
    pub fn uv_fs_close(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        file: c_int,
        cb: *const c_void,
    ) -> c_int;
    pub fn uv_fs_copyfile(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        path: *const std::ffi::c_char,
        new_path: *const std::ffi::c_char,
        flags: c_int,
        cb: *const c_void,
    ) -> c_int;
    pub fn uv_fs_fsync(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        file: c_int,
        cb: *const c_void,
    ) -> c_int;
    pub fn uv_fs_chmod(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        path: *const std::ffi::c_char,
        mode: c_int,
        cb: *const c_void,
    ) -> c_int;
    #[cfg(unix)]
    pub fn uv_fs_fchmod(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        file: c_int,
        mode: c_int,
        cb: *const c_void,
    ) -> c_int;
    #[cfg(unix)]
    pub fn uv_fs_chown(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        path: *const std::ffi::c_char,
        uid: uv_uid_t,
        gid: uv_gid_t,
        cb: *const c_void,
    ) -> c_int;
    #[cfg_attr(not(unix), cfg(test))]
    pub fn uv_fs_fchown(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        file: c_int,
        uid: uv_uid_t,
        gid: uv_gid_t,
        cb: *const c_void,
    ) -> c_int;
    #[cfg(unix)]
    pub fn uv_fs_utime(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        path: *const std::ffi::c_char,
        atime: f64,
        mtime: f64,
        cb: *const c_void,
    ) -> c_int;
    pub fn uv_fs_rename(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        path: *const std::ffi::c_char,
        new_path: *const std::ffi::c_char,
        cb: *const c_void,
    ) -> c_int;
    pub fn uv_fs_mkdir(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        path: *const std::ffi::c_char,
        mode: c_int,
        cb: *const c_void,
    ) -> c_int;
    pub fn uv_fs_mkdtemp(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        tpl: *const std::ffi::c_char,
        cb: *const c_void,
    ) -> c_int;
    pub fn uv_fs_rmdir(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        path: *const std::ffi::c_char,
        cb: *const c_void,
    ) -> c_int;
    pub fn uv_fs_scandir(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        path: *const std::ffi::c_char,
        flags: c_int,
        cb: *const c_void,
    ) -> c_int;
    pub fn uv_fs_scandir_next(
        req: *mut crate::os::fs::uv_fs_buf_t,
        ent: *mut crate::os::fs::uv_dirent_t,
    ) -> c_int;
    pub fn uv_fs_unlink(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        path: *const std::ffi::c_char,
        cb: *const c_void,
    ) -> c_int;
    pub fn uv_fs_realpath(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        path: *const std::ffi::c_char,
        cb: *const c_void,
    ) -> c_int;
}

#[inline]
pub unsafe fn loop_queues(loop_: *mut Loop) -> *mut Loop {
    loop_
}

#[inline]
pub unsafe fn main_loop_ptr() -> *mut Loop {
    &raw mut crate::globals::main_loop
}

#[inline]
pub unsafe fn main_loop_events() -> *mut MultiQueue {
    unsafe { crate::globals::main_loop.events }
}

#[inline]
pub unsafe fn loop_children_watcher(loop_: *mut Loop) -> *mut uv_signal_t {
    unsafe { (&raw mut (*loop_).children_watcher).cast() }
}

#[inline]
pub unsafe fn loop_children_kill_timer(loop_: *mut Loop) -> *mut uv_timer_t {
    unsafe { (&raw mut (*loop_).children_kill_timer).cast() }
}

#[inline]
pub unsafe fn loop_poll_timer(loop_: *mut Loop) -> *mut uv_timer_t {
    unsafe { (&raw mut (*loop_).poll_timer).cast() }
}

#[inline]
pub unsafe fn loop_exit_delay_timer(loop_: *mut Loop) -> *mut uv_timer_t {
    unsafe { (&raw mut (*loop_).exit_delay_timer).cast() }
}

#[inline]
pub unsafe fn loop_async(loop_: *mut Loop) -> *mut uv_async_t {
    unsafe { (&raw mut (*loop_).async_).cast() }
}

#[inline]
pub unsafe fn loop_mutex(loop_: *mut Loop) -> *mut uv_mutex_t {
    unsafe { (&raw mut (*loop_).mutex).cast() }
}

#[inline]
pub unsafe fn loop_recursive_ptr(loop_: *mut Loop) -> *mut c_int {
    unsafe { &raw mut (*loop_).recursive }
}

#[inline]
pub unsafe fn loop_closing_ptr(loop_: *mut Loop) -> *mut bool {
    unsafe { &raw mut (*loop_).closing }
}

pub unsafe fn loop_init(loop_: *mut Loop) {
    unsafe {
        debug_assert!(uv_loop_size() <= size_of::<[u64; 192]>());
        uv_loop_init((&raw mut (*loop_).uv).cast());
        (*loop_).recursive = 0;
        (*loop_).closing = false;
        *loop_.cast::<*mut Loop>() = loop_;

        (*loop_).children.clear();
        (*loop_).events = multiqueue_new(loop_);
        (*loop_).fast_events = multiqueue_new_child((*loop_).events);
        (*loop_).thread_events = multiqueue_new(ptr::null_mut());

        uv_mutex_init(loop_mutex(loop_));
        uv_async_init(loop_.cast(), loop_async(loop_), Some(async_cb));
        uv_signal_init(loop_.cast(), loop_children_watcher(loop_));
        uv_timer_init(loop_.cast(), loop_children_kill_timer(loop_));
        let poll_timer = loop_poll_timer(loop_);
        uv_timer_init(loop_.cast(), poll_timer);
        uv_timer_init(loop_.cast(), loop_exit_delay_timer(loop_));
        *poll_timer.cast::<*mut c_void>() = Box::into_raw(Box::new(false)).cast(); // "timeout expired" flag
    }
}

/// Process `Loop.uv` events with a timeout.
unsafe fn loop_uv_run(loop_: *mut Loop, ms: i64) -> bool {
    unsafe {
        let old_rec = (*loop_).recursive;
        (*loop_).recursive = old_rec + 1;
        if old_rec != 0 {
            std::process::abort(); // Should not re-enter uv_run
        }

        let mut mode: c_int = UV_RUN_ONCE;
        let poll_timer = loop_poll_timer(loop_);
        let timeout_expired = *poll_timer.cast::<*mut bool>();
        *timeout_expired = false;

        if ms > 0 {
            uv_timer_start(poll_timer, Some(timer_cb), ms as u64, ms as u64);
        } else if ms == 0 {
            mode = UV_RUN_NOWAIT;
        }

        uv_run(loop_.cast(), mode);

        if ms > 0 {
            uv_timer_stop(poll_timer);
        }

        (*loop_).recursive -= 1; // Can re-enter uv_run now
        *timeout_expired
    }
}

/// Processes one `Loop.uv` event (at most).
/// Processes all `Loop.fast_events` events.
/// Does NOT process `Loop.events`, that is an application-specific decision.
pub unsafe fn loop_poll_events(loop_: *mut Loop, ms: i64) -> bool {
    unsafe {
        let timeout_expired = loop_uv_run(loop_, ms);
        multiqueue_process_events((*loop_).fast_events);
        timeout_expired
    }
}

/// Schedules a fast event from another thread.
pub unsafe fn loop_schedule_fast(loop_: *mut Loop, event: Event) {
    unsafe {
        let mutex = loop_mutex(loop_);
        uv_mutex_lock(mutex);
        multiqueue_put_event((*loop_).thread_events, event);
        uv_async_send(loop_async(loop_));
        uv_mutex_unlock(mutex);
    }
}

/// Schedules an event from another thread. Unlike loop_schedule_fast(), the
/// event is forwarded to `Loop.events`, instead of being processed immediately.
pub unsafe fn loop_schedule_deferred(loop_: *mut Loop, event: Event) {
    unsafe {
        let eventp = Box::into_raw(Box::new(event));
        loop_schedule_fast(
            loop_,
            Event::LoopDeferred {
                loop_,
                event: eventp,
            },
        );
    }
}

pub unsafe fn loop_deferred_event(loop_: *mut Loop, eventp: *mut Event) {
    unsafe {
        multiqueue_put_event((*loop_).events, *eventp);
        drop(Box::from_raw(eventp));
    }
}

pub unsafe fn loop_on_put(loop_: *mut Loop) {
    unsafe {
        if (*loop_).recursive != 0 {
            uv_stop(loop_.cast());
        }
    }
}

unsafe extern "C" fn loop_walk_cb(handle: *mut uv_handle_t, _arg: *mut c_void) {
    unsafe {
        if uv_is_closing(handle) == 0 {
            uv_close(handle, None);
        }
    }
}

/// Closes `loop` and its handles, and frees its structures.
pub unsafe fn loop_close(loop_: *mut Loop, wait: bool) -> bool {
    unsafe {
        let mut rv = true;
        *loop_closing_ptr(loop_) = true;
        uv_mutex_destroy(loop_mutex(loop_));
        uv_close(loop_children_watcher(loop_).cast(), None);
        uv_close(loop_children_kill_timer(loop_).cast(), None);
        uv_close(loop_poll_timer(loop_).cast(), Some(timer_close_cb));
        uv_close(loop_exit_delay_timer(loop_).cast(), None);
        uv_close(loop_async(loop_).cast(), None);
        let start = if wait { os_hrtime() } else { 0 };
        let mut didstop = false;
        loop {
            uv_run(
                loop_.cast(),
                if didstop {
                    UV_RUN_DEFAULT
                } else {
                    UV_RUN_NOWAIT
                },
            );
            if uv_loop_close(loop_.cast()) != UV_EBUSY || !wait {
                break;
            }
            let elapsed_s = (os_hrtime() - start) / NS_PER_SEC;
            if elapsed_s >= 2 {
                rv = false;
                logmsg!(
                    LOGLVL_ERR,
                    ptr::null(),
                    c"loop_close".as_ptr(),
                    line!() as c_int,
                    true,
                    c"uv_loop_close() hang?".as_ptr(),
                );
                log_uv_handles(loop_.cast());
                break;
            }
            if !didstop {
                uv_stop(loop_.cast());
                uv_walk(loop_.cast(), Some(loop_walk_cb), ptr::null_mut());
                didstop = true;
            }
        }
        let qc = loop_queues(loop_);
        multiqueue_free((*qc).fast_events);
        multiqueue_free((*qc).thread_events);
        multiqueue_free((*qc).events);
        (*qc).children = Vec::new();
        rv
    }
}

pub unsafe fn loop_purge(loop_: *mut Loop) {
    unsafe {
        let mutex = loop_mutex(loop_);
        let qc = loop_queues(loop_);
        uv_mutex_lock(mutex);
        multiqueue_purge_events((*qc).thread_events);
        multiqueue_purge_events((*qc).fast_events);
        uv_mutex_unlock(mutex);
    }
}

pub unsafe fn loop_size(loop_: *mut Loop) -> usize {
    unsafe {
        let mutex = loop_mutex(loop_);
        let qc = loop_queues(loop_);
        uv_mutex_lock(mutex);
        let rv = multiqueue_size((*qc).thread_events);
        uv_mutex_unlock(mutex);
        rv
    }
}

unsafe extern "C" fn async_cb(handle: *mut uv_async_t) {
    unsafe {
        let uv_loop = uv_handle_get_loop(handle.cast());
        let l = *uv_loop.cast::<*mut Loop>();
        let mutex = loop_mutex(l);
        let qc = loop_queues(l);
        uv_mutex_lock(mutex);
        multiqueue_move_events((*qc).fast_events, (*qc).thread_events);
        uv_mutex_unlock(mutex);
    }
}

unsafe extern "C" fn timer_cb(handle: *mut uv_timer_t) {
    unsafe {
        let timeout_expired = *handle.cast::<*mut bool>();
        *timeout_expired = true;
    }
}

unsafe extern "C" fn timer_close_cb(handle: *mut uv_handle_t) {
    unsafe {
        let data = (*handle.cast::<*mut c_void>()).cast::<bool>();
        if !data.is_null() {
            drop(Box::from_raw(data));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::libuv_proc::UvProcessHandle;
    use crate::event::signal::SignalWatcher;
    use crate::event::socket::UvSocketHandle;
    use crate::event::stream::{UV_NAMED_PIPE, UV_TCP, UV_TTY, UvStreamHandle};
    use crate::event::time::TimeWatcher;

    #[test]
    fn uv_handle_and_loop_storage_sizes_are_sufficient() {
        const UV_ASYNC: c_int = 1;
        const UV_IDLE: c_int = 6;
        const UV_PROCESS: c_int = 10;
        const UV_TIMER: c_int = 13;
        const UV_SIGNAL: c_int = 16;

        unsafe {
            let loop_dummy = Loop::new();
            assert!(size_of_val(&loop_dummy.uv) >= uv_loop_size());
            assert!(size_of_val(&loop_dummy.children_watcher) >= uv_handle_size(UV_SIGNAL));
            assert!(size_of_val(&loop_dummy.children_kill_timer) >= uv_handle_size(UV_TIMER));
            assert!(size_of_val(&loop_dummy.poll_timer) >= uv_handle_size(UV_TIMER));
            assert!(size_of_val(&loop_dummy.exit_delay_timer) >= uv_handle_size(UV_TIMER));
            assert!(size_of_val(&loop_dummy.async_) >= uv_handle_size(UV_ASYNC));

            let sig_dummy = SignalWatcher::new();
            assert!(size_of_val(&sig_dummy.uv) >= uv_handle_size(UV_SIGNAL));

            let time_dummy = TimeWatcher::new();
            assert!(size_of_val(&time_dummy.uv) >= uv_handle_size(UV_TIMER));

            let max_stream_size = uv_handle_size(UV_NAMED_PIPE)
                .max(uv_handle_size(UV_TCP))
                .max(uv_handle_size(UV_TTY))
                .max(uv_handle_size(UV_IDLE));
            assert!(size_of::<UvStreamHandle>() >= max_stream_size);
            assert!(size_of::<UvSocketHandle>() >= max_stream_size);
            assert!(size_of::<UvProcessHandle>() >= uv_handle_size(UV_PROCESS));
        }
    }
}
