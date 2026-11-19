#![forbid(unsafe_code)]

fn main() -> std::process::ExitCode {
    std::process::ExitCode::from(nero::nvim_main() as u8)
}
