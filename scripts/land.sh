#!/usr/bin/env bash
# Usage: scripts/land.sh [-m <message>]... [-F <file>] [--keep]
#
# For the maintainer, in place of a pull request. Lands the branch of the worktree this script is
# in, along with the private repos' worktrees inside it, all with everything committed. Each repo
# with commits of its own is squashed into one signed commit on its origin/main, replacing them
# (the old tips are printed, and stay in the reflog). check.sh runs on the result, then the private
# repos push to main, then this one, with a sign-off. Last, the worktrees and branches are deleted
# (--keep leaves them), unless one changed meanwhile.
#
# Signing uses git's own settings (user.signingkey, and gpg.format for an SSH key). GitHub only
# counts the signature if that key is on the account and the committer's email is verified there.
#
# The message (-m or -F) is for this repo's commit, which defaults to its own commit's when it has
# one. A private repo always uses its first commit's message, since any later ones are usually
# fixes for the checks.
#
# One landing runs at a time, so sessions landing together queue instead of racing each other.
set -euo pipefail

message=() file= keep= msgs=()
declare -A count main_was landed_at
trap 'rm -f "${msgs[@]}"' EXIT

# Refuses, before anything is changed, a worktree at $1 that can't land: one with uncommitted
# changes, not on its own branch, whose main checkout's main has commits origin/main lacks, or
# whose repo can't sign commits.
preflight() {
  local dir=$1 main branch base
  main=$(main_checkout "$dir")
  [ -z "$(git -C "$dir" status --porcelain)" ] || die "$dir has uncommitted changes. Commit (or remove) them first."
  branch=$(git -C "$dir" symbolic-ref -q --short HEAD) || die "$dir isn't on a branch"
  [ "$branch" != main ] || die "$dir is on main. Work on the branch scripts/worktree-new.sh made."
  git -C "$dir" fetch -q origin
  base=$(git -C "$dir" merge-base HEAD origin/main)
  count[$dir]=$(git -C "$dir" rev-list --count "$base..HEAD")
  [ "${count[$dir]}" = 0 ] && return
  # Landing used to move a local main that was ahead of origin back to it, dropping its commits.
  local ahead
  ahead=$(git -C "$main" rev-list --count origin/main..main)
  [ "$ahead" = 0 ] || die "local main in $main has $ahead commit(s) that aren't on origin/main. Don't push, drop or rebase them: stop and tell the maintainer, who decides whether they land."
  main_was[$dir]=$(git -C "$main" rev-parse main)
  # Signing a throwaway commit also catches a key that's configured but not in the agent.
  git -C "$dir" commit-tree -S -m "land.sh signing check" "HEAD^{tree}" >/dev/null ||
    die "git couldn't make a signed commit in $dir, for the reason above. If that's the key, set user.signingkey (and gpg.format ssh for an SSH key), and make sure the key is available."
}

# Writes to $3 the squashed commit's message for the worktree at $1, the public repo's when $2 is set.
message_for() {
  local dir=$1 public=$2 out=$3 base
  base=$(git -C "$dir" merge-base HEAD origin/main)
  if [ -z "$public" ]; then
    git -C "$dir" log -1 --format=%B "$(git -C "$dir" rev-list --reverse "$base..HEAD" | head -1)" | git stripspace >"$out"
  elif [ "${count[$dir]}" = 1 ] && [ -z "$file" ] && [ ${#message[@]} = 0 ]; then
    git -C "$dir" log -1 --format=%B HEAD | git stripspace >"$out"
  elif [ -n "$file" ]; then
    git stripspace <"$file" >"$out"
  elif [ ${#message[@]} -gt 0 ]; then
    printf '%s\n\n' "${message[@]}" | git stripspace >"$out"
  else
    die "$dir has ${count[$dir]} commits: pass the squashed commit's message with -m or -F"
  fi
  [ -s "$out" ] || die "the message for $dir is empty"
  if [ -n "$public" ]; then
    local name email
    name=$(git -C "$dir" config user.name) && email=$(git -C "$dir" config user.email) ||
      die "set git's user.name and user.email, which the sign-off needs"
    git interpret-trailers --in-place --if-exists addIfDifferent --trailer "Signed-off-by: $name <$email>" "$out"
  fi
}

# Squashes the branch of the worktree at $1 onto its origin/main, as the public repo's commit when
# $2 is set. A branch with no commits of its own is moved to its origin/main instead, so the checks
# build against the latest.
squash() {
  local dir=$1 public=$2 msg tree commit old
  if [ "${count[$dir]}" = 0 ]; then
    git -C "$dir" reset -q --hard origin/main
    return
  fi
  msg=$(mktemp)
  msgs+=("$msg")
  message_for "$dir" "$public" "$msg"
  tree=$(git -C "$dir" merge-tree --write-tree origin/main HEAD) ||
    die "$dir conflicts with origin/main. Run git rebase origin/main there, resolve, commit, and run this again."
  # commit-tree ignores commit.gpgsign, so this has to ask.
  commit=$(git -C "$dir" commit-tree -S "$tree" -p origin/main -F "$msg") ||
    die "git couldn't make the signed squashed commit in $dir, for the reason above. Nothing was pushed, and a repo already squashed can land as it is. Fix that, and run this again."
  old=$(git -C "$dir" rev-parse HEAD)
  git -C "$dir" reset -q --hard "$commit"
  echo "$dir: squashed ${count[$dir]} commit(s) into $(git -C "$dir" rev-parse --short HEAD) (the old tip was $old)"
}

# Prints the output $1 of a successful push, minus the two rule bypasses every landing makes by
# pushing straight to main. Any other bypass still shows, under its heading.
show_push_output() {
  local shown
  # -a because grep would otherwise drop everything after an invalid byte, hiding any other bypass.
  shown=$(grep -aEv '^remote: - (Changes must be made through a pull request\.|Cannot update this protected ref\.) *$' <<<"$1" || true)
  grep -aq '^remote: - ' <<<"$shown" ||
    shown=$(grep -aEv '^remote: Bypassed rule violations for refs/heads/main: *$' <<<"$shown" || true)
  if grep -aEvq '^$|^remote: *$' <<<"$shown"; then
    printf '%s\n' "$shown" >&2
  fi
}

# Pushes the squashed branch of the worktree at $1 as main, and brings its main checkout's main along.
publish() {
  local dir=$1 main output
  main=$(main_checkout "$dir")
  output=$(git -C "$dir" push -q origin "${landed_at[$dir]}:refs/heads/main" 2>&1) || {
    # Unfiltered, since GitHub words a refusal with the same lines it reports for a bypass.
    printf '%s\n' "$output" >&2
    die "the push from $dir was refused, probably because main moved. Its branch is now the squashed commit. Run this again."
  }
  show_push_output "$output"
  echo "$dir: landed $(git -C "$dir" rev-parse --short "${landed_at[$dir]}") on origin/main"
  if [ "$(git -C "$main" symbolic-ref -q HEAD)" = refs/heads/main ]; then
    git -C "$main" merge -q --ff-only origin/main ||
      echo "warning: couldn't fast-forward main in $main; run git merge --ff-only origin/main there" >&2
  else
    # Only from the value checked before squashing, so a commit made to main meanwhile isn't dropped.
    git -C "$main" update-ref refs/heads/main origin/main "${main_was[$dir]}" ||
      echo "warning: main moved while landing; it's left as it is in $main" >&2
  fi
}

land() {
  while [ $# -gt 0 ]; do
    case $1 in
    -m) message+=("$2"); shift 2 ;;
    -F) file=$(cd "$start" && realpath "$2"); shift 2 ;;
    --keep) keep=1; shift ;;
    *) die "unknown argument $1" ;;
    esac
  done

  local top main repo
  top=$(pwd -P)
  main=$(main_checkout)
  [ "$top" != "$main" ] || die "run this from a worktree made by scripts/worktree-new.sh, not the main checkout"

  exec 9>"$(git rev-parse --path-format=absolute --git-common-dir)/land.lock"
  flock -n 9 || { echo "waiting for another landing to finish..."; flock 9; }

  local repos=() nested=() landing=()
  for repo in $nested_repos; do
    if [ -d "$main/$repo/.git" ]; then
      is_worktree_of "$top/$repo" "$main/$repo" ||
        die "$top/$repo isn't a worktree of the private repo in $main/$repo. Make worktrees with scripts/worktree-new.sh."
      nested+=("$top/$repo")
    fi
  done
  repos=("${nested[@]}" "$top")

  for repo in "${repos[@]}"; do
    preflight "$repo"
    [ "${count[$repo]}" -gt 0 ] && landing+=("$repo")
  done
  [ ${#landing[@]} -gt 0 ] || die "nothing to land: no branch has commits of its own"
  for repo in "${repos[@]}"; do
    local public=
    [ "$repo" = "$top" ] && public=1
    squash "$repo" "$public"
    landed_at[$repo]=$(git -C "$repo" rev-parse HEAD)
  done

  local format_commits head folded
  format_commits=$(mktemp)
  msgs+=("$format_commits")
  CHECK_FORMAT_COMMITS=$format_commits scripts/check.sh || die "checks failed. The branches are now the squashed commits. Fix them, commit, and run this again."

  # What the formatters rewrote belongs in the one squashed commit. Any other commit that appeared
  # during the checks is someone's work, which only they can decide to land.
  head=$(git rev-parse HEAD)
  if [ "$head" != "${landed_at[$top]}" ]; then
    git merge-base --is-ancestor "${landed_at[$top]}" "$head" &&
      [ "$(git rev-list --reverse "${landed_at[$top]}..$head")" = "$(cat "$format_commits")" ] ||
      die "$top moved during the checks other than by check.sh's own commits, and nothing was pushed. Deal with that, then run this again."
    folded=$(git log -1 --format=%B "${landed_at[$top]}" | git stripspace | git commit-tree "$head^{tree}" -p "${landed_at[$top]}^" -F -)
    git update-ref HEAD "$folded" "$head"
    landed_at[$top]=$folded
  fi

  # A refused push after another repo's went through would leave the repos' mains out of step, so
  # make sure no main moved during the checks before pushing any.
  for repo in "${landing[@]}"; do
    git -C "$repo" fetch -q origin
    [ "$(git -C "$repo" rev-parse "${landed_at[$repo]}^")" = "$(git -C "$repo" rev-parse origin/main)" ] ||
      die "main moved in $repo's repo during the checks, and nothing was pushed. Run this again."
  done
  # The private repos go first, since they build against this one's main.
  for repo in "${landing[@]}"; do
    publish "$repo"
  done

  [ -n "$keep" ] && return
  local why
  for repo in "${repos[@]}"; do
    why=$(unchanged "$repo" "${landed_at[$repo]}" "${nested[@]}")
    [ -z "$why" ] || die "left the worktrees in place, since $repo changed during the landing: $why"
  done
  cd "$main"
  for repo in "${repos[@]}"; do
    local branch repo_main
    branch=$(git -C "$repo" symbolic-ref --short HEAD)
    repo_main=$(main_checkout "$repo")
    git -C "$repo_main" worktree remove "$repo"
    git -C "$repo_main" branch -q -D "$branch"
    echo "deleted $repo and its branch $branch"
  done
}

start=$PWD
# The worktree this script is in, wherever it's run from, even inside a private repo's worktree.
cd "$(dirname "$0")/.."
. scripts/lib.sh
# Run as a function so bash has read the whole script before land deletes the worktree it lives in.
land "$@"
