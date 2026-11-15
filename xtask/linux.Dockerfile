# What `cargo xtask run --linux` runs Cox in: Linux, with Xvfb for a screen.
FROM rust:1-bookworm
# Bookworm's Mesa 22 drew even vkcube black on Xvfb; backports' Mesa 25 draws it.
RUN echo 'deb http://deb.debian.org/debian bookworm-backports main' > /etc/apt/sources.list.d/backports.list \
 && apt-get update \
 && apt-get install -y --no-install-recommends \
    libvulkan-dev libwayland-dev libx11-xcb-dev libxkbcommon-dev libxkbcommon-x11-dev \
    cmake ninja-build pkg-config \
    xvfb xauth xdotool imagemagick fonts-dejavu-core libfontconfig1 \
    libnss3 libnspr4 libatk1.0-0 libatk-bridge2.0-0 libcups2 libdrm2 libgbm1 libasound2 \
    libxcomposite1 libxdamage1 libxrandr2 libxfixes3 libpango-1.0-0 libcairo2 libxshmfence1 \
 && apt-get install -y --no-install-recommends -t bookworm-backports mesa-vulkan-drivers \
 && rm -rf /var/lib/apt/lists/*
