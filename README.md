# pfl-forge

ローカルの Intent YAML を Claude Code エージェントで自動処理するマルチエージェントシステム。

## Getting Started

対象リポジトリのルートで `pfl-forge.yaml` を配置し、`.forge/intents/` に Intent YAML を作成してから `pfl-forge run` を実行する。

```sh
cd /path/to/your-repo
cp pfl-forge.yaml.example pfl-forge.yaml  # 設定を編集
pfl-forge run
```

pfl-forge はリポジトリルート（CWD）単位で動作する。Intent ID はファイル名の stem で、状態は各 Intent YAML 内の `status` フィールドに保存される。

## Usage

```sh
# Intent 処理の実行
pfl-forge run

# Operator Agent (interactive) で操作
pfl-forge parent

# 状態確認
pfl-forge status

# 承認待ち Intent の確認
pfl-forge inbox

# Intent の承認
pfl-forge approve <id>

# Clarification への回答（全回答で自動 approve）
pfl-forge answer <id> "Use OAuth2 for authentication"

# Intent の作成
pfl-forge create "タイトル" "本文"

# コードベース監査
pfl-forge audit [path]

# daemon モード
pfl-forge watch
```

## Pipeline

```
analyze → implement → rebase → review
            ↑                    │
            └── rejected ────────┘

analyze → needs_clarification → inbox → answer → re-analyze
```

## Docs

- [Architecture](docs/architecture.md) — 全体像・レイヤー構成・設計思想
- [Agent 構成](docs/agents.md) — 各エージェントの役割・モデル・ツール
- [Runner](docs/runner.md) — Flow 実行の仕組み・ステップ定義
- [Data Model](docs/data-model.md) — Intent / Task / Observation 等の YAML スキーマ
- [Testing](docs/testing.md) — テスト戦略
