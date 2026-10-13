FROM python:3.12-slim-bookworm
RUN apt-get update \
 && apt-get install -y --no-install-recommends procps tar \
 && rm -rf /var/lib/apt/lists/* \
 && pip install --no-cache-dir flask==3.0.3 PyJWT==2.9.0 \
 && useradd -m -s /bin/bash courier \
 && mkdir -p /srv/app /opt/postbox /var/spool/postbox /var/backups \
 && chown courier:courier /var/spool/postbox
COPY app.py /srv/app/app.py
COPY cleanup.easy.sh cleanup.medium.sh /usr/local/share/postbox/
COPY entrypoint.sh /entrypoint.sh
RUN chmod 0755 /entrypoint.sh
EXPOSE 80
ENTRYPOINT ["/entrypoint.sh"]
