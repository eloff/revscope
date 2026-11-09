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

# A new name for the Postgres schema of the e2e tests in a worktree on branch $1. Random, so it can
# never be the schema of a worktree that's in use, whatever has happened to that one's path or branch.
e2e_schema() {
  local slug
  slug=$(printf %s "$1" | tr -c 'a-zA-Z0-9' _ | tr 'A-Z' 'a-z' | cut -c1-40)
  echo "e2e_${slug}_$(od -An -N4 -tx1 /dev/urandom | tr -d ' \n')"
}

# The DATABASE_URL in the env file $1, or nothing if there's no such file.
e2e_url() {
  [ ! -f "$1" ] || sed -n 's/^DATABASE_URL=//p' "$1" | head -1
}

# Gives the worktree at $2, on the new branch $3, its own schema in the main checkout $1's e2e
# database, and an .e2e.env selecting it. The tests' background workers scan every project, so
# sessions sharing a schema take each other's jobs. Without a usable database the worktree links the
# shared file instead, since a change that needs no backend tests shouldn't be blocked.
e2e_worktree_env() {
  local main=$1 dest=$2 branch=$3 file=revscope/.e2e.env url schema separator error=
  [ -e "$main/$file" ] && git -C "$dest" check-ignore -q "$file" || return 0
  url=$(e2e_url "$main/$file")
  url=${url%[?&]}
  schema=$(e2e_schema "$branch")
  if ! [[ $url =~ ^postgres(ql)?://[^[:space:]\'\"#\$\\]+$ ]] || [[ $url == *options=* || $url == *\?*@* ]]; then
    # A ? before the @ is in the password, and would be taken for the start of the query below.
    error="its DATABASE_URL isn't a plain postgres:// URL without options"
  elif ! error=$(PGCONNECT_TIMEOUT=5 psql "$url" -Xq -v ON_ERROR_STOP=1 -c "CREATE SCHEMA $schema" 2>&1); then
    :
  else
    [[ $url == *\?* ]] && separator='&' || separator='?'
    (
      umask 077
      awk -v line="DATABASE_URL=$url${separator}options=-csearch_path%3D$schema" \
        '/^DATABASE_URL=/ { print line; next } 1' "$main/$file" >"$dest/$file"
    )
    return 0
  fi
  echo "warning: this worktree's backend tests will share the e2e schema with other sessions, since it couldn't have its own: $error" >&2
  link_ignored "$main" "$dest" "$file"
}

# Drops the schema a worktree's DATABASE_URL $1 selects, if it does. Read from the URL rather than
# worked out again, so a renamed branch still finds it.
e2e_drop_schema() {
  local url=$1 schema error
  [[ $url =~ [?\&]options=-csearch_path%3D(e2e_[a-z0-9_]+)$ ]] || return 0
  schema=${BASH_REMATCH[1]}
  # The timeout is so a hung test holding a table can't keep land.sh, and its lock, waiting forever.
  error=$(PGCONNECT_TIMEOUT=5 psql "$url" -Xq -v ON_ERROR_STOP=1 \
    -c "SET lock_timeout = '5s'" -c "DROP SCHEMA IF EXISTS $schema CASCADE" 2>&1) ||
    echo "warning: couldn't drop the e2e schema $schema, which is harmless but stays behind: $error" >&2
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
