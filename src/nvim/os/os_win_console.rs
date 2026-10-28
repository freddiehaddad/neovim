//! Windows console management (`src/nvim/os/os_win_console.c`).

#[cfg(windows)]
use crate::globals::*;
#[cfg(windows)]
use crate::log::{LOGLVL_INF, LOGLVL_WRN, logmsg};
#[cfg(windows)]
use crate::os::env::{os_getenv_buf, os_getenv_cstr};
#[cfg(windows)]
use crate::os::fs::os_path_exists;
#[cfg(windows)]
use crate::strings::vim_snprintf;
#[cfg(windows)]
use crate::ui::ui_active;
#[cfg(windows)]
use std::ffi::{c_char, c_int, c_uint, c_ulong, c_void};

#[cfg(windows)]
pub type HANDLE = *mut c_void;
#[cfg(windows)]
type HWND = *mut c_void;
#[cfg(windows)]
type HICON = *mut c_void;
#[cfg(windows)]
pub type DWORD = c_ulong;
#[cfg(windows)]
pub type BOOL = c_int;
#[cfg(windows)]
type UINT = c_uint;
#[cfg(windows)]
type WPARAM = usize;
#[cfg(windows)]
type LPARAM = isize;
#[cfg(windows)]
type LRESULT = isize;

#[cfg(windows)]
pub(crate) const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;
#[cfg(windows)]
const GENERIC_READ: DWORD = 0x8000_0000;
#[cfg(windows)]
const GENERIC_WRITE: DWORD = 0x4000_0000;
#[cfg(windows)]
const FILE_SHARE_READ: DWORD = 0x0000_0001;
#[cfg(windows)]
const FILE_SHARE_WRITE: DWORD = 0x0000_0002;
#[cfg(windows)]
const OPEN_EXISTING: DWORD = 3;
#[cfg(windows)]
const SW_HIDE: c_int = 0;
#[cfg(windows)]
const ATTACH_PARENT_PROCESS: DWORD = 0xFFFF_FFFF;
#[cfg(windows)]
const WM_SETICON: UINT = 0x0080;
#[cfg(windows)]
const ICON_SMALL: WPARAM = 0;
#[cfg(windows)]
const ICON_BIG: WPARAM = 1;
#[cfg(windows)]
const IMAGE_ICON: UINT = 1;
#[cfg(windows)]
const LR_LOADFROMFILE: UINT = 0x0000_0010;
#[cfg(windows)]
const LR_LOADMAP3DCOLORS: UINT = 0x0000_1000;
#[cfg(windows)]
const ENABLE_VIRTUAL_TERMINAL_PROCESSING: DWORD = 0x0004;
#[cfg(windows)]
const UV_TTY_SUPPORTED: c_int = 0;

#[cfg(windows)]
struct WinConsoleState {
    #[allow(non_snake_case)]
    ORIG_TITLE: [c_char; 256],
    #[allow(non_snake_case)]
    H_WND: HWND,
    #[allow(non_snake_case)]
    H_ORIG_ICON_SMALL: HICON,
    #[allow(non_snake_case)]
    H_ORIG_ICON: HICON,
    #[allow(non_snake_case)]
    CONSOLE_INHERITED: bool,
}

#[cfg(windows)]
impl WinConsoleState {
    const fn new() -> Self {
        Self {
            ORIG_TITLE: [0; 256],
            H_WND: std::ptr::null_mut(),
            H_ORIG_ICON_SMALL: std::ptr::null_mut(),
            H_ORIG_ICON: std::ptr::null_mut(),
            CONSOLE_INHERITED: false,
        }
    }
}

#[cfg(windows)]
static WINCON_STATE: crate::globals::GlobalState<WinConsoleState> = crate::globals::GlobalState::new(WinConsoleState::new());

#[cfg(windows)]
#[inline]
fn wincon_st() -> &'static mut WinConsoleState {
    unsafe { &mut *WINCON_STATE.as_ptr() }
}

#[cfg(windows)]
#[repr(C)]
pub struct OsVersionInfoA {
    #[allow(non_snake_case)]
    pub dwOSVersionInfoSize: u32,
    #[allow(non_snake_case)]
    pub dwMajorVersion: u32,
    #[allow(non_snake_case)]
    pub dwMinorVersion: u32,
    #[allow(non_snake_case)]
    pub dwBuildNumber: u32,
    #[allow(non_snake_case)]
    pub dwPlatformId: u32,
    #[allow(non_snake_case)]
    pub szCSDVersion: [c_char; 128],
}

#[cfg(windows)]
impl OsVersionInfoA {
    pub const fn new() -> Self {
        Self {
            dwOSVersionInfoSize: 0,
            dwMajorVersion: 0,
            dwMinorVersion: 0,
            dwBuildNumber: 0,
            dwPlatformId: 0,
            szCSDVersion: [0; 128],
        }
    }
}

#[cfg(windows)]
impl Default for OsVersionInfoA {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn SetConsoleCtrlHandler(HandlerRoutine: *const c_void, Add: BOOL) -> BOOL;
    fn CreateFileA(
        lpFileName: *const c_char,
        dwDesiredAccess: DWORD,
        dwShareMode: DWORD,
        lpSecurityAttributes: *mut c_void,
        dwCreationDisposition: DWORD,
        dwFlagsAndAttributes: DWORD,
        hTemplateFile: HANDLE,
    ) -> HANDLE;
    fn FreeConsole() -> BOOL;
    fn AllocConsole() -> BOOL;
    fn GetConsoleWindow() -> HWND;
    fn AttachConsole(dwProcessId: DWORD) -> BOOL;
    pub fn GetLastError() -> DWORD;
    fn GetConsoleTitleA(lpConsoleTitle: *mut c_char, nSize: DWORD) -> DWORD;
    fn SetConsoleTitleA(lpConsoleTitle: *const c_char) -> BOOL;
    pub fn GetConsoleMode(hConsoleHandle: HANDLE, lpMode: *mut DWORD) -> BOOL;
    pub fn SetConsoleMode(hConsoleHandle: HANDLE, dwMode: DWORD) -> BOOL;
    pub fn CloseHandle(hObject: HANDLE) -> BOOL;
    pub fn GetProcessId(Process: HANDLE) -> DWORD;
    pub fn OpenProcess(dwDesiredAccess: DWORD, bInheritHandle: BOOL, dwProcessId: DWORD) -> HANDLE;
    pub fn TerminateProcess(hProcess: HANDLE, uExitCode: u32) -> BOOL;
    pub fn DebugBreakProcess(Process: *mut c_void) -> c_int;
    pub fn ExitThread(dwExitCode: u32) -> !;
    pub fn GetVersionExA(lpVersionInformation: &mut OsVersionInfoA) -> c_int;
}

#[cfg(windows)]
#[link(name = "user32")]
unsafe extern "system" {
    fn ShowWindow(hWnd: HWND, nCmdShow: c_int) -> BOOL;
    fn SendMessageA(hWnd: HWND, Msg: UINT, wParam: WPARAM, lParam: LPARAM) -> LRESULT;
    fn LoadImageA(
        hInst: *mut c_void,
        name: *const c_char,
        r#type: UINT,
        cx: c_int,
        cy: c_int,
        fuLoad: UINT,
    ) -> HANDLE;
}

#[cfg(windows)]
use crate::channel::{STDERR_FILENO, STDIN_FILENO, STDOUT_FILENO};
#[cfg(windows)]
use crate::fileio::{_get_osfhandle, _open_osfhandle, O_RDONLY, close};
#[cfg(windows)]
use crate::os::fs::MAXPATHL;

#[cfg(windows)]
use crate::event::r#loop::uv_tty_set_vterm_state;

#[cfg(windows)]
pub fn os_enable_ctrl_c() {
    unsafe {
        SetConsoleCtrlHandler(std::ptr::null(), 0);
    }
}

#[cfg(windows)]
pub fn os_open_conin_fd() -> c_int {
    unsafe {
        let conin_handle = CreateFileA(
            c"CONIN$".as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        );
        assert_ne!(conin_handle, INVALID_HANDLE_VALUE);
        let conin_fd = _open_osfhandle(conin_handle as isize, O_RDONLY);
        assert_ne!(conin_fd, -1);
        conin_fd
    }
}

#[cfg(windows)]
pub fn os_clear_hwnd() {
    wincon_st().H_WND = std::ptr::null_mut();
}

#[cfg(windows)]
pub fn os_redirect_stdin_to_conin() {
    unsafe {
        close(STDIN_FILENO);
        let conin_fd = os_open_conin_fd();
        assert_eq!(conin_fd, STDIN_FILENO);
    }
}

#[cfg(windows)]
pub fn os_redirect_stdout_stderr_to_conout() {
    unsafe {
        let conout_handle = CreateFileA(
            c"CONOUT$".as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        );
        assert_ne!(conout_handle, INVALID_HANDLE_VALUE);
        close(STDOUT_FILENO);
        let conout_fd = _open_osfhandle(conout_handle as isize, 0);
        assert_eq!(conout_fd, STDOUT_FILENO);
        close(STDERR_FILENO);
        let conerr_fd = _open_osfhandle(conout_handle as isize, 0);
        assert_eq!(conerr_fd, STDERR_FILENO);
    }
}

#[cfg(windows)]
pub fn os_reattach_console_stdio() {
    os_enable_ctrl_c();
        os_redirect_stdin_to_conin();
        os_redirect_stdout_stderr_to_conout();
}

#[cfg(windows)]
pub fn os_swap_to_hidden_console() {
    unsafe {
        FreeConsole();
        AllocConsole();
        ShowWindow(GetConsoleWindow(), SW_HIDE);
        os_reattach_console_stdio();
        wincon_st().CONSOLE_INHERITED = false;
    }
}

#[cfg(windows)]
pub fn os_attach_parent_console() {
    unsafe {
        wincon_st().CONSOLE_INHERITED =
            !GetConsoleWindow().is_null() || AttachConsole(ATTACH_PARENT_PROCESS) != 0;
        if !wincon_st().CONSOLE_INHERITED {
            logmsg!(
                LOGLVL_INF,
                std::ptr::null(),
                c"os_attach_parent_console",
                104,
                true,
                c"parent console attach failed: %lu; allocating hidden console",
                GetLastError(),
            );
            AllocConsole();
            ShowWindow(GetConsoleWindow(), SW_HIDE);
        }
        os_reattach_console_stdio();
    }
}

#[cfg(windows)]
pub fn os_detach_ui_console(stdio_ui: bool) {
    if stdio_ui || (wincon_st().CONSOLE_INHERITED && ui_active() == 0) {
            os_swap_to_hidden_console();
        }
}

#[cfg(windows)]
pub fn os_icon_reset() {
    unsafe {
        if wincon_st().H_WND.is_null() {
            return;
        }
        if !wincon_st().H_ORIG_ICON_SMALL.is_null() {
            SendMessageA(wincon_st().H_WND, WM_SETICON, ICON_SMALL, wincon_st().H_ORIG_ICON_SMALL as LPARAM);
        }
        if !wincon_st().H_ORIG_ICON.is_null() {
            SendMessageA(wincon_st().H_WND, WM_SETICON, ICON_BIG, wincon_st().H_ORIG_ICON as LPARAM);
        }
    }
}

#[cfg(windows)]
pub fn os_icon_init() {
    unsafe {
        wincon_st().H_WND = GetConsoleWindow();
        if wincon_st().H_WND.is_null() {
            return;
        }

        if let Some(vimruntime) = os_getenv_cstr(c"VIMRUNTIME") {
            let namebuff = std::ptr::addr_of_mut!(g_st().name_buff).cast::<c_char>();
            vim_snprintf!(
                namebuff,
                MAXPATHL,
                c"%s/neovim.ico",
                vimruntime.as_ptr()
            );
            if !os_path_exists(namebuff) {
                logmsg!(
                    LOGLVL_WRN,
                    std::ptr::null(),
                    c"os_icon_init",
                    148,
                    true,
                    c"neovim.ico not found: %s",
                    namebuff,
                );
            } else {
                let h_vim_icon = LoadImageA(
                    std::ptr::null_mut(),
                    namebuff,
                    IMAGE_ICON,
                    64,
                    64,
                    LR_LOADFROMFILE | LR_LOADMAP3DCOLORS,
                );
                wincon_st().H_ORIG_ICON_SMALL =
                    SendMessageA(wincon_st().H_WND, WM_SETICON, ICON_SMALL, h_vim_icon as LPARAM) as HICON;
                wincon_st().H_ORIG_ICON =
                    SendMessageA(wincon_st().H_WND, WM_SETICON, ICON_BIG, h_vim_icon as LPARAM) as HICON;
            }
        }
    }
}

#[cfg(windows)]
pub fn os_title_save() {
    unsafe {
        let ptr = std::ptr::addr_of_mut!(wincon_st().ORIG_TITLE).cast::<c_char>();
        GetConsoleTitleA(ptr, 256);
    }
}

#[cfg(windows)]
pub fn os_title_reset() {
    unsafe {
        let ptr = std::ptr::addr_of!(wincon_st().ORIG_TITLE).cast::<c_char>();
        SetConsoleTitleA(ptr);
    }
}

#[cfg(windows)]
pub fn os_tty_guess_term(term: *mut *const c_char, out_fd: c_int) {
    unsafe {
        let mut conemu_buf = [0 as c_char; 16];
        let conemu_env = os_getenv_buf(
            c"ConEmuANSI",
            conemu_buf.as_mut_ptr(),
            conemu_buf.len(),
        );
        let conemu_ansi = !conemu_env.is_null() && crate::memory::cstr_as_bytes(conemu_env) == b"ON";
        let mut vtp = false;

        let handle = _get_osfhandle(out_fd) as HANDLE;
        let mut dw_mode: DWORD = 0;
        if handle != INVALID_HANDLE_VALUE && GetConsoleMode(handle, &mut dw_mode) != 0 {
            dw_mode |= ENABLE_VIRTUAL_TERMINAL_PROCESSING;
            if SetConsoleMode(handle, dw_mode) != 0 {
                vtp = true;
            }
        }

        if (*term).is_null() {
            if vtp {
                *term = c"vtpcon".as_ptr();
            } else if conemu_ansi {
                *term = c"conemu".as_ptr();
            } else {
                *term = c"win32con".as_ptr();
            }
        }

        if conemu_ansi {
            uv_tty_set_vterm_state(UV_TTY_SUPPORTED);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn clear_hwnd_and_icon_reset_handle_null() {
        #[cfg(windows)]
        {
            super::os_clear_hwnd();
            super::os_icon_reset();
        }
    }
}
