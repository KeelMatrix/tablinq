# Gateway HTTP Monitoring

Tablinq persists one row per incoming gateway HTTP request in
`gateway_http_requests`. Bodies are capped, sensitive headers are redacted, and
streaming response bodies are intentionally omitted.

Latest gateway requests:

```sql
SELECT id, created_at, request_id, method, path, route, response_status_code, duration_ms
FROM gateway_http_requests
ORDER BY created_at DESC
LIMIT 25;
```

Failed or error HTTP responses:

```sql
SELECT created_at, request_id, method, path, response_status_code, parse_error
FROM gateway_http_requests
WHERE response_status_code >= 400 OR parse_error IS NOT NULL
ORDER BY created_at DESC;
```

Lookup by `X-Request-Id`:

```sql
SELECT *
FROM gateway_http_requests
WHERE request_id = 'req-example'
ORDER BY id DESC;
```

Join gateway HTTP records to durable request history:

```sql
SELECT g.created_at,
       g.method,
       g.path,
       g.response_status_code,
       r.job_id,
       r.status,
       r.execution_kind
FROM gateway_http_requests g
LEFT JOIN request_history r ON r.request_id = g.request_id
WHERE g.request_id = 'req-example';
```

Inspect captured bodies and truncation flags:

```sql
SELECT request_id,
       request_body_truncated,
       request_body_sha256,
       substr(request_body_text, 1, 2000) AS request_excerpt,
       response_body_truncated,
       response_body_sha256,
       response_capture_omitted_reason,
       substr(response_body_text, 1, 2000) AS response_excerpt
FROM gateway_http_requests
ORDER BY created_at DESC
LIMIT 10;
```
