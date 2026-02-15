# pfl-forge New Architecture

pfl-forge を「タスク実行エンジン」から「自律開発パートナー」へ再設計する。

## 使用前提条件

- 対象リポジトリに **pre-commit hook** を設定し、コミット時に静的検査を実行すること
  - テスト、リント、フォーマットなど
  - Implement Agent のコミットが pre-commit hook で検証されるため、専用の Verify Agent は不要

## 3つのパラダイムシフト

### 1. Task → Intent + Observation

```
現在:  Task(YAML) → 固定パイプライン → Result
新:    Intent(any trigger) → 柔軟な Flow → Action[] → Learn
```

Intent のソース:
- **Human** — `.forge/tasks/` に手動作成（現行と同じ）
- **Audit** — `pfl-forge audit` で起動される監査エージェントが発見
- **Epiphany** — 実装中にエージェントが気づいた問題（`.forge/observations.yaml` に書き出し）
- **Reflection** — タスク完了後の Reflect Agent が発見

### 2. 固定パイプライン → タスク性質に応じた柔軟 Flow

タスクの種類ごとにデフォルト Flow テンプレートを持ち、実行中にルールベースで調整する。

| タスク種別 | デフォルト Flow |
|-----------|---------------|
| `refactor` | `[implement]` |
| `feature` | `[analyze, implement, review]` |
| `fix` | `[analyze, implement]` |
| `audit` | `[audit, report]` |
| `test` | `[analyze, implement]` |
| `skill_extraction` | `[observe, abstract, record]` |

### 3. ステートレス実行 → 学習する開発パートナー

- **Skills** — 繰り返しパターンをテンプレート化
- **Rules** — プロジェクト固有の規約を学習
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
Quick Classification     ← ルールベースで種別・Flow・リスク決定
  │
Execution Engine         ← Flow ステップの逐次実行 + ルールベース調整
  │
Reflect Agent            ← タスク完了後の振り返り
  │
Knowledge Base           ← Skills / Rules / History
```

---

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

---

## Quick Classification

AI エージェントではなく、決定論的なルール。ラベル・キーワード・ソースから:

1. タスク種別を判定（refactor / feature / fix / test / audit）
2. デフォルト Flow テンプレートを選択
3. 初期リスクレベルを設定

---

## Execution Engine

Flow ステップを逐次実行し、各ステップの結果に応じて**ルールベース**で残りの Flow を調整する。

調整ルールの例:
- `analyze` が `complexity: high` を返す → `review` ステップを追加
- `implement` の変更が 10 行未満 → `review` をスキップ
- `review` が `rejected` を返す → `implement + review` サイクルを追加
- `analyze` が `depends_on` を返す → 依存 intent の完了まで implement を遅延

設計方針:
- ルールは Rust コード。ユニットテスト可能、予測可能、デバッグ可能
- 精度は「最初から正解する」ではなく「途中で修正できる」で担保
- 段階的拡張: ハードコードルール → 学習ベースルールの追加

全エージェントは実行中に `.forge/observations.yaml` へ気づきを書き出せる。

### 並列タスク協調

analyze 実行時に、他の active な intent の情報をコンテキストとして注入する。

注入する情報（intent ごと）:
- タイトル、ステータス（analyzing / implementing / reviewing）
- relevant_files、プラン概要（analyze 済みの場合）

これにより analyze agent は:
- 他タスクが変更予定のファイルを把握し、コンフリクトを避けた計画を立てられる
- 明確な依存関係がある場合、`depends_on: [intent-id]` を出力できる

Execution Engine は `depends_on` を確認し、該当 intent が完了するまで implement を遅延させる。

主な価値は**依存検出**にある。コンフリクト回避は副次的な効果で、発生時はコンフリクト解決のフォールバックで対処する。

### コンフリクト解決

並列ワーカーが同時に作業すると、main へのリベース時にコンフリクトが発生しうる。

解決戦略（段階的フォールバック）:

1. **`git rebase`** — 成功すればそのまま統合
2. **同じプランで再実装** — rebase abort し、updated main から worktree を再作成、analyze 結果を再利用して implement のみ再実行
3. **Failed** — 再実装でも失敗した場合は人間にエスカレート

再実装で十分な理由:
- 並列タスクは別の関心事を扱うため、プラン自体は main が進んでも有効
- pre-commit hook が通れば結果の正しさも担保される

---

## エージェント構成

| Agent | 責務 | 状態 |
|-------|------|------|
| **Analyze** | タスク分析、実装計画 | 既存（ほぼ同じ） |
| **Implement** | コード実装 + observation 書き出し | 既存（observation 追加） |
| **Review** | コードレビュー | 既存（ほぼ同じ） |
| **Audit** | コードベース監査 → Intent 生成 | **新規** |
| **Reflect** | タスク完了後の振り返り → 学習 | **新規** |
| **Orchestrate** | インタラクティブセッション | 既存（拡張） |
| ~~Architect~~ | Analyze に統合、Flow 調整で代替 | **削除** |
| ~~Verify~~ | pre-commit hook で代替 | **削除** |

### Audit Agent

`pfl-forge audit` で起動。包括的なコードベース監査を行う:
- テストカバレッジの薄い領域
- 設計品質（巨大関数、密結合、責務の混在）
- コード規約違反（プロジェクト固有ルール）
- 技術的負債（TODO、非推奨 API、重複コード）
- ドキュメントと実装の乖離

発見事項を Intent として登録する。

### Reflect Agent

各タスク完了後に実行。以下を評価:
- Flow 選択は適切だったか
- 他に気づいた問題はないか
- テンプレート化できるパターンはないか
- ルール化すべき規約はないか

出力:
- 新しい observation → Intent Registry
- Knowledge Base 更新（skills, rules, history）

### Epiphany 収集（二重アプローチ）

1. **プロンプト指示**: 全エージェントに「タスクと無関係な気づきは `.forge/observations.yaml` に書き出せ」と指示
2. **事後リフレクション**: Reflect Agent がタスク完了後に「他に何か気づいたか」を問う

両方を併用する。

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

### Rules / History（`.forge/knowledge/`）

```
.forge/knowledge/
  rules/     ← プロジェクト固有の規約
  history/   ← 実行記録
```

フォーマット: 当面は YAML。

スケール見積もり:
- Skills: プロジェクトあたり 10-50 個。Claude Code が自動注入
- Rules: 20-100 個。プロンプトに全量注入可能
- History: 無制限に増加。将来 pgvector 等への移行が必要になる可能性あり

Rules / History はインターフェースを抽象化し、バックエンド変更に備える。
各エージェントのプロンプトに関連コンテキストとして注入する。

---

## CLI サブコマンド

| コマンド | 用途 | 状態 |
|---------|------|------|
| `run` | タスク処理（柔軟 Flow 対応） | 既存・拡張 |
| `audit` | コードベース監査 → Intent 生成 | **新規** |
| `inbox` | 提案された Intent の一覧・承認・却下 | **新規** |
| `approve` | 特定 Intent の承認（例: `approve 3,5,7`） | **新規** |
| `status` | 処理状態の表示 | 既存 |
| `rules` | 学習済み Rules の閲覧・編集 | **新規** |
| `parent` | インタラクティブセッション | 既存 |
| `create` | 手動タスク作成 | 既存 |
| `clean` | worktree クリーンアップ | 既存 |
| `watch` | daemon モード | 既存 |

---

## 想定される日常フロー

```
朝:
  pfl-forge audit           → 監査実行、Intent が inbox に蓄積
  pfl-forge inbox            → 提案された Intent を確認
  pfl-forge approve 3,5,7    → 良いものを承認
  pfl-forge run              → 承認済み + 自動実行 Intent を処理
  （各タスク完了後に Reflect が走り、knowledge が成長）

作業中:
  pfl-forge create "Feature X" "details..."  → 手動タスク作成
  pfl-forge run                              → 処理実行
  （implement 中: 「ここテスト薄い」→ observation → 次回 audit で拾われる）
```

---

## AI バックエンド

**Claude CLI (`claude -p`) を継続使用。**

- ツール実行（Bash, ファイル操作）が CLI 経由で無料
- 実装・監査など重いタスクに必須
- 軽量タスク（分類、リフレクション）でのオーバーヘッドは許容
- 2つのバックエンドのメンテナンスコストを回避

---

## 未決事項

- [ ] リファクタか書き直しか — 現行コードベースへの適用方法
- [ ] `.forge/observations.yaml` のスキーマ
- [ ] 非 human Intent の YAML スキーマ
- [ ] Audit Agent のスコープとプロンプト設計
- [ ] Knowledge Base（Rules / History）のインターフェース抽象化の設計
- [ ] Rule の YAML 表現形式
- [ ] History の記録内容
- [ ] Execution Engine の Flow 調整ルールの全容（上記は例示）
