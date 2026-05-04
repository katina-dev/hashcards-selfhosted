# hashcards Docker example

## Quick start

1. Build the image from the repo root:

       docker build -t hashcards:latest .

2. Edit `docker-compose.yml` and replace `/path/to/your/cards` with the
   absolute path to your card directory on the host.

3. Start:

       docker compose up -d

4. Open http://localhost:8000 in your browser.

## Windows hosts

If your cards directory is on a Windows path (not in WSL2's filesystem),
add a polling fallback to the compose `command:` field:

    command: ["serve", "/cards", "--rescan-interval", "30s"]

This is needed because inotify events from Windows-side edits don't
reliably propagate into WSL2 containers.

## Backup

The SQLite database lives at `/cards/hashcards.db` inside the
container, which means it lives in your bind-mounted host directory.
Back up that directory and you back up everything (cards + progress).
