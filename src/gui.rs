use crate::build;
use anyhow::Result;
use eframe::egui::{self, Color32, RichText, ScrollArea, Vec2};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex, PoisonError};

// ============================================================
// Tab enum
// ============================================================

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Build,
    Configs,
    System,
    Settings,
    About,
}

/// Payload of an in-GUI Dockerfile preview ("Generate Only", CLI
/// `--generate-only` parity): the file that `BuildConfig::resolve_dockerfile`
/// picked — bundled, custom `-f`, or freshly synthesized into
/// `%TEMP%/forge2k/` — plus its full content read back for display.
#[derive(Clone)]
struct DockerfilePreview {
    /// Resolved Dockerfile path, shown verbatim in the preview panel.
    path: PathBuf,
    /// Full file content, shown in a read-only code-style view.
    content: String,
    /// True when this action synthesized (and wrote) the file — the same
    /// write side effect as the CLI's `--generate-only`; false when an
    /// existing bundled/custom Dockerfile was resolved as-is.
    synthesized: bool,
}

/// Shared state of one System-tab probe (BuildKit / Docker storage): the
/// last result (`None` = never checked) and whether a probe is currently
/// running on its worker thread. The probes spawn `docker buildx version`,
/// `docker info` and PowerShell/`df` subprocesses that can take seconds —
/// they must never run on the UI thread (same reason `start_build` uses a
/// worker thread).
#[derive(Clone)]
struct ProbeState<T> {
    result: Option<T>,
    running: bool,
}

// Manual impl: the derive would require T: Default, but the default state
// is `None`/`false` regardless of the payload type.
impl<T> Default for ProbeState<T> {
    fn default() -> Self {
        Self {
            result: None,
            running: false,
        }
    }
}

// ============================================================
// Application State
// ============================================================

pub struct Forge2kApp {
    // Build config
    method: String,
    version: String,
    mpi: String,
    cpu: String,
    cuda: String,
    variant: String,
    jobs: String,
    tag: String,
    shm_size: String,

    // Advanced build options (CLI parity: --features / --no-cache /
    // -f dockerfile / --force)
    no_cache: bool,
    force: bool,
    /// Selected feature presets. Values only ever come from the
    /// build-side `build::KNOWN_FEATURES` registry (checkbox toggles and
    /// the load-time filter); the UI never hardcodes a feature name.
    features: Vec<String>,
    /// Raw draft of the custom Dockerfile path; empty = none. Stored
    /// verbatim in settings; existence is checked on edit-commit.
    dockerfile: String,
    /// Cached verdict of the last on-commit existence check of
    /// `dockerfile` (blur or Enter) — never recomputed per frame.
    dockerfile_missing: bool,

    // Build state
    build_logs: Arc<Mutex<Vec<build::LogLine>>>,
    is_building: Arc<Mutex<bool>>,
    cancel_flag: Arc<Mutex<bool>>,
    build_error: Arc<Mutex<Option<String>>>,
    build_status: Arc<Mutex<Option<build::BuildOutcome>>>,

    /// Result of the last "Generate Only" action (CLI `--generate-only`
    /// parity). `Some` keeps the floating preview window open — `Ok` shows
    /// the resolved/synthesized Dockerfile, `Err` its failure reason; `None`
    /// means closed/discarded.
    generate_preview: Option<Result<DockerfilePreview, String>>,

    // System state
    docker_status: String,
    docker_running: bool,
    docker_checked: bool,
    registry_status: String,
    current_mirror: String,

    // System-tab probes (BuildKit / Docker storage), shared with worker
    // threads via Arc<Mutex> exactly like the build state above.
    buildkit_probe: Arc<Mutex<ProbeState<build::BuildKitStatus>>>,
    storage_probe: Arc<Mutex<ProbeState<build::DockerStorageStatus>>>,

    /// "Restart Docker" worker state (CLI `mirror --restart` parity). The
    /// Windows arm of `build::restart_docker` sleeps and polls for up to a
    /// minute, so it runs on a worker thread through `run_probe`. Windows
    /// only: elsewhere it shells out to `sudo systemctl`, which a GUI
    /// process cannot run (no tty for the password prompt) — there the
    /// Settings tab only shows the manual command and needs no state.
    #[cfg(windows)]
    restart_probe: Arc<Mutex<ProbeState<Result<(), String>>>>,

    // Version list cache: one disk scan at startup or via the explicit
    // Refresh button — the frame loop never rescans dockerfiles/.
    configs_cache: Vec<build::ConfigInfo>,

    // Configs tab (CLI `list` parity): method/version filters mirroring
    // `list --method/--version`. `None` = All; method is only ever set to
    // spack/toolchain (the CLI's value_parser enum — native is not listed).
    browse_method: Option<String>,
    browse_version: Option<String>,
    /// Command line last copied via a Configs-tab Copy button, shown as
    /// click feedback next to the filter row.
    browse_copied: Option<String>,

    // Settings
    mirror_input: String,
    settings_message: String,

    // UI
    active_tab: Tab,
    log_auto_scroll: bool,
}

impl Default for Forge2kApp {
    fn default() -> Self {
        let current_mirror = build::get_registry_mirror().unwrap_or_default();
        // Settings persist across restarts at <home>/.forge2k/settings.json;
        // a missing/corrupt file silently degrades to the defaults below.
        let settings = crate::settings::ForgeSettings::load();
        // Ignore feature values outside the build-side registry: a
        // hand-edited settings.json cannot inject unknown presets into the
        // checkbox-driven selection.
        let features: Vec<String> = settings
            .features
            .into_iter()
            .filter(|f| build::KNOWN_FEATURES.contains(&f.as_str()))
            .collect();
        let dockerfile = settings.dockerfile;
        let dockerfile_missing = !dockerfile.is_empty() && !Path::new(&dockerfile).exists();
        Self {
            method: "spack".into(),
            version: "2026.1".into(),
            mpi: "mpich".into(),
            cpu: "x86_64".into(),
            cuda: "none".into(),
            variant: "psmp".into(),
            jobs: "0".into(),
            tag: String::new(),
            shm_size: "1g".into(),
            build_logs: Arc::new(Mutex::new(Vec::new())),
            is_building: Arc::new(Mutex::new(false)),
            cancel_flag: Arc::new(Mutex::new(false)),
            build_error: Arc::new(Mutex::new(None)),
            build_status: Arc::new(Mutex::new(None)),
            generate_preview: None,
            docker_status: "Not checked".into(),
            docker_running: false,
            docker_checked: false,
            registry_status: "Not checked".into(),
            current_mirror,
            buildkit_probe: Arc::new(Mutex::new(ProbeState::default())),
            storage_probe: Arc::new(Mutex::new(ProbeState::default())),
            #[cfg(windows)]
            restart_probe: Arc::new(Mutex::new(ProbeState::default())),
            configs_cache: build::list_available_configs(),
            browse_method: None,
            browse_version: None,
            browse_copied: None,
            mirror_input: settings.mirror_input,
            settings_message: String::new(),
            no_cache: settings.no_cache,
            force: settings.force,
            features,
            dockerfile,
            dockerfile_missing,
            active_tab: Tab::Build,
            log_auto_scroll: true,
        }
    }
}

impl Forge2kApp {
    /// Pure state → `BuildConfig` mapping (no `ui` access): the single
    /// place the Build tab's inputs become a `build::BuildConfig`, kept
    /// ui-free so it can be unit-tested without a GUI context. The only
    /// non-field logic is the CLI-parity auto-jobs resolution (`0` =
    /// available parallelism).
    fn collect_build_config(&self) -> build::BuildConfig {
        let num_jobs: u32 = self.jobs.parse().unwrap_or(0);
        let num_jobs = if num_jobs == 0 {
            std::thread::available_parallelism()
                .map(|n| n.get() as u32)
                .unwrap_or(4)
        } else {
            num_jobs
        };

        build::BuildConfig {
            method: self.method.clone(),
            version: self.version.clone(),
            mpi: self.mpi.clone(),
            cpu: self.cpu.clone(),
            cuda: self.cuda.clone(),
            variant: self.variant.clone(),
            jobs: num_jobs,
            tag: if self.tag.is_empty() {
                String::new()
            } else {
                self.tag.clone()
            },
            no_cache: self.no_cache,
            dockerfile: if self.dockerfile.is_empty() {
                None
            } else {
                Some(PathBuf::from(&self.dockerfile))
            },
            shm_size: self.shm_size.clone(),
            features: self.features.clone(),
        }
    }

    /// Pure validation of the advanced-option combination: non-empty =
    /// invalid combination that must never reach the build layer. Mirrors
    /// the CLI's up-front validation — the features/Dockerfile mutual
    /// exclusion enforced by `BuildConfig::resolve_dockerfile` and the arm
    /// rules enforced by `build::validate_feature_arm` — so START BUILD
    /// fails fast with a readable reason instead of surfacing a mid-build
    /// generator error.
    fn build_config_blockers(&self) -> Vec<String> {
        let mut blockers = Vec::new();
        if !self.features.is_empty() && !self.dockerfile.is_empty() {
            blockers.push(
                "Feature presets and a custom Dockerfile are mutually exclusive \
                 (feature presets only affect the synthesized Dockerfile): uncheck \
                 the feature(s) or clear the Dockerfile path."
                    .into(),
            );
        }
        if let Err(e) = build::validate_feature_arm(&self.method, &self.version, &self.features) {
            blockers.push(e.to_string());
        }
        blockers
    }

    fn start_build(&mut self) {
        if *self
            .is_building
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
        {
            return;
        }

        // Hard gate on invalid advanced-option combinations: refuse before
        // any build state is touched.
        let blockers = self.build_config_blockers();
        if !blockers.is_empty() {
            let mut logs = self
                .build_logs
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            for blocker in &blockers {
                logs.push(build::LogLine {
                    timestamp: "ERROR".into(),
                    text: blocker.clone(),
                    level: build::LogLevel::Error,
                });
            }
            return;
        }

        // Clear previous logs
        self.build_logs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        *self
            .build_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
        *self
            .cancel_flag
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = false;

        let config = self.collect_build_config();

        let (tx, rx) = mpsc::channel::<build::LogLine>();

        let logs = self.build_logs.clone();
        let is_building = self.is_building.clone();
        let cancel = self.cancel_flag.clone();
        let error = self.build_error.clone();
        let status = self.build_status.clone();

        *is_building.lock().unwrap_or_else(PoisonError::into_inner) = true;
        *status.lock().unwrap_or_else(PoisonError::into_inner) = None;

        // Spawn log collector thread
        std::thread::spawn(move || {
            for line in rx {
                logs.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(line);
            }
        });

        // Spawn build thread
        std::thread::spawn(move || {
            // Route by build method: native builds run directly on the host
            // (execute_native_build), everything else goes through Docker
            // (execute_build with a bundled or synthesized Dockerfile).
            let result = match config.method.as_str() {
                "native" => build::execute_native_build(&config, tx.clone(), cancel),
                _ => build::execute_build(&config, tx, cancel),
            };
            match result {
                Ok(build::BuildOutcome::Success) => {
                    *status.lock().unwrap_or_else(PoisonError::into_inner) =
                        Some(build::BuildOutcome::Success);
                }
                Ok(build::BuildOutcome::Failed) => {
                    *status.lock().unwrap_or_else(PoisonError::into_inner) =
                        Some(build::BuildOutcome::Failed);
                    *error.lock().unwrap_or_else(PoisonError::into_inner) =
                        Some("Build failed. Check the build output for details.".into());
                }
                Ok(build::BuildOutcome::Cancelled) => {
                    *status.lock().unwrap_or_else(PoisonError::into_inner) =
                        Some(build::BuildOutcome::Cancelled);
                }
                Err(e) => {
                    *status.lock().unwrap_or_else(PoisonError::into_inner) =
                        Some(build::BuildOutcome::Failed);
                    *error.lock().unwrap_or_else(PoisonError::into_inner) = Some(e.to_string());
                }
            }
            *is_building.lock().unwrap_or_else(PoisonError::into_inner) = false;
        });
    }

    fn cancel_build(&mut self) {
        *self
            .cancel_flag
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = true;
    }

    /// CLI `--generate-only` parity: resolve or synthesize the Dockerfile for
    /// the current Build-tab configuration and open the preview window with
    /// its content and path. No build is started and Docker is not probed —
    /// the CLI skips its Docker gate for `--generate-only`, so this stays
    /// usable while Docker is down.
    ///
    /// The hard gate is START BUILD's: `build_config_blockers` refuses
    /// invalid advanced-option combinations before any synthesis side
    /// effect. Every failure (illegal combination, generator error,
    /// unreadable file) lands as ERROR lines in the build log AND inside
    /// the preview panel.
    ///
    /// Runs on the UI thread on purpose: the work is one in-memory template
    /// render plus one small (few KiB) file write + read, far below a
    /// noticeable frame hitch — a worker thread would only add state
    /// plumbing for no perceived-latency gain.
    fn generate_only(&mut self) {
        // START BUILD's hard gate, verbatim: refuse invalid combinations
        // before anything is synthesized.
        let blockers = self.build_config_blockers();
        if !blockers.is_empty() {
            let mut logs = self
                .build_logs
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            for blocker in &blockers {
                logs.push(build::LogLine {
                    timestamp: "ERROR".into(),
                    text: blocker.clone(),
                    level: build::LogLevel::Error,
                });
            }
            self.generate_preview = Some(Err(blockers.join("\n")));
            return;
        }

        let config = self.collect_build_config();
        let result = config.resolve_dockerfile().and_then(|path| {
            // The generator writes synthesized files into %TEMP%/forge2k/;
            // anything else is a bundled or custom `-f` Dockerfile that was
            // resolved as-is (no file written by this action).
            let synthesized = path.starts_with(std::env::temp_dir().join("forge2k"));
            let content = std::fs::read_to_string(&path).map_err(|e| {
                anyhow::anyhow!(
                    "Failed to read the resolved Dockerfile {}: {e}",
                    path.display()
                )
            })?;
            Ok(DockerfilePreview {
                path,
                content,
                synthesized,
            })
        });

        match result {
            Ok(preview) => {
                // CLI parity message ("Generated Dockerfile: <path>") in the
                // build log area, with the same timestamp format as the
                // build engine's log lines.
                self.build_logs
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(build::LogLine {
                        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                        text: format!("Generated Dockerfile: {}", preview.path.display()),
                        level: build::LogLevel::Info,
                    });
                self.generate_preview = Some(Ok(preview));
            }
            Err(e) => {
                self.build_logs
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(build::LogLine {
                        timestamp: "ERROR".into(),
                        text: e.to_string(),
                        level: build::LogLevel::Error,
                    });
                self.generate_preview = Some(Err(e.to_string()));
            }
        }
    }

    fn check_docker(&mut self) {
        match build::check_docker() {
            build::DockerStatus::Installed { version, running } => {
                self.docker_status = version;
                self.docker_running = running;
            }
            build::DockerStatus::NotInstalled => {
                self.docker_status = "Not installed".into();
                self.docker_running = false;
            }
            build::DockerStatus::NotRunning => {
                self.docker_status = "Installed (not running)".into();
                self.docker_running = false;
            }
        }
        self.docker_checked = true;
    }

    fn check_registry(&mut self) {
        match build::check_registry() {
            build::NetworkStatus::Good => {
                self.registry_status = "✅ Fast".into();
            }
            build::NetworkStatus::Slow(t) => {
                self.registry_status = format!("⚠️ Slow ({})", t);
            }
            build::NetworkStatus::Blocked(reason) => {
                self.registry_status = format!("❌ Blocked - {}", reason);
                if self.current_mirror.is_empty() {
                    self.registry_status
                        .push_str("\n   Try: Settings → Auto-detect Mirror");
                }
            }
            build::NetworkStatus::Unknown(reason) => {
                self.registry_status = format!("❓ {}", reason);
            }
        }
    }

    /// Launch `probe` on a worker thread and store its result in
    /// `probe_state` when done. A no-op while that probe is already
    /// running, so double-clicks and repaint races are absorbed; the UI
    /// thread only ever touches the mutex, never the subprocesses.
    fn run_probe<T, F>(&self, probe_state: &Arc<Mutex<ProbeState<T>>>, probe: F)
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        {
            let mut state = probe_state.lock().unwrap_or_else(PoisonError::into_inner);
            if state.running {
                return;
            }
            state.running = true;
        }
        let shared = Arc::clone(probe_state);
        std::thread::spawn(move || {
            let result = probe();
            let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
            state.result = Some(result);
            state.running = false;
        });
    }

    /// Persist the current settings to `<home>/.forge2k/settings.json`.
    ///
    /// Called only from user-commit points in the Settings tab (edit
    /// committed via `changed()`, Apply, auto-detect, known-mirror "Use")
    /// and the Build tab's advanced options (checkbox toggles, Dockerfile
    /// edit committed via `changed()` + `lost_focus()`) — never from the
    /// frame loop — so the file is written on real changes only. Failures
    /// degrade to a red settings message instead of panicking.
    fn persist_settings(&mut self) {
        let settings = crate::settings::ForgeSettings {
            mirror_input: self.mirror_input.clone(),
            no_cache: self.no_cache,
            force: self.force,
            features: self.features.clone(),
            dockerfile: self.dockerfile.clone(),
        };
        if let Err(e) = settings.save() {
            self.settings_message = format!("❌ Failed to save settings: {:#}", e);
        }
    }
}

impl eframe::App for Forge2kApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Keep repainting during build and while System-tab probes run on
        // their worker threads (buildx/PowerShell/df can take seconds), and
        // while the Settings tab's Docker restart runs on its worker thread
        // (the Windows restart polls for up to 60s).
        let probes_running = {
            let buildkit = self
                .buildkit_probe
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let storage = self
                .storage_probe
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            #[cfg(windows)]
            let restarting = self
                .restart_probe
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .running;
            #[cfg(not(windows))]
            let restarting = false;
            buildkit.running || storage.running || restarting
        };
        if *self
            .is_building
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            || probes_running
        {
            ctx.request_repaint();
        }

        // ===== Custom style =====
        let mut style = (*ctx.style()).clone();
        style.visuals.dark_mode = true;
        style.visuals.override_text_color = Some(Color32::from_rgb(220, 220, 220));
        style.visuals.window_fill = Color32::from_rgb(18, 18, 24);
        style.visuals.panel_fill = Color32::from_rgb(24, 24, 32);
        style.visuals.faint_bg_color = Color32::from_rgb(30, 30, 40);
        style.visuals.extreme_bg_color = Color32::from_rgb(12, 12, 18);
        style.visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(30, 30, 42);
        style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(35, 35, 48);
        style.visuals.widgets.active.bg_fill = Color32::from_rgb(50, 50, 70);
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(45, 45, 60);
        style.visuals.selection.bg_fill = Color32::from_rgb(60, 80, 120);
        style.visuals.hyperlink_color = Color32::from_rgb(100, 180, 255);
        ctx.set_style(style);

        // ===== Top Panel =====
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.label(
                    RichText::new("🔥 Forge2K")
                        .size(22.0)
                        .color(Color32::from_rgb(255, 200, 50))
                        .strong(),
                );
                ui.label(
                    RichText::new("v1.0.1")
                        .size(12.0)
                        .color(Color32::from_rgb(120, 120, 140)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(12.0);
                    if *self
                        .is_building
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                    {
                        ui.label(
                            RichText::new("● BUILDING")
                                .size(11.0)
                                .color(Color32::from_rgb(255, 180, 50)),
                        );
                    }
                    let status_color = if self.docker_running {
                        Color32::from_rgb(80, 220, 100)
                    } else {
                        Color32::from_rgb(200, 80, 80)
                    };
                    ui.label(
                        RichText::new(if self.docker_running {
                            "● Docker OK"
                        } else {
                            "○ Docker"
                        })
                        .size(11.0)
                        .color(status_color),
                    );
                });
            });
        });

        // ===== Main Content =====
        egui::CentralPanel::default().show(ctx, |ui| {
            // Tab bar
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                let tabs = [
                    ("🛠 Build", Tab::Build),
                    ("📋 Configs", Tab::Configs),
                    ("📊 System", Tab::System),
                    ("⚙ Settings", Tab::Settings),
                    ("ℹ About", Tab::About),
                ];
                for (label, tab) in &tabs {
                    let is_active = self.active_tab == *tab;
                    let btn =
                        egui::Button::new(RichText::new(*label).size(14.0).color(if is_active {
                            Color32::from_rgb(255, 200, 50)
                        } else {
                            Color32::from_rgb(160, 160, 180)
                        }))
                        .fill(if is_active {
                            Color32::from_rgb(40, 40, 55)
                        } else {
                            Color32::TRANSPARENT
                        })
                        .min_size(Vec2::new(90.0, 28.0));
                    if ui.add(btn).clicked() {
                        self.active_tab = *tab;
                    }
                }
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            match self.active_tab {
                Tab::Build => self.render_build_tab(ui, ctx),
                Tab::Configs => self.render_configs_tab(ui),
                Tab::System => self.render_system_tab(ui),
                Tab::Settings => self.render_settings_tab(ui),
                Tab::About => self.render_about_tab(ui),
            }
        });
    }
}

// ============================================================
// Tab Renderers
// ============================================================

impl Forge2kApp {
    fn render_build_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let is_building_val = *self
            .is_building
            .lock()
            .unwrap_or_else(PoisonError::into_inner);

        ui.horizontal(|ui| {
            // ─── LEFT: Configuration Panel ───
            egui::SidePanel::left("config_panel")
                .resizable(false)
                .default_width(280.0)
                .min_width(240.0)
                .show_inside(ui, |ui| {
                    ScrollArea::vertical().show(ui, |ui| {
                        self.render_build_config_fields(ui);
                        self.render_advanced_options(ui);
                        self.render_build_controls(ui, is_building_val);
                    });
                });

            // ─── RIGHT: Build Log ───
            egui::CentralPanel::default().show_inside(ui, |ui| {
                self.render_build_log_panel(ui, is_building_val);
            });
        });

        // Floating preview window (opened by "Generate Only"); a no-op while
        // `generate_preview` is None.
        self.render_dockerfile_preview(ctx);
    }

    /// Floating preview window for the last "Generate Only" action:
    /// scrolling read-only Dockerfile text in code style, the resolved path
    /// (with the `%TEMP%/forge2k/` write side effect called out for
    /// synthesized files) and an explicit Close button. Failures (illegal
    /// combinations, generator errors) render here too, next to the button
    /// that triggered them. Closable via the title-bar X or the Close
    /// button; either sets `generate_preview = None`.
    fn render_dockerfile_preview(&mut self, ctx: &egui::Context) {
        // Clone out so the closure can take `&mut self` for the Close path.
        let Some(preview) = self.generate_preview.clone() else {
            return;
        };
        let mut open = true;
        egui::Window::new(
            RichText::new("📄 Dockerfile 预览 / Dockerfile Preview")
                .size(14.0)
                .color(Color32::from_rgb(255, 200, 50))
                .strong(),
        )
        .open(&mut open)
        .default_size(Vec2::new(680.0, 480.0))
        .resizable(true)
        .collapsible(false)
        .show(ctx, |ui| {
            match &preview {
                Ok(p) => {
                    ui.label(
                        RichText::new(if p.synthesized {
                            format!(
                                "Synthesized & written (same side effect as CLI --generate-only): {}",
                                p.path.display()
                            )
                        } else {
                            format!(
                                "Resolved bundled/custom Dockerfile (nothing written): {}",
                                p.path.display()
                            )
                        })
                        .size(11.0)
                        .family(egui::FontFamily::Monospace)
                        .color(Color32::from_rgb(100, 180, 255)),
                    );
                    ui.add_space(4.0);
                    let frame = egui::Frame::dark_canvas(ui.style())
                        .fill(Color32::from_rgb(10, 10, 16));
                    frame.show(ui, |ui| {
                        ScrollArea::vertical().show(ui, |ui| {
                            ui.add_space(4.0);
                            // `&mut &str` buffer + code_editor(): read-only
                            // monospace view, exactly what the requirement
                            // asks for (display, not editing).
                            ui.add(
                                egui::TextEdit::multiline(&mut p.content.as_str())
                                    .code_editor()
                                    .desired_width(ui.available_width()),
                            );
                        });
                    });
                }
                Err(reason) => {
                    ui.label(
                        RichText::new(format!("❌ Generation failed:\n{reason}"))
                            .size(12.0)
                            .color(Color32::from_rgb(255, 100, 100)),
                    );
                }
            }
            ui.add_space(6.0);
            if ui.button("Close").clicked() {
                self.generate_preview = None;
            }
        });
        if !open {
            self.generate_preview = None;
        }
    }

    /// Configuration area of the Build tab: method/version/mpi/cpu/cuda/
    /// variant combos and jobs/shm/tag inputs.
    fn render_build_config_fields(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.heading(
            RichText::new("Build Configuration")
                .size(15.0)
                .color(Color32::from_rgb(255, 200, 50)),
        );
        ui.add_space(8.0);

        // Method
        ui.label(
            RichText::new("Build Method")
                .size(12.0)
                .color(Color32::from_rgb(150, 150, 170)),
        );
        egui::ComboBox::from_id_salt("method")
            .selected_text(&self.method)
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.method,
                    "spack".into(),
                    "Spack (Docker, Ubuntu 24.04)",
                );
                ui.selectable_value(
                    &mut self.method,
                    "toolchain".into(),
                    "Toolchain (Docker, Ubuntu 22.04)",
                );
                ui.selectable_value(&mut self.method, "native".into(), "Native (direct on host)");
            });
        ui.add_space(6.0);

        // Version (same source as the CLI: build::list_available_configs,
        // cached in configs_cache and re-scanned only via the Refresh
        // button; native builds are host-local and keep their own list)
        ui.label(
            RichText::new("CP2K Version")
                .size(12.0)
                .color(Color32::from_rgb(150, 150, 170)),
        );
        let docker_configs: Vec<&build::ConfigInfo> = if self.method == "native" {
            Vec::new()
        } else {
            self.configs_cache
                .iter()
                .filter(|c| c.method == self.method)
                .collect()
        };
        let versions: Vec<String> = if self.method == "native" {
            native_version_list()
        } else {
            let mut vs: Vec<String> = docker_configs.iter().map(|c| c.version.clone()).collect();
            vs.sort();
            vs.dedup();
            vs
        };
        let selected_is_synthesized = self.method != "native"
            && docker_configs.iter().any(|c| {
                c.version == self.version && c.source == build::DockerfileSource::Synthesized
            });
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("version")
                .selected_text(&self.version)
                .show_ui(ui, |ui| {
                    for v in &versions {
                        ui.selectable_value(&mut self.version, v.clone(), v);
                    }
                });
            // Manual re-scan: the only place the frame loop touches the
            // dockerfiles/ directory again.
            if ui.button("⟳ Refresh").clicked() {
                self.configs_cache = build::list_available_configs();
            }
        });
        if selected_is_synthesized {
            ui.label(
                RichText::new("synthesized: no bundled Dockerfile, generated at build time")
                    .size(10.0)
                    .color(Color32::from_rgb(255, 200, 80)),
            );
        }
        if self.method == "native" {
            // native_version_list() deliberately leaves out 2026.2 (see its
            // doc comment): surface that instead of letting the user wonder
            // why the picker is narrower than the CLI --version string.
            ui.label(
                RichText::new("2026.2 has no verified native build matrix yet")
                    .size(10.0)
                    .color(Color32::from_rgb(150, 150, 170)),
            );
        }
        ui.add_space(6.0);

        // MPI
        ui.label(
            RichText::new("MPI Implementation")
                .size(12.0)
                .color(Color32::from_rgb(150, 150, 170)),
        );
        let mpis = match self.method.as_str() {
            "toolchain" => vec!["mpich"],
            _ => vec!["mpich", "openmpi"],
        };
        // Stale-value fallback, same pattern as CUDA/CPU below: switching to
        // toolchain narrows the set to mpich, so an openmpi selection carried
        // over from spack must not leak into the build config.
        if !mpis.iter().any(|m| *m == self.mpi) {
            self.mpi = "mpich".into();
        }
        egui::ComboBox::from_id_salt("mpi")
            .selected_text(&self.mpi)
            .show_ui(ui, |ui| {
                for m in &mpis {
                    ui.selectable_value(&mut self.mpi, m.to_string(), *m);
                }
            });
        ui.add_space(6.0);

        // CPU Target
        ui.label(
            RichText::new("CPU Target")
                .size(12.0)
                .color(Color32::from_rgb(150, 150, 170)),
        );
        let cpus: Vec<&str> = match self.method.as_str() {
            "toolchain" => vec!["generic"],
            // Native Step 0 whitelist (validate_native_config,
            // build/execute.rs): cpu in {'generic', 'x86_64'} — 'x86_64' is
            // translated to 'generic' for the toolchain script, every other
            // target is rejected outright, so the picker must not offer it.
            "native" => vec!["x86_64", "generic"],
            _ => vec![
                "x86_64",
                "cascadelake",
                "haswell",
                "skylake-avx512",
                "generic",
            ],
        };
        if !cpus.iter().any(|c| *c == self.cpu) {
            // Method switched to one that no longer offers the current
            // target (e.g. x86_64 on toolchain, cascadelake on native):
            // fall back to 'generic', the one target valid across all three
            // methods — mirroring the CUDA reset below. Without this, the
            // stale selection would ride collect_build_config into the
            // build (the synthesized toolchain script rejects a literal
            // --target-cpu=x86_64, and native Step 0 rejects the rest).
            self.cpu = "generic".into();
        }
        egui::ComboBox::from_id_salt("cpu")
            .selected_text(&self.cpu)
            .show_ui(ui, |ui| {
                for c in &cpus {
                    ui.selectable_value(&mut self.cpu, c.to_string(), *c);
                }
            });
        ui.add_space(6.0);

        // CUDA (options come from the build-side support matrix so the GUI
        // can never offer a combination the executor would reject: spack is
        // CPU-only, toolchain/native carry the bundled none/P100/V100 set —
        // native + master stays a build-time Step 0 rejection; an empty set
        // — only unknown methods — renders the control disabled)
        ui.label(
            RichText::new("CUDA Support")
                .size(12.0)
                .color(Color32::from_rgb(150, 150, 170)),
        );
        let cuda_options = build::cuda_options_for_method(&self.method);
        if !cuda_options.iter().any(|c| *c == self.cuda) {
            // Method changed to one that no longer supports the current
            // selection: fall back to the always-valid 'none'.
            self.cuda = "none".into();
        }
        ui.scope(|ui| {
            if cuda_options.is_empty() {
                ui.disable();
            }
            egui::ComboBox::from_id_salt("cuda")
                .selected_text(&self.cuda)
                .show_ui(ui, |ui| {
                    for c in &cuda_options {
                        ui.selectable_value(&mut self.cuda, c.to_string(), *c);
                    }
                });
        });
        if cuda_options.is_empty() {
            ui.label(
                RichText::new("CUDA is not supported by this build method")
                    .size(10.0)
                    .color(Color32::from_rgb(150, 150, 170)),
            );
        }
        ui.add_space(6.0);

        // Variant
        ui.label(
            RichText::new("Binary Variant")
                .size(12.0)
                .color(Color32::from_rgb(150, 150, 170)),
        );
        egui::ComboBox::from_id_salt("variant")
            .selected_text(&self.variant)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.variant, "psmp".into(), "psmp (MPI+OpenMP)");
                ui.selectable_value(&mut self.variant, "ssmp".into(), "ssmp (OpenMP only)");
                ui.selectable_value(&mut self.variant, "pdbg".into(), "pdbg (MPI+OpenMP debug)");
                ui.selectable_value(&mut self.variant, "sdbg".into(), "sdbg (OpenMP debug)");
            });
        ui.add_space(6.0);

        // Jobs
        ui.label(
            RichText::new("Parallel Jobs (0 = auto)")
                .size(12.0)
                .color(Color32::from_rgb(150, 150, 170)),
        );
        ui.add(egui::TextEdit::singleline(&mut self.jobs).desired_width(80.0));
        ui.add_space(6.0);

        // SHM Size
        ui.label(
            RichText::new("Shared Memory Size")
                .size(12.0)
                .color(Color32::from_rgb(150, 150, 170)),
        );
        ui.add(egui::TextEdit::singleline(&mut self.shm_size).desired_width(80.0));
        ui.add_space(6.0);

        // Custom tag
        ui.label(
            RichText::new("Custom Image Tag (optional)")
                .size(12.0)
                .color(Color32::from_rgb(150, 150, 170)),
        );
        ui.add(
            egui::TextEdit::singleline(&mut self.tag)
                .desired_width(200.0)
                .hint_text("auto-generate"),
        );
        ui.add_space(12.0);
    }

    /// Advanced options of the Build tab (CLI parity): feature presets
    /// (`--features`), no-cache (`--no-cache`), a custom Dockerfile
    /// (`-f/--dockerfile`) and force (`--force`), in one collapsible
    /// section.
    ///
    /// Registry-driven: the feature checkboxes iterate
    /// `build::KNOWN_FEATURES` and their enable/disable state comes from
    /// `build::features_supported_arm`, so this UI layer hardcodes neither
    /// a feature name nor an arm rule. Every control persists on its
    /// commit point via `persist_settings`.
    fn render_advanced_options(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(
            RichText::new("高级选项 / Advanced Options")
                .size(14.0)
                .color(Color32::from_rgb(255, 200, 50)),
        )
        .default_open(false)
        .show(ui, |ui| {
            // ─── Feature presets (registry-driven, arm-linked) ───
            ui.label(
                RichText::new("Feature Presets (--features)")
                    .size(12.0)
                    .color(Color32::from_rgb(150, 150, 170)),
            );
            let features_supported = build::features_supported_arm(&self.method, &self.version);
            ui.scope(|ui| {
                if !features_supported {
                    ui.disable();
                }
                for feature in build::KNOWN_FEATURES {
                    let mut checked = self.features.iter().any(|selected| selected == feature);
                    if ui.checkbox(&mut checked, feature).changed() {
                        if checked {
                            self.features.push(feature.to_string());
                        } else {
                            self.features.retain(|selected| selected != feature);
                        }
                        self.persist_settings();
                    }
                }
            });
            if !features_supported {
                // Live reason for the grayed-out checkboxes (CUDA-dropdown
                // precedent: a helper line instead of a mid-build error).
                let reason = if self.method == "native" {
                    "Feature presets are not supported by the native method (host build)"
                } else if self.method == "spack" {
                    "Feature presets require the toolchain method"
                } else if self.version == "master" {
                    "Feature presets are not available for master (its synthesized build step is frozen)"
                } else {
                    "Feature presets require CP2K v2026.1+ (release-cmake build arm)"
                };
                ui.label(
                    RichText::new(reason)
                        .size(10.0)
                        .color(Color32::from_rgb(150, 150, 170)),
                );
            }
            ui.add_space(6.0);

            // ─── Custom Dockerfile (mutually exclusive with features) ───
            ui.label(
                RichText::new("Custom Dockerfile (optional, -f)")
                    .size(12.0)
                    .color(Color32::from_rgb(150, 150, 170)),
            );
            let dockerfile_edit = ui.add(
                egui::TextEdit::singleline(&mut self.dockerfile)
                    .desired_width(200.0)
                    .hint_text("empty = bundled / auto-generated"),
            );
            // Existence is checked on edit-commit (blur / Enter), never
            // per frame; the verdict is cached in `dockerfile_missing`.
            if dockerfile_edit.changed() && dockerfile_edit.lost_focus() {
                self.dockerfile_missing =
                    !self.dockerfile.is_empty() && !Path::new(&self.dockerfile).exists();
                self.persist_settings();
            }
            if self.dockerfile_missing {
                ui.label(
                    RichText::new(format!("❌ File not found: {}", self.dockerfile))
                        .size(10.0)
                        .color(Color32::from_rgb(255, 100, 100)),
                );
            }
            // Live mutual-exclusion hint; start_build gates on the same
            // rule, so the combination can never reach the build layer.
            if !self.features.is_empty() && !self.dockerfile.is_empty() {
                ui.label(
                    RichText::new(
                        "❌ Feature presets and a custom Dockerfile are mutually \
                         exclusive — uncheck the feature(s) or clear the path.",
                    )
                    .size(10.0)
                    .color(Color32::from_rgb(255, 100, 100)),
                );
            }
            ui.add_space(6.0);

            // ─── Toggles ───
            if ui
                .checkbox(&mut self.no_cache, "No cache (--no-cache)")
                .changed()
            {
                self.persist_settings();
            }
            if ui
                .checkbox(
                    &mut self.force,
                    "Force build even if Docker is not running (--force)",
                )
                .changed()
            {
                self.persist_settings();
            }
            ui.add_space(12.0);
        });
    }

    /// Control area of the Build tab: start/cancel button, outcome status
    /// bar and build error display.
    fn render_build_controls(&mut self, ui: &mut egui::Ui, is_building_val: bool) {
        // Build button
        ui.add_space(4.0);
        if is_building_val {
            let cancel_btn = egui::Button::new(
                RichText::new("⛔ CANCEL BUILD")
                    .size(14.0)
                    .color(Color32::from_rgb(255, 100, 100)),
            )
            .fill(Color32::from_rgb(60, 20, 20))
            .min_size(Vec2::new(240.0, 36.0));
            if ui.add(cancel_btn).clicked() {
                self.cancel_build();
            }
        } else {
            let build_btn = egui::Button::new(
                RichText::new("🔥 START BUILD")
                    .size(14.0)
                    .color(Color32::from_rgb(20, 20, 30)),
            )
            .fill(Color32::from_rgb(255, 200, 50))
            .min_size(Vec2::new(240.0, 36.0));
            if ui.add(build_btn).clicked() {
                self.check_docker();
                // --force parity: a forced build skips the Docker
                // pre-flight refusal (the build itself may still fail if
                // Docker is actually needed). Invalid advanced-option
                // combinations are refused inside start_build either way.
                if self.docker_running || self.force {
                    self.start_build();
                } else {
                    // Will show error in log
                    let logs = self.build_logs.clone();
                    std::thread::spawn(move || {
                        let mut l = logs.lock().unwrap_or_else(PoisonError::into_inner);
                        l.push(build::LogLine {
                            timestamp: "ERROR".into(),
                            text: "Docker is not running. Please start Docker first.".into(),
                            level: build::LogLevel::Error,
                        });
                    });
                }
            }
        }

        // "Generate Only" (CLI --generate-only parity): resolve/synthesize
        // the Dockerfile and preview it in-app without starting a build or
        // requiring Docker. Disabled while a build is running (same hard
        // gate and config snapshot as START BUILD).
        ui.add_space(4.0);
        let gen_btn = egui::Button::new(RichText::new("📄 Generate Only").size(12.0).color(
            if is_building_val {
                Color32::from_rgb(120, 120, 140)
            } else {
                Color32::from_rgb(160, 200, 255)
            },
        ))
        .fill(if is_building_val {
            Color32::from_rgb(30, 30, 40)
        } else {
            Color32::from_rgb(40, 40, 60)
        })
        .min_size(Vec2::new(240.0, 26.0));
        if ui.add_enabled(!is_building_val, gen_btn).clicked() {
            self.generate_only();
        }

        // Build result status bar (outcome-driven)
        if let Some(outcome) = &*self
            .build_status
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
        {
            ui.add_space(8.0);
            let (icon, text, color) = match outcome {
                build::BuildOutcome::Success => (
                    "✅",
                    "Build completed successfully.",
                    Color32::from_rgb(100, 220, 120),
                ),
                build::BuildOutcome::Failed => {
                    ("❌", "Build failed.", Color32::from_rgb(255, 100, 100))
                }
                build::BuildOutcome::Cancelled => {
                    ("⏹", "Build cancelled.", Color32::from_rgb(255, 200, 80))
                }
            };
            ui.label(
                RichText::new(format!("{} {}", icon, text))
                    .color(color)
                    .size(12.0),
            );
        }

        // Build error display
        if let Some(err) = &*self
            .build_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
        {
            ui.add_space(8.0);
            ui.label(
                RichText::new(format!("❌ {}", err))
                    .color(Color32::from_rgb(255, 100, 100))
                    .size(11.0),
            );
        }
    }

    /// Log area of the Build tab: output header with auto-scroll/clear
    /// controls and the scrolling log view.
    fn render_build_log_panel(&mut self, ui: &mut egui::Ui, is_building_val: bool) {
        ui.horizontal(|ui| {
            ui.heading(
                RichText::new("Build Output")
                    .size(15.0)
                    .color(Color32::from_rgb(255, 200, 50)),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.checkbox(&mut self.log_auto_scroll, "Auto-scroll");
                if ui.button("Clear").clicked() {
                    self.build_logs
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .clear();
                }
            });
        });
        ui.add_space(4.0);

        let logs = self
            .build_logs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let frame = egui::Frame::dark_canvas(ui.style()).fill(Color32::from_rgb(10, 10, 16));
        frame.show(ui, |ui| {
            ScrollArea::vertical()
                .stick_to_bottom(self.log_auto_scroll && is_building_val)
                .show(ui, |ui| {
                    ui.add_space(4.0);
                    if logs.is_empty() {
                        ui.label(
                            RichText::new("  ⚡ Configure your build and click START BUILD")
                                .size(13.0)
                                .color(Color32::from_rgb(80, 80, 100)),
                        );
                        ui.label(
                            RichText::new("  Build logs will appear here in real-time.")
                                .size(12.0)
                                .color(Color32::from_rgb(60, 60, 80)),
                        );
                    } else {
                        for line in &logs {
                            // Level-driven severity: Error → red, Warn →
                            // yellow. No current producer emits Warn, so the
                            // rendered split is unchanged from the old
                            // `is_error` two-state coloring; the legacy
                            // emoji-prefix heuristics below are kept for
                            // Info lines to preserve the exact pre-existing
                            // three-color appearance.
                            let color = if line.level == build::LogLevel::Error {
                                Color32::from_rgb(255, 100, 100)
                            } else if line.level == build::LogLevel::Warn {
                                Color32::from_rgb(255, 200, 80)
                            } else if line.text.starts_with("✅")
                                || line.text.starts_with("🚀")
                                || line.text.starts_with("📋")
                            {
                                Color32::from_rgb(100, 220, 130)
                            } else if line.text.starts_with("⚠") || line.text.starts_with("⏹") {
                                Color32::from_rgb(255, 200, 80)
                            } else {
                                Color32::from_rgb(180, 185, 195)
                            };
                            ui.label(
                                RichText::new(format!("  {}", line.text))
                                    .size(12.0)
                                    .family(egui::FontFamily::Monospace)
                                    .color(color),
                            );
                        }
                    }
                    ui.add_space(4.0);
                });
        });
    }

    fn render_system_tab(&mut self, ui: &mut egui::Ui) {
        ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(8.0);

            // Docker Status Card
            ui.heading(
                RichText::new("🐳 Docker Engine")
                    .size(16.0)
                    .color(Color32::from_rgb(255, 200, 50)),
            );
            ui.add_space(6.0);

            let docker_card = egui::Frame::group(ui.style())
                .fill(Color32::from_rgb(28, 28, 38))
                .corner_radius(8.0);
            docker_card.show(ui, |ui| {
                ui.horizontal(|ui| {
                    if self.docker_checked {
                        if self.docker_running {
                            ui.label(
                                RichText::new("●")
                                    .color(Color32::from_rgb(80, 220, 100))
                                    .size(20.0),
                            );
                            ui.label(
                                RichText::new("Running")
                                    .color(Color32::from_rgb(80, 220, 100))
                                    .strong(),
                            );
                        } else {
                            ui.label(
                                RichText::new("●")
                                    .color(Color32::from_rgb(220, 80, 80))
                                    .size(20.0),
                            );
                            ui.label(
                                RichText::new("Not Running")
                                    .color(Color32::from_rgb(220, 80, 80))
                                    .strong(),
                            );
                        }
                    } else {
                        ui.label(
                            RichText::new("○")
                                .color(Color32::from_rgb(120, 120, 140))
                                .size(20.0),
                        );
                        ui.label("Not checked");
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Check Docker").clicked() {
                            self.check_docker();
                        }
                    });
                });
                ui.add_space(4.0);
                if self.docker_checked {
                    ui.label(format!("Version: {}", self.docker_status));
                }
            });

            ui.add_space(16.0);

            // BuildKit (buildx) Card — CLI check parity (`docker buildx version`)
            ui.heading(
                RichText::new("🧱 Docker BuildKit")
                    .size(16.0)
                    .color(Color32::from_rgb(255, 200, 50)),
            );
            ui.add_space(6.0);

            let buildkit_card = egui::Frame::group(ui.style())
                .fill(Color32::from_rgb(28, 28, 38))
                .corner_radius(8.0);
            buildkit_card.show(ui, |ui| {
                // Snapshot under a short lock; the worker thread keeps
                // writing the original behind the mutex.
                let snap = self
                    .buildkit_probe
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone();
                let (dot, dot_color, state_label) = if snap.running {
                    (
                        "●",
                        Color32::from_rgb(255, 200, 50),
                        RichText::new("Checking…").strong(),
                    )
                } else {
                    match &snap.result {
                        Some(build::BuildKitStatus::Available { .. }) => (
                            "●",
                            Color32::from_rgb(80, 220, 100),
                            RichText::new("Installed").color(Color32::from_rgb(80, 220, 100)).strong(),
                        ),
                        Some(build::BuildKitStatus::NotDetected) => (
                            "●",
                            Color32::from_rgb(220, 80, 80),
                            RichText::new("Not detected").color(Color32::from_rgb(220, 80, 80)).strong(),
                        ),
                        None => (
                            "○",
                            Color32::from_rgb(120, 120, 140),
                            RichText::new("Not checked"),
                        ),
                    }
                };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(dot).color(dot_color).size(20.0));
                    ui.label(state_label);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let btn_label = if snap.running {
                            "Checking…"
                        } else {
                            "Check BuildKit"
                        };
                        if ui
                            .add_enabled(!snap.running, egui::Button::new(btn_label))
                            .clicked()
                        {
                            self.run_probe(&self.buildkit_probe, build::check_buildkit);
                        }
                    });
                });
                ui.add_space(4.0);
                match &snap.result {
                    Some(build::BuildKitStatus::Available { version }) => {
                        ui.label(format!("Version: {}", version));
                    }
                    Some(build::BuildKitStatus::NotDetected) => {
                        ui.label(
                            RichText::new(
                                "BuildKit (buildx) not detected — builds fall back to the legacy builder.",
                            )
                            .size(12.0)
                            .color(Color32::from_rgb(255, 200, 80)),
                        );
                        ui.label(
                            RichText::new(
                                "Docker Desktop ships buildx by default. On Linux, install the plugin:",
                            )
                            .size(11.0)
                            .color(Color32::from_rgb(150, 150, 170)),
                        );
                        ui.label(
                            RichText::new("  sudo apt-get install docker-buildx-plugin")
                                .size(11.0)
                                .family(egui::FontFamily::Monospace),
                        );
                    }
                    None => {
                        ui.label(
                            RichText::new("Click \"Check BuildKit\" to probe the buildx plugin.")
                                .size(12.0)
                                .color(Color32::from_rgb(150, 150, 170)),
                        );
                    }
                }
            });

            ui.add_space(16.0);

            // Registry Status Card
            ui.heading(
                RichText::new("🌐 Docker Registry")
                    .size(16.0)
                    .color(Color32::from_rgb(255, 200, 50)),
            );
            ui.add_space(6.0);

            let reg_card = egui::Frame::group(ui.style())
                .fill(Color32::from_rgb(28, 28, 38))
                .corner_radius(8.0);
            reg_card.show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!("Status: {}", self.registry_status));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Test Registry").clicked() {
                            self.check_registry();
                        }
                    });
                });
                ui.add_space(4.0);
                if !self.current_mirror.is_empty() {
                    ui.label(
                        RichText::new(format!("Mirror: {}", self.current_mirror))
                            .color(Color32::from_rgb(100, 180, 255)),
                    );
                }
            });

            ui.add_space(16.0);

            // Docker Storage Card — CLI check parity (root dir + disk free),
            // probing through the same shared function as `forge2k check`.
            ui.heading(
                RichText::new("💾 Docker Storage")
                    .size(16.0)
                    .color(Color32::from_rgb(255, 200, 50)),
            );
            ui.add_space(6.0);

            let storage_card = egui::Frame::group(ui.style())
                .fill(Color32::from_rgb(28, 28, 38))
                .corner_radius(8.0);
            storage_card.show(ui, |ui| {
                let snap = self
                    .storage_probe
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone();
                let (dot, dot_color, state_label) = if snap.running {
                    (
                        "●",
                        Color32::from_rgb(255, 200, 50),
                        RichText::new("Checking…").strong(),
                    )
                } else {
                    match &snap.result {
                        Some(build::DockerStorageStatus {
                            disk: build::DiskSpaceStatus::Ok(_),
                            ..
                        }) => (
                            "●",
                            Color32::from_rgb(80, 220, 100),
                            RichText::new("Checked").color(Color32::from_rgb(80, 220, 100)).strong(),
                        ),
                        Some(build::DockerStorageStatus {
                            disk: build::DiskSpaceStatus::Unknown(_),
                            ..
                        }) => (
                            "●",
                            Color32::from_rgb(255, 200, 50),
                            RichText::new("Unknown").color(Color32::from_rgb(255, 200, 50)).strong(),
                        ),
                        None => (
                            "○",
                            Color32::from_rgb(120, 120, 140),
                            RichText::new("Not checked"),
                        ),
                    }
                };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(dot).color(dot_color).size(20.0));
                    ui.label(state_label);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let btn_label = if snap.running {
                            "Checking…"
                        } else {
                            "Check Storage"
                        };
                        if ui
                            .add_enabled(!snap.running, egui::Button::new(btn_label))
                            .clicked()
                        {
                            self.run_probe(&self.storage_probe, build::check_docker_storage);
                        }
                    });
                });
                ui.add_space(4.0);
                match &snap.result {
                    Some(status) => {
                        match &status.root_dir {
                            Some(root) => {
                                ui.label(format!("Root directory: {}", root));
                            }
                            None => {
                                ui.label(
                                    RichText::new(
                                        "Root directory: unknown (Docker daemon not reachable)",
                                    )
                                    .size(12.0)
                                    .color(Color32::from_rgb(255, 200, 80)),
                                );
                            }
                        }
                        match &status.disk {
                            build::DiskSpaceStatus::Ok(entries) => {
                                for entry in entries {
                                    ui.label(format!("{} {}", entry.volume, entry.summary()));
                                }
                            }
                            build::DiskSpaceStatus::Unknown(reason) => {
                                ui.label(
                                    RichText::new(format!("Disk space unknown: {}", reason))
                                        .size(12.0)
                                        .color(Color32::from_rgb(255, 200, 80)),
                                );
                            }
                        }
                    }
                    None => {
                        ui.label(
                            RichText::new(
                                "Click \"Check Storage\" to probe the Docker root directory and free disk space.",
                            )
                            .size(12.0)
                            .color(Color32::from_rgb(150, 150, 170)),
                        );
                    }
                }
            });

            ui.add_space(16.0);

            // Installation guide
            ui.heading(
                RichText::new("📖 Docker Installation Guide")
                    .size(16.0)
                    .color(Color32::from_rgb(255, 200, 50)),
            );
            ui.add_space(6.0);

            let guide_frame = egui::Frame::group(ui.style())
                .fill(Color32::from_rgb(28, 28, 38))
                .corner_radius(8.0);
            guide_frame.show(ui, |ui| {
                ui.label(
                    RichText::new("Docker is required to build CP2K images.")
                        .size(12.0)
                        .color(Color32::from_rgb(150, 150, 170)),
                );
                ui.add_space(4.0);
                if ui.button("📋 Show Installation Guide").clicked() {
                    build::print_docker_install_guide();
                }
            });

            ui.add_space(16.0);

            // Quick Help
            ui.heading(
                RichText::new("💡 Quick Tips")
                    .size(16.0)
                    .color(Color32::from_rgb(255, 200, 50)),
            );
            ui.add_space(6.0);

            let tips_frame = egui::Frame::group(ui.style())
                .fill(Color32::from_rgb(28, 28, 38))
                .corner_radius(8.0);
            tips_frame.show(ui, |ui| {
                ui.label("• CP2K builds take 1-3 hours depending on your system");
                ui.label("• Use 'forge2k list' in terminal to see all configs");
                ui.label("• Use 'forge2k mirror' to configure registry mirror if network is slow");
                ui.label("• For WSL2, ensure Docker Desktop WSL integration is enabled");
                ui.label("• Minimum 50GB free disk space recommended for builds");
            });
        });
    }

    fn render_settings_tab(&mut self, ui: &mut egui::Ui) {
        ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(8.0);

            ui.heading(
                RichText::new("⚙ Docker Registry Mirror")
                    .size(16.0)
                    .color(Color32::from_rgb(255, 200, 50)),
            );
            ui.add_space(4.0);
            ui.label(
                "Configure a registry mirror to speed up Docker pulls in restricted networks.",
            );
            ui.add_space(8.0);

            let settings_card = egui::Frame::group(ui.style())
                .fill(Color32::from_rgb(28, 28, 38))
                .corner_radius(8.0);
            settings_card.show(ui, |ui| {
                ui.label("Mirror URL:");
                ui.horizontal(|ui| {
                    let mirror_edit = ui.add(
                        egui::TextEdit::singleline(&mut self.mirror_input)
                            .desired_width(400.0)
                            .hint_text("https://docker.mirrors.ustc.edu.cn"),
                    );
                    // Persist the draft only when the user commits an edit
                    // (content changed AND the field lost focus / Enter) —
                    // never per frame, never per keystroke.
                    if mirror_edit.changed() && mirror_edit.lost_focus() {
                        self.persist_settings();
                    }
                    if ui.button("Apply").clicked() {
                        if self.mirror_input.starts_with("http://")
                            || self.mirror_input.starts_with("https://")
                        {
                            match build::set_registry_mirror(&self.mirror_input) {
                                Ok(()) => {
                                    self.current_mirror = self.mirror_input.clone();
                                    self.settings_message =
                                        "✅ Mirror configured! Restart Docker to apply.".into();
                                }
                                Err(e) => {
                                    self.settings_message = format!("❌ Failed: {}", e);
                                }
                            }
                        } else {
                            self.settings_message =
                                "❌ Invalid URL. Must start with http:// or https://".into();
                        }
                        // The typed draft is user intent: keep it across
                        // restarts regardless of whether the mirror itself
                        // was accepted by the Docker daemon.
                        self.persist_settings();
                    }
                });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("🔍 Auto-detect Best Mirror").clicked() {
                        if let Some(url) = build::detect_best_mirror() {
                            self.mirror_input = url.clone();
                            self.settings_message = format!("✅ Found working mirror: {}", url);
                            self.persist_settings();
                        } else {
                            self.settings_message =
                                "❌ No working mirror found. Try a manual URL.".into();
                        }
                    }
                    ui.add_space(8.0);
                    if ui.button("🗑 Remove Mirror").clicked() {
                        match build::remove_registry_mirror() {
                            Ok(()) => {
                                self.current_mirror.clear();
                                self.settings_message =
                                    "✅ Mirror removed. Restart Docker to apply.".into();
                            }
                            Err(e) => {
                                self.settings_message = format!("❌ Failed: {}", e);
                            }
                        }
                    }
                });

                // ─── Restart Docker (CLI `mirror --restart` parity) ───
                // Applying or removing a mirror only takes effect once the
                // daemon restarts; this is the in-GUI equivalent.
                #[cfg(windows)]
                {
                    ui.add_space(8.0);
                    // Snapshot under a short lock; the worker thread keeps
                    // writing the original behind the mutex.
                    let snap = self
                        .restart_probe
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .clone();
                    ui.horizontal(|ui| {
                        // A restart tears down Docker Desktop, which would kill
                        // an in-flight build — keep the button disabled while
                        // the Build tab is busy (CANCEL BUILD is the way out).
                        let building = *self
                            .is_building
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner);
                        let btn_label = if snap.running {
                            "⏳ Restarting…"
                        } else if building {
                            "🐳 Restart Docker (build in progress)"
                        } else {
                            "🐳 Restart Docker"
                        };
                        if ui
                            .add_enabled(!snap.running && !building, egui::Button::new(btn_label))
                            .clicked()
                        {
                            // Worker thread, not the UI thread:
                            // restart_docker sleeps 3s and then polls for up
                            // to 60s until the engine answers — a synchronous
                            // call would freeze the whole window for that
                            // long. `run_probe` also absorbs double-clicks
                            // while a restart is in flight.
                            self.run_probe(&self.restart_probe, || {
                                build::restart_docker().map_err(|e| format!("{:#}", e))
                            });
                        }
                        if snap.running {
                            ui.label(
                                RichText::new(
                                    "Restarting Docker Desktop — the engine can take up to a minute to come back.",
                                )
                                .size(12.0)
                                .color(Color32::from_rgb(255, 200, 80))
                                .strong(),
                            );
                        }
                    });
                    match &snap.result {
                        Some(Ok(())) => {
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(
                                    "✅ Docker restarted. Use \"Check Docker\" on the System tab to confirm the engine is back up.",
                                )
                                .size(12.0)
                                .color(Color32::from_rgb(100, 220, 120)),
                            );
                        }
                        Some(Err(e)) => {
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(format!(
                                    "❌ Restart failed: {e}\n   \
                                     Start Docker Desktop manually, then use \
                                     \"Check Docker\" on the System tab."
                                ))
                                .size(12.0)
                                .color(Color32::from_rgb(255, 100, 100)),
                            );
                        }
                        None => {}
                    }
                }
                #[cfg(not(windows))]
                {
                    // The non-Windows arm of restart_docker shells out to
                    // `sudo systemctl restart docker` (falling back to
                    // rootless `systemctl --user restart docker`). A GUI
                    // process has no tty, so sudo could never read the
                    // password — show the manual command instead of failing.
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(
                            "To apply the mirror change, restart the Docker daemon manually:",
                        )
                        .size(12.0),
                    );
                    ui.label(
                        RichText::new("  sudo systemctl restart docker")
                            .size(12.0)
                            .family(egui::FontFamily::Monospace),
                    );
                    ui.label(
                        RichText::new("  (rootless Docker: systemctl --user restart docker)")
                            .size(11.0)
                            .family(egui::FontFamily::Monospace)
                            .color(Color32::from_rgb(150, 150, 170)),
                    );
                    ui.label(
                        RichText::new(
                            "The GUI cannot run this for you: it has no terminal for the sudo password prompt.",
                        )
                        .size(11.0)
                        .color(Color32::from_rgb(255, 200, 80)),
                    );
                }

                if !self.settings_message.is_empty() {
                    ui.add_space(8.0);
                    // Save failures (and all other errors) render in red.
                    let message_color = if self.settings_message.starts_with('❌') {
                        Color32::from_rgb(255, 100, 100)
                    } else {
                        Color32::from_rgb(220, 220, 220)
                    };
                    ui.label(
                        RichText::new(&self.settings_message)
                            .size(12.0)
                            .color(message_color),
                    );
                }
            });

            ui.add_space(8.0);
            if !self.current_mirror.is_empty() {
                ui.label(
                    RichText::new(format!("Current mirror: {}", self.current_mirror))
                        .color(Color32::from_rgb(100, 180, 255)),
                );
            }

            ui.add_space(24.0);

            // Known mirrors
            ui.heading(
                RichText::new("Known Public Mirrors")
                    .size(14.0)
                    .color(Color32::from_rgb(255, 200, 50)),
            );
            ui.add_space(4.0);
            let known_mirrors = crate::mirrors::known_mirrors();
            let mirror_frame = egui::Frame::group(ui.style())
                .fill(Color32::from_rgb(28, 28, 38))
                .corner_radius(8.0);
            mirror_frame.show(ui, |ui| {
                for m in known_mirrors {
                    let (name, url, desc) = (m.name, m.url, m.desc);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(name).strong().size(12.0));
                        ui.label(
                            RichText::new(url)
                                .color(Color32::from_rgb(100, 180, 255))
                                .size(12.0),
                        );
                        ui.label(
                            RichText::new(desc)
                                .color(Color32::from_rgb(120, 120, 140))
                                .size(11.0),
                        );
                        if ui.button("Use").clicked() {
                            self.mirror_input = url.to_string();
                            self.persist_settings();
                        }
                    });
                }
            });
        });
    }

    fn render_about_tab(&mut self, ui: &mut egui::Ui) {
        ui.add_space(16.0);

        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("🔥 Forge2K")
                    .size(32.0)
                    .color(Color32::from_rgb(255, 200, 50))
                    .strong(),
            );
            ui.label(
                RichText::new("Version 1.0.1")
                    .size(14.0)
                    .color(Color32::from_rgb(120, 120, 140)),
            );
            ui.add_space(8.0);
            ui.label(RichText::new("One-click CP2K Docker Image Builder").size(16.0));
            ui.add_space(16.0);

            ui.label("A beautiful cross-platform GUI + CLI tool for building CP2K Docker images,");
            ui.label("with Spack (v2025.2+), Toolchain (v2023.2+), v2026.2 (cmake), master and native builds.");
            ui.add_space(16.0);

            let features = [
                "🔥 One-click build with full configuration options",
                "📦 Build methods: Spack, Toolchain, native & master branch",
                "🖥 Beautiful GUI & powerful CLI interfaces",
                "⚙ Settings persistence (~/.forge2k/settings.json)",
                "⏹ Cancellable builds from both GUI and CLI",
                "🌐 Docker registry mirror management (GUI Settings & CLI mirror)",
                "🐳 Docker Engine detection with installation guide",
                "🎯 CPU target optimization (x86_64, Cascadelake, etc.)",
                "🎮 CUDA GPU options (P100, V100), filtered by build method",
                "🔄 Real-time build log streaming",
                "🪟 Cross-platform: Windows, Linux, macOS",
                "✅ Unit & behavior tests gated by CI (fmt, clippy, check, test)",
            ];
            for f in &features {
                ui.label(RichText::new(*f).size(13.0));
            }

            ui.add_space(16.0);
            ui.label(RichText::new("⚠ Known limitations").size(14.0).strong());
            let limitations = [
                "• Toolchain CUDA (P100/V100) variants: full image-level e2e deferred",
                "• 2026.1, 2026.2 & master: no bundled Dockerfile, synthesized per build (e2e deferred)",
                "• Release-mode CLI build: docker stdout not forwarded (kept as-is, pending OD-1)",
            ];
            for l in &limitations {
                ui.label(
                    RichText::new(*l)
                        .size(13.0)
                        .color(Color32::from_rgb(160, 140, 100)),
                );
            }

            ui.add_space(16.0);
            ui.label(
                RichText::new("Powered by Rust  •  egui  •  Docker")
                    .color(Color32::from_rgb(100, 100, 120)),
            );
            ui.label(
                RichText::new("Inspired by github.com/cp2k/cp2k-containers")
                    .color(Color32::from_rgb(80, 80, 100)),
            );
            ui.add_space(8.0);
            ui.label(
                RichText::new("Made with ❤️ for the computational chemistry community")
                    .color(Color32::from_rgb(120, 120, 140)),
            );
        });
    }

    /// Configs tab: full preset-configuration browser, CLI `list` parity
    /// (src/main.rs `cmd_list`). Data comes from the same
    /// `build::list_available_configs()` scan the Build tab caches in
    /// `configs_cache`; the tab's Refresh re-runs that scan on the UI
    /// thread on purpose — it is one `read_dir` sweep of the bundled
    /// dockerfiles/ directory (the same latency class as the Generate-Only
    /// path), so a worker thread would only add plumbing for no
    /// perceived-latency gain.
    ///
    /// No second sort/filter is invented here: `list_available_configs`
    /// already returns entries ordered method → version (descending) →
    /// mpi → cpu → cuda (src/build/catalog.rs), exactly the order
    /// `cmd_list` prints, and the filters below mirror the CLI's
    /// `--method` (spack/toolchain enum) and `--version` exact-match
    /// semantics.
    fn render_configs_tab(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.heading(
            RichText::new("Available Build Configurations")
                .size(15.0)
                .color(Color32::from_rgb(255, 200, 50)),
        );
        ui.add_space(8.0);

        // ─── Filter row (CLI `list --method/--version` parity) ───
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Method:")
                    .size(12.0)
                    .color(Color32::from_rgb(150, 150, 170)),
            );
            egui::ComboBox::from_id_salt("browse_method")
                .selected_text(self.browse_method.as_deref().unwrap_or("All"))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.browse_method, None, "All");
                    ui.selectable_value(&mut self.browse_method, Some("spack".into()), "spack");
                    ui.selectable_value(
                        &mut self.browse_method,
                        Some("toolchain".into()),
                        "toolchain",
                    );
                });

            ui.add_space(8.0);
            ui.label(
                RichText::new("Version:")
                    .size(12.0)
                    .color(Color32::from_rgb(150, 150, 170)),
            );
            // Version choices are the scanned versions of the currently
            // selected method filter (All → every scanned version), the
            // same set `list --method <m>` accepts.
            let versions: Vec<String> = {
                let mut vs: Vec<String> = self
                    .configs_cache
                    .iter()
                    .filter(|c| self.browse_method.as_ref().is_none_or(|m| c.method == *m))
                    .map(|c| c.version.clone())
                    .collect();
                vs.sort();
                vs.dedup();
                vs
            };
            egui::ComboBox::from_id_salt("browse_version")
                .selected_text(self.browse_version.as_deref().unwrap_or("All"))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.browse_version, None, "All");
                    for v in &versions {
                        ui.selectable_value(&mut self.browse_version, Some(v.clone()), v);
                    }
                });

            // Same single scan the Build tab's Refresh runs: swapping
            // `configs_cache` feeds both tabs.
            if ui.button("⟳ Refresh").clicked() {
                self.configs_cache = build::list_available_configs();
            }

            // Click feedback for the last Copy press; the full command is
            // on hover so the row stays compact.
            if let Some(cmd) = self.browse_copied.clone() {
                ui.label(
                    RichText::new("✓ Copied to clipboard")
                        .size(11.0)
                        .color(Color32::from_rgb(80, 220, 100)),
                )
                .on_hover_text(cmd);
            }
        });
        ui.add_space(8.0);

        // ─── Filtered list (same filter semantics as cmd_list) ───
        // Cloned once per frame from the cache so the render closure can
        // borrow `self` mutably for the Copy path; the scan itself is not
        // re-run here (frame loops never rescan dockerfiles/).
        let configs: Vec<build::ConfigInfo> = self
            .configs_cache
            .iter()
            .filter(|c| {
                self.browse_method.as_ref().is_none_or(|m| c.method == *m)
                    && self.browse_version.as_ref().is_none_or(|v| c.version == *v)
            })
            .cloned()
            .collect();

        if configs.is_empty() {
            // CLI parity: cmd_list's "No configurations match the given
            // filters." info line, in the same yellow as its ℹ marker.
            ui.label(
                RichText::new("ℹ No configurations match the given filters.")
                    .size(13.0)
                    .color(Color32::from_rgb(255, 200, 80)),
            );
            return;
        }

        ui.label(
            RichText::new(format!(
                "Showing {} of {} scanned configurations",
                configs.len(),
                self.configs_cache.len()
            ))
            .size(11.0)
            .color(Color32::from_rgb(120, 120, 140)),
        );
        ui.add_space(4.0);

        ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(2.0);
            for (i, config) in configs.iter().enumerate() {
                // Line 1 (CLI parity): "#N  description" + the synthesized
                // marker, in the Build tab's amber synthesized style.
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(format!("#{}", i + 1))
                            .size(13.0)
                            .strong()
                            .color(Color32::from_rgb(255, 200, 50)),
                    );
                    ui.label(
                        RichText::new(&config.description)
                            .size(13.0)
                            .color(Color32::from_rgb(120, 200, 255)),
                    );
                    if config.source == build::DockerfileSource::Synthesized {
                        ui.label(
                            RichText::new(
                                "synthesized: no bundled Dockerfile, generated at build time",
                            )
                            .size(10.0)
                            .color(Color32::from_rgb(255, 200, 80)),
                        );
                    }
                });
                // Line 2 (CLI parity): Method + Version + base image.
                ui.label(
                    RichText::new(format!(
                        "      Method: {}/{} ({})",
                        config.method, config.version, config.base_image
                    ))
                    .size(11.0)
                    .color(Color32::from_rgb(160, 160, 180)),
                );
                // Line 3 (CLI parity): MPI / CPU / CUDA / Variant specs.
                ui.label(
                    RichText::new(format!(
                        "      Specs: MPI={} / CPU={} / CUDA={} / Variant={}",
                        config.mpi, config.cpu, config.cuda, config.variant
                    ))
                    .size(11.0)
                    .color(Color32::from_rgb(120, 170, 255)),
                );
                // Line 4 (CLI parity): the copyable build command, from the
                // single assembler `build::cli_build_command` — the same
                // source cmd_list prints from.
                ui.horizontal_wrapped(|ui| {
                    let cmd = build::cli_build_command(config);
                    ui.label(
                        RichText::new(&cmd)
                            .size(11.0)
                            .family(egui::FontFamily::Monospace)
                            .color(Color32::from_rgb(100, 180, 255)),
                    );
                    if ui.button("Copy").clicked() {
                        ui.ctx().copy_text(cmd.clone());
                        self.browse_copied = Some(cmd.clone());
                    }
                });
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(2.0);
            }
        });
    }
}

// ============================================================
// Native version advertisement
// ============================================================

/// Versions the native method advertises in its picker.
///
/// 2026.2 is intentionally absent: upstream deleted the root Makefile in
/// v2026.1 and the native release arm still builds via `make`, so a 2026.2
/// selection could never succeed (cmake migration is backlog). "2026.1" is
/// a baseline legacy entry with the same defect — kept for now, tracked in
/// the README known-limitations backlog. `master` stays: the native arm
/// uses cmake for it.
pub(crate) fn native_version_list() -> Vec<String> {
    ["master", "2026.1", "2025.2", "2024.3"]
        .into_iter()
        .map(String::from)
        .collect()
}

// ============================================================
// Entry Point
// ============================================================

pub fn run_gui() -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("🔥 Forge2K - CP2K Docker Image Builder")
            .with_inner_size(Vec2::new(1024.0, 720.0))
            .with_min_inner_size(Vec2::new(800.0, 600.0)),
        ..Default::default()
    };

    eframe::run_native(
        "Forge2K",
        options,
        Box::new(|_cc| Ok(Box::new(Forge2kApp::default()))),
    )
    .map_err(|e| anyhow::anyhow!("GUI error: {}", e))?;

    Ok(())
}

// ============================================================
// CLI `list` parity helper
// ============================================================

#[cfg(test)]
mod config_tab_tests {
    use super::*;

    /// The Configs tab's Copy payload must reproduce the CLI `list`
    /// "Build:" line character-for-character. Both sides consume the single
    /// assembler `build::cli_build_command` (src/build/catalog.rs), so this
    /// pins the shared output rather than a duplicated format string.
    #[test]
    fn cli_build_command_matches_cli_list_format() {
        let c = build::ConfigInfo {
            method: "toolchain".into(),
            version: "2026.2".into(),
            mpi: "mpich".into(),
            cpu: "generic".into(),
            cuda: "P100".into(),
            variant: "psmp".into(),
            base_image: String::new(),
            description: String::new(),
            source: build::DockerfileSource::Synthesized,
        };
        assert_eq!(
            build::cli_build_command(&c),
            "forge2k build -m toolchain -v 2026.2 --mpi mpich --cpu generic --cuda P100 --variant psmp"
        );
    }
}
