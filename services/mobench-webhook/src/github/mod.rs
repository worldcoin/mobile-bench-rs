pub mod artifacts;
pub mod auth;
pub mod checks;
pub mod workflows;

#[derive(Clone)]
pub struct GitHubClients {
    pub workflows: workflows::GitHubWorkflowsClient,
    pub artifacts: artifacts::GitHubArtifactsClient,
    pub checks: checks::GitHubChecksClient,
}

impl GitHubClients {
    pub fn new(auth: auth::GitHubAppAuth, api_base_url: impl Into<String>) -> Self {
        let api_base_url = api_base_url.into();

        Self {
            workflows: workflows::GitHubWorkflowsClient::new(auth.clone(), api_base_url.clone()),
            artifacts: artifacts::GitHubArtifactsClient::new(auth.clone(), api_base_url.clone()),
            checks: checks::GitHubChecksClient::new(auth, api_base_url),
        }
    }
}
