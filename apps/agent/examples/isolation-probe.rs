//! Disposable Linux-container qualification fixture, never shipped in runtime/tools images.
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream, UdpSocket},
    os::unix::fs::MetadataExt,
    sync::{Arc, Mutex},
    time::Duration,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("serve") => return serve(),
        Some("filesystem") => filesystem()?,
        Some("terminal" | "terminal-cancel" | "terminal-renew" | "terminal-half-close")
            if (2..=3).contains(&args.len()) =>
        {
            terminal_request(&args)?
        }
        Some("session") if args.len() == 2 => return session(&args[1]),
        Some("udp-session") if args.len() == 2 => return udp_session(&args[1]),
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
    let _journal = fixture_storage()?;
    serve_http()?;
    for address in ["0.0.0.0:18080", "[::]:18081", "0.0.0.0:18082", "[::]:18083"] {
        let listener = TcpListener::bind(address)?;
        std::thread::spawn(move || respond(listener));
    }
    serve_push("127.0.0.1:18088", "127.0.0.1:18089")?;
    serve_push("[::1]:18090", "[::1]:18091")?;
    for address in ["0.0.0.0:18080", "[::]:18081", "0.0.0.0:18082", "[::]:18083"] {
        let socket = UdpSocket::bind(address)?;
        std::thread::spawn(move || udp_respond(socket));
    }
    for address in ["127.0.0.1:53", "[::1]:53"] {
        let listener = TcpListener::bind(address)?;
        std::thread::spawn(move || respond(listener));
        let socket = UdpSocket::bind(address)?;
        std::thread::spawn(move || udp_respond(socket));
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
    if address.starts_with("http://") || address.starts_with("https://") {
        return http(address);
    }
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
    fn udp_exchange_rejects_a_stale_reply() {
        let server = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let client = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        client.connect(server.local_addr().unwrap()).unwrap();
        client
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let peer = client.local_addr().unwrap();
        let producer = std::thread::spawn(move || {
            let mut bytes = [0; 8];
            server.recv_from(&mut bytes).unwrap();
            server.send_to(&1u64.to_be_bytes(), peer).unwrap();
            server.recv_from(&mut bytes).unwrap();
            server.send_to(&1u64.to_be_bytes(), peer).unwrap();
        });
        assert!(udp_exchange(&client, 1).is_ok());
        assert!(udp_exchange(&client, 2).is_err());
        producer.join().unwrap();
    }

    #[test]
    fn receiving_a_push_does_not_send_another_request() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_millis(100)))
                .unwrap();
            stream.write_all(b"fixture").unwrap();
            let mut byte = [0];
            assert!(stream.read_exact(&mut byte).is_err());
        });
        let mut stream = TcpStream::connect(address).unwrap();
        assert!(session_action(&mut stream, "receive").is_ok());
        server.join().unwrap();
    }

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

fn udp_respond(socket: UdpSocket) {
    let mut bytes = [0; 8];
    while let Ok((length, peer)) = socket.recv_from(&mut bytes) {
        if length == bytes.len() {
            let _ = socket.send_to(&bytes, peer);
        }
    }
}

fn udp_exchange(socket: &UdpSocket, sequence: u64) -> std::io::Result<()> {
    let expected = sequence.to_be_bytes();
    socket.send(&expected)?;
    let mut bytes = [0; 9];
    let length = socket.recv(&mut bytes)?;
    if length != expected.len() || bytes[..length] != expected {
        return Err(std::io::Error::other("stale or invalid UDP response"));
    }
    Ok(())
}

fn udp_session(address: &str) -> Result<(), Box<dyn std::error::Error>> {
    let address: SocketAddr = address.parse()?;
    let local = if address.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let socket = UdpSocket::bind(local)?;
    socket.connect(address)?;
    socket.set_read_timeout(Some(Duration::from_millis(700)))?;
    socket.set_write_timeout(Some(Duration::from_millis(700)))?;
    // UDP connect alone sends nothing. Admission requires a complete exchange.
    println!("{}", json!({"connected":udp_exchange(&socket, 1).is_ok()}));
    std::io::stdout().flush()?;
    for (index, line) in std::io::stdin().lock().lines().take(4).enumerate() {
        if line? != "exchange" {
            return Err("invalid UDP session command".into());
        }
        let connected = udp_exchange(&socket, index as u64 + 2).is_ok();
        println!("{}", json!({"connected":connected}));
        std::io::stdout().flush()?;
    }
    Ok(())
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
        let connected = session_action(&mut stream, &line?).is_ok();
        println!("{}", json!({"connected":connected}));
        std::io::stdout().flush()?;
    }
    Ok(())
}

fn session_action(stream: &mut TcpStream, command: &str) -> std::io::Result<()> {
    match command {
        "exchange" => exchange(stream),
        "receive" => read_fixture(stream),
        _ => Err(std::io::Error::other("invalid session command")),
    }
}

fn serve_push(address: &str, control: &str) -> std::io::Result<()> {
    let listener = TcpListener::bind(address)?;
    let control = TcpListener::bind(control)?;
    let current: Arc<Mutex<Option<TcpStream>>> = Arc::new(Mutex::new(None));
    let producer = Arc::clone(&current);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let _ = stream.set_write_timeout(Some(Duration::from_millis(700)));
            let mut slot = producer.lock().unwrap();
            *slot = Some(stream);
            if let Some(target) = slot.as_mut() {
                let _ = target.write_all(b"fixture");
            }
        }
    });
    std::thread::spawn(move || {
        for mut stream in control.incoming().flatten() {
            let mut slot = current.lock().unwrap();
            if let Some(target) = slot.as_mut()
                && target.write_all(b"fixture").is_ok()
            {
                let _ = stream.write_all(b"fixture");
            }
        }
    });
    Ok(())
}

fn terminal(
    mode: &str,
    request: &str,
    lease_millis: u64,
) -> Result<Value, Box<dyn std::error::Error>> {
    let mut socket =
        std::os::unix::net::UnixStream::connect("/run/cubebot-executor/terminal.sock")?;
    socket.set_read_timeout(Some(Duration::from_secs(150)))?;
    let invocation: cubebot_protocol::terminal::Invocation = serde_json::from_str(request)?;
    invocation.validate().map_err(std::io::Error::other)?;
    let not_after_millis = cubebot_agent::execution::terminal::monotonic_millis()? + lease_millis;
    let request = serde_json::to_vec(&cubebot_protocol::terminal::Request::Execute {
        invocation,
        not_after_millis,
    })?;
    socket.write_all(&(request.len() as u32).to_be_bytes())?;
    socket.write_all(&request)?;
    terminal_exchange(socket, mode, lease_millis)
}
fn terminal_exchange(
    socket: std::os::unix::net::UnixStream,
    mode: &str,
    lease_millis: u64,
) -> Result<Value, Box<dyn std::error::Error>> {
    if mode == "terminal-cancel" {
        std::thread::sleep(Duration::from_millis(200));
        return Ok(json!({"cancel_requested":true}));
    }
    if mode == "terminal-half-close" {
        socket.shutdown(std::net::Shutdown::Write)?;
    }
    let continuing = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let renewal = if mode == "terminal-renew" {
        Some(renew_terminal(
            socket.try_clone()?,
            continuing.clone(),
            lease_millis,
        ))
    } else {
        None
    };
    let mut output = Vec::new();
    let read = socket.take(65536).read_to_end(&mut output);
    continuing.store(false, std::sync::atomic::Ordering::SeqCst);
    if let Some(renewal) = renewal {
        let _ = renewal.join();
    }
    read?;
    Ok(serde_json::from_slice(&output)?)
}

fn fixture_storage() -> Result<cubebot_agent::journal::Journal, Box<dyn std::error::Error>> {
    let root = std::path::Path::new("/var/lib/cubebot");
    let mut journal = cubebot_agent::journal::Journal::open(&root.join("journal"))?;
    journal.append(b"fixture recovery evidence")?;
    fs::write(
        root.join("bootstrap-token"),
        b"fixture-only-not-a-credential",
    )?;
    Ok(journal)
}

fn serve_http() -> std::io::Result<()> {
    for address in ["0.0.0.0:18084", "[::]:18085"] {
        let listener = TcpListener::bind(address)?;
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = http_reply(stream);
            }
        });
    }
    Ok(())
}
fn http_reply(mut stream: TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
    let mut request = String::new();
    std::io::BufReader::new(&mut stream)
        .take(1024)
        .read_line(&mut request)?;
    let target = if request.starts_with("GET /redirect-private-v6 ") {
        Some("http://[::1]:18085/")
    } else if request.starts_with("GET /redirect-private ") {
        Some("http://127.0.0.1:18084/")
    } else {
        None
    };
    match target {
        Some(target) => write!(
            stream,
            "HTTP/1.1 302 Found\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        ),
        None => stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nfixture"),
    }
}
fn http(address: &str) -> Result<Value, Box<dyn std::error::Error>> {
    let url = reqwest::Url::parse(address)?;
    let resolve = url
        .query_pairs()
        .find(|(key, _)| key == "resolve")
        .map(|(_, value)| value.parse::<std::net::IpAddr>())
        .transpose()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let connected = runtime
        .block_on(http_request(url, resolve))
        .unwrap_or(false);
    Ok(json!({"connected":connected}))
}
async fn http_request(
    url: reqwest::Url,
    resolve: Option<std::net::IpAddr>,
) -> Result<bool, reqwest::Error> {
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .redirect(reqwest::redirect::Policy::limited(5));
    if let Some(address) = resolve {
        builder = builder.resolve(
            "qualification.invalid",
            SocketAddr::new(address, url.port_or_known_default().unwrap_or(80)),
        );
    }
    Ok(builder
        .build()?
        .get(url)
        .send()
        .await?
        .status()
        .is_success())
}

fn renew_terminal(
    mut socket: std::os::unix::net::UnixStream,
    continuing: Arc<std::sync::atomic::AtomicBool>,
    lease_millis: u64,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while continuing.load(std::sync::atomic::Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(200));
            let Ok(now) = cubebot_agent::execution::terminal::monotonic_millis() else {
                return;
            };
            if socket
                .write_all(&(now + lease_millis).to_be_bytes())
                .is_err()
            {
                return;
            }
        }
    })
}

fn terminal_request(args: &[String]) -> Result<Value, Box<dyn std::error::Error>> {
    let lease = args
        .get(2)
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(60000);
    terminal(&args[0], &args[1], lease)
}
