#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

docker compose \
  --project-directory "$script_dir" \
  --file "$script_dir/docker-compose.yml" \
  up --build --detach

docker compose \
  --project-directory "$script_dir" \
  --file "$script_dir/docker-compose.yml" \
  ps