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
        "RUN git clone --recursive https://github.com/cp2k/cp2k.git /opt/cp2k".to_string()
    } else {
        format!("RUN git clone --recursive -b support/v{version} https://github.com/cp2k/cp2k.git /opt/cp2k")
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
    let use_cmake = config.version == "master";
    let git_clone = if config.version == "master" {
        "RUN git clone --recursive https://github.com/cp2k/cp2k.git /opt/cp2k".to_string()
    } else {
        format!(
            "RUN git clone --recursive -b support/v{} https://github.com/cp2k/cp2k.git /opt/cp2k",
            config.version
        )
    };

    let build_step = if use_cmake {
        // Master branch uses CMake + Ninja (arch file no longer exists)
        r#"SHELL ["/bin/bash", "-c"]
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
    cp /opt/cp2k/tools/toolchain/scripts/tool_kit.sh /toolchain/scripts"#
            .to_string()
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
                    "ENV CUDA_PATH /usr/local/cuda\nENV LD_LIBRARY_PATH /usr/local/cuda/lib64\nENV CUDA_CACHE_DISABLE 1"
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
