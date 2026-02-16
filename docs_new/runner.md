# Runner

Flow ステップを逐次実行し、各ステップの結果に応じてルールベースで残りの Flow を調整する。

呼び出し元:
- `pfl-forge run` — CLI から直接
- Operator Agent — インタラクティブセッション内で `pfl-forge run` を実行

Runner 自身は AI エージェントではなく、Rust コードによる決定論的な制御ロジック。

---

## Flow テンプレート

タスク種別ごとにデフォルト Flow を持つ。Quick Classification（[data-model.md](data-model.md) 参照）がタスク種別を判定し、対応するテンプレートを選択する。

| タスク種別 | デフォルト Flow |
|-----------|---------------|
| `feature` | `[analyze, implement, review]` |
| `fix` | `[analyze, implement]` |
| `refactor` | `[implement]` |
| `test` | `[analyze, implement]` |
| `audit` | `[audit, report]` |
| `skill_extraction` | `[observe, abstract, record]` |

---

## 実行フロー

1 Intent を処理する流れ:

```
Intent 受取
  │
  ├─ Quick Classification → Flow テンプレート選択
  │
  ├─ analyze ステップ
  │    Analyze Agent 呼び出し
  │    ├─ Task[] → 各 Task を implement へ
  │    ├─ 子 Intent[] → 各子 Intent がフルパイプラインを経る
  │    └─ needs_clarification → Intent を一時停止、inbox へ
  │
  ├─ implement ステップ（Task ごと）
  │    worktree 作成 → Task ファイル配置 → Implement Agent 呼び出し
  │    └─ コミット数 > 0 で成功
  │
  ├─ rebase
  │    main への rebase 実行
  │    └─ 失敗時はコンフリクト解決フローへ
  │
  ├─ review ステップ（Flow に含まれる場合）
  │    Review Agent 呼び出し
  │    ├─ approved → 次へ
  │    └─ rejected → implement + review サイクル再実行
  │
  ├─ 統合
  │    worktree の変更を main にマージ
  │
  └─ reflect ステップ
       Reflect Agent 呼び出し（Intent 完了後に自動実行）
```

### Analyze → Task の関係

Analyze は Intent を 1 つ以上の Task に分解する。各 Task が独立した implement 実行単位になる。Task 間に `depends_on` がある場合は依存順に逐次実行し、独立した Task は並列実行できる。

### --resume によるセッション継続

- **Analyze**: `needs_clarification` → 人間が回答 → `--resume` で同一セッション継続
- **Implement**: `review` で rejected → `--resume` で同一セッションに review feedback を注入

新プロセスを起動するのではなく、前回の探索コンテキストを活用することでトークン消費を抑える。

---

## Flow 調整ルール

各ステップの結果に応じて、残りの Flow をルールベースで調整する。

### analyze の結果による調整

| 条件 | 調整 |
|------|------|
| `complexity: high` | `review` ステップを追加 |
| `needs_clarification` | Intent を一時停止し inbox へ |
| `depends_on: [intent-id]` | 依存 Intent の完了まで implement を遅延 |

### implement の結果による調整

| 条件 | 調整 |
|------|------|
| 変更が 10 行未満 | `review` をスキップ |

### review の結果による調整

| 条件 | 調整 |
|------|------|
| `rejected` | implement + review サイクルを追加（設定上限まで） |
| 全リトライ後も `rejected` | Error 状態 |

### 設計方針

- ルールは Rust コード。ユニットテスト可能、予測可能、デバッグ可能
- 精度は「最初から正解する」ではなく「途中で修正できる」で担保
- 段階的拡張: ハードコードルール → 学習ベースルールの追加

---

## 並列タスク協調

### コンテキスト注入

analyze 実行時に、他の active な Intent の情報を Analyze Agent に注入する:

- タイトル、ステータス（analyzing / implementing / reviewing）
- relevant_files、プラン概要（analyze 済みの場合）

これにより Analyze Agent は:
- 他タスクが変更予定のファイルを把握し、コンフリクトを避けた計画を立てられる
- 明確な依存関係がある場合、`depends_on: [intent-id]` を出力できる

主な価値は**依存検出**にある。コンフリクト回避は副次的な効果で、発生時はコンフリクト解決で対処する。

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

## History 記録

Runner が各 Intent の実行記録を自動的に History に書き込む。個々のエージェントは記録を意識する必要がない。

記録するフィールド:
- Intent メタデータ（type, source, risk, title）
- 実行された Flow（ステップ一覧 + 調整内容）
- 各ステップの結果と所要時間
- 最終結果（success / failed / escalated）+ 失敗理由
- 生成された Observation への参照
- タイムスタンプ

History は「構造化されたサマリ」。エージェント内部の操作ログ（個別ファイル読み込み等）は記録しない。
