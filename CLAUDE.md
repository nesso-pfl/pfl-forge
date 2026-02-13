# pfl-forge

Multi-agent issue processor powered by Claude Code.

## Architecture

- `src/pipeline/` — fetch → triage (deep → consult) → execute → integrate (rebase → test → review → push) / report の各ステージ
- `src/claude/` — Claude Code CLI (`claude -p`) のラッパー
- `src/git/` — worktree/branch 操作
- `src/github/` — octocrab による GitHub API 操作
- `src/state/` — YAML ファイルベースの状態管理
- `src/parent_prompt.rs` — 親エージェント用 prompt 生成

エージェント構成の詳細は [docs/agents.md](docs/agents.md) を参照。

## CLI subcommands

- `run` — issue 処理 (fetch → triage → execute → integrate)
- `watch` — daemon モードでポーリング
- `status` — 処理状態の表示
- `clean` — 完了済み worktree の削除
- `clarifications` — 未回答の clarification 一覧
- `answer <number> "<text>"` — clarification への回答
- `parent` — 親エージェント (interactive Claude Code session) を起動

## Development

```sh
cargo build
cargo test
```

## Key conventions

- Worker は `claude -p --allowedTools` で起動（`--dangerously-skip-permissions` は使わない）
- Parent は `claude --append-system-prompt --allowedTools Bash` + `exec()` で起動
- `env_remove("CLAUDECODE")` で nested Claude Code 呼び出しを有効化
- Git worktree でワーカー間のファイルシステム隔離
- octocrab で GitHub API 操作（`gh` CLI 不要）
- 設計変更時は docs/ 配下のドキュメント更新も検討すること
