# pfl-forge

GitHub issue を Claude Code Worker で自動処理するマルチエージェントシステム。

## Usage

```sh
# issue 処理の実行
pfl-forge run

# 親エージェント (interactive) で操作
pfl-forge parent

# 状態確認
pfl-forge status

# 未回答の clarification を確認・回答
pfl-forge clarifications
pfl-forge answer 42 "Use OAuth2 for authentication"

# daemon モード
pfl-forge watch
```

## Configuration

`pfl-forge.yaml` で対象リポジトリと設定を定義する。

```yaml
repos:
  - name: my-app
    path: /home/user/repos/my-app
    github: owner/my-app
    test_command: cargo test
    issue_label: forge

settings:
  parallel_workers: 4
  models:
    triage_deep: sonnet
    default: sonnet
    complex: opus
```

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
