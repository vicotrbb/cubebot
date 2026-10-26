//! Disposable Linux-container qualification fixture, never shipped in runtime/tools images.
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    os::unix::fs::MetadataExt,
    time::Duration,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("serve") => return serve(),
        Some("filesystem") => filesystem()?,
        Some("connect") if args.len() == 2 => connect(&args[1])?,
        _ => return Err("invalid probe command".into()),
    };
    println!("{result}");
    Ok(())
}
fn identity() -> Result<Value, Box<dyn std::error::Error>> {
    let status = fs::read_to_string("/proc/self/status")?;
    let uid: u32 = status
        .lines()
        .find(|line| line.starts_with("Uid:"))
        .and_then(|line| line.split_whitespace().nth(2))
        .ok_or("missing process UID")?
        .parse()?;
    Ok(json!({"uid":uid,"pid_namespace":fs::metadata("/proc/self/ns/pid")?.ino()}))
}
fn serve() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::path::Path::new("/var/lib/cubebot");
    let mut journal = cubebot_agent::journal::Journal::open(&root.join("journal"))?;
    journal.append(b"fixture recovery evidence")?;
    fs::write(
        root.join("bootstrap-token"),
        b"fixture-only-not-a-credential",
    )?;
    let ipv4 = TcpListener::bind("0.0.0.0:18080")?;
    let ipv6 = TcpListener::bind("[::]:18081")?;
    std::thread::spawn(move || respond(ipv4));
    std::thread::spawn(move || respond(ipv6));
    println!("{}", identity()?);
    std::io::stdout().flush()?;
    loop {
        std::thread::park();
    }
}
fn respond(listener: TcpListener) {
    for mut stream in listener.incoming().flatten() {
        let _ = stream.write_all(b"fixture");
    }
}
fn filesystem() -> Result<Value, Box<dyn std::error::Error>> {
    let mut result = identity()?;
    result["reads_runtime_token"] = json!(fs::read("/storage/runtime/bootstrap-token").is_ok());
    result["reads_runtime_journal"] = json!(fs::read("/storage/runtime/journal/state").is_ok());
    result["writes_runtime_directory"] =
        json!(fs::write("/storage/runtime/escape", b"probe").is_ok());
    result["links_runtime_token"] = json!(
        fs::hard_link(
            "/storage/runtime/bootstrap-token",
            "/home/agent/linked-token"
        )
        .is_ok()
    );
    fs::write("/home/agent/probe-write", b"retained tool workspace")?;
    result["writes_tool_workspace"] = json!(true);
    Ok(result)
}
fn connect(address: &str) -> Result<Value, Box<dyn std::error::Error>> {
    let address: SocketAddr = address.parse()?;
    let connected = TcpStream::connect_timeout(&address, Duration::from_millis(700))
        .and_then(|mut stream| {
            stream.set_read_timeout(Some(Duration::from_millis(700)))?;
            let mut bytes = [0u8; 7];
            stream.read_exact(&mut bytes)?;
            if &bytes != b"fixture" {
                return Err(std::io::Error::other("unexpected fixture"));
            }
            Ok(())
        })
        .is_ok();
    Ok(json!({"connected":connected}))
}
