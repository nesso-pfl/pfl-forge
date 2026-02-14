# pfl-forge

ローカルタスク YAML を Claude Code Worker で自動処理するマルチエージェントシステム。

## Getting Started

対象リポジトリのルートで `pfl-forge.yaml` を配置し、`.forge/tasks/` にタスク YAML を作成してから `pfl-forge run` を実行する。

```sh
cd /path/to/your-repo
cp pfl-forge.yaml.example pfl-forge.yaml  # 設定を編集
pfl-forge run
```

pfl-forge はリポジトリルート（CWD）単位で動作する。タスク ID はファイル名（UUID 等の任意文字列）で、状態は `.forge/state.yaml` に保存される。

## Usage

```sh
# タスク処理の実行
pfl-forge run

# 親エージェント (interactive) で操作
pfl-forge parent

# 状態確認
pfl-forge status

# 未回答の clarification を確認・回答
pfl-forge clarifications
pfl-forge answer my-task-id "Use OAuth2 for authentication"

# daemon モード
pfl-forge watch
```

## Configuration

`pfl-forge.yaml` をリポジトリルートに配置する。設定例は [pfl-forge.yaml.example](pfl-forge.yaml.example) を参照。

## Pipeline

```
fetch → deep triage → (consultation) → execute → integrate → report
                          ↓
                   NeedsClarification
                          ↓
                   parent agent が
                   ユーザーに質問
```

## Docs

- [Agent 構成](docs/agents.md) — 各エージェントの役割・モデル・ツール
