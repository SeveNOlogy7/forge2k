use super::BuildConfig;
use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;

// ============================================================
// Feature presets (`--features`)
// ============================================================

/// Feature presets known to the release-cmake arm. Single source of truth:
/// the CLI builds its `PossibleValuesParser` candidate list from this array
/// and the generator-layer [`validate_features`] validates against it, so
/// the two cannot drift (`t_feat_registry_single_source`).
pub(crate) const KNOWN_FEATURES: [&str; 1] = ["deepmd"];

/// Generator-layer (authoritative) feature validation: unknown values and
/// duplicates fail loud. The CLI rejects unknown values one layer earlier
/// via clap; this covers GUI/programmatic callers and the `deepmd,deepmd`
/// duplicate that clap cannot see.
pub(crate) fn validate_features(features: &[String]) -> Result<()> {
    for f in features {
        if !KNOWN_FEATURES.contains(&f.as_str()) {
            return Err(anyhow!(
                "unknown feature '{f}' in --features (known features: {})",
                KNOWN_FEATURES.join(", ")
            ));
        }
    }
    let mut seen: Vec<&str> = Vec::new();
    for f in features {
        if seen.contains(&f.as_str()) {
            return Err(anyhow!("duplicate feature '{f}' in --features"));
        }
        seen.push(f.as_str());
    }
    Ok(())
}

/// Whether `--features` is implemented for this (method, version) arm: only
/// the toolchain method's release-cmake arm (CP2K v2026.1+, not `master`,
/// whose synthesized step is a byte-frozen regression guard).
pub(crate) fn features_supported_arm(method: &str, version: &str) -> bool {
    method == "toolchain" && version != "master" && toolchain_uses_cmake(version)
}

/// Reject non-empty features on every arm outside the release-cmake one:
/// spack, native, the pre-2026 make arms and the frozen master arm stay
/// untouched rather than silently dropping the presets.
pub(crate) fn validate_feature_arm(method: &str, version: &str, features: &[String]) -> Result<()> {
    if features.is_empty() || features_supported_arm(method, version) {
        return Ok(());
    }
    Err(anyhow!(
        "feature preset(s) [{}] require method 'toolchain' with CP2K v2026.1+ (release-cmake build arm); got method='{}', version='{}'",
        features.join(", "),
        method,
        version
    ))
}

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
    // Authoritative feature validation (the CLI validates again before this
    // point for fail-fast; GUI/programmatic callers land here directly).
    validate_features(&config.features)?;
    validate_feature_arm(&config.method, &config.version, &config.features)?;
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
    // OpenBLAS workaround for the deepmd preset (docs on the constant):
    // `validate_feature_arm` has already rejected non-empty features on
    // every arm outside the toolchain release-cmake one, so the `deepmd`
    // flag alone is the gate. A feature-free render substitutes "" into the
    // template's own blank line, so all non-deepmd renders stay
    // byte-identical.
    let openblas_patch = if config.features.iter().any(|f| f == "deepmd") {
        OPENBLAS_SMALL_MATRIX_PATCH_STEP
    } else {
        ""
    };

    let build_step = if use_cmake {
        if config.version == "master" {
            // Master branch uses CMake + Ninja (arch file no longer exists)
            MASTER_CMAKE_BUILD_STEP.to_string()
        } else {
            // Releases from v2026.1 on ship no root Makefile/arch trees
            // either, so the same CMake arm runs with the release
            // base-flag additions and the optional CUDA triple.
            release_cmake_build_step(&config.cuda, &config.features)
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
            ("OPENBLAS_PATCH", openblas_patch),
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

/// Config-existence guards rendered by the `deepmd` feature preset: inserted
/// as `&& \` RUN segments between the dbcsr sed and the cmake call (§d of
/// the preset spec). They are drift detectors, not locators: the toolchain
/// `install/setup` exports CMAKE_PREFIX_PATH for both packages, so a missing
/// `*Config.cmake` means the `--install-all` set changed and the build must
/// fail here with an explicit message instead of deep inside
/// `find_package(... REQUIRED)`. `find -print -quit` returns 0 with empty
/// output on a miss, so `test -n` carries the verdict.
const FEATURE_CONFIG_GUARDS: &str = r#"    test -n "$(find "${TOOLCHAIN_DIR}/install" -maxdepth 6 ! -type l -name TorchConfig.cmake -print -quit)" || { echo "ERROR: TorchConfig.cmake not found under ${TOOLCHAIN_DIR}/install; the setup-exported CMAKE_PREFIX_PATH cannot resolve Torch (toolchain --install-all drift)" >&2; exit 1; } && \
    test -n "$(find "${TOOLCHAIN_DIR}/install" -maxdepth 6 ! -type l -name DeePMDConfig.cmake -print -quit)" || { echo "ERROR: DeePMDConfig.cmake not found under ${TOOLCHAIN_DIR}/install; find_package(DeePMD REQUIRED CONFIG) cannot resolve (toolchain --install-all drift)" >&2; exit 1; } && \
"#;

/// OpenBLAS workaround rendered by the `deepmd` feature preset: one RUN
/// between the clone (which has just put the toolchain script on disk) and
/// the toolchain install RUN (which has not run yet), rewriting the
/// `NUM_THREADS=128` make-variable lines of the toolchain's
/// `install_openblas.sh` to also pass `SMALL_MATRIX_OPT=0`.
///
/// Why (core + gdb + source level, probe run 39037606858; full write-up in
/// `.omz/.../results/deepmd-root-cause.md`): the toolchain's pinned
/// OpenBLAS 0.3.33 compiles the small-matrix batched-GEMM path on x86_64
/// (`Makefile.system:266-267` plain `SMALL_MATRIX_OPT = 1`, turned into
/// `-DSMALL_MATRIX_OPT` at `:291-293`). That path hands a table offset
/// (observed `0x428`) to its worker as a function pointer
/// (`interface/gemm_batch.c` `BLAS_SMALL_B0_OPT` dispatch ->
/// `driver/level3/gemm_batch_thread.c:53`), and the first small batched GEMM
/// of a CP2K deepmd NNP evaluation jumps to it and SIGSEGVs. The upstream
/// fix (OpenBLAS PR #5980) is develop-only — neither v0.3.33 nor v0.3.34
/// carries it — and every environment knob was proven inert (8 arms).
///
/// Mechanism: `SMALL_MATRIX_OPT=0` on the make command line is a GNU make
/// *command-line variable*, which overrides the makefile's plain assignment
/// and rides MAKEFLAGS into OpenBLAS's recursive sub-makes; no object is
/// then compiled with `-DSMALL_MATRIX_OPT`, which gates the whole offending
/// code (dispatch and kernels alike), so the build stays link-complete —
/// the same configuration as the architectures where the variable is never
/// set. The sed pattern carries the trailing space so the script's
/// explanatory `# NUM_THREADS=128: ...` comment is not touched, and only
/// the text before the line-continuation backslash changes, so the
/// continuation is preserved verbatim. `-i` rewrites in place; the `= 2`
/// count guard turns upstream script drift (variable renamed, call shape
/// changed) into a loud build failure instead of a silently unpatched
/// OpenBLAS.
const OPENBLAS_SMALL_MATRIX_PATCH_STEP: &str = r#"# deepmd preset: build OpenBLAS with SMALL_MATRIX_OPT=0. On x86_64 this
# variable is on by default (Makefile.system:266-267) and enables the
# small-matrix gemm_batch dispatch that hands a table offset (0x428) to
# driver/level3/gemm_batch_thread.c:53 as a function pointer -> SIGSEGV in
# deepmd runs. The upstream fix (OpenBLAS PR #5980) is develop-only; neither
# v0.3.33 nor v0.3.34 carries it (core+gdb backtrace, probe run 39037606858).
# The make command line wins over the makefile assignment and is inherited by
# OpenBLAS's recursive sub-makes (MAKEFLAGS), so no object gets the define.
RUN sed -i 's/NUM_THREADS=128 /NUM_THREADS=128 SMALL_MATRIX_OPT=0 /' /opt/cp2k/tools/toolchain/scripts/stage2/install_openblas.sh && \
    [ "$(grep -c 'NUM_THREADS=128 SMALL_MATRIX_OPT=0 ' /opt/cp2k/tools/toolchain/scripts/stage2/install_openblas.sh)" = 2 ]
"#;

/// CMake build step for releases >= 2026.1: the master CMake arm plus
/// `-DCP2K_USE_LIBTORCH=OFF` in the shared base flag set. v2026.2's
/// CMakeLists makes LIBTORCH follow CP2K_USE_EVERYTHING and the toolchain
/// `--install-all` installs libtorch, so it is explicitly OFF for both the
/// CPU and the GPU render (plan default 8) — no Torch_DIR runtime
/// resolution is introduced because no Torch dependency remains. The
/// `deepmd` feature preset (`--features deepmd`) flips DEEPMD and LIBTORCH
/// to ON in place — upstream binds the pair (make_cp2k.sh:305-311) — and
/// inserts the two config guards; with an empty feature list the rendered
/// bytes are unchanged. A `cuda` other than "none" additionally appends the
/// upstream CUDA triple (`-DCP2K_USE_ACCEL=CUDA -DCP2K_WITH_GPU=<GPU>
/// -DCMAKE_CUDA_ARCHITECTURES=<sm>`, make_cp2k.sh:898-901 with the
/// P100/V100 sm table at :434-449). The pre-2026 make arm and master's
/// step above are untouched by this builder.
fn release_cmake_build_step(cuda: &str, features: &[String]) -> String {
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
    // Wave ci-h2o (CI functional-test finding): pin the compiled-in CP2K
    // data dir to the image's shipped /opt/cp2k/data. Upstream's cache
    // default is the sentinel CP2K_DATA_DIR="default" (CMakeLists.txt:288);
    // src/CMakeLists.txt:1825-1826 resolves it at configure time to
    // ${CMAKE_INSTALL_FULL_DATAROOTDIR}/cp2k/data (and :2095 installs the
    // data tree there), i.e. the binary would bake an install-relative
    // share path instead of the path this Dockerfile itself ships via
    // `COPY --from=build /opt/cp2k/data/` (both arms). The pin also makes
    // the ci/cp2k-tests H2O suite's no-env contract the exact runtime path.
    // Feature preset (`--features`): deepmd flips the pair to ON. Upstream
    // binds the two — `--enable deepmd` implies `-DCP2K_USE_LIBTORCH=ON`
    // (make_cp2k.sh:305-311) — and both install trees already ship in the
    // toolchain `--install-all` set, so the pair moves together. With no
    // features both flags stay OFF: the rendered bytes stay unchanged.
    let deepmd = features.iter().any(|f| f == "deepmd");
    let (deepmd_flag, libtorch_flag) = if deepmd { ("ON", "ON") } else { ("OFF", "OFF") };
    let guards = if deepmd { FEATURE_CONFIG_GUARDS } else { "" };

    let mut step = format!(
        r#"SHELL ["/bin/bash", "-c"]
WORKDIR /opt/cp2k
ENV TOOLCHAIN_DIR=/opt/cp2k/tools/toolchain
RUN source ${{TOOLCHAIN_DIR}}/install/setup && \
    sed -i 's/set(PACKAGE_VERSION "[^"]*")/set(PACKAGE_VERSION "2.10.0")/' tools/toolchain/install/dbcsr-*/lib*/cmake/dbcsr/DBCSRConfigVersion.cmake && \
{guards}    cmake -GNinja \
    -DCMAKE_INSTALL_PREFIX=/opt/cp2k/install \
    -DCP2K_DATA_DIR=/opt/cp2k/data \
    -DCP2K_USE_EVERYTHING=ON \
    -DCP2K_USE_DLAF=OFF \
    -DCP2K_USE_PEXSI=OFF \
    -DCP2K_USE_DEEPMD={deepmd_flag} \
    -DCP2K_USE_LIBTORCH={libtorch_flag} \
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
