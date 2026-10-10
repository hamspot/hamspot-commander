//! Stop the Gpredict this process started, and remove its hardware files.
//!
//! `on_exit` does not run after a crash. A child process waits on a pipe.
//! The kernel closes that pipe when this process is gone, and the child
//! finishes the work. The read returning is the signal. There is no timer.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::unix::io::FromRawFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::gpredict_hw;

static RECORD: Mutex<Option<PathBuf>> = Mutex::new(None);
static WATCHING: AtomicBool = AtomicBool::new(false);

/// Holds the pipe open. Dropping it lets the cleanup child run.
pub struct Guard {
    _write: File,
    _child: Child,
}

pub fn child_is_watching() -> bool {
    WATCHING.load(Ordering::SeqCst)
}

/// `$XDG_RUNTIME_DIR/hamspot-commander`, or `~/.cache/hamspot-commander`.
pub fn runtime_dir() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_RUNTIME_DIR") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("hamspot-commander"));
        }
    }
    let home = std::env::var_os("HOME")?;
    if home.is_empty() {
        return None;
    }
    Some(PathBuf::from(home).join(".cache").join("hamspot-commander"))
}

/// Start the cleanup child and write this process's instance record.
///
/// The returned guard owns the write end. A crash closes it without running
/// `Drop`. The child is its own process group, so the window's signals do
/// not kill it before it can read that close.
pub fn install() -> Option<Guard> {
    let runtime = runtime_dir()?;
    fs::create_dir_all(&runtime).ok()?;
    let pid = std::process::id();
    let record = runtime.join(pid.to_string());
    fs::write(&record, b"").ok()?;
    *RECORD.lock().expect("instance record") = Some(record);
    let hwconf = gpredict_hw::hwconf_dir().unwrap_or_default();
    let (read, write) = pipe()?;
    let exe = std::env::current_exe().ok()?;
    let mut child = Command::new(exe);
    child
        .arg("--gpredict-cleanup")
        .arg(&runtime)
        .arg(pid.to_string())
        .arg(&hwconf)
        .stdin(Stdio::from(read))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    match child.spawn() {
        Ok(child) => {
            WATCHING.store(true, Ordering::SeqCst);
            Some(Guard {
                _write: write,
                _child: child,
            })
        }
        Err(_) => None,
    }
}

/// Remember a Gpredict this process started. The cleanup child reads this
/// after the pipe closes. A Gpredict started some other way is not listed.
pub fn note_gpredict(pid: u32) {
    if pid <= 1 {
        return;
    }
    let Some(path) = RECORD.lock().expect("instance record").clone() else {
        return;
    };
    let mut file = match fs::OpenOptions::new().create(true).append(true).open(path) {
        Ok(file) => file,
        Err(_) => return,
    };
    let _ = writeln!(file, "gpredict {pid}");
}

/// True when the first argument is `--gpredict-cleanup`. The process then
/// waits for stdin to close and does not open the window.
pub fn run_cleanup_arg() -> bool {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("--gpredict-cleanup") {
        return false;
    }
    let Some(runtime) = args.next() else {
        return true;
    };
    let Some(pid) = args.next().and_then(|text| text.parse().ok()) else {
        return true;
    };
    let Some(hwconf) = args.next() else {
        return true;
    };
    serve(io::stdin(), Path::new(&runtime), pid, Path::new(&hwconf));
    true
}

/// Block until `input` closes, then stop our Gpredict and, if this was the
/// last Commander, delete the `hamspot_` hardware files.
pub fn serve<R: Read>(mut input: R, runtime: &Path, pid: u32, hwconf: &Path) {
    let mut buf = [0u8; 256];
    loop {
        match input.read(&mut buf) {
            Ok(0) => break,
            Ok(_) => continue,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    cleanup(runtime, pid, hwconf);
}

pub fn cleanup(runtime: &Path, self_pid: u32, hwconf: &Path) {
    let record = runtime.join(self_pid.to_string());
    for pid in gpredict_pids(&record) {
        stop_group(pid);
    }
    if !other_alive(runtime, self_pid) {
        let _ = remove_owned(hwconf);
    }
    let _ = fs::remove_file(record);
}

/// Used when the cleanup child could not be started. The window's `on_exit`
/// is the only remaining signal, and a crash still skips it.
pub fn cleanup_self() {
    let Some(runtime) = runtime_dir() else {
        return;
    };
    let hwconf = gpredict_hw::hwconf_dir().unwrap_or_default();
    cleanup(&runtime, std::process::id(), &hwconf);
}

fn gpredict_pids(record: &Path) -> Vec<i32> {
    let Ok(text) = fs::read_to_string(record) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            if parts.next() != Some("gpredict") {
                return None;
            }
            let pid = parts.next()?.parse().ok()?;
            if pid <= 1 {
                None
            } else {
                Some(pid)
            }
        })
        .collect()
}

fn other_alive(runtime: &Path, self_pid: u32) -> bool {
    let Ok(entries) = fs::read_dir(runtime) else {
        return false;
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        if pid == self_pid || pid == 0 {
            continue;
        }
        if Path::new(&format!("/proc/{pid}")).is_dir() {
            return true;
        }
    }
    false
}

fn owned_name(name: &str) -> bool {
    name.starts_with("hamspot_")
        && (name.ends_with(".rig") || name.ends_with(".rot"))
        && !name.contains("..")
        && !name.contains('/')
        && !name.contains('\\')
}

/// Delete `hamspot_*.rig` and `hamspot_*.rot`. A symlink is unlinked and not
/// followed. Any other name, including a hand-written radio file, stays.
pub fn remove_owned(dir: &Path) -> Result<(), String> {
    let dir = match dir.canonicalize() {
        Ok(dir) => dir,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(format!("cannot resolve {}: {err}", dir.display())),
    };
    let entries =
        fs::read_dir(&dir).map_err(|err| format!("cannot read {}: {err}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("cannot read {}: {err}", dir.display()))?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if !owned_name(&name) {
            continue;
        }
        let path = dir.join(&name);
        if path.parent() != Some(dir.as_path()) {
            continue;
        }
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() || meta.is_file() {
            fs::remove_file(&path)
                .map_err(|err| format!("cannot remove {}: {err}", path.display()))?;
        }
    }
    Ok(())
}

/// `SIGTERM` the process group, then wait until that pid is gone.
/// A pidfd becoming readable is the process exiting. If it is still there,
/// `SIGKILL` and wait once more. Pid 0 and pid 1 are refused.
fn stop_group(pid: i32) {
    if pid <= 1 {
        return;
    }
    unsafe {
        libc::kill(-pid, libc::SIGTERM);
    }
    reap(pid);
    if alive(pid) {
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
        reap(pid);
    }
}

fn alive(pid: i32) -> bool {
    Path::new(&format!("/proc/{pid}")).is_dir()
}

fn reap(pid: i32) {
    // pidfd_open is Linux. A missing pid is already gone; there is nothing to wait for.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if fd < 0 {
        return;
    }
    let mut pollfd = libc::pollfd {
        fd: fd as libc::c_int,
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        let ready = unsafe { libc::poll(&mut pollfd, 1, -1) };
        if ready < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
        }
        break;
    }
    unsafe {
        libc::close(fd as libc::c_int);
    }
}

fn pipe() -> Option<(File, File)> {
    let mut fds = [0; 2];
    // Both ends are cloexec so the child does not inherit the write end and
    // keep the pipe open after Commander is gone. Stdio moves the read end
    // onto stdin, which stays open in the child.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return None;
    }
    Some(unsafe { (File::from_raw_fd(fds[0]), File::from_raw_fd(fds[1])) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
    use std::thread;

    fn scratch() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, AtomicOrdering::Relaxed);
        let path = std::env::temp_dir().join(format!("hamspot-exit-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn seed(hw: &Path) {
        fs::write(hw.join("hamspot_Dummy.rig"), b"rig").unwrap();
        fs::write(hw.join("hamspot_Tower.rot"), b"rot").unwrap();
        fs::write(hw.join("MyRig.rig"), b"manual").unwrap();
    }

    #[test]
    fn the_last_instance_removes_owned_files_and_keeps_a_manual_file() {
        let hw = scratch();
        let runtime = scratch();
        seed(&hw);
        let pid = 2_147_000_001;
        assert!(!Path::new(&format!("/proc/{pid}")).exists());
        fs::write(runtime.join(pid.to_string()), b"").unwrap();
        cleanup(&runtime, pid, &hw);
        assert!(!hw.join("hamspot_Dummy.rig").exists());
        assert!(!hw.join("hamspot_Tower.rot").exists());
        assert_eq!(fs::read(hw.join("MyRig.rig")).unwrap(), b"manual");
        assert!(!runtime.join(pid.to_string()).exists());
        let _ = fs::remove_dir_all(&hw);
        let _ = fs::remove_dir_all(&runtime);
    }

    #[test]
    fn a_living_commander_keeps_the_hardware_files() {
        let hw = scratch();
        let runtime = scratch();
        seed(&hw);
        let dying = 2_147_000_002u32;
        assert!(!Path::new(&format!("/proc/{dying}")).exists());
        fs::write(runtime.join(dying.to_string()), b"").unwrap();
        fs::write(runtime.join(std::process::id().to_string()), b"other\n").unwrap();
        cleanup(&runtime, dying, &hw);
        assert!(hw.join("hamspot_Dummy.rig").exists());
        assert!(hw.join("hamspot_Tower.rot").exists());
        assert_eq!(fs::read(hw.join("MyRig.rig")).unwrap(), b"manual");
        assert!(!runtime.join(dying.to_string()).exists());
        assert!(runtime.join(std::process::id().to_string()).exists());
        let _ = fs::remove_dir_all(&hw);
        let _ = fs::remove_dir_all(&runtime);
    }

    #[test]
    fn a_dead_instance_record_does_not_block_removal() {
        let hw = scratch();
        let runtime = scratch();
        seed(&hw);
        let dying = 2_147_000_003u32;
        let gone = 2_147_000_004u32;
        assert!(!Path::new(&format!("/proc/{dying}")).exists());
        assert!(!Path::new(&format!("/proc/{gone}")).exists());
        fs::write(runtime.join(dying.to_string()), b"").unwrap();
        fs::write(runtime.join(gone.to_string()), b"").unwrap();
        cleanup(&runtime, dying, &hw);
        assert!(!hw.join("hamspot_Dummy.rig").exists());
        assert!(runtime.join(gone.to_string()).exists());
        let _ = fs::remove_dir_all(&hw);
        let _ = fs::remove_dir_all(&runtime);
    }

    #[test]
    fn a_symlink_is_removed_without_writing_through_it() {
        let hw = scratch();
        let runtime = scratch();
        let target = scratch();
        let file = target.join("kept.txt");
        fs::write(&file, b"keep").unwrap();
        symlink(&file, hw.join("hamspot_Link.rig")).unwrap();
        fs::write(hw.join("MyRig.rig"), b"manual").unwrap();
        let pid = 2_147_000_005;
        fs::write(runtime.join(pid.to_string()), b"").unwrap();
        cleanup(&runtime, pid, &hw);
        assert!(!hw.join("hamspot_Link.rig").exists());
        assert_eq!(fs::read(&file).unwrap(), b"keep");
        assert_eq!(fs::read(hw.join("MyRig.rig")).unwrap(), b"manual");
        let _ = fs::remove_dir_all(&hw);
        let _ = fs::remove_dir_all(&runtime);
        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn a_missing_gpredict_pid_is_not_an_error() {
        let hw = scratch();
        let runtime = scratch();
        seed(&hw);
        fs::write(hw.join("hamspot_a..b.rig"), b"odd").unwrap();
        let pid = 2_147_000_006;
        fs::write(runtime.join(pid.to_string()), b"not a pid line\n").unwrap();
        cleanup(&runtime, pid, &hw);
        assert!(!hw.join("hamspot_Dummy.rig").exists());
        assert!(hw.join("hamspot_a..b.rig").exists());
        assert_eq!(fs::read(hw.join("MyRig.rig")).unwrap(), b"manual");
        let _ = fs::remove_dir_all(&hw);
        let _ = fs::remove_dir_all(&runtime);
    }

    #[test]
    fn closing_the_pipe_runs_cleanup() {
        let hw = scratch();
        let runtime = scratch();
        seed(&hw);
        let pid = 2_147_000_007;
        fs::write(runtime.join(pid.to_string()), b"").unwrap();
        let (read, write) = pipe().expect("pipe");
        let hw_thread = hw.clone();
        let runtime_thread = runtime.clone();
        let worker = thread::spawn(move || serve(read, &runtime_thread, pid, &hw_thread));
        drop(write);
        worker.join().expect("cleanup thread");
        assert!(!hw.join("hamspot_Dummy.rig").exists());
        assert!(!hw.join("hamspot_Tower.rot").exists());
        assert_eq!(fs::read(hw.join("MyRig.rig")).unwrap(), b"manual");
        let _ = fs::remove_dir_all(&hw);
        let _ = fs::remove_dir_all(&runtime);
    }
}
