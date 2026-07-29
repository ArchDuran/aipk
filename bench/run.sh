#!/usr/bin/env bash
# Provenance benchmark: vanilla RAG vs --strict-render, both measured by `aipk verify`.
# Metric: share of factual sentences grounded in canonical claims (coverage, 0..1).
# Corpus is fictional (Meridian Robotics) so the base model cannot know it from pretraining.
#
# Usage: bench/run.sh [--rebuild]     env: MODEL=llama3.2 AIPK=target/release/aipk
set -uo pipefail
cd "$(dirname "$0")/.."

AIPK=${AIPK:-target/release/aipk}
MODEL=${MODEL:-llama3.2}
WORK=bench/work
PROJ=$WORK/meridian
PKG=$PROJ/meridian.aipk

if [ "${1:-}" = "--rebuild" ]; then rm -rf "$WORK"; fi

if [ ! -f "$PKG" ]; then
    echo "== building benchmark package =="
    mkdir -p "$WORK"
    "$AIPK" init meridian --dir "$PROJ"
    # pipeline = add-docs + extract-claims + build, включая CLMV (векторы claims);
    # ручная цепочка add-docs/extract-claims/build CLMV не создаёт.
    "$AIPK" pipeline bench/corpus/*.md --dir "$PROJ" --model "$MODEL" --auto-promote -o "$PKG"
fi

RES=bench/results.md
{
    echo "# Provenance benchmark — vanilla RAG vs strict-render"
    echo
    echo "Model: \`$MODEL\` · corpus: fictional (3 docs) · verifier: \`aipk verify\`"
    echo
    echo "| # | type | question | vanilla cov | strict cov |"
    echo "|---|------|----------|-------------|------------|"
} > "$RES"

n=0
v_refusals=0; s_refusals=0; out_total=0
declare -a V_IN S_IN V_OUT S_OUT
while IFS= read -r line; do
    q=$(jq -r .q <<<"$line"); typ=$(jq -r .type <<<"$line"); n=$((n+1))
    v_ans=$("$AIPK" run "$PKG" "$q" --model "$MODEL" 2>/dev/null)
    s_ans=$("$AIPK" run "$PKG" "$q" --model "$MODEL" --strict-render 2>/dev/null)
    if [ "$typ" = "out" ]; then
        out_total=$((out_total+1))
        grep -qi 'insufficient grounded\|not.*in.*the.*provided\|cannot answer\|no information' <<<"$v_ans" && v_refusals=$((v_refusals+1))
        grep -qi 'insufficient grounded\|not.*in.*the.*provided\|cannot answer\|no information' <<<"$s_ans" && s_refusals=$((s_refusals+1))
    fi
    v_cov=$("$AIPK" verify "$PKG" "$v_ans" --json 2>/dev/null | jq -r '.summary.coverage // 0')
    s_cov=$("$AIPK" verify "$PKG" "$s_ans" --json 2>/dev/null | jq -r '.summary.coverage // 0')
    v_cov=${v_cov:-0}; s_cov=${s_cov:-0}
    if [ "$typ" = "in" ]; then V_IN+=("$v_cov"); S_IN+=("$s_cov"); else V_OUT+=("$v_cov"); S_OUT+=("$s_cov"); fi
    printf "| %d | %s | %s | %s | %s |\n" "$n" "$typ" "${q:0:60}" "$v_cov" "$s_cov" >> "$RES"
    printf "q%-2d [%s]  vanilla=%-6s strict=%s\n" "$n" "$typ" "$v_cov" "$s_cov"
done < bench/questions.jsonl

avg() { printf '%s\n' "$@" | awk '{s+=$1} END {if (NR) printf "%.3f", s/NR; else printf "n/a"}'; }
{
    echo
    echo "## Summary"
    echo
    echo "| slice | vanilla | strict-render |"
    echo "|-------|---------|---------------|"
    echo "| in-corpus avg coverage | $(avg "${V_IN[@]:-0}") | $(avg "${S_IN[@]:-0}") |"
    echo "| out-of-corpus avg coverage | $(avg "${V_OUT[@]:-0}") | $(avg "${S_OUT[@]:-0}") |"
    echo "| out-of-corpus refusal rate | $v_refusals/$out_total | $s_refusals/$out_total |"
    echo
    echo "Coverage = доля предложений ответа, подтверждённых canonical claims."
    echo "In-corpus: выше = лучше. Out-of-corpus: показывает, галлюцинирует ли режим на незнаемом."
} >> "$RES"

echo; echo "== summary written to $RES =="
tail -8 "$RES"
