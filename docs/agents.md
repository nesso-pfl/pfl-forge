# Agents

pfl-forge は複数の Claude Code エージェントを使い分けてタスクを処理する。

各エージェントの呼び出しロジック（プロンプト組み立て・Claude CLI 実行・出力パース）は `src/agents/` に、system prompt は `src/prompt/*.md` に定義されている。

すべてのエージェント呼び出しは `process_task()` から直接行われる。`src/pipeline/` はエージェント間を繋ぐインフラ（worktree 準備・rebase・ファイル I/O・state 管理）のみを担当する。

## Orchestrate Agent

`pfl-forge parent` で起動するインタラクティブセッション。
ユーザーとの対話窓口として機能し、Bash ツールのみを持つ。

- `pfl-forge run/status/clarifications/answer/create` 等のサブコマンドを呼び出して処理を制御
- NeedsClarification が発生した場合、ユーザーに質問を提示し回答を記録
- `claude --append-system-prompt --allowedTools Bash` + `exec()` で起動

## Analyze Agent

タスクの詳細分析を行う読み取り専用エージェント。`claude -p` で非対話実行。

- モデル: `models.triage_deep` (default: sonnet)
- ツール: `triage_tools` (default: Read, Glob, Grep)
- 出力: `AnalysisResult` (complexity, plan, relevant_files, implementation_steps, context)
- 分析が不十分な場合は Architect Agent にエスカレート

## Architect Agent

Analyze Agent で十分な分析ができなかった場合に呼ばれる補助エージェント。

- モデル: `models.triage_deep` (default: sonnet)
- ツール: `triage_tools` (default: Read, Glob, Grep)
- 出力: `ArchitectOutcome::Resolved(AnalysisResult)` または `ArchitectOutcome::NeedsClarification(String)`
- NeedsClarification の場合、`.forge/clarifications/<id>.md` にファイルを作成

## Implement Agent

実際のコード変更を行うエージェント。Git worktree 内で動作する。

- モデル: complexity に応じて `models.default` (low/medium) または `models.complex` (high)
- ツール: `worker_tools` (default: Bash, Read, Write, Edit, Glob, Grep)
- worktree 内の `.forge/task.yaml` から実装計画・関連ファイル・ステップ・コンテキストを読み取る
- worktree 内でタスクの実装を行い、コミットを作成
- 出力: CLI stdout（成功/失敗は `process_task` がコミット数で判定）

## Review Agent

Implement Agent の成果物を検証するコードレビューエージェント。

- モデル: `models.default` (default: sonnet)
- ツール: `triage_tools` (default: Read, Glob, Grep)
- base branch との diff をレビューし、タスクの要件を満たしているか判定
- 出力: `ReviewResult` (approved, issues, suggestions)
- `process_task` から直接呼ばれ、rejected の場合は review feedback を付けて Implement Agent を再実行（`max_review_retries` 回まで）
- 全リトライ後も rejected なら Error 状態にする

## Agent 間の YAML 通信

エージェント間のデータ受け渡しは `.forge/` ディレクトリを介して行われる:

- `.forge/work/{id}-{NNN}.yaml` — analyze の結果をタスク YAML としてリポジトリルートに書き出す。`status` フィールド（pending → executing → completed/failed）でロック管理。
- `.forge/task.yaml` — execute ステージが worktree 内に書き出し、Implement Agent が読み取る。
- `.forge/review.yaml` — Review Agent の結果（approved, issues, suggestions）。integrate ステージで書き出し、監査ログとして機能。

`.forge/` は `.gitignore` に自動追加されるため、コミットには含まれない。

## Agent 間の関係

```
Orchestrate Agent (interactive)
  └─ pfl-forge run (CLI)
       └─ process_task (タスク単位で独立並列実行)
            ├─ Analyze Agent
            │    └─ Architect Agent (必要時)
            │         └─ NeedsClarification → Orchestrate に戻る
            │    → .forge/work/*.yaml にタスク書き出し
            └─ loop (max_review_retries + 1):
                 ├─ Implement Agent ← .forge/task.yaml を読む
                 └─ Review Agent → .forge/review.yaml を書く
                      └─ rejected → Implement Agent に feedback 渡して再実行
```

## モデル選択

| Agent | 設定キー | Default |
|-------|---------|---------|
| Analyze | `models.triage_deep` | sonnet |
| Architect | `models.triage_deep` | sonnet |
| Implement (low/medium) | `models.default` | sonnet |
| Implement (high) | `models.complex` | opus |
| Review | `models.default` | sonnet |
| Orchestrate | `--model` 引数 | (claude default) |
