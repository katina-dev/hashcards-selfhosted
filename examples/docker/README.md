# hashcards Docker example

A reference `docker-compose.yml` for running the `serve` subcommand
as a long-running container on Linux, macOS, or Windows.

## Quick start (Linux / macOS)

1. Build the image from the repo root:

       docker build -t hashcards:latest .

2. Edit `docker-compose.yml`:
   - Replace `/path/to/your/cards` with the absolute path to your card
     directory on the host.
   - Set `TZ` to your IANA timezone (e.g. `America/Los_Angeles`,
     `Europe/Berlin`). This is what makes the daily streak, retention
     window, and review timestamps line up with your wall clock.

3. Start:

       docker compose up -d

4. Open http://localhost:8000.

5. Stop:

       docker compose down

## Quick start (Windows)

The same flow works on Windows with **Docker Desktop**, with one
extra knob and a couple of path-format gotchas to know about.

### Prerequisites

- [Docker Desktop for Windows](https://www.docker.com/products/docker-desktop/),
  with the WSL2 backend enabled (this is the default on modern
  installs). Verify with `docker info` — `Server > OS` should say
  something Linux-y.
- A way to run shell commands. PowerShell works; WSL (Ubuntu) is
  usually friendlier because path handling and quoting behave like
  Linux.

### Where to put your cards directory

Two reasonable choices, with different tradeoffs:

| Location | Path example | Pros | Cons |
|---|---|---|---|
| **Inside WSL2** (recommended) | `/home/you/cards` | Fast file watching, `inotify` works, native Linux perms | Browse from Windows via `\\wsl$\Ubuntu\home\you\cards` |
| **Windows filesystem** | `C:\Users\you\cards` | Browse and edit from Windows Explorer / VS Code naturally | Slower disk; **`inotify` doesn't propagate** across the WSL2 boundary, so you need a polling fallback (see below) |

### Path format in `docker-compose.yml`

YAML can swallow backslashes. Use forward slashes or quote the path:

    volumes:
      - "C:/Users/you/cards:/cards"   # Windows host path
      - "/home/you/cards:/cards"      # WSL2 path (run compose from WSL)

Avoid `C:\Users\you\cards` — even when it works, it's a footgun.

### Polling fallback for Windows-side card edits

If your cards live on the Windows filesystem (anything under `C:\…`,
`/mnt/c/…`), `inotify` events don't reliably reach the container.
Tell `serve` to poll instead by overriding the command in
`docker-compose.yml`:

    command: ["serve", "/cards", "--rescan-interval", "30s"]

You only need this on Windows hosts (or any other setup with a
non-`inotify` filesystem); skip it on Linux / macOS or when your
cards live inside WSL2.

### Running compose from WSL vs PowerShell

Either works. From WSL: `cd` to the repo and run `docker compose
up -d` as you would on Linux. From PowerShell: same command, but be
careful with quoting if you customise the volume path inline.

## Set the timezone

The image bundles `tzdata`. The `TZ` environment variable in
`docker-compose.yml` controls everything date-related inside the
container — the daily streak, retention windows, and the timestamps
written into the SQLite log.

    environment:
      TZ: America/Los_Angeles

To check what the container thinks the current time is:

    docker exec hashcards date

If that prints UTC when you set a different `TZ`, the image probably
predates the `tzdata` change — rebuild with `--no-cache` (see below).

## Updating the image

When you `git pull` a new version of hashcards, rebuild and restart:

    docker compose down
    docker build --no-cache -t hashcards:latest .
    docker compose up -d

`--no-cache` avoids picking up stale cached Rust build layers and
ensures any Dockerfile changes (new system packages, base image
updates) actually take effect.

## Backup

The SQLite database lives at `/cards/hashcards.db` inside the
container, which means it lives in your bind-mounted host directory.
Back up that directory and you've backed up everything (cards +
review history). Stopping the container first gives you a perfectly
consistent snapshot; otherwise SQLite WAL mode is generally safe to
copy live.

## Accessing from other devices on your LAN

The container publishes port 8000 on all host interfaces, so once
it's running point any other device on the same network at
`http://<host-ip>:8000`. To find the host's IP:

- Linux / macOS: `ip addr` or `ifconfig`
- Windows: `ipconfig` (look for the IPv4 address of your active Wi-Fi
  or Ethernet adapter)

If your host firewall blocks inbound port 8000, allow it.

## Troubleshooting

- **Port 8000 already in use** — change the host side of the port
  mapping in `docker-compose.yml`, e.g. `"8001:8000"`, then open
  `http://localhost:8001`.
- **`docker exec hashcards date` shows UTC** despite `TZ` being set —
  rebuild with `--no-cache`; older images didn't include `tzdata`.
- **Edits to cards don't show up** — on Windows with cards on the
  Windows filesystem, add `--rescan-interval 30s` (see above).
  Otherwise, `docker logs hashcards` should show watcher activity
  when you save a file.
- **`/cards` is empty inside the container** — double-check the
  bind-mount path in `docker-compose.yml`; it must be an absolute
  path that exists on the host. On Windows, also make sure Docker
  Desktop has permission to share the drive (Settings → Resources
  → File sharing).
- **Container exits immediately** — `docker logs hashcards` will
  print the reason. The most common one is the `/cards` directory
  missing or unreadable.
