# AI Commit Message for OpenIDEA

A button above the commit message (Commit tool window and Commit dialog) that writes the message from the changes you included: only the checked files and changed blocks, as Commit would commit them.

It talks to the AI you already use:

| Backend | How it signs in | Permission to grant |
| --- | --- | --- |
| **Claude Code** (default) | Your `claude` CLI login, the same one OpenIDEA's Claude Code agent uses. No API key. | `process:claude` |
| **Codex** | Your `codex login`, the same one OpenIDEA's Codex agent uses. No API key. | `process:codex` |
| Anthropic API | API key in Settings › Plugins › AI Commit Message (in the list) › Secrets, kept in the system keychain | `net:api.anthropic.com`, `secrets` |
| OpenAI API | API key in Settings › Plugins › AI Commit Message (in the list) › Secrets, kept in the system keychain | `net:api.openai.com`, `secrets` |
| Local model | Any OpenAI-compatible server (Ollama, LM Studio, llama.cpp, vLLM). No key is sent. | `net:localhost` or `net:127.0.0.1`; any other host over HTTPS: `net:ask`, then allow the host when the IDE asks |

All permissions except `vcs.read`, `vcs.write` and `storage` are optional: grant only the backend you use.

## Features

- Conventional Commits, plain or gitmoji style; the message in any language you choose.
- Subject only, subject + body, or a body only when the change needs one.
- Text you already typed in the message box is used as a hint ("fix login crash" becomes a proper message). When amending, the previous message is kept and extended.
- Never overwrites what you type: if the draft changed while the model was writing, the suggestion comes as a notification with **Insert anyway** and **Copy**.
- Errors and results show as notifications of the project, with **Retry** and **Test backend**.
- A **Conventional Commits message** before-commit check (Commit Options), active when the style is `conventional`.

## Requirements

OpenIDEA with plugin API 1.5 (0.1.41 or later), trusted project. For the CLI backends, `claude` or `codex` on your `PATH` and signed in (run it once in a terminal).

Claude Code runs as `claude -p --output-format text --tools "" --setting-sources user --strict-mcp-config --disable-slash-commands --no-session-persistence --model=haiku --append-system-prompt …` with the prompt on stdin: no tools, no MCP servers, and none of the project's `.claude` settings (print mode skips Claude Code's trust dialog, so a repository's hooks or `ANTHROPIC_BASE_URL` would otherwise apply). It needs a recent Claude Code. Codex runs as `codex exec --skip-git-repo-check --sandbox read-only --ephemeral --ignore-user-config --ignore-rules -c features.shell_tool=false -c features.view_image=false -c features.multi_agent=false -c features.apps=false --color never [--model=…] -`: the read-only sandbox alone still lets Codex read any file (with its shell, or an MCP server from `~/.codex/config.toml`), so it runs without your Codex config, hooks and tools; your `codex login` still works. Set the model in the plugin, since `~/.codex/config.toml` is not read. Tested with codex-cli 0.167.0. Only the model can be set (Settings › Plugins › AI Commit Message › Backends, with a **Test backend** button): settings can come from a project's `.openidea/settings.toml`, so the plugin takes no other command-line arguments from them. A run may take up to about 60 seconds (the IDE's limit for a plugin task).

## Building

```sh
cargo test
openidea-plugin build          # dist/plugin.wasm + check
openidea-plugin link           # try it in the IDE
openidea-plugin pack . --key <your.key> --publisher mrboombastic
```

## License

MIT
