use std::io::IsTerminal;

#[must_use]
pub fn has_interactive_terminal() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}
