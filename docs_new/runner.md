# Runner

Flow ステップを逐次実行し、各ステップの結果に応じてルールベースで残りの Flow を調整する。

呼び出し元:
- `pfl-forge run` — CLI から直接
- Operator Agent — インタラクティブセッション内で `pfl-forge run` を実行

Runner 自身は AI エージェントではなく、Rust コードによる決定論的な制御ロジック。

---

## デフォルト Flow

通常タスク（feature / fix / refactor / test）は全て同一のデフォルト Flow で処理する:

```
[analyze, implement, review]
```

種別ごとにフローを分岐しない理由:
- refactor でも大規模なら analyze が必要。test でも正しいテストか review が必要
- 種別の差はフローではなく、各エージェントへのコンテキスト注入で吸収する

別パイプラインを持つ種別（Intent の `type` フィールドで分岐）:

| 種別 | Flow |
|------|------|
| `audit` | `[audit, report]` |
| `skill_extraction` | `[observe, abstract, record]` |

### Non-agent ステップ

Flow 内のステップのうち、AI エージェントではなく Runner の Rust コードで実行するもの:

- **report** — `[audit, report]` の第2ステップ。Audit Agent が書き出した Observation を読み、CLI に人間向けサマリを出力する
- **rebase** — implement 後、review 前に毎回実行

### Runner が自動挿入するステップ

Flow テンプレートには含まれないが、Runner が固定で実行するステップ:

- **rebase** — 上述の通り、Runner が implement と review の間に挿入
- **reflect** — 子 Intent を持たない Intent の完了後に自動実行。子 Intent に分解された場合、親 Intent では reflect しない（学びは実際に実装した単位に紐づく）

---

## 実行フロー

1 Intent を処理する流れ:

```
Intent 受取
  │
  ├─ analyze ステップ
  │    Analyze Agent 呼び出し
  │    ├─ Task[] → 各 Task を implement へ
  │    ├─ 子 Intent[] → 各子 Intent がフルパイプラインを経る
  │    └─ needs_clarification → Intent を一時停止、inbox へ
  │
  ├─ implement ステップ（Task ごと）
  │    worktree 作成 → setup commands → Task ファイル配置 → Implement Agent 呼び出し
  │    └─ コミット数 > 0 で成功
  │
  ├─ rebase
  │    main への rebase 実行
  │    └─ 失敗時はコンフリクト解決フローへ
  │
  ├─ review ステップ
  │    Review Agent 呼び出し
  │    ├─ approved → 次へ
  │    └─ rejected → implement + review サイクル再実行
  │
  ├─ 統合
  │    worktree の変更を main にマージ
  │
  └─ reflect ステップ
       Reflect Agent 呼び出し（子を持たない Intent の完了後に自動実行）
```

### Worktree Setup

git worktree には追跡ファイルしか含まれない。`.gitignore` 対象の生成物（API クライアント、`node_modules` 等）は worktree に存在しないため、Implement Agent 起動前にセットアップが必要になる場合がある。

`pfl-forge.yaml` の `worktree_setup` にコマンドを列挙すると、Runner が worktree 作成後・Implement Agent 起動前に逐次実行する。

```yaml
worktree_setup:
  - "npm install"
  - "npm run generate-api-client"
```

Tips:

- **キャッシュ活用**: `npm install` は lockfile + ローカルキャッシュがあれば数秒で終わる。初回以外は軽い
- **親リポからコピー**: 重い生成物は `rsync` で親リポからコピーすると高速。`rsync -a ../../node_modules .` のように書ける
- **コード依存の生成物に注意**: API クライアント生成のようにソースコードに依存する生成物は、symlink やコピーではなく毎回生成すべき
- **未設定でも動く**: worktree_setup は省略可。生成物に依存しないプロジェクトでは不要

### Analyze → Task の関係

Analyze は Intent を 1 つ以上の Task に分解する。各 Task が独立した implement 実行単位になる。Task 間に `depends_on` がある場合は依存順に逐次実行し、独立した Task は並列実行できる。

### --resume によるセッション継続

- **Analyze**: `needs_clarification` → 人間が回答 → `--resume` で同一セッション継続
- **Implement**: `review` で rejected → `--resume` で同一セッションに review feedback を注入

前回の探索コンテキストを活用することでトークン消費を抑える。

---

## Flow 調整ルール

各ステップの結果に応じて、残りの Flow をルールベースで調整する。

### analyze の結果による調整

| 条件 | 調整 |
|------|------|
| `needs_clarification` | Intent を一時停止し inbox へ |
| `depends_on: [intent-id]` | 依存 Intent の完了まで implement を遅延 |

### review の結果による調整

| 条件 | 調整 |
|------|------|
| `rejected` | 該当 Task の implement + review サイクルを追加（設定上限まで） |
| 全リトライ後も `rejected` | Task を `failed` にする。Intent は残りの Task 状況に応じて `blocked`（一部失敗）または `error`（全失敗）となり inbox へ |

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
- 各ステップの session ID、消費トークン（input / output）、コスト（USD）
- 最終結果（success / failed / escalated）+ 失敗理由
- 生成された Observation への参照
- タイムスタンプ

### CLI JSON 出力からの取得

`claude -p --output-format json` はエージェントの応答テキストだけでなく、メタデータを含むラッパーオブジェクトを返す。Runner はこのラッパーから History 用のデータを抽出する。

```json
{
  "type": "result",
  "subtype": "success",
  "is_error": false,
  "duration_ms": 2514,
  "duration_api_ms": 2482,
  "num_turns": 1,
  "result": "...(エージェントの応答テキスト)",
  "session_id": "d44db260-...",
  "total_cost_usd": 0.044,
  "usage": {
    "input_tokens": 3,
    "cache_creation_input_tokens": 5586,
    "cache_read_input_tokens": 17836,
    "output_tokens": 20
  },
  "modelUsage": {
    "claude-opus-4-6": {
      "inputTokens": 3,
      "outputTokens": 20,
      "cacheReadInputTokens": 17836,
      "cacheCreationInputTokens": 5586,
      "costUSD": 0.044
    }
  }
}
```

| History フィールド | JSON パス |
|-------------------|-----------|
| session ID | `session_id` |
| 入力トークン | `usage.input_tokens` |
| 出力トークン | `usage.output_tokens` |
| キャッシュ読み込みトークン | `usage.cache_read_input_tokens` |
| コスト | `total_cost_usd` |
| 所要時間 | `duration_ms` |
| API 所要時間 | `duration_api_ms` |
| ターン数 | `num_turns` |

現在の実装（`ClaudeRunner::parse_claude_json_output`）は `result` のみ抽出しているため、ラッパー全体を返すよう拡張が必要。

History は「構造化されたサマリ」。エージェント内部の操作ログ（個別ファイル読み込み等）は記録しない。
