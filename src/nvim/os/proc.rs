//! OS process inspection and process-tree termination (`src/nvim/os/proc.c`).

#[cfg(windows)]
use crate::api::private::helpers::{arena_dict, arena_string};
#[cfg(windows)]
use crate::eval::funcs::PROCESS_ALL_ACCESS;
use crate::event::r#loop::uv_kill;
#[cfg(windows)]
use crate::memory::Arena;
#[cfg(windows)]
use std::ffi::{CStr, c_char};
use std::ffi::c_int;
#[cfg(target_os = "windows")]
use std::ffi::c_void;

#[cfg(target_os = "windows")]
use crate::api::private::validate::{Dict, NvimString};
#[cfg(windows)]
use crate::api::private::validate::{KeyValuePair, Object, ObjectData, ObjectType};

#[cfg(target_os = "windows")]
const UV_ESRCH: c_int = -4040;
#[cfg(not(target_os = "windows"))]
const UV_ESRCH: c_int = -3;

const SIGTERM_NUM: c_int = 15;
const SIGKILL_NUM: c_int = 9;

#[cfg(target_os = "windows")]
const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
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
    const fn new() -> Self {
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

#[cfg(target_os = "windows")]
impl Default for ProcessEntry32 {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_os = "windows")]
use crate::os::os_win_console::{CloseHandle, GetProcessId, OpenProcess, TerminateProcess};

#[cfg(target_os = "windows")]
unsafe extern "system" {
    fn CreateToolhelp32Snapshot(dw_flags: u32, th32_process_id: u32) -> *mut c_void;
    fn Process32First(h_snapshot: *mut c_void, lppe: *mut ProcessEntry32) -> i32;
    fn Process32Next(h_snapshot: *mut c_void, lppe: *mut ProcessEntry32) -> i32;
}

#[cfg(target_os = "windows")]
unsafe fn os_proc_tree_kill_rec(proc_handle: *mut c_void, sig: c_int) -> bool {
    if proc_handle.is_null() {
        return false;
    }
    let pid = unsafe { GetProcessId(proc_handle) };
    if pid != 0 {
        let h = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if h != INVALID_HANDLE_VALUE {
            let mut pe = ProcessEntry32::new();
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
pub fn os_proc_tree_kill(pid: c_int, sig: c_int) -> bool {
    assert!(sig == SIGTERM_NUM || sig == SIGKILL_NUM);
    #[cfg(target_os = "windows")]
    {
        if pid > 0 {
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
pub fn os_proc_children(ppid: c_int) -> Result<Vec<c_int>, c_int> {
    if ppid < 0 {
        return Err(2);
    }

    #[allow(unused_mut)]
    let mut temp: Vec<c_int> = Vec::new();

    #[cfg(target_os = "windows")]
    unsafe {
        let h = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if h == INVALID_HANDLE_VALUE {
            return Err(2);
        }
        let mut pe = ProcessEntry32::new();
        if Process32First(h, &mut pe) == 0 {
            CloseHandle(h);
            return Err(2);
        }
        loop {
            if pe.th32_parent_process_id == ppid as u32 {
                temp.push(pe.th32_process_id as c_int);
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
            return Err(2);
        };
        for tok in contents.split_ascii_whitespace() {
            if let Ok(match_pid) = tok.parse::<c_int>() {
                temp.push(match_pid);
            }
        }
    }

    Ok(temp)
}

#[cfg(target_os = "windows")]
/// Return a dictionary of process properties (`pid`, `ppid`, `name`) on Windows (`os_proc_info`).
///
/// # Safety
/// `arena` must be a valid pointer to an initialized `Arena`.
pub unsafe fn os_proc_info(pid: c_int, arena: *mut Arena) -> Dict {
    let mut pinfo = Dict {
        size: 0,
        capacity: 0,
        items: std::ptr::null_mut(),
    };

    unsafe {
        let h = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if h == INVALID_HANDLE_VALUE {
            return pinfo;
        }
        let mut pe = ProcessEntry32::new();
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
pub fn os_proc_running(pid: c_int) -> bool {
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
        assert_eq!(os_proc_children(-1), Err(2));
        assert!(os_proc_running(std::process::id() as c_int));
        assert!(!os_proc_running(c_int::MAX));
    }
}
