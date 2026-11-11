# AI Commit Message for OpenIDEA

A button above the commit message (Commit tool window and Commit dialog) that writes the message from the changes you included: only the checked files and changed blocks, as Commit would commit them.

It talks to the AI you already use:

| Backend | How it signs in | Permission to grant |
| --- | --- | --- |
| **Claude Code** (default) | Your `claude` CLI login, the same one OpenIDEA's Claude Code agent uses. No API key. | `process:claude` |
| **Codex** | Your `codex login`, the same one OpenIDEA's Codex agent uses. No API key. | `process:codex` |
| Anthropic API | API key in Settings › Plugins (system keychain) | `net:api.anthropic.com`, `secrets` |
| OpenAI API | API key in Settings › Plugins (system keychain) | `net:api.openai.com`, `secrets` |
| Local model | Any OpenAI-compatible server (Ollama, LM Studio, llama.cpp, vLLM). No key is sent. | `net:localhost` or `net:127.0.0.1`; any other host over HTTPS: `net:ask`, then allow the host when the IDE asks |

All permissions except `vcs.read`, `vcs.write` and `storage` are optional: grant only the backend you use.

## Features

- Conventional Commits, plain or gitmoji style; the message in any language you choose.
- Subject only, subject + body, or a body only when the change needs one.
- Text you already typed in the message box is used as a hint ("fix login crash" becomes a proper message).
- Never overwrites what you type: if the draft changed while the model was writing, the suggestion comes as a notification with **Insert anyway** and **Copy**.
- Errors and results show as notifications of the project, with **Retry** and **Test backend**.
- A **Conventional Commits message** before-commit check (Commit Options), active when the style is `conventional`.

## Requirements

OpenIDEA with plugin API 1.5 (0.1.41 or later), trusted project. For the CLI backends, `claude` or `codex` on your `PATH` and signed in (run it once in a terminal).

Claude Code runs as `claude -p --output-format text --tools "" --model=haiku --append-system-prompt …` with the prompt on stdin, always without tools (a Claude Code version with `--tools` is needed). Codex runs as `codex exec --skip-git-repo-check --sandbox read-only --color never [--model=…] -`. Only the model can be set (Settings › Plugins › AI Commit Message › Backends, with a **Test backend** button): settings can come from a project's `.openidea/settings.toml`, so the plugin takes no other command-line arguments from them. A run may take up to about 60 seconds (the IDE's limit for a plugin task).

## Building

```sh
cargo test
openidea-plugin build          # dist/plugin.wasm + check
openidea-plugin link           # try it in the IDE
openidea-plugin pack . --key <your.key> --publisher mrboombastic
```

## License

MIT
