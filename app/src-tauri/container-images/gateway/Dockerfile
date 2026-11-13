FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    python3 redsocks iptables ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY gateway.py /usr/local/bin/gateway.py
ENTRYPOINT ["python3", "-u", "/usr/local/bin/gateway.py"]
HEALTHCHECK --interval=5s --timeout=2s --retries=3 CMD test -f /run/gateway-ready
