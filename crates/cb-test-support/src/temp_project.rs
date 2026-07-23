use std::{path::Path, process::Command};

pub fn init_git_project(path: &Path) -> std::io::Result<()> {
    let status = Command::new("git").arg("init").arg(path).status()?;
    if !status.success() {
        return Err(std::io::Error::other("git init failed"));
    }
    Ok(())
}
