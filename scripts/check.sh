#!/usr/bin/env bash
# Usage: scripts/check.sh [--all]
#
# Runs the checks CI runs (.github/workflows/ci.yml), which are stricter than CONTRIBUTING.md's, for
# each part that differs from where this branch left origin/main, counting uncommitted changes, or
# for every part with --all. It leaves out what needs the network or a running server: the
# dependency audits, web's dev-server start, and the e2e tests. Then runs the scripts/check.sh of
# each private repo checked out inside this worktree.
#
# Formatters may rewrite files, as CONTRIBUTING.md expects. On a branch other than main, this commits
# those files and carries on. A file that already had uncommitted changes is formatted but left
# uncommitted, for the commit it's waiting on, and stays staged if it all was. Outside a formatter's
# own run, any change this repo's checks make that git status or git diff shows fails them.
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/lib.sh

all=${1:-}
base=$(git merge-base HEAD origin/main)
changed() {
  [ "$all" = --all ] || [ -n "$(git diff --name-only "$base" -- "$@")" ] || [ -n "$(git ls-files --others --exclude-standard -- "$@")" ]
}

snapshot() { git status --porcelain=v2 -z | sha1sum; git diff | sha1sum; }
# Includes the index, for a staged edit undone in the working tree, and turns off rename detection,
# since a formatter can push a staged rename below its threshold and expose the old name.
uncommitted() {
  { git -c core.quotePath=false diff --no-renames --name-only HEAD
    git -c core.quotePath=false diff --no-renames --cached --name-only; } | LC_ALL=C sort -u
}
# Files whose every change is staged, leaving out staged deletions, which have nothing to restage.
fully_staged() {
  LC_ALL=C comm -23 <(git -c core.quotePath=false diff --cached --name-only --diff-filter=d | LC_ALL=C sort) \
    <(git -c core.quotePath=false diff --name-only | LC_ALL=C sort)
}
before=$(snapshot)
other_changes=
# The full ref, since the short name reads heads/main when a tag is also called main.
head_ref=$(git symbolic-ref -q HEAD || true)
[ "$head_ref" != refs/heads/main ] || head_ref=

# Runs the formatter "$@" in directory $1 and commits the files it rewrote. It commits even if the
# formatter then fails on a lint error, or a later run would mistake those files for someone's edits.
run_formatter() {
  local dir=$1 status=0 earlier now path rewritten=() staged=()
  shift
  [ "$before" = "$(snapshot)" ] || other_changes=1
  earlier=$(uncommitted)
  while IFS= read -r path; do staged+=("$path"); done < <(fully_staged)
  (cd "$dir" && "$@") || status=$?
  if [ -n "$head_ref" ]; then
    now=$(uncommitted)
    while IFS= read -r path; do rewritten+=("$path"); done < <(LC_ALL=C comm -13 <(echo "$earlier") <(echo "$now"))
    if [ ${#rewritten[@]} -gt 0 ]; then
      git --literal-pathspecs commit -q -s -m "Apply formatting" -- "${rewritten[@]}" ||
        die "couldn't commit what a formatter rewrote (see git status). Commit it yourself, then run this again."
      [ -z "${CHECK_FORMAT_COMMITS:-}" ] || git rev-parse HEAD >>"$CHECK_FORMAT_COMMITS"
      echo "committed formatting of ${#rewritten[@]} file(s) as $(git rev-parse --short HEAD)"
    fi
    # Otherwise git add, check.sh, git commit would commit the version from before the formatter.
    [ ${#staged[@]} = 0 ] || git --literal-pathspecs add -- "${staged[@]}"
    before=$(snapshot)
  fi
  return "$status"
}

# The root biome.json is the base of every part's, and the toolchain builds every crate.
biome=; rust=; tracker=; e2e=; crates=; revscope=; cli=; web=
changed biome.json && biome=1
changed rust-toolchain.toml && rust=1
{ [ -n "$biome" ] || changed tracker; } && tracker=1
{ [ -n "$biome" ] || changed e2e; } && e2e=1
{ [ -n "$rust" ] || changed crates; } && crates=1
# The backend embeds the tracker's build (src/ingest/route.rs).
{ [ -n "$crates" ] || [ -n "$tracker" ] || changed revscope; } && revscope=1
{ [ -n "$crates" ] || changed cli; } && cli=1
{ [ -n "$biome" ] || changed web; } && web=1

if [ -n "$tracker" ]; then
  echo "== tracker"
  pnpm_deps tracker
  run_formatter tracker pnpm -s check
  (cd tracker && pnpm -s test && pnpm -s build)
fi
if [ -n "$web" ]; then
  echo "== web"
  pnpm_deps web
  run_formatter web pnpm -s check
  (cd web && pnpm -s tsc && pnpm -s test && pnpm -s build >/dev/null && pnpm -s build:bundle >/dev/null)
fi
if [ -n "$crates" ]; then
  echo "== crates"
  for crate in crates/*/; do
    run_formatter "$crate" cargo fmt
    (cd "$crate" && cargo clippy -q --all-targets -- -D warnings && cargo test -q)
  done
fi
if [ -n "$revscope" ]; then
  echo "== revscope"
  # The self-hosted build embeds the dashboard bundle (see revscope/build.rs), which is build output.
  # The web checks above just built it when they ran; otherwise build it from this tree.
  if [ -z "$web" ]; then
    pnpm_deps web
    (cd web && pnpm -s build:bundle >/dev/null)
  fi
  run_formatter revscope cargo fmt
  (
    cd revscope
    for features in "" self-hosted; do
      cargo clippy -q --all-targets --locked ${features:+--features $features} -- -D warnings
      cargo test -q ${features:+--features $features}
    done
  )
fi
if [ -n "$cli" ]; then
  echo "== cli"
  run_formatter cli cargo fmt
  (cd cli && cargo clippy -q --all-targets --locked -- -D warnings && cargo test -q --locked)
  (cd cli/npm && npm test --silent)
  (cd revscope && cargo test -q --test cli_e2e)
fi
if [ -n "$e2e" ]; then
  echo "== e2e (type check and lint only: its tests need a running target, see CONTRIBUTING.md)"
  pnpm_deps e2e
  run_formatter e2e pnpm -s check
  (cd e2e && pnpm -s tsc)
fi

[ -z "$other_changes" ] && [ "$before" = "$(snapshot)" ] ||
  die "the checks changed files they can't commit themselves (see git status). Review and commit them, then run this again."

# A private repo built with or on this one checks itself when the part it depends on changed too.
export REVSCOPE_CHANGED=$revscope REVSCOPE_WEB_CHANGED=$web
for repo in $nested_repos; do
  if is_worktree_of "$repo" "$(main_checkout)/$repo"; then
    if [ -x "$repo/scripts/check.sh" ]; then
      echo "== $repo"
      (cd "$repo" && scripts/check.sh $all)
    else
      echo "== $repo has no scripts/check.sh, so it isn't checked"
    fi
  fi
done
echo "checks passed"
