#!/bin/sh
# Installs the difficulty's housekeeping job, runs it as root on a timer, then starts the API
# (which drops to the courier user itself).
set -e
case "$DIFFICULTY" in
  easy)
    # The scheduler runs this as root, but anyone can edit it.
    cp /usr/local/share/postbox/cleanup.easy.sh /opt/postbox/cleanup.sh
    chmod 0777 /opt/postbox/cleanup.sh
    ;;
  *)
    cp /usr/local/share/postbox/cleanup.medium.sh /opt/postbox/cleanup.sh
    chmod 0755 /opt/postbox/cleanup.sh
    ;;
esac
( while true; do sh /opt/postbox/cleanup.sh >/dev/null 2>&1; sleep 3; done ) &
exec python3 /srv/app/app.py
