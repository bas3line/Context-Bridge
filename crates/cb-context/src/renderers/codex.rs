use cb_core::HandoffPackage;

use super::render_common;

#[must_use]
pub fn render_codex(package: &HandoffPackage) -> String {
    render_common(package, &package.source_agent.to_string())
}
