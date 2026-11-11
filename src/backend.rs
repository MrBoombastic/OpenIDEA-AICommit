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

    /// What to tell the user when the host refused the backend's permission.
    fn denied_message(self, local_url: &str) -> String {
        let grant = |cap: &str| {
            format!(
                "{} is not allowed: grant {cap} to AI Commit Message in Settings › Plugins.",
                self.label()
            )
        };
        match self {
            Self::Claude => grant("process:claude"),
            Self::Codex => grant("process:codex"),
            Self::Anthropic => grant("net:api.anthropic.com"),
            Self::OpenAi => grant("net:api.openai.com"),
            Self::Local => match host_of(local_url) {
                Some(host @ ("localhost" | "127.0.0.1")) => grant(&format!("net:{host}")),
                host => format!(
                    "The local model at {} is not allowed. Grant net:ask to AI Commit Message in \
Settings › Plugins, then run Generate from the button and allow the host when asked \
(after Deny, the IDE does not ask again for 30 seconds).",
                    host.unwrap_or(local_url)
                ),
            },
        }
    }
}

pub fn setting(key: &str, default: &str) -> String {
    host::setting_string(key).unwrap_or_else(|| default.to_owned())
}

/// The host of `url`, without user info, port or IPv6 brackets.
fn host_of(url: &str) -> Option<&str> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = match host.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(v6),
        None => host.split(':').next().unwrap_or(host),
    };
    (!host.is_empty()).then_some(host)
}

fn is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1") || host.ends_with(".localhost")
}

/// The URL of the OpenAI-compatible server: http only on this machine.
fn local_url_checked(url: &str) -> Result<&str, Error> {
    let host = host_of(url)
        .ok_or_else(|| Error::Failed(format!("The local model URL is not a URL: {url}")))?;
    let scheme = url
        .split_once("://")
        .map_or("", |(s, _)| s)
        .to_ascii_lowercase();
    match scheme.as_str() {
        "https" => Ok(url),
        "http" if is_loopback(host) => Ok(url),
        "http" => Err(Error::Failed(format!(
            "Use https:// for the local model at {host}: plain http is allowed only to localhost."
        ))),
        _ => Err(Error::Failed(format!(
            "The local model URL must start with http:// or https://: {url}"
        ))),
    }
}

/// A model name from the settings, or `None` for the CLI's default. It goes in
/// one `--model=` argument, so it can never be read as another option.
fn model_arg(key: &str, default: &str) -> Result<Option<String>, Error> {
    let model = setting(key, default);
    let model = model.trim();
    if model.is_empty() {
        return Ok(None);
    }
    let valid = !model.starts_with('-')
        && model.len() <= 128
        && model
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._:/-[]@".contains(c));
    if !valid {
        return Err(Error::Failed(format!(
            "The model name {model:?} is not valid: use letters, digits and . _ : / - [ ] @."
        )));
    }
    Ok(Some(format!("--model={model}")))
}

/// Asks `backend` for an answer to `system` + `user`; the raw text it returned.
pub fn ask(
    backend: Backend,
    system: &str,
    user: &str,
    project: Option<&Project>,
) -> Result<String, Error> {
    let local_url = setting("localUrl", "http://localhost:11434/v1/chat/completions");
    let local_url = local_url.trim();
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
            local_url_checked(local_url)?,
            &setting("localModel", "qwen2.5-coder:7b"),
            None,
            system,
            user,
        ),
    };
    result.map_err(|e| match e {
        Error::Denied(_) => Error::Failed(backend.denied_message(local_url)),
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
/// Always without tools: the model sees only the prompt, never the project.
/// Settings (which a project can override) choose only the model.
fn claude(system: &str, user: &str, project: Option<&Project>) -> Result<String, Error> {
    let mut args: Vec<String> = ["-p", "--output-format", "text", "--tools", ""]
        .into_iter()
        .map(String::from)
        .collect();
    args.extend(model_arg("claudeModel", "haiku")?);
    args.extend(["--append-system-prompt".into(), system.into()]);
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
    args.extend(model_arg("codexModel", "")?);
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
        "claude-fable-5-2"
            | "claude-fable-5-1"
            | "claude-fable-5"
            | "claude-opus-5-5"
            | "claude-opus-5"
            | "claude-sonnet-5-5"
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
    use super::*;

    #[test]
    fn finds_the_host() {
        assert_eq!(
            host_of("http://localhost:11434/v1/chat/completions"),
            Some("localhost")
        );
        assert_eq!(host_of("http://127.0.0.1:1234/v1"), Some("127.0.0.1"));
        assert_eq!(host_of("http://[::1]:8080/v1"), Some("::1"));
        assert_eq!(
            host_of("https://user@llm.example.com/v1"),
            Some("llm.example.com")
        );
        assert_eq!(host_of("nope"), None);
    }

    #[test]
    fn plain_http_only_to_this_machine() {
        assert!(local_url_checked("http://localhost:11434/v1/chat/completions").is_ok());
        assert!(local_url_checked("http://127.0.0.1:1234/v1/chat/completions").is_ok());
        assert!(local_url_checked("https://llm.example.com/v1/chat/completions").is_ok());
        assert!(local_url_checked("http://192.168.1.5:11434/v1/chat/completions").is_err());
        assert!(local_url_checked("file:///etc/passwd").is_err());
    }

    #[test]
    fn the_denied_message_names_a_capability_the_manifest_declares() {
        let manifest = include_str!("../plugin.toml");
        for url in ["http://localhost:1/v1", "http://127.0.0.1:1/v1"] {
            let msg = Backend::Local.denied_message(url);
            let cap = msg
                .split("grant ")
                .nth(1)
                .unwrap()
                .split(' ')
                .next()
                .unwrap();
            assert!(
                manifest.contains(&format!("\"{cap}\"")),
                "{cap} is not in plugin.toml"
            );
        }
        assert!(
            Backend::Local
                .denied_message("https://llm.example.com/v1")
                .contains("net:ask")
        );
        assert!(manifest.contains("\"net:ask\""));
    }
}
