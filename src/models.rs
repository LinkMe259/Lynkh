use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct LoginRequest {
    pub user: String,
    pub password: String,
    pub hwid: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginResponse {
    pub token: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MeResponse {
    pub user: UserInfo,
    pub session: SessionInfo,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RentalsResponse {
    pub rentals: Vec<Rental>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub expires_at: String,
    pub hwid_locked: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserInfo {
    pub id: String,
    pub name: String,
    pub email: String,
    pub role: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rental {
    pub product_id: String,
    pub product_name: String,
    pub product_status: String,
    pub product_status_label: String,
    pub rental_status: String,
    pub is_permanent: bool,
    pub started_at: String,
    pub updated_at: String,
    pub expires_at: Option<String>,
    pub remaining_seconds: Option<i64>,
}
