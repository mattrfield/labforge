FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends openssh-server redis-server procps \
 && rm -rf /var/lib/apt/lists/* \
 && mkdir -p /run/sshd /root/.ssh /var/lib/hotel-cache \
 && chmod 0700 /root/.ssh \
 && printf 'PermitRootLogin prohibit-password\nPasswordAuthentication no\nPubkeyAuthentication yes\n' > /etc/ssh/sshd_config.d/hotel.conf
COPY redis.conf /etc/hotel-redis.conf
COPY entrypoint.sh /entrypoint.sh
RUN chmod 0755 /entrypoint.sh
EXPOSE 22 6379
ENTRYPOINT ["/entrypoint.sh"]
