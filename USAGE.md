# ProxyNet - Usage Guide

ProxyNet is a Cloudflare alternative reverse proxy built in Rust with in-memory caching, database logging, and dynamic route management.

## Quick Start
Assuming you have Rust installed and have cloned the repository, follow these steps to get started:
(As of now only tested in Windows, probabyl won't work on Linux/Mac without some tweaks to file paths and commands)
### 1. Start the Servers

```bash
cargo run
```

This starts two servers:
- **Proxy Server**: `http://localhost:8080` - Handles proxied requests
- **API Server**: `http://localhost:8081` - Manages domains and configuration

### 2. Create a Domain

Add a new domain to proxy:

```powershell
# PowerShell
Invoke-RestMethod -Uri http://localhost:8081/domains -Method POST `
  -ContentType "application/json" `
  -Body '{"domain":"example.local","origin":"http://localhost:3000","enabled":true}'
```

```bash
# Bash/CMD
curl -X POST http://localhost:8081/domains \
  -H "Content-Type: application/json" \
  -d "{\"domain\":\"example.local\",\"origin\":\"http://localhost:3000\",\"enabled\":true}"
```

**Response:**
```json
{
  "success": true,
  "data": {
    "id": 1,
    "domain": "example.local",
    "origin": "http://localhost:3000",
    "enabled": true
  },
  "error": null
}
```

### 3. Route Through the Proxy

Make requests to the proxy with the Host header:

```powershell
curl.exe http://localhost:8080/ -H "Host: example.local"
```

The proxy will:
1. Check in-memory cache for the response
2. If cached, return cached response
3. If not cached, forward to origin (`http://localhost:3000`)
4. Cache the response in memory
5. Log request details to SQLite database
6. Return response to client

---

## API Endpoints

### Domains Management

#### List All Domains
```powershell
curl.exe http://localhost:8081/domains
```

**Response:**
```json
{
  "success": true,
  "data": [
    {
      "id": 1,
      "domain": "example.local",
      "origin": "http://localhost:3000",
      "enabled": true
    },
    {
      "id": 2,
      "domain": "api.local",
      "origin": "http://localhost:4000",
      "enabled": true
    }
  ],
  "error": null
}
```

#### Get Single Domain
```powershell
curl.exe http://localhost:8081/domains/1
```

#### Create Domain
```powershell
Invoke-RestMethod -Uri http://localhost:8081/domains -Method POST `
  -ContentType "application/json" `
  -Body '{"domain":"myapp.local","origin":"http://localhost:3000","enabled":true}'
```

#### Update Domain (Change Origin or Domain Name)
```powershell
Invoke-RestMethod -Uri http://localhost:8081/domains/1 -Method PATCH `
  -ContentType "application/json" `
  -Body '{"domain":"myapp.local","origin":"http://localhost:5000","enabled":true}'
```

**Changes take effect immediately** - no restart required!

#### Delete Domain
```powershell
Invoke-RestMethod -Uri http://localhost:8081/domains/1 -Method DELETE
```

After deletion, the proxy will return `404` for requests to that domain.

---

### Statistics & Monitoring

#### Get Cache & Request Stats
```powershell
curl.exe http://localhost:8081/stats
```

**Response:**
```json
{
  "success": true,
  "data": {
    "total_requests": 42,
    "cache_hits": 35,
    "cache_hit_rate": 83.33,
    "avg_response_time_ms": 145.2,
    "total_bytes_sent": 524288
  },
  "error": null
}
```

---

### Configuration

#### Get All Configuration
```powershell
curl.exe http://localhost:8081/config
```

#### Get Specific Config Key
```powershell
curl.exe http://localhost:8081/config/host
```

#### Set Configuration
```powershell
Invoke-RestMethod -Uri http://localhost:8081/config/rate_limit_per_minute -Method PATCH `
  -ContentType "application/json" `
  -Body '{"value":"1000"}'
```

#### Set Multiple Configs
```powershell
Invoke-RestMethod -Uri http://localhost:8081/config -Method POST `
  -ContentType "application/json" `
  -Body '{"host":"0.0.0.0","port":"8080","api_port":"8081"}'
```

---

## Configuration Options

These can be set via the config API:

| Key | Default | Description |
|-----|---------|-------------|
| `host` | `0.0.0.0` | Proxy server bind address |
| `port` | `8080` | Proxy server port |
| `api_port` | `8081` | API server port |
| `rate_limit_per_minute` | `null` | Requests per minute (disabled if not set) |

---

## Examples

### Example 1: Create Multiple Domains

```powershell
# Domain 1: Blog
Invoke-RestMethod -Uri http://localhost:8081/domains -Method POST `
  -ContentType "application/json" `
  -Body '{"domain":"blog.local","origin":"http://localhost:3000","enabled":true}'

# Domain 2: API
Invoke-RestMethod -Uri http://localhost:8081/domains -Method POST `
  -ContentType "application/json" `
  -Body '{"domain":"api.local","origin":"http://localhost:4000","enabled":true}'

# Domain 3: Admin
Invoke-RestMethod -Uri http://localhost:8081/domains -Method POST `
  -ContentType "application/json" `
  -Body '{"domain":"admin.local","origin":"http://localhost:5000","enabled":true}'
```

### Example 2: Route Requests

```powershell
# Route to blog
curl.exe http://localhost:8080/posts -H "Host: blog.local"

# Route to API
curl.exe http://localhost:8080/users -H "Host: api.local"

# Route to admin
curl.exe http://localhost:8080/dashboard -H "Host: admin.local"
```

### Example 3: Update Origin Server

```powershell
# Get current domains
curl.exe http://localhost:8081/domains

# Update domain 1 to new origin
Invoke-RestMethod -Uri http://localhost:8081/domains/1 -Method PATCH `
  -ContentType "application/json" `
  -Body '{"domain":"blog.local","origin":"http://localhost:9000","enabled":true}'

# Changes apply immediately - no restart needed!
curl.exe http://localhost:8080/posts -H "Host: blog.local"
```

### Example 4: Monitor Performance

```powershell
# Check stats
curl.exe http://localhost:8081/stats

# Expected high cache hit rate after repeated requests
for ($i=0; $i -lt 10; $i++) {
  curl.exe http://localhost:8080/ -H "Host: blog.local"
}

# Check stats again - should show cache hits
curl.exe http://localhost:8081/stats
```

---

## Database

ProxyNet uses SQLite for persistent storage. The database file is located at:
```
c:\Users\Prath\Programming\proxynet\proxynet.db
```

### Database Schema

**Domains Table:**
```sql
CREATE TABLE domains (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  domain TEXT UNIQUE NOT NULL,
  origin TEXT NOT NULL,
  enabled BOOLEAN DEFAULT 1,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
)
```

**Request Logs Table:**
```sql
CREATE TABLE request_logs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  domain TEXT NOT NULL,
  path TEXT NOT NULL,
  method TEXT NOT NULL,
  status_code INTEGER,
  response_time_ms REAL,
  cache_hit BOOLEAN,
  bytes_sent INTEGER,
  created_at INTEGER NOT NULL
)
```

**Config Table:**
```sql
CREATE TABLE config (
  key TEXT PRIMARY KEY,
  value TEXT,
  updated_at INTEGER NOT NULL
)
```

---

## Troubleshooting

### Domain "not configured" error
**Error:** `Domain 'example.local' not configured`
**Cause:** The domain hasn't been added yet.
**Fix:** Create the domain via the API first:
```powershell
Invoke-RestMethod -Uri http://localhost:8081/domains -Method POST `
  -ContentType "application/json" `
  -Body '{"domain":"example.local","origin":"http://localhost:3000","enabled":true}'
```

### UNIQUE constraint failed on domain name
**Error:** `UNIQUE constraint failed: domains.domain`
**Cause:** A domain with that name already exists.
**Fix:** Either use a different domain name or delete the existing one first.

### Port already in use
**Error:** `Address already in use`
**Cause:** Port 8080 or 8081 is being used by another process.
**Fix:** Change the port via config:
```powershell
Invoke-RestMethod -Uri http://localhost:8081/config -Method POST `
  -ContentType "application/json" `
  -Body '{"port":"9090","api_port":"9091"}'
```
Then restart the server.

### Changes not taking effect
**Note:** API changes (domain create/update/delete) take effect **immediately** without restart.
If changes don't apply, check:
1. The API returned `"success": true`
2. The proxy server is still running
3. You're using the correct Host header in requests

---

## Performance Tips

1. **Cache Hit Rate**: ProxyNet caches responses in memory. Make repeated requests to the same path to maximize cache hits.
2. **Rate Limiting**: Enable rate limiting to prevent abuse:
   ```powershell
   Invoke-RestMethod -Uri http://localhost:8081/config/rate_limit_per_minute -Method PATCH `
     -ContentType "application/json" `
     -Body '{"value":"10000"}'
   ```
3. **Monitor Stats**: Regularly check `/stats` to track performance and cache effectiveness.

---

## Future Improvements

- Add TLS/HTTPS support
- Implement cache TTL and eviction policies
- Add request/response transformation
- Build a web UI for management
- Add Prometheus metrics export