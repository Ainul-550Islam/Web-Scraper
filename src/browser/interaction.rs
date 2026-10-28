//! Workflow execution facade; every command goes through a typed, bounded action.
use super::actions;
use super::config::BrowserConfig;
use super::credentials::{resolve_secret, CredentialResolver};
use super::error::{BrowserError, BrowserResult};
use super::javascript;
use super::navigation::{self, WaitUntil};
use super::session::BrowserSession;
use super::wait;
use super::workflow::{BranchCondition, InputValue, ScrollDirection, Workflow, WorkflowStep};
use serde_json::json;

#[derive(Clone, Debug, Default)]
pub struct WorkflowReport {
    pub completed_steps: usize,
    pub checkpoints: Vec<String>,
    pub last_url: Option<String>,
}

pub async fn run_workflow(
    session: &BrowserSession,
    cfg: &BrowserConfig,
    workflow: &Workflow,
    resolver: &dyn CredentialResolver,
) -> BrowserResult<WorkflowReport> {
    run_workflow_scoped(session, cfg, workflow, resolver, &cfg.authorized_hosts).await
}

pub async fn run_workflow_scoped(
    session: &BrowserSession,
    cfg: &BrowserConfig,
    workflow: &Workflow,
    resolver: &dyn CredentialResolver,
    allowed_hosts: &[String],
) -> BrowserResult<WorkflowReport> {
    workflow
        .validate()
        .map_err(|e| BrowserError::Config(e.to_string()))?;
    let mut report = WorkflowReport::default();
    execute_steps(
        session,
        cfg,
        workflow.steps(),
        resolver,
        &mut report,
        0,
        allowed_hosts,
    )
    .await?;
    Ok(report)
}

async fn execute_steps(
    session: &BrowserSession,
    cfg: &BrowserConfig,
    steps: &[WorkflowStep],
    resolver: &dyn CredentialResolver,
    report: &mut WorkflowReport,
    depth: usize,
    allowed_hosts: &[String],
) -> BrowserResult<()> {
    if depth > 8 {
        return Err(BrowserError::InvalidInput("workflow branch depth"));
    }
    for step in steps {
        match step {
            WorkflowStep::Navigate { url } => {
                enforce_url_scope(url, allowed_hosts)?;
                let result =
                    navigation::navigate(session, url, WaitUntil::Load, cfg.navigation_timeout_ms)
                        .await?;
                report.last_url = Some(result.final_url);
            }
            WorkflowStep::Click { selector } => actions::click(session, selector).await?,
            WorkflowStep::Input {
                selector,
                value,
                clear_first,
            } => {
                let secret = match value {
                    InputValue::Literal { value } => value.clone(),
                    InputValue::SecretRef { reference } => resolve_secret(resolver, reference)?,
                };
                actions::input(session, selector, &secret, *clear_first).await?;
            }
            WorkflowStep::Keypress { selector, key } => {
                actions::keypress(session, selector.as_deref(), key).await?
            }
            WorkflowStep::Wait {
                condition,
                timeout_ms,
            } => wait::wait_for(session, condition, *timeout_ms).await?,
            WorkflowStep::Scroll { direction, pixels } => {
                actions::scroll(
                    session,
                    match direction {
                        ScrollDirection::Up => "up",
                        ScrollDirection::Down => "down",
                    },
                    *pixels,
                )
                .await?
            }
            WorkflowStep::Evaluate { script } => {
                javascript::evaluate(session, cfg, script, Vec::new()).await?;
            }
            WorkflowStep::Checkpoint { name, selector } => {
                if let Some(css) = selector {
                    let present = session
                        .execute(
                            "return document.querySelector(arguments[0])!==null;",
                            vec![json!(css)],
                            128,
                        )
                        .await?;
                    if present != serde_json::Value::Bool(true) {
                        return Err(BrowserError::NotFound("workflow checkpoint element"));
                    }
                }
                report.checkpoints.push(name.clone());
            }
            WorkflowStep::Branch {
                condition,
                then_steps,
                else_steps,
            } => {
                let branch = branch_condition(session, condition).await?;
                Box::pin(execute_steps(
                    session,
                    cfg,
                    if branch { then_steps } else { else_steps },
                    resolver,
                    report,
                    depth + 1,
                    allowed_hosts,
                ))
                .await?;
            }
            WorkflowStep::Fail { message } => {
                return Err(BrowserError::Policy(format!("workflow stopped: {message}")))
            }
        }
        report.completed_steps += 1;
    }
    Ok(())
}

async fn branch_condition(
    session: &BrowserSession,
    condition: &BranchCondition,
) -> BrowserResult<bool> {
    match condition {
        BranchCondition::SelectorPresent { selector } => session
            .execute(
                "return document.querySelector(arguments[0])!==null;",
                vec![json!(selector)],
                128,
            )
            .await
            .map(|v| v == serde_json::Value::Bool(true)),
        BranchCondition::UrlContains { value } => {
            session.current_url().await.map(|url| url.contains(value))
        }
        BranchCondition::TextPresent { text } => session
            .execute(
                "return (document.body?.innerText||'').includes(arguments[0]);",
                vec![json!(text)],
                128,
            )
            .await
            .map(|v| v == serde_json::Value::Bool(true)),
    }
}

fn enforce_url_scope(url: &str, allowed: &[String]) -> BrowserResult<()> {
    let parsed = url::Url::parse(url).map_err(|_| BrowserError::InvalidInput("workflow URL"))?;
    let host = parsed
        .host_str()
        .ok_or(BrowserError::InvalidInput("workflow URL host"))?;
    if allowed.is_empty() || !allowed.iter().any(|h| host.eq_ignore_ascii_case(h)) {
        return Err(BrowserError::Policy(
            "workflow navigation is outside authorized_hosts".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn navigation_scope_rejects_external_host() {
        assert!(enforce_url_scope("https://outside.test", &["example.test".into()]).is_err());
        assert!(enforce_url_scope("https://example.test/a", &["example.test".into()]).is_ok());
    }
}
