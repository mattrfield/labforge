FROM debian:bookworm-slim AS build
RUN apt-get update && apt-get install -y --no-install-recommends gcc libc6-dev && rm -rf /var/lib/apt/lists/*
COPY brewctl.c /tmp/brewctl.c
RUN gcc -O2 -o /tmp/brewctl /tmp/brewctl.c

FROM python:3.12-slim-bookworm
RUN apt-get update \
 && apt-get install -y --no-install-recommends iputils-ping procps \
 && rm -rf /var/lib/apt/lists/* \
 && pip install --no-cache-dir flask==3.0.3 \
 && useradd -m -s /bin/bash barista \
 && mkdir -p /srv/app/data /etc/brewctl \
 && echo "Espresso machine: ONLINE (boiler 93C)" > /etc/brewctl/status
COPY --from=build /tmp/brewctl /usr/local/bin/brewctl
COPY app.py /srv/app/app.py
COPY entrypoint.sh /entrypoint.sh
RUN chmod 0755 /usr/local/bin/brewctl /entrypoint.sh \
 && chown -R barista:barista /srv/app
EXPOSE 80
ENTRYPOINT ["/entrypoint.sh"]
