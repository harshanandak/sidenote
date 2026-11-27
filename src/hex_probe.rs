use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

#[derive(Deserialize)]
struct Discovery {
    port: u16,
    token: String,
    #[serde(rename = "apiVersion")]
    api_version: Option<String>,
    pid: Option<u32>,
}

fn control(
    d: &Discovery,
    version: u64,
    command: &str,
    key: Option<&str>,
) -> Result<Value, &'static str> {
    let mut request = json!({"version": version, "command": command});
    if let Some(key) = key {
        request["key"] = json!(key);
    }
    let body = request.to_string();
    let addr: SocketAddr = ([127, 0, 0, 1], d.port).into();
    let mut socket = TcpStream::connect_timeout(&addr, Duration::from_secs(3))
        .map_err(|_| "connection failed")?;
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|_| "timeout setup failed")?;
    socket
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|_| "timeout setup failed")?;
    write!(socket, "POST /control HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", d.port, d.token, body.len(), body).map_err(|_| "request failed")?;
    let mut response = Vec::new();
    socket
        .take(4 * 1024 * 1024)
        .read_to_end(&mut response)
        .map_err(|_| "response failed")?;
    let split = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("invalid HTTP response")?;
    let header = std::str::from_utf8(&response[..split]).map_err(|_| "invalid HTTP response")?;
    if header.lines().next().unwrap_or("").contains(" 404 ") {
        return Err("control unavailable");
    }
    if !header.lines().next().unwrap_or("").contains(" 200 ") {
        return Err("control HTTP error");
    }
    serde_json::from_slice(&response[split + 4..]).map_err(|_| "invalid control response")
}

fn result(v: &Result<Value, &'static str>, expected: &str) -> &'static str {
    match v {
        Ok(v) if v["status"] == expected => "supported",
        Err("control unavailable") => "missing",
        Ok(v)
            if v["status"] == "error"
                && (v["code"] == "unsupported" || v["code"] == "unknown-command") =>
        {
            "missing"
        }
        _ => "error",
    }
}
fn row(name: &str, status: &str, detail: &str) {
    println!("{name:<32} {status:<10} {detail}");
}
fn cli(path: &Path, args: &[&str]) -> Option<std::process::Output> {
    Command::new(path)
        .args(args)
        .creation_flags(0x08000000)
        .output()
        .ok()
}
fn help_has(out: &Option<std::process::Output>, text: &str) -> bool {
    out.as_ref().is_some_and(|o| {
        o.status.success()
            && String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .any(|w| w == text)
    })
}
fn advertised(yes: bool) -> &'static str {
    if yes { "supported" } else { "error" }
}

pub fn run() -> i32 {
    println!("Hex compatibility probe (read-only)");
    let path =
        PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join("hex/local-api.json");
    let d: Discovery = match fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
    {
        Some(d) => d,
        None => {
            eprintln!("Discovery unavailable or invalid (credential redacted)");
            return 1;
        }
    };
    if d.token.contains(['\r', '\n']) {
        eprintln!("Invalid discovery credential (redacted)");
        return 1;
    }
    println!(
        "Discovery API version: {}",
        d.api_version
            .as_deref()
            .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit() || b == b'.'))
            .unwrap_or("unknown")
    );
    let mut version = 4;
    let mut status = control(&d, version, "status", None);
    if let Ok(v) = &status
        && v["code"] == "version-mismatch"
        && let Some(server_version) = v["version"].as_u64().filter(|n| (1..=4).contains(n))
    {
        version = server_version;
        status = control(&d, version, "status", None);
    }
    println!(
        "Control protocol: {}",
        status
            .as_ref()
            .ok()
            .and_then(|v| v["version"].as_u64())
            .map_or("unknown".to_string(), |n| n.to_string())
    );
    let report = status.as_ref().ok();
    let app_version = report.and_then(|v| v["app_version"].as_str()).filter(|v| {
        !v.is_empty()
            && v.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
    });
    println!("Hex version: {}", app_version.unwrap_or("unknown"));
    let exe = app_version
        .map(|v| {
            PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default())
                .join("hex/versions")
                .join(v)
                .join("hex.com")
        })
        .or_else(|| {
            let pid = d.pid?;
            let output = sidenote::platform::windows_powershell()
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    &format!("(Get-Process -Id {pid}).Path"),
                ])
                .output()
                .ok()?;
            if !output.status.success() {
                return None;
            }
            let path = PathBuf::from(String::from_utf8(output.stdout).ok()?.trim());
            if !path.file_name()?.eq_ignore_ascii_case("hex.exe") {
                return None;
            }
            Some(path.with_file_name("hex.com"))
        });
    let exe = exe.filter(|_| {
        !PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default())
            .join("hex/gpu-draw.pending")
            .exists()
    });
    let root_help = exe.as_deref().and_then(|p| cli(p, &["--help"]));
    let meetings_help = exe.as_deref().and_then(|p| cli(p, &["meetings", "--help"]));
    let dictation_help = exe
        .as_deref()
        .and_then(|p| cli(p, &["dictation", "--help"]));
    let settings_help = exe.as_deref().and_then(|p| cli(p, &["settings", "--help"]));
    let vocabulary_help = exe
        .as_deref()
        .and_then(|p| cli(p, &["vocabulary", "--help"]));
    let history = exe
        .as_deref()
        .filter(|_| help_has(&root_help, "history"))
        .and_then(|p| cli(p, &["history", "--json"]));
    let history_count = history
        .as_ref()
        .filter(|o| o.status.success())
        .and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok())
        .and_then(|v| {
            v.as_array()
                .map(Vec::len)
                .or_else(|| v["entries"].as_array().map(Vec::len))
        });
    let model_status = exe
        .as_deref()
        .filter(|_| help_has(&root_help, "model"))
        .and_then(|p| cli(p, &["model", "status"]));
    let settings = control(&d, version, "settings-get", None);
    let get = control(&d, version, "settings-get", Some("meetings"));
    let models = control(&d, version, "model-list", None);
    let vocabulary = control(&d, version, "vocabulary-list", None);
    println!("\n{:<32} {:<10} Evidence", "Feature", "Result");
    row(
        "Protocol / status",
        result(&status, "status"),
        "authenticated read",
    );
    row(
        "Capabilities",
        advertised(report.is_some_and(|v| v["capabilities"].is_object())),
        "status report",
    );
    row(
        "Dictation state",
        advertised(report.is_some_and(|v| v["capture"]["state"].is_string())),
        "status report",
    );
    for (label, word, help) in [
        ("Record start", "record", &meetings_help),
        ("Record stop", "stop", &meetings_help),
        ("Dictation start", "start", &dictation_help),
        ("Dictation stop", "stop", &dictation_help),
        ("Dictation toggle", "toggle", &dictation_help),
        ("Paste last", "paste-last", &root_help),
        ("Settings set", "set", &settings_help),
        ("Vocabulary add", "add", &vocabulary_help),
        ("Vocabulary remove", "remove", &vocabulary_help),
    ] {
        row(
            label,
            advertised(help_has(help, word)),
            if help_has(help, word) {
                "present, not exercised (CLI help)"
            } else {
                "not established by help"
            },
        );
    }
    for name in [
        "Meetings list",
        "Meetings show latest",
        "Meetings search",
        "Meetings chapters",
        "Rolling transcript",
    ] {
        row(
            name,
            "error",
            "not exercised: meetings directory prohibited",
        );
    }
    row(
        "Settings list",
        result(&settings, "settings"),
        "values suppressed",
    );
    row(
        "Settings get",
        result(&get, "settings"),
        "values suppressed",
    );
    row(
        "Vocabulary list",
        result(&vocabulary, "vocabulary"),
        "entries suppressed",
    );
    row(
        "Models list",
        result(&models, "models"),
        "values suppressed",
    );
    row(
        "Models status",
        advertised(model_status.as_ref().is_some_and(|o| o.status.success())),
        "CLI output suppressed",
    );
    row(
        "History list",
        advertised(history_count.is_some()),
        &history_count.map_or("count unavailable".to_string(), |n| {
            format!("{n} returned; text suppressed")
        }),
    );
    row(
        "Hotkeys",
        advertised(result(&settings, "settings") == "supported"),
        "settings registry; actual bindings suppressed",
    );
    row(
        "Headless / hidden host",
        "error",
        "not exercised; no verified capability",
    );
    row(
        "Events stream",
        "error",
        "not connected: preserve existing subscriber",
    );
    row(
        "CLI output capture",
        advertised(
            root_help
                .as_ref()
                .is_some_and(|o| o.status.success() && !o.stdout.is_empty()),
        ),
        "hex.com --help; hidden process, piped stdout",
    );
    if let Some(caps) = report.and_then(|v| v["capabilities"].as_object()) {
        println!("\nReported capabilities:");
        for (key, value) in caps {
            if let Some(enabled) = value.as_bool()
                && key
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            {
                println!("  {key}: {enabled}");
            }
        }
    }
    if result(&status, "status") == "supported" {
        0
    } else {
        1
    }
}
