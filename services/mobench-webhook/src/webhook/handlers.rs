use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{AppState, db::models::DeliveryRecord};

pub async fn handle_delivery(state: &AppState, delivery: &DeliveryRecord) -> Result<()> {
    match (delivery.event.as_str(), delivery.action.as_deref()) {
        ("pull_request", Some("labeled")) => {
            handle_pull_request_labeled(state, &delivery.payload).await
        }
        _ => Ok(()),
    }
}

async fn handle_pull_request_labeled(state: &AppState, payload: &Value) -> Result<()> {
    let event = match serde_json::from_value::<PullRequestLabeledEvent>(payload.clone()) {
        Ok(event) => event,
        Err(_) => return Ok(()),
    };
    match event.label.as_ref().map(|label| label.name.as_str()) {
        Some("bench") => {}
        _ => return Ok(()),
    }

    if event.pull_request.state != "open" {
        return Ok(());
    }

    if event.pull_request.head.repo.full_name != event.pull_request.base.repo.full_name {
        return Ok(());
    }

    let workflow_inputs = normalized_label_inputs(event.number, &event.sender.login);
    let repo_owner = event.pull_request.base.repo.owner.login.as_str();
    let repo_name = event.pull_request.base.repo.name.as_str();
    let head_sha = event.pull_request.head.sha.as_str();
    let head_ref = event.pull_request.head.reference.as_str();

    if state
        .repos
        .dispatches
        .find_active_duplicate(repo_owner, repo_name, head_sha, &workflow_inputs)
        .await?
        .is_some()
    {
        return Ok(());
    }

    let dispatch_id = Uuid::new_v4();
    let dispatch = state
        .repos
        .dispatches
        .insert_queued(
            dispatch_id,
            repo_owner,
            repo_name,
            head_sha,
            head_ref,
            Some(event.number),
            "label",
            Some(&event.sender.login),
            workflow_inputs.clone(),
        )
        .await?;
    let github = state
        .github
        .as_ref()
        .context("missing GitHub clients for dispatch handling")?;

    github
        .workflows
        .dispatch_mobile_bench(
            repo_owner,
            repo_name,
            head_ref,
            &workflow_inputs_for_dispatch(dispatch.dispatch_id, "label", &workflow_inputs),
        )
        .await?;

    Ok(())
}

fn normalized_label_inputs(pr_number: i32, requested_by: &str) -> Value {
    json!({
        "platform": "both",
        "device_profile": "low-spec",
        "ios_device": "",
        "ios_os_version": "",
        "android_device": "",
        "android_os_version": "",
        "iterations": "30",
        "warmup": "5",
        "pr_number": pr_number.to_string(),
        "requested_by": requested_by,
    })
}

fn workflow_inputs_for_dispatch(
    dispatch_id: Uuid,
    trigger_source: &str,
    workflow_inputs: &Value,
) -> Value {
    let mut workflow_inputs = workflow_inputs.clone();
    if let Some(object) = workflow_inputs.as_object_mut() {
        object.insert(
            "dispatch_id".to_string(),
            Value::String(dispatch_id.to_string()),
        );
        object.insert(
            "trigger_source".to_string(),
            Value::String(trigger_source.to_string()),
        );
    }

    workflow_inputs
}

#[derive(Debug, Deserialize)]
struct PullRequestLabeledEvent {
    number: i32,
    label: Option<EventLabel>,
    pull_request: PullRequestSummary,
    sender: GitHubUser,
}

#[derive(Debug, Deserialize)]
struct EventLabel {
    name: String,
}

#[derive(Debug, Deserialize)]
struct PullRequestSummary {
    state: String,
    head: GitHubRef,
    base: GitHubBaseRef,
}

#[derive(Debug, Deserialize)]
struct GitHubRef {
    #[serde(rename = "ref")]
    reference: String,
    sha: String,
    repo: GitHubRepo,
}

#[derive(Debug, Deserialize)]
struct GitHubBaseRef {
    repo: GitHubRepo,
}

#[derive(Debug, Deserialize)]
struct GitHubRepo {
    name: String,
    full_name: String,
    owner: GitHubUser,
}

#[derive(Debug, Deserialize)]
struct GitHubUser {
    login: String,
}
