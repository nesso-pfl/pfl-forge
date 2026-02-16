# データモデル

## Intent

### フィールド

- **type**: `feature`, `refactor`, `fix`, `test`, `audit`, ...
- **source**: `human`, `audit`, `epiphany`, `reflection`
- **risk**: `low`, `med`, `high`
- **status**: `proposed` → `approved` → `executing` → `done` / `blocked` / `error`
- **parent**: 親 Intent の ID（子 Intent の場合）

ステータス遷移:

```
proposed → approved → executing → done      (全 Task 成功)
                                → blocked   (一部 Task が failed)
                                → error     (全 Task が failed)
```

`blocked` / `error` の Intent は inbox に入り、人間が失敗した Task の review feedback を確認して対応を決める（再実行・追加指示・却下）。成功した Task のコミットはそのまま保持される。

### リスクベースの自律実行

リスクレベルはエージェント自身が判定する（ハードコードされた閾値ではない）。

| リスク | 動作 |
|--------|------|
| `low` | 自動実行（人間の承認不要） |
| `med` / `high` | inbox に配置、人間の承認を待つ |

例:
- low: 小規模リファクタ、テスト追加
- high: アーキテクチャ変更、大規模設計変更

### Inbox

人間のアクションを待っている Intent のフィルタビュー。物理的な場所ではなく、`.forge/intents/` 内の Intent を条件で抽出する。

inbox に入る条件:
- **承認待ち** — リスク `med` / `high` で人間の承認が必要（status: `proposed`）
- **clarification 待ち** — 情報不足で人間の回答が必要（`needs_clarification`）
- **review 失敗** — 全リトライ後も Task が失敗（status: `blocked` / `error`）

### Intent Registry（`.forge/intents/`）

全ソースの Intent を `.forge/intents/*.yaml` に集約する。Runner はこのディレクトリだけを参照する。

ソースごとの生成経路:

| ソース | 入力 | 変換 | 生成先 |
|--------|------|------|--------|
| Human | `.forge/intent-drafts/*.md` | pfl-forge が frontmatter + body をパース | `.forge/intents/` |
| Audit | Audit Agent の発見 | Agent が直接生成 | `.forge/intents/` |
| Epiphany | Agent の気づき | Agent が action 必要と判断時に直接生成 | `.forge/intents/` |
| Reflection | Reflect Agent の発見 | Agent が直接生成 | `.forge/intents/` |

Human 入力のフォーマット（`.forge/intent-drafts/*.md`）:

```markdown
---
type: feature
labels: [ui, auth]
---

ログイン画面にパスワードリセットリンクを追加する。

現状ではパスワードを忘れたユーザーがリセットする手段がない。
```

frontmatter の `type`, `labels` は省略可能（Quick Classification が推定）。

## Task

Analyze Agent が Intent から生成する実行可能な作業単位。1 Task = 1 Implement Agent 実行。

### フィールド

- **intent_id**: 親 Intent の ID
- **title**: 作業内容の要約
- **plan**: 実装計画
- **relevant_files**: 関連ファイル一覧
- **implementation_steps**: 実装ステップ
- **context**: 補足情報
- **complexity**: `low`, `med`, `high`
- **depends_on**: 他の Task ID（同一 Intent 内の依存関係）
- **status**: `pending` → `implementing` → `done` / `failed`

### Analyze の出力パターン

Analyze は Intent を分析し、以下のいずれかを出力する:

| パターン | 条件 | 出力 |
|----------|------|------|
| Task 分解 | 実装計画を立てられる | Task[] — 各 Task が implement へ |
| Intent 分解 | 問題が大きすぎて1回の analyze では計画できない | 子 Intent[] — 各子 Intent がフルパイプライン（analyze → implement → review）を経る |
| Clarification | 情報不足 | `needs_clarification` — inbox へ |

Intent のステータスは Task の集約:
- 全 Task が `done` → Intent は `done`
- 一部 Task が `failed`（リトライ上限到達後） → Intent は `blocked`
- 全 Task が `failed` → Intent は `error`

## Review Result

Review Agent が返す構造化 JSON 出力。Runner がファイルに永続化する。

### フィールド

- **task_id**: 対象 Task の ID
- **verdict**: `approved` / `rejected`
- **issues**: 問題点（rejected の根拠）
- **suggestions**: 改善提案（approved でも出せる）

## Quick Classification

AI エージェントではなく、決定論的なルール。ラベル・キーワード・ソースから:

1. タスク種別を判定（refactor / feature / fix / test / audit）
2. デフォルト Flow テンプレートを選択
3. 初期リスクレベルを設定
