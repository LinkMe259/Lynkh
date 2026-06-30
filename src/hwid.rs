use sha2::{Digest, Sha256};
use std::process::Command;

pub fn stable_hwid() -> String {
    let raw_id = platform_machine_id().unwrap_or_else(fallback_machine_id);
    sha256_hex(raw_id.trim().as_bytes())
}

#[cfg(target_os = "macos")]
fn platform_machine_id() -> Option<String> {
    let output = Command::new("ioreg")
        .args(["-rd1", "-c", "IOPlatformExpertDevice"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().find_map(|line| {
        line.split_once("IOPlatformUUID")
            .and_then(|(_, value)| value.split_once('"'))
            .and_then(|(_, rest)| rest.split_once('"'))
            .map(|(uuid, _)| uuid.trim().to_owned())
            .filter(|uuid| !uuid.is_empty())
    })
}

#[cfg(target_os = "windows")]
fn platform_machine_id() -> Option<String> {
    let output = Command::new("wmic")
        .args(["csproduct", "get", "uuid"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.eq_ignore_ascii_case("uuid"))
        .map(ToOwned::to_owned)
}

#[cfg(target_os = "linux")]
fn platform_machine_id() -> Option<String> {
    std::fs::read_to_string("/etc/machine-id")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn platform_machine_id() -> Option<String> {
    None
}

fn fallback_machine_id() -> String {
    let hostname = Command::new("hostname")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .unwrap_or_else(|| "unknown-host".to_owned());

    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown-user".to_owned());

    format!("{}:{}", hostname.trim(), user.trim())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
