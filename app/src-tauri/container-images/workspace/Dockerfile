# Claude-Hydra's built-in Claude Code workspace. Runs as root so that the account's persistent
# home volume is /root (Claude Code login, settings and history live there).
FROM node:22-bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    bash ca-certificates curl git openssh-client tzdata locales procps less ripgrep \
    python3 python3-pip python3-venv build-essential \
    && rm -rf /var/lib/apt/lists/*

# The window languages a Hydra environment can choose (environment::LOCALES); Hydra sets LANG
# per terminal from the account's environment.
RUN sed -i -E 's/^# *((en_US|en_GB|zh_CN|zh_TW|ja_JP|ko_KR)\.UTF-8 UTF-8)/\1/' /etc/locale.gen \
    && locale-gen
ENV LANG=en_US.UTF-8

# Not pinned: Claude Code keeps itself up to date inside the container, the image only
# provides the first install.
RUN npm install -g @anthropic-ai/claude-code

WORKDIR /workspace
CMD ["sleep", "infinity"]
