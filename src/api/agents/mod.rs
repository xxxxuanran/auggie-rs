mod codebase_retrieval;

use anyhow::Result;
use serde::de::DeserializeOwned;
use serde::Serialize;

use super::client::ApiClient;

const AGENTS_ENDPOINT_PREFIX: &str = "agents/";

fn agents_endpoint(endpoint: &str) -> String {
    let endpoint = endpoint.trim_start_matches('/');
    if endpoint.starts_with(AGENTS_ENDPOINT_PREFIX) {
        endpoint.to_string()
    } else {
        format!("{}{}", AGENTS_ENDPOINT_PREFIX, endpoint)
    }
}

pub struct AgentsApi<'a> {
    pub(super) api: &'a ApiClient,
}

impl<'a> AgentsApi<'a> {
    async fn call_api_with_timeout<T, R>(
        &self,
        endpoint: &str,
        base_url: &str,
        access_token: Option<&str>,
        body: &T,
        timeout_secs: u64,
    ) -> Result<R>
    where
        T: Serialize,
        R: DeserializeOwned,
    {
        let endpoint = agents_endpoint(endpoint);
        self.api
            .call_api_with_timeout(&endpoint, base_url, access_token, body, timeout_secs)
            .await
    }
}

impl ApiClient {
    pub fn agents(&self) -> AgentsApi<'_> {
        AgentsApi { api: self }
    }
}
