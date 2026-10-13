# Deploying the license server on a VPS

A Docker Compose stack: the license server (this repo's `license-server`) behind **Caddy**, which gets and renews the HTTPS certificate by itself. Only Caddy is exposed; the license server is reachable only through it.

You need: a VPS (any Debian or Ubuntu with 1 vCPU and 512 MB–1 GB RAM is plenty), a domain you control, and the Gumroad product ID.

## 1. Prepare the server

1. Create the VPS and point a DNS **A** (and **AAAA**, if it has IPv6) record for your license domain, e.g. `license.example.com`, at it. Wait until `dig +short license.example.com` returns the server's address; Caddy can't get a certificate before that.
2. Open only ports **22, 80 and 443** in the provider's firewall. Docker publishes ports around `ufw`, so don't rely on `ufw` alone.
3. Install Docker with the Compose plugin ([docs.docker.com/engine/install](https://docs.docker.com/engine/install/)). Use SSH keys and turn off password login.

## 2. Install

```bash
git clone https://github.com/mattrfield/labforge.git
cd labforge/license-server/deploy
cp .env.example .env
nano .env        # LICENSE_DOMAIN, GUMROAD_PRODUCT_ID, TOKEN_TTL_SECS
docker compose build
```

## 3. Create the signing key (once)

```bash
docker compose run --rm license keygen --keys /data/keys
```

This prints the **launcher public key**. Keep that output. The private key lives in the `keys` Docker volume and never leaves the server. **Back it up now**, because if it's lost every installed launcher would need a new public key:

```bash
docker compose run --rm -T --entrypoint cat license /data/keys/license.key > license.key.backup
chmod 600 license.key.backup   # then move it somewhere safe and off the server, e.g. a password manager
```

Later you can print the public key again with `docker compose run --rm license pubkey --keys /data/keys`.

> On Windows with Git Bash, set `MSYS_NO_PATHCONV=1` first or `/data/keys` gets rewritten into a Windows path.

## 4. Start it

```bash
docker compose up -d
curl https://license.example.com/healthz     # -> ok
```

`docker compose logs -f license` shows one line per request (client address, path, status). Keys and tokens are never logged.

## 5. Point the launcher at it

In `src-tauri/src/license.rs` set:

```rust
const SERVER: &str = "https://license.example.com";
const PUBKEY: &str = "<the public key from step 3>";
```

Licensing stays hidden in released launchers until both are set. Then build and tag a launcher release as usual.

## 6. Publish pro labs

Keep pro lab sources **outside this public repo**, for example in a private `labforge-pro-labs` repo with one folder per lab (`<id>/lab.yaml`, `compose.yaml`, …) and `tier: pro` in each `lab.yaml`. From a machine that has python3, rsync and SSH access:

```bash
license-server/deploy/publish-pro-packs.sh user@your-server https://license.example.com ../labforge-pro-labs
```

It builds the packs, uploads them to `deploy/packs/` (packs first, the index last), and the server picks them up immediately. No restart needed. Bump a lab's `version` in `lab.yaml` whenever its files change, or installed copies won't update.

## 7. Test with a real key before announcing

1. In Gumroad create a **100% discount code** for the product (Checkout → Discounts), buy the product yourself, and copy the license key from the receipt.
2. Enter it in the launcher under System → LabForge Pro. Pro labs should unlock and install.
3. Refund that test purchase in Gumroad, then check that renewing the license fails once the current token expires. This is the real test of refund handling, so keep `TOKEN_TTL_SECS` short while testing.

## Operating it

| Task | Command |
|---|---|
| Update the server | `git pull && docker compose up -d --build` |
| Logs | `docker compose logs -f license` (or `caddy`) |
| Change the token lifetime | edit `TOKEN_TTL_SECS` in `.env`, then `docker compose up -d` |
| Restart | `docker compose restart` |
| Stop | `docker compose down` (keeps the key volume; `down -v` deletes it, so never add `-v`) |

**What's built in:** per-address rate limits (10 license requests and 60 pack downloads per 10 minutes), at most 64 requests in flight, request bodies capped at 8 KB, a read-only container filesystem with no extra Linux capabilities, a non-root user, and a refusal to start without a Gumroad product ID. Stub test keys only work with `--dev`, which this stack never uses.

**What it doesn't do:** it's one machine in one region, so a host outage stops new activations and downloads (installed labs keep working). Revocation takes effect at the next token renewal, up to `TOKEN_TTL_SECS` after a refund. The rate limiter keys on the address Caddy reports, so users behind one shared address (an office, a VPN) share a limit.

Moving to another host later only needs the same compose stack and the key volume restored from your backup. As long as the domain stays the same, no launcher release is needed.
