use crate::models::{LoginRequest, LoginResponse, MeResponse, RentalsResponse};
use reqwest::blocking::{Client, Response};

#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: Option<u16>,
    pub message: String,
}

impl ApiError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            status: None,
            message: message.into(),
        }
    }

    fn from_response(status: reqwest::StatusCode, body: String) -> Self {
        let message = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|value| {
                value
                    .get("message")
                    .or_else(|| value.get("error"))
                    .and_then(|field| field.as_str())
                    .map(ToOwned::to_owned)
            })
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| match status.as_u16() {
                401 => "Username, password, or token is invalid.".to_owned(),
                403 => "Account is banned or this HWID is not allowed.".to_owned(),
                429 => "Too many login attempts. Please wait and try again.".to_owned(),
                code => {
                    if body.trim().is_empty() {
                        format!("Request failed with status {code}.")
                    } else {
                        body
                    }
                }
            });

        Self {
            status: Some(status.as_u16()),
            message,
        }
    }
}

pub fn login(base_url: &str, request: LoginRequest) -> Result<LoginResponse, ApiError> {
    let response = http_client()?
        .post(format!("{base_url}/login"))
        .json(&request)
        .send()
        .map_err(|error| ApiError::new(error.to_string()))?;

    parse_json_response(response)
}

pub fn me(base_url: &str, token: &str) -> Result<MeResponse, ApiError> {
    let response = http_client()?
        .get(format!("{base_url}/me"))
        .bearer_auth(token)
        .send()
        .map_err(|error| ApiError::new(error.to_string()))?;

    parse_json_response(response)
}

pub fn rentals(base_url: &str, token: &str) -> Result<RentalsResponse, ApiError> {
    let response = http_client()?
        .get(format!("{base_url}/rentals"))
        .bearer_auth(token)
        .send()
        .map_err(|error| ApiError::new(error.to_string()))?;

    parse_json_response(response)
}

pub fn logout(base_url: &str, token: &str) -> Result<(), ApiError> {
    let response = http_client()?
        .post(format!("{base_url}/logout"))
        .bearer_auth(token)
        .send()
        .map_err(|error| ApiError::new(error.to_string()))?;

    parse_empty_response(response)
}

fn http_client() -> Result<Client, ApiError> {
    Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent(format!(
            "{}/{}",
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .map_err(|error| ApiError::new(error.to_string()))
}

fn parse_json_response<T: for<'de> serde::Deserialize<'de>>(
    response: Response,
) -> Result<T, ApiError> {
    let status = response.status();
    let body = response
        .text()
        .map_err(|error| ApiError::new(error.to_string()))?;

    if !status.is_success() {
        return Err(ApiError::from_response(status, body));
    }

    serde_json::from_str(&body).map_err(|error| ApiError::new(format!("Invalid JSON: {error}")))
}

fn parse_empty_response(response: Response) -> Result<(), ApiError> {
    let status = response.status();
    let body = response.text().unwrap_or_default();

    if status.is_success() {
        Ok(())
    } else {
        Err(ApiError::from_response(status, body))
    }
}
