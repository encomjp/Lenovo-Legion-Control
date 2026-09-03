#!/usr/bin/env bash
set -euo pipefail

# Rewrites commits authored/committed by Cursor Agent to your identity.
# Usage:
#   ./cursorfix.sh git@github.com:ORG/REPO.git
#   ./cursorfix.sh https://github.com/encomjp/Lenovo-Legion-Control.git
# Optional overrides:
#   CURSOR_EMAILS="cursoragent@users.noreply.github.com,cursoragent@cursor.com"
#   CURSOR_NAMES="cursoragent,Cursor Agent"
#   CURSOR_SUBSTRINGS="cursoragent,cursor agent,@cursor.com"

REPO_URL="${1:-}"
TARGET_NAME="${TARGET_NAME:-encomjp}"
TARGET_EMAIL="${TARGET_EMAIL:-46200173+encomjp@users.noreply.github.com}"
CURSOR_EMAILS="${CURSOR_EMAILS:-cursoragent@users.noreply.github.com,cursoragent@cursor.com}"
CURSOR_NAMES="${CURSOR_NAMES:-cursoragent,Cursor Agent,Cursoragent}"
CURSOR_SUBSTRINGS="${CURSOR_SUBSTRINGS:-cursoragent,cursor agent,@cursor.com}"
WORKDIR="${WORKDIR:-/tmp/repo-clean-$$.git}"

if [[ -z "$REPO_URL" ]]; then
  echo "ERROR: Missing repository URL."
  echo "Example: $0 https://github.com/encomjp/Lenovo-Legion-Control.git"
  echo "   or:   $0 git@github.com:encomjp/Lenovo-Legion-Control.git"
  exit 1
fi

if [[ -z "$TARGET_NAME" || -z "$TARGET_EMAIL" ]]; then
  cat <<MSG
ERROR: Missing TARGET_NAME or TARGET_EMAIL.
Set them explicitly, for example:
  TARGET_NAME="Your Name" TARGET_EMAIL="you@example.com" $0 git@github.com:ORG/REPO.git
MSG
  exit 1
fi

if ! command -v git >/dev/null 2>&1; then
  echo "ERROR: git is not installed."
  exit 1
fi

if ! git filter-repo -h >/dev/null 2>&1; then
  cat <<'MSG'
ERROR: git-filter-repo is not installed.
Install one of:
  - pipx install git-filter-repo
  - python3 -m pip install --user git-filter-repo
  - brew install git-filter-repo
  - sudo pacman -S git-filter-repo  (Arch)
  - sudo apt install git-filter-repo (Debian/Ubuntu newer)
Then rerun this script.
MSG
  exit 1
fi

if [[ -e "$WORKDIR" ]]; then
  echo "Cleaning existing workdir: $WORKDIR"
  rm -rf "$WORKDIR"
fi

echo "Cloning mirror: $REPO_URL"
git clone --mirror "$REPO_URL" "$WORKDIR"

cd "$WORKDIR"

CALLBACK_FILE="$(mktemp)"
trap 'rm -f "$CALLBACK_FILE"' EXIT

export TARGET_NAME TARGET_EMAIL CURSOR_EMAILS CURSOR_NAMES CURSOR_SUBSTRINGS
cat > "$CALLBACK_FILE" <<'PY'
import os

target_name = os.environ.get("TARGET_NAME", "").encode("utf-8")
target_email = os.environ.get("TARGET_EMAIL", "").encode("utf-8")
cursor_emails = {e.strip().lower().encode("utf-8") for e in os.environ.get("CURSOR_EMAILS", "").split(",") if e.strip()}
cursor_names = {n.strip().lower().encode("utf-8") for n in os.environ.get("CURSOR_NAMES", "").split(",") if n.strip()}
cursor_substrings = [s.strip().lower().encode("utf-8") for s in os.environ.get("CURSOR_SUBSTRINGS", "").split(",") if s.strip()]

def _looks_like_cursor_identity(name: bytes, email: bytes) -> bool:
    nl = name.lower()
    el = email.lower()
    if el in cursor_emails or nl in cursor_names:
        return True
    # Fallback matching when exact name/email is unknown.
    return any(sub in nl or sub in el for sub in cursor_substrings)

if _looks_like_cursor_identity(commit.author_name, commit.author_email):
    commit.author_name = target_name
    commit.author_email = target_email

if _looks_like_cursor_identity(commit.committer_name, commit.committer_email):
    commit.committer_name = target_name
    commit.committer_email = target_email
PY

echo "Rewriting history with git filter-repo..."
echo "  TARGET_NAME=$TARGET_NAME"
echo "  TARGET_EMAIL=$TARGET_EMAIL"
git filter-repo --force --commit-callback "$(cat "$CALLBACK_FILE")"
rm -f "$CALLBACK_FILE"

if ! git remote get-url origin >/dev/null 2>&1; then
  git remote add origin "$REPO_URL"
fi

echo ""
echo "Preview of rewritten refs (local mirror only, not yet pushed):"
git log --all --format='%h %an <%ae> | %cn <%ce> | %s' | head -n 10
echo ""
read -rp "Force push rewritten refs to origin? [y/N] " confirm
if [[ "$confirm" != "y" && "$confirm" != "Y" ]]; then
  echo "Aborted. Mirrored repo kept at: $WORKDIR"
  echo "To push manually later: cd $WORKDIR && git push --force --mirror origin"
  exit 0
fi

echo "Force pushing rewritten refs to origin..."
git push --force --mirror origin

echo "Done. GitHub Contributors can take some time to refresh contributor cache."
echo "Mirrored repo at: $WORKDIR (remove when done: rm -rf $WORKDIR)"
