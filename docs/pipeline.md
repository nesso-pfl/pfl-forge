# Pipeline

pfl-forge のパイプラインフローとエージェント間通信の全体像。

## 設計思想

pfl-forge のコードは3つのレイヤーに分かれる:

- **`agents/`** — Claude Code の呼び出し（プロンプト組み立て・CLI 実行・出力パース）
- **`task/`** — タスクデータの読み書き・変換（fetch・work YAML・clarification）
- **`git/`** — Git 操作（worktree 作成・rebase・gitignore 管理）
- **`process_task()`** — フロー制御。すべてのエージェント呼び出しはここから行う

`task/` と `git/` はエージェントを呼ばない。エージェントの前後処理（データ準備・結果保存・git 操作）のみを担当する。

```
process_task が呼ぶもの:
  agents:   analyze → architect → implement → review
  task:     fetch, work, clarification
  git:      worktree(create, ensure_gitignore_forge), branch(try_rebase, commit_count)
  main.rs:  prepare, write_review_yaml
```

## フロー概要

```
fetch → process_task (per task, parallel):
  analyze → (architect) → work → prepare → implement → rebase → review
                                            └── review rejected → implement に戻る (retry loop)
```

```
process_task (タスク単位で独立並列実行):
  {permit} analyze()
    └─ [分析不十分] → architect::resolve()
         └─ [NeedsClarification] → .forge/clarifications/{id}.md 書き出し, return
    └─ [成功] → work::write_tasks()
         └─ .forge/work/{id}-001.yaml 書き出し
  // permit released

  prepare()
    git worktree 作成, .forge/task.yaml 書き出し, モデル選択

  loop (max_review_retries + 1):
    {permit} implement::run()
      Implement Agent 実行（worktree 内で実装・コミット）
    // permit released

    git::branch::try_rebase()
      base branch に rebase

    {permit} review::review()
      Review Agent 実行
      write_review_yaml()
    // permit released

    if approved → Success, return
    if rejected && retries remaining → implement に戻る (review feedback 付き)
    if rejected && no retries → Error, return
```

## モジュールの役割

| モジュール | 役割 | エージェント呼び出し |
|-----------|------|-------------------|
| `task/fetch` | タスク YAML 読み込み | なし |
| `task/work` | `AnalysisResult` → `Task` YAML 変換・書き出し | なし |
| `task/clarification` | clarification ファイルの読み書き | なし |
| `git/worktree` | worktree 作成・削除・gitignore 管理 | なし |
| `git/branch` | commit 数カウント・rebase | なし |
| `main.rs (prepare)` | worktree 作成・タスクファイル配置・モデル選択 | なし |
| `main.rs (write_review_yaml)` | review.yaml 書き出し | なし |

## エージェント間通信

エージェント（Claude プロセス）間の通信はすべて `.forge/` ディレクトリ内のファイル経由で行われる。同一 Rust プロセス内のステージ間はメモリ（構造体）渡し。

### 通信マップ

| 区間 | 媒体 | ファイル/構造体 |
|------|------|-----------------|
| fetch → analyze | メモリ | `Vec<Issue>` |
| analyze → architect | メモリ | `AnalysisResult` |
| architect → ユーザー | ファイル | `.forge/clarifications/{id}.md` |
| ユーザー → analyze(再実行) | ファイル | `.forge/clarifications/{id}.answer.md` |
| analyze → prepare | ファイル | `.forge/work/{id}-001.yaml` |
| prepare → Implement Agent | ファイル | `<worktree>/.forge/task.yaml` |
| review → re-implement | メモリ | `ReviewResult` (feedback) |
| review → 監査ログ | ファイル | `<worktree>/.forge/review.yaml` |
| 全ステージ → state | ファイル | `.forge/state.yaml` |

### ファイルの役割

- **`.forge/tasks/{id}.yaml`** — ユーザーが作成するタスク定義（入力）
- **`.forge/work/{id}-001.yaml`** — analyze が書き出すタスク YAML（plan, steps, files, complexity 等）。`status` フィールドで状態管理
- **`<worktree>/.forge/task.yaml`** — prepare が worktree 内にコピーし、Implement Agent が読み取る
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
- Semaphore permit は各 Claude プロセス呼び出しごとに取得/解放（analyze, implement, review 間で他タスクが走れる）
- 並列数: `parallel_workers` で制御
