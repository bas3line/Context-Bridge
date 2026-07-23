use miette::IntoDiagnostic;
use serde::Serialize;

pub fn print_json(value: &impl Serialize) -> miette::Result<()> {
    println!("{}", serde_json::to_string_pretty(value).into_diagnostic()?);
    Ok(())
}
