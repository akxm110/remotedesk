//! Wakes a powered-off peer through its ipTIME router: the router's web admin, reached from the
//! internet through its DDNS name and remote admin port, sends the magic packet on its LAN.

use hbb_common::{
    anyhow::{anyhow, bail},
    config::{LocalConfig, PeerConfig},
    log,
    password_security::{decrypt_str_or_original, encrypt_str_or_original},
    ResultType,
};

const HOST: &str = "router-wol-host";
const USER: &str = "router-wol-user";
const PASS: &str = "router-wol-pass";
const MACS: &str = "router-wol-macs";
// The router last saved for any peer, offered for the next one since a LAN usually has several PCs.
const LAST_HOST: &str = "router-wol-last-host";
const LAST_USER: &str = "router-wol-last-user";
const LAST_PASS: &str = "router-wol-last-pass";
const ENC_VERSION: &str = "00";

/// The router saved for `id` as JSON, falling back to the last router saved for any peer.
pub fn get(id: &str) -> String {
    let options = PeerConfig::load(id).options;
    let saved = options.get(HOST).map_or(false, |h| !h.is_empty());
    let (host, user, has_pass) = if saved {
        (
            options.get(HOST).cloned().unwrap_or_default(),
            options.get(USER).cloned().unwrap_or_default(),
            options.get(PASS).map_or(false, |p| !p.is_empty()),
        )
    } else {
        (
            LocalConfig::get_option(LAST_HOST),
            LocalConfig::get_option(LAST_USER),
            !LocalConfig::get_option(LAST_PASS).is_empty(),
        )
    };
    let macs = options
        .get(MACS)
        .filter(|m| !m.is_empty())
        .or(options.get("wol-macs"))
        .cloned()
        .unwrap_or_default();
    serde_json::json!({
        "saved": saved,
        "host": host,
        "user": user,
        "has_pass": has_pass,
        "macs": macs,
    })
    .to_string()
}

/// Saves the router for `id`, or forgets it when `host` is empty. An empty `pass` keeps the
/// password already saved for this peer, or for the same router on another peer.
pub fn set(id: &str, host: &str, user: &str, pass: &str, macs: &str) {
    let mut config = PeerConfig::load(id);
    if host.is_empty() {
        for k in [HOST, USER, PASS, MACS] {
            config.options.remove(k);
        }
        config.store(id);
        return;
    }
    let enc_pass = if !pass.is_empty() {
        encrypt_str_or_original(pass, ENC_VERSION, 128)
    } else if let Some(p) = config.options.get(PASS).filter(|p| !p.is_empty()) {
        p.clone()
    } else if LocalConfig::get_option(LAST_HOST) == host {
        LocalConfig::get_option(LAST_PASS)
    } else {
        String::new()
    };
    config.options.insert(HOST.to_owned(), host.to_owned());
    config.options.insert(USER.to_owned(), user.to_owned());
    config.options.insert(PASS.to_owned(), enc_pass.clone());
    config.options.insert(MACS.to_owned(), macs.to_owned());
    config.store(id);
    LocalConfig::set_option(LAST_HOST.to_owned(), host.to_owned());
    LocalConfig::set_option(LAST_USER.to_owned(), user.to_owned());
    LocalConfig::set_option(LAST_PASS.to_owned(), enc_pass);
}

/// Has the router saved for `id` wake it. Returns an empty string on success, otherwise a
/// message for the user.
pub fn wake(id: &str) -> String {
    match wake_(id) {
        Ok(()) => String::new(),
        Err(e) => {
            log::error!("router wake of {id} failed: {e}");
            e.to_string()
        }
    }
}

fn wake_(id: &str) -> ResultType<()> {
    let options = PeerConfig::load(id).options;
    let host = options.get(HOST).cloned().unwrap_or_default();
    if host.is_empty() {
        bail!("공유기가 설정되어 있지 않습니다.");
    }
    let user = options.get(USER).cloned().unwrap_or_default();
    let (pass, ..) = decrypt_str_or_original(
        &options.get(PASS).cloned().unwrap_or_default(),
        ENC_VERSION,
    );
    let macs: Vec<String> = options
        .get(MACS)
        .filter(|m| !m.is_empty())
        .or(options.get("wol-macs"))
        .map(|m| {
            m.split(|c: char| c == ',' || c.is_whitespace())
                .filter_map(normalize_mac)
                .collect()
        })
        .unwrap_or_default();
    if macs.is_empty() {
        bail!(
            "이 컴퓨터의 MAC 주소를 모릅니다. 컴퓨터가 켜져 있을 때 한 번 접속해 두거나, 공유기 설정에서 MAC 주소를 입력해 주세요."
        );
    }
    let base = if host.starts_with("http://") || host.starts_with("https://") {
        host.trim_end_matches('/').to_owned()
    } else {
        format!("http://{}", host.trim_end_matches('/'))
    };
    iptime::wake(&base, &user, &pass, &macs).map_err(|e| anyhow!("{e}"))
}

/// `AA:BB:CC:DD:EE:FF` from any of the usual separators, or None when it is not a MAC.
fn normalize_mac(s: &str) -> Option<String> {
    let hex: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if hex.len() != 12 || s.chars().any(|c| !c.is_ascii_hexdigit() && !":-. ".contains(c)) {
        return None;
    }
    let hex = hex.to_ascii_uppercase();
    Some(
        (0..6)
            .map(|i| &hex[i * 2..i * 2 + 2])
            .collect::<Vec<_>>()
            .join(":"),
    )
}

// ---- This PC: let controllers power it on through its own router ----

/// Server option (synced to the service, which sends it in the peer info) holding this PC's
/// router as JSON: host, user, encrypted pass, mac.
const THIS_PC: &str = "router-wol-self";
/// The remote admin port opened when the router has none.
const DEFAULT_PORT: u16 = 18888;

struct LocalNet {
    gateway: String,
    mac: String,
    wired: bool,
    adapter: String,
}

#[cfg(windows)]
fn local_net() -> ResultType<LocalNet> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // The adapter carrying the default route is the one the router sees.
    let script = "[Console]::OutputEncoding=[Text.Encoding]::UTF8; \
        $r = Get-NetRoute -AddressFamily IPv4 -DestinationPrefix '0.0.0.0/0' | Sort-Object RouteMetric | Select-Object -First 1; \
        $a = Get-NetAdapter -InterfaceIndex $r.ifIndex; \
        [pscustomobject]@{gateway=$r.NextHop; mac=$a.MacAddress; media=[string]$a.PhysicalMediaType; name=$a.InterfaceDescription} | ConvertTo-Json -Compress";
    let out = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|_| anyhow!("인터넷에 연결된 네트워크를 찾지 못했습니다."))?;
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_owned();
    let gateway = s("gateway");
    if gateway.is_empty() || gateway == "0.0.0.0" {
        bail!("인터넷에 연결된 네트워크를 찾지 못했습니다.");
    }
    Ok(LocalNet {
        gateway,
        mac: normalize_mac(&s("mac")).unwrap_or_default(),
        wired: s("media") == "802.3",
        adapter: s("name"),
    })
}

#[cfg(not(windows))]
fn local_net() -> ResultType<LocalNet> {
    bail!("Windows에서만 지원합니다.")
}

fn lan_client() -> ResultType<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .no_proxy()
        .build()?)
}

/// The router's LAN address: what the user typed, or else the default gateway. The router is
/// not the gateway behind a second router or when the ipTIME runs as an access point.
fn router_addr(net: &LocalNet, typed: &str) -> String {
    let typed = typed
        .trim()
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/');
    if typed.is_empty() {
        net.gateway.clone()
    } else {
        typed.to_owned()
    }
}

/// This PC's link and the router at `router` (the default gateway when empty) for the setup
/// dialog, as JSON.
pub fn this_pc_detect(router: &str) -> String {
    let detected = (|| -> ResultType<serde_json::Value> {
        let net = local_net()?;
        let addr = router_addr(&net, router);
        let router = iptime::probe(&lan_client()?, &format!("http://{addr}"));
        Ok(serde_json::json!({
            "gateway": addr,
            "mac": net.mac,
            "wired": net.wired,
            "adapter": net.adapter,
            "iptime": router.is_some(),
            "model": router.map(|r| r.0).unwrap_or_default(),
        }))
    })();
    match detected {
        Ok(v) => v.to_string(),
        Err(e) => serde_json::json!({ "error": e.to_string() }).to_string(),
    }
}

/// Turns on remote power-on for this PC: checks the router login, opens its remote admin port
/// when it is closed and keeps what controllers need. JSON with `error` on failure.
pub fn this_pc_setup(user: &str, pass: &str, router: &str) -> String {
    match this_pc_setup_(user, pass, router) {
        Ok(v) => v.to_string(),
        Err(e) => {
            log::error!("router self setup failed: {e}");
            serde_json::json!({ "error": e.to_string() }).to_string()
        }
    }
}

fn this_pc_setup_(user: &str, pass: &str, router: &str) -> ResultType<serde_json::Value> {
    let net = local_net()?;
    if !net.wired {
        bail!("이 컴퓨터는 무선(와이파이)으로 연결되어 있습니다. 꺼진 상태에서 켜려면 공유기에 랜선으로 연결한 뒤 다시 설정해 주세요.");
    }
    if net.mac.is_empty() {
        bail!("랜카드의 MAC 주소를 읽지 못했습니다.");
    }
    let client = lan_client()?;
    let addr = router_addr(&net, router);
    let base = format!("http://{addr}");
    let Some((_, new_ui)) = iptime::probe(&client, &base) else {
        bail!("공유기({addr})가 ipTIME이 아니거나 응답하지 않습니다.");
    };
    let remote = if new_ui {
        iptime::setup_new_ui(&client, &base, user, pass)?
    } else {
        iptime::setup_old_ui(&client, &base, user, pass)?
    };
    let address = match remote.ddns.clone().or(remote.extern_ip.clone()) {
        Some(address) => address,
        None => public_ip()?,
    };
    let host = format!("{address}:{}", remote.port);
    let saved = serde_json::json!({
        "host": host,
        "user": user,
        "pass": encrypt_str_or_original(pass, ENC_VERSION, 128),
        "mac": net.mac,
    });
    crate::ui_interface::set_option(THIS_PC.to_owned(), saved.to_string());
    Ok(serde_json::json!({
        "ok": true,
        "host": host,
        "opened": remote.opened,
        "ddns": remote.ddns.is_some(),
        "warnings": remote.warnings,
    }))
}

fn public_ip() -> ResultType<String> {
    let ip = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?
        .get("https://api.ipify.org")
        .send()?
        .text()?;
    let ip = ip.trim();
    ip.parse::<std::net::Ipv4Addr>()
        .map_err(|_| anyhow!("공유기의 외부 주소를 알아내지 못했습니다."))?;
    Ok(ip.to_owned())
}

/// Whether this PC can be powered on remotely, and through which router address, as JSON.
pub fn this_pc_status() -> String {
    let saved: serde_json::Value =
        serde_json::from_str(&crate::ui_interface::get_option(THIS_PC)).unwrap_or_default();
    let host = saved.get("host").and_then(|h| h.as_str()).unwrap_or_default();
    serde_json::json!({ "configured": !host.is_empty(), "host": host }).to_string()
}

pub fn this_pc_off() {
    crate::ui_interface::set_option(THIS_PC.to_owned(), String::new());
}

/// This PC's router for the peer info, with the password readable, so a controller that logs
/// in can power it on later without asking.
pub fn this_pc_additions() -> Option<serde_json::Value> {
    let saved: serde_json::Value =
        serde_json::from_str(&hbb_common::config::Config::get_option(THIS_PC)).ok()?;
    let s = |k: &str| saved.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_owned();
    if s("host").is_empty() {
        return None;
    }
    let (pass, ..) = decrypt_str_or_original(&s("pass"), ENC_VERSION);
    Some(serde_json::json!({ "host": s("host"), "user": s("user"), "pass": pass, "mac": s("mac") }))
}

/// Keeps the router a peer sent in its peer info, so its power button works without setup.
pub fn save_from_peer(id: &str, router: &serde_json::Value) {
    let s = |k: &str| router.get(k).and_then(|x| x.as_str()).unwrap_or_default();
    let host = s("host");
    if host.is_empty() {
        return;
    }
    let options = PeerConfig::load(id).options;
    let same = options.get(HOST).map(String::as_str) == Some(host)
        && options.get(USER).map(String::as_str) == Some(s("user"))
        && options.get(MACS).map(String::as_str) == Some(s("mac"))
        && options
            .get(PASS)
            .map_or(false, |p| decrypt_str_or_original(p, ENC_VERSION).0 == s("pass"));
    if same {
        return;
    }
    set(id, host, s("user"), s("pass"), s("mac"));
    crate::conn_log::add(id, format!("원격 켜기 정보 받음: 공유기 {host}"));
}

/// Wake-on-LAN settings for every wired adapter, plus fast startup off so a shut-down PC still
/// listens. Advanced properties take effect after a restart so a remote session is not cut.
#[cfg(windows)]
const WAKE_SETTINGS_PS: &str = r#"
Get-NetAdapter -Physical | Where-Object { $_.PhysicalMediaType -eq '802.3' } | ForEach-Object {
  $n = $_.Name
  Set-NetAdapterPowerManagement -Name $n -WakeOnMagicPacket Enabled -NoRestart -ErrorAction SilentlyContinue
  foreach ($k in '*WakeOnMagicPacket', 'S5WakeOnLan', 'EnablePME', 'WakeOnMagicPacketFromS5') {
    if (Get-NetAdapterAdvancedProperty -Name $n -RegistryKeyword $k -ErrorAction SilentlyContinue) {
      Set-NetAdapterAdvancedProperty -Name $n -RegistryKeyword $k -RegistryValue 1 -NoRestart -ErrorAction SilentlyContinue
    }
  }
  powercfg /deviceenablewake $_.InterfaceDescription
}
Set-ItemProperty -Path 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Power' -Name HiberbootEnabled -Value 0 -Type DWord
"#;

/// Applies [`WAKE_SETTINGS_PS`] through a UAC prompt; false when it could not be started.
#[cfg(windows)]
pub fn this_pc_prepare_windows() -> bool {
    use hbb_common::base64::{engine::general_purpose::STANDARD, Engine as _};
    let utf16: Vec<u8> = WAKE_SETTINGS_PS
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    let args = format!(
        "-NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -EncodedCommand {}",
        STANDARD.encode(utf16)
    );
    crate::platform::windows::run_uac("powershell.exe", &args).unwrap_or(false)
}

#[cfg(not(windows))]
pub fn this_pc_prepare_windows() -> bool {
    false
}

mod iptime {
    use super::*;
    use reqwest::{blocking::Client, header::HeaderMap};
    use serde_json::{json, Value};

    const CAPTCHA_HINT: &str = "공유기가 로그인할 때 보안문자(Captcha)를 요구합니다. 공유기 관리 화면 › 시스템 관리 › 관리자 설정에서 보안문자 사용을 '사용 안함' 또는 '제한 사용'으로 바꿔 주세요.";

    pub fn wake(base: &str, user: &str, pass: &str, macs: &[String]) -> ResultType<()> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            // Routers serve their remote admin page with a self-signed certificate.
            .danger_accept_invalid_certs(true)
            .build()?;
        let bootstrap = client
            .get(format!("{base}/ui/flutter_bootstrap.js"))
            .send()
            .map_err(|e| {
                log::error!("router {base} unreachable: {e}");
                anyhow!(
                    "공유기에 연결하지 못했습니다. 주소와 포트가 맞는지, 공유기에서 원격 관리 포트를 켰는지 확인해 주세요. (같은 공유기 안에서는 원격 관리 주소로 접속되지 않습니다)"
                )
            })?;
        let is_new_ui = bootstrap.status().is_success()
            && bootstrap.text().map_or(false, |t| t.contains("service.cgi"));
        // Firmware 15 and later stores MACs with dashes, like the older pages expect too.
        let macs: Vec<String> = macs.iter().map(|m| m.replace(':', "-")).collect();
        if is_new_ui {
            wake_new_ui(&client, base, user, pass, &macs)
        } else {
            wake_old_ui(&client, base, user, pass, &macs)
        }
    }

    /// Firmware 15 and later: JSON calls to /cgi/service.cgi.
    fn wake_new_ui(
        client: &Client,
        base: &str,
        user: &str,
        pass: &str,
        macs: &[String],
    ) -> ResultType<()> {
        let call = |cookie: Option<&str>, body: Value| -> ResultType<(HeaderMap, Value)> {
            let mut req = client
                .post(format!("{base}/cgi/service.cgi"))
                .header("Referer", format!("{base}/ui/"))
                .header("Origin", base)
                .header("Cache-Control", "no-store")
                .json(&body);
            if let Some(cookie) = cookie {
                req = req.header("Cookie", format!("efm_session_id={cookie}"));
            }
            let resp = req.send()?;
            let headers = resp.headers().clone();
            let value: Value = resp.json().unwrap_or(Value::Null);
            Ok((headers, value))
        };
        let (headers, login) = call(
            None,
            json!({"method": "session/login", "params": {"id": user, "pw": pass}}),
        )?;
        if let Some(err) = login.get("error") {
            let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("");
            let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
            log::error!("router login error {code}: {msg}");
            if msg.to_lowercase().contains("captcha") || code == -31997 {
                bail!("{CAPTCHA_HINT}");
            }
            bail!("공유기 관리자 아이디나 비밀번호가 틀렸습니다.");
        }
        let session = session_cookie(&headers)
            .ok_or_else(|| anyhow!("공유기 로그인에 실패했습니다. 아이디와 비밀번호를 확인해 주세요."))?;
        let registered: Vec<String> = call(Some(&session), json!({"method": "wol/show"}))
            .ok()
            .and_then(|(_, v)| v.get("result").and_then(|r| r.as_array()).cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|e| e.get("mac").and_then(|m| m.as_str()))
            .map(|m| m.replace(':', "-").to_uppercase())
            .collect();
        for mac in macs.iter().filter(|m| !registered.contains(*m)) {
            allow_err(call(
                Some(&session),
                json!({"method": "wol/add", "params": {"mac": mac, "pcname": "RemoteDesk"}}),
            ));
        }
        let (_, signal) = call(
            Some(&session),
            json!({"method": "wol/signal", "params": macs}),
        )?;
        allow_err(call(Some(&session), json!({"method": "session/logout"})));
        if let Some(err) = signal.get("error") {
            log::error!("router wol/signal error: {err}");
            bail!("공유기가 켜기 신호를 보내지 못했습니다.");
        }
        Ok(())
    }

    /// Firmware 14 and older: the sess-bin pages.
    fn wake_old_ui(
        client: &Client,
        base: &str,
        user: &str,
        pass: &str,
        macs: &[String],
    ) -> ResultType<()> {
        let body = client
            .post(format!("{base}/sess-bin/login_handler.cgi"))
            .header("Referer", format!("{base}/sess-bin/login_session.cgi"))
            .form(&[
                ("init_status", "1"),
                ("captcha_on", "0"),
                ("captcha_file", ""),
                ("username", user),
                ("passwd", pass),
                ("default_passwd", ""),
                ("captcha_code", ""),
            ])
            .send()?
            .text()?;
        let Some(session) = set_cookie_in_script(&body) else {
            if body.to_lowercase().contains("captcha") {
                bail!("{CAPTCHA_HINT}");
            }
            bail!("공유기 관리자 아이디나 비밀번호가 틀렸습니다.");
        };
        let cookie = format!("efm_session_id={session}");
        for mac in macs {
            client
                .post(format!("{base}/sess-bin/timepro.cgi"))
                .header("Referer", format!("{base}/sess-bin/timepro.cgi"))
                .header("Cookie", &cookie)
                .form(&[
                    ("tmenu", "iframe"),
                    ("smenu", "expertconfwollist"),
                    ("nomore", "0"),
                    ("wakeupchk", mac.as_str()),
                    ("act", "wake"),
                ])
                .send()?;
        }
        allow_err(
            client
                .get(format!("{base}/sess-bin/login_session.cgi?logout=1"))
                .header("Cookie", &cookie)
                .send(),
        );
        Ok(())
    }

    fn allow_err<T, E: std::fmt::Display>(r: Result<T, E>) {
        if let Err(e) = r {
            log::warn!("router request failed: {e}");
        }
    }

    /// The `efm_session_id` the new firmware sets on login.
    fn session_cookie(headers: &HeaderMap) -> Option<String> {
        headers
            .get_all("set-cookie")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find_map(|v| {
                let rest = &v[v.find("efm_session_id=")? + "efm_session_id=".len()..];
                let id: String = rest.chars().take_while(|c| *c != ';').collect();
                (!id.trim().is_empty()).then(|| id.trim().to_owned())
            })
    }

    /// What [`setup_old_ui`] / [`setup_new_ui`] learned from, or turned on in, the router.
    pub struct Remote {
        pub port: u16,
        pub opened: bool,
        pub ddns: Option<String>,
        pub extern_ip: Option<String>,
        pub warnings: Vec<String>,
    }

    const REMOTE_HINT: &str = "공유기의 원격 관리 포트를 자동으로 켜지 못했습니다. 공유기 관리 화면 › 고급 설정 › 보안 기능 › 공유기 접속 관리에서 '원격 관리 포트 사용'을 체크하고 포트를 정한 뒤 다시 시도해 주세요.";

    /// The router's model from its login page and whether it runs the firmware 15+ UI, or None
    /// when `base` is not an ipTIME router.
    pub fn probe(client: &Client, base: &str) -> Option<(String, bool)> {
        if let Ok(resp) = client.get(format!("{base}/ui/flutter_bootstrap.js")).send() {
            if resp.status().is_success() && resp.text().map_or(false, |t| t.contains("service.cgi"))
            {
                return Some(("ipTIME".to_owned(), true));
            }
        }
        let page = client
            .get(format!("{base}/sess-bin/login_session.cgi"))
            .send()
            .ok()?
            .text()
            .ok()?;
        // ASCII lowercasing keeps byte offsets, so they index `page` too.
        let lower = page.to_ascii_lowercase();
        let start = lower.find("<title>")? + "<title>".len();
        let end = start + lower[start..].find("</title>")?;
        let title = page[start..end].trim().to_owned();
        title
            .to_ascii_lowercase()
            .contains("iptime")
            .then_some((title, false))
    }

    /// Firmware 14 and older, reached from inside its LAN: turns on the remote admin port when
    /// it is off and reads the registered ipTIME DDNS name.
    pub fn setup_old_ui(client: &Client, base: &str, user: &str, pass: &str) -> ResultType<Remote> {
        let body = client
            .post(format!("{base}/sess-bin/login_handler.cgi"))
            .header("Referer", format!("{base}/sess-bin/login_session.cgi"))
            .form(&[
                ("init_status", "1"),
                ("captcha_on", "0"),
                ("captcha_file", ""),
                ("username", user),
                ("passwd", pass),
                ("default_passwd", ""),
                ("captcha_code", ""),
            ])
            .send()?
            .text()?;
        let Some(session) = set_cookie_in_script(&body) else {
            if body.to_lowercase().contains("captcha") {
                bail!("{CAPTCHA_HINT}");
            }
            bail!("공유기 관리자 아이디나 비밀번호가 틀렸습니다.");
        };
        let cookie = format!("efm_session_id={session}");
        let referer = format!("{base}/sess-bin/timepro.cgi");
        let page = |query: &str| -> ResultType<String> {
            Ok(client
                .get(format!("{base}/sess-bin/timepro.cgi?{query}"))
                .header("Referer", &referer)
                .header("Cookie", &cookie)
                .send()?
                .text()?)
        };
        let access = page("tmenu=firewallconf&smenu=accesslist")?;
        let mut warnings = Vec::new();
        // The list of outside addresses allowed to reach the admin page.
        let ext_on = input_checked(&access, "ext_chk");
        if ext_on {
            warnings.push("공유기에 '원격 접속 허용 IP' 제한이 켜져 있어, 목록에 없는 곳에서는 켤 수 없습니다.".to_owned());
        }
        let current = input_checked(&access, "rmgmt_chk")
            .then(|| input_value(&access, "rmgmt_port").parse::<u16>().ok())
            .flatten()
            .filter(|p| *p > 0);
        let (port, opened) = match current {
            Some(port) => (port, false),
            None => {
                let port = DEFAULT_PORT.to_string();
                // What the page's own "apply" sends through its hasetup frame.
                let mut form = vec![
                    ("tmenu", "iframe"),
                    ("smenu", "hasetup"),
                    ("act", if ext_on { "ext_run" } else { "ext_stop" }),
                    ("rmgmt_chk", "on"),
                    ("rmgmt_port", port.as_str()),
                ];
                if ext_on {
                    form.push(("ext_chk", "on"));
                }
                client
                    .post(format!("{base}/sess-bin/timepro.cgi"))
                    .header("Referer", &referer)
                    .header("Cookie", &cookie)
                    .form(&form)
                    .send()?;
                let access = page("tmenu=firewallconf&smenu=accesslist")?;
                if !input_checked(&access, "rmgmt_chk") {
                    bail!("{REMOTE_HINT}");
                }
                (
                    input_value(&access, "rmgmt_port")
                        .parse()
                        .unwrap_or(DEFAULT_PORT),
                    true,
                )
            }
        };
        let ddns = page("tmenu=expertconf&smenu=ddns")
            .ok()
            .map(|p| input_value(&p, "refhostname"))
            .filter(|h| h.ends_with(".iptime.org"));
        allow_err(
            client
                .get(format!("{base}/sess-bin/login_session.cgi?logout=1"))
                .header("Cookie", &cookie)
                .send(),
        );
        Ok(Remote {
            port,
            opened,
            ddns,
            extern_ip: None,
            warnings,
        })
    }

    /// Firmware 15 and later, reached from inside its LAN. Only reads: the shape of the remote
    /// admin settings is not known well enough to write it, so a closed port gets a hint instead.
    pub fn setup_new_ui(client: &Client, base: &str, user: &str, pass: &str) -> ResultType<Remote> {
        let call = |cookie: Option<&str>, body: Value| -> ResultType<(HeaderMap, Value)> {
            let mut req = client
                .post(format!("{base}/cgi/service.cgi"))
                .header("Referer", format!("{base}/ui/"))
                .header("Origin", base)
                .header("Cache-Control", "no-store")
                .json(&body);
            if let Some(cookie) = cookie {
                req = req.header("Cookie", format!("efm_session_id={cookie}"));
            }
            let resp = req.send()?;
            let headers = resp.headers().clone();
            let value: Value = resp.json().unwrap_or(Value::Null);
            Ok((headers, value))
        };
        let (headers, login) = call(
            None,
            json!({"method": "session/login", "params": {"id": user, "pw": pass}}),
        )?;
        if let Some(err) = login.get("error") {
            let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("");
            let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
            log::error!("router login error {code}: {msg}");
            if msg.to_lowercase().contains("captcha") || code == -31997 {
                bail!("{CAPTCHA_HINT}");
            }
            bail!("공유기 관리자 아이디나 비밀번호가 틀렸습니다.");
        }
        let session = session_cookie(&headers)
            .ok_or_else(|| anyhow!("공유기 로그인에 실패했습니다. 아이디와 비밀번호를 확인해 주세요."))?;
        let result = |body: Value| -> Value {
            call(Some(&session), body)
                .ok()
                .and_then(|(_, v)| v.get("result").cloned())
                .unwrap_or(Value::Null)
        };
        let http = result(json!({"method": "http/config"}));
        log::info!("router http/config: {http}");
        let (mut port, mut enabled) = (None, None);
        walk(&http, "", &mut |path, v| {
            let path = path.to_ascii_lowercase();
            if !path.contains("remote") {
                return;
            }
            if port.is_none() && path.contains("port") {
                port = v
                    .as_u64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                    .and_then(|n| u16::try_from(n).ok())
                    .filter(|n| *n > 0);
            }
            if enabled.is_none() {
                enabled = v.as_bool();
            }
        });
        let ddns = find_str(&result(json!({"method": "ddns/config", "params": "iptime"})), |s| {
            s.ends_with(".iptime.org")
        });
        let extern_ip = find_str(
            &result(json!({"method": "network/info", "params": ["extern_addr"]})),
            |s| s.parse::<std::net::Ipv4Addr>().is_ok(),
        );
        allow_err(call(Some(&session), json!({"method": "session/logout"})));
        match (port, enabled) {
            (Some(port), Some(true) | None) => Ok(Remote {
                port,
                opened: false,
                ddns,
                extern_ip,
                warnings: Vec::new(),
            }),
            _ => bail!("{REMOTE_HINT}"),
        }
    }

    fn walk(v: &Value, path: &str, f: &mut dyn FnMut(&str, &Value)) {
        match v {
            Value::Object(map) => {
                for (k, v) in map {
                    walk(v, &format!("{path}/{k}"), f);
                }
            }
            Value::Array(items) => {
                for v in items {
                    walk(v, path, f);
                }
            }
            _ => f(path, v),
        }
    }

    fn find_str(v: &Value, pred: impl Fn(&str) -> bool) -> Option<String> {
        let mut found = None;
        walk(v, "", &mut |_, v| {
            if let Some(s) = v.as_str().filter(|s| found.is_none() && pred(s)) {
                found = Some(s.to_owned());
            }
        });
        found
    }

    /// The `<input>` tag named `name` in an old-firmware page.
    fn input_tag<'a>(html: &'a str, name: &str) -> Option<&'a str> {
        [
            format!("name=\"{name}\""),
            format!("name='{name}'"),
            format!("name={name} "),
            format!("name={name}>"),
        ]
        .iter()
        .find_map(|pat| {
            let pos = html.find(pat.as_str())?;
            let start = html[..pos].rfind('<')?;
            let end = pos + html[pos..].find('>')?;
            Some(&html[start..=end])
        })
    }

    fn input_checked(html: &str, name: &str) -> bool {
        // A bare ` checked` attribute, not `this.checked` inside an onclick.
        input_tag(html, name).map_or(false, |t| {
            let t = t.to_ascii_lowercase();
            t.match_indices(" checked").any(|(i, _)| {
                !t[i + " checked".len()..].starts_with(|c: char| c.is_ascii_alphanumeric())
            })
        })
    }

    fn input_value(html: &str, name: &str) -> String {
        let Some(tag) = input_tag(html, name) else {
            return String::new();
        };
        let Some(pos) = tag.find("value=") else {
            return String::new();
        };
        let rest = &tag[pos + "value=".len()..];
        match rest.chars().next() {
            Some(q @ ('"' | '\'')) => rest[1..].split(q).next().unwrap_or_default().to_owned(),
            _ => rest
                .split(|c: char| c.is_whitespace() || c == '>')
                .next()
                .unwrap_or_default()
                .to_owned(),
        }
        .trim()
        .to_owned()
    }

    /// The session id from `setCookie('...')` in the old login page's script.
    fn set_cookie_in_script(body: &str) -> Option<String> {
        let rest = &body[body.find("setCookie(")? + "setCookie(".len()..];
        let rest = rest.trim_start_matches(|c| c == '\'' || c == '"');
        let id: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        (!id.is_empty()).then_some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_mac;

    #[test]
    fn mac_formats() {
        assert_eq!(
            normalize_mac("aa-bb-cc-dd-ee-ff").as_deref(),
            Some("AA:BB:CC:DD:EE:FF")
        );
        assert_eq!(
            normalize_mac("AA:BB:CC:DD:EE:FF").as_deref(),
            Some("AA:BB:CC:DD:EE:FF")
        );
        assert_eq!(normalize_mac("zz:bb:cc:dd:ee:ff"), None);
        assert_eq!(normalize_mac(""), None);
    }
}
