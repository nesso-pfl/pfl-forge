# 現行実装からの変更点

| 項目 | 現行 | 新アーキ |
|------|------|----------|
| Architect Agent | Analyze が不十分な場合にエスカレート | 削除。Analyze 内で完結（`needs_clarification` で inbox へ） |
| Verify Agent | 実装後のテスト検証 | 削除。pre-commit hook で代替 |
| Analyze の起動 | `process_task()` から直接呼び出し | Runner が Flow ステップとして実行 |
| エージェント間データ | `AnalysisResult` / `ReviewResult` 型、`.forge/work/*.yaml` / `.forge/task.yaml` / `.forge/review.yaml` | 詳細は実装時に決定 |
| Review リトライ | `max_review_retries` 設定キー | Runner の Flow 調整ルールとして管理 |
| モデル設定キー | `models.triage_deep`（Analyze 専用だが名前が汎用） | `models.analyze` |
| ツール設定キー | `triage_tools`（Analyze/Architect/Review で共有） | エージェントごとに分離: `analyze_tools`, `review_tools` |
| Implement リトライ | 毎回新プロセスで再起動 | `--resume` でセッション継続（トークン節約） |
| Analyze 再実行 | clarification 回答後に新プロセスで再起動 | `--resume` でセッション継続 |
| Reflect の成果物 | Knowledge Base を直接更新 | Intent を生成し、通常のパイプライン（Analyze → Implement → Review）で更新 |
