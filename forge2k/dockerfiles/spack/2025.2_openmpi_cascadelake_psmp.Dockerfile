#
# This file was created by generate_docker_files.py
#
# Usage: docker build --shm-size=1g -f ./2025.2_openmpi_cascadelake_psmp.Dockerfile -t cp2k/cp2k:2025.2_openmpi_cascadelake_psmp .

# Stage 1: Build CP2K
ARG BASE_IMAGE="ubuntu:24.04"
FROM ${BASE_IMAGE} AS build_cp2k

# Install packages required to build the CP2K dependencies with Spack
RUN apt-get update -qq && apt-get install -qq --no-install-recommends \
    g++ gcc gfortran python3 \
    automake \
    bzip2 \
    ca-certificates \
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
RUN git clone --recursive -b support/v2025.2 https://github.com/cp2k/cp2k.git /opt/cp2k

# Retrieve the number of available CPU cores
ARG NUM_PROCS
ENV NUM_PROCS=${NUM_PROCS:-16}

# Install Spack and Spack packages
WORKDIR /root/spack
ARG SPACK_VERSION
ENV SPACK_VERSION=${SPACK_VERSION:-1.0.0}
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

# Copy Spack configuration and build recipes
ARG CP2K_VERSION
ENV CP2K_VERSION=${CP2K_VERSION:-psmp}
RUN cp -a /opt/cp2k/tools/spack/cp2k_dev_repo ${SPACK_PACKAGES_ROOT}/repos/spack_repo && \
    spack repo add --scope site ${SPACK_PACKAGES_ROOT}/repos/spack_repo/cp2k_dev_repo
RUN sed -e '/^\s*mpi:/i\      require: target="cascadelake"' -e 's/- mpich/- openmpi/' -e '/^\s*xpmem:/i\    openmpi:\n      require:\n        - +internal-hwloc' -e '/^\s*- "mpich@/ s/^ /#/' -e '/^#\s*- "openmpi@/ s/^#/ /' -i /opt/cp2k/tools/spack/cp2k_deps_all_${CP2K_VERSION}.yaml && \
    cat /opt/cp2k/tools/spack/cp2k_deps_all_${CP2K_VERSION}.yaml && \
    spack env create myenv /opt/cp2k/tools/spack/cp2k_deps_all_${CP2K_VERSION}.yaml && \
    spack -e myenv repo list

# Install CP2K dependencies via Spack
RUN spack -e myenv concretize -f
ENV SPACK_ENV_VIEW="${SPACK_ROOT}/var/spack/environments/myenv/spack-env/view"
RUN spack -e myenv env depfile -o spack_makefile && \
    make -j${NUM_PROCS} --file=spack_makefile SPACK_COLOR=never --output-sync=recurse

# Build CP2K
WORKDIR /opt/cp2k
RUN cp /opt/cp2k/tools/spack/spack_env_relocate.sh . && \
    cp /opt/spack-packages-${SPACK_PACKAGES_VERSION}/repos/spack_repo/cp2k_dev_repo/packages/cp2k/arch_name.patch . && \
    cp /opt/spack-packages-${SPACK_PACKAGES_VERSION}/repos/spack_repo/cp2k_dev_repo/packages/cp2k/check_gpu_arch_fix.patch . && \
    cp /opt/spack-packages-${SPACK_PACKAGES_VERSION}/repos/spack_repo/cp2k_dev_repo/packages/cp2k/rng_fixes.patch . && \
    cp /opt/spack-packages-${SPACK_PACKAGES_VERSION}/repos/spack_repo/cp2k_dev_repo/packages/cp2k/valgrind_fixes.patch . && \
    cp -r /opt/spack-packages-${SPACK_PACKAGES_VERSION}/repos/spack_repo/cp2k_dev_repo/packages/cp2k/spack_batch_relocate.sh . && \
    cp /opt/spack-packages-${SPACK_PACKAGES_VERSION}/repos/spack_repo/cp2k_dev_repo/packages/cp2k/relocate_cp2k_binary.sh . && \
    cp /opt/spack-packages-${SPACK_PACKAGES_VERSION}/repos/spack_repo/cp2k_dev_repo/packages/cp2k/relocate_spack_env.sh . && \
    cp /opt/spack-packages-${SPACK_PACKAGES_VERSION}/repos/spack_repo/cp2k_dev_repo/packages/cp2k/relocate_spack_env_tcl.sh . && \
    source ${SPACK_ROOT}/share/spack/setup-env.sh && \
    spack env activate myenv && \
    spack build-env -- spack install --source cp2k@${CP2K_VERSION}

# Post-install relocation to fix RPATHs
RUN source ${SPACK_ROOT}/share/spack/setup-env.sh && \
    spack env activate myenv && \
    bash ./relocate_spack_env.sh $(spack env activate --sh myenv | grep CP2K_SPACK_ENV | cut -d= -f2) /opt/cp2k/install

# Stage 2: runtime stage
FROM ${BASE_IMAGE} AS runtime

# Install runtime dependencies
RUN apt-get update -qq && apt-get install -qq --no-install-recommends \
    g++ gcc gfortran openssh-client python3 \
    ca-certificates \
    libgomp1 \
    libopenblas-dev \
    libmpich-dev \
    libpython3-dev \
    libstdc++-13-dev \
    openssh-client \
    python3 \
    python3-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy CP2K installation from build stage
COPY --from=build_cp2k /opt/cp2k/install /opt/cp2k
COPY --from=build_cp2k /opt/cp2k/exe /opt/cp2k/exe
COPY --from=build_cp2k /opt/cp2k/data /opt/cp2k/data
COPY --from=build_cp2k /opt/cp2k/tests /opt/cp2k/tests

# Create symbolic links for CP2K binaries
RUN ln -sf /opt/cp2k/exe/local/cp2k.psmp /usr/local/bin/cp2k && \
    ln -sf /opt/cp2k/exe/local/cp2k_shell.psmp /usr/local/bin/cp2k_shell

ENV PATH="/opt/cp2k/exe/local:${PATH}"
ENV LD_LIBRARY_PATH="/opt/cp2k/lib:${LD_LIBRARY_PATH}"

WORKDIR /work

ENTRYPOINT ["cp2k"]
