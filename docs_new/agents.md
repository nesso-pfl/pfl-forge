# エージェント構成

| Agent | 責務 | 状態 |
|-------|------|------|
| **Analyze** | タスク分析、実装計画 | 既存（ほぼ同じ） |
| **Implement** | コード実装 + observation 書き出し | 既存（observation 追加） |
| **Review** | コードレビュー | 既存（ほぼ同じ） |
| **Audit** | コードベース監査 → Intent 生成 | **新規** |
| **Reflect** | タスク完了後の振り返り → 学習 | **新規** |
| **Orchestrate** | インタラクティブセッション | 既存（拡張） |
| ~~Architect~~ | Analyze に統合、Flow 調整で代替 | **削除** |
| ~~Verify~~ | pre-commit hook で代替 | **削除** |

## Audit Agent

`pfl-forge audit` で起動。包括的なコードベース監査を行う:
- テストカバレッジの薄い領域
- 設計品質（巨大関数、密結合、責務の混在）
- コード規約違反（プロジェクト固有ルール）
- 技術的負債（TODO、非推奨 API、重複コード）
- ドキュメントと実装の乖離

発見事項を Intent として登録する。

## Reflect Agent

各タスク完了後に実行。以下を評価:
- Flow 選択は適切だったか
- 他に気づいた問題はないか
- テンプレート化できるパターンはないか
- ルール化すべき規約はないか

出力:
- observation の評価 → Intent 生成が必要なら Intent Registry へ
- Knowledge Base 更新（skills, rules, history）

## Epiphany 収集（二重アプローチ）

1. **プロンプト指示**: 全エージェントに「タスクと無関係な気づきは `.forge/observations.yaml` に書き出せ」と指示
2. **事後リフレクション**: Reflect Agent がタスク完了後に「他に何か気づいたか」を問う

両方を併用する。

生成ルール:
- action が必要 → `.forge/intents/` に intent を直接生成（observation は書かない）
- action 不要だが記録に値する → `.forge/observations.yaml` に observation のみ

これにより observation は常に「未処理」であり、Reflect Agent は全件を評価対象にできる。

## エージェントと Knowledge Base の関係

| Agent | History | Observation | Skills / Rules | Decision Storage |
|-------|---------|-------------|----------------|-----------------|
| **Analyze** | — | 書き出し可 | 参照（プロンプト注入） | 参照（プロンプト注入） |
| **Implement** | — | 書き出し可 | 参照（プロンプト注入） | — |
| **Review** | — | 書き出し可 | 参照（プロンプト注入） | — |
| **Audit** | 傾向分析に参照 | 書き出し可 | 参照 + 規約違反チェック | — |
| **Reflect** | Before/After 分析 | 横断分析 | 生成・更新・剪定 | — |
| **Execution Engine** | 自動記録（全件） | — | — | — |

- **History の記録主体は Execution Engine**。各 agent がステップ結果と所要時間を意識する必要はない
- **Observation の記録主体は各 agent**。実行中に気づいた摩擦や問題を `.forge/observations.yaml` に書き出す
- **Reflect Agent が両方を突き合わせてパターンを検出**し、Skills / Rules への昇格や剪定を判断する
