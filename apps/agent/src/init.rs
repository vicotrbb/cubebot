//! What `cubebot-agent guest` and `cubebot-agent container` share once the agent's environment
//! is known: its storage on the workspace volume, the desktop and its arbiter when computer use
//! is on (ADR 0085), the agent as a child, and reaping as PID 1 until the agent exits.
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

/// The workspace volume, laid out the same in both runtimes.
pub(crate) const STATE: &str = "/var/lib/cubebot";
const JOURNAL: &str = "/var/lib/cubebot/runtime";
const WORKSPACE: &str = "/var/lib/cubebot/workspace";
/// Files only the agent reads.
pub(crate) const PRIVATE: &str = "/run/cubebot";
/// The outer display's cookie (ADR 0085), for the arbiter and Xephyr.
const OUTER_AUTHORITY: &str = "/run/cubebot/display";
const PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// Prepares storage, starts the desktop and then the agent with `environment` and init's own
/// paths, which win over any duplicate; returns the agent's pid.
pub(crate) fn start(mut environment: Vec<(String, String)>, host: Host) -> io::Result<Pid> {
    let user = tool_user(&environment);
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

/// The configured tool user; the agent refuses a malformed or root one itself, and then runs no
/// tools.
fn tool_user(environment: &[(String, String)]) -> Option<(u32, u32)> {
    setting(environment, "CUBEBOT_TOOL_USER")
        .and_then(|user| user.split_once(':'))
        .and_then(|(uid, gid)| Some((uid.parse().ok()?, gid.parse().ok()?)))
        .filter(|&(uid, gid)| uid != 0 && gid != 0)
}

/// The agent's private journal, and a workspace the tool user can write.
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

/// The outer display is up and closed to every other user before this returns, so before the
/// agent runs any tool; the rest starts on its own thread. A desktop that fails to start leaves
/// the agent running without a screen.
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

/// Xvfb on :0 as root, reachable only through its path socket, which becomes root's alone once
/// it exists, and only with a cookie only root can read; returns the cookie.
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

/// An Xauthority file with one MIT-MAGIC-COOKIE-1 entry for `display` at any address.
fn xauthority(display: &str, cookie: &[u8]) -> Vec<u8> {
    let mut file = 0xffffu16.to_be_bytes().to_vec();
    for field in [&b""[..], display.as_bytes(), b"MIT-MAGIC-COOKIE-1", cookie] {
        file.extend_from_slice(&(field.len() as u16).to_be_bytes());
        file.extend_from_slice(field);
    }
    file
}

fn appears(path: &Path, server: &str) -> io::Result<()> {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !path.exists() {
        if std::time::Instant::now() > deadline {
            return Err(io::Error::other(format!(
                "{server} did not open its socket"
            )));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

/// Xephyr on :1 as root, a window filling :0 at (0, 0) and without XTEST, as the tool user's
/// display; then openbox and the wallpaper on it as the tool user, and the arbiter on :0. The
/// agent's browser module starts Chromium as a user of its own.
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
            .envs([
                ("HOME", WORKSPACE),
                ("PATH", PATH),
                ("DISPLAY", ":1"),
                ("LANG", "C.UTF-8"),
            ])
            .current_dir(WORKSPACE)
            .uid(uid)
            .gid(gid)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    };
    client("/usr/bin/openbox").spawn()?;
    // The default stays when the orchestrator is unreachable; the agent draws its own after it.
    // Once it is drawn Xephyr is serving, so its window is on :0 for the arbiter to focus.
    let drawn = client("/usr/bin/xwallpaper")
        .args(["--zoom", crate::tools::WALLPAPER])
        .status();
    match drawn {
        // PID 1's reaper may collect it first, which also means it has exited.
        Err(error) if error.raw_os_error() != Some(Errno::ECHILD as i32) => return Err(error),
        _ => (),
    }
    let input = crate::desktop::x11::connect(0, cookie)?;
    input.focus()?;
    crate::desktop::start(crate::desktop::x11::connect(0, cookie)?, input, host)?;
    File::create(crate::tools::DESKTOP_READY)?;
    eprintln!("Sandbox desktop started");
    Ok(())
}

/// As PID 1, waits for every child, orphaned tool processes included, until the agent exits, and
/// returns its exit status as a shell reports it.
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
