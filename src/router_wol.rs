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
            "이 컴퓨터의 MAC 주소를 모릅니다. 컴퓨터가 켜져 있을 때 한 번 접속해 두거나, \
             공유기 설정에서 MAC 주소를 입력해 주세요."
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

mod iptime {
    use super::*;
    use reqwest::{blocking::Client, header::HeaderMap};
    use serde_json::{json, Value};

    const CAPTCHA_HINT: &str = "공유기가 로그인할 때 보안문자(Captcha)를 요구합니다. 공유기 관리 화면 \
        › 시스템 관리 › 관리자 설정에서 보안문자 사용을 '사용 안함' 또는 '제한 사용'으로 바꿔 주세요.";

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
                    "공유기에 연결하지 못했습니다. 주소와 포트가 맞는지, 공유기에서 원격 관리 포트를 \
                     켰는지 확인해 주세요. (같은 공유기 안에서는 원격 관리 주소로 접속되지 않습니다)"
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
