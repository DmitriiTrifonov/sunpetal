#!/usr/bin/env bash
# Сверка файлов, изменённых исполнителем в worktree, со списком из поручения.
# Использование: tools/check-diff.sh <worktree> <разрешённый путь или префикс/> ...
# Падает (1), если изменён (в т.ч. не отслеживаемый) файл вне списка. Префикс с «/» на конце — каталог.
set -u
wt="${1:?worktree}"; shift
[ "$#" -gt 0 ] || { echo "нужен хотя бы один разрешённый путь" >&2; exit 2; }
changed=$({ git -C "$wt" diff --name-only HEAD; git -C "$wt" ls-files --others --exclude-standard; } | sort -u)
bad=0
while IFS= read -r f; do
  [ -z "$f" ] && continue
  ok=0
  for a in "$@"; do
    case "$a" in */) [[ "$f" == "$a"* ]] && ok=1 ;; *) [ "$f" = "$a" ] && ok=1 ;; esac
  done
  [ "$ok" = 1 ] || { echo "вне списка: $f" >&2; bad=1; }
done <<< "$changed"
[ "$bad" = 0 ] && echo "diff в рамках списка" >&2
exit "$bad"
