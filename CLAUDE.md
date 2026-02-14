# pfl-forge

Multi-agent task processor powered by Claude Code.

## Architecture

- `src/pipeline/` — fetch → triage (deep → consult) → work (task YAML) → execute → integrate (rebase → test → review) / report の各ステージ
- `src/claude/` — Claude Code CLI (`claude -p`) のラッパー
- `src/git/` — worktree/branch 操作
- `src/task.rs` — `ForgeIssue` 定義（ローカルタスク）
- `src/state/` — YAML ファイルベースの状態管理
- `src/prompt/` — 各エージェントの system prompt（`.md` ファイル、`include_str!` で埋め込み）
- `src/parent_prompt.rs` — 親エージェント用 prompt 生成

エージェント間通信は `.forge/` ディレクトリの YAML ファイルで行う。triage は `.forge/work/*.yaml` にタスクを書き出し、execute は worktree 内 `.forge/task.yaml` で Worker に渡す。review 結果は `.forge/review.yaml`。
タスクは `.forge/tasks/*.yaml` に配置する。

エージェント構成の詳細は [docs/agents.md](docs/agents.md)、パイプラインフローは [docs/pipeline.md](docs/pipeline.md) を参照。

## Config

`pfl-forge.yaml` をリポジトリルートに配置。CWD ベースの単一リポモデル。状態は `.forge/state.yaml` に保存。タスク ID はファイル名（UUID 等の任意文字列）。

## CLI subcommands

- `run` — タスク処理 (fetch → triage → execute → integrate)
- `watch` — daemon モードでポーリング
- `status` — 処理状態の表示
- `clean` — 完了済み worktree の削除
- `clarifications` — 未回答の clarification 一覧
- `answer <id> "<text>"` — clarification への回答
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
- エージェント間データは `.forge/work/*.yaml`（タスク）、`.forge/task.yaml`（worktree 内）、`.forge/review.yaml` で受け渡し（プロンプト埋め込みではなくファイル経由）
- タスク: `.forge/tasks/*.yaml` に定義
- コミット前に、変更が CLAUDE.md、README.md や docs/ の記述と矛盾しないか確認し、必要なら同じコミットで更新すること
- 作業前に `.tmp/TODO.md` を確認し、関連する課題があれば意識すること
