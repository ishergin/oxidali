#!/usr/bin/env bash
cd "$(dirname "$0")/.."

set -u

BASE="${BASE:-http://192.168.11.137}"
ADAPTER="${ADAPTER:-0}"
DEVICES="${DEVICES:-0 1 2 3}"
TARGET_DEVICES="${TARGET_DEVICES:-$DEVICES}"
ITERATIONS="${ITERATIONS:-0}"
GAP="${GAP:-0}"
CURL_TIMEOUT="${CURL_TIMEOUT:-30}"
OP_TIMEOUT="${OP_TIMEOUT:-40}"
OP_POLL="${OP_POLL:-0.5}"
NO_WAIT="${NO_WAIT:-0}"

ATTR_BODY='{"attribute_groups":["runtime_status","common_102","dt8_color","dt6_led","groups","scenes"],"memory_banks":"all"}'

TARGET_LABELS=(cct2700_l245 cct5000_l245 cct6400_l100 cct6400_l200 cct3000_l254 cct6000_l254 rgb_blue rgb_green rgb_red cct6400_l1 cct6400_l0 power_off)
TARGET_BODIES=(
  '{"power":"on","color_temperature_kelvin":2700,"level":245}'
  '{"power":"on","color_temperature_kelvin":5000,"level":245}'
  '{"power":"on","color_temperature_kelvin":6400,"level":100}'
  '{"power":"on","color_temperature_kelvin":6400,"level":200}'
  '{"power":"on","color_temperature_kelvin":3000,"level":254,"color_mode":"cct"}'
  '{"power":"on","color_temperature_kelvin":6000,"level":254,"color_mode":"cct"}'
  '{"power":"on","rgb":{"r":0,"g":0,"b":254},"level":254,"color_mode":"rgb"}'
  '{"power":"on","rgb":{"r":0,"g":254,"b":0},"level":254,"color_mode":"rgb"}'
  '{"power":"on","rgb":{"r":254,"g":0,"b":0},"level":254,"color_mode":"rgb"}'
  '{"power":"on","color_temperature_kelvin":6400,"level":1}'
  '{"power":"on","color_temperature_kelvin":6400,"level":0}'
  '{"power":"off"}'
)

total_req=0
bad_http=0
bad_op=0
START_EPOCH=$(date +%s)

ts() { date '+%H:%M:%S'; }

log() { printf '%s | %s\n' "$(ts)" "$*"; }

gap() { [ "$GAP" != "0" ] && sleep "$GAP"; return 0; }

req() {
  local method="$1" path="$2" body="${3:-}"
  local out
  if [ -n "$body" ]; then
    out=$(curl -sS -m "$CURL_TIMEOUT" -w $'\n%{http_code}' \
      -X "$method" "${BASE}${path}" \
      -H 'Content-Type: application/json' -d "$body" 2>&1)
  else
    out=$(curl -sS -m "$CURL_TIMEOUT" -w $'\n%{http_code}' \
      -X "$method" "${BASE}${path}" 2>&1)
  fi
  local code rest
  code=$(printf '%s' "$out" | tail -n1)
  rest=$(printf '%s' "$out" | sed '$d' | tr -d '\n')
  total_req=$((total_req + 1))
  case "$code" in
    2*) : ;;
    *) bad_http=$((bad_http + 1)) ;;
  esac
  printf '%s\t%s' "$code" "$rest"
}

json_field() {
  printf '%s' "$1" | sed -n "s/.*\"$2\":\"\\([^\"]*\\)\".*/\\1/p"
}

ATTR_OUTCOMES_LOG=$(mktemp "${TMPDIR:-/tmp}/d2r-attr-outcomes.XXXXXX")

record_attr_read_outcomes() {
  local label="$1" body="$2" pairs
  pairs=$(printf '%s' "$body" \
    | sed -n 's/.*"attribute_read_outcomes":{\([^}]*\)}.*/\1/p' \
    | tr ',' '\n' | tr -d '"' | grep -E 'abort$' || true)
  [ -z "$pairs" ] && return 0
  printf '%s\n' "$pairs" | sed "s/^/$label /" >> "$ATTR_OUTCOMES_LOG"
  log "    $label attr-outcomes: $(printf '%s' "$pairs" | tr '\n' ' ')"
}

wait_operation() {
  local op_id="$1" label="$2"
  [ -z "$op_id" ] && { log "    $label: no operation_id returned"; return 1; }
  local deadline=$(( $(date +%s) + OP_TIMEOUT ))
  while :; do
    local r code body status
    r=$(req GET "/api/v1/operations/${op_id}")
    code="${r%%$'\t'*}"; body="${r#*$'\t'}"
    status=$(json_field "$body" status)
    case "$status" in
      succeeded) log "    $label [$op_id] -> succeeded"; return 0 ;;
      failed|timed_out|cancelled)
        bad_op=$((bad_op + 1))
        log "    $label [$op_id] -> $status  (body=$body)"
        case "$label" in attr-read*) record_attr_read_outcomes "$label" "$body" ;; esac
        return 1 ;;
      "")
        : ;;
    esac
    if [ "$(date +%s)" -ge "$deadline" ]; then
      bad_op=$((bad_op + 1))
      log "    $label [$op_id] -> WAIT TIMEOUT (last status='${status:-none}', http=$code)"
      return 1
    fi
    sleep "$OP_POLL"
  done
}

run_async() {
  local label="$1" method="$2" path="$3" body="${4:-}"
  local r code resp op_id
  r=$(req "$method" "$path" "$body")
  code="${r%%$'\t'*}"; resp="${r#*$'\t'}"
  op_id=$(json_field "$resp" operation_id)
  log "  -> $label: HTTP $code op=${op_id:-?}"
  gap
  [ "$NO_WAIT" = "1" ] && return 0
  wait_operation "$op_id" "$label"
}

run_sync() {
  local label="$1" method="$2" path="$3" body="$4"
  local r code resp
  r=$(req "$method" "$path" "$body")
  code="${r%%$'\t'*}"; resp="${r#*$'\t'}"
  [ "${#resp}" -gt 160 ] && resp="${resp:0:160}…"
  log "  -> $label: HTTP $code ${resp:+resp=$resp}"
  gap
}

iteration() {
  local n="$1"
  log "==== iteration $n (attr: $DEVICES | target: $TARGET_DEVICES) ===="

  run_async "discovery" POST "/api/v1/adapters/${ADAPTER}/discovery-runs" \
    '{"mode":"scan_known_short_addresses"}'

  for dev in $DEVICES; do
    run_async "attr-read dev$dev" POST \
      "/api/v1/adapters/${ADAPTER}/physical-devices/${dev}/attribute-reads" "$ATTR_BODY"
  done

  local body_count=${#TARGET_BODIES[@]}
  for dev in $TARGET_DEVICES; do
    local ti=$(( (n + dev) % body_count ))
    local label=${TARGET_LABELS[$ti]}
    local body=${TARGET_BODIES[$ti]}
    run_sync "target dev$dev $label" PUT \
      "/api/v1/adapters/${ADAPTER}/physical-devices/${dev}/target-state" "$body"
  done
}

summary() {
  local dur=$(( $(date +%s) - START_EPOCH ))
  echo
  log "==== summary: requests=$total_req  non2xx=$bad_http  failed_ops=$bad_op  duration=${dur}s ===="
  local pd_json pd_count
  pd_json=$(curl -sf --max-time 10 "$BASE/api/v1/adapters/${ADAPTER}/physical-devices" 2>/dev/null) || pd_json=""
  if [ -n "$pd_json" ]; then
    pd_count=$(printf '%s' "$pd_json" | grep -o '"short_address"' | wc -l | tr -d ' ')
    log "==== registry: adapter=$ADAPTER physical_device_records=$pd_count ===="
  else
    log "==== registry: adapter=$ADAPTER physical-devices list unavailable ===="
  fi
  if [ -s "$ATTR_OUTCOMES_LOG" ]; then
    log "==== attr-read failure breakdown (count device group=class) ===="
    sort "$ATTR_OUTCOMES_LOG" | uniq -c | while read -r line; do log "  $line"; done
  fi
  rm -f "$ATTR_OUTCOMES_LOG"
}

trap 'summary; exit 130' INT TERM

log "load test against $BASE  (adapter=$ADAPTER, iterations=${ITERATIONS:-inf}, wait=$([ "$NO_WAIT" = 1 ] && echo no || echo yes))"

i=1
while :; do
  iteration "$i"
  if [ "$ITERATIONS" != "0" ] && [ "$i" -ge "$ITERATIONS" ]; then
    break
  fi
  i=$((i + 1))
done

summary
