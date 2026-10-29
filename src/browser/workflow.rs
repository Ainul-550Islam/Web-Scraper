//! Serializable browser action workflow. Values that contain credentials must
//! use a `secret_ref`; workflows never need to embed a real password.
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputValue {
    Literal { value: String },
    SecretRef { reference: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkflowStep {
    Navigate {
        url: String,
    },
    Click {
        selector: String,
    },
    Input {
        selector: String,
        value: InputValue,
        clear_first: bool,
    },
    Keypress {
        selector: Option<String>,
        key: String,
    },
    Wait {
        condition: WaitCondition,
        timeout_ms: u64,
    },
    Scroll {
        direction: ScrollDirection,
        pixels: u32,
    },
    Evaluate {
        script: String,
    },
    Checkpoint {
        name: String,
        selector: Option<String>,
    },
    Branch {
        condition: BranchCondition,
        then_steps: Vec<WorkflowStep>,
        else_steps: Vec<WorkflowStep>,
    },
    Fail {
        message: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "condition", rename_all = "snake_case", deny_unknown_fields)]
pub enum WaitCondition {
    SelectorAttached { selector: String },
    SelectorVisible { selector: String },
    TextPresent { text: String },
    UrlContains { value: String },
    PageComplete,
    Delay { millis: u64 },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScrollDirection {
    Up,
    Down,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "condition", rename_all = "snake_case", deny_unknown_fields)]
pub enum BranchCondition {
    SelectorPresent { selector: String },
    UrlContains { value: String },
    TextPresent { text: String },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct Workflow(pub Vec<WorkflowStep>);

impl Workflow {
    pub fn validate(&self) -> Result<()> {
        if self.0.len() > 256 {
            bail!("browser workflow supports at most 256 top-level steps");
        }
        let mut budget = 256usize;
        for step in &self.0 {
            validate_step(step, 0, &mut budget)?;
        }
        Ok(())
    }

    pub fn steps(&self) -> &[WorkflowStep] {
        &self.0
    }

    /// Durable workflows must refer to credentials by secret handle; literal
    /// input remains available only to explicitly ephemeral preview calls.
    pub fn validate_persistable(&self) -> Result<()> {
        self.validate()?;
        fn check(steps: &[WorkflowStep]) -> Result<()> {
            for step in steps {
                match step {
                    WorkflowStep::Input {
                        value: InputValue::Literal { .. },
                        ..
                    } => {
                        bail!("persisted workflow inputs must use secret_ref, not literal values");
                    }
                    WorkflowStep::Branch {
                        then_steps,
                        else_steps,
                        ..
                    } => {
                        check(then_steps)?;
                        check(else_steps)?;
                    }
                    _ => {}
                }
            }
            Ok(())
        }
        check(&self.0)
    }
}

fn validate_step(step: &WorkflowStep, depth: usize, budget: &mut usize) -> Result<()> {
    if depth > 8 {
        bail!("browser workflow branch nesting exceeds 8 levels");
    }
    *budget = budget.saturating_sub(1);
    if *budget == 0 {
        bail!("browser workflow exceeds 256 total steps");
    }
    let check = |v: &str, name: &str| -> Result<()> {
        if v.is_empty() || v.len() > 8192 || v.chars().any(char::is_control) {
            bail!("browser workflow {name} is empty, too long, or contains control characters");
        }
        Ok(())
    };
    match step {
        WorkflowStep::Navigate { url } => {
            check(url, "URL")?;
            let u = url::Url::parse(url)
                .map_err(|_| anyhow::anyhow!("workflow navigation URL is invalid"))?;
            if !matches!(u.scheme(), "http" | "https")
                || u.host_str().is_none()
                || !u.username().is_empty()
                || u.password().is_some()
            {
                bail!("workflow navigation URL must be http(s) without credentials");
            }
        }
        WorkflowStep::Click { selector } => check(selector, "selector")?,
        WorkflowStep::Input {
            selector, value, ..
        } => {
            check(selector, "selector")?;
            match value {
                InputValue::Literal { value } => check(value, "input")?,
                InputValue::SecretRef { reference } => check(reference, "secret reference")?,
            }
        }
        WorkflowStep::Keypress { selector, key } => {
            if let Some(s) = selector {
                check(s, "selector")?;
            }
            check(key, "key")?;
            if key.len() > 32 {
                bail!("workflow key name exceeds 32 bytes");
            }
        }
        WorkflowStep::Wait {
            condition,
            timeout_ms,
        } => {
            if !(100..=300_000).contains(timeout_ms) {
                bail!("workflow wait timeout must be 100..=300000 ms");
            }
            match condition {
                WaitCondition::SelectorAttached { selector }
                | WaitCondition::SelectorVisible { selector } => check(selector, "selector")?,
                WaitCondition::TextPresent { text } => check(text, "text")?,
                WaitCondition::UrlContains { value } => check(value, "URL match")?,
                WaitCondition::PageComplete => {}
                WaitCondition::Delay { millis } if *millis <= 30_000 => {}
                WaitCondition::Delay { .. } => bail!("workflow delay exceeds 30000 ms"),
            }
        }
        WorkflowStep::Scroll { pixels, .. } => {
            if !(1..=10_000).contains(pixels) {
                bail!("workflow scroll pixels must be 1..=10000");
            }
        }
        WorkflowStep::Evaluate { script } => {
            check(script, "script")?;
            if script.len() > 16_384 {
                bail!("workflow JavaScript exceeds 16 KiB");
            }
        }
        WorkflowStep::Checkpoint { name, selector } => {
            check(name, "checkpoint name")?;
            if let Some(s) = selector {
                check(s, "selector")?;
            }
        }
        WorkflowStep::Branch {
            condition,
            then_steps,
            else_steps,
        } => {
            match condition {
                BranchCondition::SelectorPresent { selector } => check(selector, "selector")?,
                BranchCondition::UrlContains { value } => check(value, "URL match")?,
                BranchCondition::TextPresent { text } => check(text, "text")?,
            };
            if then_steps.len() + else_steps.len() > 64 {
                bail!("workflow branch has more than 64 child steps");
            }
            for child in then_steps.iter().chain(else_steps) {
                validate_step(child, depth + 1, budget)?;
            }
        }
        WorkflowStep::Fail { message } => check(message, "failure message")?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workflow_rejects_javascript_over_limit() {
        let workflow = Workflow(vec![WorkflowStep::Evaluate {
            script: "x".repeat(16_385),
        }]);
        assert!(workflow.validate().is_err());
    }
    #[test]
    fn workflow_accepts_safe_nested_branch() {
        let workflow = Workflow(vec![WorkflowStep::Branch {
            condition: BranchCondition::UrlContains {
                value: "/login".into(),
            },
            then_steps: vec![WorkflowStep::Wait {
                condition: WaitCondition::PageComplete,
                timeout_ms: 1000,
            }],
            else_steps: vec![],
        }]);
        workflow.validate().unwrap();
    }
}
