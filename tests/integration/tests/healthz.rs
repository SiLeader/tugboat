#[path = "../helpers/mod.rs"]
mod helpers;

use std::error::Error;

use helpers::setup::TestContext;

#[tokio::test]
async fn healthz_is_available() -> Result<(), Box<dyn Error + Send + Sync>> {
    let Some(ctx) = TestContext::setup().await? else {
        eprintln!(
            "skipping integration test: set TUGBOAT_TEST_APISERVER_URL or TUGBOAT_TEST_ETCD_ENDPOINT (or install docker) to enable"
        );
        return Ok(());
    };

    let response = ctx
        .http_client()?
        .get(format!("{}/healthz", ctx.base_url))
        .send()
        .await?;
    assert!(response.status().is_success());
    let _client = ctx.client.clone();
    Ok(())
}
