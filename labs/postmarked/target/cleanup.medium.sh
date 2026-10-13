#!/bin/sh
# Sorting-hall housekeeping (runs as root from the scheduler loop): archive the label spool.
cd /var/spool/postbox && tar czf /var/backups/spool.tgz * 2>/dev/null
find /var/spool/postbox -type f -name 'label-*.txt' -mmin +60 -delete
