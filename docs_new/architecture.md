# pfl-forge New Architecture

pfl-forge を「タスク実行エンジン」から「自律開発パートナー」へ再設計する。

## 使用前提条件

- 対象リポジトリに **pre-commit hook** を設定し、コミット時に静的検査を実行すること
  - テスト、リント、フォーマットなど
  - Implement Agent のコミットが pre-commit hook で検証されるため、専用の Verify Agent は不要

## 3つのパラダイムシフト

### 1. Task → Intent + Observation

```
現在:  Task(YAML) → 固定パイプライン → Result
新:    Intent(any trigger) → 柔軟な Flow → Action[] → Learn
```

Intent のソース:
- **Human** — `.forge/tasks/*.md` に Markdown で作成 → pfl-forge が `.forge/intents/` に変換
- **Audit** — Audit Agent が `.forge/intents/` に直接生成
- **Epiphany** — 実装中にエージェントが判断: action 必要 → `.forge/intents/` に生成、それ以外 → `.forge/observations.yaml` に記録
- **Reflection** — Reflect Agent が `.forge/intents/` に直接生成

### 2. 固定パイプライン → タスク性質に応じた柔軟 Flow

タスクの種類ごとにデフォルト Flow テンプレートを持ち、実行中にルールベースで調整する。

| タスク種別 | デフォルト Flow |
|-----------|---------------|
| `refactor` | `[implement]` |
| `feature` | `[analyze, implement, review]` |
| `fix` | `[analyze, implement]` |
| `audit` | `[audit, report]` |
| `test` | `[analyze, implement]` |
| `skill_extraction` | `[observe, abstract, record]` |

### 3. ステートレス実行 → 学習する開発パートナー

- **Skills** — 繰り返しパターンをテンプレート化
- **Rules** — プロジェクト固有の規約を学習
- **History** — 成功・失敗・リジェクトの履歴を蓄積

```
Observe/Audit → Discovery → Risk Assessment → 自律実行 or 提案
     ^                                              |
     +──── Learning / Pattern Accumulation ←────────+
```

---

## レイヤー構成

```
CLI Layer
  │
Intent Registry          ← Intent の登録・管理
  │
Quick Classification     ← ルールベースで種別・Flow・リスク決定
  │
Runner                   ← Flow ステップの逐次実行 + ルールベース調整
  │
Reflect Agent            ← タスク完了後の振り返り
  │
Knowledge Base           ← Skills / Rules / History
```

---

## Runner

Flow ステップの逐次実行とルールベースの Flow 調整を行う。AI エージェントではなく Rust コードによる制御ロジック。

全エージェントは実行中に `.forge/observations.yaml` へ気づきを書き出せる。

詳細は [runner.md](runner.md) を参照。

---

## Knowledge Base

二系統で管理する:

### Skills（`.claude/skills/`）

Claude Code のネイティブ skill 機能をそのまま活用する。

```
.claude/skills/
  <name>/SKILL.md    ← YAML frontmatter + markdown（Claude Code 標準フォーマット）
```

- Claude Code が description ベースで関連スキルを自動注入する
- Implement Agent（`claude -p`）でも自動的にスキルが読み込まれる
- 自前のプロンプト注入の仕組みは不要

### Rules / History（`.forge/knowledge/`）

```
.forge/knowledge/
  rules/     ← プロジェクト固有の規約
  history/   ← 実行記録
```

フォーマット: 当面は YAML。

スケール見積もり:
- Skills: プロジェクトあたり 10-50 個。Claude Code が自動注入
- Rules: 20-100 個。プロンプトに全量注入可能
- History: 無制限に増加。将来 pgvector 等への移行が必要になる可能性あり

History エントリのフィールド:
- intent メタデータ（type, source, risk, title）
- 実行された Flow（ステップ一覧 + 調整内容）
- 各ステップの結果と所要時間
- 最終結果（success / failed / escalated）+ 失敗理由
- 生成された observation への参照
- タイムスタンプ

History は「構造化されたサマリ」。agent 内部の操作ログ（個別ファイル読み込み等）は記録しない。
プロセスの摩擦や困難は Observation が担う。

Rule の有効性検証:
- Rule に適用履歴（applied_to）を持たせる
- Reflect Agent が History の Before/After データから傾向を分析
- 効果が見られない Rule は削除候補としてフラグ

### Decision Storage（外部連携）

プロジェクト横断の個人的な判断基準・設計思想を保持する外部アプリ。
Project Rules と同列のコンテキストソースとして、各エージェントのプロンプトに事前注入する。

- Project Rules = 「このプロジェクトではこうする」（プロジェクト固有）
- Decision Storage = 「自分はこう考える」（プロジェクト横断）

事前に注入することで Analyze の判断精度が上がり、needs_clarification の発生頻度を下げる。
連携インターフェースの詳細は外部アプリの設計に依存する。

Rules / History はインターフェースを抽象化し、バックエンド変更に備える。
各エージェントのプロンプトに関連コンテキストとして注入する。

---

## CLI サブコマンド

| コマンド | 用途 | 状態 |
|---------|------|------|
| `run` | タスク処理（柔軟 Flow 対応） | 既存・拡張 |
| `audit` | コードベース監査 → Intent 生成 | **新規** |
| `inbox` | 提案された Intent の一覧・承認・却下 | **新規** |
| `approve` | 特定 Intent の承認（例: `approve 3,5,7`） | **新規** |
| `status` | 処理状態の表示 | 既存 |
| `rules` | 学習済み Rules の閲覧・編集 | **新規** |
| `parent` | インタラクティブセッション | 既存 |
| `create` | `.forge/tasks/` に Markdown タスク作成 | 既存・変更 |
| `clean` | worktree クリーンアップ | 既存 |
| `watch` | daemon モード | 既存 |

---

## 想定される日常フロー

```
朝:
  pfl-forge audit           → 監査実行、Intent が inbox に蓄積
  pfl-forge inbox            → 提案された Intent を確認
  pfl-forge approve 3,5,7    → 良いものを承認
  pfl-forge run              → 承認済み + 自動実行 Intent を処理
  （各タスク完了後に Reflect が走り、knowledge が成長）

作業中:
  pfl-forge create "Feature X" "details..."  → 手動タスク作成
  pfl-forge run                              → 処理実行
  （implement 中: 「ここテスト薄い」→ observation → 次回 audit で拾われる）
```

---

## AI バックエンド

**Claude CLI (`claude -p`) を継続使用。**

- ツール実行（Bash, ファイル操作）が CLI 経由で無料
- 実装・監査など重いタスクに必須
- 軽量タスク（分類、リフレクション）でのオーバーヘッドは許容
- 2つのバックエンドのメンテナンスコストを回避

---

## 未決事項

- [ ] リファクタか書き直しか — 現行コードベースへの適用方法
- [ ] `.forge/observations.yaml` のスキーマ
- [ ] `.forge/intents/*.yaml` のスキーマ（全ソース共通）
- [ ] Audit Agent のスコープとプロンプト設計
- [ ] Knowledge Base（Rules / History）のインターフェース抽象化の設計
- [ ] Rule の YAML 表現形式
- [ ] Runner の Flow 調整ルールの全容（上記は例示）
- [ ] Decision Storage との連携インターフェース

---

## 関連ドキュメント

- [runner.md](runner.md) — Runner の仕様・実行フロー・Flow 調整ルール
- [agents.md](agents.md) — エージェント構成・責務・Knowledge Base との関係
- [data-model.md](data-model.md) — Intent, Quick Classification 等のデータモデル定義
