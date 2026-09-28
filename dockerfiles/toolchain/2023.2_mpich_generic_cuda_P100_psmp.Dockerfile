#
# This Dockerfile is bundled with forge2k (generate_dockerfile fallback in src/build.rs; templates in src/templates/)
#
# Usage: docker build -f ./2023.2_mpich_generic_cuda_P100_psmp.Dockerfile -t cp2k/cp2k:2023.2_mpich_generic_cuda_P100_psmp .

# Stage 1: build step
FROM nvidia/cuda:12.2.0-devel-ubuntu22.04 AS build

# Setup CUDA environment
ENV CUDA_PATH=/usr/local/cuda
ENV LD_LIBRARY_PATH=/usr/local/cuda/lib64

# Disable JIT cache as there seems to be an issue with file locking on overlayfs
# See also https://github.com/cp2k/cp2k/pull/2337
ENV CUDA_CACHE_DISABLE=1

# Install packages required for the CP2K toolchain build
RUN --mount=type=cache,target=/var/cache/apt,id=apt-2204,sharing=locked \
    rm -f /etc/apt/apt.conf.d/docker-clean && echo 'Binary::apt::APT::Keep-Downloaded-Packages "true";' > /etc/apt/apt.conf.d/keep-debs && \
    apt-get update -o Acquire::Retries=3 -qq && apt-get install -o Acquire::Retries=3 -qq --no-install-recommends \
    g++ gcc gfortran libmpich-dev mpich openssh-client python3 libtool libtool-bin \
    bzip2 ca-certificates git make patch pkg-config unzip wget zlib1g-dev

# Download CP2K
RUN --mount=type=cache,target=/opt/.cache/cp2k-src,id=cp2k-src-2023.2,sharing=locked bash -c 'set -e; C=/opt/.cache/cp2k-src; REF=support/v2023.2; git config --global http.version HTTP/1.1; if [ -d "$C/HEAD/.git" ]; then git -C "$C/HEAD" fetch origin "$REF" && git -C "$C/HEAD" reset --hard FETCH_HEAD && git -C "$C/HEAD" submodule update --init --recursive; else for i in 1 2 3; do git clone --recursive -b "$REF" https://github.com/cp2k/cp2k.git "$C/HEAD" && break; rm -rf "$C/HEAD"; [ $i -eq 3 ] && exit 1; sleep 10; done; fi && mkdir -p /opt/cp2k && cp -a "$C/HEAD/." /opt/cp2k/'

# Build CP2K toolchain for target CPU generic
WORKDIR /opt/cp2k/tools/toolchain
RUN --mount=type=cache,target=/opt/cp2k/tools/toolchain/build,id=toolchain-tarballs,sharing=locked \
    /bin/bash -c -o pipefail \
    "for i in 1 2 3; do ./install_cp2k_toolchain.sh -j 8 \
     --install-all \
     --enable-cuda=yes --gpu-ver=P100 --with-libtorch=no \
     --target-cpu=generic \
     --with-cusolvermp=no \
     --with-gcc=system \
     --with-mpich=system && break; \
     echo \"toolchain install failed (attempt \$i), cleaning build dir\"; \
     rm -rf /opt/cp2k/tools/toolchain/build/*; \
     [ \$i -eq 3 ] && exit 1; \
     sleep 10; done"

# Build CP2K for target CPU generic
WORKDIR /opt/cp2k
RUN /bin/bash -c -o pipefail \
    "cp ./tools/toolchain/install/arch/local_cuda.psmp ./arch/; \
     source ./tools/toolchain/install/setup; \
     make -j 8 ARCH=local_cuda VERSION=psmp"

# Collect components for installation and remove symbolic links
# NOTE: $-signs are escaped so that the outer /bin/sh passes them through to
# bash verbatim; otherwise dash expands $3/${libdir} to empty before bash runs.
RUN /bin/bash -c -o pipefail \
    "mkdir -p /toolchain/install /toolchain/scripts; \
     for libdir in \$(ldd ./exe/local_cuda/cp2k.psmp | \
                      grep /opt/cp2k/tools/toolchain/install | \
                      awk '{print \$3}' | cut -d/ -f7 | \
                      sort | uniq) setup; do \
        cp -ar /opt/cp2k/tools/toolchain/install/\${libdir} /toolchain/install; \
     done; \
     cp /opt/cp2k/tools/toolchain/scripts/tool_kit.sh /toolchain/scripts; \
     rm -f ./exe/local_cuda/cp2k.popt; \
     rm -f ./exe/local_cuda/cp2k_shell.psmp"

# Stage 2: install step
FROM nvidia/cuda:12.2.0-devel-ubuntu22.04 AS install

# Install required packages
RUN --mount=type=cache,target=/var/cache/apt,id=apt-2204,sharing=locked \
    rm -f /etc/apt/apt.conf.d/docker-clean && echo 'Binary::apt::APT::Keep-Downloaded-Packages "true";' > /etc/apt/apt.conf.d/keep-debs && \
    apt-get update -o Acquire::Retries=3 -qq && apt-get install -o Acquire::Retries=3 -qq --no-install-recommends \
    g++ gcc gfortran libmpich-dev mpich openssh-client python3 && rm -rf /var/lib/apt/lists/*

# Install CP2K binaries
COPY --from=build /opt/cp2k/exe/local_cuda/ /opt/cp2k/exe/local_cuda/

# Install CP2K regression tests
COPY --from=build /opt/cp2k/tests/ /opt/cp2k/tests/
COPY --from=build /opt/cp2k/tools/regtesting/ /opt/cp2k/tools/regtesting/
COPY --from=build /opt/cp2k/src/grid/sample_tasks/ /opt/cp2k/src/grid/sample_tasks/

# Install CP2K database files
COPY --from=build /opt/cp2k/data/ /opt/cp2k/data/

# Install shared libraries required by the CP2K binaries
COPY --from=build /toolchain/ /opt/cp2k/tools/toolchain/

# Create links to CP2K binaries
RUN /bin/bash -c -o pipefail \
    "for binary in cp2k dumpdcd graph xyz2dcd; do \
        ln -sf /opt/cp2k/exe/local_cuda/\${binary}.psmp \
               /usr/local/bin/\${binary}; \
     done; \
     ln -sf /opt/cp2k/exe/local_cuda/cp2k.psmp \
            /usr/local/bin/cp2k_shell; \
     ln -sf /opt/cp2k/exe/local_cuda/cp2k.psmp \
            /usr/local/bin/cp2k.popt"

# Create entrypoint script file
RUN printf "#!/bin/bash\n\
ulimit -c 0\n\
export PATH=/opt/cp2k/exe/local_cuda:\${PATH}\n\
source /opt/cp2k/tools/toolchain/install/setup\n\
exec \"\$@\"\n" > /entrypoint.sh && chmod a+x /entrypoint.sh

WORKDIR /work

ENTRYPOINT ["/entrypoint.sh"]
CMD ["cp2k", "--help"]
