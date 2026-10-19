//! Helper program to exit and keep stdout/stderr open (`test/functional/fixtures/streams-test.c`).

use std::process::{self, Command, Stdio};

fn main() {
    let mut cmd = Command::new("sleep");
    cmd.arg("10")
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                unsafe extern "C" {
                    fn setsid() -> std::ffi::c_int;
                }
                let _ = setsid();
                Ok(())
            });
        }
    }

    match cmd.spawn() {
        Ok(child) => {
            eprintln!("pid: {}", child.id());
        }
        Err(err) => {
            eprintln!("{err}");
            process::exit(1);
        }
    }
}
