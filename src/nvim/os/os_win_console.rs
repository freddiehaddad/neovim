//! Windows console management (`src/nvim/os/os_win_console.c`).

#![allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    unused_imports,
    clippy::missing_safety_doc,
    clippy::upper_case_acronyms
)]

use crate::globals::*;
use crate::log::logmsg;
#[cfg(windows)]
use std::ffi::{CStr, c_char, c_int, c_uint, c_ulong, c_void};

#[cfg(windows)]
use crate::memory::xfree;

#[cfg(windows)]
type HANDLE = *mut c_void;
#[cfg(windows)]
type HWND = *mut c_void;
#[cfg(windows)]
type HICON = *mut c_void;
#[cfg(windows)]
type DWORD = c_ulong;
#[cfg(windows)]
type BOOL = c_int;
#[cfg(windows)]
type UINT = c_uint;
#[cfg(windows)]
type WPARAM = usize;
#[cfg(windows)]
type LPARAM = isize;
#[cfg(windows)]
type LRESULT = isize;

#[cfg(windows)]
const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;
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
const O_RDONLY: c_int = 0;
#[cfg(windows)]
const STDIN_FILENO: c_int = 0;
#[cfg(windows)]
const STDOUT_FILENO: c_int = 1;
#[cfg(windows)]
const STDERR_FILENO: c_int = 2;
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
const MAXPATHL: usize = 4096;
#[cfg(windows)]
const LOGLVL_INF: c_int = 1;
#[cfg(windows)]
const LOGLVL_WRN: c_int = 2;

#[cfg(windows)]
static mut ORIG_TITLE: [c_char; 256] = [0; 256];
#[cfg(windows)]
static mut H_WND: HWND = std::ptr::null_mut();
#[cfg(windows)]
static mut H_ORIG_ICON_SMALL: HICON = std::ptr::null_mut();
#[cfg(windows)]
static mut H_ORIG_ICON: HICON = std::ptr::null_mut();
#[cfg(windows)]
static mut CONSOLE_INHERITED: bool = false;

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
    fn GetLastError() -> DWORD;
    fn GetConsoleTitleA(lpConsoleTitle: *mut c_char, nSize: DWORD) -> DWORD;
    fn SetConsoleTitleA(lpConsoleTitle: *const c_char) -> BOOL;
    fn GetConsoleMode(hConsoleHandle: HANDLE, lpMode: *mut DWORD) -> BOOL;
    fn SetConsoleMode(hConsoleHandle: HANDLE, dwMode: DWORD) -> BOOL;
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
unsafe extern "C" {
    fn _open_osfhandle(osfhandle: isize, flags: c_int) -> c_int;
    fn _get_osfhandle(fd: c_int) -> isize;
    fn _close(fd: c_int) -> c_int;
}

#[cfg(all(windows, not(test)))]
unsafe extern "C" {

    fn ui_active() -> usize;
    fn os_getenv(name: *const c_char) -> *mut c_char;
    fn os_getenv_noalloc(name: *const c_char) -> *const c_char;
    fn os_path_exists(path: *const c_char) -> bool;
    fn vim_snprintf(str: *mut c_char, str_m: usize, fmt: *const c_char, ...) -> c_int;
    fn uv_tty_set_vterm_state(state: c_int);
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_enable_ctrl_c() {
    unsafe {
        SetConsoleCtrlHandler(std::ptr::null(), 0);
    }
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_open_conin_fd() -> c_int {
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_clear_hwnd() {
    unsafe {
        H_WND = std::ptr::null_mut();
    }
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_redirect_stdin_to_conin() {
    unsafe {
        _close(STDIN_FILENO);
        let conin_fd = os_open_conin_fd();
        assert_eq!(conin_fd, STDIN_FILENO);
    }
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_redirect_stdout_stderr_to_conout() {
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
        _close(STDOUT_FILENO);
        let conout_fd = _open_osfhandle(conout_handle as isize, 0);
        assert_eq!(conout_fd, STDOUT_FILENO);
        _close(STDERR_FILENO);
        let conerr_fd = _open_osfhandle(conout_handle as isize, 0);
        assert_eq!(conerr_fd, STDERR_FILENO);
    }
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_reattach_console_stdio() {
    unsafe {
        os_enable_ctrl_c();
        os_redirect_stdin_to_conin();
        os_redirect_stdout_stderr_to_conout();
    }
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_swap_to_hidden_console() {
    unsafe {
        FreeConsole();
        AllocConsole();
        ShowWindow(GetConsoleWindow(), SW_HIDE);
        os_reattach_console_stdio();
        CONSOLE_INHERITED = false;
    }
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_attach_parent_console() {
    unsafe {
        CONSOLE_INHERITED =
            !GetConsoleWindow().is_null() || AttachConsole(ATTACH_PARENT_PROCESS) != 0;
        if !CONSOLE_INHERITED {
            #[cfg(not(test))]
            logmsg!(
                LOGLVL_INF,
                std::ptr::null(),
                c"os_attach_parent_console".as_ptr(),
                104,
                true,
                c"parent console attach failed: %lu; allocating hidden console".as_ptr(),
                GetLastError(),
            );
            AllocConsole();
            ShowWindow(GetConsoleWindow(), SW_HIDE);
        }
        os_reattach_console_stdio();
    }
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_detach_ui_console(stdio_ui: bool) {
    #[cfg(test)]
    let _ = stdio_ui;

    #[cfg(not(test))]
    unsafe {
        if stdio_ui || (CONSOLE_INHERITED && ui_active() == 0) {
            os_swap_to_hidden_console();
        }
    }
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_icon_reset() {
    unsafe {
        if H_WND.is_null() {
            return;
        }
        if !H_ORIG_ICON_SMALL.is_null() {
            SendMessageA(H_WND, WM_SETICON, ICON_SMALL, H_ORIG_ICON_SMALL as LPARAM);
        }
        if !H_ORIG_ICON.is_null() {
            SendMessageA(H_WND, WM_SETICON, ICON_BIG, H_ORIG_ICON as LPARAM);
        }
    }
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_icon_init() {
    #[cfg(not(test))]
    unsafe {
        H_WND = GetConsoleWindow();
        if H_WND.is_null() {
            return;
        }

        let vimruntime = os_getenv(c"VIMRUNTIME".as_ptr());
        if !vimruntime.is_null() {
            let namebuff = std::ptr::addr_of_mut!(NameBuff).cast::<c_char>();
            vim_snprintf(namebuff, MAXPATHL, c"%s/neovim.ico".as_ptr(), vimruntime);
            if !os_path_exists(namebuff) {
                logmsg!(
                    LOGLVL_WRN,
                    std::ptr::null(),
                    c"os_icon_init".as_ptr(),
                    148,
                    true,
                    c"neovim.ico not found: %s".as_ptr(),
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
                H_ORIG_ICON_SMALL =
                    SendMessageA(H_WND, WM_SETICON, ICON_SMALL, h_vim_icon as LPARAM) as HICON;
                H_ORIG_ICON =
                    SendMessageA(H_WND, WM_SETICON, ICON_BIG, h_vim_icon as LPARAM) as HICON;
            }
            xfree(vimruntime.cast());
        }
    }
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_title_save() {
    unsafe {
        let ptr = std::ptr::addr_of_mut!(ORIG_TITLE).cast::<c_char>();
        GetConsoleTitleA(ptr, 256);
    }
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_title_reset() {
    unsafe {
        let ptr = std::ptr::addr_of!(ORIG_TITLE).cast::<c_char>();
        SetConsoleTitleA(ptr);
    }
}

#[cfg(windows)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_tty_guess_term(term: *mut *const c_char, out_fd: c_int) {
    #[cfg(test)]
    let _ = (term, out_fd);

    #[cfg(not(test))]
    unsafe {
        let conemu_env = os_getenv_noalloc(c"ConEmuANSI".as_ptr());
        let conemu_ansi = !conemu_env.is_null() && CStr::from_ptr(conemu_env).to_bytes() == b"ON";
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
        unsafe {
            super::os_clear_hwnd();
            super::os_icon_reset();
        }
    }
}
