use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Output, Stdio};

/// A command that does not flash a console window when Gust runs as a GUI app.
pub fn command(program: &str) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// Whether `program` (without `.exe`) can be started by name.
pub fn on_path(program: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| {
        dir.join(program).is_file() || dir.join(format!("{program}.exe")).is_file()
    })
}

/// Run `program` in `dir`, feed it `input` on stdin, and collect its output.
pub fn run_with_stdin(
    program: &str,
    args: &[&str],
    dir: Option<&Path>,
    input: &str,
) -> std::io::Result<Output> {
    let mut command = command(program);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let mut child = command.spawn()?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let input = input.to_owned();
    // Written from another thread: a tool that fills its stdout pipe before
    // reading all of stdin would otherwise deadlock against us.
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(input.as_bytes());
    });
    let output = child.wait_with_output()?;
    let _ = writer.join();
    Ok(output)
}

/// Run `program` in `dir` and return its stdout when it exits successfully.
pub fn run(program: &str, args: &[&str], dir: Option<&Path>) -> Option<String> {
    let mut command = command(program);
    command.args(args).stdin(Stdio::null());
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let output = command.output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}
