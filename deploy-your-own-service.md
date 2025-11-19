# Deploy your own TokeiSrv with Docker Compose

This guide walks you through deploying your own instance of `tokeisrv` using the included `docker-compose.yml`. It covers domain registration (nic.ua), Cloudflare account and zone setup, DNS delegation, Cloudflare One activation, gathering `cloudflared` credentials, storing them for Docker Compose, configuring a user whitelist, and generating badge URLs / Markdown examples.

> This document is intentionally concise. It assumes you have admin access to your domain registrar and Cloudflare, and have Docker installed.

---

## 1) Get a free `.pp.ua` domain from nic.ua

1. Visit https://nic.ua/ and create an account (sign up with your email).
2. Search for your own domain under `.pp.ua` (these have free registrations) — try your desired name like `my-tokei.pp.ua`.
3. Register it. Follow nic.ua instructions to confirm your contact and domain registration. If required, set an initial nameserver; we'll change that later to Cloudflare.

Notes:
- Keep your nic.ua account credentials secure.
- The exact UI changes, but the process is the same: create account → search domain → register.

---

## 2) Create a free Cloudflare account and add a zone for your domain

1. Go to https://dash.cloudflare.com/signup and sign up.
2. After account is created, click **Add site** and provide your domain (`my-tokei.pp.ua`).
3. Cloudflare will scan for DNS records. You can proceed without them; they can be created later.
4. Continue through the plan selection (use Free if you prefer).

Notes:
- Cloudflare will assign nameservers in the next step; copy them — you'll need them in NIC.UA.

---

## 3) Point DNS at Cloudflare (NIC.UA nameserver update)

1. In the Cloudflare dashboard for your site, note the Cloudflare nameservers given (like `abby.ns.cloudflare.com` and `rory.ns.cloudflare.com`).
2. At NIC.UA, go to domain settings → DNS Servers / Nameservers and replace the current provider nameservers with the Cloudflare nameservers.
3. Save/confirm the change. DNS propagation can take minutes to hours.

Notes:
- After delegation, Cloudflare becomes the authoritative DNS. Add DNS entries in Cloudflare for `A`, `CNAME`, etc. (Not necessary for cloudflared since Cloudflare handles the tunnel ingress).

---

## 4) Activate Cloudflare One

1. Login to Cloudflare dashboard and open the **Zero Trust** / **Cloudflare One** area (this name might vary in the dashboard; previously 'Access' or 'Tunnel').
2. Follow the onboarding for Cloudflare One. Usually it asks you to configure an account, add a site, or confirm identity provider settings.
3. For this tutorial you mainly need Cloudflare Tunnel (Argo Tunnel) which is part of Cloudflare Zero Trust; make sure your account has permission to create tunnels.

Notes:
- Cloudflare One includes multiple features; you only need to enable the tunnel for this service.
- If you have Cloudflare Access, you can set policies to protect the application.

---

## 5) Install `cloudflared`, create a Tunnel, and obtain `credentials.json` and `cert.pem`

These steps create a tunnel and produce the JSON credentials and cert token used by the `cloudflared` container.

1. Install `cloudflared` locally (or run in container):
   - GitHub releases: https://github.com/cloudflare/cloudflared/releases
   - Local (macOS/laptop): `brew install cloudflared`
   - Docker container (example): `docker run -it --rm highcanfly/net-tools:1.3.1 /bin/bash` then install or use included cloudflared.

2. Login with `cloudflared` to generate a `cert.pem`:

```bash
cloudflared tunnel login
# follow the browser link to log in and create access
# this writes ~/.cloudflared/cert.pem on your machine
```

3. Create a new Tunnel and save the generated credentials file:

```bash
cloudflared tunnel create tokeisrv-tunnel
# This prints the tunnel ID and creates a file under ~/.cloudflared/<TUNNEL_UUID>.json
# Copy the file to your project folder as ./cloudflared/credentials.json
```

4. Optionally bind a DNS name to the tunnel using `cloudflared tunnel route dns`:

```bash
cloudflared tunnel route dns <TUNNEL_UUID> tokeisrv.yourdomain.pp.ua
```

5. Confirm you have two files:

- `credentials.json` — the JSON credentials created by `cloudflared tunnel create`.
- `cert.pem` (or token) — created by `cloudflared tunnel login` and used to authorize the tunnel.

Make sure both are present in `./cloudflared/` before running Docker Compose.

Security note:
- Never commit `credentials.json` or `cert.pem` to Git. Use secrets for production.

---

## 6) Configure the whitelist

`TokeiSrv` supports a user whitelist to restrict which repo owners may be processed.

1. In Docker Compose, set the `TOKEI_USER_WHITELIST` environment variable for the `tokeisrv` service. Example:

```yaml
environment:
  - TOKEI_USER_WHITELIST=alice,bob
```

2. CLI usage:
- Start the binary with `--user-whitelist`:

```bash
./tokei_rs --user-whitelist alice,bob
```

3. Helm / Kubernetes:
- In `values.yaml`, set `userWhitelist` to a comma-separated string. Example `values.yaml` syntax:

```yaml
userWhitelist: "alice,bob"
```

4. Behaviour: If a request attempts to analyze a repo owned by a user not in the whitelist, `tokei_rs` returns a red `forbidden` SVG badge instead of cloning that repo.

---

## 7) Start the stack with Docker Compose

1. Create the `cloudflared` folder and put the files there:

```
mkdir -p cloudflared
# copy credentials.json and config.yml and cert.pem there
```

2. Example `cloudflared/config.yml`:

```yaml
# config.yml
# Replace placeholders
tunnel: <TUNNEL_ID>
credentials-file: /etc/cloudflared/creds/credentials.json
ingress:
  - hostname: tokeisrv.example.pp.ua
    service: http://tokeisrv:8000
  - service: http_status:404
```

3. Run Docker Compose to start everything:

```bash
docker compose up -d
```

4. Check logs:

```bash
docker compose logs -f tokeisrv
docker compose logs -f cloudflared
```

If `cloudflared` reports that it started, you should be able to access the tunnel's hostname on the internet and have Cloudflare route requests to your `tokeisrv` service.

---

## 8) Generating badges and Markdown examples

`TokeiSrv` exposes a GET endpoint to generate badges on-the-fly. The base route is:

```
GET /b1/{domain}/{user}/{repo}
```

Examples (replace `tokeisrv.example.pp.ua` with your domain):

- Default lines badge (SVG image):

```bash
curl "http://tokeisrv.example.pp.ua/b1/github.com/sctg-development/tokeisrv" -H "Accept: image/svg+xml"
```

- Code lines as a badge (SVG):

```bash
curl "http://tokeisrv.example.pp.ua/b1/github.com/sctg-development/tokeisrv?category=code" -H "Accept: image/svg+xml"
```

- JSON output (instead of SVG):

```bash
curl -H "Accept: application/json" "http://tokeisrv.example.pp.ua/b1/github.com/sctg-development/tokeisrv"
```

Markdown examples (insert into README or README-based dashboards):

- Inline badge (image link to the SVG):
```md
![Lines](https://tokeisrv.example.pp.ua/b1/github.com/sctg-development/tokeisrv)
```
- Linked badge (clickable link to repo):
```md
[![Lines](https://tokeisrv.example.pp.ua/b1/github.com/sctg-development/tokeisrv)](https://github.com/sctg-development/tokeisrv)
```

---

## Tips & Troubleshooting

- DNS propagation: After you change nameservers at nic.ua, the zone may take time to propagate — use `dig` or `nslookup` to verify.
- Cloudflare One: If you want to set access policies, use Cloudflare Access to define who can access the hostname.
- local testing: you can run `cloudflared` locally with `--config` and `tunnel run` to test before deploying.
- If you experience `cloudflared` errors about the config or missing `credentials.json`, re-run the `cloudflared tunnel create` and log back in.

---

## Wrapping up

You now have a simple local deployment via Docker Compose to test `tokeisrv` and expose it on the internet via Cloudflare Tunnel. For production or robust deployment, consider using the Helm chart to manage Kubernetes deployments and monitor logs/upgrade lifecycle.
