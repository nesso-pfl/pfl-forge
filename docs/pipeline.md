# Pipeline

pfl-forge のパイプラインフローとエージェント間通信の全体像。

## フロー概要

```
fetch → deep_triage → (consult) → work → execute → integrate → report
```

```
PHASE 1: TRIAGE (並列 per issue)
  fetch_tasks()
    └─ .forge/tasks/{id}.yaml を読み取り → Vec<ForgeIssue>
  deep_triage()
    └─ [分析不十分] → consult()
         └─ [NeedsClarification] → .forge/clarifications/{id}.md 書き出し
    └─ [成功] → work::write_tasks()
         └─ .forge/work/{id}-001.yaml 書き出し

PHASE 2: EXECUTE (並列 per task)
  .forge/work/{id}-001.yaml を読み取り
  git worktree 作成
  <worktree>/.forge/task.yaml 書き出し
  Worker 実行（worktree 内で実装・コミット）

PHASE 3: INTEGRATE (streaming per result)
  rebase → review → report
  <worktree>/.forge/review.yaml 書き出し
```

## エージェント間通信

エージェント（Claude プロセス）間の通信はすべて `.forge/` ディレクトリ内のファイル経由で行われる。同一 Rust プロセス内のステージ間はメモリ（構造体）渡し。

### 通信マップ

| 区間 | 媒体 | ファイル/構造体 |
|------|------|-----------------|
| fetch → triage | メモリ | `Vec<ForgeIssue>` |
| deep_triage → consult | メモリ | `DeepTriageResult` |
| consult → ユーザー | ファイル | `.forge/clarifications/{id}.md` |
| ユーザー → triage(再実行) | ファイル | `.forge/clarifications/{id}.answer.md` |
| triage → execute | ファイル | `.forge/work/{id}-001.yaml` |
| execute → Worker | ファイル | `<worktree>/.forge/task.yaml` |
| execute → integrate | メモリ | `WorkerOutput` 構造体 |
| review → 監査ログ | ファイル | `<worktree>/.forge/review.yaml` |
| 全ステージ → state | ファイル | `.forge/state.yaml` |

### ファイルの役割

- **`.forge/tasks/{id}.yaml`** — ユーザーが作成するタスク定義（入力）
- **`.forge/work/{id}-001.yaml`** — triage が書き出すタスク YAML（plan, steps, files, complexity 等）。`status` フィールドで状態管理
- **`<worktree>/.forge/task.yaml`** — execute が worktree 内にコピーし、Worker が読み取る
- **`<worktree>/.forge/review.yaml`** — Review Agent の結果（approved, issues, suggestions）
- **`.forge/clarifications/{id}.md`** — Consultation Agent が書き出す質問
- **`.forge/clarifications/{id}.answer.md`** — ユーザーの回答（`pfl-forge answer` で作成）
- **`.forge/state.yaml`** — 全タスクのステータスを永続化

## ステータス遷移

```
Pending
  ↓
Triaging
  ├─→ NeedsClarification → (ユーザー回答) → Pending → Triaging
  └─→ Executing
       ├─→ Success (terminal)
       └─→ Error (自動再試行)
```

## 並列実行

- Phase 1 (triage): `JoinSet` + `Semaphore` で issue 単位の並列処理
- Phase 2 (execute): 同上、task 単位の並列処理
- Phase 3 (integrate): ストリーミング（完了順に逐次処理）
- 並列数: `parallel_workers` で制御
