#!/usr/bin/env bash
# Usage: scripts/worktree-new.sh <branch>
#
# Creates ../revscope-<branch> on a new branch from origin/main, ready to build and test, and prints
# its path. Each private repo the main checkout has inside it gets a worktree on the same branch
# name, at the same place in the new one, set up by its scripts/worktree-setup.sh if it has one.
# If any step fails, everything it made is removed again, so no worktree is left without its
# private repos.
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/lib.sh

branch=${1:?usage: scripts/worktree-new.sh <branch>}
main=$(main_checkout)
case $branch in */*) die "use a branch name without /, which would nest the worktree's directory" ;; esac
git check-ref-format --branch "$branch" >/dev/null || die "$branch isn't a valid branch name"
dest=$(dirname "$main")/$(basename "$main")-$branch
[ -e "$dest" ] && die "$dest already exists"

repos=("$main")
for repo in $nested_repos; do
  [ -d "$main/$repo/.git" ] && repos+=("$main/$repo")
done
for repo in "${repos[@]}"; do
  git -C "$repo" rev-parse -q --verify "refs/heads/$branch" >/dev/null && die "$repo already has a branch $branch"
done

made=() done=
undo() {
  [ -z "$done" ] && [ ${#made[@]} -gt 0 ] || return 0
  local i
  for ((i = ${#made[@]} - 1; i >= 0; i--)); do
    read -r repo path <<<"${made[i]}"
    git -C "$repo" worktree remove --force "$path" && git -C "$repo" branch -q -D "$branch"
  done
  echo "error: setting up $dest failed, so it was removed" >&2
}
trap undo EXIT

add() {
  git -C "$1" fetch -q origin
  git -C "$1" worktree add -q --no-track -b "$branch" "$2" origin/main
  made+=("$1 $2")
}

add "$main" "$dest"
# Gitignored local config. The production env files stay behind, since deploys run from the main
# checkout.
link_ignored "$main" "$dest" CLAUDE.local.md .env revscope/.env revscope/.e2e.env web/.env web/app/.env revscope/GeoLite2-City.mmdb
pnpm_deps "$dest/web"

for repo in "${repos[@]:1}"; do
  path=$dest/${repo#"$main"/}
  add "$repo" "$path"
  if [ -x "$path/scripts/worktree-setup.sh" ]; then
    (cd "$path" && scripts/worktree-setup.sh)
  fi
done

done=1
echo "$dest"
