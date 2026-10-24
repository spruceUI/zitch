FROM ubuntu:20.04

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    build-essential \
    cmake \
    gcc-aarch64-linux-gnu \
    g++-aarch64-linux-gnu \
    pkg-config \
    git \
    ca-certificates \
    curl \
    unzip \
    zip \
    python3 \
    && rm -rf /var/lib/apt/lists/*

# Focal's SDL2 is 2.0.10, older than the sdl2 crate links against. This one
# is only linked to; devices load their own libSDL2-2.0.so.0.
ARG SDL_VERSION=2.28.5
RUN curl -sSfL https://github.com/libsdl-org/SDL/releases/download/release-${SDL_VERSION}/SDL2-${SDL_VERSION}.tar.gz | tar xz -C /tmp && \
    cmake -S /tmp/SDL2-${SDL_VERSION} -B /tmp/sdl-build -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_SYSTEM_NAME=Linux -DCMAKE_SYSTEM_PROCESSOR=aarch64 \
        -DCMAKE_C_COMPILER=aarch64-linux-gnu-gcc -DCMAKE_INSTALL_PREFIX=/opt/sdl2 \
        -DSDL_STATIC=OFF -DSDL_TEST=OFF -DSDL_X11=OFF -DSDL_WAYLAND=OFF -DSDL_KMSDRM=OFF \
        -DSDL_ALSA=OFF -DSDL_PULSEAUDIO=OFF -DSDL_PIPEWIRE=OFF -DSDL_JACK=OFF -DSDL_SNDIO=OFF \
        -DSDL_LIBUDEV=OFF -DSDL_DBUS=OFF -DSDL_IBUS=OFF && \
    cmake --build /tmp/sdl-build -j"$(nproc)" && \
    cmake --install /tmp/sdl-build && \
    rm -rf /tmp/SDL2-${SDL_VERSION} /tmp/sdl-build

ENV RUSTUP_HOME=/usr/local/rustup PATH=/usr/local/cargo/bin:$PATH
RUN curl -sSf https://sh.rustup.rs | CARGO_HOME=/usr/local/cargo sh -s -- -y \
        --profile minimal --default-toolchain stable --target aarch64-unknown-linux-gnu

COPY build.sh /build.sh
RUN chmod +x /build.sh

WORKDIR /src
ENTRYPOINT ["/build.sh"]
