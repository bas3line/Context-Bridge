mod claude;
mod codex;
mod opencode;

use std::fmt::Write;

use cb_core::{HandoffPackage, MessageRole, TestOutcome};

pub use claude::render_claude;
pub use codex::render_codex;
pub use opencode::render_opencode;

pub(crate) fn render_common(package: &HandoffPackage, previous_agent: &str) -> String {
    let mut output = format!(
        "You are continuing an existing coding task previously worked on in {previous_agent}.\n"
    );
    section(
        &mut output,
        "Original objective",
        package
            .original_objective
            .as_deref()
            .unwrap_or("Not recorded."),
    );
    section(
        &mut output,
        "Current objective",
        package
            .current_objective
            .as_deref()
            .unwrap_or("Not recorded."),
    );
    let _ = write!(
        output,
        "\n## Repository state\n- Root: {}\n- Branch: {}\n- HEAD: {}\n- Working tree:\n```\n{}\n```\n",
        package.project.root.display(),
        package.git.branch.as_deref().unwrap_or("none"),
        package.git.head.as_deref().unwrap_or("no commit"),
        empty_as(&package.git.status, "clean or not a Git repository")
    );
    list_section(
        &mut output,
        "Work completed",
        package
            .completed_work
            .iter()
            .map(|item| item.summary.as_str()),
    );
    list_section(
        &mut output,
        "Current implementation state",
        package
            .current_state
            .iter()
            .map(|item| item.summary.as_str()),
    );
    let decisions: Vec<_> = package
        .decisions
        .iter()
        .map(|item| {
            item.rationale.as_ref().map_or_else(
                || item.decision.clone(),
                |rationale| format!("{} — {}", item.decision, rationale),
            )
        })
        .collect();
    list_section(
        &mut output,
        "Important decisions",
        decisions.iter().map(String::as_str),
    );
    list_section(
        &mut output,
        "Assumptions",
        package
            .assumptions
            .iter()
            .map(|item| item.assumption.as_str()),
    );
    list_section(
        &mut output,
        "Failed approaches",
        package
            .failed_approaches
            .iter()
            .map(|item| item.summary.as_str()),
    );
    let files: Vec<_> = package
        .modified_files
        .iter()
        .map(|item| format!("{}: {}", item.path.display(), item.change))
        .collect();
    list_section(
        &mut output,
        "Files changed",
        files.iter().map(String::as_str),
    );
    let relevant_files: Vec<_> = package
        .relevant_files
        .iter()
        .map(|item| format!("{}: {}", item.path.display(), item.reason))
        .collect();
    list_section(
        &mut output,
        "Relevant files",
        relevant_files.iter().map(String::as_str),
    );
    let commands: Vec<_> = package
        .commands
        .iter()
        .map(|item| {
            let exit = item
                .exit_code
                .map_or_else(|| "unknown exit".to_owned(), |code| format!("exit {code}"));
            item.output_summary.as_ref().map_or_else(
                || format!("`{}` ({exit})", item.command),
                |summary| format!("`{}` ({exit}) — {summary}", item.command),
            )
        })
        .collect();
    list_section(
        &mut output,
        "Commands executed",
        commands.iter().map(String::as_str),
    );
    let tests: Vec<_> = package
        .tests
        .iter()
        .map(|item| {
            let outcome = match item.outcome {
                TestOutcome::Passed => "passed",
                TestOutcome::Failed => "failed",
                TestOutcome::Skipped => "skipped",
                TestOutcome::Interrupted => "interrupted",
            };
            format!("`{}`: {outcome} — {}", item.command, item.summary)
        })
        .collect();
    list_section(
        &mut output,
        "Tests and validation",
        tests.iter().map(String::as_str),
    );
    let errors: Vec<_> = package
        .errors
        .iter()
        .map(|item| {
            format!(
                "{} ({})",
                item.message,
                if item.resolved {
                    "resolved"
                } else {
                    "unresolved"
                }
            )
        })
        .collect();
    list_section(
        &mut output,
        "Known problems",
        errors.iter().map(String::as_str),
    );
    list_section(
        &mut output,
        "Remaining work",
        package.pending_tasks.iter().map(|item| item.task.as_str()),
    );
    output.push_str("\n## Recent conversation\n");
    if package.recent_conversation.is_empty() {
        output.push_str("No conversation was available.\n");
    } else {
        for message in &package.recent_conversation {
            let role = match message.role {
                MessageRole::User => "User",
                MessageRole::Assistant => "Assistant",
                MessageRole::System => "System",
            };
            let _ = writeln!(output, "\n**{role}:** {}", message.content);
        }
    }
    if let Some(next) = &package.recommended_next_action {
        section(&mut output, "Recommended next action", next);
    }
    output.push_str(
        "\n## Required behavior\n\
         1. Inspect the current repository before editing.\n\
         2. Treat the repository as the source of truth if this handoff differs from files.\n\
         3. Do not repeat completed work.\n\
         4. Continue from the recommended next action.\n\
         5. Ask only when genuinely blocked.\n\
         \nThis is a reconstructed context handoff, not hidden model state or private reasoning.\n",
    );
    output
}

fn section(output: &mut String, title: &str, body: &str) {
    let _ = write!(output, "\n## {title}\n{body}\n");
}

fn list_section<'a>(output: &mut String, title: &str, items: impl Iterator<Item = &'a str>) {
    let values: Vec<_> = items.collect();
    let _ = write!(output, "\n## {title}\n");
    if values.is_empty() {
        output.push_str("- None recorded.\n");
    } else {
        for value in values {
            let _ = writeln!(output, "- {value}");
        }
    }
}

fn empty_as<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.trim().is_empty() {
        fallback
    } else {
        value
    }
}
