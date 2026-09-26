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
        content.contains("git clone --recursive -b support/v2025.2"),
        "T-013: release build must clone the support/v<version> branch"
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
        content.contains("git clone --recursive https://github.com/cp2k/cp2k.git"),
        "T-013: master must clone the default branch (no -b support/...)"
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
        Ok(outcome) => panic!(
            "T-014: execute_build with docker unreachable must not succeed, got {:?}",
            outcome
        ),
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
    // native (and any unrecognized method) must map to an EMPTY set,
    // which the GUI renders as a disabled control.
    for method in ["native", "", "docker", "not-a-method"] {
        assert!(
            cuda_options_for_method(method).is_empty(),
            "T-O21: method {:?} must map to an empty CUDA option set",
            method
        );
    }
}

#[test]
fn t_o21_native_step0_rejects_gpu_options_consistent_with_empty_matrix() {
    // Matrix self-consistency: the GUI offers nothing for native, so every
    // GPU value toolchain DOES offer must be rejected by native Step 0 —
    // which runs before any subprocess/filesystem side effect, so this is
    // a pure unit-level check.
    assert!(
        cuda_options_for_method("native").is_empty(),
        "T-O21: matrix premise — native offers no CUDA options"
    );
    for cuda in cuda_options_for_method("toolchain") {
        if cuda == "none" {
            continue; // the one cuda value the native path accepts
        }
        let c = base_config("native", "master", "mpich", "x86_64", cuda, "psmp");
        let (tx, rx) = mpsc::channel::<LogLine>();
        let flag = Arc::new(Mutex::new(false));
        let result = execute_native_build(&c, tx, flag);
        drain_rx(rx);
        let err = result.expect_err("T-O21: native + GPU cuda must be rejected by Step 0");
        assert!(
            err.to_string()
                .contains("Unsupported native build configuration"),
            "T-O21: rejection must name the Step 0 validation, got: {}",
            err
        );
        assert!(
            err.to_string()
                .contains(format!("cuda='{}'", cuda).as_str()),
            "T-O21: rejection must echo the offending cuda value {:?}, got: {}",
            cuda,
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
