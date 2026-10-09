read_host_crates() {
  local list line
  list="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/host_crates.txt"
  HOST_CRATES=()
  if [[ ! -f "$list" ]]; then
    echo "read_host_crates: missing $list" >&2
    return 1
  fi
  while IFS= read -r line || [[ -n "$line" ]]; do
    line="${line%%#*}"
    line="${line//[[:space:]]/}"
    [[ -n "$line" ]] && HOST_CRATES+=("$line")
  done <"$list"
  if (( ${#HOST_CRATES[@]} == 0 )); then
    echo "read_host_crates: $list lists no crates" >&2
    return 1
  fi
}
