#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
compose_file="$script_dir/docker-compose.yml"

printf 'This stops RustEVM and permanently deletes the rust-evm-data chain volume.\n'
printf 'Type "reset" to continue: '
read -r confirmation

if [[ "$confirmation" != "reset" ]]; then
  printf 'Reset cancelled; chain data was not changed.\n'
  exit 1
fi

docker compose \
  --project-directory "$script_dir" \
  --file "$compose_file" \
  down --volumes --remove-orphans --timeout 120