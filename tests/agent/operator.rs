use pfl_forge::agent::operator;
use pfl_forge::prompt;

#[test]
fn intentがない場合のメッセージ() {
  let dir = tempfile::tempdir().unwrap();
  let msg = operator::build_initial_message(dir.path());
  assert!(msg.contains("No intents found"));
}

#[test]
fn 状態サマリにintentの件数が含まれる() {
  let dir = tempfile::tempdir().unwrap();
  let intents_dir = dir.path().join(".forge").join("intents");
  std::fs::create_dir_all(&intents_dir).unwrap();

  std::fs::write(
    intents_dir.join("feat-a.yaml"),
    "title: Feature A\nbody: Do A\nsource: human\nstatus: approved\n",
  )
  .unwrap();
  std::fs::write(
    intents_dir.join("feat-b.yaml"),
    "title: Feature B\nbody: Do B\nsource: human\nstatus: done\n",
  )
  .unwrap();

  let msg = operator::build_initial_message(dir.path());
  assert!(msg.contains("Total: 2 intents"));
  assert!(msg.contains("approved: 1"));
  assert!(msg.contains("done: 1"));
}

#[test]
fn inboxにproposedとblockedが含まれる() {
  let dir = tempfile::tempdir().unwrap();
  let intents_dir = dir.path().join(".forge").join("intents");
  std::fs::create_dir_all(&intents_dir).unwrap();

  std::fs::write(
    intents_dir.join("pending.yaml"),
    "title: Pending feature\nbody: body\nsource: human\nstatus: proposed\n",
  )
  .unwrap();
  std::fs::write(
    intents_dir.join("stuck.yaml"),
    "title: Stuck feature\nbody: body\nsource: human\nstatus: blocked\n",
  )
  .unwrap();
  std::fs::write(
    intents_dir.join("ok.yaml"),
    "title: Done feature\nbody: body\nsource: human\nstatus: done\n",
  )
  .unwrap();

  let msg = operator::build_initial_message(dir.path());
  assert!(msg.contains("## Inbox"));
  assert!(msg.contains("pending"));
  assert!(msg.contains("stuck"));
  assert!(!msg.contains("ok")); // done intents shouldn't be in inbox
}

#[test]
fn clarification待ちintentにラベルがつく() {
  let dir = tempfile::tempdir().unwrap();
  let intents_dir = dir.path().join(".forge").join("intents");
  std::fs::create_dir_all(&intents_dir).unwrap();

  std::fs::write(
    intents_dir.join("clarify.yaml"),
    "title: Needs info\nbody: body\nsource: human\nstatus: blocked\nclarifications:\n  - question: What API?\n",
  )
  .unwrap();

  let msg = operator::build_initial_message(dir.path());
  assert!(msg.contains("[needs clarification]"));
}

#[test]
fn clarificationの質問内容がinboxに表示される() {
  let dir = tempfile::tempdir().unwrap();
  let intents_dir = dir.path().join(".forge").join("intents");
  std::fs::create_dir_all(&intents_dir).unwrap();

  std::fs::write(
    intents_dir.join("clarify.yaml"),
    "title: Needs info\nbody: body\nsource: human\nstatus: blocked\nclarifications:\n  - question: Which API version?\n  - question: Use REST or gRPC?\n    answer: REST\n",
  )
  .unwrap();

  let msg = operator::build_initial_message(dir.path());
  // Unanswered question is shown
  assert!(msg.contains("Q: Which API version?"));
  // Answered question is not shown
  assert!(!msg.contains("Q: Use REST or gRPC?"));
}

#[test]
fn プロンプトがbackground実行後のポーリングを禁止する() {
  let prompt = prompt::OPERATOR;
  assert!(
    prompt.contains("do NOT poll"),
    "operator prompt must instruct not to poll after --background"
  );
  assert!(
    !prompt.contains("monitor progress"),
    "operator prompt must not encourage monitoring progress"
  );
}
