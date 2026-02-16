# データモデル

## Intent

### フィールド

- **type**: `feature`, `refactor`, `fix`, `test`, `audit`, ...
- **source**: `human`, `audit`, `epiphany`, `reflection`
- **risk**: `low`, `med`, `high`
- **status**: `proposed` → `approved` → `executing` → `done`

### リスクベースの自律実行

リスクレベルはエージェント自身が判定する（ハードコードされた閾値ではない）。

| リスク | 動作 |
|--------|------|
| `low` | 自動実行（人間の承認不要） |
| `med` / `high` | inbox に配置、人間の承認を待つ |

例:
- low: 小規模リファクタ、テスト追加
- high: アーキテクチャ変更、大規模設計変更

### Intent Registry（`.forge/intents/`）

全ソースの Intent を `.forge/intents/*.yaml` に集約する。Execution Engine はこのディレクトリだけを参照する。

ソースごとの生成経路:

| ソース | 入力 | 変換 | 生成先 |
|--------|------|------|--------|
| Human | `.forge/tasks/*.md` | pfl-forge が frontmatter + body をパース | `.forge/intents/` |
| Audit | Audit Agent の発見 | Agent が直接生成 | `.forge/intents/` |
| Epiphany | Agent の気づき | Agent が action 必要と判断時に直接生成 | `.forge/intents/` |
| Reflection | Reflect Agent の発見 | Agent が直接生成 | `.forge/intents/` |

Human 入力のフォーマット（`.forge/tasks/*.md`）:

```markdown
---
type: feature
labels: [ui, auth]
---

ログイン画面にパスワードリセットリンクを追加する。

現状ではパスワードを忘れたユーザーがリセットする手段がない。
```

frontmatter の `type`, `labels` は省略可能（Quick Classification が推定）。

## Quick Classification

AI エージェントではなく、決定論的なルール。ラベル・キーワード・ソースから:

1. タスク種別を判定（refactor / feature / fix / test / audit）
2. デフォルト Flow テンプレートを選択
3. 初期リスクレベルを設定
