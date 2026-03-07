use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::{AppState, db::models::DeliveryRecord, webhook::commands::ManualRunArgs};

pub async fn handle_delivery(state: &AppState, delivery: &DeliveryRecord) -> Result<()> {
    match (delivery.event.as_str(), delivery.action.as_deref()) {
        ("pull_request", Some("labeled")) => {
            handle_pull_request_labeled(state, &delivery.payload).await
        }
        ("issue_comment", Some("created")) => {
            handle_issue_comment_created(state, &delivery.payload).await
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

    let workflow_inputs =
        ManualRunArgs::default().workflow_inputs(event.number, &event.sender.login);
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
            &workflow_inputs_for_dispatch(dispatch.dispatch_id, "label", None, &workflow_inputs),
        )
        .await?;

    Ok(())
}

async fn handle_issue_comment_created(state: &AppState, payload: &Value) -> Result<()> {
    let event = match serde_json::from_value::<IssueCommentCreatedEvent>(payload.clone()) {
        Ok(event) => event,
        Err(_) => return Ok(()),
    };
    if event.issue.pull_request.is_none() {
        return Ok(());
    }

    if !matches!(
        event.comment.author_association.as_str(),
        "OWNER" | "MEMBER" | "COLLABORATOR"
    ) {
        return Ok(());
    }

    let workflow_inputs = match ManualRunArgs::parse_manual_command(&event.comment.body) {
        Some(args) => args.workflow_inputs(event.issue.number, &event.comment.user.login),
        None => return Ok(()),
    };
    let github = state
        .github
        .as_ref()
        .context("missing GitHub clients for dispatch handling")?;
    let repo_owner = event.repository.owner.login.as_str();
    let repo_name = event.repository.name.as_str();
    let pull_request = github
        .pull_requests
        .get_pull_request(repo_owner, repo_name, event.issue.number)
        .await?;
    if pull_request.state != "open" {
        return Ok(());
    }

    if pull_request.head.repo.full_name != pull_request.base.repo.full_name {
        return Ok(());
    }

    let head_sha = pull_request.head.sha.as_str();
    let head_ref = pull_request.head.reference.as_str();

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
            Some(event.issue.number),
            "pr_comment",
            Some(&event.comment.user.login),
            workflow_inputs.clone(),
        )
        .await?;

    github
        .workflows
        .dispatch_mobile_bench(
            repo_owner,
            repo_name,
            head_ref,
            &workflow_inputs_for_dispatch(
                dispatch.dispatch_id,
                "pr_comment",
                Some(event.comment.body.trim()),
                &workflow_inputs,
            ),
        )
        .await?;

    Ok(())
}

fn workflow_inputs_for_dispatch(
    dispatch_id: Uuid,
    trigger_source: &str,
    request_command: Option<&str>,
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
        if let Some(request_command) = request_command {
            object.insert(
                "request_command".to_string(),
                Value::String(request_command.to_string()),
            );
        }
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
struct IssueCommentCreatedEvent {
    repository: GitHubRepo,
    issue: IssueSummary,
    comment: IssueComment,
}

#[derive(Debug, Deserialize)]
struct EventLabel {
    name: String,
}

#[derive(Debug, Deserialize)]
struct IssueSummary {
    number: i32,
    pull_request: Option<IssuePullRequestContext>,
}

#[derive(Debug, Deserialize)]
struct IssueComment {
    body: String,
    author_association: String,
    user: GitHubUser,
}

#[derive(Debug, Deserialize)]
struct IssuePullRequestContext {}

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
