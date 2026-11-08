# Shared by worktree-new.sh, check.sh and land.sh. Sourced, not run.

# Private repos the maintainer checks out inside this one, each gitignored here. A worktree gets
# its own worktree of each one the main checkout has, and the scripts skip those it doesn't.
nested_repos="revscope/src/billing web/site"

die() {
  echo "error: $*" >&2
  exit 1
}

# The main checkout (the first worktree) of the repo at $1, or the current one.
main_checkout() {
  dirname "$(git -C "${1:-.}" rev-parse --path-format=absolute --git-common-dir)"
}

# Whether $1 is the top of a linked worktree of the repo whose main checkout is $2: not a link to
# it, nor the main checkout itself.
is_worktree_of() {
  [ ! -L "$1" ] && [ -f "$1/.git" ] && [ "$(git -C "$1" rev-parse --show-toplevel)" = "$(cd "$1" && pwd -P)" ] &&
    [ "$(cd "$1" && pwd -P)" != "$2" ] && [ "$(main_checkout "$1")" = "$2" ]
}

# Links each of the files after $1 and $2 from the checkout at $1 into the worktree at $2, when it
# exists there and git ignores it, so a link can never be committed or block a landing.
link_ignored() {
  local from=$1 to=$2 path
  shift 2
  for path in "$@"; do
    if [ -e "$from/$path" ] && git -C "$to" check-ignore -q "$path"; then
      mkdir -p "$(dirname "$to/$path")"
      ln -s "$from/$path" "$to/$path"
    fi
  done
}

# Installs a pnpm package's dependencies unless they're up to date with its lockfile, from the local
# store when it can.
pnpm_deps() {
  local modules=$1/node_modules/.modules.yaml
  [ -f "$modules" ] && [ ! "$1/pnpm-lock.yaml" -nt "$modules" ] && return
  (cd "$1" && { pnpm install --offline --frozen-lockfile >/dev/null 2>&1 || pnpm install --frozen-lockfile >/dev/null; })
}

# Prints any git repo inside the worktree at $1 other than its own and the paths after $1, which
# `git worktree remove` would delete along with the worktree, since gitignored directories go too.
stray_repos() {
  local dir=$1 allowed=() found
  shift
  allowed=("$@")
  find "$dir" -mindepth 2 \( -name node_modules -o -name target \) -prune -o -name .git -print 2>/dev/null |
    while read -r found; do
      local keep= path
      for path in "${allowed[@]}"; do
        [ "$found" = "$path/.git" ] && keep=1
      done
      [ -n "$keep" ] || dirname "$found"
    done
}

# Whether the worktree at $1 is still exactly the commit $2, with nothing uncommitted or stray in
# it, so removing it can lose nothing. Prints why not.
unchanged() {
  local dir=$1 want=$2
  shift 2
  if [ "$(git -C "$dir" rev-parse HEAD)" != "$want" ]; then
    echo "it has new commits"
  elif [ -n "$(git -C "$dir" status --porcelain)" ]; then
    echo "it has uncommitted changes"
  elif [ -n "$(stray_repos "$dir" "$@")" ]; then
    echo "it holds another git repo: $(stray_repos "$dir" "$@" | head -1)"
  fi
}
