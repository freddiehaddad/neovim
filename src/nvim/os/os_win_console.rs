//! Windows console management (`src/nvim/os/os_win_console.c`).

#[cfg(windows)]
use crate::log::{LOGLVL_INF, LOGLVL_WRN, logmsg};
#[cfg(windows)]
use crate::os::env::{os_getenv_buf, os_getenv_cstr};
#[cfg(windows)]
use crate::os::fs::{MAXPATHL, os_path_exists};
#[cfg(windows)]
use crate::strings::format_to_buf;
#[cfg(windows)]
use crate::ui::ui_active;
#[cfg(windows)]
use std::ffi::c_void;

#[cfg(windows)]
pub type HANDLE = *mut c_void;
#[cfg(windows)]
type HWND = *mut c_void;
#[cfg(windows)]
type HICON = *mut c_void;
#[cfg(windows)]
pub type DWORD = u32;
#[cfg(windows)]
pub type BOOL = i32;
#[cfg(windows)]
type UINT = u32;
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
const SW_HIDE: i32 = 0;
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
const UV_TTY_SUPPORTED: i32 = 0;

#[cfg(windows)]
struct WinConsoleState {
    #[allow(non_snake_case)]
    ORIG_TITLE: [u8; 256],
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
std::thread_local! {
    static WINCON_STATE: std::cell::RefCell<WinConsoleState> =
        const { std::cell::RefCell::new(WinConsoleState::new()) };
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
    pub szCSDVersion: [u8; 128],
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
        lpFileName: *const u8,
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
    fn GetConsoleTitleA(lpConsoleTitle: *mut u8, nSize: DWORD) -> DWORD;
    fn SetConsoleTitleA(lpConsoleTitle: *const u8) -> BOOL;
    pub fn GetConsoleMode(hConsoleHandle: HANDLE, lpMode: *mut DWORD) -> BOOL;
    pub fn SetConsoleMode(hConsoleHandle: HANDLE, dwMode: DWORD) -> BOOL;
    pub fn CloseHandle(hObject: HANDLE) -> BOOL;
    pub fn GetProcessId(Process: HANDLE) -> DWORD;
    pub fn OpenProcess(dwDesiredAccess: DWORD, bInheritHandle: BOOL, dwProcessId: DWORD) -> HANDLE;
    pub fn TerminateProcess(hProcess: HANDLE, uExitCode: u32) -> BOOL;
    pub fn DebugBreakProcess(Process: *mut c_void) -> i32;
    pub fn ExitThread(dwExitCode: u32) -> !;
    pub fn GetVersionExA(lpVersionInformation: &mut OsVersionInfoA) -> i32;
}

#[cfg(windows)]
#[link(name = "user32")]
unsafe extern "system" {
    fn ShowWindow(hWnd: HWND, nCmdShow: i32) -> BOOL;
    fn SendMessageA(hWnd: HWND, Msg: UINT, wParam: WPARAM, lParam: LPARAM) -> LRESULT;
    fn LoadImageA(
        hInst: *mut c_void,
        name: *const u8,
        r#type: UINT,
        cx: i32,
        cy: i32,
        fuLoad: UINT,
    ) -> HANDLE;
}

#[cfg(windows)]
use crate::channel::{STDERR_FILENO, STDIN_FILENO, STDOUT_FILENO};
#[cfg(windows)]
use crate::fileio::{_get_osfhandle, _open_osfhandle, O_RDONLY, close};

#[cfg(windows)]
use crate::event::r#loop::uv_tty_set_vterm_state;

#[cfg(windows)]
pub fn os_fill_windows_version(dst: &mut [u8; 20]) {
    use std::io::Write;
    let mut ovi = OsVersionInfoA::new();
    ovi.dwOSVersionInfoSize = std::mem::size_of::<OsVersionInfoA>() as u32;
    unsafe { GetVersionExA(&mut ovi) };
    let mut buf = Vec::with_capacity(20);
    let _ = write!(buf, "{}.{}\0", ovi.dwMajorVersion, ovi.dwMinorVersion);
    let copy_len = buf.len().min(20);
    dst[..copy_len].copy_from_slice(&buf[..copy_len]);
    dst[19] = 0;
}

#[cfg(windows)]
pub fn os_enable_ctrl_c() {
    unsafe {
        SetConsoleCtrlHandler(std::ptr::null(), 0);
    }
}

#[cfg(windows)]
pub fn os_open_conin_fd() -> i32 {
    let conin_handle = unsafe {
        CreateFileA(
            c"CONIN$".as_ptr().cast(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    assert_ne!(conin_handle, INVALID_HANDLE_VALUE);
    let conin_fd = unsafe { _open_osfhandle(conin_handle as isize, O_RDONLY) };
    assert_ne!(conin_fd, -1);
    conin_fd
}

#[cfg(windows)]
pub fn os_clear_hwnd() {
    WINCON_STATE.with_borrow_mut(|st| st.H_WND = std::ptr::null_mut());
}

#[cfg(windows)]
pub fn os_redirect_stdin_to_conin() {
    close(STDIN_FILENO);
    let conin_fd = os_open_conin_fd();
    assert_eq!(conin_fd, STDIN_FILENO);
}

#[cfg(windows)]
pub fn os_redirect_stdout_stderr_to_conout() {
    unsafe {
        let conout_handle = CreateFileA(
            c"CONOUT$".as_ptr().cast(),
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
        WINCON_STATE.with_borrow_mut(|st| st.CONSOLE_INHERITED = false);
    }
}

#[cfg(windows)]
pub fn os_attach_parent_console() {
    unsafe {
        let inherited = !GetConsoleWindow().is_null() || AttachConsole(ATTACH_PARENT_PROCESS) != 0;
        WINCON_STATE.with_borrow_mut(|st| st.CONSOLE_INHERITED = inherited);
        if !inherited {
            logmsg!(
                LOGLVL_INF,
                None,
                Some(c"os_attach_parent_console"),
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
    let inherited = WINCON_STATE.with_borrow(|st| st.CONSOLE_INHERITED);
    if stdio_ui || (inherited && ui_active() == 0) {
        os_swap_to_hidden_console();
    }
}

#[cfg(windows)]
pub fn os_icon_reset() {
    let (h_wnd, h_small, h_big) =
        WINCON_STATE.with_borrow(|st| (st.H_WND, st.H_ORIG_ICON_SMALL, st.H_ORIG_ICON));
    if h_wnd.is_null() {
        return;
    }
    if !h_small.is_null() {
        unsafe { SendMessageA(h_wnd, WM_SETICON, ICON_SMALL, h_small as LPARAM) };
    }
    if !h_big.is_null() {
        unsafe { SendMessageA(h_wnd, WM_SETICON, ICON_BIG, h_big as LPARAM) };
    }
}

#[cfg(windows)]
pub fn os_icon_init() {
    let h_wnd = unsafe { GetConsoleWindow() };
    WINCON_STATE.with_borrow_mut(|st| st.H_WND = h_wnd);
    if h_wnd.is_null() {
        return;
    }

    if let Some(vimruntime) = os_getenv_cstr(c"VIMRUNTIME") {
        let mut name_buff = [0u8; MAXPATHL];
        format_to_buf!(&mut name_buff, c"%s/neovim.ico", &vimruntime);
        let namebuff_cstr = crate::memory::bytes_as_cstr(&name_buff);
        if !os_path_exists(namebuff_cstr) {
            logmsg!(
                LOGLVL_WRN,
                None,
                Some(c"os_icon_init"),
                148,
                true,
                c"neovim.ico not found: %s",
                namebuff_cstr,
            );
        } else {
            let h_vim_icon = unsafe {
                LoadImageA(
                    std::ptr::null_mut(),
                    name_buff.as_ptr().cast(),
                    IMAGE_ICON,
                    64,
                    64,
                    LR_LOADFROMFILE | LR_LOADMAP3DCOLORS,
                )
            };
            let orig_small =
                unsafe { SendMessageA(h_wnd, WM_SETICON, ICON_SMALL, h_vim_icon as LPARAM) }
                    as HICON;
            let orig_big =
                unsafe { SendMessageA(h_wnd, WM_SETICON, ICON_BIG, h_vim_icon as LPARAM) } as HICON;
            WINCON_STATE.with_borrow_mut(|st| {
                st.H_ORIG_ICON_SMALL = orig_small;
                st.H_ORIG_ICON = orig_big;
            });
        }
    }
}

#[cfg(windows)]
pub fn os_title_save() {
    WINCON_STATE.with_borrow_mut(|st| unsafe {
        GetConsoleTitleA(st.ORIG_TITLE.as_mut_ptr(), 256);
    });
}

#[cfg(windows)]
pub fn os_title_reset() {
    WINCON_STATE.with_borrow(|st| unsafe {
        SetConsoleTitleA(st.ORIG_TITLE.as_ptr());
    });
}

#[cfg(windows)]
unsafe extern "system" {
    fn GetConsoleOutputCP() -> u32;
    fn GetConsoleCP() -> u32;
    fn SetConsoleOutputCP(wCodePageID: u32) -> i32;
    fn SetConsoleCP(wCodePageID: u32) -> i32;
}

#[cfg(windows)]
#[inline]
pub fn os_get_console_output_cp() -> u32 {
    unsafe { GetConsoleOutputCP() }
}

#[cfg(windows)]
#[inline]
pub fn os_get_console_cp() -> u32 {
    unsafe { GetConsoleCP() }
}

#[cfg(windows)]
#[inline]
pub fn os_set_console_output_cp(cp: u32) {
    unsafe {
        SetConsoleOutputCP(cp);
    }
}

#[cfg(windows)]
#[inline]
pub fn os_set_console_cp(cp: u32) {
    unsafe {
        SetConsoleCP(cp);
    }
}

#[cfg(windows)]
pub fn os_tty_guess_term(out_fd: i32) -> &'static std::ffi::CStr {
    unsafe {
        let mut conemu_buf = [0u8; 16];
        let conemu_env = os_getenv_buf(c"ConEmuANSI", &mut conemu_buf);
        let conemu_ansi = conemu_env.map(std::ffi::CStr::to_bytes) == Some(b"ON");
        let mut vtp = false;

        let handle = _get_osfhandle(out_fd) as HANDLE;
        let mut dw_mode: DWORD = 0;
        if handle != INVALID_HANDLE_VALUE && GetConsoleMode(handle, &mut dw_mode) != 0 {
            dw_mode |= ENABLE_VIRTUAL_TERMINAL_PROCESSING;
            if SetConsoleMode(handle, dw_mode) != 0 {
                vtp = true;
            }
        }

        if conemu_ansi {
            uv_tty_set_vterm_state(UV_TTY_SUPPORTED);
        }

        if vtp {
            c"vtpcon"
        } else if conemu_ansi {
            c"conemu"
        } else {
            c"win32con"
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
