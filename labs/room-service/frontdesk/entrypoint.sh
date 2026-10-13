#!/bin/sh
# Applies the difficulty, then runs vsftpd in the background and sshd in the foreground.
set -e
BACKOFFICE_PASS='Gr4ndH0tel-Pms'

case "$DIFFICULTY" in
  easy)
    PW='Welcome2Room101'
    cat > /srv/ftp/pub/wifi_export.csv <<EOF
# Staff Wi-Fi / terminal accounts - exported for the new router
user,password,department
concierge,$PW,front desk
maintenance,Hammer#2024,facilities
EOF
    cat > /home/concierge/notes.txt <<EOF
TODO
- put a password on the back-office cache (backoffice:6379). Never got round to it.
- stop reusing the Wi-Fi password for everything
EOF
    chown concierge:concierge /home/concierge/notes.txt
    ;;
  *)
    PW='sunshine1'
    HASH=$(printf '%s' "$PW" | md5sum | cut -d' ' -f1)
    cat > /srv/ftp/pub/wifi_export.csv <<EOF
# Staff Wi-Fi / terminal accounts - exported for the new router (passwords hashed, md5)
user,md5,department
concierge,$HASH,front desk
maintenance,$(printf '%s' "Xk9#vT2mQ" | md5sum | cut -d' ' -f1),facilities
EOF
    # Credentials for the property-management cache live in app config, readable by the front desk.
    cat > /opt/hotel/pms.conf <<EOF
# Property management system
cache_host=backoffice
cache_port=6379
cache_password=$BACKOFFICE_PASS
EOF
    chown root:concierge /opt/hotel/pms.conf
    chmod 0640 /opt/hotel/pms.conf
    ;;
esac

echo "concierge:$PW" | chpasswd
ssh-keygen -A >/dev/null
vsftpd /etc/vsftpd.conf &
exec /usr/sbin/sshd -D -e
