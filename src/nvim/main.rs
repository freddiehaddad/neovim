fn main() -> std::process::ExitCode {
    std::process::ExitCode::from(neovim::nvim_main() as u8)
}
