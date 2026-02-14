# pfl-forge

Multi-agent issue processor powered by Claude Code.

## Architecture

- `src/pipeline/` — fetch → triage (deep → consult) → execute → integrate (rebase → test → review → push) / report の各ステージ
- `src/claude/` — Claude Code CLI (`claude -p`) のラッパー
- `src/git/` — worktree/branch 操作
- `src/github/` — octocrab による GitHub API 操作、`TaskSource` (GitHub/Local) によるタスクソース分岐
- `src/state/` — YAML ファイルベースの状態管理
- `src/prompt/` — 各エージェントの system prompt（`.md` ファイル、`include_str!` で埋め込み）
- `src/parent_prompt.rs` — 親エージェント用 prompt 生成

エージェント間通信は worktree 内 `.forge/` ディレクトリの YAML ファイルで行う（triage.yaml, review.yaml）。
ローカルタスクは `.forge/tasks/*.yaml` に配置し、GitHub issue と同じパイプラインで処理される。

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

- Worker は `claude -p --allowedTools --append-system-prompt` で起動（`--dangerously-skip-permissions` は使わない）
- Parent は `claude --append-system-prompt --allowedTools Bash` + `exec()` で起動
- `env_remove("CLAUDECODE")` で nested Claude Code 呼び出しを有効化
- Git worktree でワーカー間のファイルシステム隔離
- エージェント間データは `.forge/triage.yaml`, `.forge/review.yaml` で受け渡し（プロンプト埋め込みではなくファイル経由）
- ローカルタスク: `.forge/tasks/*.yaml` に定義、`TaskSource::Local` として処理（push/PR をスキップ）
- octocrab で GitHub API 操作（`gh` CLI 不要）
- コミット前に、変更が CLAUDE.md や docs/ の記述と矛盾しないか確認し、必要なら同じコミットで更新すること
