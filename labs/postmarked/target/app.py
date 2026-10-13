"""Postmarked: a deliberately vulnerable parcel-tracking API (LabForge lab).

Vulnerabilities, by design:
  * GET   /api/v1/parcels/<id>             - broken object-level authorization (IDOR/BOLA)
  * PATCH /api/v1/me           (easy)      - mass assignment of `role`
  * JWT secret is weak         (medium)    - roles are read from the token, which can be forged
  * POST  /api/v1/admin/labels/preview     - server-side template injection (Jinja2)

DIFFICULTY=easy   chatty errors, full docs, roles read from the database
DIFFICULTY=medium terse errors, admin routes undocumented, roles read from the token, substring filter on templates
"""
import os
import pwd
import secrets
import sys
import time

import jwt
from flask import Flask, jsonify, render_template_string, request
from werkzeug.serving import make_server

DIFFICULTY = os.environ.get("DIFFICULTY", "easy").lower()
EASY = DIFFICULTY == "easy"
SPOOL = "/var/spool/postbox"
JWT_SECRET = secrets.token_hex(16) if EASY else "password123"

app = Flask(__name__)

USERS = {}
PARCELS = {}
_next_parcel = [1]


def add_user(username, password, role):
    uid = len(USERS) + 1
    USERS[uid] = {
        "id": uid, "username": username, "password": password, "role": role,
        "email": "%s@postmarked.example" % username, "display_name": username.title(),
    }
    return USERS[uid]


def add_parcel(owner_id, dest, note="", status="in_transit"):
    pid = _next_parcel[0]
    _next_parcel[0] += 1
    PARCELS[pid] = {"id": pid, "owner_id": owner_id, "tracking": "PM-%06d" % pid,
                    "status": status, "dest": dest, "note": note}
    return PARCELS[pid]


def seed():
    dispatch = add_user("dispatch", secrets.token_hex(12), "admin")
    alice = add_user("alice", secrets.token_hex(12), "customer")
    bob = add_user("bob", secrets.token_hex(12), "customer")
    add_parcel(dispatch["id"], "Sorting Hall",
               "Label previews for the sorting hall: POST /api/v1/admin/labels/preview (admin token). "
               "Templates can use {{ parcel.tracking }} and {{ parcel.dest }}.")
    add_parcel(alice["id"], "12 Elm Street", "Leave with neighbour")
    add_parcel(bob["id"], "98 Harbour Road", "Fragile", status="delivered")


def public(user):
    return {k: v for k, v in user.items() if k != "password"}


def fail(status, verbose, terse="request failed"):
    return jsonify({"error": verbose if EASY else terse}), status


def current_user():
    header = request.headers.get("Authorization", "")
    if not header.startswith("Bearer "):
        return None, "missing bearer token"
    try:
        claims = jwt.decode(header[7:], JWT_SECRET, algorithms=["HS256"])
    except jwt.PyJWTError as e:
        return None, "invalid token: %s" % e
    user = USERS.get(claims.get("sub"))
    if user is None:
        return None, "unknown user"
    # Easy trusts the database. Medium trusts whatever the token says.
    role = user["role"] if EASY else claims.get("role", "customer")
    return dict(user, role=role), None


def authed(f):
    from functools import wraps

    @wraps(f)
    def wrapper(*a, **kw):
        user, why = current_user()
        if user is None:
            return fail(401, why, "unauthorized")
        request.user = user
        return f(*a, **kw)
    return wrapper


@app.route("/health")
def health():
    return "ok"


@app.route("/api/v1/docs")
def docs():
    routes = [
        {"method": "POST", "path": "/api/v1/auth/register", "body": {"username": "str", "password": "str"}},
        {"method": "POST", "path": "/api/v1/auth/login", "body": {"username": "str", "password": "str"}},
        {"method": "GET", "path": "/api/v1/me"},
        {"method": "PATCH", "path": "/api/v1/me", "desc": "Update your profile"},
        {"method": "GET", "path": "/api/v1/parcels", "desc": "Your parcels"},
        {"method": "POST", "path": "/api/v1/parcels", "body": {"dest": "str", "note": "str"}},
        {"method": "GET", "path": "/api/v1/parcels/<id>"},
    ]
    if EASY:
        routes.append({"method": "POST", "path": "/api/v1/admin/labels/preview", "desc": "Admins only"})
    return jsonify({"name": "Postmarked API", "version": "1.4", "routes": routes})


@app.route("/api/v1/auth/register", methods=["POST"])
def register():
    body = request.get_json(silent=True) or {}
    username, password = str(body.get("username", "")), str(body.get("password", ""))
    if not (3 <= len(username) <= 32) or not password:
        return fail(400, "username must be 3-32 chars and password is required", "bad request")
    if any(u["username"] == username for u in USERS.values()):
        return fail(409, "username already taken", "conflict")
    user = add_user(username, password, "customer")
    return jsonify(public(user)), 201


@app.route("/api/v1/auth/login", methods=["POST"])
def login():
    body = request.get_json(silent=True) or {}
    for u in USERS.values():
        if u["username"] == body.get("username") and u["password"] == body.get("password"):
            token = jwt.encode({"sub": u["id"], "role": u["role"], "iat": int(time.time())},
                               JWT_SECRET, algorithm="HS256")
            return jsonify({"token": token})
    return fail(401, "wrong username or password", "unauthorized")


@app.route("/api/v1/me", methods=["GET", "PATCH"])
@authed
def me():
    user = USERS[request.user["id"]]
    if request.method == "PATCH":
        body = request.get_json(silent=True) or {}
        if EASY:
            # VULNERABLE: every field in the body is applied, including `role`.
            for k, v in body.items():
                if k not in ("id", "password"):
                    user[k] = v
        else:
            for k in ("display_name", "email"):
                if k in body:
                    user[k] = str(body[k])[:100]
    return jsonify(public(user))


@app.route("/api/v1/parcels", methods=["GET", "POST"])
@authed
def parcels():
    if request.method == "POST":
        body = request.get_json(silent=True) or {}
        p = add_parcel(request.user["id"], str(body.get("dest", ""))[:100], str(body.get("note", ""))[:200], "created")
        return jsonify(p), 201
    return jsonify([p for p in PARCELS.values() if p["owner_id"] == request.user["id"]])


@app.route("/api/v1/parcels/<int:pid>")
@authed
def parcel(pid):
    p = PARCELS.get(pid)
    if p is None:
        return fail(404, "no parcel with id %d" % pid, "not found")
    # VULNERABLE: never checks that the parcel belongs to the caller.
    return jsonify(p)


def template_blocked(t):
    # Homemade substring blacklist (intentionally weak).
    return any(bad in t for bad in ("__", "import", "config"))


@app.route("/api/v1/admin/labels/preview", methods=["POST"])
@authed
def label_preview():
    if request.user["role"] != "admin":
        return fail(403, "requires role admin (you are %s)" % request.user["role"], "forbidden")
    body = request.get_json(silent=True) or {}
    template = str(body.get("template", ""))
    p = PARCELS.get(int(body.get("parcel_id", 1)) if str(body.get("parcel_id", 1)).isdigit() else 1, PARCELS[1])
    if not EASY and template_blocked(template):
        return fail(400, "", "template rejected")
    try:
        # VULNERABLE: user-controlled template rendered by Jinja2.
        label = render_template_string(template, parcel=p)
    except Exception as e:
        return fail(400, "template error: %s: %s" % (type(e).__name__, e), "template error")
    try:
        with open(os.path.join(SPOOL, "label-%d.txt" % int(time.time() * 1000)), "w") as f:
            f.write(label)
    except OSError:
        pass
    return jsonify({"label": label})


@app.errorhandler(404)
def nf(_):
    return jsonify({"error": "not found"}), 404


@app.errorhandler(405)
def na(_):
    return jsonify({"error": "method not allowed"}), 405


def drop_privileges(user="courier"):
    pw = pwd.getpwnam(user)
    os.setgroups([])
    os.setgid(pw.pw_gid)
    os.setuid(pw.pw_uid)
    os.environ.update(HOME=pw.pw_dir, USER=user, LOGNAME=user)
    os.chdir(pw.pw_dir)


if __name__ == "__main__":
    seed()
    # Bind the privileged port as root, then drop to the courier user.
    server = make_server("0.0.0.0", 80, app, threaded=True)
    if os.geteuid() == 0:
        drop_privileges()
    print("Postmarked listening on :80 (%s)" % DIFFICULTY, file=sys.stderr, flush=True)
    server.serve_forever()
