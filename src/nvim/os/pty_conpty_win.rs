//! Windows ConPTY pseudo-console support (`src/nvim/os/pty_conpty_win.c`).

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    clippy::missing_safety_doc,
    clippy::upper_case_acronyms
)]

#[cfg(all(not(test), windows))]
use crate::event::r#loop::{uv_dlclose, uv_dlopen, uv_dlsym, uv_translate_sys_error};
#[cfg(all(not(test), windows))]
use crate::log::logmsg;
#[cfg(all(not(test), windows))]
use crate::os::env::os_get_pid;
#[cfg(all(not(test), windows))]
use std::ffi::c_int;
#[cfg(windows)]
use std::ffi::{c_char, c_long, c_void};

#[cfg(windows)]
use crate::memory::xfree;
#[cfg(all(not(test), windows))]
use crate::memory::{xcalloc, xmalloc, xstrdup};

#[cfg(windows)]
pub use crate::os::os_win_console::{BOOL, DWORD, HANDLE};
#[cfg(windows)]
pub type HPCON = *mut c_void;
#[cfg(windows)]
pub type HRESULT = c_long;

#[cfg(windows)]
pub use crate::extmark::TriState;

#[cfg(all(not(test), windows))]
const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;
#[cfg(all(not(test), windows))]
const PIPE_ACCESS_INBOUND: DWORD = 0x0000_0001;
#[cfg(all(not(test), windows))]
const PIPE_ACCESS_OUTBOUND: DWORD = 0x0000_0002;
#[cfg(all(not(test), windows))]
const FILE_FLAG_FIRST_PIPE_INSTANCE: DWORD = 0x0008_0000;
#[cfg(all(not(test), windows))]
const PIPE_TYPE_BYTE: DWORD = 0x0000_0000;
#[cfg(all(not(test), windows))]
const PIPE_READMODE_BYTE: DWORD = 0x0000_0000;
#[cfg(all(not(test), windows))]
const PIPE_WAIT: DWORD = 0x0000_0000;
#[cfg(windows)]
const EXTENDED_STARTUPINFO_PRESENT: DWORD = 0x0008_0000;
#[cfg(windows)]
const CREATE_UNICODE_ENVIRONMENT: DWORD = 0x0000_0400;
#[cfg(all(not(test), windows))]
const PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE: usize = 0x0002_0016;
#[cfg(windows)]
const S_OK: HRESULT = 0;

#[cfg(windows)]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct COORD {
    pub X: i16,
    pub Y: i16,
}

#[cfg(windows)]
#[repr(C)]
pub struct SECURITY_ATTRIBUTES {
    pub nLength: DWORD,
    pub lpSecurityDescriptor: *mut c_void,
    pub bInheritHandle: BOOL,
}

#[cfg(windows)]
#[repr(C)]
pub struct PROCESS_INFORMATION {
    pub hProcess: HANDLE,
    pub hThread: HANDLE,
    pub dwProcessId: DWORD,
    pub dwThreadId: DWORD,
}

#[cfg(windows)]
#[repr(C)]
pub struct STARTUPINFOW {
    pub cb: DWORD,
    pub lpReserved: *mut u16,
    pub lpDesktop: *mut u16,
    pub lpTitle: *mut u16,
    pub dwX: DWORD,
    pub dwY: DWORD,
    pub dwXSize: DWORD,
    pub dwYSize: DWORD,
    pub dwXCountChars: DWORD,
    pub dwYCountChars: DWORD,
    pub dwFillAttribute: DWORD,
    pub dwFlags: DWORD,
    pub wShowWindow: u16,
    pub cbReserved2: u16,
    pub lpReserved2: *mut u8,
    pub hStdInput: HANDLE,
    pub hStdOutput: HANDLE,
    pub hStdError: HANDLE,
}

#[cfg(windows)]
#[repr(C)]
pub struct STARTUPINFOEXW {
    pub StartupInfo: STARTUPINFOW,
    pub lpAttributeList: *mut c_void,
}

#[cfg(windows)]
#[repr(C)]
pub struct conpty_t {
    pub pty: HPCON,
    pub si_ex: STARTUPINFOEXW,
}

#[cfg(all(not(test), windows))]
use crate::os::dl::UvLib;

#[cfg(windows)]
pub type CreatePseudoConsoleFn =
    Option<unsafe extern "system" fn(COORD, HANDLE, HANDLE, DWORD, *mut HPCON) -> HRESULT>;
#[cfg(windows)]
pub type ResizePseudoConsoleFn = Option<unsafe extern "system" fn(HPCON, COORD) -> HRESULT>;
#[cfg(windows)]
pub type ClosePseudoConsoleFn = Option<unsafe extern "system" fn(HPCON)>;

#[cfg(windows)]
#[unsafe(no_mangle)]
pub static mut pCreatePseudoConsole: CreatePseudoConsoleFn = None;
#[cfg(windows)]
#[unsafe(no_mangle)]
pub static mut pResizePseudoConsole: ResizePseudoConsoleFn = None;
#[cfg(windows)]
#[unsafe(no_mangle)]
pub static mut pClosePseudoConsole: ClosePseudoConsoleFn = None;

#[cfg(windows)]
static mut HAS_CONPTY: TriState = kNone;
#[cfg(all(not(test), windows))]
static mut CONPTY_COUNT: c_int = 0;

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    #[cfg(not(test))]
    fn CreateNamedPipeA(
        lpName: *const c_char,
        dwOpenMode: DWORD,
        dwPipeMode: DWORD,
        nMaxInstances: DWORD,
        nOutBufferSize: DWORD,
        nInBufferSize: DWORD,
        nDefaultTimeOut: DWORD,
        lpSecurityAttributes: *mut SECURITY_ATTRIBUTES,
    ) -> HANDLE;
    #[cfg(not(test))]
    fn InitializeProcThreadAttributeList(
        lpAttributeList: *mut c_void,
        dwAttributeCount: DWORD,
        dwFlags: DWORD,
        lpSize: *mut usize,
    ) -> BOOL;
    #[cfg(not(test))]
    fn UpdateProcThreadAttribute(
        lpAttributeList: *mut c_void,
        dwFlags: DWORD,
        Attribute: usize,
        lpValue: *mut c_void,
        cbSize: usize,
        lpPreviousValue: *mut c_void,
        lpReturnSize: *mut usize,
    ) -> BOOL;
    fn DeleteProcThreadAttributeList(lpAttributeList: *mut c_void);
    fn CreateProcessW(
        lpApplicationName: *const u16,
        lpCommandLine: *mut u16,
        lpProcessAttributes: *mut c_void,
        lpThreadAttributes: *mut c_void,
        bInheritHandles: BOOL,
        dwCreationFlags: DWORD,
        lpEnvironment: *mut c_void,
        lpCurrentDirectory: *const u16,
        lpStartupInfo: *mut STARTUPINFOW,
        lpProcessInformation: *mut PROCESS_INFORMATION,
    ) -> BOOL;
}

#[cfg(windows)]
use crate::api::vim::{kFalse, kNone, kTrue};
#[cfg(all(not(test), windows))]
use crate::log::{LOGLVL_ERR, LOGLVL_INF};
#[cfg(all(not(test), windows))]
use crate::os::os_win_console::{CloseHandle, GetLastError};

#[cfg(windows)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_has_conpty_working() -> bool {
    unsafe {
        if HAS_CONPTY == kNone {
            HAS_CONPTY = os_dyn_conpty_init();
        }
        HAS_CONPTY == kTrue
    }
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_dyn_conpty_init() -> TriState {
    #[cfg(test)]
    {
        kFalse
    }

    #[cfg(not(test))]
    unsafe {
        let mut conpty_lib = UvLib {
            _opaque: [std::ptr::null_mut(); 4],
        };

        if uv_dlopen(c"conpty.dll".as_ptr(), &mut conpty_lib) == 0 {
            logmsg!(
                LOGLVL_INF,
                std::ptr::null(),
                c"os_dyn_conpty_init".as_ptr(),
                31,
                true,
                c"Loaded new ConPTY from conpty.dll".as_ptr(),
            );
        } else {
            uv_dlclose(&mut conpty_lib);
            if uv_dlopen(c"kernel32.dll".as_ptr(), &mut conpty_lib) != 0 {
                uv_dlclose(&mut conpty_lib);
                return kFalse;
            }
            logmsg!(
                LOGLVL_INF,
                std::ptr::null(),
                c"os_dyn_conpty_init".as_ptr(),
                40,
                true,
                c"Loaded system ConPTY from kernel32.dll".as_ptr(),
            );
        }

        let entries: [(*const c_char, *mut *mut c_void); 3] = [
            (
                c"CreatePseudoConsole".as_ptr(),
                std::ptr::addr_of_mut!(pCreatePseudoConsole).cast::<*mut c_void>(),
            ),
            (
                c"ResizePseudoConsole".as_ptr(),
                std::ptr::addr_of_mut!(pResizePseudoConsole).cast::<*mut c_void>(),
            ),
            (
                c"ClosePseudoConsole".as_ptr(),
                std::ptr::addr_of_mut!(pClosePseudoConsole).cast::<*mut c_void>(),
            ),
        ];

        for (name, ptr) in entries {
            if uv_dlsym(&mut conpty_lib, name, ptr) != 0 {
                uv_dlclose(&mut conpty_lib);
                return kFalse;
            }
        }

        kTrue
    }
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_conpty_init(
    in_name: *mut *mut c_char,
    out_name: *mut *mut c_char,
    width: u16,
    height: u16,
) -> *mut conpty_t {
    #[cfg(test)]
    {
        let _ = (in_name, out_name, width, height);
        std::ptr::null_mut()
    }

    #[cfg(not(test))]
    unsafe {
        let mut conpty_object = xcalloc(1, std::mem::size_of::<conpty_t>()).cast::<conpty_t>();
        let emsg: *const c_char;
        let in_read: HANDLE;
        let mut out_write: HANDLE = INVALID_HANDLE_VALUE;
        let mut sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as DWORD,
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: 0,
        };
        let mode = PIPE_ACCESS_INBOUND | PIPE_ACCESS_OUTBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE;

        let pid = os_get_pid();
        let count = CONPTY_COUNT;
        let in_pipe_str = format!("\\\\.\\pipe\\nvim-term-in-{pid}-{count}\0");
        *in_name = xstrdup(in_pipe_str.as_ptr().cast());

        'init: {
            in_read = CreateNamedPipeA(
                *in_name,
                mode,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                1,
                0,
                0,
                30000,
                &mut sa,
            );
            if in_read == INVALID_HANDLE_VALUE {
                emsg = c"create input pipe failed".as_ptr();
                break 'init;
            }

            let out_pipe_str = format!("\\\\.\\pipe\\nvim-term-out-{pid}-{count}\0");
            *out_name = xstrdup(out_pipe_str.as_ptr().cast());
            out_write = CreateNamedPipeA(
                *out_name,
                mode,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                1,
                0,
                0,
                30000,
                &mut sa,
            );
            if out_write == INVALID_HANDLE_VALUE {
                emsg = c"create output pipe failed".as_ptr();
                break 'init;
            }

            assert!(width <= i16::MAX as u16);
            assert!(height <= i16::MAX as u16);
            let size = COORD {
                X: width as i16,
                Y: height as i16,
            };
            let hr = (pCreatePseudoConsole.unwrap_unchecked())(
                size,
                in_read,
                out_write,
                0,
                &mut (*conpty_object).pty,
            );
            if hr < 0 {
                emsg = c"create pseudo console failed".as_ptr();
                break 'init;
            }

            (*conpty_object).si_ex.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as DWORD;
            let mut bytes_required: usize = 0;
            InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut bytes_required);
            (*conpty_object).si_ex.lpAttributeList = xmalloc(bytes_required);
            if InitializeProcThreadAttributeList(
                (*conpty_object).si_ex.lpAttributeList,
                1,
                0,
                &mut bytes_required,
            ) == 0
            {
                emsg = c"InitializeProcThreadAttributeList failed".as_ptr();
                break 'init;
            }
            if UpdateProcThreadAttribute(
                (*conpty_object).si_ex.lpAttributeList,
                0,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
                (*conpty_object).pty,
                std::mem::size_of::<HPCON>(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            ) == 0
            {
                emsg = c"UpdateProcThreadAttribute failed".as_ptr();
                break 'init;
            }

            CONPTY_COUNT += 1;
            if in_read != INVALID_HANDLE_VALUE {
                CloseHandle(in_read);
            }
            if out_write != INVALID_HANDLE_VALUE {
                CloseHandle(out_write);
            }
            return conpty_object;
        }

        logmsg!(
            LOGLVL_ERR,
            std::ptr::null(),
            c"os_conpty_init".as_ptr(),
            139,
            true,
            c"os_conpty_init:%s : error code: %d".as_ptr(),
            emsg,
            uv_translate_sys_error(GetLastError() as c_int),
        );
        os_conpty_free(conpty_object.cast());
        conpty_object = std::ptr::null_mut();
        if in_read != INVALID_HANDLE_VALUE {
            CloseHandle(in_read);
        }
        if out_write != INVALID_HANDLE_VALUE {
            CloseHandle(out_write);
        }
        conpty_object
    }
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_conpty_spawn(
    conpty_object: *mut conpty_t,
    proc_handle: *mut HANDLE,
    name: *mut u16,
    cmd_line: *mut u16,
    cwd: *mut u16,
    env: *mut u16,
) -> bool {
    unsafe {
        let mut pi = PROCESS_INFORMATION {
            hProcess: std::ptr::null_mut(),
            hThread: std::ptr::null_mut(),
            dwProcessId: 0,
            dwThreadId: 0,
        };
        if CreateProcessW(
            name,
            cmd_line,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            env.cast(),
            cwd,
            &mut (*conpty_object).si_ex.StartupInfo,
            &mut pi,
        ) == 0
        {
            return false;
        }
        *proc_handle = pi.hProcess;
        true
    }
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_conpty_set_size(conpty_object: *mut conpty_t, width: u16, height: u16) {
    assert!(width <= i16::MAX as u16);
    assert!(height <= i16::MAX as u16);
    unsafe {
        let size = COORD {
            X: width as i16,
            Y: height as i16,
        };
        if (pResizePseudoConsole.unwrap_unchecked())((*conpty_object).pty, size) != S_OK {
            #[cfg(not(test))]
            logmsg!(
                LOGLVL_ERR,
                std::ptr::null(),
                c"os_conpty_set_size".as_ptr(),
                177,
                true,
                c"ResizePseudoConsole failed: error code: %d".as_ptr(),
                uv_translate_sys_error(GetLastError() as c_int),
            );
        }
    }
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_conpty_free(data: *mut c_void) {
    unsafe {
        let conpty_object = data.cast::<conpty_t>();
        if !conpty_object.is_null() {
            if !(*conpty_object).si_ex.lpAttributeList.is_null() {
                DeleteProcThreadAttributeList((*conpty_object).si_ex.lpAttributeList);
                xfree((*conpty_object).si_ex.lpAttributeList);
            }
            if !(*conpty_object).pty.is_null()
                && let Some(close_fn) = pClosePseudoConsole
            {
                close_fn((*conpty_object).pty);
            }
        }
        xfree(conpty_object.cast());
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn conpty_free_handles_null() {
        #[cfg(windows)]
        unsafe {
            super::os_conpty_free(std::ptr::null_mut());
        }
    }
}
