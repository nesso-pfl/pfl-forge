# エージェント構成

pfl-forge は複数の Claude Code エージェントを使い分けて Intent を処理する。各エージェントの呼び出しロジック（プロンプト組み立て・CLI 実行・出力パース）は `src/agents/` に、system prompt は `src/prompt/*.md` に定義されている。

| Agent | 責務 |
|-------|------|
| **Analyze** | Intent 分析、実装計画 |
| **Implement** | コード実装 + observation 書き出し |
| **Review** | コードレビュー |
| **Audit** | コードベース監査 → Intent 生成 |
| **Reflect** | Intent 完了後の振り返り → 学習 |
| **Orchestrate** | インタラクティブセッション |

---

## Analyze Agent

### 概要

Intent の詳細分析を行う読み取り専用エージェント。`claude -p` で非対話実行。

### 起動タイミング

Execution Engine が Flow の `analyze` ステップを実行するとき。

### 入力コンテキスト

- Intent（[data-model.md](data-model.md) 参照）
- Clarification 回答（再実行時）
- Project Rules（プロンプト注入）
- Decision Storage（プロンプト注入）
- 関連する History
- 他の active な intent の情報（タイトル、ステータス、relevant_files、プラン概要）

### 処理内容

- コードベースを探索し、Intent を実行可能な単位に分解する
- 実装計画を立てられる → Task[] に分解（各 Task が implement へ）
- 問題が大きすぎる → 子 Intent[] に分解（各子が再び analyze から開始）
- 情報不足 → `needs_clarification` を返す
- モデル: `models.triage_deep`（default: opus）
- ツール: `triage_tools`（default: Read, Glob, Grep, Bash, WebSearch, WebFetch）

### 成果物

- Task[]（[data-model.md](data-model.md) 参照）または子 Intent[]

### Flow 調整への影響

- `complexity: high` → `review` ステップを追加
- `needs_clarification` → intent を一時停止し inbox へ
- `depends_on` → 依存 intent の完了まで implement を遅延

---

## Implement Agent

### 概要

コード変更を行うエージェント。Git worktree 内で動作。`claude -p --allowedTools --append-system-prompt` で起動。

### 起動タイミング

Analyze が Task を生成した後、Execution Engine が worktree を作成し Task ファイルを配置して実行。Review で rejected の場合はフィードバック付きで再実行。

### 入力コンテキスト

- worktree 内の Task ファイル（plan, relevant_files, implementation_steps, context）
- Review feedback（リトライ時）
- Skills（Claude Code が自動注入）
- Project Rules（プロンプト注入）

### 処理内容

- Task に従い実装を行い、コミットを作成
- モデル: complexity に応じて `models.default`（low/medium）または `models.complex`（high）
- ツール: `worker_tools`（default: Bash, Read, Write, Edit, Glob, Grep）
- 実行中の気づきを `.forge/observations.yaml` に書き出し可

### 成果物

- Git コミット（worktree 内）
- 成功判定: コミット数 > 0

### Flow 調整への影響

- 変更 10 行未満 → `review` をスキップ

---

## Review Agent

### 概要

Implement Agent の成果物を検証するコードレビューエージェント。`claude -p` で非対話実行。

### 起動タイミング

Implement 成功 + rebase 成功後。

### 入力コンテキスト

- Task 定義（plan）
- base branch との diff
- Skills（Claude Code が自動注入）
- Project Rules（プロンプト注入）

### 処理内容

- 5 つの検証基準でレビュー: 要件充足、パターン準拠、バグ/セキュリティ、計画整合性、テスト品質
- モデル: `models.default`（default: sonnet）
- ツール: `triage_tools`（default: Read, Glob, Grep）

### 成果物

- レビュー結果（approved/rejected, issues, suggestions）

### Flow 調整への影響

- `rejected` → implement + review サイクルを追加（設定上限まで）
- 全リトライ後も rejected → Error 状態

---

## Audit Agent

### 概要

包括的なコードベース監査を行うエージェント。`pfl-forge audit` サブコマンドで起動。

### 起動タイミング

ユーザーが `pfl-forge audit` を実行したとき。

### 入力コンテキスト

- History（傾向分析）
- Skills / Rules（規約違反チェック）

### 処理内容

- テストカバレッジの薄い領域の検出
- 設計品質の評価（巨大関数、密結合、責務の混在）
- コード規約違反のチェック（プロジェクト固有ルール）
- 技術的負債の検出（TODO、非推奨 API、重複コード）
- ドキュメントと実装の乖離の検出

### 成果物

- `.forge/intents/` に Intent を生成
- `.forge/observations.yaml` に observation を書き出し可

---

## Reflect Agent

### 概要

Intent 完了後の振り返りを行い、Knowledge Base を更新する学習エージェント。

### 起動タイミング

各 Intent 完了後に自動実行。

### 入力コンテキスト

- History（Before/After 分析）
- Observation（横断分析）

### 処理内容

- Flow 選択が適切だったかの評価
- パターン検出（テンプレート化できる繰り返しパターン）
- 規約化判断（ルール化すべき規約の特定）
- Rule の有効性検証（applied_to 履歴からの傾向分析）

### 成果物

- Knowledge Base 更新: Skills / Rules の生成・更新・剪定
- Intent Registry への昇格（observation → intent）

---

## Orchestrate Agent

### 概要

ユーザーとの対話窓口となるインタラクティブセッション。`claude --append-system-prompt --allowedTools Bash` + `exec()` で起動。

### 起動タイミング

ユーザーが `pfl-forge parent` を実行したとき。

### 入力コンテキスト

- State サマリ
- Pending clarification 一覧

### 処理内容

- `pfl-forge run/status/clarifications/answer/create/audit/inbox/approve` 等のサブコマンドを Bash 経由で実行
- NeedsClarification が発生した場合、ユーザーに質問を提示し回答を記録

### 成果物

- ユーザーインタラクション（直接的なファイル出力なし）

---

## Epiphany 収集

全エージェントが実行中に当該 Intent と無関係な気づきを記録できる二重アプローチ:

1. **プロンプト指示**: 全エージェントに「Intent と無関係な気づきは `.forge/observations.yaml` に書き出せ」と指示
2. **事後リフレクション**: Reflect Agent が Intent 完了後に「他に何か気づいたか」を問う

生成ルール:
- action が必要 → `.forge/intents/` に intent を直接生成（observation は書かない）
- action 不要だが記録に値する → `.forge/observations.yaml` に observation のみ

これにより observation は常に「未処理」であり、Reflect Agent は全件を評価対象にできる。

## エージェントと Knowledge Base の関係

| Agent | History | Observation | Skills / Rules | Decision Storage |
|-------|---------|-------------|----------------|-----------------|
| **Analyze** | — | 書き出し可 | 参照（プロンプト注入） | 参照（プロンプト注入） |
| **Implement** | — | 書き出し可 | 参照（プロンプト注入） | — |
| **Review** | — | 書き出し可 | 参照（プロンプト注入） | — |
| **Audit** | 傾向分析に参照 | 書き出し可 | 参照 + 規約違反チェック | — |
| **Reflect** | Before/After 分析 | 横断分析 | 生成・更新・剪定 | — |
| **Execution Engine** | 自動記録（全件） | — | — | — |

- **History の記録主体は Execution Engine**。各 agent がステップ結果と所要時間を意識する必要はない
- **Observation の記録主体は各 agent**。実行中に気づいた摩擦や問題を `.forge/observations.yaml` に書き出す
- **Reflect Agent が両方を突き合わせてパターンを検出**し、Skills / Rules への昇格や剪定を判断する

---

## 現行実装からの変更点

| 項目 | 現行 | 新アーキ |
|------|------|----------|
| Architect Agent | Analyze が不十分な場合にエスカレート | 削除。Analyze 内で完結（`needs_clarification` で inbox へ） |
| Verify Agent | 実装後のテスト検証 | 削除。pre-commit hook で代替 |
| Analyze の起動 | `process_task()` から直接呼び出し | Execution Engine が Flow ステップとして実行 |
| エージェント間データ | `AnalysisResult` / `ReviewResult` 型、`.forge/work/*.yaml` / `.forge/task.yaml` / `.forge/review.yaml` | 詳細は実装時に決定 |
| Review リトライ | `max_review_retries` 設定キー | Execution Engine の Flow 調整ルールとして管理 |
