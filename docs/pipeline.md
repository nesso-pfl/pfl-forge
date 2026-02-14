# Pipeline

pfl-forge のパイプラインフローとエージェント間通信の全体像。

## フロー概要

```
fetch → process_task (per task, parallel):
  analyze → (architect) → execute → integrate(review)
                          └── review rejected → re-execute (retry loop)
```

```
process_task (タスク単位で独立並列実行):
  {permit} analyze()
    └─ [分析不十分] → architect::resolve()
         └─ [NeedsClarification] → .forge/clarifications/{id}.md 書き出し, return
    └─ [成功] → work::write_tasks()
         └─ .forge/work/{id}-001.yaml 書き出し
  // permit released

  loop (max_review_retries + 1):
    {permit} execute
      git worktree 作成
      <worktree>/.forge/task.yaml 書き出し
      Implement Agent 実行（worktree 内で実装・コミット）
    // permit released

    {permit} integrate
      rebase → review
      <worktree>/.forge/review.yaml 書き出し
    // permit released

    if approved → Success, return
    if rejected && retries remaining → re-execute with review feedback
    if rejected && no retries → Error, return
```

## エージェント間通信

エージェント（Claude プロセス）間の通信はすべて `.forge/` ディレクトリ内のファイル経由で行われる。同一 Rust プロセス内のステージ間はメモリ（構造体）渡し。

### 通信マップ

| 区間 | 媒体 | ファイル/構造体 |
|------|------|-----------------|
| fetch → analyze | メモリ | `Vec<ForgeIssue>` |
| analyze → architect | メモリ | `AnalysisResult` |
| architect → ユーザー | ファイル | `.forge/clarifications/{id}.md` |
| ユーザー → analyze(再実行) | ファイル | `.forge/clarifications/{id}.answer.md` |
| analyze → execute | ファイル | `.forge/work/{id}-001.yaml` |
| execute → Implement Agent | ファイル | `<worktree>/.forge/task.yaml` |
| review → re-execute | メモリ | `ReviewResult` (feedback) |
| review → 監査ログ | ファイル | `<worktree>/.forge/review.yaml` |
| 全ステージ → state | ファイル | `.forge/state.yaml` |

### ファイルの役割

- **`.forge/tasks/{id}.yaml`** — ユーザーが作成するタスク定義（入力）
- **`.forge/work/{id}-001.yaml`** — analyze が書き出すタスク YAML（plan, steps, files, complexity 等）。`status` フィールドで状態管理
- **`<worktree>/.forge/task.yaml`** — execute が worktree 内にコピーし、Implement Agent が読み取る
- **`<worktree>/.forge/review.yaml`** — Review Agent の結果（approved, issues, suggestions）
- **`.forge/clarifications/{id}.md`** — Architect Agent が書き出す質問
- **`.forge/clarifications/{id}.answer.md`** — ユーザーの回答（`pfl-forge answer` で作成）
- **`.forge/state.yaml`** — 全タスクのステータスを永続化

## ステータス遷移

```
Pending
  ↓
Triaging
  ├─→ NeedsClarification → (ユーザー回答) → Pending → Triaging
  └─→ Executing
       ├─→ Reviewing
       │    ├─→ Success (terminal)
       │    └─→ Executing (review rejected, retry)
       └─→ Error (自動再試行)
```

## 並列実行

- タスク単位で `process_task` を `JoinSet` に spawn
- Semaphore permit は各 Claude プロセス呼び出しごとに取得/解放（analyze, execute, integrate 間で他タスクが走れる）
- 並列数: `parallel_workers` で制御
