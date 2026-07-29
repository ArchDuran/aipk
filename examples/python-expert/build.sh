#!/usr/bin/env bash
# Собирает python-expert.aipk из этой директории
# Требует: pip install "aipk[build]"
set -e

DIR="$(cd "$(dirname "$0")" && pwd)"
OUT="${1:-python-expert.aipk}"

echo "Building $OUT …"

# Добавляем навыки
for skill in "$DIR/skills"/*.md; do
  echo "  + skill: $(basename "$skill")"
  aipk add-skill "$skill" --project "$DIR"
done

# Добавляем документы (создаёт эмбеддинги)
for doc in "$DIR/docs"/*.md; do
  echo "  + docs:  $(basename "$doc")"
  aipk add-docs "$doc" --project "$DIR"
done

# Собираем пакет
aipk build --project "$DIR" --output "$OUT"

echo ""
echo "Done! Inspect with:"
echo "  aipk info $OUT"
echo ""
echo "Run with:"
echo "  aipk serve $OUT --model llama3.2 --port 8080"
