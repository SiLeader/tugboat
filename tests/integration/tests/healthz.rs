#[path = "../helpers/mod.rs"]
mod helpers;

use std::error::Error;

use helpers::setup::TestContext;

#[tokio::test]
async fn healthz_is_available() -> Result<(), Box<dyn Error + Send + Sync>> {
    let Some(ctx) = TestContext::setup().await? else {
        eprintln!(
            "skipping integration test: set {} or {} (or install docker) to enable",
            "TUGBOAT_TEST_APISERVER_URL", "TUGBOAT_TEST_ETCD_ENDPOINT"
        );
        return Ok(());
    };

    let response = reqwest::get(format!("{}/healthz", ctx.base_url)).await?;
    assert!(response.status().is_success());
    let _client = ctx.client.clone();
    Ok(())
}
