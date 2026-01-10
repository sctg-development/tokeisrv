# MIT License (MIT)

# Copyright (c) 2025 Ronan Le Meillat for SCTG Development

# Permission is hereby granted, free of charge, to any person obtaining a copy
# of this software and associated documentation files (the "Software"), to deal
# in the Software without restriction, including without limitation the rights
# to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
# copies of the Software, and to permit persons to whom the Software is
# furnished to do so, subject to the following conditions:

# The above copyright notice and this permission notice shall be included in
# all copies or substantial portions of the Software.

# THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
# IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
# FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
# AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
# LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
# OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
# THE SOFTWARE.
FROM sctg/rust-photoacoustic-static-deps:latest AS deps

FROM alpine:3.23 AS builder
# Copy static dependencies from the deps image
COPY --from=deps /usr/local/lib/*.a /usr/local/lib/
COPY --from=deps /usr/local/include /usr/local/include
COPY --from=deps /usr/local/lib/pkgconfig    /usr/local/lib/pkgconfig
RUN apk update && apk add \
    clang g++ git patch cmake build-base \
    curl curl-dev curl-static\
    pkgconfig \
    musl-dev autoconf automake libtool \
    linux-headers expat-dev expat-static
# RUN apk add --no-cache build-base curl git pkgconfig openssl-dev libc-dev libstdc++ musl-dev musl-tools cmake clang g++
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y 
RUN echo $(dpkg --print-architecture)
RUN mkdir /build
RUN if [ "$(apk --print-arch)" = "armhf" ]; then \
    . /root/.cargo/env && rustup target add armv7-unknown-linux-musleabihf; \
    echo "armv7-unknown-linux-musleabihf" > /build/_target ; \
    fi
RUN if [ "$(apk --print-arch)" = "arm64" ]; then \
    . /root/.cargo/env && rustup target add aarch64-unknown-linux-musl; \
    echo "aarch64-unknown-linux-musl" > /build/_target ; \
    fi
RUN if [ "$(apk --print-arch)" = "x86_64" ]; then \
    . /root/.cargo/env && rustup target add x86_64-unknown-linux-musl; \
    echo "x86_64-unknown-linux-musl" > /build/_target ; \
    fi
COPY Cargo.toml /build/
COPY Cargo.lock /build/
COPY src /build/src
WORKDIR /build
RUN cd /build && . /root/.cargo/env && \
    TARGET=$(cat _target) && \
    export RUSTFLAGS="-C target-feature=+crt-static -C link-arg=-static" && \
    cargo build --release --target $TARGET && \
    strip target/$TARGET/release/tokei_rs
RUN cd /build && \
    TARGET=$(cat _target) && \
    ls -l target/$TARGET/release/ && \
    ldd target/$TARGET/release/tokeisrv ||  true && \
    echo "Build completed for target: $TARGET"
RUN cd /build && \
    TARGET=$(cat _target) && \
    cp target/$TARGET/release/tokei_rs /tokeisrv

FROM alpine:3.23 AS runtime 
RUN apk add --no-cache ca-certificates
COPY --from=builder /tokeisrv /usr/local/bin/tokeisrv
COPY docker-startup.sh /usr/local/bin/docker-startup.sh
RUN chmod +x /usr/local/bin/docker-startup.sh
ENTRYPOINT ["/usr/local/bin/docker-startup.sh"]
CMD []