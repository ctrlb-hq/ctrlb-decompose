//! Deterministic synthetic log generator for benchmarking ctrlb-decompose.
//!
//! Usage:
//!   cargo run --release --example generate_logs -- --lines 100000 --seed 42

use std::io::{BufWriter, Write};

// ---------------------------------------------------------------------------
// Xorshift64 PRNG — deterministic, portable, no deps
// ---------------------------------------------------------------------------

struct Rng {
    state: u64,
}

impl Rng {
    fn new(seed: u64) -> Self {
        // Avoid zero state which is a fixed point for xorshift.
        Self {
            state: if seed == 0 { 1 } else { seed },
        }
    }

    #[inline]
    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    /// Returns a value in [0, upper).
    #[inline]
    fn next_bound(&mut self, upper: u64) -> u64 {
        self.next_u64() % upper
    }

    #[inline]
    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / ((1u64 << 53) as f64)
    }

    /// Pick a random element from a slice.
    #[inline]
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.next_bound(items.len() as u64) as usize]
    }
}

// ---------------------------------------------------------------------------
// Helpers to generate realistic variable parts
// ---------------------------------------------------------------------------

fn gen_ipv4(rng: &mut Rng) -> String {
    let subnets: &[(u8, u8, u8)] = &[
        (10, 0, 1),
        (10, 0, 2),
        (10, 0, 3),
        (172, 16, 0),
        (172, 16, 1),
        (192, 168, 1),
        (192, 168, 2),
    ];
    let (a, b, c) = subnets[rng.next_bound(subnets.len() as u64) as usize];
    let d = rng.next_bound(254) + 1;
    format!("{a}.{b}.{c}.{d}")
}

fn gen_trace_id(rng: &mut Rng) -> String {
    format!("{:016x}", rng.next_u64())
}

fn gen_uuid(rng: &mut Rng) -> String {
    let a = rng.next_u64();
    let b = rng.next_u64();
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        (a >> 32) as u32,
        (a >> 16) as u16 & 0xffff,
        a as u16 & 0x0fff,
        ((b >> 48) as u16 & 0x3fff) | 0x8000,
        b & 0xffff_ffff_ffff
    )
}

fn gen_http_method(rng: &mut Rng) -> &'static str {
    *rng.pick(&["GET", "POST", "PUT", "DELETE", "PATCH"])
}

fn gen_http_path(rng: &mut Rng) -> &'static str {
    *rng.pick(&[
        "/api/v1/users",
        "/api/v1/orders",
        "/api/v1/products",
        "/api/v2/search",
        "/api/v1/checkout",
        "/api/v1/auth/login",
        "/api/v1/auth/refresh",
        "/api/v1/inventory",
        "/api/v1/notifications",
        "/health",
        "/metrics",
        "/api/v1/payments",
    ])
}

fn gen_status_code(rng: &mut Rng) -> u16 {
    let r = rng.next_bound(100);
    match r {
        0..60 => 200,
        60..70 => 201,
        70..75 => 204,
        75..78 => 301,
        78..82 => 400,
        82..85 => 401,
        85..88 => 403,
        88..92 => 404,
        92..95 => 429,
        95..98 => 500,
        _ => 503,
    }
}

fn gen_user_id(rng: &mut Rng) -> u64 {
    rng.next_bound(99999) + 10000
}

fn gen_duration_ms(rng: &mut Rng) -> f64 {
    // Bimodal: most fast, some slow
    if rng.next_bound(100) < 85 {
        rng.next_f64() * 50.0 + 1.0
    } else {
        rng.next_f64() * 4000.0 + 200.0
    }
}

fn gen_db_table(rng: &mut Rng) -> &'static str {
    *rng.pick(&["users", "orders", "products", "sessions", "payments", "inventory", "audit_log"])
}

fn gen_cache_key(rng: &mut Rng) -> String {
    let prefixes = ["user", "session", "product", "config", "rate_limit"];
    let prefix = rng.pick(prefixes.as_slice());
    let id = rng.next_bound(10000);
    format!("{prefix}:{id}")
}

fn gen_log_level_for_status(status: u16) -> &'static str {
    match status {
        200..=299 => "INFO",
        300..=399 => "INFO",
        400..=499 => "WARN",
        _ => "ERROR",
    }
}

fn gen_pool_name(rng: &mut Rng) -> &'static str {
    *rng.pick(&["primary", "replica", "analytics", "cache"])
}

fn gen_job_name(rng: &mut Rng) -> &'static str {
    *rng.pick(&[
        "email_digest",
        "cleanup_expired",
        "sync_inventory",
        "generate_report",
        "process_webhooks",
        "reindex_search",
    ])
}

fn gen_tls_version(rng: &mut Rng) -> &'static str {
    *rng.pick(&["TLSv1.2", "TLSv1.3"])
}

fn gen_cipher(rng: &mut Rng) -> &'static str {
    *rng.pick(&[
        "TLS_AES_256_GCM_SHA384",
        "TLS_CHACHA20_POLY1305_SHA256",
        "TLS_AES_128_GCM_SHA256",
        "ECDHE-RSA-AES128-GCM-SHA256",
    ])
}

fn gen_service(rng: &mut Rng) -> &'static str {
    *rng.pick(&["api-gateway", "auth-service", "order-service", "user-service", "payment-service"])
}

fn gen_hostname(rng: &mut Rng) -> String {
    let svc = gen_service(rng);
    let idx = rng.next_bound(5);
    format!("{svc}-{idx}")
}

fn gen_region(rng: &mut Rng) -> &'static str {
    *rng.pick(&["us-east-1", "us-west-2", "eu-west-1", "ap-southeast-1"])
}

fn gen_disk_path(rng: &mut Rng) -> &'static str {
    *rng.pick(&["/dev/sda1", "/dev/nvme0n1p1", "/dev/xvda1"])
}

// ---------------------------------------------------------------------------
// Timestamp helper — increments ~1s per 100 lines
// ---------------------------------------------------------------------------

struct TimestampGen {
    base_secs: i64, // seconds since epoch
    sub_ms: u32,    // sub-second millis counter
}

impl TimestampGen {
    fn new() -> Self {
        // 2026-01-15T08:00:00Z
        Self {
            base_secs: 1768492800,
            sub_ms: 0,
        }
    }

    /// Advance by roughly 10ms per line (so ~1s per 100 lines).
    fn next(&mut self, rng: &mut Rng) -> String {
        let jitter = rng.next_bound(15) as u32; // 0-14 ms jitter
        self.sub_ms += 10 + jitter; // ~10-24ms per line
        if self.sub_ms >= 1000 {
            self.base_secs += (self.sub_ms / 1000) as i64;
            self.sub_ms %= 1000;
        }

        let total_secs = self.base_secs;
        // Decompose into date/time components (UTC)
        let days = total_secs / 86400;
        let day_secs = (total_secs % 86400) as u32;
        let h = day_secs / 3600;
        let m = (day_secs % 3600) / 60;
        let s = day_secs % 60;
        let ms = self.sub_ms;

        // Civil date from day count (algorithm from Howard Hinnant)
        let z = days + 719468;
        let era = if z >= 0 { z } else { z - 146096 } / 146097;
        let doe = (z - era * 146097) as u32;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let y = yoe as i64 + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let mo = if mp < 10 { mp + 3 } else { mp - 9 };
        let yr = if mo <= 2 { y + 1 } else { y };

        format!("{yr:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}.{ms:03}Z")
    }
}

// ---------------------------------------------------------------------------
// Pattern generators — each writes one log line to the buffer
// ---------------------------------------------------------------------------

fn emit_http_request(buf: &mut Vec<u8>, ts: &str, trace: &str, rng: &mut Rng) {
    let method = gen_http_method(rng);
    let path = gen_http_path(rng);
    let status = gen_status_code(rng);
    let level = gen_log_level_for_status(status);
    let ip = gen_ipv4(rng);
    let dur = gen_duration_ms(rng);
    let uid = gen_user_id(rng);
    let bytes = rng.next_bound(50000) + 200;
    let _ = write!(
        buf,
        "{ts} {level} [{trace}] HTTP {method} {path} status={status} client={ip} user_id={uid} duration={dur:.2}ms bytes={bytes}\n"
    );
}

fn emit_conn_pool(buf: &mut Vec<u8>, ts: &str, trace: &str, rng: &mut Rng) {
    let pool = gen_pool_name(rng);
    let active = rng.next_bound(50) + 10;
    let max = 100u64;
    let wait = gen_duration_ms(rng);
    let _ = write!(
        buf,
        "{ts} WARN [{trace}] Connection pool '{pool}' reaching capacity: active={active}/{max} avg_wait={wait:.1}ms\n"
    );
}

fn emit_timeout(buf: &mut Vec<u8>, ts: &str, trace: &str, rng: &mut Rng) {
    let host = gen_ipv4(rng);
    let port = rng.pick(&[5432u16, 6379, 3306, 9200, 27017]);
    let timeout_ms = rng.next_bound(30000) + 5000;
    let svc = gen_service(rng);
    let _ = write!(
        buf,
        "{ts} ERROR [{trace}] Timeout connecting to {svc} at {host}:{port} after {timeout_ms}ms\n"
    );
}

fn emit_auth(buf: &mut Vec<u8>, ts: &str, trace: &str, rng: &mut Rng) {
    let ip = gen_ipv4(rng);
    let uid = gen_user_id(rng);
    if rng.next_bound(100) < 75 {
        let method = rng.pick(&["password", "oauth2", "sso", "api_key"]);
        let _ = write!(
            buf,
            "{ts} INFO [{trace}] Authentication successful for user_id={uid} from {ip} method={method}\n"
        );
    } else {
        let reason = rng.pick(&[
            "invalid_password",
            "expired_token",
            "account_locked",
            "mfa_required",
        ]);
        let _ = write!(
            buf,
            "{ts} WARN [{trace}] Authentication failed for user_id={uid} from {ip} reason={reason}\n"
        );
    }
}

fn emit_db_query(buf: &mut Vec<u8>, ts: &str, trace: &str, rng: &mut Rng) {
    let table = gen_db_table(rng);
    let op = rng.pick(&["SELECT", "INSERT", "UPDATE", "DELETE"]);
    let dur = gen_duration_ms(rng);
    let rows = rng.next_bound(1000);
    let level = if dur > 500.0 { "WARN" } else { "INFO" };
    let _ = write!(
        buf,
        "{ts} {level} [{trace}] Database query {op} on {table}: rows={rows} duration={dur:.2}ms\n"
    );
}

fn emit_cache(buf: &mut Vec<u8>, ts: &str, trace: &str, rng: &mut Rng) {
    let key = gen_cache_key(rng);
    let hit = rng.next_bound(100) < 70;
    if hit {
        let age = rng.next_bound(3600);
        let _ = write!(
            buf,
            "{ts} DEBUG [{trace}] Cache HIT key={key} age={age}s\n"
        );
    } else {
        let reason = rng.pick(&["expired", "evicted", "missing"]);
        let _ = write!(
            buf,
            "{ts} DEBUG [{trace}] Cache MISS key={key} reason={reason}\n"
        );
    }
}

fn emit_background_job(buf: &mut Vec<u8>, ts: &str, trace: &str, rng: &mut Rng) {
    let job = gen_job_name(rng);
    let job_id = gen_uuid(rng);
    let dur = rng.next_bound(60000) + 100;
    let items = rng.next_bound(10000);
    let status = rng.pick(&["completed", "completed", "completed", "failed", "retrying"]);
    let level = if *status == "failed" {
        "ERROR"
    } else if *status == "retrying" {
        "WARN"
    } else {
        "INFO"
    };
    let _ = write!(
        buf,
        "{ts} {level} [{trace}] Background job '{job}' id={job_id} status={status} processed={items} duration={dur}ms\n"
    );
}

fn emit_memory_warning(buf: &mut Vec<u8>, ts: &str, trace: &str, rng: &mut Rng) {
    let hostname = gen_hostname(rng);
    let used_pct = 80.0 + rng.next_f64() * 18.0;
    let used_mb = (rng.next_bound(8000) + 4000) as f64;
    let total_mb = (used_mb / used_pct * 100.0).round();
    let _ = write!(
        buf,
        "{ts} WARN [{trace}] Memory usage on {hostname}: {used_pct:.1}% ({used_mb:.0}MB / {total_mb:.0}MB)\n"
    );
}

fn emit_gc_pause(buf: &mut Vec<u8>, ts: &str, trace: &str, rng: &mut Rng) {
    let hostname = gen_hostname(rng);
    let pause_ms = rng.next_f64() * 200.0 + 10.0;
    let gc_gen = rng.pick(&["young", "old", "full"]);
    let freed_mb = rng.next_bound(512) + 16;
    let _ = write!(
        buf,
        "{ts} WARN [{trace}] GC pause on {hostname}: type={gc_gen} pause={pause_ms:.1}ms freed={freed_mb}MB\n"
    );
}

fn emit_rate_limit(buf: &mut Vec<u8>, ts: &str, trace: &str, rng: &mut Rng) {
    let ip = gen_ipv4(rng);
    let uid = gen_user_id(rng);
    let limit = rng.pick(&[100u32, 500, 1000, 5000]);
    let window = rng.pick(&["1m", "5m", "1h"]);
    let _ = write!(
        buf,
        "{ts} WARN [{trace}] Rate limit exceeded for user_id={uid} from {ip}: limit={limit}/{window}\n"
    );
}

fn emit_deployment(buf: &mut Vec<u8>, ts: &str, trace: &str, rng: &mut Rng) {
    let svc = gen_service(rng);
    let version = format!(
        "{}.{}.{}",
        rng.next_bound(5) + 1,
        rng.next_bound(20),
        rng.next_bound(100)
    );
    let region = gen_region(rng);
    let replicas = rng.next_bound(10) + 1;
    let phase = rng.pick(&["started", "healthy", "completed"]);
    let _ = write!(
        buf,
        "{ts} INFO [{trace}] Deployment {phase} for {svc} v{version} in {region}: replicas={replicas}\n"
    );
}

fn emit_health_check(buf: &mut Vec<u8>, ts: &str, trace: &str, rng: &mut Rng) {
    let svc = gen_service(rng);
    let hostname = gen_hostname(rng);
    let latency = rng.next_f64() * 20.0 + 0.5;
    let status = if rng.next_bound(100) < 95 {
        "healthy"
    } else {
        "degraded"
    };
    let level = if status == "healthy" { "INFO" } else { "WARN" };
    let _ = write!(
        buf,
        "{ts} {level} [{trace}] Health check {status} for {svc} on {hostname}: latency={latency:.1}ms\n"
    );
}

fn emit_tls_handshake(buf: &mut Vec<u8>, ts: &str, trace: &str, rng: &mut Rng) {
    let ip = gen_ipv4(rng);
    let version = gen_tls_version(rng);
    let cipher = gen_cipher(rng);
    let dur = rng.next_f64() * 50.0 + 2.0;
    let _ = write!(
        buf,
        "{ts} INFO [{trace}] TLS handshake completed with {ip}: version={version} cipher={cipher} duration={dur:.1}ms\n"
    );
}

fn emit_disk_io(buf: &mut Vec<u8>, ts: &str, trace: &str, rng: &mut Rng) {
    let hostname = gen_hostname(rng);
    let disk = gen_disk_path(rng);
    let read_mbps = rng.next_f64() * 500.0 + 10.0;
    let write_mbps = rng.next_f64() * 300.0 + 5.0;
    let iops = rng.next_bound(50000) + 100;
    let util_pct = rng.next_f64() * 100.0;
    let _ = write!(
        buf,
        "{ts} INFO [{trace}] Disk IO on {hostname} {disk}: read={read_mbps:.1}MB/s write={write_mbps:.1}MB/s iops={iops} util={util_pct:.1}%\n"
    );
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn parse_args() -> (u64, u64) {
    let args: Vec<String> = std::env::args().collect();
    let mut lines: u64 = 10000;
    let mut seed: u64 = 42;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--lines" => {
                i += 1;
                lines = args[i].parse().expect("--lines must be a number");
            }
            "--seed" => {
                i += 1;
                seed = args[i].parse().expect("--seed must be a number");
            }
            other => {
                eprintln!("Unknown argument: {other}");
                eprintln!("Usage: generate_logs [--lines N] [--seed N]");
                std::process::exit(1);
            }
        }
        i += 1;
    }
    (lines, seed)
}

fn main() {
    let (lines, seed) = parse_args();
    let mut rng = Rng::new(seed);
    let mut ts_gen = TimestampGen::new();

    let stdout = std::io::stdout().lock();
    let mut out = BufWriter::with_capacity(256 * 1024, stdout);

    // Reusable buffer for formatting a single line before writing to BufWriter.
    let mut line_buf: Vec<u8> = Vec::with_capacity(512);

    // Cumulative distribution for pattern selection (out of 1000):
    //  HTTP requests:       350
    //  Connection pool:      80  -> 430
    //  Timeout:              70  -> 500
    //  Auth:                120  -> 620
    //  Database:             80  -> 700
    //  Cache:                80  -> 780
    //  Background jobs:      50  -> 830
    //  Memory warnings:      30  -> 860
    //  GC pauses:            30  -> 890
    //  Rate limiter:         30  -> 920
    //  Deployment:           20  -> 940
    //  Health checks:        30  -> 970
    //  TLS handshakes:       20  -> 990
    //  Disk IO:              10  -> 1000

    for _ in 0..lines {
        let ts = ts_gen.next(&mut rng);
        let trace = gen_trace_id(&mut rng);

        line_buf.clear();

        let r = rng.next_bound(1000);
        match r {
            0..350 => emit_http_request(&mut line_buf, &ts, &trace, &mut rng),
            350..430 => emit_conn_pool(&mut line_buf, &ts, &trace, &mut rng),
            430..500 => emit_timeout(&mut line_buf, &ts, &trace, &mut rng),
            500..620 => emit_auth(&mut line_buf, &ts, &trace, &mut rng),
            620..700 => emit_db_query(&mut line_buf, &ts, &trace, &mut rng),
            700..780 => emit_cache(&mut line_buf, &ts, &trace, &mut rng),
            780..830 => emit_background_job(&mut line_buf, &ts, &trace, &mut rng),
            830..860 => emit_memory_warning(&mut line_buf, &ts, &trace, &mut rng),
            860..890 => emit_gc_pause(&mut line_buf, &ts, &trace, &mut rng),
            890..920 => emit_rate_limit(&mut line_buf, &ts, &trace, &mut rng),
            920..940 => emit_deployment(&mut line_buf, &ts, &trace, &mut rng),
            940..970 => emit_health_check(&mut line_buf, &ts, &trace, &mut rng),
            970..990 => emit_tls_handshake(&mut line_buf, &ts, &trace, &mut rng),
            _ => emit_disk_io(&mut line_buf, &ts, &trace, &mut rng),
        }

        out.write_all(&line_buf).unwrap();
    }

    out.flush().unwrap();
}
