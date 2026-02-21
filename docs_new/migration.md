# 現行実装からの変更点

## エージェント構成

| 項目 | 現行 | 新アーキ |
|------|------|----------|
| Architect Agent | Analyze が不十分な場合にエスカレート | 削除。Analyze 内で完結（`needs_clarification` で inbox へ） |
| Verify Agent | 実装後のテスト検証 | 削除。pre-commit hook で代替 |
| Analyze の起動 | `process_task()` から直接呼び出し | Runner が Flow ステップとして実行 |
| エージェント間データ | `AnalysisResult` / `ReviewResult` 型、`.forge/work/*.yaml` / `.forge/task.yaml` / `.forge/review.yaml` | 詳細は実装時に決定 |
| Review リトライ | `max_review_retries` 設定キー | Runner の Flow 調整ルールとして管理 |
| モデル設定キー | リネーム済み: `models.analyze` | — |
| ツール設定キー | リネーム済み: `analyze_tools`。`review_tools` は未分離 | エージェントごとに分離 |
| Implement リトライ | 毎回新プロセスで再起動 | `--resume` でセッション継続（トークン節約） |
| Analyze 再実行 | clarification 回答後に新プロセスで再起動 | `--resume` でセッション継続 |
| Reflect の成果物 | Knowledge Base を直接更新 | Intent を生成し、通常のパイプライン（Analyze → Implement → Review）で更新 |

## データモデル

### 使い回せる型

**`work::Task` → Task**

大部分のフィールドがそのまま対応する。

| フィールド | 現行 | 新アーキ | 差分 |
|-----------|------|----------|------|
| `id` | `String` | あり | そのまま |
| `title` | `String` | あり | そのまま |
| `body` | `String` | — | 削除（body は Intent 側） |
| `plan` | `String` | あり | そのまま |
| `relevant_files` | `Vec<String>` | あり | そのまま |
| `implementation_steps` | `Vec<String>` | あり | そのまま |
| `context` | `String` | あり | そのまま |
| `complexity` | `String` | あり | そのまま |
| `status` | `WorkStatus` | あり | バリアント微調整済み（`Implementing`） |
| — | — | `intent_id` | 追加（親 Intent への参照） |
| — | — | `depends_on` | 追加（同一 Intent 内の Task 間依存） |

**`ReviewResult` → Review Result**

| フィールド | 現行 | 新アーキ | 差分 |
|-----------|------|----------|------|
| `approved` | `bool` | — | `verdict: approved/rejected` enum に変更 |
| `issues` | `Vec<String>` | あり | そのまま |
| `suggestions` | `Vec<String>` | あり | そのまま |
| — | — | `task_id` | 追加 |

**`AnalysisResult`（中間型として存続）**

フィールド（`complexity`, `plan`, `relevant_files`, `implementation_steps`, `context`）が Task にそのまま流れるため、Analyze Agent の JSON 出力パース用の中間型として引き続き使える。

### 新規作成

**Intent** — `Issue` とは根本的に異なる概念。`Issue` はタスク入力のコンテナ（`id`, `title`, `body`, `labels`）だが、Intent はライフサイクル管理（`type`, `source`, `risk`, `status`, `parent`）を含む。

### 廃止

| 型 | 理由 |
|----|------|
| `ArchitectOutcome` | Architect Agent 削除に伴い不要 |
| `TaskStatus`（state） | Intent のステータスモデル（`proposed` → `approved` → `executing` → `done`）に置き換え |
| `ClarificationContext` / `PendingClarification` | `needs_clarification` が Intent の一時停止になるため再設計 |

## アーキテクチャ

### パラダイムシフト

| 現行 | 新アーキ |
|------|----------|
| `Task(YAML)` → 固定パイプライン → `Result` | `Intent(any trigger)` → 柔軟な Flow → `Action[]` → Learn |
| 固定パイプライン（fetch → analyze → implement → review） | タスク種別に応じた Flow テンプレート + ルールベース調整 |
| ステートレス実行（毎回ゼロから） | Knowledge Base（Skills / History）で学習を蓄積 |

### CLI サブコマンド

| コマンド | 状態 | 変更内容 |
|---------|------|----------|
| `run` | 拡張 | 柔軟 Flow 対応 |
| `create` | 変更 | `.forge/intent-drafts/` に Markdown 作成 |
| `audit` | **新規** | コードベース監査 → Observation 記録 |
| `inbox` | **新規** | 提案された Intent の一覧・承認・却下 |
| `approve` | **新規** | 特定 Intent の承認 |
| `status` / `parent` / `clean` / `watch` | 既存 | 変更なし |
| `clarifications` / `answer` | **廃止** | `needs_clarification` が Intent の一時停止 + inbox に統合 |

### 未決事項（移行関連）

- リファクタか書き直しか — 現行コードベースへの適用方法
