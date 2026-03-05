# pfl-forge

ローカルの Intent YAML を Claude Code エージェントで自動処理するマルチエージェントシステム。

Intent（やりたいこと）を YAML で記述し、`pfl-forge run` を実行すると、分析・実装・レビューのパイプラインを自動で回す。Git worktree による隔離で、複数の Intent を安全に並列処理できる。

## 前提条件

- **Rust** (stable) — ビルドに必要
- **Claude Code CLI** (`claude`) — インストール・認証済みであること。pfl-forge は内部で `claude -p` を呼び出す
- **Git** — worktree 操作に必要

## インストール

```sh
git clone https://github.com/nesso-pfl/pfl-forge
cd pfl-forge
cargo build --release
# バイナリ: target/release/pfl-forge
```

パスの通った場所にコピーするか、`cargo install --path .` でインストールする。

起動時に GitHub Releases から新しいバージョンがあれば自動更新される。

## クイックスタート

```sh
cd /path/to/your-repo

# 1. 初期化（pfl-forge.yaml と .forge/ を作成）
pfl-forge init

# 2. pfl-forge.yaml を環境に合わせて編集
$EDITOR pfl-forge.yaml

# 3. Operator Agent を起動（対話的に操作）
pfl-forge
```

`pfl-forge`（サブコマンド省略）で起動する Operator Agent が主要な操作インターフェース。Intent の作成・承認・実行をすべて対話的に行える。

個別コマンドを直接使う場合:

```sh
# Intent を作成
pfl-forge draft "ログインバリデーションの修正" "メールアドレスの形式チェックを追加する"

# 承認待ち一覧を確認・承認・実行
pfl-forge inbox
pfl-forge approve fix-login-validation
pfl-forge run
```

## 基本概念

### Intent

「やりたいこと」の単位。`.forge/intents/<id>.yaml` に保存される。ID はファイル名の stem。

Intent はステータスに従って処理される:

```
proposed → approved → implementing → done
                                   → blocked  (一部失敗)
                                   → error    (全失敗)
```

`proposed` の Intent は人間が `approve` するまで処理されない。

### パイプライン

```
[analyze] → [implement] → [rebase] → [review]
                 ↑                       │
                 └─── rejected ──────────┘  (max_review_retries まで)
```

1. **Analyze** — Intent を読み、コードベースを調査し、実装タスクを生成する
2. **Implement** — Git worktree 内で Claude Code がコードを書き、コミットする
3. **Rebase** — worktree のブランチを base_branch に自動リベースする
4. **Review** — diff を5つの観点でレビューし、approve/reject を判定する。reject 時は feedback 付きで Implement に戻す

Analyze が情報不足と判断した場合は `needs_clarification` となり、`inbox` に表示される。`answer` で回答すると処理が再開する。

### Observation

エージェントがコードベースから発見した知見。`.forge/observations.yaml` に蓄積される。Reflect Agent がこれを読み、改善提案の Intent を自動生成する。

## CLI リファレンス

グローバルオプション: `-c, --config <PATH>` — 設定ファイルのパス（デフォルト: `pfl-forge.yaml`）

### `init`

CWD に `pfl-forge.yaml`（テンプレート）と `.forge/intents/`、`.forge/intent-drafts/` を作成する。既に `pfl-forge.yaml` が存在する場合はエラー。

```sh
pfl-forge init
```

### `run`

承認済み（`approved` / `implementing`）の Intent をパイプラインで処理する。

```sh
pfl-forge run
pfl-forge run --dry-run    # 分析のみ、実装しない
```

`--dry-run` は Analyze Agent だけ実行し、タスク分割の結果を確認できる。

処理が中断された場合、次回の `run` で `sessions` と成果物から自動再開する。

### `watch`

daemon モードで定期的に Intent をポーリングし、自動処理する。

```sh
pfl-forge watch
```

ポーリング間隔は `poll_interval_secs`（デフォルト: 300秒）で設定。

### `status`

全 Intent の ID・ステータス・タイトルを一覧表示する。

```sh
pfl-forge status
```

### `inbox`

人間のアクションが必要な Intent を表示する。`proposed`、`blocked`、`error`、未回答の clarification がある Intent が対象。

```sh
pfl-forge inbox
```

各 Intent の ID、ステータス、リスク、ソース、タイトル、未回答の質問が表示される。

### `approve <ids>`

Intent を承認して処理対象にする。カンマ区切りで複数指定可能。

```sh
pfl-forge approve fix-login
pfl-forge approve "fix-login,add-auth,update-docs"
```

### `answer <id> "<answer>"`

Clarification（Analyze Agent からの質問）に回答する。全ての質問に回答すると自動的に `approved` になる。

```sh
pfl-forge answer fix-login "RFC 5322 準拠のチェックで"
```

### `create "<title>" "<body>"`

Intent YAML を `.forge/intents/` に直接作成する。`source: human`、`status: proposed` で保存される。ID はタイトルから自動生成（slug 化）。

```sh
pfl-forge create "認証機能の追加" "OAuth2 による認証を実装する"
```

### `draft "<title>" "<body>"`

Intent ドラフト（Markdown）を `.forge/intent-drafts/` に作成する。`run` 実行時に自動で Intent YAML に変換される。`create` との違いは、ドラフトは Markdown 形式で保存され、`type` や `risk` を後から frontmatter で追加編集できる点。

```sh
pfl-forge draft "認証機能の追加" "OAuth2 による認証を実装する"
# → .forge/intent-drafts/add-auth.md
```

### `operator`

Operator Agent（対話型の Claude Code セッション）を起動する。pfl-forge のコンテキストを持った状態で対話的に操作できる。サブコマンド省略時のデフォルト動作。

```sh
pfl-forge                        # サブコマンド省略でも起動
pfl-forge operator
pfl-forge operator --model opus  # モデルを指定
```

### `audit [path]`

コードベースを監査し、Observation として記録する。

```sh
pfl-forge audit           # リポジトリ全体
pfl-forge audit src/auth  # 特定のパスのみ
```

結果は `.forge/observations.yaml` に書き込まれ、標準出力にも表示される。

### `clean`

`done` ステータスの Intent に対応する Git worktree を削除する。

```sh
pfl-forge clean
```

### `eval <agent>`

プロンプト評価フレームワーク。`evals/` 以下のフィクスチャを実行してエージェントの出力品質を検証する。

```sh
pfl-forge eval analyze
pfl-forge eval review
pfl-forge eval review --fixture edge-case    # 特定のフィクスチャのみ
```

フィクスチャが1つでも失敗すると exit code 1 で終了する。

## 設定ファイル

`pfl-forge.yaml` をリポジトリルートに配置する。全フィールドにデフォルト値があり、省略可能。

```yaml
# ブランチ・並列数
base_branch: main              # リベース・マージ先のブランチ (default: main)
parallel_workers: 4            # 最大並列 Intent 処理数 (default: 4)

# エージェントごとのモデル
models:
  analyze: opus                # Analyze Agent (default: opus)
  implement: sonnet            # Implement Agent — low/med 複雑度 (default: sonnet)
  implement_complex: opus      # Implement Agent — high 複雑度 (default: opus)
  review: sonnet               # Review Agent (default: sonnet)
  reflect: sonnet              # Reflect Agent (default: sonnet)
  skill: sonnet                # Skill Agent (default: sonnet)
  audit: opus                  # Audit Agent (default: opus)

# エージェントに許可するツール
implement_tools:               # Implement Agent 用
  - Bash
  - Read
  - Write
  - Edit
  - Glob
  - Grep

analyze_tools:                 # Analyze / Audit Agent 用
  - Read
  - Glob
  - Grep
  - Bash
  - WebSearch
  - WebFetch

# タイムアウト・リトライ
worker_timeout_secs: 1200      # Implement Agent のタイムアウト秒 (default: 1200)
analyze_timeout_secs: 600      # Analyze/Audit Agent のタイムアウト秒 (default: 600)
max_review_retries: 2          # レビュー reject 時の再実装最大回数 (default: 2)

# Worktree
worktree_dir: .pfl-worktrees   # worktree の作成先 (default: .pfl-worktrees)

# worktree 作成後、Implement Agent 起動前に実行するコマンド
# worktree_setup:
#   - npm install
#   - npm run generate-api-client

# daemon モード
poll_interval_secs: 300        # watch のポーリング間隔秒 (default: 300)

# MCP
mcp_config: .claude/mcp.json   # MCP 設定ファイルのパス (optional)
```

## Intent YAML の書き方

### 直接作成

`.forge/intents/<id>.yaml` を手動で作成する:

```yaml
title: "ログインバリデーションの修正"
body: |
  現状のバリデーションがメールアドレスの形式チェックを行っていない。
  RFC 5322 準拠のチェックを追加する。
type: fix              # feature | refactor | fix | test | audit | skill_extraction
source: human          # human | reflection
risk: low              # low | med | high
status: proposed       # proposed で作成し、approve で承認する
```

**type の種類:**

| type | 用途 |
|------|------|
| `feature` | 新機能の追加 |
| `fix` | バグ修正 |
| `refactor` | リファクタリング |
| `test` | テストの追加・修正 |
| `audit` | コードベース監査（専用パイプライン） |
| `skill_extraction` | パターン抽出 → `.claude/skills/` に保存 |

**risk の意味:**

| risk | 説明 |
|------|------|
| `low` | 小規模な変更、影響範囲が限定的 |
| `med` | 中規模、複数ファイルにまたがる |
| `high` | 大規模、アーキテクチャに影響する。Implement に opus が使われる |

### Markdown ドラフトから作成

`.forge/intent-drafts/<id>.md` に Markdown で記述すると、`run` 実行時に自動で Intent YAML に変換される。

```markdown
---
type: feature
risk: low
---

認証機能の追加

OAuth2 による認証を実装する。
Google と GitHub のプロバイダーに対応すること。
```

1段落目がタイトル、2段落目以降が本文になる。frontmatter の `type` と `risk` は省略可能。

### Clarification（質問）への対応

Analyze Agent が情報不足と判断すると、Intent に clarification が追加される:

```yaml
clarifications:
  - question: "RFC 5322 準拠？それとも簡易チェック？"
    answer: null    # null = 未回答
```

`inbox` で確認し、`answer` で回答する:

```sh
pfl-forge inbox
pfl-forge answer fix-login "RFC 5322 準拠で"
```

全ての質問に回答すると自動的に `approved` になり、次回の `run` で処理される。

### 依存関係

Intent 間の依存関係を `depends_on` で指定できる。依存先が `done` になるまで処理を待つ。

```yaml
depends_on:
  - setup-database
  - add-user-model
```

## ディレクトリ構造

```
your-repo/
  pfl-forge.yaml                    # 設定ファイル
  .forge/
    intents/                        # Intent YAML（1ファイル = 1 Intent）
      fix-login-validation.yaml
      add-auth-feature.yaml
    intent-drafts/                  # Markdown ドラフト（run 時に自動変換）
      my-feature.md
    observations.yaml               # エージェントの発見・知見（append-only）
    knowledge/
      history/                      # 完了した Intent の履歴
        fix-login-validation.yaml
      logs/                         # 実行サマリー（Reflect Agent 用）
        fix-login-validation.yaml
  .pfl-worktrees/                   # Git worktree（Intent ごとに隔離）
    forge-fix-login-validation/
```

## 並列処理

`parallel_workers` で指定した数まで Intent を並列処理する。各 Intent は独立した Git worktree で実行されるため、ファイルシステムの競合は起きない。

Analyze Agent は他のアクティブな Intent の情報を受け取り、依存関係の検出や競合の回避を行う。

## レジュームと障害復旧

- `run` が中断された場合、Intent の `sessions` が YAML に保存されている
- 次回の `run` で `.forge/tasks/{intent-id}.yaml`、worktree の存在、`sessions`、`clarifications` から再開箇所を導出する
- Claude Code のセッション ID が保存されているため、文脈を維持したまま再開できる

## ログ

`RUST_LOG` 環境変数でログレベルを制御する:

```sh
RUST_LOG=info pfl-forge run      # デフォルト
RUST_LOG=debug pfl-forge run     # 詳細ログ
```

## エージェント構成

| エージェント | 役割 | デフォルトモデル |
|-------------|------|----------------|
| Analyze | Intent の分析・タスク分割 | opus |
| Implement | worktree 内でコード実装・コミット | sonnet（high 複雑度は opus） |
| Review | diff の5観点レビュー | sonnet |
| Reflect | 完了後の振り返り・改善 Intent 生成 | sonnet |
| Audit | コードベース監査 | opus |
| Skill | パターン抽出 → SKILL.md 生成 | sonnet |
| Operator | 対話型セッション（`operator` コマンド、デフォルト） | 設定可能 |

## 典型的なワークフロー

### 日常的な使い方

```sh
# 朝: コードベースを監査
pfl-forge audit

# 手動で Intent を追加
pfl-forge create "エラーハンドリングの改善" "API レスポンスのエラー処理を統一する"

# 受信箱を確認して承認
pfl-forge inbox
pfl-forge approve "error-handling-improvement"

# 実行して放置
pfl-forge run
```

### daemon モード

```sh
# バックグラウンドで常駐
pfl-forge watch

# 別ターミナルで Intent を追加・承認するだけ
pfl-forge create "..." "..."
pfl-forge approve "..."
# → 自動的に処理される
```

### 完了後のクリーンアップ

```sh
pfl-forge status    # done になったことを確認
pfl-forge clean     # worktree を削除
```

## Docs

実装の詳細については以下のドキュメントを参照:

- [Architecture](docs/architecture.md) — 全体像・レイヤー構成・設計思想
- [Agents](docs/agents.md) — 各エージェントの責務・入出力
- [Runner](docs/runner.md) — Flow 実行の仕組み・ステップ定義
- [Data Model](docs/data-model.md) — Intent / Task / Observation の YAML スキーマ
- [Testing](docs/testing.md) — テスト戦略
