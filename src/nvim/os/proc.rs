//! OS process inspection and process-tree termination (`src/nvim/os/proc.c`).

#![allow(dead_code, unused_imports, unused_mut)]

use std::ffi::{CStr, c_char, c_int, c_void};

use crate::memory::{Arena, xrealloc};

#[cfg(target_os = "windows")]
use crate::api::private::validate::{Dict, KeyValuePair, Object, ObjectData, ObjectType};
#[cfg(target_os = "windows")]
use crate::map_glyph_cache::NvimString;

#[cfg(target_os = "windows")]
const UV_ESRCH: c_int = -4040;
#[cfg(not(target_os = "windows"))]
const UV_ESRCH: c_int = -3;

const SIGTERM_NUM: c_int = 15;
const SIGKILL_NUM: c_int = 9;

#[cfg(not(test))]
unsafe extern "C" {
    fn uv_kill(pid: c_int, signum: c_int) -> c_int;
}

#[cfg(test)]
unsafe fn uv_kill(pid: c_int, _signum: c_int) -> c_int {
    if pid <= 0 { UV_ESRCH } else { 0 }
}

#[cfg(target_os = "windows")]
const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
#[cfg(target_os = "windows")]
const PROCESS_ALL_ACCESS: u32 = 0x001F_FFFF;
#[cfg(target_os = "windows")]
const INVALID_HANDLE_VALUE: *mut c_void = (-1isize) as *mut c_void;

#[cfg(target_os = "windows")]
#[repr(C)]
struct ProcessEntry32 {
    dw_size: u32,
    cnt_usage: u32,
    th32_process_id: u32,
    th32_default_heap_id: usize,
    th32_module_id: u32,
    cnt_threads: u32,
    th32_parent_process_id: u32,
    pc_pri_class_base: i32,
    dw_flags: u32,
    sz_exe_file: [c_char; 260],
}

#[cfg(target_os = "windows")]
impl ProcessEntry32 {
    fn zeroed() -> Self {
        Self {
            dw_size: std::mem::size_of::<Self>() as u32,
            cnt_usage: 0,
            th32_process_id: 0,
            th32_default_heap_id: 0,
            th32_module_id: 0,
            cnt_threads: 0,
            th32_parent_process_id: 0,
            pc_pri_class_base: 0,
            dw_flags: 0,
            sz_exe_file: [0; 260],
        }
    }
}

#[cfg(all(target_os = "windows", not(test)))]
unsafe extern "system" {
    fn GetProcessId(process: *mut c_void) -> u32;
    fn CreateToolhelp32Snapshot(dw_flags: u32, th32_process_id: u32) -> *mut c_void;
    fn Process32First(h_snapshot: *mut c_void, lppe: *mut ProcessEntry32) -> i32;
    fn Process32Next(h_snapshot: *mut c_void, lppe: *mut ProcessEntry32) -> i32;
    fn OpenProcess(
        dw_desired_access: u32,
        b_inherit_handle: i32,
        dw_process_id: u32,
    ) -> *mut c_void;
    fn CloseHandle(h_object: *mut c_void) -> i32;
    fn TerminateProcess(h_process: *mut c_void, u_exit_code: u32) -> i32;
}

#[cfg(all(target_os = "windows", not(test)))]
unsafe extern "C" {
    fn arena_dict(arena: *mut Arena, max_size: usize) -> Dict;
    fn arena_string(arena: *mut Arena, str_val: NvimString) -> NvimString;
}

#[cfg(all(target_os = "windows", not(test)))]
unsafe fn os_proc_tree_kill_rec(proc_handle: *mut c_void, sig: c_int) -> bool {
    if proc_handle.is_null() {
        return false;
    }
    let pid = unsafe { GetProcessId(proc_handle) };
    if pid != 0 {
        let h = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if h != INVALID_HANDLE_VALUE {
            let mut pe = ProcessEntry32::zeroed();
            if unsafe { Process32First(h, &mut pe) != 0 } {
                loop {
                    if pe.th32_parent_process_id == pid {
                        let ph = unsafe { OpenProcess(PROCESS_ALL_ACCESS, 0, pe.th32_process_id) };
                        if !ph.is_null() {
                            unsafe {
                                os_proc_tree_kill_rec(ph, sig);
                                CloseHandle(ph);
                            }
                        }
                    }
                    if unsafe { Process32Next(h, &mut pe) == 0 } {
                        break;
                    }
                }
            }
            unsafe {
                CloseHandle(h);
            }
        }
    }
    unsafe { TerminateProcess(proc_handle, sig as u32) != 0 }
}

/// Kill process `pid` and its process group / descendants (`os_proc_tree_kill`).
#[unsafe(no_mangle)]
pub extern "C" fn os_proc_tree_kill(pid: c_int, sig: c_int) -> bool {
    assert!(sig == SIGTERM_NUM || sig == SIGKILL_NUM);
    #[cfg(target_os = "windows")]
    {
        if pid > 0 {
            #[cfg(not(test))]
            unsafe {
                let h = OpenProcess(PROCESS_ALL_ACCESS, 0, pid as u32);
                let ok = os_proc_tree_kill_rec(h, sig);
                if !h.is_null() {
                    CloseHandle(h);
                }
                return ok;
            }
        }
        false
    }

    #[cfg(not(target_os = "windows"))]
    {
        if pid == 0 {
            return false;
        }
        unsafe { uv_kill(-pid, sig) == 0 }
    }
}

/// Get the immediate child PIDs of `ppid` (`os_proc_children`).
///
/// # Safety
/// `proc_list` and `proc_count` must be valid writable pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_proc_children(
    ppid: c_int,
    proc_list: *mut *mut c_int,
    proc_count: *mut usize,
) -> c_int {
    if ppid < 0 {
        return 2;
    }

    let mut temp: *mut c_int = std::ptr::null_mut();
    unsafe {
        *proc_list = std::ptr::null_mut();
        *proc_count = 0;
    }

    #[cfg(all(target_os = "windows", not(test)))]
    unsafe {
        let h = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if h == INVALID_HANDLE_VALUE {
            return 2;
        }
        let mut pe = ProcessEntry32::zeroed();
        if Process32First(h, &mut pe) == 0 {
            CloseHandle(h);
            return 2;
        }
        loop {
            if pe.th32_parent_process_id == ppid as u32 {
                let count = *proc_count;
                temp = xrealloc(
                    temp.cast::<c_void>(),
                    (count + 1) * std::mem::size_of::<c_int>(),
                )
                .cast::<c_int>();
                *temp.add(count) = pe.th32_process_id as c_int;
                *proc_count = count + 1;
            }
            if Process32Next(h, &mut pe) == 0 {
                break;
            }
        }
        CloseHandle(h);
    }

    #[cfg(target_os = "linux")]
    {
        let path = format!("/proc/{ppid}/task/{ppid}/children");
        let Ok(contents) = std::fs::read_to_string(&path) else {
            return 2;
        };
        for tok in contents.split_ascii_whitespace() {
            if let Ok(match_pid) = tok.parse::<c_int>() {
                unsafe {
                    let count = *proc_count;
                    temp = xrealloc(
                        temp.cast::<c_void>(),
                        (count + 1) * std::mem::size_of::<c_int>(),
                    )
                    .cast::<c_int>();
                    *temp.add(count) = match_pid;
                    *proc_count = count + 1;
                }
            }
        }
    }

    unsafe {
        *proc_list = temp;
    }
    0
}

#[cfg(target_os = "windows")]
/// Return a dictionary of process properties (`pid`, `ppid`, `name`) on Windows (`os_proc_info`).
///
/// # Safety
/// `arena` must be a valid pointer to an initialized `Arena`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn os_proc_info(pid: c_int, arena: *mut Arena) -> Dict {
    let mut pinfo = Dict {
        size: 0,
        capacity: 0,
        items: std::ptr::null_mut(),
    };

    #[cfg(test)]
    {
        let _ = (pid, arena);
        pinfo
    }

    #[cfg(not(test))]
    unsafe {
        let h = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if h == INVALID_HANDLE_VALUE {
            return pinfo;
        }
        let mut pe = ProcessEntry32::zeroed();
        if Process32First(h, &mut pe) == 0 {
            CloseHandle(h);
            return pinfo;
        }
        loop {
            if pe.th32_process_id == pid as u32 {
                break;
            }
            if Process32Next(h, &mut pe) == 0 {
                break;
            }
        }
        CloseHandle(h);

        if pe.th32_process_id == pid as u32 {
            pinfo = arena_dict(arena, 3);
            let put = |dict: &mut Dict, key: &CStr, value: Object| {
                let idx = dict.size;
                *dict.items.add(idx) = KeyValuePair {
                    key: NvimString {
                        data: key.as_ptr().cast_mut(),
                        size: key.to_bytes().len(),
                    },
                    value,
                };
                dict.size += 1;
            };

            put(
                &mut pinfo,
                c"pid",
                Object {
                    obj_type: ObjectType::kObjectTypeInteger,
                    data: ObjectData {
                        integer: i64::from(pid),
                    },
                },
            );
            put(
                &mut pinfo,
                c"ppid",
                Object {
                    obj_type: ObjectType::kObjectTypeInteger,
                    data: ObjectData {
                        integer: i64::from(pe.th32_parent_process_id as c_int),
                    },
                },
            );
            let exe_ptr = pe.sz_exe_file.as_ptr();
            let exe_len = CStr::from_ptr(exe_ptr).to_bytes().len();
            let name_str = arena_string(
                arena,
                NvimString {
                    data: exe_ptr.cast_mut(),
                    size: exe_len,
                },
            );
            put(
                &mut pinfo,
                c"name",
                Object {
                    obj_type: ObjectType::kObjectTypeString,
                    data: ObjectData { string: name_str },
                },
            );
        }

        pinfo
    }
}

/// Return true if process `pid` is running (`os_proc_running`).
#[unsafe(no_mangle)]
pub extern "C" fn os_proc_running(pid: c_int) -> bool {
    let err = unsafe { uv_kill(pid, 0) };
    if err == 0 {
        return true;
    }
    if err == UV_ESRCH {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_ppid_returns_error_and_running_checks_pid() {
        let mut list: *mut c_int = std::ptr::null_mut();
        let mut count: usize = 99;
        unsafe {
            assert_eq!(os_proc_children(-1, &mut list, &mut count), 2);
        }
        assert!(os_proc_running(1));
        assert!(!os_proc_running(-1));
    }
}
