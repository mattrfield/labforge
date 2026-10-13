FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends openssh-server vsftpd procps \
 && rm -rf /var/lib/apt/lists/* \
 && useradd -m -s /bin/bash concierge \
 && mkdir -p /srv/ftp/pub /var/run/vsftpd/empty /run/sshd /opt/hotel \
 && chmod 0555 /srv/ftp
COPY vsftpd.conf /etc/vsftpd.conf
COPY entrypoint.sh /entrypoint.sh
RUN chmod 0755 /entrypoint.sh
EXPOSE 21 22
ENTRYPOINT ["/entrypoint.sh"]
