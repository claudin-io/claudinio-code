use reqwest::{Client, ClientBuilder};

const USER_AGENT: &str = concat!("Claudinio-Code/", env!("CARGO_PKG_VERSION"));

/// Returns a `reqwest::Client` pre-configured with the Claudinio-Code User-Agent.
/// Use instead of `reqwest::Client::new()`.
pub fn default_client() -> Client {
    Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .expect("hardcoded user_agent always builds")
}

/// Returns a `reqwest::ClientBuilder` pre-configured with the Claudinio-Code User-Agent.
/// Use instead of `reqwest::Client::builder()` — callers can add timeouts before `.build()`.
pub fn default_client_builder() -> ClientBuilder {
    Client::builder().user_agent(USER_AGENT)
}
