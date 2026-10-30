#!/usr/bin/env bash
# Build the server (release) and link its binaries into ~/.local/bin, so the systemd units (and your shell) run the
# code of this checkout. An existing regular file there is moved to a backup folder first; links are re-pointed
# idempotently. The systemd units in deploy/systemd are copied to ~/.config/systemd/user (the old ones backed up).
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
dest="${HOME}/.local/bin"
units="${HOME}/.config/systemd/user"
backup="${HOME}/.local/share/life-install-backup/$(date +%Y%m%d-%H%M%S)"
bins=(life life-ingest life-diary life-librarian mail-sync mail-files calendar-sync)

cd "$repo"
cargo build --release -p life-server --bins
target="$(cargo metadata --format-version 1 --no-deps | grep -o '"target_directory":"[^"]*"' | cut -d'"' -f4)"
target="${target:-$repo/target}"
mkdir -p "$dest" "$units"

for name in "${bins[@]}"; do
    src="$target/release/$name"
    target_link="$dest/$name"
    if [ -e "$target_link" ] && [ ! -L "$target_link" ]; then
        mkdir -p "$backup"
        mv "$target_link" "$backup/$name"
        echo "backed up $target_link -> $backup/$name"
    fi
    ln -sfn "$src" "$target_link"
    echo "linked $target_link -> $src"
done

for unit in "$repo"/server/deploy/systemd/*; do
    name="$(basename "$unit")"
    if [ -e "$units/$name" ] && ! cmp -s "$unit" "$units/$name"; then
        mkdir -p "$backup"
        cp "$units/$name" "$backup/$name"
        echo "backed up $units/$name -> $backup/$name"
    fi
    cp "$unit" "$units/$name"
done
systemctl --user daemon-reload 2>/dev/null || true
echo "done. restart the service with: systemctl --user restart life-ingest"
