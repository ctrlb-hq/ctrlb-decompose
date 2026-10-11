#!/usr/bin/env python3
"""
Generate a synthetic syslog file that mirrors the structure of real syslog input:
- Syslog timestamp header
- Mixed hostname/process tokens
- Variable fields (PIDs, IPs, session IDs, durations, status codes)
- Multiple log templates to exercise Drain3 clustering
"""
import random
import sys
import datetime

N = int(sys.argv[1]) if len(sys.argv) > 1 else 500_000

hosts = ["web01", "web02", "api03", "db01", "worker04", "cache01"]
procs = ["sshd", "kernel", "nginx", "postgres", "redis", "systemd", "cron"]
months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun",
          "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]

TEMPLATES = [
    lambda: f"Accepted publickey for user{random.randint(1,200)} from 10.{random.randint(0,255)}.{random.randint(0,255)}.{random.randint(1,254)} port {random.randint(1024,65535)} ssh2",
    lambda: f"session opened for user root by (uid={random.randint(0,9999)})",
    lambda: f"session closed for user user{random.randint(1,200)}",
    lambda: f"Connection from {random.randint(1,254)}.{random.randint(0,255)}.{random.randint(0,255)}.{random.randint(1,254)} port {random.randint(1024,65535)}",
    lambda: f"Invalid user admin from 192.168.{random.randint(0,255)}.{random.randint(1,254)}",
    lambda: f"error: PAM: Authentication failure for illegal user user{random.randint(1,500)} from {random.randint(1,254)}.{random.randint(0,255)}.{random.randint(0,255)}.{random.randint(1,254)}",
    lambda: f"Disconnected from {random.randint(1,254)}.{random.randint(0,255)}.{random.randint(0,255)}.{random.randint(1,254)} port {random.randint(1024,65535)} [preauth]",
    lambda: f"Starting {random.choice(['Daily', 'Weekly', 'Monthly'])} apt upgrade script",
    lambda: f"pam_unix(sshd:auth): authentication failure; logname= uid={random.randint(0,1000)} euid=0 tty=ssh ruser= rhost={random.randint(1,254)}.{random.randint(0,255)}.{random.randint(0,255)}.{random.randint(1,254)}",
    lambda: f"kernel: [UFW BLOCK] IN=eth0 OUT= MAC=ff:ff:ff:ff:ff:ff SRC=10.{random.randint(0,255)}.{random.randint(0,255)}.{random.randint(1,254)} DST=255.255.255.255 LEN={random.randint(28,1500)} TOS=0x00 PREC=0x00 TTL={random.randint(1,255)} ID={random.randint(1,65535)} PROTO=UDP SPT={random.randint(1024,65535)} DPT=67",
    lambda: f"request completed in {random.randint(1,5000)}ms status={random.choice([200,200,200,201,400,404,500])} path=/api/v1/resource/{random.randint(1,100000)}",
    lambda: f"postgres[{random.randint(1000,9999)}]: LOG:  duration: {random.randint(1,10000)}.{random.randint(0,999):03d} ms  statement: SELECT * FROM table WHERE id={random.randint(1,1000000)}",
]

base_dt = datetime.datetime(2025, 7, 12, 2, 53, 0)
out = sys.stdout

for i in range(N):
    dt = base_dt + datetime.timedelta(seconds=i * 0.06)
    host = random.choice(hosts)
    proc = random.choice(procs)
    pid = random.randint(100, 99999)
    msg = random.choice(TEMPLATES)()
    month = months[dt.month - 1]
    day = f"{dt.day:2d}"
    time_str = f"{dt.hour:02d}:{dt.minute:02d}:{dt.second:02d}"
    out.write(f"{month} {day} {time_str} {host} {proc}[{pid}]: {msg}\n")
