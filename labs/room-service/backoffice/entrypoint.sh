#!/bin/sh
# Applies the difficulty, then runs Redis (as root, like the hotel's admin left it) and sshd.
set -e
cp /etc/hotel-redis.conf /tmp/redis.conf
case "$DIFFICULTY" in
  easy) ;;
  *) echo "requirepass Gr4ndH0tel-Pms" >> /tmp/redis.conf ;;
esac
ssh-keygen -A >/dev/null
redis-server /tmp/redis.conf --logfile /var/log/redis.log &
exec /usr/sbin/sshd -D -e
