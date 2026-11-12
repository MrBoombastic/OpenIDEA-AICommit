//! AI Commit Message: a button above the commit message that writes it from
//! the included changes, with a signed-in CLI (Claude Code, Codex) or an API.

mod backend;
mod conventional;
mod prompt;

use openidea_plugin::commit::{CommitChecks, CommitProblem};
use openidea_plugin::{
    CommitDraft, Error, Invocation, NotificationAction, NotificationLevel, Plugin, Project, host,
    register_plugin,
};

use backend::{Backend, setting};

const LAST: &str = "last-message";
/// A notification in the project's window (API 1.5), or a global one when the
/// command ran outside a project (or the project has closed).
fn notify(
    project: Option<&Project>,
    level: NotificationLevel,
    title: &str,
    body: &str,
    actions: &[NotificationAction],
) -> Result<(), Error> {
    if let Some(p) = project {
        match p.notify(level, title, body, actions) {
            Err(Error::NotFound(_)) => {}
            other => return other,
        }
    }
    host::notify_with_actions(level, title, body, actions)
}

/// Shows `err` as a notification with Retry and Test backend, instead of the
/// IDE's plain error. Cancelling stays silent.
fn report(project: Option<&Project>, title: &str, retry: &str, err: Error) -> Result<(), Error> {
    if matches!(err, Error::Cancelled) {
        return Err(err);
    }
    let body = match &err {
        Error::Failed(m) => m.clone(),
        other => other.to_string(),
    };
    let mut actions = vec![NotificationAction::new("Retry", retry)];
    if retry != "test-backend" {
        actions.push(NotificationAction::new("Test backend", "test-backend"));
    }
    notify(project, NotificationLevel::Error, title, &body, &actions).or(Err(err))
}

#[derive(Default)]
struct AiCommit;

fn options() -> prompt::Options {
    let d = prompt::Options::default();
    prompt::Options {
        style: setting("style", &d.style),
        language: setting("language", &d.language),
        body: setting("body", &d.body),
        use_draft: host::setting_bool("useDraft").unwrap_or(d.use_draft),
        instructions: setting("instructions", ""),
        max_diff_bytes: host::setting_number("maxDiffKb")
            .map(|kb| (kb.clamp(4.0, 1024.0) as usize) * 1024)
            .unwrap_or(d.max_diff_bytes),
    }
}

fn backend() -> Backend {
    Backend::from_setting(&setting("backend", "claude"))
}

fn draft() -> Result<CommitDraft, Error> {
    host::pending_commit()?
        .ok_or_else(|| Error::Failed("This project has no Git repository.".into()))
}

/// Puts `message` in the message box of the draft at `revision`; when the user
/// typed meanwhile, offers it in a notification instead of overwriting.
fn deliver(message: &str, revision: u64, project: Option<&Project>) -> Result<(), Error> {
    let _ = host::storage::set(LAST, message);
    match host::set_commit_message(revision, message) {
        Ok(()) => Ok(()),
        Err(Error::Stale) => {
            let preview: String = message
                .lines()
                .next()
                .unwrap_or_default()
                .chars()
                .take(120)
                .collect();
            notify(
                project,
                NotificationLevel::Info,
                "The commit changed while the message was written",
                &format!("Nothing was overwritten. Suggestion: {preview}"),
                &[
                    NotificationAction::new("Insert anyway", "insert").argument(message),
                    NotificationAction::new("Copy", "copy").argument(message),
                ],
            )
        }
        Err(e) => Err(e),
    }
}

impl AiCommit {
    fn generate(&self, ctx: &Invocation) -> Result<Option<String>, Error> {
        let project = ctx.project();
        match self.write_message(project.as_ref()) {
            Ok(()) => Ok(None),
            Err(e) => report(
                project.as_ref(),
                "The commit message was not written",
                "generate",
                e,
            )
            .map(|()| None),
        }
    }

    fn write_message(&self, project: Option<&Project>) -> Result<(), Error> {
        let draft = draft()?;
        if draft.changes.is_empty() {
            return Err(Error::Failed("Select the files to commit first: the message is written from the included changes.".into()));
        }
        let backend = backend();
        let user = prompt::build(&draft, &options());
        let task = host::begin_task(
            &format!("Writing the commit message ({})", backend.label()),
            true,
        );
        task.report(0, None, None);
        let answer = backend::ask(backend, prompt::SYSTEM, &user, project);
        if task.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let message = prompt::clean(&answer?);
        if message.is_empty() {
            return Err(Error::Failed("The model returned an empty message.".into()));
        }
        drop(task);
        deliver(&message, draft.revision, project)
    }

    /// Inserts `arg` (from a notification) or the last generated message.
    fn insert(&self, arg: Option<&str>) -> Result<Option<String>, Error> {
        let message = match arg {
            Some(m) => m.to_owned(),
            None => host::storage::get(LAST)?
                .ok_or_else(|| Error::Failed("No message was generated yet.".into()))?,
        };
        let draft = draft()?;
        host::set_commit_message(draft.revision, &message)?;
        Ok(None)
    }

    fn copy(&self, arg: Option<&str>) -> Result<Option<String>, Error> {
        let message = match arg {
            Some(m) => m.to_owned(),
            None => host::storage::get(LAST)?
                .ok_or_else(|| Error::Failed("No message was generated yet.".into()))?,
        };
        host::copy_to_clipboard(&message).map_err(|e| match e {
            Error::Denied(_) => Error::Failed(
                "Copying needs the clipboard permission (Settings › Plugins › AI Commit Message)."
                    .into(),
            ),
            other => other,
        })?;
        Ok(None)
    }

    /// A tiny request to the chosen backend: is it installed, signed in and allowed?
    fn test_backend(&self, ctx: &Invocation) -> Result<Option<String>, Error> {
        let project = ctx.project();
        let backend = backend();
        let task = host::begin_task(&format!("Testing {}", backend.label()), true);
        let answer = backend::ask(
            backend,
            "Answer with one word.",
            "Reply with the word OK.",
            project.as_ref(),
        );
        drop(task);
        let answer = match answer {
            Ok(a) => a,
            Err(e) => {
                let title = format!("{} does not work", backend.label());
                return report(project.as_ref(), &title, "test-backend", e).map(|()| None);
            }
        };
        let answer: String = prompt::clean(&answer).chars().take(80).collect();
        notify(
            project.as_ref(),
            NotificationLevel::Info,
            &format!("{} works", backend.label()),
            &format!("It answered: {answer}"),
            &[],
        )?;
        Ok(None)
    }
}

impl Plugin for AiCommit {
    fn run_command(
        &mut self,
        id: &str,
        ctx: &Invocation,
        arg: Option<&str>,
    ) -> Result<Option<String>, Error> {
        match id {
            "generate" => self.generate(ctx),
            "insert" => self.insert(arg),
            "copy" => self.copy(arg),
            "test-backend" => self.test_backend(ctx),
            _ => Err(Error::NotFound(format!("command {id}"))),
        }
    }
}

impl CommitChecks for AiCommit {
    fn check(
        &mut self,
        check: &str,
        draft: &CommitDraft,
        _ctx: &Invocation,
    ) -> Result<Vec<CommitProblem>, Error> {
        if check != "conventional" || setting("style", "conventional") != "conventional" {
            return Ok(Vec::new());
        }
        Ok(conventional::problem(&draft.message)
            .map(CommitProblem::new)
            .into_iter()
            .collect())
    }
}

register_plugin!(AiCommit, commit_checks);

#[cfg(test)]
mod tests {
    use super::*;
    use openidea_plugin::mock::MockHost;
    use openidea_plugin::{CommitChange, HttpResponse, ProcessOutput};

    const CAPS: &[&str] = &[
        "vcs.read",
        "vcs.write",
        "storage",
        "process:claude",
        "process:codex",
        "net:api.anthropic.com",
        "net:localhost",
        "secrets",
        "clipboard",
    ];

    /// The body of the last notification sent to the project, and its buttons.
    fn last_notice(host: &MockHost) -> (String, Vec<String>) {
        let (root, n) = host
            .project_notifications()
            .pop()
            .expect("no project notification");
        assert_eq!(root, "/repo");
        let commands = n.actions.iter().map(|a| a.command.clone()).collect();
        (n.notification.body, commands)
    }

    fn host_with_draft(message: &str) -> MockHost {
        let host = MockHost::install().with_capabilities(CAPS);
        host.set_project_root(Some("/repo"));
        host.set_commit_draft(Some(CommitDraft {
            revision: 7,
            message: message.into(),
            amend: false,
            changes: vec![CommitChange {
                repository: "/repo".into(),
                path: "/repo/src/login.rs".into(),
                old_path: None,
                status: "modified".into(),
                diff: Some("@@ -1 +1 @@\n-panic!()\n+return Err(e)\n".into()),
                truncated: false,
            }],
        }));
        host
    }

    fn ok(stdout: &str) -> Result<ProcessOutput, Error> {
        Ok(ProcessOutput {
            status: Some(0),
            stdout: stdout.into(),
            stderr: String::new(),
        })
    }

    #[test]
    fn claude_writes_the_message_from_the_diff() {
        let host = host_with_draft("");
        host.on_process("claude", |args, stdin| {
            assert!(args.starts_with(&["-p".to_owned()]));
            assert!(stdin.unwrap().contains("+return Err(e)"));
            ok("```\nfix(login): return an error instead of panicking\n```\n")
        });
        AiCommit
            .run_command("generate", &host.invocation(), None)
            .unwrap();
        assert_eq!(
            host.commit_draft().unwrap().message,
            "fix(login): return an error instead of panicking"
        );
        let run = &host.process_runs()[0];
        assert_eq!(
            run.args[..5],
            ["-p", "--output-format", "text", "--tools", ""]
        );
        assert!(run.args.contains(&"--model=haiku".to_owned()));
        assert!(host.project_notifications().is_empty());
        assert_eq!(
            host.storage().get(LAST).unwrap(),
            "fix(login): return an error instead of panicking"
        );
        assert_eq!(host.tasks().len(), 1);
    }

    #[test]
    fn codex_reads_the_prompt_from_stdin() {
        let host = host_with_draft("");
        host.set_setting("backend", "\"codex\"");
        host.set_setting("codexModel", "\"gpt-5-codex\"");
        host.on_process("codex", |_, stdin| {
            assert!(stdin.unwrap().starts_with(prompt::SYSTEM));
            ok("fix: handle login errors\n")
        });
        AiCommit
            .run_command("generate", &host.invocation(), None)
            .unwrap();
        let run = &host.process_runs()[0];
        assert_eq!(run.args.first().unwrap(), "exec");
        assert_eq!(run.args.last().unwrap(), "-");
        assert!(run.args.contains(&"--model=gpt-5-codex".to_owned()));
        assert!(
            run.args
                .windows(2)
                .any(|w| w == ["-c", "features.shell_tool=false"])
        );
        assert!(run.args.contains(&"--ephemeral".to_owned()));
        assert!(run.args.contains(&"--ignore-user-config".to_owned()));
        assert_eq!(
            host.commit_draft().unwrap().message,
            "fix: handle login errors"
        );
    }

    #[test]
    fn a_failing_cli_reports_its_stderr() {
        let host = host_with_draft("");
        host.on_process("claude", |_, _| {
            Ok(ProcessOutput {
                status: Some(1),
                stdout: String::new(),
                stderr: "Invalid API key · Please run /login".into(),
            })
        });
        AiCommit
            .run_command("generate", &host.invocation(), None)
            .unwrap();
        let (body, actions) = last_notice(&host);
        assert!(body.contains("Please run /login"), "{body}");
        assert_eq!(actions, ["generate", "test-backend"]);
        assert_eq!(host.commit_draft().unwrap().message, "");
    }

    #[test]
    fn a_missing_grant_says_what_to_grant() {
        let host = host_with_draft("");
        let host = host.with_capabilities(&["vcs.read", "vcs.write", "storage"]);
        AiCommit
            .run_command("generate", &host.invocation(), None)
            .unwrap();
        assert!(last_notice(&host).0.contains("grant process:claude"));
    }

    #[test]
    fn project_settings_cannot_add_cli_arguments() {
        let host = host_with_draft("");
        // Settings of the old versions, now ignored.
        host.set_setting("claudeExtraArgs", "\"--dangerously-skip-permissions\"");
        host.set_setting("claudeNoTools", "false");
        host.set_setting("claudeModel", "\"--allowedTools Bash\"");
        host.on_process("claude", |_, _| ok("fix: x"));
        AiCommit
            .run_command("generate", &host.invocation(), None)
            .unwrap();
        assert!(host.process_runs().is_empty());
        assert!(last_notice(&host).0.contains("is not valid"));
        host.set_setting("claudeModel", "\"sonnet\"");
        AiCommit
            .run_command("generate", &host.invocation(), None)
            .unwrap();
        let args = &host.process_runs()[0].args;
        assert!(args.windows(2).any(|w| w == ["--tools", ""]));
        assert!(args.iter().all(|a| !a.contains("dangerously")));
        assert!(args.contains(&"--model=sonnet".to_owned()));
        assert!(args.windows(2).any(|w| w == ["--setting-sources", "user"]));
        assert!(args.contains(&"--strict-mcp-config".to_owned()));
    }

    #[test]
    fn local_model_on_127_0_0_1() {
        let host = host_with_draft("");
        host.set_setting("backend", "\"local\"");
        host.set_setting("localUrl", "\"http://127.0.0.1:1234/v1/chat/completions\"");
        AiCommit
            .run_command("generate", &host.invocation(), None)
            .unwrap();
        let (body, _) = last_notice(&host);
        assert!(body.contains("grant net:127.0.0.1"), "{body}");
        let host = host.with_capabilities(&[CAPS, &["net:127.0.0.1"]].concat());
        host.on_fetch(|req| {
            assert!(req.headers.iter().all(|(k, _)| k != "authorization"));
            Ok(HttpResponse {
                status: 200,
                headers: vec![],
                body: br#"{"choices":[{"message":{"content":"feat: y"}}]}"#.to_vec(),
            })
        });
        AiCommit
            .run_command("generate", &host.invocation(), None)
            .unwrap();
        assert_eq!(host.commit_draft().unwrap().message, "feat: y");
    }

    #[test]
    fn local_model_elsewhere_needs_https_and_net_ask() {
        let host = host_with_draft("");
        host.set_setting("backend", "\"local\"");
        host.set_setting("localUrl", "\"http://10.0.0.2:11434/v1/chat/completions\"");
        AiCommit
            .run_command("generate", &host.invocation(), None)
            .unwrap();
        assert!(last_notice(&host).0.contains("Use https://"));
        assert!(host.requests().is_empty());
        host.set_setting(
            "localUrl",
            "\"https://llm.example.com/v1/chat/completions\"",
        );
        AiCommit
            .run_command("generate", &host.invocation(), None)
            .unwrap();
        assert!(last_notice(&host).0.contains("net:ask"));
    }

    #[test]
    fn typing_meanwhile_is_never_overwritten() {
        let host = host_with_draft("");
        // The answer was written for revision 7; the user typed, so the draft moved on.
        host.edit_commit_message("my own text");
        let project = host.invocation().project();
        deliver("feat: x", 7, project.as_ref()).unwrap();
        assert_eq!(host.commit_draft().unwrap().message, "my own text");
        assert_eq!(last_notice(&host).1, ["insert", "copy"]);
        AiCommit
            .run_command("insert", &host.invocation(), Some("feat: x"))
            .unwrap();
        assert_eq!(host.commit_draft().unwrap().message, "feat: x");
    }

    #[test]
    fn nothing_selected_is_an_error() {
        let host = MockHost::install().with_capabilities(CAPS);
        host.set_commit_draft(Some(CommitDraft {
            revision: 1,
            message: String::new(),
            amend: false,
            changes: vec![],
        }));
        host.set_project_root(Some("/repo"));
        AiCommit
            .run_command("generate", &host.invocation(), None)
            .unwrap();
        assert!(last_notice(&host).0.contains("Select the files"));
    }

    #[test]
    fn anthropic_api_uses_the_keychain_secret() {
        let host = host_with_draft("");
        host.set_setting("backend", "\"anthropic\"");
        host.set_secret("anthropic-api-key", "sk-test");
        host.set_setting("anthropicModel", "\"claude-opus-5-5\"");
        host.on_fetch(|req| {
            assert_eq!(req.url, "https://api.anthropic.com/v1/messages");
            assert!(req.headers.contains(&("x-api-key".into(), "sk-test".into())));
            assert!(req.headers.contains(&("anthropic-version".into(), "2026-10-21".into())));
            let body: serde_json::Value = serde_json::from_slice(req.body.as_ref().unwrap()).unwrap();
            assert_eq!(body["model"], "claude-opus-5-5");
            assert_eq!(body["output_config"]["effort"], "low");
            assert_eq!(body["fallbacks"], "default");
            assert!(req.headers.iter().any(|(k, v)| k == "anthropic-beta" && v == "server-side-fallback-2026-07-01"));
            Ok(HttpResponse {
                status: 200,
                headers: vec![],
                body: br#"{"stop_reason":"end_turn","content":[{"type":"thinking","thinking":""},{"type":"text","text":"refactor: tidy login"}]}"#.to_vec(),
            })
        });
        AiCommit
            .run_command("generate", &host.invocation(), None)
            .unwrap();
        assert_eq!(host.commit_draft().unwrap().message, "refactor: tidy login");
    }

    #[test]
    fn anthropic_api_without_a_key_asks_for_one() {
        let host = host_with_draft("");
        host.set_setting("backend", "\"anthropic\"");
        AiCommit
            .run_command("generate", &host.invocation(), None)
            .unwrap();
        assert!(last_notice(&host).0.contains("Anthropic API key"));
        assert!(host.requests().is_empty());
    }

    #[test]
    fn conventional_check_follows_the_style_setting() {
        let host = host_with_draft("Fixed stuff");
        let draft = host.commit_draft().unwrap();
        let problems = AiCommit
            .check("conventional", &draft, &host.invocation())
            .unwrap();
        assert_eq!(problems.len(), 1);
        host.set_setting("style", "\"plain\"");
        assert!(
            AiCommit
                .check("conventional", &draft, &host.invocation())
                .unwrap()
                .is_empty()
        );
    }
}
