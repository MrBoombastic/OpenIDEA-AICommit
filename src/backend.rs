//! The model behind the button: a signed-in CLI or an HTTP API.

use openidea_plugin::host::{self, Command};
use openidea_plugin::{Error, HttpRequest, Project};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Claude,
    Codex,
    Anthropic,
    OpenAi,
    Local,
}

impl Backend {
    pub fn from_setting(value: &str) -> Self {
        match value {
            "codex" => Self::Codex,
            "anthropic" => Self::Anthropic,
            "openai" => Self::OpenAi,
            "local" => Self::Local,
            _ => Self::Claude,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
            Self::Anthropic => "Anthropic API",
            Self::OpenAi => "OpenAI API",
            Self::Local => "local model",
        }
    }

    /// The capability the backend needs, for the "not granted" message.
    fn capability(self, local_url: &str) -> String {
        match self {
            Self::Claude => "process:claude".into(),
            Self::Codex => "process:codex".into(),
            Self::Anthropic => "net:api.anthropic.com".into(),
            Self::OpenAi => "net:api.openai.com".into(),
            Self::Local => format!("net:{}", host_of(local_url).unwrap_or("localhost")),
        }
    }
}

pub fn setting(key: &str, default: &str) -> String {
    host::setting_string(key).unwrap_or_else(|| default.to_owned())
}

fn host_of(url: &str) -> Option<&str> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    Some(host.split(':').next().unwrap_or(host))
}

fn words(s: &str) -> impl Iterator<Item = &str> {
    s.split_whitespace()
}

/// Asks `backend` for an answer to `system` + `user`; the raw text it returned.
pub fn ask(
    backend: Backend,
    system: &str,
    user: &str,
    project: Option<&Project>,
) -> Result<String, Error> {
    let local_url = setting("localUrl", "http://localhost:11434/v1/chat/completions");
    let result = match backend {
        Backend::Claude => claude(system, user, project),
        Backend::Codex => codex(system, user, project),
        Backend::Anthropic => anthropic(system, user),
        Backend::OpenAi => openai_compatible(
            "https://api.openai.com/v1/chat/completions",
            &setting("openaiModel", "gpt-4.1-mini"),
            Some("Bearer {{secret:openai-api-key}}"),
            system,
            user,
        ),
        Backend::Local => openai_compatible(
            &local_url,
            &setting("localModel", "qwen2.5-coder:7b"),
            None,
            system,
            user,
        ),
    };
    result.map_err(|e| match e {
        Error::Denied(_) => Error::Failed(format!(
            "{} is not allowed: grant {} to AI Commit Message in Settings › Plugins.",
            backend.label(),
            backend.capability(&local_url)
        )),
        other => other,
    })
}

/// Runs an approved CLI with the prompt on stdin and returns its stdout.
fn run_cli(
    program: &str,
    args: Vec<String>,
    stdin: &str,
    project: Option<&Project>,
) -> Result<String, Error> {
    let cmd = Command::new(program).args(args);
    let out = host::run_command(&cmd, project, Some(stdin))?;
    if out.status != Some(0) {
        let detail = if out.stderr.trim().is_empty() {
            out.stdout.trim()
        } else {
            out.stderr.trim()
        };
        let detail: String = detail.chars().take(600).collect();
        return Err(Error::Failed(match out.status {
            Some(code) => format!("{program} exited with code {code}: {detail}"),
            None => format!("{program} was stopped: {detail}"),
        }));
    }
    if out.stdout.trim().is_empty() {
        return Err(Error::Failed(format!(
            "{program} returned nothing. {}",
            out.stderr.trim()
        )));
    }
    Ok(out.stdout)
}

/// `claude -p`: Claude Code in print mode, signed in as the user is in the IDE's agent.
fn claude(system: &str, user: &str, project: Option<&Project>) -> Result<String, Error> {
    let mut args: Vec<String> = vec!["-p".into(), "--output-format".into(), "text".into()];
    let model = setting("claudeModel", "haiku");
    if !model.trim().is_empty() {
        args.extend(["--model".into(), model.trim().into()]);
    }
    if host::setting_bool("claudeNoTools").unwrap_or(true) {
        args.extend(["--tools".into(), String::new()]);
    }
    args.extend(["--append-system-prompt".into(), system.into()]);
    args.extend(words(&setting("claudeExtraArgs", "")).map(String::from));
    run_cli("claude", args, user, project)
}

/// `codex exec -`: the prompt comes from stdin; the final message goes to stdout.
fn codex(system: &str, user: &str, project: Option<&Project>) -> Result<String, Error> {
    let mut args: Vec<String> = [
        "exec",
        "--skip-git-repo-check",
        "--sandbox",
        "read-only",
        "--color",
        "never",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    let model = setting("codexModel", "");
    if !model.trim().is_empty() {
        args.extend(["-m".into(), model.trim().into()]);
    }
    args.extend(words(&setting("codexExtraArgs", "")).map(String::from));
    args.push("-".into());
    run_cli("codex", args, &format!("{system}\n\n{user}"), project)
}

fn api_error(service: &str, status: u16, body: &str) -> Error {
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .and_then(Value::as_str)
                .map(String::from)
        })
        .unwrap_or_else(|| body.chars().take(300).collect());
    Error::Failed(format!("{service} answered {status}: {message}"))
}

/// Models that take the server-side refusal fallback (`fallbacks: "default"`).
fn takes_fallbacks(model: &str) -> bool {
    matches!(
        model,
        "claude-fable-5-1" | "claude-opus-5-5" | "claude-opus-5" | "claude-sonnet-5-5"
    )
}

/// The Anthropic Messages API, with the key from the system keychain.
fn anthropic(system: &str, user: &str) -> Result<String, Error> {
    if !host::secret_is_set("anthropic-api-key") {
        return Err(Error::Failed(
            "Enter the Anthropic API key under Settings › Plugins › AI Commit Message.".into(),
        ));
    }
    let model = setting("anthropicModel", "claude-opus-5-5");
    let mut body = json!({
        "model": model,
        "max_tokens": 16000,
        "system": system,
        "messages": [{ "role": "user", "content": user }],
    });
    let effort = setting("anthropicEffort", "low");
    if effort != "default" {
        body["output_config"] = json!({ "effort": effort });
    }
    let mut req = HttpRequest::post("https://api.anthropic.com/v1/messages", body.to_string())
        .header("content-type", "application/json")
        .header("x-api-key", "{{secret:anthropic-api-key}}")
        .header("anthropic-version", "2026-10-21");
    if takes_fallbacks(&model) {
        body["fallbacks"] = json!("default");
        req.body = Some(body.to_string().into_bytes());
        req = req.header("anthropic-beta", "server-side-fallback-2026-07-01");
    }
    req.timeout_ms = Some(58_000);
    let res = host::fetch(&req)?;
    let text = res.text();
    if res.status != 200 {
        return Err(api_error("Anthropic API", res.status, &text));
    }
    let v: Value =
        serde_json::from_str(&text).map_err(|e| Error::Failed(format!("Anthropic API: {e}")))?;
    if v["stop_reason"] == "refusal" {
        return Err(Error::Failed(
            "Anthropic API declined to write this message (refusal).".into(),
        ));
    }
    let answer: String = v["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect();
    if answer.trim().is_empty() {
        return Err(Error::Failed("Anthropic API returned no text.".into()));
    }
    Ok(answer)
}

/// OpenAI's chat completions, or any compatible local server.
fn openai_compatible(
    url: &str,
    model: &str,
    auth: Option<&str>,
    system: &str,
    user: &str,
) -> Result<String, Error> {
    if auth.is_some() && !host::secret_is_set("openai-api-key") {
        return Err(Error::Failed(
            "Enter the OpenAI API key under Settings › Plugins › AI Commit Message.".into(),
        ));
    }
    let body = json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user },
        ],
    });
    let mut req =
        HttpRequest::post(url, body.to_string()).header("content-type", "application/json");
    if let Some(auth) = auth {
        req = req.header("authorization", auth);
    }
    req.timeout_ms = Some(58_000);
    let service = if auth.is_some() {
        "OpenAI API"
    } else {
        "Local model"
    };
    let res = host::fetch(&req)?;
    let text = res.text();
    if res.status != 200 {
        return Err(api_error(service, res.status, &text));
    }
    let v: Value =
        serde_json::from_str(&text).map_err(|e| Error::Failed(format!("{service}: {e}")))?;
    let answer = v
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if answer.trim().is_empty() {
        return Err(Error::Failed(format!("{service} returned no text.")));
    }
    Ok(answer.to_owned())
}

#[cfg(test)]
mod tests {
    use super::host_of;

    #[test]
    fn finds_the_host() {
        assert_eq!(
            host_of("http://localhost:11434/v1/chat/completions"),
            Some("localhost")
        );
        assert_eq!(host_of("http://127.0.0.1:1234/v1"), Some("127.0.0.1"));
        assert_eq!(host_of("nope"), None);
    }
}
