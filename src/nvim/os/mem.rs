//! System memory information (`src/nvim/os/mem.c`).

#[cfg(not(test))]
unsafe extern "C" {
    fn uv_get_total_memory() -> u64;
}

#[inline]
pub const fn bytes_to_kib(bytes: u64) -> u64 {
    bytes / 1024
}

/// Get the total system physical memory in KiB (`os_get_total_mem_kib`).
#[unsafe(no_mangle)]
pub extern "C" fn os_get_total_mem_kib() -> u64 {
    #[cfg(not(test))]
    let total_bytes = unsafe { uv_get_total_memory() };
    #[cfg(test)]
    let total_bytes = 16 * 1024 * 1024 * 1024u64;

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
