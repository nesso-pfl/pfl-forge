# Agents

pfl-forge は複数の Claude Code エージェントを使い分けて issue を処理する。

各エージェントの system prompt は `src/prompt/*.md` に定義されており、`--append-system-prompt` で渡される。

## Parent Agent

`pfl-forge parent` で起動するインタラクティブセッション。
ユーザーとの対話窓口として機能し、Bash ツールのみを持つ。

- `pfl-forge run/status/clarifications/answer` 等のサブコマンドを呼び出して処理を制御
- NeedsClarification が発生した場合、ユーザーに質問を提示し回答を記録
- `claude --append-system-prompt --allowedTools Bash` + `exec()` で起動

## Deep Triage Agent

issue の詳細分析を行う読み取り専用エージェント。`claude -p` で非対話実行。

- モデル: `settings.models.triage_deep` (default: sonnet)
- ツール: `settings.triage_tools` (default: Read, Glob, Grep)
- 出力: `DeepTriageResult` (complexity, plan, relevant_files, implementation_steps, context)
- 分析が不十分な場合は Consultation Agent にエスカレート

## Consultation Agent

Deep Triage で十分な分析ができなかった場合に呼ばれる補助エージェント。

- モデル: `settings.models.triage_deep` (default: sonnet)
- ツール: `settings.triage_tools` (default: Read, Glob, Grep)
- 出力: `ConsultationOutcome::Resolved(DeepTriageResult)` または `ConsultationOutcome::NeedsClarification(String)`
- NeedsClarification の場合、`.forge/clarifications/<number>.md` にファイルを作成

## Execute Agent (Worker)

実際のコード変更を行うエージェント。Git worktree 内で動作する。

- モデル: complexity に応じて `settings.models.default` (low/medium) または `settings.models.complex` (high)
- ツール: `settings.worker_tools` + `repo.extra_tools` (default: Bash, Read, Write, Edit, Glob, Grep)
- worktree 内の `.forge/task.yaml` から実装計画・関連ファイル・ステップ・コンテキストを読み取る
- worktree 内で issue の実装を行い、コミットを作成
- 出力: `ExecuteResult` (Success, TestFailure, Unclear, Error)

## Review Agent

Worker の成果物を検証するコードレビューエージェント。

- モデル: `settings.models.default` (default: sonnet)
- ツール: `settings.triage_tools` (default: Read, Glob, Grep)
- base branch との diff をレビューし、issue の要件を満たしているか判定
- 出力: `ReviewResult` (approved, issues, suggestions)
- integrate フロー内で呼ばれ、approved でなければ PR 説明に指摘事項を含める

## Agent 間の YAML 通信

エージェント間のデータ受け渡しは `.forge/` ディレクトリを介して行われる:

- `.forge/work/issue-{N}-{NNN}.yaml` — triage の結果をタスク YAML としてリポジトリルートに書き出す。`status` フィールド（pending → executing → completed/failed）でロック管理。
- `.forge/task.yaml` — execute ステージが worktree 内に書き出し、Worker が読み取る。
- `.forge/review.yaml` — Review Agent の結果（approved, issues, suggestions）。integrate ステージで書き出し、監査ログとして機能。

`.forge/` は `.gitignore` に自動追加されるため、コミットには含まれない。

## Agent 間の関係

```
Parent Agent (interactive)
  └─ pfl-forge run (CLI)
       ├─ Phase 1: Deep Triage Agent (並列)
       │    └─ Consultation Agent (必要時)
       │         └─ NeedsClarification → Parent に戻る
       │    → .forge/work/*.yaml にタスク書き出し
       ├─ Phase 2: Execute Agent (Worker, 並列) ← .forge/task.yaml を読む
       └─ Phase 3: integrate (streaming)
            └─ Review Agent → .forge/review.yaml を書く
```

## モデル選択

| Agent | 設定キー | Default |
|-------|---------|---------|
| Deep Triage | `models.triage_deep` | sonnet |
| Consultation | `models.triage_deep` | sonnet |
| Execute (low/medium) | `models.default` | sonnet |
| Execute (high) | `models.complex` | opus |
| Review | `models.default` | sonnet |
| Parent | `--model` 引数 | (claude default) |
