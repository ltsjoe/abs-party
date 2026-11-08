# abs-party

Synchronized audiobook listening for Audiobookshelf. Multiple people listen to the same book in real time: one person controls play, pause, seek, and speed; everyone else follows automatically.

Each listener streams audio directly from their own Audiobookshelf account and their progress is saved there independently. The party server is a lightweight sync bus only but proxies requests to avoid CORS issues.


## How it works

Users authenticate with their Audiobookshelf credentials (username/password or API key). The host creates a room and picks a book from a searchable library browser. Guests join by room code. The host's playback controls are broadcast over WebSocket to all participants, who each have their own ABS playback session open.

Before joining, each participant's current position is saved as an ABS bookmark (`pre_party_<datetime>`), so they can return to exactly where they were if the session moves their progress.

Rooms are in-memory and ephemeral -- they disappear when the last participant leaves.


## Stack

- Backend: Rust (axum 0.7, tokio), single binary
- Frontend: Vue 3 + Vite SPA, served as static files from the same binary
- No database


## Book picker

When creating a room the host browses their ABS library. The picker supports:

- **Sort** by last listened, title (A–Z), or date added
- **Search** by title or author — uses the ABS server-side search API so results are not limited to the first page of items
- **Pagination** for browsed (non-search) results

Switching sort mode and typing in the search box are coordinated: the active sort applies to search results, and changing the sort re-runs the current search query.


## Security notes

All ABS REST calls from the browser go through the `/api/abs-proxy/*` route on the party server rather than directly to ABS. This avoids CORS issues and lets the server enforce an allowlist of permitted endpoints. Only read and session-management calls are forwarded; write/delete/admin paths are blocked.

Display names are taken from the ABS username and cannot be overridden. Two participants with the same ABS username cannot be in the same room simultaneously.


## Environment variables

| Variable | Required | Default | Description |
|---|---|---|---|
| `ABS_BASE_URL` | yes | `http://localhost:13378` | Base URL of your ABS instance, no trailing slash |
| `BIND` | no | `0.0.0.0:3456` | TCP address to listen on |
| `PORT` | no | -- | Alternative to BIND; sets port on `0.0.0.0` |
| `STATIC_DIR` | no | `./static` | Path to compiled Vue SPA files |
| `CORS_ORIGIN` | no | -- | Allowed cross-origin (e.g. `http://localhost:5173` for Vite dev). Omit in production. |
| `RUST_LOG` | no | -- | Log filter (e.g. `abs_party=info`) |
| `BYPASS_AUTH` | no | `0` | Set to `1` to skip ABS token validation. Never use in production. |
| `LOCAL_LOGIN` | no | `1` | Set to `0` to turn off the ABS username/password and API key sign-in. Needs SAML configured. |
| `SAML_IDP_METADATA_URL` / `SAML_IDP_METADATA_FILE` | no | -- | IdP metadata. Setting either one turns on SAML sign-in (see below). |
| `PUBLIC_URL` | with SAML | -- | Public `https://` URL of this server, no trailing slash. |
| `ABS_ADMIN_API_KEY` / `ABS_ADMIN_API_KEY_FILE` | with SAML | -- | API key of an ABS admin user, used to mint per-user keys. |
| `SAML_USERNAME_ATTRIBUTE` | no | Authentik's username attribute | SAML attribute holding the ABS username, or `NameID`. |
| `SAML_SP_ENTITY_ID` | no | `$PUBLIC_URL/auth/saml/metadata` | SP entity ID; the IdP's audience must match it. |
| `SSO_KEY_TTL_HOURS` | no | `12` | Lifetime of the ABS key minted for each SAML sign-in. |
| `SAML_LOGIN_LABEL` | no | `Sign in with single sign-on` | Text of the sign-in button. |

Copy `abs-party.env.example` and edit it before deploying.


## SAML sign-in

Optional single sign-on through a SAML identity provider, written for
[Authentik](https://goauthentik.io/); any SAML 2.0 IdP that signs its assertions
should work. The test suite runs the full flow against a signing test IdP and a mock
ABS; it has not yet been tried against a live Authentik.

### How it works

Listeners still need their own ABS account, and each one still streams from and
saves progress to that account. SAML only replaces typing the ABS password:

1. **Sign in** sends the browser to the IdP with a SAML AuthnRequest (HTTP-Redirect).
2. The IdP posts a signed response back to `/auth/saml/acs`. The server checks the
   signature against the IdP metadata's certificate, plus the audience, recipient,
   time window and `InResponseTo`. A short-lived HttpOnly cookie ties the response to
   the browser that started the sign-in, and each request ID is accepted only once.
3. The server finds the ABS user with the same username (case-insensitive, like ABS)
   and, using the admin API key, mints an ABS API key for that user
   (`POST /api/api-keys`, named `abs-party-sso:<user>:<time>`, expires after
   `SSO_KEY_TTL_HOURS`).
4. The browser gets that key through a single-use code (`/?sso=<code>`, valid 60 s),
   so the key itself never shows up in a URL. From there on it works exactly like a
   token from the password sign-in.
5. **Logout** deletes the key from ABS. Keys left behind expire on their own, and
   expired `abs-party-sso:` keys are deleted on later sign-ins.

Refused: users with no matching ABS account (the server never creates ABS users),
disabled ABS users, and the ABS **root** user (an SSO path to root would turn the IdP
account into the root key).

### Security trade-off

Without SAML this server holds no credentials. **With SAML it holds an ABS admin API
key**, so a compromise of this server is a compromise of ABS. Use a dedicated ABS
user of type *admin* (not root), keep the key in a file (`ABS_ADMIN_API_KEY_FILE`),
and keep this server patched.

The server refuses to start if the IdP metadata has no signing certificate: the SAML
library skips signature checks entirely without one. It accepts RSA or ECDSA
signatures with SHA-256/384/512 only, so SHA-1 is refused.

### Setup with Authentik

1. **ABS:** create a user of type *admin* for the party server (e.g. `abs-party`) and
   an API key for it (*Settings → API Keys*, ABS 2.26 or later). Every listener needs
   an ABS user whose username equals their Authentik username.
2. **Authentik:** *Applications → Providers → Create → SAML Provider*:
   - **ACS URL:** `https://party.example.com/auth/saml/acs`
   - **Service Provider Binding:** Post
   - **Audience:** `https://party.example.com/auth/saml/metadata` (must equal `SAML_SP_ENTITY_ID`)
   - **Signing Certificate:** any certificate, with **Sign assertions** on
   - **Digest / Signature algorithm:** SHA256 / RSA-SHA256 (the defaults)
   - Property mappings: keep the default *authentik default SAML Mapping: Username*,
     which sends `http://schemas.goauthentik.io/2021/02/saml/username`.

   Then create an application for the provider and bind the users or group allowed in.
   Or import this server's metadata from `https://party.example.com/auth/saml/metadata`.
3. **Party server:** set `SAML_IDP_METADATA_URL` to the provider's metadata URL
   (*Download metadata* on the provider page, or
   `https://auth.example.com/api/v3/providers/saml/<id>/metadata/?download`),
   plus `PUBLIC_URL` and `ABS_ADMIN_API_KEY_FILE`. The metadata is read once at
   startup, so restart the server after rotating the Authentik certificate.
4. Once SSO works, `LOCAL_LOGIN=0` hides the password form.

Building from source needs libxml2, xmlsec1 (OpenSSL backend), libltdl and libclang;
on Debian: `apt install pkg-config libclang-dev libxml2-dev libxmlsec1-dev libssl-dev libltdl-dev`.
The Dockerfile does this.


## Docker with lima/nerdctl

Build and start:

```sh
lima nerdctl compose down
lima nerdctl compose build --no-cache
lima nerdctl compose up -d
lima nerdctl compose logs -f
```

The `down` step is required. Running `up` without it reuses the old container and silently ignores the new image.

`--no-cache` is needed when source files have changed between builds.

To set the ABS URL, either edit `docker-compose.yml` directly or create a `.env` file next to it:

```
ABS_BASE_URL=https://your-abs-instance.example.com
```


## Systemd (binary deployment)

Build the binary and Vue SPA manually, then install:

```sh
sudo useradd -r -s /sbin/nologin abs-party
sudo mkdir -p /opt/abs-party/static /etc/abs-party
sudo cp abs-party abs-party.env.example /opt/abs-party/
sudo cp abs-party.service /etc/systemd/system/
sudo cp abs-party.env.example /etc/abs-party/env
# edit /etc/abs-party/env
sudo systemctl daemon-reload
sudo systemctl enable --now abs-party
```

Logs: `journalctl -u abs-party -f`


## Development

Backend (requires Rust 1.85+):

```sh
cd server
cargo run
# or
cargo test
```

Frontend (runs on port 5173, proxies API calls to the backend on 3456):

```sh
cd client
npm install
npm run dev
```

Set `CORS_ORIGIN=http://localhost:5173` in the backend environment when running both separately.


## WebSocket protocol

The browser connects to `GET /ws`. All messages are JSON with a `type` field.

Client to server:

```
{ "type": "create_room", "abs_token": "...", "item_id": "...", "item_title": "...", "item_author": "...", "library_id": "..." }
{ "type": "join",        "abs_token": "...", "room_id": "abc12345" }
{ "type": "play",        "position": 1234.5 }
{ "type": "pause",       "position": 1234.5 }
{ "type": "seek",        "position": 1234.5 }
{ "type": "speed",       "rate": 1.25 }
{ "type": "ping",        "sent_at": 1719000000000 }
```

Server to client:

```
{ "type": "room_state",        "room_id": "...", "item": {...}, "position": 0.0, "playing": false, "speed": 1.0, "participants": [...] }
{ "type": "play",              "position": 1234.5, "host_time": 1719000000000 }
{ "type": "pause",             "position": 1234.5 }
{ "type": "seek",              "position": 1234.5 }
{ "type": "speed",             "rate": 1.25 }
{ "type": "participant_joined","name": "alice" }
{ "type": "participant_left",  "name": "alice" }
{ "type": "host_changed",      "name": "alice" }
{ "type": "pong",              "sent_at": 1719000000000 }
{ "type": "error",             "message": "..." }
```

Only the host (room creator, or whoever took over after the host left) may send play/pause/seek/speed. Guest attempts are silently dropped by the server.
