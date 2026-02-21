# pfl-forge

Multi-agent task processor powered by Claude Code.

## Architecture

- `src/agent/` — Claude Code 呼び出し（プロンプト組み立て・CLI 実行・出力パース）
- `src/intent/` — Intent 定義・読み込み・Registry・draft 変換
- `src/task/` — Task 構造体・work YAML I/O
- `src/runner/` — Flow 実行エンジン（ステップ逐次実行 + ルールベース調整）
- `src/knowledge/` — History 記録・Observation 読み書き
- `src/claude/` — Claude Code CLI (`claude -p`) のラッパー
- `src/git/` — worktree/branch 操作（rebase・gitignore 管理を含む）
- `src/prompt/` — 各エージェントの system prompt（`.md` ファイル、`include_str!` で埋め込み）
- `src/main.rs` — CLI のみ、runner に委譲

Runner が全エージェント呼び出しを管理する。Intent は `.forge/intents/*.yaml`、Observation は `.forge/observations.yaml`。
エージェント間データは `.forge/` ディレクトリの YAML ファイルで受け渡し。

エージェント構成の詳細は [docs/agents.md](docs/agents.md)、パイプラインフローは [docs/runner.md](docs/runner.md) を参照。

## Config

`pfl-forge.yaml` をリポジトリルートに配置。CWD ベースの単一リポモデル。Intent ID はファイル名 stem。

## CLI subcommands

- `run` — Intent 処理（柔軟 Flow 対応）
- `watch` — daemon モードでポーリング
- `status` — 処理状態の表示
- `clean` — 完了済み worktree の削除
- `create "<title>" "<body>"` — Intent draft 作成
- `parent` — Operator Agent (interactive Claude Code session) を起動
- `audit [path]` — コードベース監査 → Observation 記録
- `inbox` — 承認待ち Intent の一覧
- `approve <ids>` — Intent の承認

## Development

```sh
cargo build
cargo test
```

## Key conventions

- Implement Agent は `claude -p --allowedTools --append-system-prompt` で起動（`--dangerously-skip-permissions` は使わない）
- Operator Agent は `claude --append-system-prompt --allowedTools Bash` + `exec()` で起動
- `env_remove("CLAUDECODE")` で nested Claude Code 呼び出しを有効化
- Git worktree でワーカー間のファイルシステム隔離
- エージェント間データは `.forge/` ディレクトリの YAML ファイルで受け渡し（プロンプト埋め込みではなくファイル経由）
- Intent: `.forge/intents/*.yaml` に定義
- コミット前に、変更が CLAUDE.md、README.md や docs/ の記述と矛盾しないか確認し、必要なら同じコミットで更新すること
- 作業前に `.tmp/TODO.md` を確認し、関連する課題があれば意識すること
