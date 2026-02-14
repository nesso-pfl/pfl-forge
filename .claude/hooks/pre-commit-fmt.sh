#!/bin/bash
INPUT=$(cat)
COMMAND=$(echo "$INPUT" | jq -r '.tool_input.command // empty')

if [[ "$COMMAND" == *"git commit"* ]]; then
  cd "$CLAUDE_PROJECT_DIR"
  cargo fmt --all
  git diff --name-only --cached | xargs -r git add
fi

exit 0
