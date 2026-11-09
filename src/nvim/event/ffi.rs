//! Raw `libuv` C FFI types and `extern "C"` declarations for the event subsystem.

use std::ffi::c_void;

pub(crate) const UV_RUN_DEFAULT: i32 = 0;
pub(crate) const UV_RUN_ONCE: i32 = 1;
pub(crate) const UV_RUN_NOWAIT: i32 = 2;
pub(crate) const UV_REQ: i32 = 1;
pub(crate) const UV_TTY_MODE_NORMAL: i32 = 0;

#[cfg(windows)]
pub(crate) const UV_EBUSY: i32 = -4082;
#[cfg(not(windows))]
pub(crate) const UV_EBUSY: i32 = -16;

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

#[repr(transparent)]
#[derive(Debug, Default)]
pub struct VolatileBool(std::cell::UnsafeCell<bool>);

impl VolatileBool {
    #[inline]
    pub const fn new(val: bool) -> Self {
        Self(std::cell::UnsafeCell::new(val))
    }

    #[inline]
    pub fn get(&self) -> bool {
        unsafe { std::ptr::read_volatile(self.0.get()) }
    }

    #[inline]
    pub fn set(&self, val: bool) {
        unsafe { std::ptr::write_volatile(self.0.get(), val) }
    }
}

#[repr(C, align(8))]
pub struct UvMutex([u8; 128]);

impl UvMutex {
    pub const fn new() -> Self {
        Self([0; 128])
    }

    #[inline]
    pub fn init_recursive(&mut self) {
        unsafe { uv_mutex_init_recursive((&raw mut self.0).cast()) };
    }

    #[inline]
    pub fn lock(&mut self) {
        unsafe { uv_mutex_lock((&raw mut self.0).cast()) };
    }

    #[inline]
    pub fn unlock(&mut self) {
        unsafe { uv_mutex_unlock((&raw mut self.0).cast()) };
    }
}

impl Default for UvMutex {
    fn default() -> Self {
        Self::new()
    }
}

#[inline]
pub fn loop_print_all_handles(loop_: &mut uv_loop_t, file: &crate::os::fs::CFile) {
    unsafe { uv_print_all_handles(&raw mut *loop_, file.as_nonnull().as_ptr()) };
}

#[inline]
pub fn uv_strerror(err: i32) -> &'static std::ffi::CStr {
    let p = unsafe { c_uv_strerror(err) };
    if p.is_null() {
        c""
    } else {
        unsafe { std::ffi::CStr::from_ptr(p) }
    }
}

#[inline]
pub fn uv_err_name(err: i32) -> &'static std::ffi::CStr {
    let p = unsafe { c_uv_err_name(err) };
    if p.is_null() {
        c""
    } else {
        unsafe { std::ffi::CStr::from_ptr(p) }
    }
}

#[inline]
pub fn uv_sleep_safe(msec: u32) {
    unsafe {
        uv_sleep(msec);
    }
}

unsafe extern "C" {
    pub fn uv_loop_size() -> usize;
    pub fn uv_handle_size(h_type: i32) -> usize;
    pub fn uv_req_size(r_type: i32) -> usize;
    pub fn uv_handle_get_loop(handle: *const uv_handle_t) -> *mut uv_loop_t;
    pub fn uv_handle_get_type(handle: *const uv_handle_t) -> i32;
    pub fn uv_guess_handle(file: i32) -> i32;
    #[link_name = "uv_strerror"]
    fn c_uv_strerror(err: i32) -> *const std::ffi::c_char;
    #[link_name = "uv_err_name"]
    fn c_uv_err_name(err: i32) -> *const std::ffi::c_char;
    pub fn uv_translate_sys_error(sys_errno: i32) -> i32;

    pub fn uv_loop_init(loop_: *mut uv_loop_t) -> i32;
    pub fn uv_loop_close(loop_: *mut uv_loop_t) -> i32;
    pub fn uv_run(loop_: *mut uv_loop_t, mode: i32) -> i32;
    pub fn uv_stop(loop_: *mut uv_loop_t);
    pub fn uv_update_time(loop_: *mut uv_loop_t);
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
