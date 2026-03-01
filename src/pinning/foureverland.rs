/// 4EVERLAND pinning API
/// 4EVERLAND exposes a Pinata-compatible pinning API endpoint.
use std::time::Duration;

use serde_json::json;
use tracing::debug;

pub async fn pin_by_cid(token: &str, cid: &str) -> Result<(), anyhow::Error> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;
    let body = json!({
        "cid": cid,
        "name": cid
    });

    debug!(%cid, "pinning to 4everland");

    // 4EVERLAND IPFS pinning endpoint (Pinata-compatible)
    let resp = client
        .post("https://api.4everland.dev/pin/pinByHash")
        .bearer_auth(token)
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("4everland error {status}: {text}"));
    }

    Ok(())
}
