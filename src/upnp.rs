//! Direct paths through a router's UPnP port, for when TCP punching fails and there is no relay
//! server. Inbound: the controlled machine opens a port on its router and names it to the
//! controller. Reverse: a controlled peer behind the carrier-grade NAT of USB tethering or a phone
//! hotspot cannot accept anything, so the controller opens a port on its own router and the peer
//! connects out to it. Either connection is then secured like any other, so the controller still
//! verifies the peer's signed id before using it.
use hbb_common::{
    allow_err, anyhow::anyhow, bail, log, tokio, tokio::net::TcpListener, ResultType, Stream,
};
use std::{
    io::{Read, Write},
    net::{IpAddr, SocketAddr, TcpStream, UdpSocket},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::Duration,
};

pub const REVERSE_PORT: u16 = 21120;
// Spread over a range by id so several machines behind one router each get a port.
const INBOUND_PORT_BASE: u16 = 21130;
const INBOUND_PORTS: u16 = 50;
// The rendezvous server passes the relay server field on untouched in both PunchHoleSent and
// RelayResponse, so the inbound address rides there behind this tag.
const INBOUND_TAG: &str = "|upnp://";
const INBOUND_CONNECT_MS: u64 = 5000;
const LEASE_SECS: u32 = 3600;
const RENEW_SECS: u64 = 1200;
const REVERSE_WAIT_MS: u64 = 8000;
const SERVICES: [&str; 3] = [
    "urn:schemas-upnp-org:service:WANIPConnection:1",
    "urn:schemas-upnp-org:service:WANIPConnection:2",
    "urn:schemas-upnp-org:service:WANPPPConnection:1",
];

static MAPPING_STARTED: AtomicBool = AtomicBool::new(false);
static INBOUND_STARTED: AtomicBool = AtomicBool::new(false);
static INBOUND_ADDR: Mutex<Option<SocketAddr>> = Mutex::new(None);

struct Igd {
    host: SocketAddr,
    control_path: String,
    service: &'static str,
}

fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let from = s.find(start)? + start.len();
    let len = s[from..].find(end)?;
    Some(&s[from..from + len])
}

fn split_url(url: &str) -> ResultType<(SocketAddr, String)> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| anyhow!("unsupported url: {}", url))?;
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], rest[i..].to_owned()),
        None => (rest, "/".to_owned()),
    };
    let host = if host.contains(':') {
        host.to_owned()
    } else {
        format!("{host}:80")
    };
    Ok((host.parse()?, path))
}

fn http(host: SocketAddr, request: &str) -> ResultType<String> {
    let mut stream = TcpStream::connect_timeout(&host, Duration::from_secs(3))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.write_all(request.as_bytes())?;
    let mut buf = Vec::new();
    // Bytes read before a timeout are kept in buf, which is all a closing server sends.
    if let Err(err) = stream.read_to_end(&mut buf) {
        if buf.is_empty() {
            return Err(err.into());
        }
    }
    let response = String::from_utf8_lossy(&buf).into_owned();
    let status = response.lines().next().unwrap_or_default();
    if !status.contains(" 200 ") {
        bail!(
            "{} {}",
            status,
            between(&response, "<errorDescription>", "</errorDescription>").unwrap_or_default()
        );
    }
    Ok(response)
}

fn read_description(location: &str) -> ResultType<Igd> {
    let (host, path) = split_url(location)?;
    let desc = http(
        host,
        &format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"),
    )?;
    for service in SERVICES {
        let Some(pos) = desc.find(&format!("<serviceType>{service}</serviceType>")) else {
            continue;
        };
        let Some(control) = between(&desc[pos..], "<controlURL>", "</controlURL>") else {
            continue;
        };
        let control = control.trim();
        let control_path = if control.starts_with("http://") {
            split_url(control)?.1
        } else if control.starts_with('/') {
            control.to_owned()
        } else {
            format!("/{control}")
        };
        return Ok(Igd {
            host,
            control_path,
            service,
        });
    }
    bail!("no WAN connection service at {}", location)
}

// Windows sends multicast out of the lowest-metric interface, often a Wi-Fi Direct or VPN
// adapter that has no router behind it, so the search is sent from the internet-facing address.
fn internet_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    Some(socket.local_addr().ok()?.ip())
}

fn discover() -> ResultType<Igd> {
    let socket = match internet_ip() {
        Some(ip) => UdpSocket::bind((ip, 0))?,
        None => UdpSocket::bind("0.0.0.0:0")?,
    };
    socket.set_read_timeout(Some(Duration::from_secs(3)))?;
    let request = "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\n\r\n";
    socket.send_to(request.as_bytes(), "239.255.255.250:1900")?;
    let mut buf = [0u8; 2048];
    for _ in 0..5 {
        let (n, _) = socket.recv_from(&mut buf)?;
        let response = String::from_utf8_lossy(&buf[..n]);
        let location = response.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim()
                .eq_ignore_ascii_case("location")
                .then(|| value.trim().to_owned())
        });
        if let Some(location) = location {
            match read_description(&location) {
                Ok(igd) => return Ok(igd),
                Err(err) => log::debug!("UPnP device at {} unusable: {}", location, err),
            }
        }
    }
    bail!("no UPnP internet gateway found")
}

fn soap(igd: &Igd, action: &str, args: &str) -> ResultType<String> {
    let service = igd.service;
    let body = format!(
        "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body><u:{action} xmlns:u=\"{service}\">{args}</u:{action}></s:Body></s:Envelope>"
    );
    let request = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: text/xml; charset=\"utf-8\"\r\nSOAPAction: \"{service}#{action}\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        igd.control_path,
        igd.host,
        body.len()
    );
    http(igd.host, &request)
}

fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || (o[0] == 100 && (o[1] & 0xc0) == 64))
        }
        IpAddr::V6(_) => true,
    }
}

/// Maps TCP `port` of the router to the same port on this machine and returns the public
/// address it is reachable at.
pub fn map_port(port: u16) -> ResultType<SocketAddr> {
    let igd = discover()?;
    let local_ip = {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.connect(igd.host)?;
        socket.local_addr()?.ip()
    };
    let add = |lease: u32| {
        soap(
            &igd,
            "AddPortMapping",
            &format!("<NewRemoteHost></NewRemoteHost><NewExternalPort>{port}</NewExternalPort><NewProtocol>TCP</NewProtocol><NewInternalPort>{port}</NewInternalPort><NewInternalClient>{local_ip}</NewInternalClient><NewEnabled>1</NewEnabled><NewPortMappingDescription>RemoteDesk</NewPortMappingDescription><NewLeaseDuration>{lease}</NewLeaseDuration>"),
        )
    };
    // Some older routers only accept permanent mappings.
    if let Err(err) = add(LEASE_SECS) {
        log::debug!("UPnP leased mapping refused ({}), trying a permanent one", err);
        add(0)?;
    }
    let response = soap(&igd, "GetExternalIPAddress", "")?;
    let ip: IpAddr = between(
        &response,
        "<NewExternalIPAddress>",
        "</NewExternalIPAddress>",
    )
    .ok_or_else(|| anyhow!("router did not report its external address"))?
    .trim()
    .parse()?;
    if !is_public(ip) {
        bail!("router is behind another NAT ({})", ip);
    }
    Ok(SocketAddr::new(ip, port))
}

pub fn unmap_port(port: u16) -> ResultType<()> {
    let igd = discover()?;
    soap(
        &igd,
        "DeletePortMapping",
        &format!("<NewRemoteHost></NewRemoteHost><NewExternalPort>{port}</NewExternalPort><NewProtocol>TCP</NewProtocol>"),
    )?;
    Ok(())
}

fn start_mapping() {
    if MAPPING_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| loop {
        match map_port(REVERSE_PORT) {
            Ok(addr) => log::info!("UPnP: reverse connections can reach {}", addr),
            Err(err) => log::info!("UPnP: no reverse connections: {}", err),
        }
        std::thread::sleep(Duration::from_secs(RENEW_SECS));
    });
}

/// Listens for the reverse connection of the peer about to be asked to punch. None when another
/// session of this controller is already waiting on the port.
pub async fn reverse_listener() -> Option<TcpListener> {
    start_mapping();
    // Not listen_any: its fallback shares the port, and two sessions must not both wait on it.
    match TcpListener::bind(("0.0.0.0", REVERSE_PORT)).await {
        Ok(listener) => Some(listener),
        Err(err) => {
            log::debug!("UPnP: reverse port busy: {}", err);
            None
        }
    }
}

pub async fn accept_reverse(listener: TcpListener) -> ResultType<Stream> {
    let (stream, addr) = hbb_common::timeout(REVERSE_WAIT_MS, listener.accept()).await??;
    log::info!("Reverse connection from {}", addr);
    stream.set_nodelay(true).ok();
    let local_addr = stream.local_addr()?;
    Ok(Stream::from(stream, local_addr))
}

/// Connects out to a controller that may be waiting on its router's reverse port, for when this
/// side's network cannot accept the controller's connection.
#[cfg(not(target_os = "ios"))]
pub fn spawn_reverse_connect(
    controller: SocketAddr,
    server: crate::server::ServerPtr,
    meta: crate::server::ConnectionMeta,
) {
    let ip = match controller.ip() {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(IpAddr::V6(v6), IpAddr::V4),
        ip => ip,
    };
    let addr = SocketAddr::new(ip, REVERSE_PORT);
    if !is_public(addr.ip()) {
        return;
    }
    tokio::spawn(async move {
        for _ in 0..3 {
            match hbb_common::timeout(2000, tokio::net::TcpStream::connect(addr)).await {
                Ok(Ok(stream)) => {
                    log::info!("Reverse connection to {} established", addr);
                    stream.set_nodelay(true).ok();
                    let local_addr = stream.local_addr().unwrap_or(addr);
                    allow_err!(
                        crate::server::create_tcp_connection(
                            server,
                            Stream::from(stream, local_addr),
                            addr,
                            true,
                            meta,
                        )
                        .await
                    );
                    return;
                }
                _ => hbb_common::sleep(1.).await,
            }
        }
    });
}

pub fn attach_inbound(relay_server: String) -> String {
    match *INBOUND_ADDR.lock().unwrap() {
        Some(addr) => format!("{relay_server}{INBOUND_TAG}{addr}"),
        None => relay_server,
    }
}

/// Splits what `attach_inbound` added off the relay server. The address comes from the peer, so
/// only a public one in the inbound range is taken.
pub fn split_inbound(relay_server: &str) -> (String, Option<SocketAddr>) {
    let Some((relay, addr)) = relay_server.split_once(INBOUND_TAG) else {
        return (relay_server.to_owned(), None);
    };
    let addr = addr.parse::<SocketAddr>().ok().filter(|a| {
        is_public(a.ip())
            && (INBOUND_PORT_BASE..INBOUND_PORT_BASE + INBOUND_PORTS).contains(&a.port())
    });
    (relay.to_owned(), addr)
}

pub async fn connect_inbound(addr: SocketAddr) -> ResultType<Stream> {
    let stream =
        hbb_common::timeout(INBOUND_CONNECT_MS, tokio::net::TcpStream::connect(addr)).await??;
    log::info!("Connected through the peer's router port {}", addr);
    stream.set_nodelay(true).ok();
    let local_addr = stream.local_addr()?;
    Ok(Stream::from(stream, local_addr))
}

/// Opens this machine's inbound port on the router and serves connections arriving on it.
#[cfg(not(target_os = "ios"))]
pub fn start_inbound(server: crate::server::ServerPtr) {
    use hbb_common::config::{self, Config};
    if INBOUND_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let offset = Config::get_id()
        .bytes()
        .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32))
        % INBOUND_PORTS as u32;
    let port = INBOUND_PORT_BASE + offset as u16;
    tokio::spawn(async move {
        let listener = match TcpListener::bind(("0.0.0.0", port)).await {
            Ok(listener) => listener,
            Err(err) => {
                log::info!("UPnP: no inbound port {}: {}", port, err);
                return;
            }
        };
        std::thread::spawn(move || loop {
            let addr = match map_port(port) {
                Ok(addr) => {
                    log::info!("UPnP: inbound connections can reach {}", addr);
                    Some(addr)
                }
                Err(err) => {
                    log::info!("UPnP: no inbound port: {}", err);
                    None
                }
            };
            *INBOUND_ADDR.lock().unwrap() = addr;
            std::thread::sleep(Duration::from_secs(RENEW_SECS));
        });
        loop {
            let (stream, addr) = match listener.accept().await {
                Ok(accepted) => accepted,
                Err(err) => {
                    log::warn!("UPnP: inbound accept failed: {}", err);
                    hbb_common::sleep(1.).await;
                    continue;
                }
            };
            if config::option2bool("stop-service", &Config::get_option("stop-service")) {
                continue;
            }
            log::info!("Inbound connection from {}", addr);
            stream.set_nodelay(true).ok();
            let local_addr = stream.local_addr().unwrap_or(addr);
            let server = server.clone();
            tokio::spawn(async move {
                allow_err!(
                    crate::server::create_tcp_connection(
                        server,
                        Stream::from(stream, local_addr),
                        addr,
                        true,
                        Default::default(),
                    )
                    .await
                );
            });
        }
    });
}

/// `--upnp-test <log file>`: opens the reverse port, accepts connections for 90 seconds so it
/// can be probed from outside, then closes the port again.
pub fn self_test(log_path: &str) {
    let mut out = String::new();
    match map_port(REVERSE_PORT) {
        Ok(addr) => {
            out.push_str(&format!("mapped {}\n", addr));
            match std::net::TcpListener::bind(("0.0.0.0", REVERSE_PORT)) {
                Ok(listener) => {
                    listener.set_nonblocking(true).ok();
                    let end = std::time::Instant::now() + Duration::from_secs(90);
                    while std::time::Instant::now() < end {
                        if let Ok((_, from)) = listener.accept() {
                            out.push_str(&format!("accepted from {}\n", from));
                            std::fs::write(log_path, &out).ok();
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
                Err(err) => out.push_str(&format!("listen failed: {}\n", err)),
            }
            match unmap_port(REVERSE_PORT) {
                Ok(()) => out.push_str("unmapped\n"),
                Err(err) => out.push_str(&format!("unmap failed: {}\n", err)),
            }
        }
        Err(err) => out.push_str(&format!("map failed: {}\n", err)),
    }
    std::fs::write(log_path, out).ok();
}
