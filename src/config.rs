use std::env;

pub const DEFAULT_HOST: &str = "http://localhost:3000/";
pub const HOST_ENV_VAR: &str = "PROGRAM_API_HOST";

const PROGRAM_API_PATH: &str = "api/programs";

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub host: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            host: configured_host(),
        }
    }
}

impl AppConfig {
    pub fn api_base_url(&self) -> String {
        program_api_base_url(&self.host)
    }
}

pub fn configured_host() -> String {
    env::var(HOST_ENV_VAR).unwrap_or_else(|_| DEFAULT_HOST.to_owned())
}

pub fn normalize_host(host: &str) -> String {
    let trimmed = host.trim();
    let with_scheme = if has_scheme(trimmed) {
        trimmed.to_owned()
    } else {
        format!("http://{trimmed}")
    };

    with_scheme.trim_end_matches('/').to_owned()
}

pub fn program_api_base_url(host: &str) -> String {
    let normalized = normalize_host(host);

    if normalized.ends_with(PROGRAM_API_PATH) {
        normalized
    } else {
        format!("{normalized}/{PROGRAM_API_PATH}")
    }
}

fn has_scheme(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_api_url_from_simple_localhost_host() {
        assert_eq!(
            program_api_base_url("localhost:3000/"),
            "http://localhost:3000/api/programs"
        );
    }

    #[test]
    fn keeps_existing_api_path() {
        assert_eq!(
            program_api_base_url("http://localhost:3000/api/programs"),
            "http://localhost:3000/api/programs"
        );
    }
}
