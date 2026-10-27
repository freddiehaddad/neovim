//! Windows ConPTY pseudo-console support (`src/nvim/os/pty_conpty_win.c`).

#[cfg(windows)]
use crate::event::r#loop::{uv_dlclose, uv_dlopen, uv_dlsym, uv_translate_sys_error};
#[cfg(windows)]
use crate::log::{LOGLVL_ERR, LOGLVL_INF, logmsg};
#[cfg(windows)]
use crate::os::env::os_get_pid;
#[cfg(windows)]
use crate::os::os_win_console::INVALID_HANDLE_VALUE;
#[cfg(windows)]
use std::ffi::{CString, c_char, c_int, c_long, c_void};

#[cfg(windows)]
use crate::os::os_win_console::{BOOL, DWORD, HANDLE};
#[cfg(windows)]
pub type HPCON = *mut c_void;
#[cfg(windows)]
pub type HRESULT = c_long;

#[cfg(windows)]
use crate::extmark::TriState;

#[cfg(windows)]
const PIPE_ACCESS_INBOUND: DWORD = 0x0000_0001;
#[cfg(windows)]
const PIPE_ACCESS_OUTBOUND: DWORD = 0x0000_0002;
#[cfg(windows)]
const FILE_FLAG_FIRST_PIPE_INSTANCE: DWORD = 0x0008_0000;
#[cfg(windows)]
const PIPE_TYPE_BYTE: DWORD = 0x0000_0000;
#[cfg(windows)]
const PIPE_READMODE_BYTE: DWORD = 0x0000_0000;
#[cfg(windows)]
const PIPE_WAIT: DWORD = 0x0000_0000;
#[cfg(windows)]
const EXTENDED_STARTUPINFO_PRESENT: DWORD = 0x0008_0000;
#[cfg(windows)]
const CREATE_UNICODE_ENVIRONMENT: DWORD = 0x0000_0400;
#[cfg(windows)]
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
    pub attr_list: Vec<u8>,
}

#[cfg(windows)]
impl conpty_t {
    pub const fn new() -> Self {
        Self {
            pty: std::ptr::null_mut(),
            si_ex: STARTUPINFOEXW {
                StartupInfo: STARTUPINFOW {
                    cb: 0,
                    lpReserved: std::ptr::null_mut(),
                    lpDesktop: std::ptr::null_mut(),
                    lpTitle: std::ptr::null_mut(),
                    dwX: 0,
                    dwY: 0,
                    dwXSize: 0,
                    dwYSize: 0,
                    dwXCountChars: 0,
                    dwYCountChars: 0,
                    dwFillAttribute: 0,
                    dwFlags: 0,
                    wShowWindow: 0,
                    cbReserved2: 0,
                    lpReserved2: std::ptr::null_mut(),
                    hStdInput: std::ptr::null_mut(),
                    hStdOutput: std::ptr::null_mut(),
                    hStdError: std::ptr::null_mut(),
                },
                lpAttributeList: std::ptr::null_mut(),
            },
            attr_list: Vec::new(),
        }
    }
}

#[cfg(windows)]
use crate::os::dl::UvLib;

#[cfg(windows)]
pub type CreatePseudoConsoleFn =
    Option<unsafe extern "system" fn(COORD, HANDLE, HANDLE, DWORD, *mut HPCON) -> HRESULT>;
#[cfg(windows)]
pub type ResizePseudoConsoleFn = Option<unsafe extern "system" fn(HPCON, COORD) -> HRESULT>;
#[cfg(windows)]
pub type ClosePseudoConsoleFn = Option<unsafe extern "system" fn(HPCON)>;

#[cfg(windows)]
struct ConptyModuleState {
    pCreatePseudoConsole: CreatePseudoConsoleFn,
    pResizePseudoConsole: ResizePseudoConsoleFn,
    pClosePseudoConsole: ClosePseudoConsoleFn,
    HAS_CONPTY: TriState,
    CONPTY_COUNT: c_int,
}

#[cfg(windows)]
impl ConptyModuleState {
    const fn new() -> Self {
        Self {
            pCreatePseudoConsole: None,
            pResizePseudoConsole: None,
            pClosePseudoConsole: None,
            HAS_CONPTY: kNone,
            CONPTY_COUNT: 0,
        }
    }
}

#[cfg(windows)]
static CONPTY_STATE: crate::globals::GlobalState<ConptyModuleState> = crate::globals::GlobalState::new(ConptyModuleState::new());

#[cfg(windows)]
#[inline]
fn conpty_st() -> &'static mut ConptyModuleState {
    unsafe { &mut *CONPTY_STATE.as_ptr() }
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
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
    fn InitializeProcThreadAttributeList(
        lpAttributeList: *mut c_void,
        dwAttributeCount: DWORD,
        dwFlags: DWORD,
        lpSize: *mut usize,
    ) -> BOOL;
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
use crate::extmark::{kFalse, kNone, kTrue};
#[cfg(windows)]
use crate::os::os_win_console::{CloseHandle, GetLastError};

#[cfg(windows)]
pub fn os_has_conpty_working() -> bool {
    if conpty_st().HAS_CONPTY == kNone {
            conpty_st().HAS_CONPTY = os_dyn_conpty_init();
        }
        conpty_st().HAS_CONPTY == kTrue
}

#[cfg(windows)]
pub fn os_dyn_conpty_init() -> TriState {
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
                std::ptr::addr_of_mut!(conpty_st().pCreatePseudoConsole).cast::<*mut c_void>(),
            ),
            (
                c"ResizePseudoConsole".as_ptr(),
                std::ptr::addr_of_mut!(conpty_st().pResizePseudoConsole).cast::<*mut c_void>(),
            ),
            (
                c"ClosePseudoConsole".as_ptr(),
                std::ptr::addr_of_mut!(conpty_st().pClosePseudoConsole).cast::<*mut c_void>(),
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
pub fn os_conpty_init(
    in_name: &mut Option<CString>,
    out_name: &mut Option<CString>,
    width: u16,
    height: u16,
) -> Option<Box<conpty_t>> {
    unsafe {
        let mut conpty_object = Box::new(conpty_t::new());
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
        let count = conpty_st().CONPTY_COUNT;
        let in_pipe_cstr =
            CString::new(format!("\\\\.\\pipe\\nvim-term-in-{pid}-{count}")).unwrap();
        let in_pipe_ptr = in_pipe_cstr.as_ptr();
        *in_name = Some(in_pipe_cstr);

        'init: {
            in_read = CreateNamedPipeA(
                in_pipe_ptr,
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

            let out_pipe_cstr =
                CString::new(format!("\\\\.\\pipe\\nvim-term-out-{pid}-{count}")).unwrap();
            let out_pipe_ptr = out_pipe_cstr.as_ptr();
            *out_name = Some(out_pipe_cstr);
            out_write = CreateNamedPipeA(
                out_pipe_ptr,
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
            let hr = (conpty_st().pCreatePseudoConsole.unwrap_unchecked())(
                size,
                in_read,
                out_write,
                0,
                &mut conpty_object.pty,
            );
            if hr < 0 {
                emsg = c"create pseudo console failed".as_ptr();
                break 'init;
            }

            conpty_object.si_ex.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as DWORD;
            let mut bytes_required: usize = 0;
            InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut bytes_required);
            conpty_object.attr_list = vec![0u8; bytes_required];
            conpty_object.si_ex.lpAttributeList = conpty_object.attr_list.as_mut_ptr().cast();
            if InitializeProcThreadAttributeList(
                conpty_object.si_ex.lpAttributeList,
                1,
                0,
                &mut bytes_required,
            ) == 0
            {
                conpty_object.si_ex.lpAttributeList = std::ptr::null_mut();
                emsg = c"InitializeProcThreadAttributeList failed".as_ptr();
                break 'init;
            }
            if UpdateProcThreadAttribute(
                conpty_object.si_ex.lpAttributeList,
                0,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
                conpty_object.pty,
                std::mem::size_of::<HPCON>(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            ) == 0
            {
                emsg = c"UpdateProcThreadAttribute failed".as_ptr();
                break 'init;
            }

            conpty_st().CONPTY_COUNT += 1;
            if in_read != INVALID_HANDLE_VALUE {
                CloseHandle(in_read);
            }
            if out_write != INVALID_HANDLE_VALUE {
                CloseHandle(out_write);
            }
            return Some(conpty_object);
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
        drop(conpty_object);
        if in_read != INVALID_HANDLE_VALUE {
            CloseHandle(in_read);
        }
        if out_write != INVALID_HANDLE_VALUE {
            CloseHandle(out_write);
        }
        None
    }
}

#[cfg(windows)]
pub unsafe fn os_conpty_spawn(
    conpty_object: &mut conpty_t,
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
            &mut conpty_object.si_ex.StartupInfo,
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
pub fn os_conpty_set_size(conpty_object: &conpty_t, width: u16, height: u16) {
    assert!(width <= i16::MAX as u16);
    assert!(height <= i16::MAX as u16);
    unsafe {
        let size = COORD {
            X: width as i16,
            Y: height as i16,
        };
        if (conpty_st().pResizePseudoConsole.unwrap_unchecked())(conpty_object.pty, size) != S_OK {
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
impl Drop for conpty_t {
    fn drop(&mut self) {
        unsafe {
            if !self.si_ex.lpAttributeList.is_null() {
                DeleteProcThreadAttributeList(self.si_ex.lpAttributeList);
            }
            if !self.pty.is_null()
                && let Some(close_fn) = conpty_st().pClosePseudoConsole
            {
                close_fn(self.pty);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn conpty_new_drops_cleanly() {
        #[cfg(windows)]
        {
            let _ = super::conpty_t::new();
        }
    }
}
