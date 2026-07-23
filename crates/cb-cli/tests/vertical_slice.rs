use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use cb_test_support::FakeAgentExecutable;
use serde_json::Value;
use tempfile::TempDir;

#[test]
fn fake_claude_to_opencode_to_codex_vertical_slice() {
    let fixture = TestFixture::new();
    let claude_events = fixture_path("claude", "v1.0");
    let opencode_events = fixture_path("opencode", "v1.0");
    let codex_events = fixture_path("codex", "v1.0");

    let run_output = fixture.cb(
        &["run", "claude"],
        &[
            ("CB_FAKE_SOURCE_EVENTS", claude_events.as_os_str()),
            (
                "CB_FAKE_MODIFY_FILE",
                fixture.project.join("tracked.txt").as_os_str(),
            ),
            ("CB_FAKE_MODIFY_CONTENT", "changed by fake Claude".as_ref()),
            ("CB_FAKE_EXTERNAL_SESSION_ID", "claude-session-1".as_ref()),
        ],
    );
    assert_success(&run_output);
    let run_json: Value = serde_json::from_slice(&run_output.stdout).expect("run emits JSON");
    let session_id = run_json["bridge_session_id"]
        .as_str()
        .expect("bridge session id")
        .to_owned();

    let opencode_handoff = fixture.root.path().join("opencode-handoff.md");
    let opencode_output = fixture.cb(
        &["continue", "--last", "--from", "claude", "--to", "opencode"],
        &[
            ("CB_FAKE_SOURCE_EVENTS", opencode_events.as_os_str()),
            ("CB_FAKE_CAPTURE_BOOTSTRAP", opencode_handoff.as_os_str()),
            ("CB_FAKE_EXTERNAL_SESSION_ID", "opencode-session-1".as_ref()),
        ],
    );
    assert_success(&opencode_output);
    let first_handoff = fs::read_to_string(&opencode_handoff).expect("OpenCode handoff captured");
    for expected in [
        "Build authentication with refresh-token rotation.",
        "Store refresh-token hashes, not plaintext tokens.",
        "tracked.txt: modified",
        "12 authentication tests passed.",
        "Add refresh-token rotation and replay detection.",
        "I implemented the authentication request path",
    ] {
        assert!(
            first_handoff.contains(expected),
            "handoff missing `{expected}`:\n{first_handoff}"
        );
    }
    assert!(!first_handoff.contains("must-never-enter-a-handoff"));
    assert!(first_handoff.contains("reconstructed context handoff"));

    let codex_handoff = fixture.root.path().join("codex-handoff.md");
    let codex_output = fixture.cb(
        &["continue", "--last", "--to", "codex"],
        &[
            ("CB_FAKE_SOURCE_EVENTS", codex_events.as_os_str()),
            ("CB_FAKE_CAPTURE_BOOTSTRAP", codex_handoff.as_os_str()),
            ("CB_FAKE_EXTERNAL_SESSION_ID", "codex-session-1".as_ref()),
        ],
    );
    assert_success(&codex_output);
    let second_handoff = fs::read_to_string(&codex_handoff).expect("Codex handoff captured");
    assert!(second_handoff.contains("Build authentication with refresh-token rotation."));
    assert!(second_handoff.contains("Implemented one-time refresh-token rotation."));
    assert!(second_handoff.contains("Invalidate the refresh-token family on logout."));
    assert!(!second_handoff.contains("must-never-enter-a-handoff"));

    let timeline_output = fixture.cb(&["timeline", &session_id], &[]);
    assert_success(&timeline_output);
    assert!(
        !String::from_utf8_lossy(&timeline_output.stdout).contains("must-never-enter-a-handoff"),
        "default timeline must omit secret-classified events"
    );
    let timeline: Vec<Value> =
        serde_json::from_slice(&timeline_output.stdout).expect("timeline emits JSON");
    let recorded_snapshot = timeline.iter().find_map(|event| {
        (event["kind"] == "git_state")
            .then(|| &event["payload"]["data"])
            .filter(|payload| payload["filesystem_fingerprint"].is_string())
    });
    let recorded_snapshot = recorded_snapshot.expect("initial filesystem snapshot is persisted");
    assert!(
        recorded_snapshot["filesystem_file_count"]
            .as_u64()
            .is_some_and(|count| count >= 1),
        "initial snapshot should include tracked files: {recorded_snapshot}"
    );
    assert_eq!(
        recorded_snapshot["filesystem_fingerprint"]
            .as_str()
            .map(str::len),
        Some(64),
        "filesystem snapshot fingerprint must be a BLAKE3 digest"
    );
    for external_id in ["claude-1", "opencode-1"] {
        assert_eq!(
            timeline
                .iter()
                .filter(|event| event["external_event_id"] == external_id)
                .count(),
            1,
            "refresh must not duplicate `{external_id}`"
        );
    }

    let show_output = fixture.cb(&["show", &session_id], &[]);
    assert_success(&show_output);
    let show: Value = serde_json::from_slice(&show_output.stdout).expect("show emits JSON");
    let links = show["external_links"].as_array().expect("links array");
    assert_eq!(links.len(), 3);
    assert!(links.iter().any(|link| link["agent"] == "claude"));
    assert!(links.iter().any(|link| link["agent"] == "opencode"));
    assert!(links.iter().any(|link| link["agent"] == "codex"));

    let other_project = fixture.root.path().join("other-project");
    fs::create_dir_all(&other_project).expect("second project directory");
    let cross_project_import = fixture.cb_for_project(
        &other_project,
        &["import", "claude", "--session", "claude-session-1"],
        &[],
    );
    assert!(
        !cross_project_import.status.success(),
        "an external session linked to one project must not be imported into another"
    );
    let cross_project_error = String::from_utf8_lossy(&cross_project_import.stderr);
    assert!(
        cross_project_error.contains("already linked")
            && cross_project_error.contains("project contexts"),
        "cross-project error should explain containment: {cross_project_error}"
    );
    let other_sessions = fixture.cb_for_project(&other_project, &["sessions"], &[]);
    assert_success(&other_sessions);
    let other_sessions: Vec<Value> =
        serde_json::from_slice(&other_sessions.stdout).expect("other project sessions emit JSON");
    assert!(other_sessions.is_empty());
    for command in [
        vec!["show", session_id.as_str()],
        vec!["timeline", session_id.as_str()],
        vec!["diff", session_id.as_str()],
        vec!["export", session_id.as_str()],
    ] {
        let output = fixture.cb_for_project(&other_project, &command, &[]);
        assert!(
            !output.status.success(),
            "{command:?} must not expose a session owned by another project"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("belongs to"),
            "{command:?} should explain cross-project containment: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let manual = fixture.cb(
        &["continue", "--session", &session_id, "--to", "claude"],
        &[("CB_TEST_MODE", "false".as_ref())],
    );
    assert_success(&manual);
    let manual: Value = serde_json::from_slice(&manual.stdout).expect("manual fallback JSON");
    assert_eq!(manual["launched"], false);
    let manual_path = manual["manual_handoff_path"]
        .as_str()
        .expect("manual handoff path");
    assert!(Path::new(manual_path).is_file());
    assert!(
        fs::read_to_string(manual_path)
            .expect("manual handoff")
            .contains("Build authentication with refresh-token rotation.")
    );

    let sessions_before = fixture.sessions();
    let bad_directory = fixture.fake_sessions.join("claude");
    fs::create_dir_all(&bad_directory).expect("fake import directory");
    fs::write(bad_directory.join("bad.jsonl"), "not-json\n").expect("malformed fixture");
    let failed_import = fixture.cb(&["import", "claude", "--session", "bad"], &[]);
    assert!(!failed_import.status.success());
    assert_eq!(fixture.sessions().len(), sessions_before.len());

    let crash_output = fixture.cb(
        &["run", "claude"],
        &[
            ("CB_FAKE_SOURCE_EVENTS", claude_events.as_os_str()),
            ("CB_FAKE_EXTERNAL_SESSION_ID", "claude-crash".as_ref()),
            ("CB_FAKE_EXIT_CODE", "19".as_ref()),
        ],
    );
    assert_eq!(crash_output.status.code(), Some(19));
    let doctor = fixture.cb(&["doctor"], &[]);
    assert_success(&doctor);
    let doctor: Value = serde_json::from_slice(&doctor.stdout).expect("doctor emits JSON");
    assert_eq!(doctor["database_health"], "healthy");

    #[cfg(unix)]
    {
        let ready = fixture.root.path().join("claude-interrupted.ready");
        let mut interrupted = fixture.cb_command(
            &["run", "claude"],
            &[
                ("CB_FAKE_SOURCE_EVENTS", claude_events.as_os_str()),
                ("CB_FAKE_EXTERNAL_SESSION_ID", "claude-interrupted".as_ref()),
                ("CB_FAKE_READY_PATH", ready.as_os_str()),
                ("CB_FAKE_SLEEP_SECONDS", "30".as_ref()),
            ],
        );
        interrupted
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let child = interrupted.spawn().expect("spawn interruptible cb");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !ready.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            ready.exists(),
            "fake agent became ready before interruption"
        );
        let status = Command::new("kill")
            .arg("-INT")
            .arg(child.id().to_string())
            .status()
            .expect("send interrupt");
        assert!(status.success());
        let output = child.wait_with_output().expect("interrupted cb exits");
        assert_eq!(output.status.code(), Some(130));
        let doctor = fixture.cb(&["doctor"], &[]);
        assert_success(&doctor);
        let doctor: Value = serde_json::from_slice(&doctor.stdout).expect("doctor emits JSON");
        assert_eq!(doctor["database_health"], "healthy");
    }
}

#[test]
fn post_exit_capture_failures_still_reconcile_and_finalize() {
    let fixture = TestFixture::new();
    let malformed_events = fixture.root.path().join("malformed-events.jsonl");
    fs::write(&malformed_events, "this is not JSON\n").expect("write malformed event sink");
    let tracked = fixture.project.join("tracked.txt");

    let run = fixture.cb(
        &["run", "claude"],
        &[
            ("CB_FAKE_SOURCE_EVENTS", malformed_events.as_os_str()),
            ("CB_FAKE_MODIFY_FILE", tracked.as_os_str()),
            (
                "CB_FAKE_MODIFY_CONTENT",
                "changed despite malformed capture".as_ref(),
            ),
            ("CB_FAKE_MALFORMED_SESSION_METADATA", "not-json".as_ref()),
            ("CB_FAKE_EXIT_CODE", "23".as_ref()),
        ],
    );
    assert_eq!(
        run.status.code(),
        Some(23),
        "child exit code must be preserved"
    );
    let run_json: Value = serde_json::from_slice(&run.stdout).expect("run emits persisted outcome");
    let session_id = run_json["bridge_session_id"]
        .as_str()
        .expect("bridge session id")
        .to_owned();
    assert_eq!(
        run_json["post_exit_capture_failures"]
            .as_array()
            .map(Vec::len),
        Some(2),
        "both malformed post-exit artifacts must be reported"
    );
    assert_eq!(
        fs::read_to_string(&tracked).expect("tracked file"),
        "changed despite malformed capture\n"
    );

    let after_run = fixture.cb(&["timeline", &session_id], &[]);
    assert_success(&after_run);
    let after_run: Vec<Value> =
        serde_json::from_slice(&after_run.stdout).expect("timeline emits JSON");
    assert!(after_run.iter().any(|event| {
        event["kind"] == "file_modified" && event["payload"]["data"]["path"] == "tracked.txt"
    }));
    for stage in ["event_sink", "session_metadata"] {
        assert!(after_run.iter().any(|event| {
            event["kind"] == "error" && event["metadata"]["capture_stage"] == stage
        }));
    }
    let after_run_show = fixture.cb(&["show", &session_id], &[]);
    assert_success(&after_run_show);
    let after_run_show: Value =
        serde_json::from_slice(&after_run_show.stdout).expect("show emits JSON");
    assert_eq!(after_run_show["session"]["status"], "failed");

    let continued = fixture.cb(
        &[
            "continue",
            "--session",
            &session_id,
            "--from",
            "claude",
            "--to",
            "opencode",
        ],
        &[
            ("CB_FAKE_SOURCE_EVENTS", malformed_events.as_os_str()),
            ("CB_FAKE_MODIFY_FILE", tracked.as_os_str()),
            (
                "CB_FAKE_MODIFY_CONTENT",
                "continued despite malformed capture".as_ref(),
            ),
            ("CB_FAKE_MALFORMED_SESSION_METADATA", "not-json".as_ref()),
        ],
    );
    assert_eq!(
        continued.status.code(),
        Some(1),
        "a successful child with incomplete capture must fail the bridge command"
    );
    assert!(
        String::from_utf8_lossy(&continued.stderr).contains("post-exit artifacts"),
        "failure should explain that persistence completed: {}",
        String::from_utf8_lossy(&continued.stderr)
    );
    let continued_json: Value =
        serde_json::from_slice(&continued.stdout).expect("continue emits persisted outcome");
    assert_eq!(
        continued_json["post_exit_capture_failures"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    assert_eq!(
        fs::read_to_string(&tracked).expect("tracked file"),
        "continued despite malformed capture\n"
    );

    let final_show = fixture.cb(&["show", &session_id], &[]);
    assert_success(&final_show);
    let final_show: Value = serde_json::from_slice(&final_show.stdout).expect("show emits JSON");
    assert_eq!(final_show["session"]["status"], "failed");
    assert_eq!(final_show["session"]["active_agent"], "opencode");

    let final_timeline = fixture.cb(&["timeline", &session_id], &[]);
    assert_success(&final_timeline);
    let final_timeline: Vec<Value> =
        serde_json::from_slice(&final_timeline.stdout).expect("timeline emits JSON");
    assert!(
        final_timeline
            .iter()
            .filter(|event| event["kind"] == "error")
            .count()
            >= 4,
        "both runs must record canonical capture errors"
    );
    assert!(
        final_timeline
            .iter()
            .filter(|event| {
                event["kind"] == "file_modified"
                    && event["payload"]["data"]["path"] == "tracked.txt"
            })
            .count()
            >= 2,
        "both post-exit project changes must be reconciled"
    );
}

#[test]
fn production_launch_does_not_accept_the_test_capture_protocol() {
    let fixture = TestFixture::new();
    let claude_events = fixture_path("claude", "v1.0");
    let output = fixture.cb(
        &["run", "claude"],
        &[
            ("CB_TEST_MODE", "false".as_ref()),
            ("CB_FAKE_SOURCE_EVENTS", claude_events.as_os_str()),
            ("CB_FAKE_EXTERNAL_SESSION_ID", "fabricated-session".as_ref()),
        ],
    );
    assert_success(&output);
    let run: Value = serde_json::from_slice(&output.stdout).expect("run emits JSON");
    assert!(run["external_session_id"].is_null());
    let session_id = run["bridge_session_id"]
        .as_str()
        .expect("bridge session id");

    let show = fixture.cb(&["show", session_id], &[]);
    assert_success(&show);
    let show: Value = serde_json::from_slice(&show.stdout).expect("show emits JSON");
    assert!(
        show["external_links"].as_array().expect("links").is_empty(),
        "launch-only profiles must not create test-protocol links"
    );
    let timeline = fixture.cb(&["timeline", session_id], &[]);
    assert_success(&timeline);
    assert!(
        !String::from_utf8_lossy(&timeline.stdout)
            .contains("Build authentication with refresh-token rotation."),
        "production profiles must not import the fake event sink"
    );
}

struct TestFixture {
    root: TempDir,
    project: PathBuf,
    data_dir: PathBuf,
    fake_sessions: PathBuf,
    path: OsString,
}

impl TestFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("temporary test root");
        let project = root.path().join("project");
        let data_dir = root.path().join("data");
        let fake_bin = root.path().join("bin");
        let fake_sessions = root.path().join("sessions");
        fs::create_dir_all(&project).expect("project directory");
        fs::write(project.join("tracked.txt"), "initial\n").expect("tracked fixture");
        for name in ["claude", "opencode", "codex"] {
            FakeAgentExecutable::install(&fake_bin, name).expect("fake agent executable");
        }
        git(&project, &["init", "-q"]);
        git(&project, &["config", "user.name", "Context Bridge Test"]);
        git(
            &project,
            &["config", "user.email", "context-bridge@example.invalid"],
        );
        git(&project, &["add", "tracked.txt"]);
        git(&project, &["commit", "-q", "-m", "initial"]);
        let mut paths = vec![fake_bin.clone()];
        paths.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        let path = std::env::join_paths(paths).expect("test PATH");
        Self {
            root,
            project,
            data_dir,
            fake_sessions,
            path,
        }
    }

    fn cb(&self, args: &[&str], environment: &[(&str, &std::ffi::OsStr)]) -> Output {
        self.cb_for_project(&self.project, args, environment)
    }

    fn cb_for_project(
        &self,
        project: &Path,
        args: &[&str],
        environment: &[(&str, &std::ffi::OsStr)],
    ) -> Output {
        self.cb_command_for_project(project, args, environment)
            .output()
            .expect("run cb")
    }

    fn cb_command(&self, args: &[&str], environment: &[(&str, &std::ffi::OsStr)]) -> Command {
        self.cb_command_for_project(&self.project, args, environment)
    }

    fn cb_command_for_project(
        &self,
        project: &Path,
        args: &[&str],
        environment: &[(&str, &std::ffi::OsStr)],
    ) -> Command {
        let home = self.root.path().join("home");
        fs::create_dir_all(&home).expect("test home");
        let mut command = Command::new(env!("CARGO_BIN_EXE_cb"));
        command
            .arg("--project")
            .arg(project)
            .arg("--data-dir")
            .arg(&self.data_dir)
            .arg("--json")
            .args(args)
            .env("HOME", home)
            .env("PATH", &self.path)
            .env("CB_TEST_MODE", "true")
            .env("CB_PREVIEW_BEFORE_HANDOFF", "false")
            .env("CB_AGENT_CLAUDE_EXECUTABLE", "claude")
            .env("CB_AGENT_OPENCODE_EXECUTABLE", "opencode")
            .env("CB_AGENT_CODEX_EXECUTABLE", "codex")
            .env("CB_FAKE_SESSIONS_DIR", &self.fake_sessions)
            .env_remove("CB_EVENT_SINK")
            .env_remove("CB_SESSION_METADATA")
            .env_remove("CB_BOOTSTRAP_PATH")
            .env_remove("CB_BRIDGE_SESSION_ID");
        for (key, value) in environment {
            command.env(key, value);
        }
        command
    }

    fn sessions(&self) -> Vec<Value> {
        let output = self.cb(&["sessions"], &[]);
        assert_success(&output);
        serde_json::from_slice(&output.stdout).expect("sessions emit JSON")
    }
}

fn fixture_path(agent: &str, version: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(agent)
        .join(version)
        .join("session.jsonl")
}

fn git(project: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(project)
        .status()
        .expect("execute git");
    assert!(status.success(), "git {args:?} failed");
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
