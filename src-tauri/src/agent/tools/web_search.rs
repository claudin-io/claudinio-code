use crate::agent::app_sign;
use crate::agent::provider::AgentConfig;
use serde::Deserialize;
use serde::Serialize;

#[derive(Deserialize)]
pub struct WebSearchArgs {
    pub query: String,
    pub max_results: Option<u32>,
}

#[derive(Serialize, Deserialize)]
struct WebSearchResult {
    title: String,
    url: String,
    content: String,
    score: Option<f64>,
}

#[derive(Serialize, Deserialize)]
struct WebSearchResponse {
    answer: Option<String>,
    results: Vec<WebSearchResult>,
    query: String,
}

#[derive(Deserialize)]
struct ErrorResponse {
    error: String,
    retry_after: Option<u64>,
    upgrade_url: Option<String>,
}

pub async fn execute(args: WebSearchArgs, config: &AgentConfig) -> Result<String, String> {
    if config.api_key.is_empty() {
        return Err("Not logged in — sign in with claudin.io in Settings to use web search.".into());
    }

    let path = "/api/app/websearch";
    let url = format!("{}{}", config.services_url.trim_end_matches('/'), path);

    let mut body = serde_json::json!({ "query": args.query });
    if let Some(n) = args.max_results {
        body["max_results"] = serde_json::json!(n.clamp(1, 10));
    }
    let body_bytes = serde_json::to_vec(&body).map_err(|e| format!("encode request: {e}"))?;

    let signature_headers = app_sign::sign("POST", path, &body_bytes);

    let client = crate::http::default_client();
    let mut req = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", config.api_key))
        .body(body_bytes);
    for (name, value) in signature_headers {
        req = req.header(name, value);
    }

    let resp = req.send().await.map_err(|e| format!("web search request failed: {e}"))?;
    let status = resp.status();

    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        let retry_after = resp
            .json::<ErrorResponse>()
            .await
            .ok()
            .and_then(|e| e.retry_after)
            .unwrap_or(60);
        return Err(format!(
            "Web search rate limit reached — try again in {retry_after}s."
        ));
    }
    if status == reqwest::StatusCode::FORBIDDEN {
        let msg = resp
            .json::<ErrorResponse>()
            .await
            .ok()
            .map(|e| match e.upgrade_url {
                Some(u) => format!("{} ({u})", e.error),
                None => e.error,
            })
            .unwrap_or_else(|| "forbidden".into());
        return Err(format!("Web search unavailable: {msg}"));
    }
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err("Web search auth failed — sign in again with claudin.io in Settings.".into());
    }
    if !status.is_success() {
        return Err(format!("Web search failed with status {status}"));
    }

    let parsed: WebSearchResponse = resp
        .json()
        .await
        .map_err(|e| format!("invalid web search response: {e}"))?;

    let mut out = String::new();
    if let Some(answer) = parsed.answer.filter(|a| !a.is_empty()) {
        out.push_str(&answer);
        out.push_str("\n\n");
    }
    if parsed.results.is_empty() {
        out.push_str("No results found.");
    } else {
        for (i, r) in parsed.results.iter().enumerate() {
            out.push_str(&format!("{}. {} — {}\n", i + 1, r.title, r.url));
            if !r.content.is_empty() {
                out.push_str(&format!("   {}\n", r.content));
            }
        }
    }
    Ok(out)
}
