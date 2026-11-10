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
use std::ffi::{CStr, CString, c_void};

#[cfg(windows)]
use crate::os::os_win_console::{BOOL, DWORD, HANDLE};
#[cfg(windows)]
pub type HPCON = *mut c_void;
#[cfg(windows)]
pub type HRESULT = i32;

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
    #[allow(non_snake_case)]
    pub X: i16,
    #[allow(non_snake_case)]
    pub Y: i16,
}

#[cfg(windows)]
#[repr(C)]
pub struct SECURITY_ATTRIBUTES {
    #[allow(non_snake_case)]
    pub nLength: DWORD,
    #[allow(non_snake_case)]
    pub lpSecurityDescriptor: *mut c_void,
    #[allow(non_snake_case)]
    pub bInheritHandle: BOOL,
}

#[cfg(windows)]
#[repr(C)]
pub struct PROCESS_INFORMATION {
    #[allow(non_snake_case)]
    pub hProcess: HANDLE,
    #[allow(non_snake_case)]
    pub hThread: HANDLE,
    #[allow(non_snake_case)]
    pub dwProcessId: DWORD,
    #[allow(non_snake_case)]
    pub dwThreadId: DWORD,
}

#[cfg(windows)]
#[repr(C)]
pub struct STARTUPINFOW {
    pub cb: DWORD,
    #[allow(non_snake_case)]
    pub lpReserved: *mut u16,
    #[allow(non_snake_case)]
    pub lpDesktop: *mut u16,
    #[allow(non_snake_case)]
    pub lpTitle: *mut u16,
    #[allow(non_snake_case)]
    pub dwX: DWORD,
    #[allow(non_snake_case)]
    pub dwY: DWORD,
    #[allow(non_snake_case)]
    pub dwXSize: DWORD,
    #[allow(non_snake_case)]
    pub dwYSize: DWORD,
    #[allow(non_snake_case)]
    pub dwXCountChars: DWORD,
    #[allow(non_snake_case)]
    pub dwYCountChars: DWORD,
    #[allow(non_snake_case)]
    pub dwFillAttribute: DWORD,
    #[allow(non_snake_case)]
    pub dwFlags: DWORD,
    #[allow(non_snake_case)]
    pub wShowWindow: u16,
    #[allow(non_snake_case)]
    pub cbReserved2: u16,
    #[allow(non_snake_case)]
    pub lpReserved2: *mut u8,
    #[allow(non_snake_case)]
    pub hStdInput: HANDLE,
    #[allow(non_snake_case)]
    pub hStdOutput: HANDLE,
    #[allow(non_snake_case)]
    pub hStdError: HANDLE,
}

#[cfg(windows)]
#[repr(C)]
pub struct STARTUPINFOEXW {
    #[allow(non_snake_case)]
    pub StartupInfo: STARTUPINFOW,
    #[allow(non_snake_case)]
    pub lpAttributeList: *mut c_void,
}

#[cfg(windows)]
#[repr(C)]
pub struct Conpty {
    pub pty: HPCON,
    pub si_ex: STARTUPINFOEXW,
    pub attr_list: Vec<u8>,
}

#[cfg(windows)]
impl Conpty {
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
#[derive(Clone, Copy)]
struct ConptyFns {
    #[allow(non_snake_case)]
    pCreatePseudoConsole:
        unsafe extern "system" fn(COORD, HANDLE, HANDLE, DWORD, *mut HPCON) -> HRESULT,
    #[allow(non_snake_case)]
    pResizePseudoConsole: unsafe extern "system" fn(HPCON, COORD) -> HRESULT,
    #[allow(non_snake_case)]
    pClosePseudoConsole: unsafe extern "system" fn(HPCON),
}

#[cfg(windows)]
static CONPTY_FNS: std::sync::OnceLock<Option<ConptyFns>> = std::sync::OnceLock::new();

#[cfg(windows)]
static CONPTY_COUNT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateNamedPipeA(
        lpName: *const u8,
        dwOpenMode: DWORD,
        dwPipeMode: DWORD,
        nMaxInstances: DWORD,
        nOutBufferSize: DWORD,
        nInBufferSize: DWORD,
        nDefaultTimeOut: DWORD,
        lpSecurityAttributes: &mut SECURITY_ATTRIBUTES,
    ) -> HANDLE;
    fn InitializeProcThreadAttributeList(
        lpAttributeList: *mut c_void,
        dwAttributeCount: DWORD,
        dwFlags: DWORD,
        lpSize: &mut usize,
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
        lpStartupInfo: &mut STARTUPINFOW,
        lpProcessInformation: &mut PROCESS_INFORMATION,
    ) -> BOOL;
}

#[cfg(windows)]
use crate::os::os_win_console::{CloseHandle, GetLastError};

#[cfg(windows)]
pub fn os_has_conpty_working() -> bool {
    conpty_fns().is_some()
}

#[cfg(windows)]
fn conpty_fns() -> Option<ConptyFns> {
    *CONPTY_FNS.get_or_init(|| {
        let mut conpty_lib = UvLib {
            _opaque: [std::ptr::null_mut(); 4],
        };

        if unsafe { uv_dlopen(c"conpty.dll".as_ptr(), &mut conpty_lib) } == 0 {
            logmsg!(
                LOGLVL_INF,
                None,
                Some(c"os_dyn_conpty_init"),
                31,
                true,
                c"Loaded new ConPTY from conpty.dll",
            );
        } else {
            unsafe { uv_dlclose(&mut conpty_lib) };
            if unsafe { uv_dlopen(c"kernel32.dll".as_ptr(), &mut conpty_lib) } != 0 {
                unsafe { uv_dlclose(&mut conpty_lib) };
                return None;
            }
            logmsg!(
                LOGLVL_INF,
                None,
                Some(c"os_dyn_conpty_init"),
                40,
                true,
                c"Loaded system ConPTY from kernel32.dll",
            );
        }

        let mut create_fn: CreatePseudoConsoleFn = None;
        let mut resize_fn: ResizePseudoConsoleFn = None;
        let mut close_fn: ClosePseudoConsoleFn = None;
        let entries: [(&CStr, *mut *mut c_void); 3] = [
            (
                c"CreatePseudoConsole",
                std::ptr::addr_of_mut!(create_fn).cast::<*mut c_void>(),
            ),
            (
                c"ResizePseudoConsole",
                std::ptr::addr_of_mut!(resize_fn).cast::<*mut c_void>(),
            ),
            (
                c"ClosePseudoConsole",
                std::ptr::addr_of_mut!(close_fn).cast::<*mut c_void>(),
            ),
        ];

        for (name, ptr) in entries {
            if unsafe { uv_dlsym(&mut conpty_lib, name.as_ptr(), ptr) } != 0 {
                unsafe { uv_dlclose(&mut conpty_lib) };
                return None;
            }
        }

        Some(ConptyFns {
            pCreatePseudoConsole: create_fn?,
            pResizePseudoConsole: resize_fn?,
            pClosePseudoConsole: close_fn?,
        })
    })
}

#[cfg(windows)]
pub fn os_conpty_init(
    in_name: &mut Option<CString>,
    out_name: &mut Option<CString>,
    width: u16,
    height: u16,
) -> Option<Box<Conpty>> {
    let mut conpty_object = Box::new(Conpty::new());
    let emsg: &CStr;
    let in_read: HANDLE;
    let mut out_write: HANDLE = INVALID_HANDLE_VALUE;
    let mut sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as DWORD,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 0,
    };
    let mode = PIPE_ACCESS_INBOUND | PIPE_ACCESS_OUTBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE;

    let pid = os_get_pid();
    let count = CONPTY_COUNT.load(std::sync::atomic::Ordering::Relaxed);
    let in_pipe_cstr = CString::new(format!("\\\\.\\pipe\\nvim-term-in-{pid}-{count}")).unwrap();
    let in_pipe_ptr = in_pipe_cstr.as_ptr().cast();
    *in_name = Some(in_pipe_cstr);

    'init: {
        in_read = unsafe {
            CreateNamedPipeA(
                in_pipe_ptr,
                mode,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                1,
                0,
                0,
                30000,
                &mut sa,
            )
        };
        if in_read == INVALID_HANDLE_VALUE {
            emsg = c"create input pipe failed";
            break 'init;
        }

        let out_pipe_cstr =
            CString::new(format!("\\\\.\\pipe\\nvim-term-out-{pid}-{count}")).unwrap();
        let out_pipe_ptr = out_pipe_cstr.as_ptr().cast();
        *out_name = Some(out_pipe_cstr);
        out_write = unsafe {
            CreateNamedPipeA(
                out_pipe_ptr,
                mode,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                1,
                0,
                0,
                30000,
                &mut sa,
            )
        };
        if out_write == INVALID_HANDLE_VALUE {
            emsg = c"create output pipe failed";
            break 'init;
        }

        assert!(width <= i16::MAX as u16);
        assert!(height <= i16::MAX as u16);
        let size = COORD {
            X: width as i16,
            Y: height as i16,
        };
        let hr = unsafe {
            (conpty_fns().unwrap().pCreatePseudoConsole)(
                size,
                in_read,
                out_write,
                0,
                &mut conpty_object.pty,
            )
        };
        if hr < 0 {
            emsg = c"create pseudo console failed";
            break 'init;
        }

        conpty_object.si_ex.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as DWORD;
        let mut bytes_required: usize = 0;
        unsafe {
            InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut bytes_required)
        };
        conpty_object.attr_list = vec![0u8; bytes_required];
        conpty_object.si_ex.lpAttributeList = conpty_object.attr_list.as_mut_ptr().cast();
        if unsafe {
            InitializeProcThreadAttributeList(
                conpty_object.si_ex.lpAttributeList,
                1,
                0,
                &mut bytes_required,
            )
        } == 0
        {
            conpty_object.si_ex.lpAttributeList = std::ptr::null_mut();
            emsg = c"InitializeProcThreadAttributeList failed";
            break 'init;
        }
        if unsafe {
            UpdateProcThreadAttribute(
                conpty_object.si_ex.lpAttributeList,
                0,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
                conpty_object.pty,
                std::mem::size_of::<HPCON>(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } == 0
        {
            emsg = c"UpdateProcThreadAttribute failed";
            break 'init;
        }

        CONPTY_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if in_read != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(in_read) };
        }
        if out_write != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(out_write) };
        }
        return Some(conpty_object);
    }

    logmsg!(
        LOGLVL_ERR,
        None,
        Some(c"os_conpty_init"),
        139,
        true,
        c"os_conpty_init:%s : error code: %d",
        emsg,
        unsafe { uv_translate_sys_error(GetLastError() as i32) },
    );
    drop(conpty_object);
    if in_read != INVALID_HANDLE_VALUE {
        unsafe { CloseHandle(in_read) };
    }
    if out_write != INVALID_HANDLE_VALUE {
        unsafe { CloseHandle(out_write) };
    }
    None
}

#[cfg(windows)]
pub fn os_conpty_spawn(
    conpty_object: &mut Conpty,
    proc_handle: &mut HANDLE,
    name: Option<&[u16]>,
    cmd_line: &mut [u16],
    cwd: Option<&[u16]>,
    env: Option<&mut [u16]>,
) -> bool {
    let mut pi = PROCESS_INFORMATION {
        hProcess: std::ptr::null_mut(),
        hThread: std::ptr::null_mut(),
        dwProcessId: 0,
        dwThreadId: 0,
    };
    if unsafe {
        CreateProcessW(
            name.map_or(std::ptr::null(), <[u16]>::as_ptr),
            cmd_line.as_mut_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            env.map_or(std::ptr::null_mut(), |s| s.as_mut_ptr().cast()),
            cwd.map_or(std::ptr::null(), <[u16]>::as_ptr),
            &mut conpty_object.si_ex.StartupInfo,
            &mut pi,
        )
    } == 0
    {
        return false;
    }
    *proc_handle = pi.hProcess;
    true
}

#[cfg(windows)]
pub fn os_conpty_set_size(conpty_object: &Conpty, width: u16, height: u16) {
    assert!(width <= i16::MAX as u16);
    assert!(height <= i16::MAX as u16);
    {
        let size = COORD {
            X: width as i16,
            Y: height as i16,
        };
        if unsafe { (conpty_fns().unwrap().pResizePseudoConsole)(conpty_object.pty, size) } != S_OK
        {
            logmsg!(
                LOGLVL_ERR,
                None,
                Some(c"os_conpty_set_size"),
                177,
                true,
                c"ResizePseudoConsole failed: error code: %d",
                unsafe { uv_translate_sys_error(GetLastError() as i32) },
            );
        }
    }
}

#[cfg(windows)]
impl Drop for Conpty {
    fn drop(&mut self) {
        if !self.si_ex.lpAttributeList.is_null() {
            unsafe { DeleteProcThreadAttributeList(self.si_ex.lpAttributeList) };
        }
        if !self.pty.is_null()
            && let Some(fns) = conpty_fns()
        {
            unsafe { (fns.pClosePseudoConsole)(self.pty) };
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn conpty_new_drops_cleanly() {
        #[cfg(windows)]
        {
            let _ = super::Conpty::new();
        }
    }
}
