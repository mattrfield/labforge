"""Bean There: a deliberately vulnerable coffee shop site (LabForge lab).

Vulnerabilities, by design:
  * /staff/login   - SQL injection (string-built query)
  * /admin/diag    - OS command injection (ping via shell=True)

DIFFICULTY=easy   verbose errors, no input filtering
DIFFICULTY=medium quiet errors, homemade blacklist filters on both inputs
"""
import os
import pwd
import secrets
import sqlite3
import subprocess
import sys

from flask import Flask, redirect, render_template_string, request, session, url_for
from werkzeug.serving import make_server

DIFFICULTY = os.environ.get("DIFFICULTY", "easy").lower()
EASY = DIFFICULTY == "easy"
DB_PATH = "/srv/app/data/shop.db"

app = Flask(__name__)
app.secret_key = secrets.token_hex(16)

MENU = [
    ("Espresso", "Double shot, house blend", 3.00),
    ("Flat White", "Silky microfoam over a double ristretto", 4.20),
    ("Cappuccino", "Equal parts espresso, milk and foam", 4.00),
    ("Cold Brew", "Steeped for 18 hours", 4.50),
    ("Oat Latte", "Barista-grade oat milk", 4.60),
    ("Almond Croissant", "Baked this morning", 3.80),
]

BASE = """<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Bean There{% if title %} - {{ title }}{% endif %}</title>
<style>
  body{font-family:Georgia,serif;background:#f6efe6;color:#3b2a20;margin:0}
  header{background:#3b2a20;color:#f6efe6;padding:14px 24px;display:flex;gap:24px;align-items:baseline}
  header a{color:#e8c9a0;text-decoration:none} header h1{margin:0;font-size:22px;margin-right:auto}
  main{max-width:720px;margin:24px auto;padding:0 16px}
  table{border-collapse:collapse;width:100%} td,th{padding:8px;border-bottom:1px solid #d9c8b4;text-align:left}
  input,select{padding:8px;margin:4px 0;width:100%;box-sizing:border-box}
  button{padding:8px 18px;background:#6f4e37;color:#fff;border:0;border-radius:4px;cursor:pointer}
  .err{background:#fbe3e0;border:1px solid #d98b82;padding:10px;border-radius:4px}
  .ok{background:#e4f1e0;border:1px solid #9bc48f;padding:10px;border-radius:4px}
  pre{background:#2a1d15;color:#e8d9c5;padding:12px;border-radius:4px;overflow:auto}
  .muted{color:#8a7565;font-size:14px}
</style></head><body>
<header><h1>&#9749; Bean There</h1>
<a href="/">Menu</a>
{% if session.get('user') %}<a href="/staff">Orders</a>
{% if session.get('role') == 'admin' %}<a href="/admin/diag">Diagnostics</a>{% endif %}
<a href="/staff/logout">Log out ({{ session['user'] }})</a>
{% else %}<a href="/staff/login">Staff</a>{% endif %}
</header><main>
{{ body|safe }}
</main></body></html>"""


def page(body_tpl, title="", **ctx):
    body = render_template_string(body_tpl, **ctx)
    return render_template_string(BASE, body=body, title=title)


def db():
    conn = sqlite3.connect(DB_PATH)
    conn.row_factory = sqlite3.Row
    return conn


def init_db():
    os.makedirs(os.path.dirname(DB_PATH), exist_ok=True)
    if os.path.exists(DB_PATH):
        os.remove(DB_PATH)
    conn = db()
    conn.executescript(
        """
        CREATE TABLE staff (id INTEGER PRIMARY KEY, username TEXT, password TEXT, role TEXT);
        CREATE TABLE orders (id INTEGER PRIMARY KEY, customer TEXT, item TEXT, qty INTEGER, note TEXT);
        """
    )
    # Random per start: the login can't be guessed, only bypassed.
    conn.execute("INSERT INTO staff (username, password, role) VALUES (?,?,?)",
                 ("admin", secrets.token_hex(12), "admin"))
    conn.execute("INSERT INTO staff (username, password, role) VALUES (?,?,?)",
                 ("maria", secrets.token_hex(12), "staff"))
    conn.executemany(
        "INSERT INTO orders (customer, item, qty, note) VALUES (?,?,?,?)",
        [("Dana", "Flat White", 2, "extra hot"), ("Priya", "Cold Brew", 1, ""),
         ("Tom", "Almond Croissant", 3, "for the office")],
    )
    conn.commit()
    conn.close()


# ---- medium-difficulty homemade filters (intentionally weak) ----

def login_blocked(value):
    # Case-sensitive blacklist: "OR" / "--" are blocked, "or" and "/*" are not.
    return any(bad in value for bad in ("OR", "--", "UNION", "SELECT"))


def host_blocked(value):
    # Blocks the obvious separators but forgets "|" and newlines.
    return any(bad in value for bad in (";", "&", "`", "$(", ">", "<"))


# ---- routes ----

@app.route("/health")
def health():
    return "ok"


@app.route("/")
def index():
    return page(
        """<h2>Today's menu</h2>
        <table>{% for n, d, p in menu %}<tr><td><b>{{ n }}</b><br><span class="muted">{{ d }}</span></td>
        <td>${{ '%.2f'|format(p) }}</td></tr>{% endfor %}</table>
        <h3>Order ahead</h3>
        {% if placed %}<p class="ok">Thanks! Your order is in.</p>{% endif %}
        <form method="post" action="/order">
          <label>Name <input name="customer" required></label>
          <label>Item <select name="item">{% for n, d, p in menu %}<option>{{ n }}</option>{% endfor %}</select></label>
          <label>Quantity <input name="qty" type="number" min="1" max="20" value="1"></label>
          <label>Note <input name="note"></label>
          <button>Place order</button>
        </form>""",
        menu=MENU, placed=request.args.get("placed"),
    )


@app.route("/order", methods=["POST"])
def order():
    try:
        qty = max(1, min(20, int(request.form.get("qty", "1"))))
    except ValueError:
        qty = 1
    conn = db()
    conn.execute(
        "INSERT INTO orders (customer, item, qty, note) VALUES (?,?,?,?)",
        (request.form.get("customer", "")[:60], request.form.get("item", "")[:60],
         qty, request.form.get("note", "")[:200]),
    )
    conn.commit()
    conn.close()
    return redirect(url_for("index", placed=1))


LOGIN_TPL = """<h2>Staff login</h2>
{% if error %}<div class="err">{{ error }}</div>{% endif %}
{% if query %}<pre>{{ query }}</pre>{% endif %}
<form method="post">
  <label>Username <input name="username" autocomplete="off"></label>
  <label>Password <input name="password" type="password"></label>
  <button>Log in</button>
</form>"""


@app.route("/staff/login", methods=["GET", "POST"])
def login():
    if request.method == "GET":
        return page(LOGIN_TPL, "Staff login")
    username = request.form.get("username", "")
    password = request.form.get("password", "")
    if not EASY and (login_blocked(username) or login_blocked(password)):
        return page(LOGIN_TPL, "Staff login", error="Invalid credentials."), 401
    # VULNERABLE: string-built SQL.
    query = ("SELECT id, username, role FROM staff WHERE username = '%s' AND password = '%s' ORDER BY id"
             % (username, password))
    conn = db()
    try:
        row = conn.execute(query).fetchone()
    except sqlite3.Error as e:
        conn.close()
        if EASY:
            return page(LOGIN_TPL, "Staff login", error="Database error: %s" % e, query=query), 500
        return page(LOGIN_TPL, "Staff login", error="Invalid credentials."), 401
    conn.close()
    if row is None:
        return page(LOGIN_TPL, "Staff login", error="Invalid credentials."), 401
    session["user"], session["role"] = row["username"], row["role"]
    return redirect(url_for("staff"))


@app.route("/staff/logout")
def logout():
    session.clear()
    return redirect(url_for("index"))


@app.route("/staff")
def staff():
    if not session.get("user"):
        return redirect(url_for("login"))
    conn = db()
    orders = conn.execute("SELECT * FROM orders ORDER BY id DESC").fetchall()
    conn.close()
    try:
        with open("/etc/brewctl/status") as f:
            machine = f.read().strip()
    except OSError:
        machine = "unknown"
    return page(
        """<h2>Open orders</h2>
        <p class="ok">{{ machine }}</p>
        <table><tr><th>#</th><th>Customer</th><th>Item</th><th>Qty</th><th>Note</th></tr>
        {% for o in orders %}<tr><td>{{ o['id'] }}</td><td>{{ o['customer'] }}</td><td>{{ o['item'] }}</td>
        <td>{{ o['qty'] }}</td><td>{{ o['note'] }}</td></tr>{% endfor %}</table>""",
        "Orders", orders=orders, machine=machine,
    )


DIAG_TPL = """<h2>Network diagnostics</h2>
<p class="muted">Check that the till, printer and espresso machine are reachable.</p>
{% if error %}<div class="err">{{ error }}</div>{% endif %}
<form method="post">
  <label>Host <input name="host" value="{{ host }}" placeholder="10.66.11.1"></label>
  <button>Ping</button>
</form>
{% if output is not none %}<pre>{{ output }}</pre>{% endif %}"""


@app.route("/admin/diag", methods=["GET", "POST"])
def diag():
    if not session.get("user"):
        return redirect(url_for("login"))
    if session.get("role") != "admin":
        return page('<div class="err">Admins only.</div>', "Forbidden"), 403
    host, output, error = "", None, None
    if request.method == "POST":
        host = request.form.get("host", "")
        if not EASY and host_blocked(host):
            error = "Invalid host."
        else:
            # VULNERABLE: user input interpolated into a shell command.
            proc = subprocess.run("ping -c 1 -W 1 " + host, shell=True,
                                  capture_output=True, text=True, timeout=15)
            output = proc.stdout + proc.stderr
    return page(DIAG_TPL, "Diagnostics", host=host, output=output, error=error)


def drop_privileges(user="barista"):
    pw = pwd.getpwnam(user)
    os.setgroups([])
    os.setgid(pw.pw_gid)
    os.setuid(pw.pw_uid)
    os.environ.update(HOME=pw.pw_dir, USER=user, LOGNAME=user)
    os.chdir(pw.pw_dir)


if __name__ == "__main__":
    # Bind the privileged port as root, then drop to the barista user.
    server = make_server("0.0.0.0", 80, app, threaded=True)
    if os.geteuid() == 0:
        pw = pwd.getpwnam("barista")
        os.makedirs(os.path.dirname(DB_PATH), exist_ok=True)
        os.chown(os.path.dirname(DB_PATH), pw.pw_uid, pw.pw_gid)
        drop_privileges()
    init_db()
    print("Bean There listening on :80 (%s)" % DIFFICULTY, file=sys.stderr, flush=True)
    server.serve_forever()
