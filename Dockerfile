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

FROM ubuntu:resolute AS builder
RUN apt-get update && apt-get install -y build-essential curl git pkg-config libssl-dev libc-dev libstdc++-13-dev libgcc-13-dev \
    zip git libcurl4-openssl-dev musl-dev musl-tools cmake libclang-dev g++
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y 
RUN echo $(dpkg --print-architecture)
RUN mkdir /build
RUN if [ "$(dpkg --print-architecture)" = "armhf" ]; then \
    . /root/.cargo/env && rustup target add armv7-unknown-linux-musleabihf; \
    ln -svf /usr/bin/ar /usr/bin/arm-linux-musleabihf-ar; \
    ln -svf /usr/bin/strip /usr/bin/arm-linux-musleabihf-strip; \
    ln -svf /usr/bin/ranlib /usr/bin/arm-linux-musleabihf-ranlib; \
    echo "armv7-unknown-linux-musleabihf" > /build/_target ; \
    fi
RUN if [ "$(dpkg --print-architecture)" = "arm64" ]; then \
    . /root/.cargo/env && rustup target add aarch64-unknown-linux-musl; \
    ln -svf /usr/bin/ar /usr/bin/aarch64-linux-musl-ar; \
    ln -svf /usr/bin/strip /usr/bin/aarch64-linux-musl-strip; \
    ln -svf /usr/bin/ranlib /usr/bin/aarch64-linux-musl-ranlib; \
    echo "aarch64-unknown-linux-musl" > /build/_target ; \
    fi
RUN if [ "$(dpkg --print-architecture)" = "amd64" ]; then \
    . /root/.cargo/env && rustup target add x86_64-unknown-linux-musl; \
    echo "x86_64-unknown-linux-musl" > /build/_target ; \
    fi
COPY Cargo.toml /build/
COPY Cargo.lock /build/
COPY src /build/src
WORKDIR /build
RUN cd /build && . /root/.cargo/env && \
    TARGET=$(cat _target) && \
    cargo build --release --target $TARGET
RUN cd /build && \
    TARGET=$(cat _target) && \
    cp target/$TARGET/release/tokei_rs /tokeisrv

FROM ubuntu:resolute AS runtime 
RUN apt-get update && apt-get install -y libssl ca-certificates git
COPY --from=builder /tokeisrv /usr/local/bin/tokeisrv
COPY docker-startup.sh /usr/local/bin/docker-startup.sh
RUN chmod +x /usr/local/bin/docker-startup.sh
ENTRYPOINT ["/usr/local/bin/docker-startup.sh"]
CMD []