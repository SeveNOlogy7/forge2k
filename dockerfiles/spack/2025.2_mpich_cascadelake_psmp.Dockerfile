#
# This Dockerfile is bundled with forge2k (generate_dockerfile fallback in src/build.rs; templates in src/templates/)
#
# Usage: docker build --shm-size=1g -f ./2025.2_mpich_cascadelake_psmp.Dockerfile -t cp2k/cp2k:2025.2_mpich_cascadelake_psmp .

# Stage 1: Build CP2K
ARG BASE_IMAGE="ubuntu:24.04"
FROM ${BASE_IMAGE} AS build_cp2k

# Install packages required to build the CP2K dependencies with Spack
RUN --mount=type=cache,target=/var/cache/apt,id=apt-2404,sharing=locked \
    rm -f /etc/apt/apt.conf.d/docker-clean && echo 'Binary::apt::APT::Keep-Downloaded-Packages "true";' > /etc/apt/apt.conf.d/keep-debs && \
    apt-get update -o Acquire::Retries=3 -qq && apt-get install -o Acquire::Retries=3 -qq --no-install-recommends \
    g++ gcc gfortran python3 \
    automake \
    bzip2 \
    ca-certificates \
    ccache \
    cmake \
    git \
    libncurses-dev \
    libssh-dev \
    libssl-dev \
    libtool-bin \
    lsb-release \
    make \
    ninja-build \
    openssh-client \
    patch \
    pkgconf \
    python3-dev \
    python3-pip \
    python3-venv \
    unzip \
    wget \
    xxd \
    xz-utils \
    zstd && rm -rf /var/lib/apt/lists/*

# Download CP2K
RUN --mount=type=cache,target=/opt/.cache/cp2k-src,id=cp2k-src-2025.2,sharing=locked bash -c 'set -e; C=/opt/.cache/cp2k-src; REF=support/v2025.2; git config --global http.version HTTP/1.1; if [ -d "$C/HEAD/.git" ]; then git -C "$C/HEAD" fetch origin "$REF" && git -C "$C/HEAD" reset --hard FETCH_HEAD && git -C "$C/HEAD" submodule update --init --recursive; else for i in 1 2 3; do git clone --recursive -b "$REF" https://github.com/cp2k/cp2k.git "$C/HEAD" && break; rm -rf "$C/HEAD"; [ $i -eq 3 ] && exit 1; sleep 10; done; fi && mkdir -p /opt/cp2k && cp -a "$C/HEAD/." /opt/cp2k/'

# Retrieve the number of available CPU cores
ARG NUM_PROCS
ENV NUM_PROCS=${NUM_PROCS:-16}

# Install Spack and Spack packages
WORKDIR /root/spack
ARG SPACK_VERSION
ENV SPACK_VERSION=${SPACK_VERSION:-1.2.2}
ARG SPACK_PACKAGES_VERSION
ENV SPACK_PACKAGES_VERSION=${SPACK_PACKAGES_VERSION:-2025.07.0}
ARG SPACK_REPO=https://github.com/spack/spack
ENV SPACK_ROOT=/opt/spack-${SPACK_VERSION}
ARG SPACK_PACKAGES_REPO=https://github.com/spack/spack-packages
ENV SPACK_PACKAGES_ROOT=/opt/spack-packages-${SPACK_PACKAGES_VERSION}
RUN mkdir -p ${SPACK_ROOT} && \
    wget -q ${SPACK_REPO}/archive/v${SPACK_VERSION}.tar.gz && \
    tar -xzf v${SPACK_VERSION}.tar.gz -C /opt && rm -f v${SPACK_VERSION}.tar.gz && \
    mkdir -p ${SPACK_PACKAGES_ROOT} && \
    wget -q ${SPACK_PACKAGES_REPO}/archive/v${SPACK_PACKAGES_VERSION}.tar.gz && \
    tar -xzf v${SPACK_PACKAGES_VERSION}.tar.gz -C /opt && rm -f v${SPACK_PACKAGES_VERSION}.tar.gz

ENV PATH="${SPACK_ROOT}/bin:${PATH}"

# Add Spack packages builtin repository
RUN spack repo add --scope site ${SPACK_PACKAGES_ROOT}/repos/spack_repo/builtin

# Find all compilers
RUN spack compiler find

# Find all external packages
RUN spack external find --all --not-buildable

# Register the local buildcache mirror and enable ccache for Spack builds.
# NOTE: Spack 1.2.2 `spack config add` has no --scope flag; the plain form
# writes /root/.spack/config.yaml (user scope, Wave 0 probe-verified).
RUN spack config add config:ccache:true && \
    spack config add "concretizer:targets:granularity:generic" && \
    spack mirror add --scope site --unsigned spack-bc file:///opt/spack-buildcache
ENV CCACHE_DIR=/opt/spack-ccache

# Copy Spack configuration and build recipes
ARG CP2K_VERSION
ENV CP2K_VERSION=${CP2K_VERSION:-psmp}
RUN cp -a /opt/cp2k/tools/spack/cp2k_dev_repo ${SPACK_PACKAGES_ROOT}/repos/spack_repo && \
    spack repo add --scope site ${SPACK_PACKAGES_ROOT}/repos/spack_repo/cp2k_dev_repo
RUN test -f /opt/cp2k/tools/spack/cp2k_deps_all_${CP2K_VERSION}.yaml && \
    cat /opt/cp2k/tools/spack/cp2k_deps_all_${CP2K_VERSION}.yaml && \
    spack env create myenv /opt/cp2k/tools/spack/cp2k_deps_all_${CP2K_VERSION}.yaml && \
    spack -e myenv repo list

# Install CP2K dependencies via Spack
RUN spack -e myenv concretize -f
ENV SPACK_ENV_VIEW="${SPACK_ROOT}/var/spack/environments/myenv/spack-env/view"
RUN --mount=type=cache,target=/opt/spack-1.2.2/var/spack/cache,id=spack-src,sharing=locked \
    --mount=type=cache,target=/opt/spack-buildcache,id=spack-buildcache,sharing=locked \
    --mount=type=cache,target=/opt/spack-ccache,id=spack-ccache,sharing=locked \
    spack -e myenv env depfile -o spack_makefile && \
    make -j${NUM_PROCS} --file=spack_makefile SPACK_COLOR=never --output-sync=recurse

# Export the Spack environment view for the CMake build
RUN cp -ar ${SPACK_ENV_VIEW}/bin ${SPACK_ENV_VIEW}/include ${SPACK_ENV_VIEW}/lib /opt/spack

# Push installed packages into the local buildcache mirror (non-fatal:
# segmented WARNs so cache-infrastructure trouble never breaks the build.
# A warm-mirror no-op push exits 1 in Spack 1.2.2 -- benign; judge push
# success by the "Pushed <n>/<m>" lines in the build transcript)
RUN --mount=type=cache,target=/opt/spack-buildcache,id=spack-buildcache,sharing=locked \
    spack -e myenv buildcache push --unsigned spack-bc || echo "WARN: buildcache push failed (non-fatal)"; \
    spack buildcache update-index spack-bc || echo "WARN: buildcache update-index failed (non-fatal)"

# Run CMake
# NOTE: the Spack env view keeps py-torch files as symlinks, so the find in
# cmake_cp2k.sh must follow symlinks to locate TorchConfig.cmake.
WORKDIR /opt/cp2k
RUN /bin/bash -c -o pipefail "find /opt/spack -name TorchConfig.cmake > /tmp/torch_probe.txt 2>/dev/null || true; echo ===TORCH PROBE===; cat /tmp/torch_probe.txt; sed -i 's|find /opt/spack/lib -name TorchConfig.cmake|find -L /opt/spack/lib -name TorchConfig.cmake|' ./cmake/cmake_cp2k.sh; source ./cmake/cmake_cp2k.sh spack_all ${CP2K_VERSION}"

# Compile CP2K for target CPU cascadelake
ARG LOG_LINES
ENV LOG_LINES=${LOG_LINES:-200}
WORKDIR /opt/cp2k/build
RUN /bin/bash -c -o pipefail " \
    echo -e '\nCompiling CP2K ... \c'; \
    if ninja --verbose &>ninja.log; then \
      echo -e 'done\n'; \
      echo -e 'Installing CP2K ... \c'; \
      if ninja --verbose install &>install.log; then \
        echo -e 'done\n'; \
      else \
        echo -e 'failed\n'; \
        tail -n ${LOG_LINES} install.log; \
        exit 1; \
      fi; \
      cat cmake.log ninja.log install.log | gzip >build_cp2k.log.gz; \
    else \
      echo -e 'failed\n'; \
      tail -n ${LOG_LINES} ninja.log; \
      cat cmake.log ninja.log | gzip >build_cp2k.log.gz; \
      exit 1; \
    fi"

# Store build arguments from base image needed in next stage
RUN echo "${CP2K_VERSION}" >/CP2K_VERSION

# Stage 2: Install CP2K
FROM ${BASE_IMAGE} AS runtime

# Install required packages
RUN --mount=type=cache,target=/var/cache/apt,id=apt-2404,sharing=locked \
    rm -f /etc/apt/apt.conf.d/docker-clean && echo 'Binary::apt::APT::Keep-Downloaded-Packages "true";' > /etc/apt/apt.conf.d/keep-debs && \
    apt-get update -o Acquire::Retries=3 -qq && apt-get install -o Acquire::Retries=3 -qq --no-install-recommends \
    g++ gcc gfortran python3 && rm -rf /var/lib/apt/lists/*

# Import build arguments from base image
COPY --from=build_cp2k /CP2K_VERSION /

# Install CP2K dependencies built with Spack
WORKDIR /opt
COPY --from=build_cp2k /opt/spack ./spack

# Install CP2K binaries
WORKDIR /opt/cp2k
COPY --from=build_cp2k /opt/cp2k/bin ./bin

# Install CP2K libraries
COPY --from=build_cp2k /opt/cp2k/lib ./lib

# Install CP2K database files
COPY --from=build_cp2k /opt/cp2k/share ./share

# Install CP2K regression tests
COPY --from=build_cp2k /opt/cp2k/tests ./tests
COPY --from=build_cp2k /opt/cp2k/src/grid/sample_tasks ./src/grid/sample_tasks

# Install CP2K/Quickstep CI benchmarks
COPY --from=build_cp2k /opt/cp2k/benchmarks/CI ./benchmarks/CI

# Import compressed build log file
COPY --from=build_cp2k /opt/cp2k/build/build_cp2k.log.gz /opt/cp2k/build/build_cp2k.log.gz

# Create links to CP2K binaries
WORKDIR /opt/cp2k/bin
RUN CP2K_VERSION=$(cat /CP2K_VERSION) && \
    ln -sf cp2k.${CP2K_VERSION} cp2k && \
    ln -sf cp2k.${CP2K_VERSION} cp2k.$(echo ${CP2K_VERSION} | sed "s/smp/opt/") && \
    ln -sf cp2k.${CP2K_VERSION} cp2k_shell && \
    ln -sf dumpdcd.${CP2K_VERSION} dumpdcd && \
    ln -sf graph.${CP2K_VERSION} graph && \
    ln -sf xyz2dcd.${CP2K_VERSION} xyz2dcd

# Update library search path
RUN echo "/opt/cp2k/lib\n/opt/spack/lib\n/opt/spack/lib/python3.12/site-packages/torch/lib" >/etc/ld.so.conf.d/cp2k.conf && ldconfig

# Create entrypoint script file
RUN printf "#!/bin/bash\n\
ulimit -c 0 -s unlimited\n\
export OMP_STACKSIZE=64M\n\
export PATH=/opt/cp2k/bin:/opt/spack/bin:\${PATH}\n\
\"\$@\"" \
>/opt/cp2k/bin/entrypoint.sh && chmod 755 /opt/cp2k/bin/entrypoint.sh

# Create shortcut for regression test
RUN printf "/opt/cp2k/tests/do_regtest.py \$* /opt/cp2k/bin $(cat /CP2K_VERSION)" \
>/opt/cp2k/bin/run_tests && chmod 755 /opt/cp2k/bin/run_tests

# Define entrypoint
WORKDIR /mnt
ENTRYPOINT ["/opt/cp2k/bin/entrypoint.sh"]
CMD ["cp2k", "--help"]

# EOF
