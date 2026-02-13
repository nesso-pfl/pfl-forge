# pfl-forge

Multi-agent issue processor powered by Claude Code.

## Architecture

- `src/pipeline/` — fetch → triage → execute → report の各ステージ
- `src/claude/` — Claude Code CLI (`claude -p`) のラッパー
- `src/git/` — worktree/branch 操作
- `src/github/` — octocrab による GitHub API 操作
- `src/state/` — YAML ファイルベースの状態管理

## Development

```sh
cargo build
cargo test
```

## Key conventions

- Worker は `claude -p --allowedTools` で起動（`--dangerously-skip-permissions` は使わない）
- `env_remove("CLAUDECODE")` で nested Claude Code 呼び出しを有効化
- Git worktree でワーカー間のファイルシステム隔離
- octocrab で GitHub API 操作（`gh` CLI 不要）
