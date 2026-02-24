# テスト戦略

## テスト層

### Unit Test（`src/` 内 `#[cfg(test)]`）

private 関数やヘルパーの関数単位テスト。pub API のテストは `tests/` に書く。

命名規則: `<関数名>_<条件や入力>_<期待結果>`。`test_` プレフィックスは不要（`#[test]` 属性で十分）。

```
// good: 関数名 + 何が起きるか
extract_json_strips_code_block_markers
parse_claude_json_output_extracts_result_field
empty_yaml_produces_valid_config_defaults

// bad: test_ 二重、条件が曖昧
test_extract_json_code_block
test_default_config
```

モジュールパス（`claude::runner::tests::`）が場所を示すので、テスト名は入力条件と期待結果に集中する。

### Spec Test（`tests/`）

仕様ベースの振る舞いテスト。pub API のみ対象。ディレクトリ構成は docs に対応:

| ディレクトリ | 対応 doc | テスト内容 |
|-------------|----------|-----------|
| `tests/agent/` | agents.md | 各エージェントの入出力仕様（Claude trait のモック経由） |
| `tests/runner/` | runner.md | Flow 実行・調整ルール・リトライ・自動挿入ステップ |
| `tests/data_model/` | data-model.md | Intent / Task / Observation / History の YAML パース・バリデーション |

エージェントテストは `Claude` trait のモック実装を使い、`claude` プロセスを起動せずに検証する。

#### 並列実行のテスト方針

Runner の `run_intents` は `parallel_workers` で複数 Intent を並列処理する。並列の安全性は設計レベルで担保している（Intent ごとに独立した worktree、Intent ファイルは ID 別で競合しない）。

Spec test では `parallel_workers = 1` の順次実行でロジックを検証する。並列固有の問題（race condition 等）はテストでの再現が困難で flaky になりやすいため、実運用で検出・対処する方針とする。

### Prompt Eval（`evals/`）

プロンプト変更の品質評価。実際に `claude -p` を呼ぶため CI では skip し、プロンプト改善時に手動で実行する。

#### 目的

プロンプトの変更前後で出力品質がリグレッションしていないかを検証する。LLM の出力は非決定的なので「精度」をスカラーで追うのではなく、出力の特徴を評価軸で検査する。

#### 実行モード

| モード | コードベース | 用途 | 頻度 |
|--------|-------------|------|------|
| **最新コミット** | HEAD | プロンプトが現在のコードベースで機能するか確認 | 日常的 |
| **固定コミット** | `repo_ref` 指定 | プロンプト変更の前後比較（同条件でリグレッション検出） | プロンプト大改修時 |

通常は最新コミットで実行する。固定コミットはプロンプトを大きく書き換えたときのリグレッション確認用。

#### フィクスチャ構成

Intent + 期待する出力特性の組み合わせ。`repo_ref` は省略可（省略時は最新コミットで実行）。

```
evals/
  analyze/
    fixtures/
      trait-extraction.yaml   # Intent + 期待する出力特性
      large-refactor.yaml
    ...
  review/
    fixtures/
      ...
```

フィクスチャ例:

```yaml
# evals/analyze/fixtures/simple-feature.yaml
intent:
  title: "Add a health check endpoint"
  body: "Add a GET /health endpoint that returns 200 OK"
repo_ref: abc1234  # 省略可。指定時はそのコミットの worktree で実行
expectations:
  relevant_files_contain:    # relevant_files に含まれるべきパターン
    - "src/"
  plan_mentions:             # plan に含まれるべきキーワード
    - "health"
    - "endpoint"
  steps_mention:             # implementation_steps に含まれるべきキーワード
    - "handler"
  has_implementation_steps: true  # implementation_steps が1つ以上
  complexity_is_one_of:      # complexity の許容値
    - low
    - medium
  min_relevant_files: 1      # relevant_files の最小数
```

Review eval 用:

```yaml
# evals/review/fixtures/correct-impl.yaml
intent:
  title: "Add unit tests for config parsing"
  body: "Add tests for edge cases in config parsing"
expectations:
  should_approve: true       # approved であること（false positive 検出）
```

#### 評価軸

エージェントごとに固有の評価軸を定義する。

**Analyze Agent**:

| 評価軸 | 検証内容 |
|--------|---------|
| CLAUDE.md との整合性 | 出力がプロジェクト方針と矛盾しないか |
| コード探索の深さ | relevant_files が表層的でなく十分か |
| 前提変更の先読み | 実装に必要な前提条件が steps に含まれているか |
| 依存チェーンの検出 | depends_on が適切に設定されているか |
| 分解の粒度 | Task が大きすぎず小さすぎないか |

**Review Agent**:

| 評価軸 | 検証内容 |
|--------|---------|
| 誤検出率 | 正しい実装を rejected にしていないか |
| 見逃し率 | 明らかな問題を approved にしていないか |
| feedback の具体性 | issues / suggestions が再実装に十分な情報を含むか |

#### 検証方法

- **構造チェック**: JSON スキーマ準拠、必須フィールド存在
- **内容チェック**: 特定キーワードの有無、フィールド値の妥当性
- **LLM-as-judge**: 別の LLM に出力を評価させる（複雑な品質判断）

現在は構造チェックと内容チェックのみ実装済み。LLM-as-judge は決定論的チェックでは判断が難しい場面（feedback の具体性、プロジェクト方針との整合性など）が出てきた時点で追加する。
