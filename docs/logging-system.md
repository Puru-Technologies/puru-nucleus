# Structured Logging & Cloud Log Transport

Status: **design + partial rollout.** The service-side format change is live on **`puru-auth` only**; the
nucleus transport (`fetch_logs`) and the other services are **not built yet**. Until they are, **only auth
produces the structured log that nucleus can ship** — everything below the "Rollout status" section is the
plan to finish it.

---

## 1. Goals

1. **Ingestible** — logs are structured (one JSON object per line), so any log system reads them with no custom parser.
2. **Queryable** — filter by **time** and by **labels** (level, service, logger, trace id) instead of grepping text.
3. **Viewable** — oxygen renders a real table (time · level · logger · message) with filters + search + expandable stack traces.
4. **Cost-effective transport** — pulled on demand, gzipped, **inline in Firestore for small logs (no GCS object)** and **GCS only for large ones**, auto-expired after 7 days.

## 2. Log format — ECS JSON Lines (dual output)

Each service emits **two** streams:

| Stream | File | Purpose |
| --- | --- | --- |
| **Pretty** | `${LOGS}/<basename>.log` (rolling) + plain console | Human tailing on the box / RDP |
| **Structured** | `${LOGS}/<basename>.json` (rolling, gzipped archives) | **What nucleus ships to the cloud** |

The structured stream is **ECS** (Elastic Common Schema) — the de-facto ingestable JSON shape, so we're never
locked to one backend. One line looks like:

```json
{"@timestamp":"2026-08-15T16:20:01.123Z","log.level":"INFO","process.thread.name":"http-8081-3",
 "service.name":"puru-auth","log.logger":"com.puru.auth.TokenSvc","message":"JWT verify failed",
 "error.type":"...","error.stack_trace":"...","ecs.version":"8.11"}
```

- `service.name` ← `spring.application.name`
- exceptions add `error.type` / `error.message` / `error.stack_trace`
- MDC entries (e.g. `trace.id`) appear as top-level fields — these are the "labels" the viewer filters on.

## 3. Enabling it in a service (the per-repo change)

Uses **Spring Boot's built-in structured logging**, which requires **Boot ≥ 3.4** (the
`org.springframework.boot.logging.logback.StructuredLogEncoder` class does not exist before 3.4). No extra
dependency needed.

**a. `src/main/resources/logback-spring.xml`** — add the ECS JSON appender and reference it from `<root>` and
the app logger. Reference template (from `puru-auth`):

```xml
<property name="LOGS" value="./logs" />
<springProperty scope="context" name="LOG_BASENAME" source="log.basename" defaultValue="auth" />

<!-- Plain console (no ANSI colour — nucleus captures stdout verbatim to a file) -->
<appender name="Console" class="ch.qos.logback.core.ConsoleAppender">
    <encoder class="ch.qos.logback.classic.encoder.PatternLayoutEncoder">
        <pattern>%d{ISO8601} %-5level [%t] %C: %msg%n%throwable{7}</pattern>
    </encoder>
</appender>

<!-- Human-readable rolling file -->
<appender name="RollingFile" class="ch.qos.logback.core.rolling.RollingFileAppender">
    <file>${LOGS}/${LOG_BASENAME}.log</file>
    <encoder class="ch.qos.logback.classic.encoder.PatternLayoutEncoder">
        <pattern>app=AUTH [%d] level=%p class=%C thread=[%t] %m%n%throwable{5}</pattern>
    </encoder>
    <rollingPolicy class="ch.qos.logback.core.rolling.SizeAndTimeBasedRollingPolicy">
        <fileNamePattern>${LOGS}/archived/${LOG_BASENAME}-%d{yyyy-MM-dd}.%i.log</fileNamePattern>
        <maxFileSize>10MB</maxFileSize><maxHistory>7</maxHistory><totalSizeCap>500MB</totalSizeCap>
    </rollingPolicy>
</appender>

<!-- ECS JSON — the structured stream nucleus ships -->
<appender name="JsonFile" class="ch.qos.logback.core.rolling.RollingFileAppender">
    <file>${LOGS}/${LOG_BASENAME}.json</file>
    <encoder class="org.springframework.boot.logging.logback.StructuredLogEncoder">
        <format>ecs</format>
    </encoder>
    <rollingPolicy class="ch.qos.logback.core.rolling.SizeAndTimeBasedRollingPolicy">
        <fileNamePattern>${LOGS}/archived/${LOG_BASENAME}-%d{yyyy-MM-dd}.%i.json.gz</fileNamePattern>
        <maxFileSize>20MB</maxFileSize><maxHistory>7</maxHistory><totalSizeCap>1GB</totalSizeCap>
    </rollingPolicy>
</appender>

<root level="info">
    <appender-ref ref="Console" /><appender-ref ref="RollingFile" /><appender-ref ref="JsonFile" />
</root>
```

Notes:
- The file **must** be `logback-spring.xml` (not `logback.xml`) — the encoder needs Spring's environment for `service.name`.
- Console is deliberately **plain** — the old config used ANSI colours (`%highlight`/`%blue`), and nucleus's stdout capture stored the escape codes, bloating and corrupting the captured file.
- Rolling policies bound disk (the old console capture grew unbounded — one file hit ~10 MB).

**b. `src/main/resources/application.properties`** — set the service identity:

```properties
spring.application.name=puru-auth
```

## 4. Nucleus launch wiring (to build)

Nucleus starts each service as `java -jar` with `current_dir = C:\PuruNucleus` (`services/native.rs`), so
`LOGS=./logs` resolves to `C:\PuruNucleus\logs`. When it launches a service it should inject:

| Env var | Value | Effect |
| --- | --- | --- |
| `SPRING_APPLICATION_NAME` | the service name (e.g. `puru-auth`) | ECS `service.name` |
| `LOG_BASENAME` | the service name | file becomes `<service>.json` so nucleus knows the path |
| `LOGS` (or `-DLOGS`) | nucleus logs dir | where the `.json` lands (defaults correctly via cwd) |

Nucleus then reads/ships `${LOGS}/<service>.json`.

> Because nucleus supplies these at launch, adding a **new service needs no per-service name edit** beyond the
> logback template + Boot ≥ 3.4.

## 5. Cloud transport — the `fetch_logs` command (to build)

Reuses the existing Firestore command pipeline (`daemon/commands.rs :: execute`). Oxygen enqueues a command;
the daemon's command listener runs it and writes the result back into the command doc.

**Request** — `hospital/{code}/commands/{id}`:
```
command_type: "fetch_logs"
params: { service_name: "puru-auth", lines?: 500, level?: "WARN", since_minutes?: 60 }
        // lines default 500, hard cap 5000
```

**Handler steps:**
1. Read `${LOGS}/<service>.json`, tail last N lines (never the whole file).
2. **Pre-filter** by `level` (`log.level >=`) and `since_minutes` (`@timestamp >=`) — structured fields make
   this cheap and cut payload size.
3. gzip the result.
4. **Deliver by size** (~256 KB gzip threshold):

```jsonc
// small → inline in Firestore (no GCS object, free)
{ "kind":"inline", "service":"puru-auth", "lines":500,
  "encoding":"gzip+base64", "raw_bytes":128000, "data":"H4sI…" }

// large → gzip uploaded to GCS
{ "kind":"gcs", "service":"puru-auth", "lines":5000, "gzip_bytes":410000,
  "url":"gs://<bucket>/hospital/PURUT2/logs/puru-auth-20260815T1620Z.log.gz" }
```

The result JSON goes in the command doc's result/`message` field.

## 6. Auto-attach on alerts (to build)

When the watchdog raises a **service-down / crash** alert, auto-pull the **last ~200 lines inline only**
(never GCS on auto) and attach to the alert doc, with a **per-service cooldown (~30 min)** so a crash-looping
service can't spam uploads.

## 7. Retention — GCS 7-day expiry (one-time ops)

Only the overflow blobs live in GCS; expire them with a **bucket lifecycle rule** (applied once, not per object):

```bash
gcloud storage buckets update gs://<bucket> \
  --lifecycle-file=logs-lifecycle.json
# logs-lifecycle.json:
# { "rule": [ { "action": {"type":"Delete"},
#              "condition": {"age":7, "matchesPrefix":["hospital/","logs/"]} } ] }
```
(Scope the prefix to wherever the log blobs are written.)

## 8. Query & view (to build, oxygen repo)

Kept **simple: time-range + label filters** (level, service, logger, trace id) + full-text on `message`. Viewer:
- ungzips inline `data` (or downloads the GCS `url`),
- renders a table (time · level · logger · message) with an expandable stack trace,
- filters client-side on the parsed ECS fields.

No BigQuery/SQL for now (can be added later by pointing a BigQuery external table at the GCS JSONL).

## 9. Data-sensitivity note

Hospital logs can contain **PHI/PII**. Inline puts it in the Firestore command doc; GCS puts it in the bucket
— both must stay restricted to hospital admins. Consider an optional regex redaction pass (emails / MRNs /
phone numbers) before transport if logs are shown to broader roles.

---

## Rollout status

| Service | Spring Boot | Structured logging | State |
| --- | --- | --- | --- |
| **puru-auth** | 4.0.0 | ✅ native | **Done** — logback + `spring.application.name` changed |
| puru-has | 3.3.0 | needs Boot ≥ 3.4 | Pending — bump to ≥ 3.4, then apply template |
| puru-pacs | 3.2.0 | needs Boot ≥ 3.4 | Pending — bump to ≥ 3.4, then apply template |
| puru-realtime | 3.2.0 | needs Boot ≥ 3.4 | Pending — bump to ≥ 3.4, then apply template |

**Until has/pacs/realtime are upgraded to Boot ≥ 3.4 and rebuilt, only `puru-auth` emits the `.json` stream,
so nucleus log transport works for auth only.**

## TODO checklist

- [x] `puru-auth`: `logback-spring.xml` ECS JSON appender + `spring.application.name`
- [ ] Upgrade `puru-has`, `puru-pacs`, `puru-realtime` to Boot ≥ 3.4; apply the same template; rebuild via CI
- [ ] Nucleus: inject `SPRING_APPLICATION_NAME` / `LOG_BASENAME` per service at launch (`services/native.rs`)
- [ ] Nucleus: `fetch_logs` command handler (tail + level/time filter + gzip + inline/GCS deliver)
- [ ] Nucleus: auto-attach recent logs on service-down alerts (with cooldown)
- [ ] Ops: create the GCS 7-day lifecycle rule for the log-blob prefix
- [ ] Oxygen: "fetch logs" request UI + structured viewer (time/label filters, ungzip inline / download GCS)
