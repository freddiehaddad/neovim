//! System memory information (`src/nvim/os/mem.c`).

use crate::event::r#loop::uv_get_total_memory;

#[inline]
pub const fn bytes_to_kib(bytes: u64) -> u64 {
    bytes / 1024
}

/// Get the total system physical memory in KiB (`os_get_total_mem_kib`).
#[unsafe(no_mangle)]
pub extern "C" fn os_get_total_mem_kib() -> u64 {
    let total_bytes = unsafe { uv_get_total_memory() };

    bytes_to_kib(total_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_bytes_to_kib() {
        assert_eq!(bytes_to_kib(0), 0);
        assert_eq!(bytes_to_kib(1023), 0);
        assert_eq!(bytes_to_kib(1024), 1);
        assert_eq!(bytes_to_kib(1024 * 1024), 1024);
        assert!(os_get_total_mem_kib() > 0);
    }
}
