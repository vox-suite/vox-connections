use super::food_delivery::{Client, FoodDeliveryError};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Serialize, Deserialize)]
pub(crate) struct SetupCredentials {
    pub verifier: String,
    pub client_id: String,
}
#[derive(Deserialize)]
pub(crate) struct Tokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: i64,
    pub token_type: String,
}
impl Tokens {
    fn validate(self) -> Result<Self, FoodDeliveryError> {
        if self.access_token.trim().is_empty()
            || !self.token_type.eq_ignore_ascii_case("bearer")
            || self.expires_in <= 0
            || self.expires_in > 31_536_000
        {
            return Err(FoodDeliveryError::Parse(
                "invalid OAuth token response".into(),
            ));
        }
        Ok(self)
    }
}
impl Client {
    fn auth_base(&self) -> &str {
        &self.auth_base
    }
    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }
    pub(crate) async fn register(&self, redirect: &str) -> Result<String, FoodDeliveryError> {
        let response = self.http.post(format!("{}/register", self.auth_base()))
            .json(&json!({"client_name":"Vox", "redirect_uris":[redirect], "grant_types":["authorization_code","refresh_token"],
                "response_types":["code"], "token_endpoint_auth_method":"none"}))
            .send().await.map_err(|_| FoodDeliveryError::Http("OAuth registration unavailable".into()))?;
        if !response.status().is_success() {
            return Err(FoodDeliveryError::Http(format!(
                "OAuth registration HTTP {}",
                response.status()
            )));
        }
        let value: Value = response
            .json()
            .await
            .map_err(|_| FoodDeliveryError::Parse("invalid OAuth registration".into()))?;
        value
            .get("client_id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .ok_or_else(|| {
                FoodDeliveryError::Parse("OAuth registration did not return a client ID".into())
            })
    }
    pub(crate) fn auth_url(
        &self,
        client_id: &str,
        redirect: &str,
        state: &str,
        verifier: &str,
    ) -> String {
        let mut url = url::Url::parse(&format!("{}/authorize", self.auth_base())).unwrap();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        url.query_pairs_mut().extend_pairs([
            ("response_type", "code"),
            ("client_id", client_id),
            ("redirect_uri", redirect),
            ("state", state),
            ("code_challenge_method", "S256"),
            ("code_challenge", &challenge),
            ("scope", "mcp:tools"),
        ]);
        url.into()
    }
    async fn tokens(&self, fields: Value) -> Result<Tokens, FoodDeliveryError> {
        let request = self.http.post(format!("{}/token", self.auth_base()));
        let request = if self.provider == "swiggy" {
            request.json(&fields)
        } else {
            let fields: Vec<(&str, &str)> = fields
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str().unwrap_or_default()))
                .collect();
            let body = url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(fields)
                .finish();
            request
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(body)
        };
        let response = request
            .send()
            .await
            .map_err(|_| FoodDeliveryError::Http("OAuth endpoint unavailable".into()))?;
        if !response.status().is_success() {
            if response.status().as_u16() == 401 {
                return Err(FoodDeliveryError::Unauthorized);
            }
            let status = response.status();
            let error: Value = response.json().await.unwrap_or(Value::Null);
            if matches!(
                error.get("error").and_then(Value::as_str),
                Some("invalid_grant" | "invalid_token")
            ) {
                return Err(FoodDeliveryError::Unauthorized);
            }
            return Err(FoodDeliveryError::Http(format!("OAuth HTTP {status}")));
        }
        response
            .json::<Tokens>()
            .await
            .map_err(|_| FoodDeliveryError::Parse("invalid token response".into()))?
            .validate()
    }
    pub(crate) async fn exchange(
        &self,
        client_id: &str,
        redirect: &str,
        code: &str,
        verifier: &str,
    ) -> Result<Tokens, FoodDeliveryError> {
        self.tokens(json!({"grant_type":"authorization_code", "client_id":client_id, "redirect_uri":redirect,"code":code,"code_verifier":verifier})).await
    }
    pub(crate) async fn refresh(
        &self,
        client_id: &str,
        refresh: &str,
    ) -> Result<Tokens, FoodDeliveryError> {
        self.tokens(
            json!({"grant_type":"refresh_token","client_id":client_id,"refresh_token":refresh}),
        )
        .await
    }
}
