// ============================================================
// Tests (Wave 4: T-013 pure-function unit tests,
// T-014 failure-propagation & cancellation behavior tests)
// ============================================================

use super::*;

// --------------------------------------------------------
// Shared helpers
// --------------------------------------------------------

fn base_config(
    method: &str,
    version: &str,
    mpi: &str,
    cpu: &str,
    cuda: &str,
    variant: &str,
) -> BuildConfig {
    BuildConfig {
        method: method.to_string(),
        version: version.to_string(),
        mpi: mpi.to_string(),
        cpu: cpu.to_string(),
        cuda: cuda.to_string(),
        variant: variant.to_string(),
        jobs: 8,
        tag: String::new(),
        no_cache: false,
        shm_size: "1g".to_string(),
        dockerfile: None,
    }
}

/// Serializes every test that spawns child processes or mutates PATH.
/// std::env mutations are process-global; without this lock a
/// PATH-clearing test would break a concurrently-spawning sibling.
static PROCESS_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Restores PATH on drop, even if the assertion panics.
struct PathGuard {
    old: String,
}

impl PathGuard {
    fn set(new_path: &str) -> PathGuard {
        let old = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", new_path);
        PathGuard { old }
    }
}

impl Drop for PathGuard {
    fn drop(&mut self) {
        std::env::set_var("PATH", &self.old);
    }
}

/// A unique temp directory removed on drop (best effort).
struct TempGuard(PathBuf);

impl TempGuard {
    fn new(tag: &str) -> TempGuard {
        let dir = std::env::temp_dir().join(format!("forge2k_w4_{}_{}", tag, std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        TempGuard(dir)
    }
}

impl Drop for TempGuard {
    fn drop(&mut self) {
        // On Windows, handles from a just-killed child are released
        // asynchronously; retry briefly so no temp dir is left behind.
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if std::fs::remove_dir_all(&self.0).is_ok() || std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// Install a fake `docker` executable (a copy of the shell) in `dir`
/// so `Command::new("docker")` resolves without a real daemon.
fn install_fake_docker(dir: &Path) {
    #[cfg(target_os = "windows")]
    {
        let src =
            std::env::var("ComSpec").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".into());
        std::fs::copy(&src, dir.join("docker.exe")).expect("copy shell as fake docker.exe");
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::fs::copy("/bin/sh", dir.join("docker")).expect("copy shell as fake docker");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.join("docker"), std::fs::Permissions::from_mode(0o755))
            .expect("chmod fake docker");
    }
}

/// A short-lived child process used to drive the cancel flag from a
/// background thread (event-driven, no wall-clock assertion).
fn spawn_short_sleep_subprocess() -> std::process::Child {
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/D", "/C", "ping -n 1 127.0.0.1 > nul"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn short sleep subprocess (cmd/ping)")
    }
    #[cfg(not(target_os = "windows"))]
    {
        Command::new("sleep")
            .arg("1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn short sleep subprocess (sleep)")
    }
}

/// Long-running scripted command: sleeps ~30s, then writes a sentinel
/// file. If the process is killed before finishing, the sentinel is
/// never written — an event-based (not timing-based) kill proof.
fn long_sleep_with_sentinel(sentinel: &Path) -> (&'static str, Vec<String>) {
    #[cfg(target_os = "windows")]
    {
        (
            "cmd",
            vec![
                "/D".to_string(),
                "/C".to_string(),
                format!(
                    "ping -n 30 127.0.0.1 > nul & echo done > \"{}\"",
                    sentinel.display()
                ),
            ],
        )
    }
    #[cfg(not(target_os = "windows"))]
    {
        (
            "sh",
            vec![
                "-c".to_string(),
                format!("sleep 30; echo done > '{}'", sentinel.display()),
            ],
        )
    }
}

fn drain_rx(rx: mpsc::Receiver<LogLine>) {
    for _ in rx.try_iter() {}
}

// --------------------------------------------------------
// T-013: pure-function unit tests
// --------------------------------------------------------

#[test]
fn t13_dockerfile_name_spack_combinations() {
    let c = base_config("spack", "2025.2", "mpich", "x86_64", "none", "psmp");
    assert_eq!(
        c.dockerfile_name(),
        "spack/2025.2_mpich_x86_64_psmp.Dockerfile",
        "T-013: spack name must be version_mpi_cpu_variant.Dockerfile (no cuda segment)"
    );
    let c = base_config("spack", "2026.1", "openmpi", "generic", "none", "ssmp");
    assert_eq!(
        c.dockerfile_name(),
        "spack/2026.1_openmpi_generic_ssmp.Dockerfile",
        "T-013: spack openmpi/generic/ssmp combination must assemble in fixed order"
    );
    // cuda is not part of the spack filename scheme
    let c = base_config("spack", "2025.2", "mpich", "cascadelake", "P100", "psmp");
    assert_eq!(
        c.dockerfile_name(),
        "spack/2025.2_mpich_cascadelake_psmp.Dockerfile",
        "T-013: spack filenames must not embed a cuda segment"
    );
}

#[test]
fn t13_dockerfile_name_toolchain_combinations() {
    let c = base_config("toolchain", "2023.2", "mpich", "generic", "P100", "psmp");
    assert_eq!(
        c.dockerfile_name(),
        "toolchain/2023.2_mpich_generic_cuda_P100_psmp.Dockerfile",
        "T-013: toolchain + cuda must insert '_cuda_<GPU>' before the variant"
    );
    let c = base_config("toolchain", "2025.2", "mpich", "x86_64", "none", "sdbg");
    assert_eq!(
        c.dockerfile_name(),
        "toolchain/2025.2_mpich_x86_64_sdbg.Dockerfile",
        "T-013: toolchain without cuda must omit the cuda segment entirely"
    );
}

#[test]
fn t13_dockerfile_search_paths_fallback_order() {
    let paths = dockerfile_search_paths();
    assert_eq!(
        paths.len(),
        3,
        "T-013: exactly three search locations expected"
    );
    let exe_dir = std::env::current_exe()
        .expect("current_exe")
        .parent()
        .expect("exe parent")
        .to_path_buf();
    assert_eq!(
        paths[0],
        exe_dir.join("dockerfiles"),
        "T-013: first fallback must be dockerfiles/ next to the executable"
    );
    assert_eq!(
        paths[1],
        PathBuf::from("dockerfiles"),
        "T-013: second fallback must be ./dockerfiles"
    );
    assert_eq!(
        paths[2],
        PathBuf::from("."),
        "T-013: last fallback must be the working directory itself"
    );
}

#[test]
fn t13_default_tag_versioned_release() {
    let c = base_config("spack", "2025.2", "mpich", "x86_64", "none", "psmp");
    assert_eq!(
        c.default_tag(),
        "cp2k/cp2k:2025.2_mpich_x86_64_psmp",
        "T-013: tag must be cp2k/cp2k:<ver>_<mpi>_<cpu>_<variant>"
    );
    let c = base_config(
        "toolchain",
        "2026.1",
        "openmpi",
        "cascadelake",
        "V100",
        "ssmp",
    );
    assert_eq!(
        c.default_tag(),
        "cp2k/cp2k:2026.1_openmpi_cascadelake_cuda_V100_ssmp",
        "T-013: cuda suffix '_cuda_<GPU>' must sit between cpu and variant"
    );
}

#[test]
fn t13_default_tag_master_gets_yyyymmdd_datestamp() {
    let before = Local::now().format("%Y%m%d").to_string();
    let c = base_config("toolchain", "master", "mpich", "generic", "none", "psmp");
    let tag = c.default_tag();
    let after = Local::now().format("%Y%m%d").to_string();
    assert!(
        tag.starts_with("cp2k/cp2k:master_"),
        "T-013: master builds must be tagged 'master_<date>', got: {}",
        tag
    );
    let date_part = tag
        .trim_start_matches("cp2k/cp2k:master_")
        .split('_')
        .next()
        .expect("date segment")
        .to_string();
    assert_eq!(
        date_part.len(),
        8,
        "T-013: datestamp must be YYYYMMDD (8 digits), got: {}",
        date_part
    );
    assert!(
        date_part.chars().all(|ch| ch.is_ascii_digit()),
        "T-013: datestamp must be all digits, got: {}",
        date_part
    );
    assert!(
        date_part == before || date_part == after,
        "T-013: datestamp {} must match the call-time date ({}..{})",
        date_part,
        before,
        after
    );
}

#[test]
fn t13_default_tag_explicit_tag_overrides_default() {
    let mut c = base_config("spack", "2025.2", "mpich", "x86_64", "none", "psmp");
    c.tag = "myrepo/cp2k-custom:v9".to_string();
    assert_eq!(
        c.default_tag(),
        "myrepo/cp2k-custom:v9",
        "T-013: an explicit tag must be returned verbatim"
    );
}

#[test]
fn t13_mirrors_table_complete() {
    let mirrors = crate::mirrors::known_mirrors();
    assert!(
        mirrors.len() >= 5,
        "T-013: mirrors table must keep at least 5 entries, got {}",
        mirrors.len()
    );
    let mut urls: Vec<&str> = Vec::new();
    for m in mirrors {
        assert!(
            !m.name.trim().is_empty(),
            "T-013: mirror name must be non-empty"
        );
        assert!(
            !m.desc.trim().is_empty(),
            "T-013: mirror desc must be non-empty"
        );
        assert!(
            m.url.starts_with("https://"),
            "T-013: mirror url must be https, got: {}",
            m.url
        );
        assert!(
            !m.url.ends_with('/'),
            "T-013: mirror url must not end with '/'"
        );
        urls.push(m.url);
    }
    let n = urls.len();
    urls.sort();
    urls.dedup();
    assert_eq!(urls.len(), n, "T-013: mirror urls must be unique");
    // single-source contract: registry_mirror_urls() must yield exactly
    // the table's urls in order
    let iter_urls: Vec<&str> = crate::mirrors::registry_mirror_urls().collect();
    let table_urls: Vec<&str> = mirrors.iter().map(|m| m.url).collect();
    assert_eq!(
        iter_urls, table_urls,
        "T-013: registry_mirror_urls must be derived from known_mirrors in order"
    );
}

#[test]
fn t13_spack_dockerfile_synthesized_key_fields() {
    // pre-2026 release: old deps yaml + spack 1.0.0
    let c = base_config("spack", "2025.2", "mpich", "x86_64", "none", "psmp");
    let content = generate_dockerfile_content(&c).expect("render spack template");
    assert!(
        content.contains("FROM ubuntu:24.04"),
        "T-013: spack image must start FROM ubuntu:24.04"
    );
    assert!(
        content.contains("id=cp2k-src-2025.2,sharing=locked")
            && content.contains("REF=support/v2025.2;"),
        "T-013: release build must seed cp2k-src-2025.2 cache and pin REF=support/v2025.2"
    );
    assert!(
        content.contains("ARG NUM_PROCS"),
        "T-013: NUM_PROCS build-arg must be present"
    );
    assert!(
        content.contains("cp2k_deps_all_${CP2K_VERSION}.yaml"),
        "T-013: 2025.x must use the legacy cp2k_deps_all deps yaml"
    );
    assert!(
        content.contains("SPACK_VERSION=1.0.0"),
        "T-013: 2025.x must pin spack 1.0.0"
    );
    assert!(
        content.contains(
            "spack mirror add --scope site --unsigned spack-bc file:///opt/spack-buildcache"
        ),
        "T-013: synthesized spack must register the unsigned local buildcache mirror spack-bc"
    );
    assert!(
        !content.contains("{{"),
        "T-013: no template placeholder may survive rendering, found '{{' in:\n{}",
        content
    );
    // 2026.1: new deps yaml + spack 1.1.1
    let c = base_config("spack", "2026.1", "openmpi", "cascadelake", "none", "psmp");
    let content = generate_dockerfile_content(&c).expect("render spack template");
    assert!(
        content.contains("cp2k_deps_${CP2K_VERSION}.yaml"),
        "T-013: 2026.1 must use the renamed cp2k_deps deps yaml"
    );
    assert!(
        content.contains("SPACK_VERSION=1.1.1"),
        "T-013: 2026.1 must pin spack 1.1.1"
    );
    assert!(
        content.contains("openmpi"),
        "T-013: openmpi config must contain an openmpi sed/replace step"
    );
    assert!(
        !content.contains("{{"),
        "T-013: no template placeholder may survive rendering (2026.1 openmpi)"
    );
}

#[test]
fn t13_toolchain_dockerfile_synthesized_key_fields() {
    let c = base_config("toolchain", "2023.2", "mpich", "generic", "none", "psmp");
    let content = generate_dockerfile_content(&c).expect("render toolchain template");
    assert!(
        content.contains("FROM ubuntu:22.04"),
        "T-013: non-cuda toolchain must be FROM ubuntu:22.04"
    );
    assert!(
        content.contains("--enable-cuda=no"),
        "T-013: cuda disabled must render --enable-cuda=no"
    );
    assert!(
        content.contains("--target-cpu=generic"),
        "T-013: CPU placeholder must be substituted with the configured target"
    );
    assert!(
        content.contains("ENTRYPOINT [\"/usr/local/bin/entrypoint.sh\"]"),
        "T-013: toolchain image must keep the entrypoint"
    );
    assert!(
        !content.contains("{{"),
        "T-013: no placeholder may survive (toolchain no-cuda)"
    );

    let c = base_config("toolchain", "2023.2", "mpich", "generic", "P100", "psmp");
    let content = generate_dockerfile_content(&c).expect("render toolchain template");
    assert!(
        content.contains("FROM nvidia/cuda:12.2.0-devel-ubuntu22.04"),
        "T-013: cuda toolchain must be FROM the nvidia/cuda devel image"
    );
    assert!(
        content.contains("--enable-cuda=yes"),
        "T-013: cuda enabled must render --enable-cuda=yes"
    );
    assert!(
        content.contains("--gpu-ver=P100"),
        "T-013: cuda build must pass --gpu-ver=<GPU>"
    );
    assert!(
        !content.contains("{{"),
        "T-013: no placeholder may survive (toolchain cuda)"
    );

    // master: cmake/ninja build path
    let c = base_config("toolchain", "master", "mpich", "generic", "none", "psmp");
    let content = generate_dockerfile_content(&c).expect("render toolchain template");
    assert!(
        content.contains("id=cp2k-src-master,sharing=locked") && content.contains("REF=master;"),
        "T-013: master build must seed cp2k-src-master cache and pin REF=master"
    );
    assert!(
        content.contains("cmake -GNinja"),
        "T-013: master must use the cmake+ninja build step"
    );
    assert!(
        !content.contains("{{"),
        "T-013: no placeholder may survive (toolchain master)"
    );
}

// --------------------------------------------------------
// Phase 3 Wave 4 (T-C13/T-C14): cache-mount key-field
// assertions for the bundled and synthesized dockerfiles
// --------------------------------------------------------
//
// Pure-text assertions over the six bundled dockerfiles and the
// generated content — no Docker daemon needed, CI-safe. Each
// assertion pins a structural mount element: target/id/sharing as
// ONE mount spec, the canon-git id/REF/seed-copy as ONE physical
// line (same style as the synthesized tests above). The stage-2
// apt mount is asserted for every bundled file: wave 2 retrofitted
// the toolchain install-stage apt too, so the real contract is two
// mounts per file (plan F2 states the >=1 minimum; we pin reality).

/// Reads a bundled dockerfile relative to the crate root
/// (CARGO_MANIFEST_DIR), panicking with the path on failure.
fn bundled_dockerfile(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "T-C13: bundled dockerfile must be readable at {}: {}",
            path.display(),
            e
        )
    })
}

/// Splits dockerfile content at the second top-level `FROM` line
/// into (stage1, stage2); panics if a second stage is missing.
fn split_stages(content: &str) -> (&str, &str) {
    let mut froms = content.match_indices("\nFROM ");
    froms
        .next()
        .expect("T-C13: dockerfile must contain a first FROM stage");
    let second = froms
        .next()
        .expect("T-C13: dockerfile must contain a second FROM stage");
    let at = second.0 + 1; // keep the newline on the stage-1 side
    (&content[..at], &content[at..])
}

/// The one physical line carrying `needle`, for same-line structural
/// combos (canon-git id/target/REF/seed-copy all live on one line).
fn line_containing<'a>(content: &'a str, needle: &str, what: &str) -> &'a str {
    content
        .lines()
        .find(|l| l.contains(needle))
        .unwrap_or_else(|| {
            panic!(
                "T-C13/T-C14: expected a dockerfile line containing {:?} ({})",
                needle, what
            )
        })
}

/// The canon-git structural parts that must sit on the ONE RUN line
/// carrying the version-keyed cache id (shared bundled/synthesized).
fn assert_canon_git_line(git_line: &str, expected_ref: &str) {
    for part in [
        "target=/opt/.cache/cp2k-src",
        "sharing=locked",
        expected_ref,
        "git clone --recursive -b \"$REF\" https://github.com/cp2k/cp2k.git",
        "cp -a \"$C/HEAD/.\" /opt/cp2k/",
    ] {
        assert!(
            git_line.contains(part),
            "canon-git line must contain {:?} (id/REF/seed-copy combo), got: {}",
            part,
            git_line
        );
    }
}

#[test]
fn t_c13_bundled_spack_dockerfiles_cache_mount_fields() {
    const SPACK_FILES: [&str; 3] = [
        "dockerfiles/spack/2025.2_mpich_cascadelake_psmp.Dockerfile",
        "dockerfiles/spack/2025.2_mpich_x86_64_psmp.Dockerfile",
        "dockerfiles/spack/2025.2_openmpi_cascadelake_psmp.Dockerfile",
    ];
    for rel in SPACK_FILES {
        let content = bundled_dockerfile(rel);
        let (stage1, stage2) = split_stages(&content);

        // Stage 1 and stage 2 apt share the one apt-2404 cache pool
        // (one id per base distro, plan default 4).
        let apt_spec = "--mount=type=cache,target=/var/cache/apt,id=apt-2404,sharing=locked";
        assert!(
            stage1.contains(apt_spec) && stage2.contains(apt_spec),
            "T-C13: {} apt RUNs must mount the apt-2404 cache in BOTH stages (stage1 hit: {}, stage2 hit: {})",
            rel,
            stage1.contains(apt_spec),
            stage2.contains(apt_spec)
        );

        // canon-git keyed cp2k-src-2025.2, REF pinned to support/v2025.2.
        let git_line = line_containing(
            &content,
            "id=cp2k-src-2025.2",
            &format!("{} canon-git block keyed cp2k-src-2025.2", rel),
        );
        assert_canon_git_line(git_line, "REF=support/v2025.2");

        // Bundled spack files pin the literal default source-cache path
        // of SPACK_VERSION=1.2.2 (plan default 7, bundled variant).
        let spack_src_spec =
            "--mount=type=cache,target=/opt/spack-1.2.2/var/spack/cache,id=spack-src,sharing=locked";
        assert!(
            content.contains(spack_src_spec),
            "T-C13: {} spack install RUN must mount id=spack-src on the literal default cache path (target/id/sharing combo)",
            rel
        );

        // Wave 1 (plan t10): the depfile make RUN mounts the local
        // buildcache AND the ccache pool next to spack-src; the push RUN
        // re-mounts the buildcache (two buildcache mounts per file), and
        // ccache stays exclusive to the make RUN.
        let bc_mount =
            "--mount=type=cache,target=/opt/spack-buildcache,id=spack-buildcache,sharing=locked";
        assert!(
            content.matches(bc_mount).count() >= 2,
            "T-C13: {} must mount id=spack-buildcache on BOTH the make and the push RUNs, got {} mounts",
            rel,
            content.matches(bc_mount).count()
        );
        let ccache_mount =
            "--mount=type=cache,target=/opt/spack-ccache,id=spack-ccache,sharing=locked";
        assert!(
            content.matches(ccache_mount).count() == 1,
            "T-C13: {} must mount id=spack-ccache exactly once, on the make RUN, got {}",
            rel,
            content.matches(ccache_mount).count()
        );

        // Mirror/compiler-cache wiring: ccache installed via apt, spack
        // config enables it, the mirror is added unsigned at site scope,
        // and CCACHE_DIR points into the ccache pool.
        assert!(
            content.contains("    ccache \\"),
            "T-C13: {} apt list must install ccache",
            rel
        );
        assert!(
            content.contains("spack config add config:ccache:true"),
            "T-C13: {} must enable ccache via 'spack config add config:ccache:true'",
            rel
        );
        let mirror_line = line_containing(
            &content,
            "file:///opt/spack-buildcache",
            &format!("{} buildcache mirror registration", rel),
        );
        assert!(
            mirror_line.contains(
                "spack mirror add --scope site --unsigned spack-bc file:///opt/spack-buildcache"
            ),
            "T-C13: {} must add the unsigned site-scope spack-bc mirror, got: {}",
            rel,
            mirror_line
        );
        assert!(
            content.contains("ENV CCACHE_DIR=/opt/spack-ccache"),
            "T-C13: {} must point CCACHE_DIR at the spack-ccache pool",
            rel
        );

        // Push is segmented and non-fatal: push and update-index each
        // carry their own distinguishable WARN text on the same physical
        // line as the command they guard.
        let push_line = line_containing(
            &content,
            "buildcache push --unsigned spack-bc",
            &format!("{} buildcache push step", rel),
        );
        assert!(
            push_line.contains(r#"|| echo "WARN: buildcache push failed (non-fatal)""#),
            "T-C13: {} push step must swallow failure with its own WARN, got: {}",
            rel,
            push_line
        );
        let index_line = line_containing(
            &content,
            "buildcache update-index spack-bc",
            &format!("{} buildcache update-index step", rel),
        );
        assert!(
            index_line.contains(r#"|| echo "WARN: buildcache update-index failed (non-fatal)""#),
            "T-C13: {} update-index step must swallow failure with its own WARN, got: {}",
            rel,
            index_line
        );
    }
}

#[test]
fn t_c13_bundled_toolchain_dockerfiles_cache_mount_fields() {
    const TOOLCHAIN_FILES: [&str; 3] = [
        "dockerfiles/toolchain/2023.2_mpich_generic_psmp.Dockerfile",
        "dockerfiles/toolchain/2023.2_mpich_generic_cuda_P100_psmp.Dockerfile",
        "dockerfiles/toolchain/2023.2_mpich_generic_cuda_V100_psmp.Dockerfile",
    ];
    for rel in TOOLCHAIN_FILES {
        let content = bundled_dockerfile(rel);
        let (stage1, stage2) = split_stages(&content);

        // 22.04 base (plain + both CUDA variants) => one apt id.
        let apt_spec = "--mount=type=cache,target=/var/cache/apt,id=apt-2204,sharing=locked";
        assert!(
            stage1.contains(apt_spec) && stage2.contains(apt_spec),
            "T-C13: {} apt RUNs must mount the apt-2204 cache in BOTH stages (stage1 hit: {}, stage2 hit: {})",
            rel,
            stage1.contains(apt_spec),
            stage2.contains(apt_spec)
        );

        // All three variants share the cp2k-src-2023.2 seed cache.
        let git_line = line_containing(
            &content,
            "id=cp2k-src-2023.2",
            &format!("{} canon-git block keyed cp2k-src-2023.2", rel),
        );
        assert_canon_git_line(git_line, "REF=support/v2023.2");

        // Tarball downloads share one global cache across the variants
        // (plan default 4).
        let tarball_spec =
            "--mount=type=cache,target=/opt/cp2k/tools/toolchain/build,id=toolchain-tarballs,sharing=locked";
        assert!(
            content.contains(tarball_spec),
            "T-C13: {} toolchain RUN must mount id=toolchain-tarballs on the toolchain build dir (target/id/sharing combo)",
            rel
        );
    }
}

#[test]
fn t_c14_spack_synthesized_cache_mount_fields() {
    // (version, mpi, cpu): pre-2026 release, 2026.x release, master.
    let cases: [(&str, &str, &str); 3] = [
        ("2025.2", "mpich", "x86_64"),
        ("2026.1", "openmpi", "cascadelake"),
        ("master", "mpich", "x86_64"),
    ];
    let apt_spec = "--mount=type=cache,target=/var/cache/apt,id=apt-2404,sharing=locked";
    let spack_src_spec =
        "--mount=type=cache,target=/opt/spack-source-cache,id=spack-src,sharing=locked";
    for (version, mpi, cpu) in cases {
        let c = base_config("spack", version, mpi, cpu, "none", "psmp");
        let content = generate_dockerfile_content(&c).expect("render spack template");
        let (stage1, stage2) = split_stages(&content);

        assert!(
            stage1.contains(apt_spec) && stage2.contains(apt_spec),
            "T-C14: synthesized spack {} must mount apt-2404 in BOTH stages (stage1 hit: {}, stage2 hit: {})",
            version,
            stage1.contains(apt_spec),
            stage2.contains(apt_spec)
        );
        // Fixed source-cache path (plan default 7): the install RUN and
        // the `spack install --source` RUN must both carry the mount.
        assert!(
            content.matches(spack_src_spec).count() >= 2,
            "T-C14: synthesized spack {} must mount id=spack-src on /opt/spack-source-cache in at least the install and --source RUNs",
            version
        );
        assert!(
            content.contains("spack config add config:source_cache:/opt/spack-source-cache"),
            "T-C14: synthesized spack {} must pin the fixed cache path via 'spack config add config:source_cache:...'",
            version
        );

        // canon-git: version-keyed id + REF on one line; master pins
        // REF=master (plan default 10).
        let expected_id = format!("id=cp2k-src-{}", version);
        let expected_ref = if version == "master" {
            "REF=master;".to_string()
        } else {
            format!("REF=support/v{};", version)
        };
        let git_line = line_containing(
            &content,
            &expected_id,
            &format!(
                "synthesized spack {} canon-git block keyed {}",
                version, expected_id
            ),
        );
        assert_canon_git_line(git_line, &expected_ref);

        // Wave 2 (plan t11): the install RUN carries the buildcache and
        // ccache pools next to spack-src; the push RUN and the cp2k
        // --source RUN each re-mount the buildcache (3 buildcache mounts
        // per render), ccache on install + --source only.
        let bc_mount =
            "--mount=type=cache,target=/opt/spack-buildcache,id=spack-buildcache,sharing=locked";
        assert!(
            content.matches(bc_mount).count() >= 3,
            "T-C14: synthesized spack {} must mount id=spack-buildcache on the install, push and --source RUNs, got {}",
            version,
            content.matches(bc_mount).count()
        );
        let ccache_mount =
            "--mount=type=cache,target=/opt/spack-ccache,id=spack-ccache,sharing=locked";
        assert!(
            content.matches(ccache_mount).count() >= 2,
            "T-C14: synthesized spack {} must mount id=spack-ccache on the install and --source RUNs, got {}",
            version,
            content.matches(ccache_mount).count()
        );

        // Mirror/compiler-cache wiring, same contract as the bundled
        // variant: unsigned site-scope mirror + ccache config + ENV.
        assert!(
            content.contains("spack config add config:ccache:true")
                && content.contains(
                    "spack mirror add --scope site --unsigned spack-bc file:///opt/spack-buildcache"
                )
                && content.contains("ENV CCACHE_DIR=/opt/spack-ccache"),
            "T-C14: synthesized spack {} must register the unsigned spack-bc mirror and wire CCACHE_DIR",
            version
        );

        // Dependencies install signature-unchecked from the unsigned
        // mirror, and the push RUN exists AFTER the dependency install
        // (push what was just built).
        assert!(
            content.contains("spack -e myenv install --no-check-signature"),
            "T-C14: synthesized spack {} must install deps with --no-check-signature from the unsigned mirror",
            version
        );
        let install_at = content
            .find("--no-check-signature")
            .expect("T-C14: --no-check-signature marker must exist");
        let push_at = content
            .find("buildcache push --unsigned spack-bc")
            .expect("T-C14: buildcache push marker must exist");
        assert!(
            push_at > install_at,
            "T-C14: synthesized spack {} buildcache push RUN must come after the dependency install RUN",
            version
        );

        // Segmented, distinguishable WARN texts (push vs update-index).
        assert!(
            content.contains(r#"echo "WARN: buildcache push failed (non-fatal)""#)
                && content.contains(r#"echo "WARN: buildcache update-index failed (non-fatal)""#),
            "T-C14: synthesized spack {} push/update-index steps must each carry their own non-fatal WARN text",
            version
        );

        // cp2k itself is never pushed: its --source install line must not
        // take part in any buildcache push (deps-only mirror).
        let cp2k_source_line = line_containing(
            &content,
            "spack install --source cp2k",
            &format!("synthesized spack {} cp2k --source install", version),
        );
        assert!(
            !cp2k_source_line.contains("buildcache"),
            "T-C14: synthesized spack {} must not push cp2k itself into the buildcache, got: {}",
            version,
            cp2k_source_line
        );

        assert!(
            !content.contains("{{"),
            "T-C14: no template placeholder may survive rendering (synthesized spack {}):\n{}",
            version,
            content
        );
    }
}

#[test]
fn t_c14_toolchain_synthesized_cache_mount_fields() {
    // (version, mpi, cpu, cuda): 2023.2 plain, 2023.2 CUDA, master plain.
    let cases: [(&str, &str, &str, &str); 3] = [
        ("2023.2", "mpich", "generic", "none"),
        ("2023.2", "mpich", "generic", "P100"),
        ("master", "mpich", "generic", "none"),
    ];
    let apt_spec = "--mount=type=cache,target=/var/cache/apt,id=apt-2204,sharing=locked";
    let tarball_spec =
        "--mount=type=cache,target=/opt/cp2k/tools/toolchain/build,id=toolchain-tarballs,sharing=locked";
    for (version, mpi, cpu, cuda) in cases {
        let c = base_config("toolchain", version, mpi, cpu, cuda, "psmp");
        let content = generate_dockerfile_content(&c).expect("render toolchain template");
        let (stage1, stage2) = split_stages(&content);

        assert!(
            stage1.contains(apt_spec) && stage2.contains(apt_spec),
            "T-C14: synthesized toolchain {} (cuda={}) must mount apt-2204 in BOTH stages (stage1 hit: {}, stage2 hit: {})",
            version,
            cuda,
            stage1.contains(apt_spec),
            stage2.contains(apt_spec)
        );
        assert!(
            content.contains(tarball_spec),
            "T-C14: synthesized toolchain {} (cuda={}) must mount id=toolchain-tarballs on the toolchain build dir (target/id/sharing combo)",
            version,
            cuda
        );

        let expected_id = format!("id=cp2k-src-{}", version);
        let expected_ref = if version == "master" {
            "REF=master;".to_string()
        } else {
            format!("REF=support/v{};", version)
        };
        let git_line = line_containing(
            &content,
            &expected_id,
            &format!(
                "synthesized toolchain {} (cuda={}) canon-git block keyed {}",
                version, cuda, expected_id
            ),
        );
        assert_canon_git_line(git_line, &expected_ref);

        assert!(
            !content.contains("{{"),
            "T-C14: no template placeholder may survive rendering (synthesized toolchain {} cuda={}):\n{}",
            version,
            cuda,
            content
        );
    }
}

#[test]
fn t13_resolve_dockerfile_custom_paths() {
    let tmp = TempGuard::new("t13_resolve");
    let df = tmp.0.join("custom.Dockerfile");
    std::fs::write(&df, "FROM scratch\n").expect("write custom dockerfile");

    let mut c = base_config("spack", "2025.2", "mpich", "x86_64", "none", "psmp");
    c.dockerfile = Some(df.clone());
    assert_eq!(
        c.resolve_dockerfile().expect("existing custom dockerfile"),
        df,
        "T-013: an existing custom dockerfile must be used as-is"
    );

    let missing = tmp.0.join("does-not-exist.Dockerfile");
    c.dockerfile = Some(missing.clone());
    let err = c
        .resolve_dockerfile()
        .expect_err("missing custom dockerfile must error");
    assert!(
        err.to_string().contains("Custom Dockerfile not found"),
        "T-013: missing custom dockerfile error must say so, got: {}",
        err
    );
}

#[test]
fn t13_resolve_dockerfile_falls_back_to_synthesized_when_no_bundled_match() {
    // 2099.9 matches nothing bundled anywhere: must synthesize at runtime
    let c = base_config("spack", "2099.9", "mpich", "x86_64", "none", "psmp");
    let path = c.resolve_dockerfile().expect("synthesized fallback");
    assert!(
        path.exists(),
        "T-013: synthesized fallback must write a real file, got: {}",
        path.display()
    );
    let content = std::fs::read_to_string(&path).expect("read synthesized file");
    assert!(
        content.contains("FROM ubuntu:24.04") && !content.contains("{{"),
        "T-013: synthesized file must be a fully rendered spack Dockerfile"
    );
    // cleanup receipt: remove the generated file
    let _ = std::fs::remove_file(&path);
    assert!(
        !path.exists(),
        "T-13: generated dockerfile must be cleaned up"
    );
}

// --------------------------------------------------------
// T-014: failure propagation & cancellation behavior
// --------------------------------------------------------

#[test]
fn t14_execute_build_docker_unreachable_returns_err_not_success() {
    let _serial = PROCESS_TESTS.lock().unwrap_or_else(PoisonError::into_inner);
    let tmp = TempGuard::new("t14_nodocker");
    let df = tmp.0.join("df.Dockerfile");
    std::fs::write(&df, "FROM scratch\n").expect("write dockerfile");

    let mut c = base_config("spack", "2099.9", "mpich", "x86_64", "none", "psmp");
    c.dockerfile = Some(df);

    let (tx, rx) = mpsc::channel::<LogLine>();
    let flag = Arc::new(Mutex::new(false));

    // Inject "docker does not exist" by emptying PATH (restored on drop).
    let _guard = PathGuard::set("");
    let result = execute_build(&c, tx, flag);
    drop(_guard);
    drain_rx(rx);

    match result {
        // Platform note: with PATH cleared, spawn("docker") fails on typical
        // desktops (=> Err mentioning docker), but runners whose PATH search
        // still resolves docker.exe get a real CLI failure mapped to the
        // graceful Ok(Failed). Either way the invariant under test holds:
        // docker unreachable must NEVER report Success.
        Ok(BuildOutcome::Success) => {
            panic!("T-014: execute_build with docker unreachable must not report Success")
        }
        Ok(_other) => { /* graceful Failed/Cancelled is acceptable */ }
        Err(e) => assert!(
            e.to_string().contains("docker"),
            "T-014: spawn failure must mention docker, got: {}",
            e
        ),
    }
}

#[test]
fn t14_execute_build_cancel_flag_pre_set_returns_cancelled_and_kills_child() {
    let _serial = PROCESS_TESTS.lock().unwrap_or_else(PoisonError::into_inner);
    let tmp = TempGuard::new("t14_cancel_build");
    let df = tmp.0.join("df.Dockerfile");
    std::fs::write(&df, "FROM scratch\n").expect("write dockerfile");
    install_fake_docker(&tmp.0);

    let mut c = base_config("spack", "2099.9", "mpich", "x86_64", "none", "psmp");
    c.dockerfile = Some(df);

    let (tx, rx) = mpsc::channel::<LogLine>();
    let flag = Arc::new(Mutex::new(true)); // cancel flag already set

    // A short sleep subprocess drives the flag from a background thread
    // as well (idempotent: the flag is what matters, not who set it).
    let flag2 = flag.clone();
    std::thread::spawn(move || {
        let mut short = spawn_short_sleep_subprocess();
        let _ = short.wait();
        *flag2.lock().unwrap_or_else(PoisonError::into_inner) = true;
    });

    // The fake `docker` (a shell copy) is spawned, then the poll loop
    // observes the cancel flag before try_wait and kills the child.
    let _guard = PathGuard::set(&tmp.0.to_string_lossy());
    let result = execute_build(&c, tx, flag);
    drop(_guard);
    drain_rx(rx);

    assert_eq!(
        result.expect("execute_build must not error"),
        BuildOutcome::Cancelled,
        "T-014: execute_build with a pre-set cancel flag must return Cancelled (child killed, not joined)"
    );
}

#[test]
fn t14_run_cmd_logged_cancel_kills_child_before_sentinel_is_written() {
    let _serial = PROCESS_TESTS.lock().unwrap_or_else(PoisonError::into_inner);
    let tmp = TempGuard::new("t14_cancel_cmd");
    let sentinel = tmp.0.join("sentinel.txt");
    let _ = std::fs::remove_file(&sentinel);

    let (cmd, args) = long_sleep_with_sentinel(&sentinel);
    let (tx, rx) = mpsc::channel::<LogLine>();
    let flag = Arc::new(Mutex::new(false));

    // Background thread: run a short sleep subprocess, then set the
    // cancel flag (event-driven; no precise timing assertion).
    let flag2 = flag.clone();
    std::thread::spawn(move || {
        let mut short = spawn_short_sleep_subprocess();
        let _ = short.wait();
        *flag2.lock().unwrap_or_else(PoisonError::into_inner) = true;
    });

    let status = run_cmd_logged(cmd, &args, None, &tx, &flag)
        .expect("run_cmd_logged must not error on cancel");
    drain_rx(rx);

    assert_eq!(
        status,
        CmdStatus::Cancelled,
        "T-014: run_cmd_logged must return Cancelled when the flag is set mid-run"
    );

    // Kill proof (event-based, generous slack): the sentinel is written
    // by the child *after* its 30s sleep; it must never appear because
    // the child was killed. Poll ~3s to catch a premature write.
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        assert!(
            !sentinel.exists(),
            "T-014: sentinel file appeared -> the child was NOT killed and ran to completion"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn t14_run_cmd_logged_completion_returns_completed() {
    let _serial = PROCESS_TESTS.lock().unwrap_or_else(PoisonError::into_inner);
    let (tx, rx) = mpsc::channel::<LogLine>();
    let flag = Arc::new(Mutex::new(false));
    #[cfg(target_os = "windows")]
    let (cmd, args): (&str, Vec<String>) =
        ("cmd", vec!["/D".into(), "/C".into(), "echo hi".into()]);
    #[cfg(not(target_os = "windows"))]
    let (cmd, args): (&str, Vec<String>) = ("echo", vec!["hi".to_string()]);
    let status =
        run_cmd_logged(cmd, &args, None, &tx, &flag).expect("trivial command must not error");
    drain_rx(rx);
    assert_eq!(
        status,
        CmdStatus::Completed,
        "T-014: a successful command must map to CmdStatus::Completed"
    );
}

#[test]
fn t14_execute_native_build_rejects_unsupported_combination() {
    // Native path is hardcoded for x86_64/mpich/psmp/cuda=none; anything
    // else must fail explicitly before any step runs.
    let c = base_config("native", "master", "openmpi", "generic", "none", "psmp");
    let (tx, rx) = mpsc::channel::<LogLine>();
    let flag = Arc::new(Mutex::new(false));
    let result = execute_native_build(&c, tx, flag);
    drain_rx(rx);
    let err = result.expect_err("T-014: unsupported native combination must return Err");
    assert!(
        err.to_string()
            .contains("Unsupported native build configuration"),
        "T-014: error must name the unsupported combination, got: {}",
        err
    );
}

// Note on execute_native_build cancellation coverage: every cancellable
// step in execute_native_build funnels through the same `run_step!` ->
// run_cmd_logged -> CmdStatus::Cancelled -> `return
// Ok(BuildOutcome::Cancelled)` mapping exercised by
// t14_run_cmd_logged_cancel_kills_child_before_sentinel_is_written.
// The surrounding steps (git clone of the full cp2k tree, apt-get, the
// toolchain script) are hardcoded and cannot be injected without a real
// Linux build host, so the mapping is covered at the run_cmd_logged
// level (documented in the results JSON).

// --------------------------------------------------------
// Wave 5 (t14): native 6-combination whitelist matrix,
// toolchain-args flag surface, prereq package mapping
// --------------------------------------------------------
//
// These tests pin the Wave 4 parameterization (validate_native_config,
// native_toolchain_args) and the t9 prereq fix (probe list + apt package
// mapping). Expected values are independent literals — never re-derived
// from the production code under test — and the Dockerfile cross-checks
// keep them anchored to the bundled reference files. The red drill in the
// result JSON proves that reverting a pinned production arm turns the
// corresponding assertion red.

#[test]
fn t14_native_whitelist_accepts_six_combinations_and_rejects_the_rest() {
    // Exactly cpu ∈ {generic, x86_64} × cuda ∈ {none, P100, V100} pass,
    // with mpi='mpich' and variant='psmp' locked. The version dimension is
    // pinned to a release (2023.2), so a pass here can never come from the
    // master+cuda exception path.
    let mut accepted = 0usize;
    for cpu in ["generic", "x86_64"] {
        for cuda in ["none", "P100", "V100"] {
            match super::execute::validate_native_config("2023.2", cpu, "mpich", "psmp", cuda) {
                Ok(()) => accepted += 1,
                Err(e) => panic!(
                    "T-014: whitelisted 2023.2 combination cpu={:?} cuda={:?} must pass Step 0, got: {}",
                    cpu, cuda, e
                ),
            }
        }
    }
    assert_eq!(
        accepted, 6,
        "T-014: exactly the 6 whitelisted combinations must pass"
    );
    assert!(
        super::execute::validate_native_config("master", "x86_64", "mpich", "psmp", "none").is_ok(),
        "T-014: master + cuda='none' must stay supported (cmake path)"
    );

    // One representative per rejection axis; each must Err AND list the
    // full supported set (not merely echo the offending values).
    let rejected: [(&str, &str, &str, &str, &str); 5] = [
        ("2023.2", "x86_64", "openmpi", "psmp", "none"), // mpi axis
        ("2023.2", "x86_64", "mpich", "ssmp", "none"),   // variant axis
        ("2023.2", "x86_64", "mpich", "pdbg", "none"),   // variant axis
        ("2023.2", "cascadelake", "mpich", "psmp", "none"), // cpu axis
        ("master", "x86_64", "mpich", "psmp", "P100"),   // version x cuda axis
    ];
    for (version, cpu, mpi, variant, cuda) in rejected {
        let err = super::execute::validate_native_config(version, cpu, mpi, variant, cuda)
            .expect_err("T-014: non-whitelist combination must be rejected");
        assert!(
            err.contains("Unsupported native build configuration"),
            "T-014: rejection must name the Step 0 validation, got: {}",
            err
        );
        for needle in [
            "cpu in {'generic', 'x86_64'}",
            "mpi='mpich'",
            "variant='psmp'",
            "cuda in {'none', 'P100', 'V100'}",
        ] {
            assert!(
                err.contains(needle),
                "T-014: rejection message must list supported-set member {:?}, got: {}",
                needle,
                err
            );
        }
        for echo in [
            format!("version='{}'", version),
            format!("cpu='{}'", cpu),
            format!("mpi='{}'", mpi),
            format!("variant='{}'", variant),
            format!("cuda='{}'", cuda),
        ] {
            assert!(
                err.contains(&echo),
                "T-014: rejection message must echo offending value {:?}, got: {}",
                echo,
                err
            );
        }
    }
}

#[test]
fn t14_native_toolchain_args_match_bundled_dockerfile_flag_surface() {
    // Exact expected argv per whitelisted cpu x cuda combination. These
    // literals ARE the bundled-Dockerfile flag surface
    // (dockerfiles/toolchain/2023.2_mpich_generic[_cuda_<GPU>]_psmp.Dockerfile
    // lines 23-33), written out independently of the implementation.
    let cases: [(&str, &str, &[&str]); 6] = [
        (
            "generic",
            "none",
            &[
                "-j",
                "8",
                "--install-all",
                "--enable-cuda=no",
                "--target-cpu=generic",
                "--with-cusolvermp=no",
                "--with-gcc=system",
                "--with-mpich=system",
            ],
        ),
        (
            "x86_64",
            "none",
            &[
                "-j",
                "8",
                "--install-all",
                "--enable-cuda=no",
                "--target-cpu=generic",
                "--with-cusolvermp=no",
                "--with-gcc=system",
                "--with-mpich=system",
            ],
        ),
        (
            "generic",
            "P100",
            &[
                "-j",
                "8",
                "--install-all",
                "--enable-cuda=yes",
                "--gpu-ver=P100",
                "--with-libtorch=no",
                "--target-cpu=generic",
                "--with-cusolvermp=no",
                "--with-gcc=system",
                "--with-mpich=system",
            ],
        ),
        (
            "x86_64",
            "P100",
            &[
                "-j",
                "8",
                "--install-all",
                "--enable-cuda=yes",
                "--gpu-ver=P100",
                "--with-libtorch=no",
                "--target-cpu=generic",
                "--with-cusolvermp=no",
                "--with-gcc=system",
                "--with-mpich=system",
            ],
        ),
        (
            "generic",
            "V100",
            &[
                "-j",
                "8",
                "--install-all",
                "--enable-cuda=yes",
                "--gpu-ver=V100",
                "--with-libtorch=no",
                "--target-cpu=generic",
                "--with-cusolvermp=no",
                "--with-gcc=system",
                "--with-mpich=system",
            ],
        ),
        (
            "x86_64",
            "V100",
            &[
                "-j",
                "8",
                "--install-all",
                "--enable-cuda=yes",
                "--gpu-ver=V100",
                "--with-libtorch=no",
                // x86_64 is the CLI baseline name; the toolchain script
                // vocabulary only knows `generic` (probe 38061298248).
                "--target-cpu=generic",
                "--with-cusolvermp=no",
                "--with-gcc=system",
                "--with-mpich=system",
            ],
        ),
    ];
    for (cpu, cuda, want) in cases {
        let c = base_config("native", "2023.2", "mpich", cpu, cuda, "psmp");
        let got = super::execute::native_toolchain_args(&c);
        let want: Vec<String> = want.iter().map(|s| s.to_string()).collect();
        assert_eq!(
            got, want,
            "T-014: native toolchain args for cpu={:?} cuda={:?} must match the bundled Dockerfile flag surface exactly",
            cpu, cuda
        );

        if cuda == "none" {
            assert!(
                got.iter().any(|a| a == "--enable-cuda=no"),
                "T-014: cuda=none combination must pass --enable-cuda=no, got: {:?}",
                got
            );
            assert!(
                !got.iter().any(|a| a.starts_with("--gpu-ver=")),
                "T-014: cuda=none combination must not pass --gpu-ver, got: {:?}",
                got
            );
            assert!(
                !got.iter().any(|a| a == "--with-libtorch=no"),
                "T-014: cuda=none combination must not pass --with-libtorch=no, got: {:?}",
                got
            );
        } else {
            assert!(
                got.iter().any(|a| a == "--enable-cuda=yes"),
                "T-014: cuda={:?} combination must pass --enable-cuda=yes, got: {:?}",
                cuda,
                got
            );
            let gpu_flag = format!("--gpu-ver={}", cuda);
            assert!(
                got.iter().any(|a| a == &gpu_flag),
                "T-014: cuda={:?} combination must pass {:?}, got: {:?}",
                cuda,
                gpu_flag,
                got
            );
            assert!(
                got.iter().any(|a| a == "--with-libtorch=no"),
                "T-014: cuda={:?} combination must pass --with-libtorch=no (bundled CUDA Dockerfile), got: {:?}",
                cuda,
                got
            );
        }
        // Regression guard for the runner-probed 2023.2 rejection: the
        // script answers --with-deepmd with "Unknown flag", so NO
        // combination may ever carry it.
        assert!(
            !got.iter().any(|a| a.contains("deepmd")),
            "T-014: no native combination may pass --with-deepmd (rejected by the 2023.2 script), got: {:?}",
            got
        );
    }

    // Cross-check the literal tables above against the bundled files: the
    // flag spellings and grouping must be the ones the images actually use.
    const GENERIC_DF: &str = "dockerfiles/toolchain/2023.2_mpich_generic_psmp.Dockerfile";
    const P100_DF: &str = "dockerfiles/toolchain/2023.2_mpich_generic_cuda_P100_psmp.Dockerfile";
    const V100_DF: &str = "dockerfiles/toolchain/2023.2_mpich_generic_cuda_V100_psmp.Dockerfile";
    let generic = bundled_dockerfile(GENERIC_DF);
    for flag in [
        "--install-all",
        "--enable-cuda=no",
        "--target-cpu=generic",
        "--with-cusolvermp=no",
        "--with-gcc=system",
        "--with-mpich=system",
    ] {
        assert!(
            generic.contains(flag),
            "T-014: bundled {} must carry {:?} (native args reference)",
            GENERIC_DF,
            flag
        );
    }
    assert!(
        !generic.contains("--gpu-ver=") && !generic.contains("--with-libtorch=no"),
        "T-014: bundled {} (cuda=none) must not carry CUDA-only flags",
        GENERIC_DF
    );
    for (rel, gpu) in [(P100_DF, "P100"), (V100_DF, "V100")] {
        let df = bundled_dockerfile(rel);
        let cuda_run = format!("--enable-cuda=yes --gpu-ver={} --with-libtorch=no", gpu);
        assert!(
            df.contains(&cuda_run),
            "T-014: bundled {} must carry the CUDA flag run {:?} (native args reference)",
            rel,
            cuda_run
        );
        for flag in [
            "--install-all",
            "--target-cpu=generic",
            "--with-cusolvermp=no",
            "--with-gcc=system",
            "--with-mpich=system",
        ] {
            assert!(
                df.contains(flag),
                "T-014: bundled {} must carry {:?} (native args reference)",
                rel,
                flag
            );
        }
    }
    for rel in [GENERIC_DF, P100_DF, V100_DF] {
        assert!(
            !bundled_dockerfile(rel).contains("deepmd"),
            "T-014: bundled {} must not carry the rejected --with-deepmd flag",
            rel
        );
    }
}

#[test]
fn t14_native_prereq_lists_and_package_mapping_are_pinned() {
    use super::execute::{
        apt_install_packages, prereq_package_for, NATIVE_APT_EXTRAS, NATIVE_REQUIRED_TOOLS,
    };

    // t9 mpich supply fix: mpicc is probed, mpich/libmpich-dev/bzip2 are
    // installed from apt (package names identical to the bundled
    // Dockerfile package line).
    assert!(
        NATIVE_REQUIRED_TOOLS.contains(&"mpicc"),
        "T-014: probe list must include mpicc (mpich prereq fix), got: {:?}",
        NATIVE_REQUIRED_TOOLS
    );
    for pkg in ["mpich", "libmpich-dev", "bzip2"] {
        assert!(
            NATIVE_APT_EXTRAS.contains(&pkg),
            "T-014: apt extras must include {:?} (mpich supply / bunzip2 provider), got: {:?}",
            pkg,
            NATIVE_APT_EXTRAS
        );
    }

    // Table-driven mapping coverage (gate-review addition): every probed
    // tool name maps to the real Ubuntu package that provides it, and the
    // table covers the probe list EXACTLY — a future tool added to
    // NATIVE_REQUIRED_TOOLS without a mapping row fails here instead of
    // silently regressing the apt batch.
    const EXPECTED_PACKAGE: &[(&str, &str)] = &[
        ("gcc", "gcc"),
        ("g++", "g++"),
        ("gfortran", "gfortran"),
        ("git", "git"),
        ("make", "make"),
        ("cmake", "cmake"),
        ("wget", "wget"),
        ("bunzip2", "bzip2"),
        ("mpicc", "libmpich-dev"),
    ];
    for &(tool, pkg) in EXPECTED_PACKAGE {
        assert!(
            NATIVE_REQUIRED_TOOLS.contains(&tool),
            "T-014: mapping-table row {:?} is not in the probe list — stale test row",
            tool
        );
        assert_eq!(
            prereq_package_for(tool),
            pkg,
            "T-014: probe {:?} must map to its provider package {:?}",
            tool,
            pkg
        );
    }
    for &tool in NATIVE_REQUIRED_TOOLS {
        assert!(
            EXPECTED_PACKAGE.iter().any(|&(t, _)| t == tool),
            "T-014: probe tool {:?} has no row in the expected mapping table — map it and register it here",
            tool
        );
        let pkg = prereq_package_for(tool);
        assert!(
            !pkg.is_empty() && !pkg.contains('/') && !pkg.contains(char::is_whitespace),
            "T-014: mapped package for {:?} must be a bare package name, got {:?}",
            tool,
            pkg
        );
    }

    // apt batch: probe names never appear, providers do, and every extra
    // survives dedup exactly once (bzip2 arrives twice: bunzip2 mapping +
    // extras row).
    let pkgs = apt_install_packages(NATIVE_REQUIRED_TOOLS);
    assert!(
        !pkgs.iter().any(|p| p.as_str() == "mpicc"),
        "T-014: mpicc is a probe name, never an apt package: {:?}",
        pkgs
    );
    assert!(
        !pkgs.iter().any(|p| p.as_str() == "bunzip2"),
        "T-014: bunzip2 is a probe name, never an apt package: {:?}",
        pkgs
    );
    for &tool in NATIVE_REQUIRED_TOOLS {
        let pkg = prereq_package_for(tool);
        assert!(
            pkgs.iter().any(|p| p.as_str() == pkg),
            "T-014: provider {:?} of probe {:?} must appear in the apt batch",
            pkg,
            tool
        );
    }
    for &extra in NATIVE_APT_EXTRAS {
        let n = pkgs.iter().filter(|p| p.as_str() == extra).count();
        assert_eq!(
            n, 1,
            "T-014: extra {:?} must appear exactly once after dedup, got {}",
            extra, n
        );
    }
    let bz = pkgs.iter().filter(|p| p.as_str() == "bzip2").count();
    assert_eq!(
        bz, 1,
        "T-014: bzip2 (bunzip2 mapping + extras) must be deduplicated to exactly one entry, got {}",
        bz
    );

    // Dedup also holds when the same tool is reported missing twice.
    let dup = apt_install_packages(&["bunzip2", "bunzip2", "mpicc", "mpicc"]);
    assert_eq!(
        dup.iter().filter(|p| p.as_str() == "bzip2").count(),
        1,
        "T-014: repeated bunzip2 misses must still yield one bzip2 entry: {:?}",
        dup
    );
    assert_eq!(
        dup.iter().filter(|p| p.as_str() == "libmpich-dev").count(),
        1,
        "T-014: repeated mpicc misses must still yield one libmpich-dev entry: {:?}",
        dup
    );
}

// --------------------------------------------------------
// Wave 5 (T-O21): method → CUDA option support matrix
// --------------------------------------------------------

#[test]
fn t_o21_cuda_options_toolchain_offers_gpu_variants() {
    // The GUI's only source for the CUDA list; bundled toolchain
    // dockerfiles exist for P100 and V100 (plus CPU-only "none").
    assert_eq!(
        cuda_options_for_method("toolchain"),
        vec!["none", "P100", "V100"],
        "T-O21: toolchain must offer none + both bundled GPU variants"
    );
}

#[test]
fn t_o21_cuda_options_spack_is_cpu_only() {
    // Bundled spack images are CPU-only: exactly one option, "none" —
    // no GPU variant may leak into the spack list.
    assert_eq!(
        cuda_options_for_method("spack"),
        vec!["none"],
        "T-O21: spack must offer only cuda=none (bundled images are CPU-only)"
    );
}

#[test]
fn t_o21_cuda_options_native_and_unknown_offer_empty_set() {
    // Flipped premise (Wave 4 whitelist expansion): native now offers the
    // bundled CUDA set, in the matrix's declared order; only unrecognized
    // methods still map to an EMPTY set, which the GUI renders disabled.
    assert_eq!(
        cuda_options_for_method("native"),
        vec!["none", "P100", "V100"],
        "T-O21: native must offer none + both bundled GPU variants"
    );
    for method in ["", "docker", "not-a-method"] {
        assert!(
            cuda_options_for_method(method).is_empty(),
            "T-O21: method {:?} must map to an empty CUDA option set",
            method
        );
    }
}

#[test]
fn t_o21_native_step0_rejects_gpu_options_consistent_with_empty_matrix() {
    // Flipped premise (Wave 4 whitelist expansion): the native matrix now
    // offers the bundled CUDA set, so every offered combination must be
    // ACCEPTED by the pure Step 0 validator for a pinned release version
    // (2023.2); master+CUDA (version-dimension rule) and non-whitelist
    // combinations stay rejected. The validator has no subprocess or
    // filesystem side effect, so this stays a pure unit-level check.
    for cuda in cuda_options_for_method("native") {
        for cpu in ["x86_64", "generic"] {
            assert!(
                super::execute::validate_native_config("2023.2", cpu, "mpich", "psmp", cuda)
                    .is_ok(),
                "T-O21: 2023.2 native combination cpu={:?}, cuda={:?} must pass Step 0",
                cpu,
                cuda
            );
        }
    }
    for (version, cpu, mpi, variant, cuda) in [
        ("2023.2", "x86_64", "openmpi", "psmp", "none"),
        ("master", "x86_64", "mpich", "psmp", "P100"),
    ] {
        let err = super::execute::validate_native_config(version, cpu, mpi, variant, cuda)
            .expect_err("T-O21: non-whitelist native combination must be rejected");
        assert!(
            err.contains("Unsupported native build configuration"),
            "T-O21: rejection must name the Step 0 validation, got: {}",
            err
        );
    }
}

// --------------------------------------------------------
// Wave 5 (T-O22): LogLine / LogLevel semantics
// --------------------------------------------------------
//
// Scope note: LogLevel/LogLine are in-memory structs with no serde impl —
// the crate never serializes them — and the level→color (gui.rs) and
// level→prefix (main.rs) mappings are inline in files outside this wave's
// write whitelist. These tests pin the pure, in-crate contract instead.

#[test]
fn t_o22_log_level_debug_spelling_is_stable() {
    // Derived Debug is a LogLevel's only textual rendering; a variant
    // rename would silently change every {:?} diagnostic.
    assert_eq!(format!("{:?}", LogLevel::Info), "Info");
    assert_eq!(format!("{:?}", LogLevel::Warn), "Warn");
    assert_eq!(format!("{:?}", LogLevel::Error), "Error");
}

#[test]
fn t_o22_log_line_clone_roundtrip_preserves_all_fields() {
    // LogLine travels as an in-memory message (mpsc::Sender<LogLine>);
    // Clone is its "serialization" — all three fields must roundtrip.
    for level in [LogLevel::Info, LogLevel::Warn, LogLevel::Error] {
        let line = LogLine {
            timestamp: "12:34:56".to_string(),
            text: format!("probe {:?}", level),
            level,
        };
        let copy = line.clone();
        assert_eq!(copy.timestamp, line.timestamp);
        assert_eq!(copy.text, line.text, "text must survive clone");
        assert_eq!(copy.level, line.level, "level must survive clone");
    }
}

#[test]
fn t_o22_log_level_equality_matrix_pins_branch_conditions() {
    // gui.rs coloring and main.rs prefixing branch on exactly
    // `level == LogLevel::Error` / `level == LogLevel::Warn`; pin the full
    // (is_warn, is_error) truth table those inline matchers rely on.
    let (inf, wrn, err) = (LogLevel::Info, LogLevel::Warn, LogLevel::Error);
    assert_eq!((inf == wrn, inf == err), (false, false));
    assert_eq!((wrn == wrn, wrn == err), (true, false));
    assert_eq!((err == wrn, err == err), (false, true));
}

// --------------------------------------------------------
// Wave 1 (t26): CP2K 2026.2 toolchain capability assertions
// (red wave — the cmake arm, the CUDA cmake flags and the 2026.2
// registrations do not exist yet; these tests go green in Wave 2)
// --------------------------------------------------------

/// (i) Predicate boundary: `toolchain_uses_cmake` must select the make
/// path for releases that still ship a root Makefile (up to v2025.2) and
/// the cmake path for master and every release from v2026.1 on (upstream
/// deleted the root Makefile/arch in v2026.1).
#[test]
fn t26_toolchain_uses_cmake_predicate_boundary() {
    // false side (regression guard for the make-based releases)
    for version in ["2023.2", "2025.2"] {
        assert!(
            !super::generate::toolchain_uses_cmake(version),
            "T26: toolchain_uses_cmake({version}) must be false — pre-2026 releases build via make"
        );
    }
    // true side: collect every violation so one red message pins the
    // whole boundary (the wave-1 stub answers false for everything)
    let mut got_false: Vec<&str> = Vec::new();
    for version in ["2026.1", "2026.2", "2027.1", "master"] {
        if !super::generate::toolchain_uses_cmake(version) {
            got_false.push(version);
        }
    }
    assert!(
        got_false.is_empty(),
        "T26: toolchain_uses_cmake must be true for master and >=2026.1 (no root Makefile upstream), got false for: {:?}",
        got_false
    );
}

/// (ii) 2026.2 CPU render (toolchain/mpich/generic/none/psmp) must go
/// through the cmake arm: the full cmake flag surface, no make step, no
/// CUDA cmake flag, and the structural cache mounts stay intact
/// (t_c14 pattern).
#[test]
fn t26_2026_2_cpu_render_uses_cmake_arm() {
    let c = base_config("toolchain", "2026.2", "mpich", "generic", "none", "psmp");
    let content =
        generate_dockerfile_content(&c).expect("render toolchain template for 2026.2 CPU");

    // structural mounts keep the t_c14 contract (green now, regression
    // guard once Wave 2 lands)
    let (stage1, stage2) = split_stages(&content);
    let apt_spec = "--mount=type=cache,target=/var/cache/apt,id=apt-2204,sharing=locked";
    assert!(
        stage1.contains(apt_spec) && stage2.contains(apt_spec),
        "T26: 2026.2 CPU render must mount apt-2204 in BOTH stages (stage1 hit: {}, stage2 hit: {})",
        stage1.contains(apt_spec),
        stage2.contains(apt_spec)
    );
    let tarball_spec =
        "--mount=type=cache,target=/opt/cp2k/tools/toolchain/build,id=toolchain-tarballs,sharing=locked";
    assert!(
        content.contains(tarball_spec),
        "T26: 2026.2 CPU render must mount id=toolchain-tarballs on the toolchain build dir (target/id/sharing combo)"
    );

    // canon-git: version-keyed id and support/v2026.2 ref on ONE line
    let git_line = line_containing(
        &content,
        "id=cp2k-src-2026.2",
        "2026.2 CPU canon-git block keyed cp2k-src-2026.2",
    );
    assert_canon_git_line(git_line, "REF=support/v2026.2;");

    // the toolchain script stays CPU-only
    assert!(
        content.contains("--enable-cuda=no") && content.contains("--target-cpu=generic"),
        "T26: 2026.2 CPU render must pass --enable-cuda=no and --target-cpu=generic to the toolchain script"
    );

    // cmake-arm flag surface (master-arm base set + LIBTORCH OFF, plan
    // default 8); collect every missing marker into one red message
    const REQUIRED: [&str; 6] = [
        "cmake -GNinja",
        "-DCP2K_USE_EVERYTHING=ON",
        "-DCP2K_USE_DLAF=OFF",
        "-DCP2K_USE_PEXSI=OFF",
        "-DCP2K_USE_DEEPMD=OFF",
        "-DCP2K_USE_LIBTORCH=OFF",
    ];
    let mut missing: Vec<&str> = Vec::new();
    for marker in REQUIRED {
        if !content.contains(marker) {
            missing.push(marker);
        }
    }
    assert!(
        missing.is_empty(),
        "T26: 2026.2 CPU render must carry the full cmake-arm flag surface, missing: {:?}",
        missing
    );

    // the make arm must be gone, and no CUDA cmake flag may leak into a
    // CPU-only render
    assert!(
        !content.contains("make -j"),
        "T26: 2026.2 CPU render must not carry the make build step (upstream deleted the root Makefile in v2026.1)"
    );
    assert!(
        !content.contains("-DCP2K_USE_ACCEL"),
        "T26: 2026.2 CPU render must not carry any CUDA cmake flag"
    );
    assert!(
        !content.contains("{{"),
        "T26: no template placeholder may survive rendering (2026.2 CPU):\n{}",
        content
    );
}

/// (iii) 2026.2 GPU render (toolchain/mpich/generic/P100/psmp): nvidia
/// base + the CUDA toolchain flag run, plus the CUDA cmake triple and
/// the shared LIBTORCH OFF (upstream make_cp2k.sh:898-901, P100 -> sm 60).
#[test]
fn t26_2026_2_gpu_render_uses_cuda_cmake_flags() {
    let c = base_config("toolchain", "2026.2", "mpich", "generic", "P100", "psmp");
    let content =
        generate_dockerfile_content(&c).expect("render toolchain template for 2026.2 GPU");

    assert!(
        content.contains("FROM nvidia/cuda:12.2.0-devel-ubuntu22.04"),
        "T26: 2026.2 GPU render must be FROM the nvidia/cuda 12.2 devel image"
    );
    let cuda_run = "--enable-cuda=yes --gpu-ver=P100 --with-libtorch=no";
    assert!(
        content.contains(cuda_run),
        "T26: 2026.2 GPU render must pass the CUDA flag run {:?} to the toolchain script",
        cuda_run
    );

    // CUDA cmake flag surface; collect every missing marker into one
    // red message
    const REQUIRED: [&str; 4] = [
        "-DCP2K_USE_ACCEL=CUDA",
        "-DCP2K_WITH_GPU=P100",
        "-DCMAKE_CUDA_ARCHITECTURES=60",
        "-DCP2K_USE_LIBTORCH=OFF",
    ];
    let mut missing: Vec<&str> = Vec::new();
    for marker in REQUIRED {
        if !content.contains(marker) {
            missing.push(marker);
        }
    }
    assert!(
        missing.is_empty(),
        "T26: 2026.2 GPU render must carry the CUDA cmake flag surface, missing: {:?}",
        missing
    );

    assert!(
        !content.contains("{{"),
        "T26: no template placeholder may survive rendering (2026.2 GPU):\n{}",
        content
    );
}

/// (iv) resolve_dockerfile() for 2026.2 (no bundled file) must write a
/// real file carrying the 2026.2 canon-git line and the cmake-arm build
/// step, fully rendered (fallback pattern of
/// t13_resolve_dockerfile_falls_back_to_synthesized_when_no_bundled_match,
/// with cleanup receipt).
#[test]
fn t26_2026_2_resolve_dockerfile_synthesizes_cmake_file() {
    let c = base_config("toolchain", "2026.2", "mpich", "generic", "none", "psmp");
    let path = c.resolve_dockerfile().expect("2026.2 synthesized fallback");
    assert!(
        path.exists(),
        "T26: 2026.2 synthesized fallback must write a real file, got: {}",
        path.display()
    );
    let content = std::fs::read_to_string(&path).expect("read synthesized 2026.2 file");

    // 2026.2 marker: canon-git id + ref on ONE line
    let git_line = line_containing(
        &content,
        "id=cp2k-src-2026.2",
        "resolved 2026.2 canon-git block keyed cp2k-src-2026.2",
    );
    assert_canon_git_line(git_line, "REF=support/v2026.2;");

    // capability marker: the resolved file must be the cmake path
    assert!(
        content.contains("cmake -GNinja"),
        "T26: resolved 2026.2 dockerfile must use the cmake+ninja build step"
    );
    assert!(
        !content.contains("{{"),
        "T26: resolved 2026.2 dockerfile must be fully rendered"
    );

    // cleanup receipt: remove the generated file
    let _ = std::fs::remove_file(&path);
    assert!(
        !path.exists(),
        "T26: generated 2026.2 dockerfile must be cleaned up"
    );
}

/// (v) catalog: the synthesized table must advertise exactly the two
/// toolchain 2026.2 entries as Synthesized; spack 2026.2 must NOT be
/// advertised (upstream tools/spack reshuffle; delegating to
/// make_cp2k.sh is backlog only).
#[test]
fn t26_catalog_advertises_2026_2_toolchain_only() {
    let configs = list_available_configs();

    // both 2026.2 toolchain entries must be advertised as Synthesized;
    // collect all absent tuples so one red message pins the full set
    let mut missing: Vec<String> = Vec::new();
    for cuda in ["none", "P100"] {
        let want = format!("(toolchain,2026.2,mpich,generic,{cuda},psmp)");
        match configs.iter().find(|c| {
            c.method == "toolchain"
                && c.version == "2026.2"
                && c.mpi == "mpich"
                && c.cpu == "generic"
                && c.cuda == cuda
                && c.variant == "psmp"
        }) {
            None => missing.push(want),
            Some(hit) => assert_eq!(
                hit.source,
                DockerfileSource::Synthesized,
                "T26: {} must be advertised as Synthesized, got {:?}",
                want,
                hit.source
            ),
        }
    }
    assert!(
        missing.is_empty(),
        "T26: list_available_configs must advertise the 2026.2 toolchain entries, missing: {:?}",
        missing
    );

    assert!(
        !configs
            .iter()
            .any(|c| c.method == "spack" && c.version == "2026.2"),
        "T26: spack 2026.2 must not be advertised (upstream tools/spack reshuffle)"
    );
}

// --------------------------------------------------------
// Wave 3: DBCSR config-version patch in the release-cmake arm
// --------------------------------------------------------
//
// Probe run 38393254283 (stage 2b, cmake-configure.txt:48-55): the dbcsr
// tarball build's `git describe` climbs out of its build dir into the outer
// cp2k clone whose HEAD sits on tag v2026.2, so the installed
// DBCSRConfigVersion.cmake reports the malformed version
// `2026.2.v2026.2v2026.2` and `find_package(DBCSR 2.8)` (CP2K
// CMakeLists.txt:551) is rejected by the SameMajorVersion check. Upstream
// CI is immune (COPY, no .git); every git-clone build must patch the
// installed config version before configuring. The matcher below is
// deliberately tolerant (`sed -i` + `DBCSRConfigVersion.cmake` on ONE
// physical line) so a cwd-driven path adjustment cannot break the test.

/// The release-cmake arm (2026.2 CPU and GPU renders) must patch the dbcsr
/// config version between sourcing the toolchain env and the cmake call.
#[test]
fn t_wave3_dbcsr_version_patch_sits_between_source_and_cmake() {
    for cuda in ["none", "P100"] {
        let c = base_config("toolchain", "2026.2", "mpich", "generic", cuda, "psmp");
        let content =
            generate_dockerfile_content(&c).expect("render toolchain template for 2026.2");

        let sed_line = content
            .lines()
            .find(|l| l.contains("sed -i") && l.contains("DBCSRConfigVersion.cmake"))
            .unwrap_or_else(|| {
                panic!(
                    "T-W3: 2026.2 (cuda={cuda}) render must carry a dbcsr config-version sed line (sed -i + DBCSRConfigVersion.cmake on one physical line)"
                )
            });
        assert!(
            sed_line.contains("PACKAGE_VERSION"),
            "T-W3: 2026.2 (cuda={cuda}) dbcsr sed line must rewrite PACKAGE_VERSION, got: {sed_line}"
        );

        // ordering: after the toolchain env source, before the cmake call
        let src_at = content
            .find("source ${TOOLCHAIN_DIR}/install/setup")
            .expect("T-W3: release-cmake render must source the toolchain env");
        let sed_at = content
            .find(sed_line)
            .expect("T-W3: sed line located above must be a substring of the render");
        let cmake_at = content
            .find("cmake -GNinja")
            .expect("T-W3: release-cmake render must call cmake");
        assert!(
            src_at < sed_at && sed_at < cmake_at,
            "T-W3: dbcsr sed line must sit after the toolchain source (at {src_at}) and before the cmake call (at {cmake_at}), found at {sed_at}"
        );
    }
}

/// Master (cmake arm) and 2023.2 (make arm) must stay untouched: the dbcsr
/// patch is release-cmake-arm-only. Master is a rolling branch whose
/// describe yields a v*.*-N-gHASH string by construction (no tag on HEAD),
/// and Wave 2 froze its rendered bytes.
#[test]
fn t_wave3_master_and_make_arms_render_no_dbcsr_patch() {
    for (version, what) in [
        ("master", "master cmake arm"),
        ("2023.2", "2023.2 make arm"),
    ] {
        let c = base_config("toolchain", version, "mpich", "generic", "none", "psmp");
        let content =
            generate_dockerfile_content(&c).expect("render toolchain template for {version}");
        assert!(
            !content.contains("DBCSRConfigVersion.cmake"),
            "T-W3: the {what} render must stay free of the dbcsr config-version patch"
        );
    }
}

// --------------------------------------------------------
// Wave 3: minimal OFF set for components absent from the
// toolchain --install-all install set
// --------------------------------------------------------
//
// e2e runs 38404980161 (toolchain_2026.2_generic, CPU) and 38405057080
// (cuda_2026.2_P100, GPU) both died at the SAME configure point: CP2K
// v2026.2 CMakeLists.txt:943 `find_package(openPMD 0.16.1 REQUIRED)`
// behind `CP2K_USE_OPENPMD` (CMakeLists.txt:174-175). The toolchain
// `--install-all` set (36 packages) ships no openPMD. The other toolchain
// "Packages not used" entries are safe under the rendered flags: dftd4 is
// found inside the tblite install dir (run log: `Found DFTD4: .../
// tblite-0.6.0/lib/libdftd4.a`), libgint is CUDA-gated on the cmake side
// (CP2K_USE_LIBGINT dependent option, CMakeLists.txt:241-244; the GPU
// toolchain installs it and the GPU run passed CMakeLists.txt:836),
// libxstream is OpenCL-only and cusolvermp defaults OFF. Hence the
// minimal configure-stage OFF set is exactly {CP2K_USE_OPENPMD}; master
// and the make arms stay untouched (pinned regression guards).

/// The 2026.2 release-cmake renders (CPU and GPU) must disable openPMD on
/// top of the existing DLAF/PEXSI/DEEPMD/LIBTORCH OFF surface, with the
/// new flag slotted into the shared base flag block (after LIBTORCH OFF,
/// before the IPO/LTO block) so the CUDA triple keeps its slot.
#[test]
fn t_wave3_2026_2_release_arm_disables_openpmd() {
    for cuda in ["none", "P100"] {
        let c = base_config("toolchain", "2026.2", "mpich", "generic", cuda, "psmp");
        let content =
            generate_dockerfile_content(&c).expect("render toolchain template for 2026.2");

        assert!(
            content.contains("-DCP2K_USE_OPENPMD=OFF"),
            "T-W3: 2026.2 (cuda={cuda}) release-cmake render must carry -DCP2K_USE_OPENPMD=OFF (e2e runs 38404980161/38405057080 died at CMakeLists.txt:943 find_package(openPMD 0.16.1 REQUIRED); the toolchain --install-all set ships no openPMD)"
        );

        // ordering: the new flag belongs to the shared base block — after
        // the existing LIBTORCH OFF and before the IPO/LTO block — so the
        // CUDA triple (GPU render) keeps its slot between the two.
        let libtorch_at = content
            .find("-DCP2K_USE_LIBTORCH=OFF")
            .expect("T-W3: release-cmake render must carry the LIBTORCH OFF flag");
        let openpmd_at = content
            .find("-DCP2K_USE_OPENPMD=OFF")
            .expect("T-W3: OPENPMD OFF flag located above must be a substring");
        let ipo_at = content
            .find("-DCMAKE_INTERPROCEDURAL_OPTIMIZATION=OFF")
            .expect("T-W3: release-cmake render must carry the IPO OFF flag");
        assert!(
            libtorch_at < openpmd_at && openpmd_at < ipo_at,
            "T-W3: 2026.2 (cuda={cuda}) OPENPMD OFF must sit in the shared base block (LIBTORCH at {libtorch_at}, OPENPMD at {openpmd_at}, IPO at {ipo_at})"
        );
    }
}

/// Master (cmake arm) and 2023.2 (make arm) must stay untouched: the
/// openPMD OFF is release-cmake-arm-only and both arms are byte-frozen by
/// earlier wave guards.
#[test]
fn t_wave3_master_and_make_arms_render_no_openpmd_off() {
    for (version, what) in [
        ("master", "master cmake arm"),
        ("2023.2", "2023.2 make arm"),
    ] {
        let c = base_config("toolchain", version, "mpich", "generic", "none", "psmp");
        let content =
            generate_dockerfile_content(&c).expect("render toolchain template for {version}");
        assert!(
            !content.contains("CP2K_USE_OPENPMD"),
            "T-W3: the {what} render must stay free of the openPMD OFF flag (pinned regression guard)"
        );
    }
}
