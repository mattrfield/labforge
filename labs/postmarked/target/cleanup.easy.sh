#!/bin/sh
# Sorting-hall housekeeping (runs as root from the scheduler loop): drop stale label previews.
find /var/spool/postbox -type f -name 'label-*.txt' -mmin +60 -delete
