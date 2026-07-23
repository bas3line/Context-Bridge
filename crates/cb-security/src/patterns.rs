pub const DEFAULT_EXCLUDED_PATHS: &[&str] = &[
    ".env",
    ".env.*",
    "**/.env",
    "**/.env.*",
    "**/secrets/**",
    "**/.ssh/**",
    "**/.aws/credentials",
    "**/.config/gcloud/**",
    "**/*id_rsa*",
    "**/*id_ed25519*",
    "**/*.pem",
    "**/*.key",
    "**/keychain/**",
    "**/browser-data/**",
    "**/.context-bridge/**",
    "**/.context-bridge-data/**",
    // Generated dependencies and agent-owned state are neither useful source
    // context nor safe/cheap to snapshot before an interactive launch.
    "node_modules",
    "**/node_modules",
    "**/node_modules/**",
    ".opencode",
    "**/.opencode",
    "**/.opencode/**",
    ".claude",
    "**/.claude",
    "**/.claude/**",
    ".codex",
    "**/.codex",
    "**/.codex/**",
    ".commandcode",
    "**/.commandcode",
    "**/.commandcode/**",
];

pub const SECRET_MARKERS: &[&str] = &[
    "-----BEGIN PRIVATE KEY-----",
    "-----BEGIN OPENSSH PRIVATE KEY-----",
    "AKIA",
    "ghp_",
    "github_pat_",
    "sk-ant-",
    "sk-proj-",
    "xoxb-",
    "xoxp-",
];

pub const ASSIGNMENT_MARKERS: &[&str] = &[
    "password=",
    "passwd=",
    "api_key=",
    "apikey=",
    "secret=",
    "token=",
    "authorization: bearer ",
];

/// Text that is not a conclusive credential match, but is unsafe to include in
/// a target-agent prompt under strict redaction.  Keep these separate from
/// [`ASSIGNMENT_MARKERS`] so callers can preserve the `PotentialSecret`
/// sensitivity classification while still redacting fail-closed.
pub const POTENTIAL_SECRET_MARKERS: &[&str] = &["credential", "private key", "access token"];
