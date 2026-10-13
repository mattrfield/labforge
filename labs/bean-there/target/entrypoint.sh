#!/bin/sh
# Applies the difficulty overlay, then starts the shop (it drops to the barista user itself).
set -e
case "$DIFFICULTY" in
  easy) chmod u+s /usr/bin/find ;;
  *)    chmod u+s /usr/local/bin/brewctl ;;
esac
exec python3 /srv/app/app.py
