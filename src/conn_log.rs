//! Per-PC connection history, kept on disk so a failed connection can be explained later
//! (the peer card's "접속 로그" menu shows it).
use hbb_common::{chrono, config::Config, log};
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
};

/// A log past this size is cut back to its newest `KEEP_LEN` bytes.
const MAX_LEN: u64 = 512 * 1024;
const KEEP_LEN: u64 = 384 * 1024;

fn path(id: &str) -> Option<PathBuf> {
    let name: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if name.trim_matches('_').is_empty() {
        return None;
    }
    Some(Config::path("conn_logs").join(format!("{name}.log")))
}

pub fn add(id: &str, text: impl AsRef<str>) {
    let Some(path) = path(id) else {
        return;
    };
    let mut text = text.as_ref().trim().to_owned();
    if let Some(why) = explain(&text) {
        text = format!("{text}\n    → {why}");
    }
    let line = format!(
        "[{}] {}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        text
    );
    if let Err(e) = append(&path, &line) {
        log::warn!("connection log {:?}: {}", path, e);
    }
}

fn append(path: &PathBuf, line: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(line.as_bytes())?;
    if file.metadata()?.len() > MAX_LEN {
        drop(file);
        let mut file = fs::File::open(path)?;
        file.seek(SeekFrom::End(-(KEEP_LEN as i64)))?;
        let mut tail = Vec::new();
        file.read_to_end(&mut tail)?;
        // Start on a whole line.
        let start = tail.iter().position(|b| *b == b'\n').map_or(0, |i| i + 1);
        fs::write(path, &tail[start..])?;
    }
    Ok(())
}

pub fn get(id: &str) -> String {
    path(id)
        .and_then(|p| fs::read(p).ok())
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default()
}

pub fn clear(id: &str) {
    if let Some(p) = path(id) {
        fs::remove_file(p).ok();
    }
}

/// Plain-language reason for the errors people actually run into.
fn explain(text: &str) -> Option<&'static str> {
    let t = text.to_lowercase();
    let why = if t.contains("offline") {
        "상대 PC가 꺼져 있거나, RemoteDesk가 실행 중이 아니거나, 인터넷이 끊겨 시그널 서버에 등록되어 있지 않습니다."
    } else if t.contains("id does not exist") {
        "그 ID로 등록된 PC가 없습니다. ID를 다시 확인하세요 (상대 PC에서 ID가 바뀌었을 수 있음)."
    } else if t.contains("failed to connect via rendezvous server") {
        "시그널 서버가 응답하지 않았습니다. 이 PC의 인터넷 또는 서버 상태 문제입니다."
    } else if t.contains("failed to make direct connection") {
        "양쪽 공유기(NAT) 사이에서 직접 연결 길을 뚫지 못했습니다. 중계 서버 없이 쓰므로 이 경우 연결이 안 됩니다. 한쪽 공유기에서 UPnP를 켜거나 포트포워딩을 하면 해결됩니다."
    } else if t.contains("failed to connect via relay server") {
        "중계 서버에 연결하지 못했습니다."
    } else if t.contains("reset by the peer") || t.contains("connection reset") {
        "상대 PC가 연결을 끊었습니다 (접속 거부, 버전 차이, 상대 프로그램 재시작 등)."
    } else if t.contains("timeout") || t.contains("timed out") {
        "응답 시간이 초과됐습니다. 네트워크가 불안정하거나 상대 PC가 응답하지 않습니다."
    } else if t.contains("wrong password") {
        "비밀번호가 틀렸습니다."
    } else if t.contains("key mismatch") {
        "서버 키가 다릅니다. 상대 PC가 다른 서버 설정을 쓰고 있습니다."
    } else if t.contains("not allowed") || t.contains("denied") || t.contains("refused") {
        "상대 PC에서 접속을 허용하지 않았습니다."
    } else if t.contains("please try later") {
        "잠시 후 다시 시도하세요."
    } else {
        return None;
    };
    Some(why)
}
