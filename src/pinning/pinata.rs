/// Pinata pinByCID API
/// Docs: https://docs.pinata.cloud/api-reference/endpoint/pin-by-cid
use serde_json::json;
use tracing::debug;

pub async fn pin_by_cid(jwt: &str, cid: &str) -> Result<(), anyhow::Error> {
    let client = reqwest::Client::new();
    let body = json!({
        "hashToPin": cid,
        "pinataOptions": { "cidVersion": 1 }
    });

    debug!(%cid, "pinning to pinata");

    let resp = client
        .post("https://api.pinata.cloud/pinning/pinByHash")
        .bearer_auth(jwt)
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("pinata error {status}: {text}"));
    }

    Ok(())
}
