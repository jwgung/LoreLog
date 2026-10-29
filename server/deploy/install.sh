#!/usr/bin/env bash
# Link the scripts in bin/ into ~/.local/bin so the systemd units (and your shell) run the code in this repo.
# An existing regular file is moved to a backup folder first; links are re-pointed idempotently.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dest="${HOME}/.local/bin"
backup="${HOME}/.local/share/life-install-backup/$(date +%Y%m%d-%H%M%S)"
mkdir -p "$dest"

for src in "$repo"/bin/*; do
    name="$(basename "$src")"
    target="$dest/$name"
    if [ -e "$target" ] && [ ! -L "$target" ]; then
        mkdir -p "$backup"
        mv "$target" "$backup/$name"
        echo "backed up $target -> $backup/$name"
    fi
    ln -sfn "$src" "$target"
    echo "linked $target -> $src"
done
echo "done. restart the service with: systemctl --user restart life-ingest"
