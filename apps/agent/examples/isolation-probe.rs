//! Disposable Linux-container qualification fixture, never shipped in runtime/tools images.
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    os::unix::fs::MetadataExt,
    time::Duration,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("serve") => return serve(),
        Some("filesystem") => filesystem()?,
        Some("session") if args.len() == 2 => return session(&args[1]),
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
    for address in ["0.0.0.0:18080", "[::]:18081", "0.0.0.0:18082", "[::]:18083"] {
        let listener = TcpListener::bind(address)?;
        std::thread::spawn(move || respond(listener));
    }
    println!("{}", identity()?);
    std::io::stdout().flush()?;
    loop {
        std::thread::park();
    }
}
fn respond(listener: TcpListener) {
    for stream in listener.incoming().flatten() {
        std::thread::spawn(move || {
            let _ = respond_session(stream);
        });
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
            read_fixture(&mut stream)
        })
        .is_ok();
    Ok(json!({"connected":connected}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistent_exchange_requires_a_fresh_server_reply() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut byte = [0];
            for _ in 0..2 {
                stream.read_exact(&mut byte).unwrap();
                stream.write_all(b"fixture").unwrap();
            }
        });
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(700)))
            .unwrap();
        assert!(exchange(&mut stream).is_ok());
        assert!(exchange(&mut stream).is_ok());
        server.join().unwrap();
        assert!(exchange(&mut stream).is_err());
    }
}

fn respond_session(mut stream: TcpStream) -> std::io::Result<()> {
    stream.write_all(b"fixture")?;
    let mut byte = [0];
    while stream.read_exact(&mut byte).is_ok() {
        stream.write_all(b"fixture")?;
    }
    Ok(())
}

fn exchange(stream: &mut TcpStream) -> std::io::Result<()> {
    stream.write_all(b"?")?;
    read_fixture(stream)
}

fn read_fixture(stream: &mut TcpStream) -> std::io::Result<()> {
    let mut bytes = [0u8; 7];
    stream.read_exact(&mut bytes)?;
    if &bytes != b"fixture" {
        return Err(std::io::Error::other("unexpected fixture"));
    }
    Ok(())
}

fn session(address: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut stream = TcpStream::connect_timeout(&address.parse()?, Duration::from_secs(2))?;
    stream.set_read_timeout(Some(Duration::from_millis(700)))?;
    stream.set_write_timeout(Some(Duration::from_millis(700)))?;
    read_fixture(&mut stream)?;
    println!("{}", json!({"connected":true}));
    std::io::stdout().flush()?;
    for line in std::io::stdin().lock().lines().take(4) {
        if line? != "exchange" {
            return Err("invalid session command".into());
        }
        println!("{}", json!({"connected":exchange(&mut stream).is_ok()}));
        std::io::stdout().flush()?;
    }
    Ok(())
}
