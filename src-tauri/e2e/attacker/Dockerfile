# Throwaway "attacker box" for the Docker end-to-end tests: joins a lab's network and runs a solution script.
FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends python3 openssh-client sshpass redis-tools curl ca-certificates \
 && rm -rf /var/lib/apt/lists/*
