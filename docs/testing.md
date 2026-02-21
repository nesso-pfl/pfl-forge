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

### Prompt Eval（`evals/`）

プロンプト変更の品質評価。実際に `claude -p` を呼ぶため CI では skip し、プロンプト改善時に手動で実行する。

#### 目的

プロンプトの変更前後で出力品質がリグレッションしていないかを検証する。LLM の出力は非決定的なので「精度」をスカラーで追うのではなく、出力の特徴を評価軸で検査する。

#### フィクスチャ構成

特定コミットのリポジトリ状態 + Intent の組み合わせを固定し、繰り返し評価可能にする。

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
# evals/analyze/fixtures/trait-extraction.yaml
intent:
  title: "tests/ に仕様ベーステストを書く"
  body: "CLAUDE.md の Testing セクションに従い、spec test を tests/ に追加する"
repo_ref: abc1234  # 評価対象のコミット（ClaudeRunner が具象型の状態）
expectations:
  relevant_files_contain:
    - src/claude/runner.rs
  steps_mention: "trait"
  step_ordering: "trait 化がテスト記述より前"
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

ツールは promptfoo 等を想定するが、フィクスチャと評価軸の定義が先。ツール選定は実装時に決める。
