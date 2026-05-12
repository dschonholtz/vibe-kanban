use api_types::{Issue, ListIssuesResponse, MutationResponse, UpdateIssueRequest};
use db::models::{
    execution_process::{ExecutionProcess, ExecutionProcessStatus},
    repo::Repo,
    requests::{
        CreateAndStartWorkspaceRequest, CreateAndStartWorkspaceResponse, LinkedIssueInfo,
        WorkspaceRepoInput,
    },
};
use executors::{model_selector::PermissionPolicy, profile::ExecutorConfig};
use rmcp::{
    ErrorData, handler::server::wrapper::Parameters, model::CallToolResult, schemars, tool,
    tool_router,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{McpServer, ToolError};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct McpWorkspaceRepoInput {
    #[schemars(description = "The repository ID")]
    repo_id: Uuid,
    #[schemars(description = "The branch for this repository")]
    branch: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct StartWorkspaceRequest {
    #[schemars(description = "Name for the workspace")]
    name: String,
    #[schemars(
        description = "Optional prompt for the first workspace session. If omitted/empty, the linked issue title/description is used."
    )]
    prompt: Option<String>,
    #[schemars(
        description = "The coding agent executor to run ('CLAUDE_CODE', 'AMP', 'GEMINI', 'CODEX', 'OPENCODE', 'CURSOR_AGENT', 'QWEN_CODE', 'COPILOT', 'DROID')"
    )]
    executor: String,
    #[schemars(description = "Optional executor variant, if needed")]
    variant: Option<String>,
    #[schemars(description = "Optional model override for the executor, such as 'gpt-5.5'")]
    model_id: Option<String>,
    #[schemars(description = "Optional reasoning effort override, such as 'high'")]
    reasoning_id: Option<String>,
    #[schemars(description = "Optional permission policy: 'auto', 'supervised', or 'plan'")]
    permission_policy: Option<String>,
    #[schemars(description = "Repository selection for the workspace")]
    repositories: Vec<McpWorkspaceRepoInput>,
    #[schemars(
        description = "Optional issue ID to link the workspace to. When provided, the workspace will be associated with this remote issue."
    )]
    issue_id: Option<Uuid>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct StartWorkspaceResponse {
    workspace_id: String,
    execution_id: String,
    execution_status: String,
    execution: Value,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ExecuteIssueRequest {
    #[schemars(
        description = "Project ID containing the issue. Optional if issue_id is provided or the MCP context supplies a project."
    )]
    project_id: Option<Uuid>,
    #[schemars(description = "Issue UUID to execute. Prefer this when available.")]
    issue_id: Option<Uuid>,
    #[schemars(description = "Human-readable issue key, such as DFD-2.")]
    simple_id: Option<String>,
    #[schemars(description = "Optional workspace name. Defaults to '<simple_id> <title>'.")]
    name: Option<String>,
    #[schemars(
        description = "Optional prompt override. Defaults to the linked issue title and description."
    )]
    prompt: Option<String>,
    #[schemars(
        description = "Coding agent executor to run. Defaults to CODEX. Allowed values include CODEX, CLAUDE_CODE, CURSOR_AGENT, OPENCODE, GEMINI, AMP, QWEN_CODE, COPILOT, DROID."
    )]
    executor: Option<String>,
    #[schemars(description = "Optional executor variant, if needed")]
    variant: Option<String>,
    #[schemars(description = "Optional model override. Defaults to executor configuration.")]
    model_id: Option<String>,
    #[schemars(description = "Optional reasoning effort override, such as 'high'.")]
    reasoning_id: Option<String>,
    #[schemars(
        description = "Permission policy for unattended work. Defaults to 'auto'. Allowed values: auto, supervised, plan."
    )]
    permission_policy: Option<String>,
    #[schemars(description = "Repository UUID to use for the workspace.")]
    repo_id: Option<Uuid>,
    #[schemars(description = "Repository name/display name to use when repo_id is omitted.")]
    repo_name: Option<String>,
    #[schemars(
        description = "Target branch for the repository. Defaults to repo default branch or main."
    )]
    branch: Option<String>,
    #[schemars(
        description = "Start a fresh workspace even if this issue already has a running OpenClaw execution recorded."
    )]
    force_new_workspace: Option<bool>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct ExecuteIssueResponse {
    action: String,
    issue_id: String,
    simple_id: String,
    workspace_id: Option<String>,
    execution_id: Option<String>,
    execution_status: Option<String>,
    executor: String,
    repo_id: Option<String>,
    branch: Option<String>,
    already_running: bool,
    execution: Option<Value>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SyncProjectExecutionsRequest {
    #[schemars(
        description = "Project ID to reconcile. Optional if the MCP context supplies a project."
    )]
    project_id: Option<Uuid>,
    #[schemars(
        description = "Issue status to watch for runnable work. Defaults to 'In progress'."
    )]
    status: Option<String>,
    #[schemars(
        description = "Optional status to move issues to when their recorded execution completes, such as 'In review' or 'Done'. If omitted, completed issues stay in their current status."
    )]
    completed_status: Option<String>,
    #[schemars(
        description = "Optional status to move issues to when their recorded execution fails or is killed. If omitted, failed issues stay in their current status."
    )]
    failed_status: Option<String>,
    #[schemars(
        description = "Coding agent executor to run for newly-started issues. Defaults to CODEX."
    )]
    executor: Option<String>,
    #[schemars(description = "Optional executor variant, if needed.")]
    variant: Option<String>,
    #[schemars(description = "Optional model override for newly-started issues.")]
    model_id: Option<String>,
    #[schemars(description = "Optional reasoning effort override for newly-started issues.")]
    reasoning_id: Option<String>,
    #[schemars(
        description = "Permission policy for newly-started issues. Defaults to 'auto'. Allowed values: auto, supervised, plan."
    )]
    permission_policy: Option<String>,
    #[schemars(description = "Repository UUID to use for newly-started workspaces.")]
    repo_id: Option<Uuid>,
    #[schemars(description = "Repository name/display name to use when repo_id is omitted.")]
    repo_name: Option<String>,
    #[schemars(description = "Target branch for newly-started workspaces.")]
    branch: Option<String>,
    #[schemars(description = "Maximum matching issues to process. Defaults to 20.")]
    limit: Option<usize>,
    #[schemars(description = "Report what would happen without mutating issues or starting work.")]
    dry_run: Option<bool>,
    #[schemars(
        description = "Start a fresh workspace for issues whose recorded execution failed or was killed. Defaults to false."
    )]
    rerun_failed: Option<bool>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct SyncIssueExecutionSummary {
    issue_id: String,
    simple_id: String,
    title: String,
    action: String,
    workspace_id: Option<String>,
    execution_id: Option<String>,
    execution_status: Option<String>,
    issue_status: String,
    details: Option<String>,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct SyncProjectExecutionsResponse {
    project_id: String,
    watched_status: String,
    examined_count: usize,
    result_count: usize,
    dry_run: bool,
    results: Vec<SyncIssueExecutionSummary>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct LinkWorkspaceIssueRequest {
    #[schemars(description = "The workspace ID to link")]
    workspace_id: Uuid,
    #[schemars(description = "The issue ID to link the workspace to")]
    issue_id: Uuid,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct LinkWorkspaceIssueResponse {
    #[schemars(description = "Whether the linking was successful")]
    success: bool,
    #[schemars(description = "The workspace ID that was linked")]
    workspace_id: String,
    #[schemars(description = "The issue ID it was linked to")]
    issue_id: String,
}

fn build_workspace_prompt_from_issue(issue: &api_types::Issue) -> Option<String> {
    let title = issue.title.trim();
    let description = issue
        .description
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .unwrap_or_default();

    if title.is_empty() && description.is_empty() {
        return None;
    }

    if description.is_empty() {
        return Some(title.to_string());
    }

    if title.is_empty() {
        return Some(description.to_string());
    }

    Some(format!("{title}\n\n{description}"))
}

#[tool_router(router = task_attempts_tools_router, vis = "pub")]
impl McpServer {
    #[tool(description = "Create a new workspace and start its first session.")]
    async fn start_workspace(
        &self,
        Parameters(StartWorkspaceRequest {
            name,
            prompt,
            executor,
            variant,
            model_id,
            reasoning_id,
            permission_policy,
            repositories,
            issue_id,
        }): Parameters<StartWorkspaceRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        if repositories.is_empty() {
            return Self::err("At least one repository must be specified.", None::<&str>);
        }

        let executor_trimmed = executor.trim();
        if executor_trimmed.is_empty() {
            return Self::err("Executor must not be empty.", None::<&str>);
        }

        let prompt = prompt.and_then(|prompt| {
            let trimmed = prompt.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });

        let executor_config = match Self::executor_config_from_options(
            executor_trimmed,
            variant,
            model_id,
            reasoning_id,
            permission_policy,
            None,
        ) {
            Ok(config) => config,
            Err(error) => return Ok(Self::tool_error(error)),
        };

        let workspace_repos: Vec<WorkspaceRepoInput> = repositories
            .into_iter()
            .map(|r| WorkspaceRepoInput {
                repo_id: r.repo_id,
                target_branch: r.branch,
            })
            .collect();

        let (linked_issue, issue_prompt) = if let Some(issue_id) = issue_id {
            let issue_url = self.url(&format!("/api/remote/issues/{issue_id}"));
            let issue: api_types::Issue = match self.send_json(self.client.get(&issue_url)).await {
                Ok(issue) => issue,
                Err(e) => return Ok(Self::tool_error(e)),
            };

            (
                Some(LinkedIssueInfo {
                    remote_project_id: issue.project_id,
                    issue_id,
                }),
                build_workspace_prompt_from_issue(&issue),
            )
        } else {
            (None, None)
        };

        let workspace_prompt = match prompt.or(issue_prompt) {
            Some(prompt) => prompt,
            None => {
                return Self::err(
                    "Provide `prompt`, or `issue_id` that has a non-empty title/description.",
                    None::<&str>,
                );
            }
        };

        let create_and_start_payload = CreateAndStartWorkspaceRequest {
            name: Some(name.clone()),
            repos: workspace_repos,
            linked_issue,
            executor_config,
            prompt: workspace_prompt,
            attachment_ids: None,
        };

        let create_and_start_url = self.url("/api/workspaces/start");
        let create_and_start_response: CreateAndStartWorkspaceResponse = match self
            .send_json(
                self.client
                    .post(&create_and_start_url)
                    .json(&create_and_start_payload),
            )
            .await
        {
            Ok(response) => response,
            Err(e) => return Ok(Self::tool_error(e)),
        };

        // Link workspace to remote issue if issue_id is provided
        if let Some(issue_id) = issue_id
            && let Err(e) = self
                .link_workspace_to_issue(create_and_start_response.workspace.id, issue_id)
                .await
        {
            return Ok(Self::tool_error(e));
        }

        let response = StartWorkspaceResponse {
            workspace_id: create_and_start_response.workspace.id.to_string(),
            execution_id: create_and_start_response.execution_process.id.to_string(),
            execution_status: Self::execution_process_status_label(
                &create_and_start_response.execution_process.status,
            )
            .to_string(),
            execution: match Self::serialize_execution_process(
                &create_and_start_response.execution_process,
            ) {
                Ok(value) => value,
                Err(error) => return Ok(Self::tool_error(error)),
            },
        };

        McpServer::success(&response)
    }

    #[tool(
        description = "Execute a Vibe Kanban issue by creating a linked workspace and starting the requested coding agent. This is the preferred tool for 'start', 'run', 'execute', or 'progress' requests because it actually launches agent work instead of only moving the issue on the board."
    )]
    async fn execute_issue(
        &self,
        Parameters(ExecuteIssueRequest {
            project_id,
            issue_id,
            simple_id,
            name,
            prompt,
            executor,
            variant,
            model_id,
            reasoning_id,
            permission_policy,
            repo_id,
            repo_name,
            branch,
            force_new_workspace,
        }): Parameters<ExecuteIssueRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let issue = match self
            .resolve_issue_for_execution(project_id, issue_id, simple_id)
            .await
        {
            Ok(issue) => issue,
            Err(error) => return Ok(Self::tool_error(error)),
        };

        let response = match self
            .start_execution_for_issue(
                issue,
                name,
                prompt,
                executor,
                variant,
                model_id,
                reasoning_id,
                permission_policy,
                repo_id,
                repo_name,
                branch,
                force_new_workspace.unwrap_or(false),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => return Ok(Self::tool_error(error)),
        };

        McpServer::success(&response)
    }

    #[tool(
        description = "Reconcile a project board with agent execution state. It scans issues in a runnable status, starts missing executions with execute_issue semantics, reports already-running work, and can optionally move completed or failed executions to a configured status."
    )]
    async fn sync_project_executions(
        &self,
        Parameters(SyncProjectExecutionsRequest {
            project_id,
            status,
            completed_status,
            failed_status,
            executor,
            variant,
            model_id,
            reasoning_id,
            permission_policy,
            repo_id,
            repo_name,
            branch,
            limit,
            dry_run,
            rerun_failed,
        }): Parameters<SyncProjectExecutionsRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        let project_id = match self.resolve_project_id(project_id) {
            Ok(project_id) => project_id,
            Err(error) => return Ok(Self::tool_error(error)),
        };
        let watched_status =
            normalize_optional(status).unwrap_or_else(|| "In progress".to_string());
        let watched_status_id = match self.resolve_status_id(project_id, &watched_status).await {
            Ok(status_id) => status_id,
            Err(error) => return Ok(Self::tool_error(error)),
        };

        let completed_status_id = match normalize_optional(completed_status) {
            Some(status) => match self.resolve_status_id(project_id, &status).await {
                Ok(status_id) => Some((status, status_id)),
                Err(error) => return Ok(Self::tool_error(error)),
            },
            None => None,
        };
        let failed_status_id = match normalize_optional(failed_status) {
            Some(status) => match self.resolve_status_id(project_id, &status).await {
                Ok(status_id) => Some((status, status_id)),
                Err(error) => return Ok(Self::tool_error(error)),
            },
            None => None,
        };

        let url = self.url(&format!("/api/remote/issues?project_id={project_id}"));
        let response: ListIssuesResponse = match self.send_json(self.client.get(&url)).await {
            Ok(response) => response,
            Err(error) => return Ok(Self::tool_error(error)),
        };

        let dry_run = dry_run.unwrap_or(false);
        let rerun_failed = rerun_failed.unwrap_or(false);
        let limit = limit.unwrap_or(20);
        let mut examined_count = 0usize;
        let mut results = Vec::new();

        for issue in response
            .issues
            .into_iter()
            .filter(|issue| issue.status_id == watched_status_id)
            .take(limit)
        {
            examined_count += 1;
            let issue_id = issue.id.to_string();
            let simple_id = issue.simple_id.clone();
            let title = issue.title.clone();
            let result = match self
                .sync_issue_execution(
                    issue,
                    &watched_status,
                    completed_status_id.as_ref(),
                    failed_status_id.as_ref(),
                    executor.clone(),
                    variant.clone(),
                    model_id.clone(),
                    reasoning_id.clone(),
                    permission_policy.clone(),
                    repo_id,
                    repo_name.clone(),
                    branch.clone(),
                    dry_run,
                    rerun_failed,
                )
                .await
            {
                Ok(result) => result,
                Err(error) => SyncIssueExecutionSummary {
                    issue_id,
                    simple_id,
                    title,
                    action: "error".to_string(),
                    workspace_id: None,
                    execution_id: None,
                    execution_status: None,
                    issue_status: watched_status.clone(),
                    details: Some(error.to_string()),
                },
            };
            results.push(result);
        }

        McpServer::success(&SyncProjectExecutionsResponse {
            project_id: project_id.to_string(),
            watched_status,
            examined_count,
            result_count: results.len(),
            dry_run,
            results,
        })
    }

    #[tool(
        description = "Link an existing workspace to a remote issue. This associates the workspace with the issue for tracking."
    )]
    async fn link_workspace_issue(
        &self,
        Parameters(LinkWorkspaceIssueRequest {
            workspace_id,
            issue_id,
        }): Parameters<LinkWorkspaceIssueRequest>,
    ) -> Result<CallToolResult, ErrorData> {
        if let Err(e) = self.link_workspace_to_issue(workspace_id, issue_id).await {
            return Ok(Self::tool_error(e));
        }

        McpServer::success(&LinkWorkspaceIssueResponse {
            success: true,
            workspace_id: workspace_id.to_string(),
            issue_id: issue_id.to_string(),
        })
    }
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

impl McpServer {
    #[allow(clippy::too_many_arguments)]
    async fn start_execution_for_issue(
        &self,
        issue: Issue,
        name: Option<String>,
        prompt: Option<String>,
        executor: Option<String>,
        variant: Option<String>,
        model_id: Option<String>,
        reasoning_id: Option<String>,
        permission_policy: Option<String>,
        repo_id: Option<Uuid>,
        repo_name: Option<String>,
        branch: Option<String>,
        force_new_workspace: bool,
    ) -> Result<ExecuteIssueResponse, ToolError> {
        let executor_name = executor
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("CODEX");
        let executor_config = Self::executor_config_from_options(
            executor_name,
            variant,
            model_id,
            reasoning_id,
            permission_policy,
            Some(PermissionPolicy::Auto),
        )?;

        if !force_new_workspace
            && let Some(execution_id) = Self::metadata_execution_id(&issue)
            && let Ok(existing) = self.fetch_execution_process(execution_id).await
            && existing.status == ExecutionProcessStatus::Running
        {
            let execution = Self::serialize_execution_process(&existing)?;
            return Ok(ExecuteIssueResponse {
                action: "already_running".to_string(),
                issue_id: issue.id.to_string(),
                simple_id: issue.simple_id.clone(),
                workspace_id: Self::metadata_workspace_id(&issue).map(|id| id.to_string()),
                execution_id: Some(existing.id.to_string()),
                execution_status: Some(
                    Self::execution_process_status_label(&existing.status).to_string(),
                ),
                executor: executor_config.executor.to_string(),
                repo_id: Self::metadata_repo_id(&issue).map(|id| id.to_string()),
                branch: Self::metadata_branch(&issue),
                already_running: true,
                execution: Some(execution),
            });
        }

        let (repo_id, branch) = self
            .resolve_repo_for_execution(repo_id, repo_name, branch)
            .await?;

        let workspace_prompt = normalize_optional(prompt)
            .or_else(|| build_workspace_prompt_from_issue(&issue))
            .ok_or_else(|| {
                ToolError::message(
                    "Provide `prompt`, or execute an issue that has a non-empty title/description.",
                )
            })?;
        let workspace_name = normalize_optional(name).unwrap_or_else(|| {
            format!("{} {}", issue.simple_id, issue.title)
                .trim()
                .to_string()
        });

        let create_and_start_payload = CreateAndStartWorkspaceRequest {
            name: Some(workspace_name),
            repos: vec![WorkspaceRepoInput {
                repo_id,
                target_branch: branch.clone(),
            }],
            linked_issue: Some(LinkedIssueInfo {
                remote_project_id: issue.project_id,
                issue_id: issue.id,
            }),
            executor_config: executor_config.clone(),
            prompt: workspace_prompt,
            attachment_ids: None,
        };

        let create_and_start_url = self.url("/api/workspaces/start");
        let create_and_start_response: CreateAndStartWorkspaceResponse = self
            .send_json(
                self.client
                    .post(&create_and_start_url)
                    .json(&create_and_start_payload),
            )
            .await?;

        self.link_workspace_to_issue(create_and_start_response.workspace.id, issue.id)
            .await?;

        self.record_issue_execution(
            &issue,
            create_and_start_response.workspace.id,
            create_and_start_response.execution_process.id,
            executor_config.executor.to_string(),
            repo_id,
            branch.clone(),
        )
        .await?;

        let execution =
            Self::serialize_execution_process(&create_and_start_response.execution_process)?;

        Ok(ExecuteIssueResponse {
            action: "started".to_string(),
            issue_id: issue.id.to_string(),
            simple_id: issue.simple_id,
            workspace_id: Some(create_and_start_response.workspace.id.to_string()),
            execution_id: Some(create_and_start_response.execution_process.id.to_string()),
            execution_status: Some(
                Self::execution_process_status_label(
                    &create_and_start_response.execution_process.status,
                )
                .to_string(),
            ),
            executor: executor_config.executor.to_string(),
            repo_id: Some(repo_id.to_string()),
            branch: Some(branch),
            already_running: false,
            execution: Some(execution),
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn sync_issue_execution(
        &self,
        issue: Issue,
        watched_status: &str,
        completed_status: Option<&(String, Uuid)>,
        failed_status: Option<&(String, Uuid)>,
        executor: Option<String>,
        variant: Option<String>,
        model_id: Option<String>,
        reasoning_id: Option<String>,
        permission_policy: Option<String>,
        repo_id: Option<Uuid>,
        repo_name: Option<String>,
        branch: Option<String>,
        dry_run: bool,
        rerun_failed: bool,
    ) -> Result<SyncIssueExecutionSummary, ToolError> {
        if let Some(execution_id) = Self::metadata_execution_id(&issue)
            && let Ok(execution) = self.fetch_execution_process(execution_id).await
        {
            let execution_status = Self::execution_process_status_label(&execution.status);
            return match execution.status {
                ExecutionProcessStatus::Running => {
                    let workspace_id = Self::metadata_workspace_id(&issue).map(|id| id.to_string());
                    Ok(SyncIssueExecutionSummary {
                        issue_id: issue.id.to_string(),
                        simple_id: issue.simple_id,
                        title: issue.title,
                        action: "already_running".to_string(),
                        workspace_id,
                        execution_id: Some(execution.id.to_string()),
                        execution_status: Some(execution_status.to_string()),
                        issue_status: watched_status.to_string(),
                        details: None,
                    })
                }
                ExecutionProcessStatus::Completed => {
                    let updated_issue = if dry_run {
                        issue.clone()
                    } else if let Some((_, status_id)) = completed_status {
                        self.update_issue_execution_state(
                            &issue,
                            Some(*status_id),
                            execution_status,
                        )
                        .await?
                    } else {
                        self.update_issue_execution_state(&issue, None, execution_status)
                            .await?
                    };
                    let workspace_id =
                        Self::metadata_workspace_id(&updated_issue).map(|id| id.to_string());

                    Ok(SyncIssueExecutionSummary {
                        issue_id: updated_issue.id.to_string(),
                        simple_id: updated_issue.simple_id,
                        title: updated_issue.title,
                        action: if completed_status.is_some() {
                            "completed_moved".to_string()
                        } else {
                            "completed_recorded".to_string()
                        },
                        workspace_id,
                        execution_id: Some(execution.id.to_string()),
                        execution_status: Some(execution_status.to_string()),
                        issue_status: completed_status
                            .map(|(status, _)| status.clone())
                            .unwrap_or_else(|| watched_status.to_string()),
                        details: None,
                    })
                }
                ExecutionProcessStatus::Failed | ExecutionProcessStatus::Killed
                    if rerun_failed && !dry_run =>
                {
                    let issue_title = issue.title.clone();
                    let mut response = self
                        .start_execution_for_issue(
                            issue,
                            None,
                            None,
                            executor,
                            variant,
                            model_id,
                            reasoning_id,
                            permission_policy,
                            repo_id,
                            repo_name,
                            branch,
                            true,
                        )
                        .await?;
                    response.action = "reran_failed".to_string();
                    Ok(Self::execution_response_to_sync_summary(
                        response,
                        issue_title,
                        watched_status.to_string(),
                        None,
                    ))
                }
                ExecutionProcessStatus::Failed | ExecutionProcessStatus::Killed => {
                    let updated_issue = if dry_run {
                        issue.clone()
                    } else if let Some((_, status_id)) = failed_status {
                        self.update_issue_execution_state(
                            &issue,
                            Some(*status_id),
                            execution_status,
                        )
                        .await?
                    } else {
                        self.update_issue_execution_state(&issue, None, execution_status)
                            .await?
                    };
                    let workspace_id =
                        Self::metadata_workspace_id(&updated_issue).map(|id| id.to_string());
                    Ok(SyncIssueExecutionSummary {
                        issue_id: updated_issue.id.to_string(),
                        simple_id: updated_issue.simple_id,
                        title: updated_issue.title,
                        action: if failed_status.is_some() {
                            "failed_moved".to_string()
                        } else {
                            "failed_recorded".to_string()
                        },
                        workspace_id,
                        execution_id: Some(execution.id.to_string()),
                        execution_status: Some(execution_status.to_string()),
                        issue_status: failed_status
                            .map(|(status, _)| status.clone())
                            .unwrap_or_else(|| watched_status.to_string()),
                        details: Some("Recorded execution is terminal; set rerun_failed=true to start a fresh workspace.".to_string()),
                    })
                }
            };
        }

        if dry_run {
            let workspace_id = Self::metadata_workspace_id(&issue).map(|id| id.to_string());
            let execution_id = Self::metadata_execution_id(&issue).map(|id| id.to_string());
            return Ok(SyncIssueExecutionSummary {
                issue_id: issue.id.to_string(),
                simple_id: issue.simple_id,
                title: issue.title,
                action: "would_start".to_string(),
                workspace_id,
                execution_id,
                execution_status: None,
                issue_status: watched_status.to_string(),
                details: None,
            });
        }

        let response = self
            .start_execution_for_issue(
                issue.clone(),
                None,
                None,
                executor,
                variant,
                model_id,
                reasoning_id,
                permission_policy,
                repo_id,
                repo_name,
                branch,
                false,
            )
            .await?;
        Ok(Self::execution_response_to_sync_summary(
            response,
            issue.title,
            watched_status.to_string(),
            None,
        ))
    }

    fn execution_response_to_sync_summary(
        response: ExecuteIssueResponse,
        title: String,
        issue_status: String,
        details: Option<String>,
    ) -> SyncIssueExecutionSummary {
        SyncIssueExecutionSummary {
            issue_id: response.issue_id,
            simple_id: response.simple_id,
            title,
            action: response.action,
            workspace_id: response.workspace_id,
            execution_id: response.execution_id,
            execution_status: response.execution_status,
            issue_status,
            details,
        }
    }

    fn executor_config_from_options(
        executor: &str,
        variant: Option<String>,
        model_id: Option<String>,
        reasoning_id: Option<String>,
        permission_policy: Option<String>,
        default_permission_policy: Option<PermissionPolicy>,
    ) -> Result<ExecutorConfig, ToolError> {
        let base_executor = Self::parse_executor_agent(executor)
            .map_err(|_| ToolError::message(format!("Unknown executor '{executor}'.")))?;

        Ok(ExecutorConfig {
            executor: base_executor,
            variant: normalize_optional(variant),
            model_id: normalize_optional(model_id),
            agent_id: None,
            reasoning_id: normalize_optional(reasoning_id),
            permission_policy: Self::parse_permission_policy(
                permission_policy,
                default_permission_policy,
            )?,
        })
    }

    fn parse_permission_policy(
        permission_policy: Option<String>,
        default_permission_policy: Option<PermissionPolicy>,
    ) -> Result<Option<PermissionPolicy>, ToolError> {
        let Some(value) = normalize_optional(permission_policy) else {
            return Ok(default_permission_policy);
        };
        let normalized = value.replace(['-', ' '], "_").to_ascii_uppercase();
        match normalized.as_str() {
            "AUTO" | "NEVER" => Ok(Some(PermissionPolicy::Auto)),
            "SUPERVISED" | "UNLESS_TRUSTED" | "ON_REQUEST" => {
                Ok(Some(PermissionPolicy::Supervised))
            }
            "PLAN" => Ok(Some(PermissionPolicy::Plan)),
            _ => Err(ToolError::message(format!(
                "Unknown permission policy '{value}'. Use auto, supervised, or plan."
            ))),
        }
    }

    async fn resolve_issue_for_execution(
        &self,
        project_id: Option<Uuid>,
        issue_id: Option<Uuid>,
        simple_id: Option<String>,
    ) -> Result<Issue, ToolError> {
        if let Some(issue_id) = issue_id {
            let url = self.url(&format!("/api/remote/issues/{issue_id}"));
            return self.send_json(self.client.get(&url)).await;
        }

        let project_id = self.resolve_project_id(project_id)?;
        let simple_id = normalize_optional(simple_id)
            .ok_or_else(|| ToolError::message("Provide issue_id or simple_id."))?;
        let url = self.url(&format!("/api/remote/issues?project_id={project_id}"));
        let response: ListIssuesResponse = self.send_json(self.client.get(&url)).await?;
        response
            .issues
            .into_iter()
            .find(|issue| issue.simple_id.eq_ignore_ascii_case(&simple_id))
            .ok_or_else(|| {
                ToolError::message(format!(
                    "Issue '{simple_id}' was not found in project {project_id}."
                ))
            })
    }

    async fn resolve_repo_for_execution(
        &self,
        repo_id: Option<Uuid>,
        repo_name: Option<String>,
        branch: Option<String>,
    ) -> Result<(Uuid, String), ToolError> {
        let url = self.url("/api/repos");
        let repos: Vec<Repo> = self.send_json(self.client.get(&url)).await?;

        let repo = if let Some(repo_id) = repo_id {
            repos
                .into_iter()
                .find(|repo| repo.id == repo_id)
                .ok_or_else(|| {
                    ToolError::message(format!("Repository '{repo_id}' was not found."))
                })?
        } else if let Some(repo_name) = normalize_optional(repo_name) {
            let needle = repo_name.to_ascii_lowercase();
            repos
                .into_iter()
                .find(|repo| {
                    repo.name.eq_ignore_ascii_case(&needle)
                        || repo.display_name.eq_ignore_ascii_case(&needle)
                })
                .ok_or_else(|| {
                    ToolError::message(format!("Repository '{repo_name}' was not found."))
                })?
        } else if repos.len() == 1 {
            repos.into_iter().next().expect("repo count checked")
        } else {
            let names = repos
                .iter()
                .map(|repo| repo.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(ToolError::message(format!(
                "Repository is ambiguous. Provide repo_id or repo_name. Available repositories: {names}"
            )));
        };

        let branch = normalize_optional(branch)
            .or_else(|| repo.default_target_branch.clone())
            .unwrap_or_else(|| "main".to_string());

        Ok((repo.id, branch))
    }

    async fn fetch_execution_process(
        &self,
        execution_id: Uuid,
    ) -> Result<ExecutionProcess, ToolError> {
        let url = self.url(&format!("/api/execution-processes/{execution_id}"));
        self.send_json(self.client.get(&url)).await
    }

    async fn record_issue_execution(
        &self,
        issue: &Issue,
        workspace_id: Uuid,
        execution_id: Uuid,
        executor: String,
        repo_id: Uuid,
        branch: String,
    ) -> Result<Issue, ToolError> {
        let mut metadata = if issue.extension_metadata.is_object() {
            issue.extension_metadata.clone()
        } else {
            json!({})
        };
        metadata["openclaw"] = json!({
            "workspace_id": workspace_id,
            "execution_id": execution_id,
            "executor": executor,
            "repo_id": repo_id,
            "branch": branch,
            "status": "running"
        });

        self.patch_issue(issue.id, None, Some(metadata)).await
    }

    async fn update_issue_execution_state(
        &self,
        issue: &Issue,
        status_id: Option<Uuid>,
        execution_status: &str,
    ) -> Result<Issue, ToolError> {
        let mut metadata = if issue.extension_metadata.is_object() {
            issue.extension_metadata.clone()
        } else {
            json!({})
        };
        let mut openclaw_metadata = metadata
            .get("openclaw")
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or_else(|| json!({}));
        openclaw_metadata["status"] = json!(execution_status);
        metadata["openclaw"] = openclaw_metadata;

        self.patch_issue(issue.id, status_id, Some(metadata)).await
    }

    async fn patch_issue(
        &self,
        issue_id: Uuid,
        status_id: Option<Uuid>,
        extension_metadata: Option<Value>,
    ) -> Result<Issue, ToolError> {
        let payload = UpdateIssueRequest {
            status_id,
            title: None,
            description: None,
            priority: None,
            start_date: None,
            target_date: None,
            completed_at: None,
            sort_order: None,
            parent_issue_id: None,
            parent_issue_sort_order: None,
            extension_metadata,
        };

        let url = self.url(&format!("/api/remote/issues/{issue_id}"));
        let response: MutationResponse<Issue> = self
            .send_json(self.client.patch(&url).json(&payload))
            .await?;
        Ok(response.data)
    }

    fn metadata_execution_id(issue: &Issue) -> Option<Uuid> {
        issue
            .extension_metadata
            .pointer("/openclaw/execution_id")
            .and_then(Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
    }

    fn metadata_workspace_id(issue: &Issue) -> Option<Uuid> {
        issue
            .extension_metadata
            .pointer("/openclaw/workspace_id")
            .and_then(Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
    }

    fn metadata_repo_id(issue: &Issue) -> Option<Uuid> {
        issue
            .extension_metadata
            .pointer("/openclaw/repo_id")
            .and_then(Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
    }

    fn metadata_branch(issue: &Issue) -> Option<String> {
        issue
            .extension_metadata
            .pointer("/openclaw/branch")
            .and_then(Value::as_str)
            .map(ToString::to_string)
    }
}
