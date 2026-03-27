# Docker Compose Fragment System — Reference Guide

## Overview

Nucleus assembles `docker-compose.yml` by **concatenating individual fragment files** instead of downloading a monolithic template and stripping disabled services. MySQL and RabbitMQ run on the host (not in Docker).

## GCS Layout

```
gs://puru-releases/templates/
  fragments/
    core.yml
    backend.yml
    has.yml
    pacs.yml
    pathology.yml
    comm_server.yml
    realtime.yml
    medical.yml
    bridge.yml
    frontend.yml
  env/
    general.env
    database.env
    rabbitmq.env
    has.env
    mail.env
    pacs.env
    database-neon.env
    rabbitmq-neon.env
```

No `{os}` prefix — shared across all platforms.

Backup path: `gs://puru-automated-backup/{hospital_code}/config/docker-compose.yml`

## Fragment → Module Mapping

| Fragment file      | Module key | Compose service name | Docker image    | Image tag placeholder |
|--------------------|-----------|---------------------|-----------------|----------------------|
| `core.yml`         | *(always)* | —                   | —               | —                    |
| `backend.yml`      | xenon     | backend             | puru-xenon      | `{{XENON_TAG}}`      |
| `has.yml`          | has       | has                 | puru-has         | `{{HAS_TAG}}`        |
| `pacs.yml`         | pacs      | pacs                | puru-pacs        | `{{PACS_TAG}}`       |
| `pathology.yml`    | argon     | pathology           | puru-argon       | `{{ARGON_TAG}}`      |
| `comm_server.yml`  | comm      | comm_server         | puru-comm        | `{{COMM_TAG}}`       |
| `realtime.yml`     | realtime  | realtime            | puru-realtime    | `{{REALTIME_TAG}}`   |
| `medical.yml`      | neon      | medical             | puru-neon        | `{{NEON_TAG}}`       |
| `bridge.yml`       | bridge    | bridge              | puru-bridge      | `{{BRIDGE_TAG}}`     |
| `frontend.yml`     | hydrogen  | frontend            | puru-hydrogen    | `{{HYDROGEN_TAG}}`   |

## Assembly Logic (in Rust)

```
1. Read core.yml            → "version: \"3.8\""
2. Append "\nservices:\n"
3. For each FRAGMENTS entry where module is enabled:
     Read fragment file, append content verbatim
4. Result = complete docker-compose.yml (with {{placeholders}})
5. substitute_variables() replaces placeholders with actual values
```

Pure string concatenation — no YAML parsing.

## Fragment Format Rules

### core.yml — Skeleton only
```yaml
version: "3.8"
```
- No `services:` key — the assembly code appends that.
- No trailing newline required (code handles it).

### Service fragments — Indented service blocks
```yaml
  backend:
    image: gcr.io/puru-255206/puru-xenon:{{XENON_TAG}}
    container_name: backend
    restart: unless-stopped
    network_mode: host
    env_file:
      - ./env/general.env
      - ./env/database.env
      - ./env/rabbitmq.env
```

**Critical formatting rules:**
1. **2-space indent** for the service name (e.g., `  backend:`) — it goes under `services:`.
2. **4-space indent** for service properties (e.g., `    image:`).
3. **6-space indent** for nested lists (e.g., `      - ./env/general.env`).
4. **No `depends_on`** — MySQL/RabbitMQ are on host, not Docker containers.
5. **No `volumes`** — no database containers to persist.
6. **`network_mode: host`** on every service — connects to host MySQL/RabbitMQ via localhost.
7. **Use `{{TAG_PLACEHOLDER}}`** for image tags — see mapping table above.
8. **End with a newline** — ensures clean concatenation between fragments.
9. **No `services:` key** in the fragment — only the indented service block.

### Available placeholders for substitute_variables()

| Placeholder             | Source                          |
|------------------------|---------------------------------|
| `{{HOSPITAL_CODE}}`    | NucleusConfig.hospital_code     |
| `{{SERVER_IP}}`        | NucleusConfig.server_ip         |
| `{{MYSQL_PASSWORD}}`   | NucleusConfig.mysql_password    |
| `{{RABBITMQ_PASSWORD}}`| Hardcoded "puru123" default     |
| `{{XENON_TAG}}`        | Default "latest"                |
| `{{HAS_TAG}}`          | Default "latest"                |
| `{{PACS_TAG}}`         | Default "latest"                |
| `{{ARGON_TAG}}`        | Default "latest"                |
| `{{COMM_TAG}}`         | Default "latest"                |
| `{{REALTIME_TAG}}`     | Default "latest"                |
| `{{NEON_TAG}}`         | Default "latest"                |
| `{{BRIDGE_TAG}}`       | Default "latest"                |
| `{{HYDROGEN_TAG}}`     | Default "latest"                |

Placeholders can appear in any fragment or env file — `substitute_variables()` does a global find-replace on the assembled content.

## How to Upload Fragments to GCS

### Prerequisites
- `gsutil` CLI authenticated with access to `puru-releases` bucket
- Fragment files created locally following the format rules above

### Upload all fragments
```bash
# From directory containing fragment files
gsutil -m cp core.yml backend.yml has.yml pacs.yml pathology.yml \
  comm_server.yml realtime.yml medical.yml bridge.yml frontend.yml \
  gs://puru-releases/templates/fragments/

# Upload env templates
gsutil -m cp general.env database.env rabbitmq.env has.env mail.env \
  pacs.env database-neon.env rabbitmq-neon.env \
  gs://puru-releases/templates/env/
```

### Upload a single updated fragment
```bash
gsutil cp backend.yml gs://puru-releases/templates/fragments/backend.yml
```

### Verify uploads
```bash
gsutil ls gs://puru-releases/templates/fragments/
gsutil ls gs://puru-releases/templates/env/
```

### Force re-download on hospital machines
Nucleus skips fragments that already exist locally. To force re-download after updating a fragment on GCS:
```bash
# On the hospital machine, delete local fragments
rm -rf ~/puru/docker/fragments/
# Then re-run download from Nucleus UI or restart
```

Or delete a specific fragment:
```bash
rm ~/puru/docker/fragments/backend.yml
```

## How to Generate Fragments from a Full docker-compose.yml

### Instructions for Claude

Given a full `docker-compose.yml`, split it into fragments as follows:

1. **Identify the version line** → save as `core.yml`
   ```yaml
   version: "3.8"
   ```

2. **For each service block under `services:`**, check the mapping table:
   - If the service matches a known fragment name → save as that fragment file
   - Each fragment contains ONLY the indented service block (2-space indent for service name)
   - Do NOT include `services:` key in fragments

3. **Replace hardcoded image tags** with `{{PLACEHOLDER}}` variables:
   - `puru-xenon:2.3.5` → `puru-xenon:{{XENON_TAG}}`
   - `puru-has:1.0.0` → `puru-has:{{HAS_TAG}}`
   - etc. (see mapping table)

4. **Replace hardcoded config values** with placeholders:
   - Hospital codes → `{{HOSPITAL_CODE}}`
   - Server IPs → `{{SERVER_IP}}`
   - MySQL passwords → `{{MYSQL_PASSWORD}}`
   - RabbitMQ passwords → `{{RABBITMQ_PASSWORD}}`

5. **Remove these if present** (not needed in fragment system):
   - `depends_on` blocks (MySQL/RabbitMQ on host)
   - `volumes` top-level key and volume definitions (no DB containers)
   - `networks` top-level key (using `network_mode: host`)
   - Any `database`, `rabbitmq`, `auth`, `fileserver` service blocks (infra is on host)

6. **Ensure `network_mode: host`** is present on every service fragment.

7. **Ensure `env_file`** references use relative `./env/` paths.

8. **Validate**: Concatenating `core.yml` + `\nservices:\n` + all fragment files should produce valid docker-compose YAML.

### Example: Splitting a full compose file

**Input** (`docker-compose.yml`):
```yaml
version: "3.8"

services:
  database:
    image: mysql:8.0
    container_name: purusql
    volumes:
      - mysql_data:/var/lib/mysql

  rabbitmq:
    image: rabbitmq:3.12-management
    container_name: rabbitmq

  backend:
    image: gcr.io/puru-255206/puru-xenon:2.3.5
    container_name: backend
    restart: unless-stopped
    network_mode: host
    depends_on:
      - database
      - rabbitmq
    env_file:
      - ./env/general.env
      - ./env/database.env
      - ./env/rabbitmq.env

  pacs:
    image: gcr.io/puru-255206/puru-pacs:1.2.0
    container_name: pacs
    restart: unless-stopped
    network_mode: host
    depends_on:
      - database
    env_file:
      - ./env/general.env
      - ./env/database.env
      - ./env/pacs.env

volumes:
  mysql_data:
```

**Output fragments:**

`core.yml`:
```yaml
version: "3.8"
```

`backend.yml`:
```yaml
  backend:
    image: gcr.io/puru-255206/puru-xenon:{{XENON_TAG}}
    container_name: backend
    restart: unless-stopped
    network_mode: host
    env_file:
      - ./env/general.env
      - ./env/database.env
      - ./env/rabbitmq.env
```

`pacs.yml`:
```yaml
  pacs:
    image: gcr.io/puru-255206/puru-pacs:{{PACS_TAG}}
    container_name: pacs
    restart: unless-stopped
    network_mode: host
    env_file:
      - ./env/general.env
      - ./env/database.env
      - ./env/pacs.env
```

**Dropped**: `database` service, `rabbitmq` service, `volumes` section, all `depends_on` blocks.

## Adding a New Service

If a new service is added to the Puru ecosystem:

1. **Create the fragment file** following the format rules.
2. **Add entry to `FRAGMENTS` constant** in `src-tauri/src/compose_template/mod.rs`:
   ```rust
   ("new_service.yml", Some("new_module_key")),
   ```
3. **Add field to `ServiceModules` struct** and its `Default` impl.
4. **Add tag placeholder** to `TemplateVariables` struct and `substitute_variables()`.
5. **Add tag field** to `build_variables_from_config()`.
6. **Update TypeScript** `ServiceModules` and `TemplateVariables` interfaces in `tauri.service.ts`.
7. **Update Angular** compose component module toggles.
8. **Upload fragment** to `gs://puru-releases/templates/fragments/`.
9. **Update Firestore** hospital modules config to include the new key.

## Code Locations

| What | File |
|------|------|
| FRAGMENTS constant, assembly logic | `src-tauri/src/compose_template/mod.rs` |
| Tauri commands | `src-tauri/src/commands/mod.rs` |
| Invoke handler registration | `src-tauri/src/main.rs` |
| TypeScript interfaces | `src/app/core/services/tauri.service.ts` |
| Compose UI component | `src/app/features/compose/compose.component.ts` |
| Tests | Bottom of `compose_template/mod.rs` |
