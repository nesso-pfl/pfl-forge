# pfl-forge Architecture

自律開発パートナー。Intent を受け取り、柔軟な Flow で処理し、学習を蓄積する。

## 使用前提条件

- 対象リポジトリに **pre-commit hook** を設定し、コミット時に静的検査を実行すること
  - テスト、リント、フォーマットなど
  - Implement Agent のコミットが pre-commit hook で検証されるため、専用の Verify Agent は不要

## コア概念

### Intent

あらゆるトリガーから生成される作業単位。

ソース:
- **Human** — `.forge/intent-drafts/*.md` に Markdown で作成 → pfl-forge が `.forge/intents/` に変換
- **Reflection** — Reflect Agent が Observation を評価し `.forge/intents/` に生成

エージェントの気づき（Epiphany / Audit）は全て `.forge/observations.yaml` に記録され、Reflect が Intent 化するか判断する。

### 柔軟 Flow

タスクの種類ごとにデフォルト Flow テンプレートを持ち、実行中にルールベースで調整する。

通常タスク（feature / fix / refactor / test）は全て `[analyze, implement, review]` で処理する。種別の差はフローではなく、各エージェントへのコンテキスト注入で吸収する。

| 種別 | Flow |
|------|------|
| 通常タスク | `[analyze, implement, review]` |
| `audit` | `[audit, report]` |
| `skill_extraction` | `[observe, abstract, record]` |

### Knowledge Base

実行から学習を蓄積する。

- **Skills** — 繰り返しパターンをテンプレート化
- **History** — 成功・失敗・リジェクトの履歴を蓄積

```
Observe/Audit → Discovery → Risk Assessment → 自律実行 or 提案
     ^                                              |
     +──── Learning / Pattern Accumulation ←────────+
```

---

## レイヤー構成

```
CLI Layer
  │
Intent Registry          ← Intent の登録・管理
  │
Runner                   ← Flow ステップの逐次実行 + ルールベース調整
  │
Reflect Agent            ← タスク完了後の振り返り
  │
Knowledge Base           ← Skills / History
```

---

## Runner

Flow ステップの逐次実行とルールベースの Flow 調整を行う。AI エージェントではなく Rust コードによる制御ロジック。

全エージェントは実行中に `.forge/observations.yaml` へ気づきを書き出せる。

詳細は [runner.md](runner.md) を参照。

---

## Knowledge Base

二系統で管理する:

### Skills（`.claude/skills/`）

Claude Code のネイティブ skill 機能をそのまま活用する。

```
.claude/skills/
  <name>/SKILL.md    ← YAML frontmatter + markdown（Claude Code 標準フォーマット）
```

- Claude Code が description ベースで関連スキルを自動注入する
- Implement Agent（`claude -p`）でも自動的にスキルが読み込まれる
- 自前のプロンプト注入の仕組みは不要

### History（`.forge/knowledge/history/`）

フォーマット: 当面は YAML。無制限に増加するため、将来 pgvector 等への移行が必要になる可能性あり。

History エントリのフィールド:
- intent メタデータ（type, source, risk, title）
- 実行された Flow（ステップ一覧 + 調整内容）
- 各ステップの結果と所要時間
- 最終結果（success / failed / escalated）+ 失敗理由
- 生成された observation への参照
- タイムスタンプ

History は「構造化されたサマリ」。agent 内部の操作ログ（個別ファイル読み込み等）は記録しない。
プロセスの摩擦や困難は Observation が担う。

プロジェクト固有の規約は CLAUDE.md と `.claude/skills/` で管理する。全エージェントが `claude -p` で起動するため自動的に読み込まれ、Runner による注入は不要。規約の追加・更新は Reflect が Intent を生成し、通常の Implement → Review フローで CLAUDE.md を編集する。

### Decision Storage（外部連携）

プロジェクト横断の個人的な判断基準・設計思想を保持する外部アプリ。

- CLAUDE.md / Skills = 「このプロジェクトではこうする」（プロジェクト固有）
- Decision Storage = 「自分はこう考える」（プロジェクト横断）

MCP Server 経由で接続する。Runner がプロンプトに事前注入するのではなく、エージェントが実行中に必要に応じて MCP ツールで検索・取得する。Runner は MCP 設定（`--mcp-config`）を渡すだけ。

理由: Runner は Intent の title/body しか持たず適切な検索クエリを組み立てられない。エージェント（特に Analyze）がコードベース探索中に背景情報を必要とした瞬間に取得するのが自然。

History のバックエンドは当面 YAML ファイル。スケール問題が顕在化した時点で抽象化を検討する。

---

## CLI サブコマンド

| コマンド | 用途 |
|---------|------|
| `run` | タスク処理（柔軟 Flow 対応） |
| `audit` | コードベース監査 → Observation 記録 |
| `inbox` | 提案された Intent の一覧・承認・却下 |
| `approve` | 特定 Intent の承認（例: `approve 3,5,7`） |
| `status` | 処理状態の表示 |
| `parent` | インタラクティブセッション |
| `create` | `.forge/intent-drafts/` に Markdown 作成 |
| `clean` | worktree クリーンアップ |
| `watch` | daemon モード |

---

## 想定される日常フロー

```
朝:
  pfl-forge audit           → 監査実行、Observation を記録しサマリ表示
  pfl-forge inbox            → 提案された Intent を確認
  pfl-forge approve 3,5,7    → 良いものを承認
  pfl-forge run              → 承認済み + 自動実行 Intent を処理
  （各タスク完了後に Reflect が走り、knowledge が成長）

作業中:
  pfl-forge create "Feature X" "details..."  → 手動タスク作成
  pfl-forge run                              → 処理実行
  （implement 中: 「ここテスト薄い」→ observation → Reflect が拾い Intent 化）
```

---

## AI バックエンド

**Claude CLI (`claude -p`) を継続使用。**

- ツール実行（Bash, ファイル操作）が CLI 経由で無料
- 実装・監査など重いタスクに必須
- 軽量タスク（分類、リフレクション）でのオーバーヘッドは許容
- 2つのバックエンドのメンテナンスコストを回避

---

## 関連ドキュメント

- [runner.md](runner.md) — Runner の仕様・実行フロー・Flow 調整ルール
- [agents.md](agents.md) — エージェント構成・責務・Knowledge Base との関係
- [data-model.md](data-model.md) — Intent, Task 等のデータモデル定義
- [migration.md](migration.md) — 現行実装からの変更点
