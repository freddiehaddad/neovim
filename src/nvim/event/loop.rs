//! Event loop wrapper around `uv_loop_t` (`src/nvim/event/loop.c`).

use crate::event::multiqueue::{
    Event, MultiQueue, multiqueue_free, multiqueue_move_events, multiqueue_new,
    multiqueue_new_child, multiqueue_process_events, multiqueue_purge_events, multiqueue_put_event,
    multiqueue_size,
};
use crate::log::{LOGLVL_ERR, log_uv_handles, logmsg};
use crate::os::time::os_hrtime;
use std::ffi::c_void;
use std::ptr;

pub(crate) const UV_RUN_DEFAULT: i32 = 0;
pub(crate) const UV_RUN_ONCE: i32 = 1;
pub(crate) const UV_RUN_NOWAIT: i32 = 2;
pub(crate) const UV_REQ: i32 = 1;
pub(crate) const UV_TTY_MODE_NORMAL: i32 = 0;

#[cfg(windows)]
const UV_EBUSY: i32 = -4082;
#[cfg(not(windows))]
const UV_EBUSY: i32 = -16;

pub(crate) const NS_PER_SEC: u64 = 1_000_000_000;

#[allow(non_camel_case_types)]
pub type uv_loop_t = c_void;
#[allow(non_camel_case_types)]
pub type uv_handle_t = c_void;
#[allow(non_camel_case_types)]
pub type uv_signal_t = c_void;
#[allow(non_camel_case_types)]
pub type uv_timer_t = c_void;
#[allow(non_camel_case_types)]
pub type uv_async_t = c_void;
#[allow(non_camel_case_types)]
pub type uv_mutex_t = c_void;
#[allow(non_camel_case_types)]
pub type uv_connect_t = c_void;
#[allow(non_camel_case_types)]
pub type uv_fs_t = c_void;
#[cfg(windows)]
#[allow(non_camel_case_types)]
pub type uv_uid_t = u8;
#[cfg(not(windows))]
#[allow(non_camel_case_types)]
pub type uv_uid_t = u32;
#[cfg(windows)]
#[allow(non_camel_case_types)]
pub type uv_gid_t = u8;
#[cfg(not(windows))]
#[allow(non_camel_case_types)]
pub type uv_gid_t = u32;
#[allow(non_camel_case_types)]
pub type uv_thread_t = usize;

#[allow(non_camel_case_types)]
pub type uv_async_cb = Option<unsafe extern "C" fn(handle: *mut uv_async_t)>;
#[allow(non_camel_case_types)]
pub type uv_timer_cb = Option<unsafe extern "C" fn(handle: *mut uv_timer_t)>;
#[allow(non_camel_case_types)]
pub type uv_close_cb = Option<unsafe extern "C" fn(handle: *mut uv_handle_t)>;
#[allow(non_camel_case_types)]
pub type uv_walk_cb = Option<unsafe extern "C" fn(handle: *mut uv_handle_t, arg: *mut c_void)>;
#[allow(non_camel_case_types)]
pub type uv_signal_cb = Option<unsafe extern "C" fn(handle: *mut uv_signal_t, signum: i32)>;
#[allow(non_camel_case_types)]
pub type uv_connect_cb = Option<unsafe extern "C" fn(req: *mut c_void, status: i32)>;
#[allow(non_camel_case_types)]
pub type uv_write_cb = Option<unsafe extern "C" fn(req: *mut c_void, status: i32)>;
#[allow(non_camel_case_types)]
pub type uv_fs_cb = Option<unsafe extern "C" fn(req: *mut uv_fs_t)>;
#[allow(non_camel_case_types)]
pub type uv_alloc_cb = Option<
    unsafe extern "C" fn(
        handle: *mut uv_handle_t,
        suggested_size: usize,
        buf: *mut crate::event::wstream::uv_buf_t,
    ),
>;
#[allow(non_camel_case_types)]
pub type uv_read_cb = Option<
    unsafe extern "C" fn(
        stream: *mut crate::event::stream::uv_stream_t,
        nread: isize,
        buf: *const crate::event::wstream::uv_buf_t,
    ),
>;
#[allow(non_camel_case_types)]
pub type uv_idle_cb = Option<unsafe extern "C" fn(handle: *mut crate::event::stream::uv_idle_t)>;
#[allow(non_camel_case_types)]
pub type uv_connection_cb =
    Option<unsafe extern "C" fn(server: *mut crate::event::stream::uv_stream_t, status: i32)>;
#[cfg(windows)]
#[allow(non_camel_case_types)]
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
    pub recursive: i32,
    pub closing: bool,
    pub timeout_expired: bool,
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
            timeout_expired: false,
        }
    }
}

unsafe extern "C" {
    pub fn uv_loop_size() -> usize;
    pub fn uv_handle_size(h_type: i32) -> usize;
    pub fn uv_req_size(r_type: i32) -> usize;
    pub fn uv_handle_get_loop(handle: *const uv_handle_t) -> *mut uv_loop_t;
    pub fn uv_handle_get_type(handle: *const uv_handle_t) -> i32;
    pub fn uv_guess_handle(file: i32) -> i32;
    pub fn uv_strerror(err: i32) -> *const std::ffi::c_char;
    pub fn uv_err_name(err: i32) -> *const std::ffi::c_char;
    pub fn uv_translate_sys_error(sys_errno: i32) -> i32;

    pub fn uv_loop_init(loop_: *mut uv_loop_t) -> i32;
    pub fn uv_loop_close(loop_: *mut uv_loop_t) -> i32;
    pub fn uv_run(loop_: *mut uv_loop_t, mode: i32) -> i32;
    pub fn uv_stop(loop_: *mut uv_loop_t);
    pub fn uv_walk(loop_: *mut uv_loop_t, walk_cb: uv_walk_cb, arg: *mut c_void);

    pub fn uv_mutex_init(handle: *mut uv_mutex_t) -> i32;
    pub fn uv_mutex_init_recursive(handle: *mut c_void) -> i32;
    pub fn uv_mutex_destroy(handle: *mut uv_mutex_t);
    pub fn uv_mutex_lock(handle: *mut uv_mutex_t);
    pub fn uv_mutex_unlock(handle: *mut uv_mutex_t);

    pub fn uv_async_init(
        loop_: *mut uv_loop_t,
        async_: *mut uv_async_t,
        async_cb: uv_async_cb,
    ) -> i32;
    pub fn uv_async_send(async_: *mut uv_async_t) -> i32;

    pub fn uv_signal_init(loop_: *mut uv_loop_t, handle: *mut uv_signal_t) -> i32;

    pub fn uv_timer_init(loop_: *mut uv_loop_t, handle: *mut uv_timer_t) -> i32;
    pub fn uv_timer_start(
        handle: *mut uv_timer_t,
        cb: uv_timer_cb,
        timeout: u64,
        repeat: u64,
    ) -> i32;
    pub fn uv_timer_stop(handle: *mut uv_timer_t) -> i32;
    pub fn uv_timer_get_due_in(handle: *const uv_timer_t) -> u64;

    pub fn uv_is_closing(handle: *const uv_handle_t) -> i32;
    pub fn uv_close(handle: *mut uv_handle_t, close_cb: uv_close_cb);
    pub fn uv_unref(handle: *mut uv_handle_t);
    pub fn uv_recv_buffer_size(handle: *mut uv_handle_t, value: &mut i32) -> i32;

    pub fn uv_pipe(fds: *mut i32, read_flags: i32, write_flags: i32) -> i32;
    pub fn uv_pipe_init(loop_: *mut uv_loop_t, handle: *mut c_void, ipc: i32) -> i32;
    pub fn uv_pipe_open(handle: *mut c_void, file: i32) -> i32;
    pub fn uv_pipe_bind2(
        handle: *mut crate::event::stream::uv_pipe_t,
        name: *const std::ffi::c_char,
        namelen: usize,
        flags: u32,
    ) -> i32;
    pub fn uv_pipe_connect(
        req: *mut c_void,
        handle: *mut c_void,
        name: *const std::ffi::c_char,
        cb: uv_connect_cb,
    );
    pub fn uv_tty_init(loop_: *mut uv_loop_t, tty: *mut c_void, fd: i32, unused: i32) -> i32;
    pub fn uv_tty_set_mode(tty: *mut c_void, mode: i32) -> i32;
    pub fn uv_tty_reset_mode() -> i32;
    pub fn uv_tty_get_winsize(
        tty: *mut crate::event::stream::uv_tty_t,
        width: &mut i32,
        height: &mut i32,
    ) -> i32;
    #[cfg(windows)]
    pub fn uv_tty_set_vterm_state(state: i32);
    pub fn uv_write(
        req: *mut c_void,
        handle: *mut c_void,
        bufs: *const crate::event::wstream::uv_buf_t,
        nbufs: u32,
        cb: uv_write_cb,
    ) -> i32;
    pub fn uv_read_start(
        stream: *mut crate::event::stream::uv_stream_t,
        alloc_cb: uv_alloc_cb,
        read_cb: uv_read_cb,
    ) -> i32;
    pub fn uv_read_stop(stream: *mut crate::event::stream::uv_stream_t) -> i32;
    pub fn uv_stream_set_blocking(
        handle: *mut crate::event::stream::uv_stream_t,
        blocking: i32,
    ) -> i32;
    pub fn uv_stream_get_write_queue_size(
        stream: *const crate::event::stream::uv_stream_t,
    ) -> usize;
    pub fn uv_idle_init(loop_: *mut uv_loop_t, idle: *mut crate::event::stream::uv_idle_t) -> i32;
    pub fn uv_idle_start(idle: *mut crate::event::stream::uv_idle_t, cb: uv_idle_cb) -> i32;
    pub fn uv_idle_stop(idle: *mut crate::event::stream::uv_idle_t) -> i32;
    pub fn uv_fs_read(
        loop_: *mut uv_loop_t,
        req: *mut uv_fs_t,
        file: i32,
        bufs: *const crate::event::wstream::uv_buf_t,
        nbufs: u32,
        offset: i64,
        cb: uv_fs_cb,
    ) -> i32;
    pub fn uv_fs_write(
        loop_: *mut uv_loop_t,
        req: *mut uv_fs_t,
        file: i32,
        bufs: *const crate::event::wstream::uv_buf_t,
        nbufs: u32,
        offset: i64,
        cb: uv_fs_cb,
    ) -> i32;
    pub fn uv_fs_req_cleanup(req: *mut c_void);
    pub fn uv_fs_get_result(req: *const c_void) -> isize;
    pub fn uv_chdir(dir: *const std::ffi::c_char) -> i32;
    pub fn uv_cwd(buffer: *mut std::ffi::c_char, size: &mut usize) -> i32;
    pub fn uv_kill(pid: i32, signum: i32) -> i32;
    pub fn uv_signal_start(handle: *mut uv_signal_t, signal_cb: uv_signal_cb, signum: i32) -> i32;
    pub fn uv_signal_stop(handle: *mut uv_signal_t) -> i32;
    pub fn uv_dlopen(filename: *const std::ffi::c_char, lib: *mut crate::os::dl::UvLib) -> i32;
    pub fn uv_dlclose(lib: *mut crate::os::dl::UvLib);
    pub fn uv_dlsym(
        lib: *mut crate::os::dl::UvLib,
        name: *const std::ffi::c_char,
        ptr: *mut *mut c_void,
    ) -> i32;
    pub fn uv_dlerror(lib: *const crate::os::dl::UvLib) -> *const std::ffi::c_char;
    pub fn uv_getaddrinfo(
        loop_: *mut uv_loop_t,
        req: *mut crate::event::socket::uv_getaddrinfo_t,
        getaddrinfo_cb: *mut c_void,
        node: *const std::ffi::c_char,
        service: *const std::ffi::c_char,
        hints: *const crate::event::socket::addrinfo,
    ) -> i32;
    pub fn uv_freeaddrinfo(ai: *mut c_void);
    pub fn uv_tcp_init(loop_: *mut uv_loop_t, handle: *mut crate::event::socket::uv_tcp_t) -> i32;
    pub fn uv_tcp_nodelay(handle: *mut crate::event::socket::uv_tcp_t, enable: i32) -> i32;
    pub fn uv_tcp_bind(
        handle: *mut crate::event::socket::uv_tcp_t,
        addr: *const c_void,
        flags: u32,
    ) -> i32;
    pub fn uv_tcp_getsockname(
        handle: *const crate::event::socket::uv_tcp_t,
        name: *mut c_void,
        namelen: &mut i32,
    ) -> i32;
    pub fn uv_tcp_connect(
        req: *mut uv_connect_t,
        handle: *mut crate::event::socket::uv_tcp_t,
        addr: *const c_void,
        cb: uv_connect_cb,
    ) -> i32;
    pub fn uv_listen(
        stream: *mut crate::event::stream::uv_stream_t,
        backlog: i32,
        cb: uv_connection_cb,
    ) -> i32;
    pub fn uv_accept(
        server: *mut crate::event::stream::uv_stream_t,
        client: *mut crate::event::stream::uv_stream_t,
    ) -> i32;
    pub fn uv_spawn(
        loop_: *mut uv_loop_t,
        handle: *mut crate::event::libuv_proc::uv_process_t,
        options: *const crate::event::libuv_proc::uv_process_options_t,
    ) -> i32;
    pub fn uv_process_get_pid(handle: *const crate::event::libuv_proc::uv_process_t) -> i32;
    pub fn uv_os_environ(envitems: *mut *mut uv_env_item_t, count: &mut i32) -> i32;
    pub fn uv_os_free_environ(envitems: *mut uv_env_item_t, count: i32);
    pub fn uv_os_getenv(
        name: *const std::ffi::c_char,
        buffer: *mut std::ffi::c_char,
        size: &mut usize,
    ) -> i32;
    pub fn uv_os_setenv(name: *const std::ffi::c_char, value: *const std::ffi::c_char) -> i32;
    pub fn uv_os_unsetenv(name: *const std::ffi::c_char) -> i32;
    pub fn uv_os_homedir(buffer: *mut std::ffi::c_char, size: &mut usize) -> i32;
    pub fn uv_os_getpid() -> i32;
    pub fn uv_random(
        loop_: *mut c_void,
        req: *mut c_void,
        buf: *mut c_void,
        buflen: usize,
        flags: u32,
        cb: *const c_void,
    ) -> i32;
    pub fn uv_print_all_handles(loop_: *mut uv_loop_t, stream: *mut crate::fileio::FILE);
    pub fn uv_gettimeofday(tv: &mut UvTimeval64) -> i32;
    pub fn uv_uptime(uptime: &mut f64) -> i32;
    pub fn uv_sleep(msec: u32);
    pub fn uv_hrtime() -> u64;
    pub fn uv_clock_gettime(clock_id: i32, ts: &mut UvTimespec64) -> i32;
    pub fn uv_now(loop_ptr: *const c_void) -> u64;
    pub fn uv_thread_self() -> uv_thread_t;
    pub fn uv_thread_equal(t1: *const uv_thread_t, t2: *const uv_thread_t) -> i32;
    #[cfg(windows)]
    pub fn uv_thread_create(tid: *mut uv_thread_t, entry: uv_thread_cb, arg: *mut c_void) -> i32;
    #[cfg(windows)]
    pub fn uv_thread_detach(tid: *mut uv_thread_t) -> i32;
    #[cfg(unix)]
    pub fn uv_disable_stdio_inheritance();

    pub fn uv_exepath(buffer: *mut std::ffi::c_char, size: *mut usize) -> i32;
    pub fn uv_fs_get_ptr(req: *const crate::os::fs::uv_fs_buf_t) -> *mut c_void;
    pub fn uv_fs_get_path(req: *const crate::os::fs::uv_fs_buf_t) -> *const std::ffi::c_char;
    pub fn uv_fs_get_statbuf(req: *mut crate::os::fs::uv_fs_buf_t)
    -> *mut crate::os::fs::uv_stat_t;
    pub fn uv_fs_stat(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        path: *const std::ffi::c_char,
        cb: *const c_void,
    ) -> i32;
    pub fn uv_fs_fstat(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        file: i32,
        cb: *const c_void,
    ) -> i32;
    pub fn uv_fs_lstat(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        path: *const std::ffi::c_char,
        cb: *const c_void,
    ) -> i32;
    pub fn uv_fs_access(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        path: *const std::ffi::c_char,
        mode: i32,
        cb: *const c_void,
    ) -> i32;
    pub fn uv_fs_open(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        path: *const std::ffi::c_char,
        flags: i32,
        mode: i32,
        cb: *const c_void,
    ) -> i32;
    pub fn uv_fs_close(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        file: i32,
        cb: *const c_void,
    ) -> i32;
    pub fn uv_fs_copyfile(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        path: *const std::ffi::c_char,
        new_path: *const std::ffi::c_char,
        flags: i32,
        cb: *const c_void,
    ) -> i32;
    pub fn uv_fs_fsync(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        file: i32,
        cb: *const c_void,
    ) -> i32;
    pub fn uv_fs_chmod(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        path: *const std::ffi::c_char,
        mode: i32,
        cb: *const c_void,
    ) -> i32;
    #[cfg(unix)]
    pub fn uv_fs_fchmod(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        file: i32,
        mode: i32,
        cb: *const c_void,
    ) -> i32;
    #[cfg(unix)]
    pub fn uv_fs_chown(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        path: *const std::ffi::c_char,
        uid: uv_uid_t,
        gid: uv_gid_t,
        cb: *const c_void,
    ) -> i32;
    #[cfg_attr(not(unix), cfg(test))]
    pub fn uv_fs_fchown(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        file: i32,
        uid: uv_uid_t,
        gid: uv_gid_t,
        cb: *const c_void,
    ) -> i32;
    #[cfg(unix)]
    pub fn uv_fs_utime(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        path: *const std::ffi::c_char,
        atime: f64,
        mtime: f64,
        cb: *const c_void,
    ) -> i32;
    pub fn uv_fs_rename(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        path: *const std::ffi::c_char,
        new_path: *const std::ffi::c_char,
        cb: *const c_void,
    ) -> i32;
    pub fn uv_fs_mkdir(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        path: *const std::ffi::c_char,
        mode: i32,
        cb: *const c_void,
    ) -> i32;
    pub fn uv_fs_mkdtemp(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        tpl: *const std::ffi::c_char,
        cb: *const c_void,
    ) -> i32;
    pub fn uv_fs_rmdir(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        path: *const std::ffi::c_char,
        cb: *const c_void,
    ) -> i32;
    pub fn uv_fs_scandir(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        path: *const std::ffi::c_char,
        flags: i32,
        cb: *const c_void,
    ) -> i32;
    pub fn uv_fs_scandir_next(
        req: *mut crate::os::fs::uv_fs_buf_t,
        ent: *mut crate::os::fs::uv_dirent_t,
    ) -> i32;
    pub fn uv_fs_unlink(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        path: *const std::ffi::c_char,
        cb: *const c_void,
    ) -> i32;
    pub fn uv_fs_realpath(
        loop_: *mut c_void,
        req: *mut crate::os::fs::uv_fs_buf_t,
        path: *const std::ffi::c_char,
        cb: *const c_void,
    ) -> i32;
}

#[inline]
pub fn loop_queues(loop_: &mut Loop) -> &'static mut Loop {
    let p: *mut Loop = loop_;
    unsafe { &mut *p }
}

#[inline]
pub fn main_loop_ptr() -> &'static mut Loop {
    &mut crate::globals::g_st().main_loop
}

#[inline]
pub fn main_loop_events() -> *mut MultiQueue {
    crate::globals::g_st().main_loop.events
}

#[inline]
pub fn loop_children_watcher(loop_: &mut Loop) -> *mut uv_signal_t {
    (&raw mut loop_.children_watcher).cast()
}

#[inline]
pub fn loop_children_kill_timer(loop_: &mut Loop) -> *mut uv_timer_t {
    (&raw mut loop_.children_kill_timer).cast()
}

#[inline]
pub fn loop_poll_timer(loop_: &mut Loop) -> *mut uv_timer_t {
    (&raw mut loop_.poll_timer).cast()
}

#[inline]
pub fn loop_exit_delay_timer(loop_: &mut Loop) -> *mut uv_timer_t {
    (&raw mut loop_.exit_delay_timer).cast()
}

#[inline]
pub fn loop_async(loop_: &mut Loop) -> *mut uv_async_t {
    (&raw mut loop_.async_).cast()
}

#[inline]
pub fn loop_mutex(loop_: &mut Loop) -> *mut uv_mutex_t {
    (&raw mut loop_.mutex).cast()
}

#[inline]
pub fn loop_recursive_ptr(loop_: &mut Loop) -> *mut i32 {
    &raw mut loop_.recursive
}

#[inline]
pub fn loop_closing_ptr(loop_: &mut Loop) -> *mut bool {
    &raw mut loop_.closing
}

pub fn loop_init(loop_: &mut Loop) {
    unsafe {
        debug_assert!(uv_loop_size() <= size_of::<[u64; 192]>());
        let uv_ptr: *mut uv_loop_t = (&raw mut loop_.uv).cast();
        uv_loop_init(uv_ptr);
        loop_.recursive = 0;
        loop_.closing = false;
        *uv_ptr.cast::<*mut Loop>() = &raw mut *loop_;

        loop_.children.clear();
        loop_.events = multiqueue_new(&raw mut *loop_);
        loop_.fast_events = multiqueue_new_child(&mut *loop_.events);
        loop_.thread_events = multiqueue_new(ptr::null_mut());

        uv_mutex_init(loop_mutex(loop_));
        uv_async_init(uv_ptr, loop_async(loop_), Some(async_cb));
        uv_signal_init(uv_ptr, loop_children_watcher(loop_));
        uv_timer_init(uv_ptr, loop_children_kill_timer(loop_));
        let poll_timer = loop_poll_timer(loop_);
        uv_timer_init(uv_ptr, poll_timer);
        uv_timer_init(uv_ptr, loop_exit_delay_timer(loop_));
        loop_.timeout_expired = false;
        *poll_timer.cast::<*mut bool>() = &raw mut loop_.timeout_expired;
    }
}

/// Process `Loop.uv` events with a timeout.
fn loop_uv_run(loop_: &mut Loop, ms: i64) -> bool {
    let old_rec = loop_.recursive;
    loop_.recursive = old_rec + 1;
    if old_rec != 0 {
        std::process::abort(); // Should not re-enter uv_run
    }

    let mut mode: i32 = UV_RUN_ONCE;
    let poll_timer = loop_poll_timer(loop_);
    let timeout_expired = unsafe { *poll_timer.cast::<*mut bool>() };
    unsafe {
        *timeout_expired = false;
    }

    if ms > 0 {
        unsafe { uv_timer_start(poll_timer, Some(timer_cb), ms as u64, ms as u64) };
    } else if ms == 0 {
        mode = UV_RUN_NOWAIT;
    }

    unsafe { uv_run((&raw mut loop_.uv).cast(), mode) };

    if ms > 0 {
        unsafe { uv_timer_stop(poll_timer) };
    }

    loop_.recursive -= 1; // Can re-enter uv_run now
    unsafe { ptr::read_volatile(timeout_expired) }
}

/// Processes one `Loop.uv` event (at most).
/// Processes all `Loop.fast_events` events.
/// Does NOT process `Loop.events`, that is an application-specific decision.
pub fn loop_poll_events(loop_: &mut Loop, ms: i64) -> bool {
    let timeout_expired = loop_uv_run(loop_, ms);
    multiqueue_process_events(unsafe { &mut *loop_.fast_events });
    timeout_expired
}

/// Schedules a fast event from another thread.
pub fn loop_schedule_fast(loop_: &mut Loop, event: Event) {
    unsafe {
        let mutex = loop_mutex(loop_);
        uv_mutex_lock(mutex);
        multiqueue_put_event(&mut *loop_.thread_events, event);
        uv_async_send(loop_async(loop_));
        uv_mutex_unlock(mutex);
    }
}

/// Schedules an event from another thread. Unlike loop_schedule_fast(), the
/// event is forwarded to `Loop.events`, instead of being processed immediately.
pub fn loop_schedule_deferred(loop_: &mut Loop, event: Event) {
    let loop_ptr = &raw mut *loop_;
    loop_schedule_fast(
        loop_,
        Event::LoopDeferred {
            loop_: loop_ptr,
            event: Box::new(event),
        },
    );
}

pub fn loop_deferred_event(loop_: &mut Loop, event: Event) {
    multiqueue_put_event(unsafe { &mut *loop_.events }, event);
}

pub fn loop_on_put(loop_: &mut Loop) {
    if loop_.recursive != 0 {
        unsafe { uv_stop((&raw mut loop_.uv).cast()) };
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
pub fn loop_close(loop_: &mut Loop, wait: bool) -> bool {
    let mut rv = true;
    unsafe {
        *loop_closing_ptr(loop_) = true;
    }
    unsafe { uv_mutex_destroy(loop_mutex(loop_)) };
    unsafe { uv_close(loop_children_watcher(loop_).cast(), None) };
    unsafe { uv_close(loop_children_kill_timer(loop_).cast(), None) };
    unsafe { uv_close(loop_poll_timer(loop_).cast(), None) };
    unsafe { uv_close(loop_exit_delay_timer(loop_).cast(), None) };
    unsafe { uv_close(loop_async(loop_).cast(), None) };
    let start = if wait { os_hrtime() } else { 0 };
    let mut didstop = false;
    loop {
        unsafe {
            uv_run(
                (&raw mut loop_.uv).cast(),
                if didstop {
                    UV_RUN_DEFAULT
                } else {
                    UV_RUN_NOWAIT
                },
            )
        };
        if unsafe { uv_loop_close((&raw mut loop_.uv).cast()) } != UV_EBUSY || !wait {
            break;
        }
        let elapsed_s = (os_hrtime() - start) / NS_PER_SEC;
        if elapsed_s >= 2 {
            rv = false;
            logmsg!(
                LOGLVL_ERR,
                ptr::null(),
                c"loop_close",
                line!() as i32,
                true,
                c"uv_loop_close() hang?",
            );
            log_uv_handles((&raw mut loop_.uv).cast());
            break;
        }
        if !didstop {
            unsafe { uv_stop((&raw mut loop_.uv).cast()) };
            unsafe {
                uv_walk(
                    (&raw mut loop_.uv).cast(),
                    Some(loop_walk_cb),
                    ptr::null_mut(),
                )
            };
            didstop = true;
        }
    }
    let qc = loop_queues(loop_);
    multiqueue_free(qc.fast_events);
    multiqueue_free(qc.thread_events);
    multiqueue_free(qc.events);
    qc.children = Vec::new();
    rv
}

pub fn loop_purge(loop_: &mut Loop) {
    unsafe {
        let mutex = loop_mutex(loop_);
        let qc = loop_queues(loop_);
        uv_mutex_lock(mutex);
        multiqueue_purge_events(&mut *qc.thread_events);
        multiqueue_purge_events(&mut *qc.fast_events);
        uv_mutex_unlock(mutex);
    }
}

pub fn loop_size(loop_: &mut Loop) -> usize {
    unsafe {
        let mutex = loop_mutex(loop_);
        let qc = loop_queues(loop_);
        uv_mutex_lock(mutex);
        let rv = multiqueue_size(&*qc.thread_events);
        uv_mutex_unlock(mutex);
        rv
    }
}

unsafe extern "C" fn async_cb(handle: *mut uv_async_t) {
    unsafe {
        let uv_loop = uv_handle_get_loop(handle.cast());
        let l = *uv_loop.cast::<*mut Loop>();
        let mutex = loop_mutex(&mut *l);
        let qc = loop_queues(&mut *l);
        uv_mutex_lock(mutex);
        multiqueue_move_events(&mut *qc.fast_events, &mut *qc.thread_events);
        uv_mutex_unlock(mutex);
    }
}

unsafe extern "C" fn timer_cb(handle: *mut uv_timer_t) {
    unsafe {
        let timeout_expired = *handle.cast::<*mut bool>();
        *timeout_expired = true;
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
        const UV_ASYNC: i32 = 1;
        const UV_IDLE: i32 = 6;
        const UV_PROCESS: i32 = 10;
        const UV_TIMER: i32 = 13;
        const UV_SIGNAL: i32 = 16;

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
