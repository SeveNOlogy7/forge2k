use super::BuildConfig;
use anyhow::{Context, Result};
use std::path::PathBuf;

impl BuildConfig {
    /// Generate a Dockerfile from embedded templates
    pub fn generate_dockerfile(&self) -> Result<PathBuf> {
        let content = generate_dockerfile_content(self)?;
        let tmp_dir = std::env::temp_dir().join("forge2k");
        std::fs::create_dir_all(&tmp_dir).context("Failed to create temp dir for Dockerfile")?;

        let filename = format!("{}.Dockerfile", self.default_tag().replace(['/', ':'], "_"));
        let path = tmp_dir.join(&filename);
        std::fs::write(&path, &content).context("Failed to write generated Dockerfile")?;
        Ok(path)
    }
}

/// Substitute `{{TOKEN}}` placeholders in an include_str! Dockerfile
/// template. Byte-exact replacement; every token must be provided.
fn render_template(template: &str, params: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (key, value) in params {
        out = out.replace(&format!("{{{{{}}}}}", key), value);
    }
    out
}

/// Build the canon-git `RUN` block: cp2k is cloned into a per-version
/// BuildKit cache mount (id=cp2k-src-<version>) and `cp -a`-copied out to
/// /opt/cp2k, so a same-version rebuild refreshes via `git fetch` + `reset
/// --hard` instead of re-downloading the full tree (plan default 2/10).
/// Network hardening (plan FIX1, T-CFIX1): git is pinned to HTTP/1.1 to
/// sidestep proxy HTTP/2 framing drops, and the fresh-clone branch retries
/// three times (removing the partial tree between attempts) since `git
/// clone` cannot resume. Byte-identical to the canon-git line in the
/// bundled dockerfiles.
fn canon_git_clone_block(version: &str, git_ref: &str) -> String {
    format!(
        "RUN --mount=type=cache,target=/opt/.cache/cp2k-src,id=cp2k-src-{version},sharing=locked \
bash -c 'set -e; C=/opt/.cache/cp2k-src; REF={git_ref}; \
git config --global http.version HTTP/1.1; if [ -d \"$C/HEAD/.git\" ]; then \
git -C \"$C/HEAD\" fetch origin \"$REF\" && git -C \"$C/HEAD\" reset --hard FETCH_HEAD && \
git -C \"$C/HEAD\" submodule update --init --recursive; else for i in 1 2 3; do \
git clone --recursive -b \"$REF\" https://github.com/cp2k/cp2k.git \"$C/HEAD\" && break; \
rm -rf \"$C/HEAD\"; [ $i -eq 3 ] && exit 1; sleep 10; done; fi && mkdir -p /opt/cp2k && \
cp -a \"$C/HEAD/.\" /opt/cp2k/'"
    )
}

/// Generate Dockerfile content from build config
pub(crate) fn generate_dockerfile_content(config: &BuildConfig) -> Result<String> {
    match config.method.as_str() {
        "toolchain" => generate_toolchain_dockerfile(config),
        _ => generate_spack_dockerfile(config),
    }
}

fn generate_spack_dockerfile(config: &BuildConfig) -> Result<String> {
    let cpu = &config.cpu;
    let mpi = &config.mpi;
    let version = &config.version;
    let git_clone = if version == "master" {
        canon_git_clone_block(version, "master")
    } else {
        canon_git_clone_block(version, &format!("support/v{version}"))
    };

    // v2026.1+ renamed cp2k_deps_all_psmp.yaml -> cp2k_deps_psmp.yaml
    // and added require: target="" inside packages:all section
    let use_new_deps = version == "master"
        || version.starts_with("2026")
        || version.starts_with("2027")
        || version.starts_with("2028");
    let (deps_yaml, spack_ver, spack_pkgs_ver) = if use_new_deps {
        ("cp2k_deps_${CP2K_VERSION}.yaml", "1.1.1", "2026.03.0")
    } else {
        ("cp2k_deps_all_${CP2K_VERSION}.yaml", "1.0.0", "2025.07.0")
    };

    let mpi_setup = match mpi.as_str() {
        "openmpi" => {
            if use_new_deps {
                format!("RUN sed -i 's/target=\"\"/target=\"{cpu}\"/' /opt/cp2k/tools/spack/{deps} && \
                         sed -e 's/- mpich/- openmpi/' \
                         -e '/^\\s*xpmem:/i\\    openmpi:\\n      require:\\n        - +internal-hwloc' \
                         -e '/^\\s*- \"mpich@/ s/^ /#/' \
                         -e '/^#\\s*- \"openmpi@/ s/^#/ /' \
                         -i /opt/cp2k/tools/spack/{deps}",
                        cpu = cpu, deps = deps_yaml)
            } else {
                format!("RUN sed -e '/^\\s*mpi:/i\\      require: target=\"{cpu}\"' \
                         -e 's/- mpich/- openmpi/' \
                         -e '/^\\s*xpmem:/i\\    openmpi:\\n      require:\\n        - +internal-hwloc' \
                         -e '/^\\s*- \"mpich@/ s/^ /#/' \
                         -e '/^#\\s*- \"openmpi@/ s/^#/ /' \
                         -i /opt/cp2k/tools/spack/{deps}",
                        cpu = cpu, deps = deps_yaml)
            }
        }
        _ => {
            if use_new_deps {
                format!(
                    "RUN sed -i 's/target=\"\"/target=\"{cpu}\"/' /opt/cp2k/tools/spack/{deps}",
                    cpu = cpu,
                    deps = deps_yaml
                )
            } else {
                format!(
                    "RUN sed -e '/^\\s*mpi:/i\\      require: target=\"{cpu}\"' \
                         -i /opt/cp2k/tools/spack/{deps}",
                    cpu = cpu,
                    deps = deps_yaml
                )
            }
        }
    };

    Ok(render_template(
        include_str!("../templates/spack.Dockerfile.tmpl"),
        &[
            ("GIT_CLONE", &git_clone),
            ("SPACK_VER", spack_ver),
            ("SPACK_PKGS_VER", spack_pkgs_ver),
            ("MPI_SETUP", &mpi_setup),
            ("DEPS_YAML", deps_yaml),
        ],
    ))
}

fn generate_toolchain_dockerfile(config: &BuildConfig) -> Result<String> {
    let cuda_enabled = config.cuda != "none";
    let gpu_ver = if config.cuda == "none" {
        "no"
    } else {
        &config.cuda
    };
    let cuda_flag = if cuda_enabled { "yes" } else { "no" };
    let base_image = if cuda_enabled {
        "nvidia/cuda:12.2.0-devel-ubuntu22.04"
    } else {
        "ubuntu:22.04"
    };
    let arch_dir = if cuda_enabled { "local_cuda" } else { "local" };
    let cuda_extra = if cuda_enabled {
        format!("--gpu-ver={} --with-libtorch=no", gpu_ver)
    } else {
        String::new()
    };
    let use_cmake = toolchain_uses_cmake(&config.version);
    let git_clone = if config.version == "master" {
        canon_git_clone_block(&config.version, "master")
    } else {
        canon_git_clone_block(&config.version, &format!("support/v{}", config.version))
    };

    let build_step = if use_cmake {
        if config.version == "master" {
            // Master branch uses CMake + Ninja (arch file no longer exists)
            MASTER_CMAKE_BUILD_STEP.to_string()
        } else {
            // Releases from v2026.1 on ship no root Makefile/arch trees
            // either, so the same CMake arm runs with the release
            // base-flag additions and the optional CUDA triple.
            release_cmake_build_step(&config.cuda)
        }
    } else {
        // Tagged releases use old make approach with arch files
        format!(
            r#"WORKDIR /opt/cp2k
RUN cp ./tools/toolchain/install/arch/{arch}.psmp ./arch/ && \
    source ./tools/toolchain/install/setup && \
    make -j ${{NUM_PROCS:-8}} ARCH={arch} VERSION=psmp

RUN mkdir -p /toolchain/install /toolchain/scripts && \
    for libdir in $(ldd ./exe/{arch}/cp2k.psmp | \
                     grep /opt/cp2k/tools/toolchain/install | \
                     awk '{{print $3}}' | cut -d/ -f7 | \
                     sort | uniq) setup; do \
       cp -ar /opt/cp2k/tools/toolchain/install/${{libdir}} /toolchain/install; \
    done && \
    cp /opt/cp2k/tools/toolchain/scripts/tool_kit.sh /toolchain/scripts"#,
            arch = arch_dir
        )
    };

    let copy_step: String = if use_cmake {
        "COPY --from=build /opt/cp2k/install/ /opt/cp2k/install/\n\
         COPY --from=build /opt/cp2k/data/ /opt/cp2k/data/\n\
         COPY --from=build /toolchain/ /opt/cp2k/tools/toolchain/"
            .into()
    } else {
        format!(
            "COPY --from=build /opt/cp2k/exe/{arch}/ /opt/cp2k/exe/{arch}/\n\
                 COPY --from=build /opt/cp2k/data/ /opt/cp2k/data/\n\
                 COPY --from=build /toolchain/ /opt/cp2k/tools/toolchain/",
            arch = arch_dir
        )
    };

    let link_step: String = if use_cmake {
        r#"RUN ln -sf /opt/cp2k/install/bin/cp2k.psmp /usr/local/bin/cp2k && \
    ln -sf /opt/cp2k/install/bin/cp2k_shell.psmp /usr/local/bin/cp2k_shell && \
    ln -sf /opt/cp2k/install/bin/cp2k.popt /usr/local/bin/cp2k.popt
ENV PATH="/opt/cp2k/install/bin:${PATH}"
RUN printf '#!/bin/bash\n\
ulimit -c 0 -s unlimited\n\
export OMP_STACKSIZE=16M\n\
source /opt/cp2k/tools/toolchain/install/setup\n\
export LD_LIBRARY_PATH="/opt/cp2k/install/lib:${LD_LIBRARY_PATH}"\n\
exec "$@"' > /usr/local/bin/entrypoint.sh && chmod 755 /usr/local/bin/entrypoint.sh"#
            .to_string()
    } else {
        format!(
            r#"RUN for binary in cp2k dumpdcd graph xyz2dcd; do \
        ln -sf /opt/cp2k/exe/{arch}/${{binary}}.psmp /usr/local/bin/${{binary}}; \
    done && \
    ln -sf /opt/cp2k/exe/{arch}/cp2k.psmp /usr/local/bin/cp2k_shell
ENV PATH="/opt/cp2k/exe/{arch}:${{PATH}}"
RUN printf '#!/bin/bash\n\
ulimit -c 0 -s unlimited\n\
export OMP_STACKSIZE=16M\n\
source /opt/cp2k/tools/toolchain/install/setup\n\
export LD_LIBRARY_PATH="/opt/cp2k/install/lib:${{LD_LIBRARY_PATH}}"\n\
exec "$@"' > /usr/local/bin/entrypoint.sh && chmod 755 /usr/local/bin/entrypoint.sh"#,
            arch = arch_dir
        )
    };

    Ok(render_template(
        include_str!("../templates/toolchain.Dockerfile.tmpl"),
        &[
            ("BASE_IMAGE", base_image),
            (
                "CUDA_ENV",
                if cuda_enabled {
                    "ENV CUDA_PATH=/usr/local/cuda\nENV LD_LIBRARY_PATH=/usr/local/cuda/lib64\nENV CUDA_CACHE_DISABLE=1"
                } else {
                    ""
                },
            ),
            ("CUDA_FLAG", cuda_flag),
            ("CUDA_EXTRA", &cuda_extra),
            ("CPU", &config.cpu),
            ("GIT_CLONE", &git_clone),
            ("BUILD_STEP", &build_step),
            ("COPY_STEP", &copy_step),
            ("LINK_STEP", &link_step),
        ],
    ))
}

/// The master-branch CMake build step, kept verbatim from the generator
/// that predates the 2026.x arm: master renders byte-identical in every
/// combination (pinned regression guard — the plan's non-goal forbids
/// any change to master's existing synthesized behavior).
const MASTER_CMAKE_BUILD_STEP: &str = r#"SHELL ["/bin/bash", "-c"]
WORKDIR /opt/cp2k
ENV TOOLCHAIN_DIR=/opt/cp2k/tools/toolchain
RUN source ${TOOLCHAIN_DIR}/install/setup && \
    cmake -GNinja \
    -DCMAKE_INSTALL_PREFIX=/opt/cp2k/install \
    -DCP2K_USE_EVERYTHING=ON \
    -DCP2K_USE_DLAF=OFF \
    -DCP2K_USE_PEXSI=OFF \
    -DCP2K_USE_DEEPMD=OFF \
    -DCMAKE_INTERPROCEDURAL_OPTIMIZATION=OFF \
    -DCMAKE_C_FLAGS="-fno-lto" \
    -DCMAKE_CXX_FLAGS="-fno-lto" \
    -DCMAKE_Fortran_FLAGS="-fno-lto" \
    -DCMAKE_EXE_LINKER_FLAGS="-fno-lto" \
    -Werror=dev \
    -B build -S . && \
    ninja -C build -j ${NUM_PROCS:-8} && \
    cmake --install build --prefix /opt/cp2k/install

RUN mkdir -p /toolchain/install /toolchain/scripts && \
    for d in /opt/cp2k/tools/toolchain/install/*/; do \
        libdir=$(basename "$d"); \
        cp -a "$d" /toolchain/install/; \
    done && \
    cp /opt/cp2k/tools/toolchain/install/setup /toolchain/install/ && \
    cp /opt/cp2k/tools/toolchain/scripts/tool_kit.sh /toolchain/scripts"#;

/// CUDA compute capability for a GPU model, per the upstream make_cp2k.sh
/// GPU table (P100 -> sm_60, V100 -> sm_70, :434-449). Unknown models fall
/// back to the P100 (sm_60) vector — the CI-tested default; the CLI value
/// parser admits only none/P100/V100.
fn cuda_sm_code(gpu: &str) -> &'static str {
    match gpu {
        "V100" => "70",
        _ => "60",
    }
}

/// CMake build step for releases >= 2026.1: the master CMake arm plus
/// `-DCP2K_USE_LIBTORCH=OFF` in the shared base flag set. v2026.2's
/// CMakeLists makes LIBTORCH follow CP2K_USE_EVERYTHING and the toolchain
/// `--install-all` installs libtorch, so it is explicitly OFF for both the
/// CPU and the GPU render (plan default 8) — no Torch_DIR runtime
/// resolution is introduced because no Torch dependency remains. A
/// `cuda` other than "none" additionally appends the upstream CUDA triple
/// (`-DCP2K_USE_ACCEL=CUDA -DCP2K_WITH_GPU=<GPU>
/// -DCMAKE_CUDA_ARCHITECTURES=<sm>`, make_cp2k.sh:898-901 with the
/// P100/V100 sm table at :434-449). The pre-2026 make arm and master's
/// step above are untouched by this builder.
fn release_cmake_build_step(cuda: &str) -> String {
    // The CUDA triple slots between the shared base flags and the
    // IPO/LTO block; CPU renders (cuda == "none") leave it empty.
    let cuda_flags = if cuda == "none" {
        String::new()
    } else {
        format!(
            "    -DCP2K_USE_ACCEL=CUDA \\\n    -DCP2K_WITH_GPU={} \\\n    -DCMAKE_CUDA_ARCHITECTURES={} \\\n",
            cuda,
            cuda_sm_code(cuda)
        )
    };
    // Wave 3 (probe run 38393254283): the dbcsr tarball build's `git describe` climbs into the outer cp2k clone (HEAD on tag v2026.2), so the installed DBCSRConfigVersion.cmake reports the malformed version `2026.2.v2026.2v2026.2` that SameMajorVersion rejects for find_package(DBCSR 2.8) (upstream CI's COPY ships no .git); cwd is the cp2k root and a glob miss must fail the build loudly, hence no `|| true`.
    // Wave 3 (e2e runs 38404980161 CPU / 38405057080 GPU): both configure runs died at CP2K v2026.2 CMakeLists.txt:943 `find_package(openPMD 0.16.1 REQUIRED)` (CP2K_USE_OPENPMD, :174-175); the toolchain `--install-all` set ships no openPMD, so OPENPMD is the sole additional configure-stage OFF beyond the base four.
    let mut step = String::from(
        r#"SHELL ["/bin/bash", "-c"]
WORKDIR /opt/cp2k
ENV TOOLCHAIN_DIR=/opt/cp2k/tools/toolchain
RUN source ${TOOLCHAIN_DIR}/install/setup && \
    sed -i 's/set(PACKAGE_VERSION "[^"]*")/set(PACKAGE_VERSION "2.10.0")/' tools/toolchain/install/dbcsr-*/lib*/cmake/dbcsr/DBCSRConfigVersion.cmake && \
    cmake -GNinja \
    -DCMAKE_INSTALL_PREFIX=/opt/cp2k/install \
    -DCP2K_USE_EVERYTHING=ON \
    -DCP2K_USE_DLAF=OFF \
    -DCP2K_USE_PEXSI=OFF \
    -DCP2K_USE_DEEPMD=OFF \
    -DCP2K_USE_LIBTORCH=OFF \
    -DCP2K_USE_OPENPMD=OFF \
"#,
    );
    step.push_str(&cuda_flags);
    step.push_str(
        r#"    -DCMAKE_INTERPROCEDURAL_OPTIMIZATION=OFF \
    -DCMAKE_C_FLAGS="-fno-lto" \
    -DCMAKE_CXX_FLAGS="-fno-lto" \
    -DCMAKE_Fortran_FLAGS="-fno-lto" \
    -DCMAKE_EXE_LINKER_FLAGS="-fno-lto" \
    -Werror=dev \
    -B build -S . && \
    ninja -C build -j ${NUM_PROCS:-8} && \
    cmake --install build --prefix /opt/cp2k/install

RUN mkdir -p /toolchain/install /toolchain/scripts && \
    for d in /opt/cp2k/tools/toolchain/install/*/; do \
        libdir=$(basename "$d"); \
        cp -a "$d" /toolchain/install/; \
    done && \
    cp /opt/cp2k/tools/toolchain/install/setup /toolchain/install/ && \
    cp /opt/cp2k/tools/toolchain/scripts/tool_kit.sh /toolchain/scripts"#,
    );
    step
}

/// Whether the toolchain build path must use the CMake arm (`true`) or
/// the legacy make+arch-file arm (`false`): `master` and every release
/// from v2026.1 on deleted the root Makefile/arch trees upstream (v2025.2
/// is the last release still shipping them), so the boundary is
/// `master || version >= "2026.1"`. String comparison; plan risk 10 pins
/// the boundary semantics by unit test.
pub(crate) fn toolchain_uses_cmake(version: &str) -> bool {
    version == "master" || version >= "2026.1"
}
