use crate::desktop::Host;
use nix::{
    errno::Errno,
    sys::wait::{WaitStatus, waitpid},
    unistd::Pid,
};
use std::{
    fs::{DirBuilder, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::{
        fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
        process::CommandExt,
    },
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

pub(crate) const STATE: &str = "/var/lib/cubebot";
const JOURNAL: &str = "/var/lib/cubebot/runtime";
const WORKSPACE: &str = "/var/lib/cubebot/workspace";
pub(crate) const PRIVATE: &str = "/run/cubebot";
const OUTER_AUTHORITY: &str = "/run/cubebot/display";
const PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";
/// A window mapped before openbox manages windows can be dropped, so the desktop is not ready until this exists.
const OPENBOX_STARTED: &str = "/tmp/.cubebot-openbox-started";

pub(crate) fn start(mut environment: Vec<(String, String)>, host: Host) -> io::Result<Pid> {
    // The agent refuses a malformed or root tool user itself, and then runs no tools.
    let user = setting(&environment, "CUBEBOT_TOOL_USER").and_then(crate::tools::tool_user);
    storage(user)?;
    environment.extend([
        ("PATH".to_owned(), PATH.to_owned()),
        ("CUBEBOT_JOURNAL_DIRECTORY".to_owned(), JOURNAL.to_owned()),
        (
            "CUBEBOT_WORKSPACE_DIRECTORY".to_owned(),
            WORKSPACE.to_owned(),
        ),
    ]);
    if setting(&environment, "CUBEBOT_COMPUTER_ENABLED") == Some("1")
        && let Some((uid, gid)) = user
    {
        desktop(uid, gid, host);
    }
    let agent = Command::new(std::env::current_exe()?)
        .env_clear()
        .envs(environment)
        .spawn()?;
    Ok(Pid::from_raw(agent.id() as i32))
}

fn setting<'a>(environment: &'a [(String, String)], wanted: &str) -> Option<&'a str> {
    environment
        .iter()
        .rev()
        .find(|(name, _)| name == wanted)
        .map(|(_, value)| value.as_str())
}

fn storage(tool_user: Option<(u32, u32)>) -> io::Result<()> {
    DirBuilder::new().recursive(true).create(JOURNAL)?;
    std::fs::set_permissions(JOURNAL, std::fs::Permissions::from_mode(0o700))?;
    DirBuilder::new()
        .recursive(true)
        .mode(0o755)
        .create(WORKSPACE)?;
    if let Some((uid, gid)) = tool_user {
        std::os::unix::fs::chown(WORKSPACE, Some(uid), Some(gid))?;
    }
    Ok(())
}

/// The outer display is closed to every other user before this returns, so before the agent runs any tool.
fn desktop(uid: u32, gid: u32, host: Host) {
    match outer_display() {
        Ok(cookie) => drop(std::thread::spawn(move || {
            if let Err(error) = start_desktop(uid, gid, &cookie, host) {
                eprintln!("Sandbox desktop failed: {error}");
            }
        })),
        Err(error) => eprintln!("Sandbox desktop failed: {error}"),
    }
}

/// Reachable only through its path socket, root's alone once it exists, and only with a cookie only root can read.
fn outer_display() -> io::Result<[u8; 16]> {
    let mut cookie = [0; 16];
    File::open("/dev/urandom")?.read_exact(&mut cookie)?;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(OUTER_AUTHORITY)?
        .write_all(&xauthority("0", &cookie))?;
    Command::new("/usr/bin/Xvfb")
        .args([
            ":0",
            "-screen",
            "0",
            "1280x800x24",
            "-auth",
            OUTER_AUTHORITY,
        ])
        .args(["-nolisten", "tcp", "-nolisten", "local", "-noreset"])
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()?;
    appears(Path::new("/tmp/.X11-unix/X0"), "Xvfb")?;
    std::fs::set_permissions("/tmp/.X11-unix/X0", std::fs::Permissions::from_mode(0o700))?;
    Ok(cookie)
}

fn xauthority(display: &str, cookie: &[u8]) -> Vec<u8> {
    let mut file = 0xffffu16.to_be_bytes().to_vec();
    for field in [&b""[..], display.as_bytes(), b"MIT-MAGIC-COOKIE-1", cookie] {
        file.extend_from_slice(&(field.len() as u16).to_be_bytes());
        file.extend_from_slice(field);
    }
    file
}

fn openbox(command: &mut Command) -> io::Result<()> {
    // A container's writable root keeps /tmp across restarts.
    match std::fs::remove_file(OPENBOX_STARTED) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => (),
    }
    command
        .args(["--startup", &format!("touch {OPENBOX_STARTED}")])
        .spawn()?;
    appears(Path::new(OPENBOX_STARTED), "openbox")
}

fn appears(path: &Path, server: &str) -> io::Result<()> {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !path.exists() {
        if std::time::Instant::now() > deadline {
            return Err(io::Error::other(format!("{server} did not start")));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

/// Xephyr on :1, without XTEST, is the tool user's display; the arbiter stays on :0.
fn start_desktop(uid: u32, gid: u32, cookie: &[u8], host: Host) -> io::Result<()> {
    Command::new("/usr/bin/Xephyr")
        .args([
            ":1",
            "-screen",
            "1280x800",
            "-extension",
            "XTEST",
            "-no-host-grab",
        ])
        .args(["-nolisten", "tcp", "-nolisten", "local", "-noreset"])
        .env_clear()
        .envs([("DISPLAY", ":0"), ("XAUTHORITY", OUTER_AUTHORITY)])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()?;
    appears(Path::new("/tmp/.X11-unix/X1"), "Xephyr")?;
    let client = |program: &str| {
        let mut command = Command::new(program);
        command
            .env_clear()
            .envs([("HOME", WORKSPACE), ("DISPLAY", ":1"), ("LANG", "C.UTF-8")])
            .env("PATH", crate::tools::user_path(Path::new(WORKSPACE)))
            .current_dir(WORKSPACE)
            .uid(uid)
            .gid(gid)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    };
    openbox(&mut client("/usr/bin/openbox"))?;
    // The default stays when the orchestrator is unreachable; once drawn, Xephyr is serving, so its window is on :0 for the arbiter.
    let drawn = client("/usr/bin/xwallpaper")
        .args(["--zoom", crate::tools::WALLPAPER])
        .status();
    match drawn {
        // PID 1's reaper may collect it first, which also means it has exited.
        Err(error) if error.raw_os_error() != Some(Errno::ECHILD as i32) => return Err(error),
        _ => (),
    }
    client("/usr/bin/tint2").spawn()?;
    let input = crate::desktop::x11::connect(0, cookie)?;
    input.focus()?;
    crate::desktop::start(crate::desktop::x11::connect(0, cookie)?, input, host)?;
    File::create(crate::tools::DESKTOP_READY)?;
    eprintln!("Sandbox desktop started");
    Ok(())
}

pub(crate) fn reap(agent: Pid) -> io::Result<u8> {
    loop {
        match waitpid(None, None) {
            Ok(WaitStatus::Exited(pid, code)) if pid == agent => {
                eprintln!("Agent exited with status {code}");
                return Ok(code as u8);
            }
            Ok(WaitStatus::Signaled(pid, signal, _)) if pid == agent => {
                eprintln!("Agent ended by {signal}");
                return Ok(128 + signal as u8);
            }
            Ok(_) | Err(Errno::EINTR) => (),
            Err(error) => return Err(error.into()),
        }
    }
}
