# エージェント構成

pfl-forge は複数の Claude Code エージェントを使い分けて Intent を処理する。各エージェントの呼び出しロジック（プロンプト組み立て・CLI 実行・出力パース）は `src/agent/` に、system prompt は `src/prompt/*.md` に定義されている。

| Agent | 責務 |
|-------|------|
| **Analyze** | Intent 分析、実装計画 |
| **Implement** | コード実装 + observation 書き出し |
| **Review** | コードレビュー |
| **Audit** | コードベース監査 → Observation 記録 |
| **Reflect** | Intent 完了後の振り返り → 学習 |
| **Operator** | インタラクティブセッション |

---

## Operator Agent

ユーザーとの対話窓口となるインタラクティブセッション。`claude --append-system-prompt --allowedTools Bash` + `exec()` で起動。

### 起動タイミング

ユーザーが `pfl-forge parent` を実行したとき。

### 入力コンテキスト

- State サマリ
- Inbox（承認待ち Intent、clarification 待ち Intent）

### 処理内容

- `pfl-forge run/status/create/audit/inbox/approve` 等のサブコマンドを Bash 経由で実行
- `needs_clarification` で一時停止した Intent について、ユーザーに質問を提示し回答を記録

### 成果物

- ユーザーインタラクション（直接的なファイル出力なし）

---

## 非対話エージェント共通仕様

以下は Analyze, Implement, Review, Audit, Reflect に共通する仕様:

- **起動**: `claude -p --allowedTools <tools> --append-system-prompt <prompt> --model <model> --output-format json`
- **nested 呼び出し対応**: `CLAUDECODE` / `CLAUDE_CODE_ENTRYPOINT` 環境変数を除去
- **Skills 自動注入**: Claude Code が `.claude/skills/` を自動的に読み込む
- **Observation 書き出し**: 実行中の気づきを `.forge/observations.yaml` に書き出せる
- **タイムアウト**: 設定時間超過でプロセスを kill

各エージェント固有のモデル・ツール・プロンプトは個別セクションに記載。

---

## Analyze Agent

### 概要

Intent の詳細分析を行う読み取り専用エージェント。

### 起動タイミング

Runner が Flow の `analyze` ステップを実行するとき。

### 入力コンテキスト

- Intent（[data-model.md](data-model.md) 参照）
- CLAUDE.md / Skills（`claude -p` が自動読み込み）
- Decision Storage（MCP ツール経由で実行中に取得）
- 関連する History
- 他の active な intent の情報（タイトル、ステータス、relevant_files、プラン概要）

### 処理内容

- コードベースを探索し、Intent を実行可能な単位に分解する
- 実装計画を立てられる → Task[] に分解（各 Task が implement へ）
- 問題が大きすぎる → 子 Intent[] に分解（各子が再び analyze から開始）
- 情報不足 → `needs_clarification` を返す
- Clarification 回答後は `--resume` でセッションを継続し、前回の探索コンテキストを活用する
- モデル: `models.analyze`（default: opus）
- ツール: `analyze_tools`（default: Read, Glob, Grep, Bash, WebSearch, WebFetch）

### 成果物

- Task[]（[data-model.md](data-model.md) 参照）または子 Intent[]
- 各出力に応じた Runner の Flow 調整は [runner.md](runner.md) 参照

---

## Implement Agent

### 概要

コード変更を行うエージェント。Git worktree 内で動作。

### 起動タイミング

Analyze が Task を生成した後、Runner が worktree を作成し Task ファイルを配置して実行。

### 入力コンテキスト

- worktree 内の Task ファイル（plan, relevant_files, implementation_steps, context）
- CLAUDE.md / Skills（`claude -p` が自動読み込み）

### 処理内容

- Task に従い実装を行い、コミットを作成
- Review で rejected の場合、`--resume` で同一セッションを継続し review feedback を入力として渡す（コンテキスト再構築のトークン消費を回避）
- モデル: complexity に応じて `models.default`（low/medium）または `models.complex`（high）
- ツール: `implement_tools`（default: Bash, Read, Write, Edit, Glob, Grep）

### 成果物

- Git コミット（worktree 内）
- 成功判定: コミット数 > 0

---

## Review Agent

### 概要

Implement Agent の成果物を検証するコードレビューエージェント。

### 起動タイミング

Implement 成功 + rebase 成功後。

### 入力コンテキスト

- Task 定義（plan）
- base branch との diff
- CLAUDE.md / Skills（`claude -p` が自動読み込み）

### 処理内容

- 5 つの検証基準でレビュー: 要件充足、パターン準拠、バグ/セキュリティ、計画整合性、テスト品質
- モデル: `models.default`（default: sonnet）
- ツール: `review_tools`（default: Read, Glob, Grep）

### 成果物

- Review Result（[data-model.md](data-model.md) 参照）
- Runner がファイルに永続化（履歴・Reflect 用）
- rejected 時は Review Result を Implement セッションへ `--resume` 経由で渡す
- review 結果に応じた Runner の Flow 調整は [runner.md](runner.md) 参照

---

## Audit Agent

### 概要

包括的なコードベース監査を行うエージェント。`pfl-forge audit` サブコマンドで起動。

### 起動タイミング

ユーザーが `pfl-forge audit` を実行したとき。デフォルトはコードベース全体。パス引数で対象を絞れる（例: `pfl-forge audit src/handler/`）。

### 入力コンテキスト

- History（傾向分析）
- CLAUDE.md / Skills（規約違反チェックの基準）

### 処理内容

- モデル: `models.audit`（default: opus）
- テストカバレッジの薄い領域の検出
- 設計品質の評価（巨大関数、密結合、責務の混在）
- コード規約違反のチェック（プロジェクト固有ルール）
- 技術的負債の検出（TODO、非推奨 API、重複コード）
- ドキュメントと実装の乖離の検出

### 成果物

- `.forge/observations.yaml` に Observation を記録
- Intent は生成しない。Reflect Agent が Observation を評価し、必要に応じて Intent を提案する

---

## Reflect Agent

### 概要

Intent 完了後の振り返りを行い、改善 Intent を生成する学習エージェント。

### 起動タイミング

各 Intent 完了後に自動実行。

### 入力コンテキスト

- History（Before/After 分析）
- Observation（横断分析）

### 処理内容

- Flow 選択が適切だったかの評価
- パターン検出（テンプレート化できる繰り返しパターン）
- 規約化判断（CLAUDE.md に追記すべき規約の特定）
- CLAUDE.md / Skills の有効性検証（History の傾向分析から不要な記述を検出）

### 成果物

- `.forge/intents/` に Intent を生成（source: `reflection`）
  - Skills / CLAUDE.md の生成・更新・剪定の提案
  - Observation から action が必要なもの
- リスクベースで承認フローに乗る（low → 自動実行、med/high → inbox）

---

## Epiphany 収集

全エージェントが実行中に当該 Intent と無関係な気づきを `.forge/observations.yaml` に記録できる。

1. **プロンプト指示**: 全エージェントに「Intent と無関係な気づきは `.forge/observations.yaml` に書き出せ」と指示
2. **事後リフレクション**: Reflect Agent が Intent 完了後に「他に何か気づいたか」を問う

エージェントは Intent を直接生成しない。全ての気づきは Observation として記録し、Reflect が Intent 化するか判断する。

## エージェントと Knowledge Base の関係

| Agent | History | Observation | CLAUDE.md / Skills | Decision Storage |
|-------|---------|-------------|-------------------|-----------------|
| **Analyze** | — | 書き出し可 | 自動読み込み | 参照（MCP 経由） |
| **Implement** | — | 書き出し可 | 自動読み込み + 編集可 | — |
| **Review** | — | 書き出し可 | 自動読み込み | — |
| **Audit** | 傾向分析に参照 | 書き出し可 | 自動読み込み | — |
| **Reflect** | Before/After 分析 | 横断分析 | Intent 経由で変更提案 | — |
| **Runner** | 自動記録（全件） | — | — | — |

- **History の記録主体は Runner**（[runner.md](runner.md) 参照）。各 agent がステップ結果と所要時間を意識する必要はない
- **Observation の記録主体は各 agent**。実行中に気づいた摩擦や問題を `.forge/observations.yaml` に書き出す
- **Reflect Agent が両方を突き合わせてパターンを検出**し、Skills / CLAUDE.md の変更を Intent として提案する（実際の更新は Implement Agent が行う）

