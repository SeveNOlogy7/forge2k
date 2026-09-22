use crate::build;
use anyhow::Result;
use eframe::egui::{self, Color32, RichText, ScrollArea, Vec2};
use std::sync::{mpsc, Arc, Mutex};

// ============================================================
// Tab enum
// ============================================================

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Build,
    System,
    Settings,
    About,
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

    // Build state
    build_logs: Arc<Mutex<Vec<build::LogLine>>>,
    is_building: Arc<Mutex<bool>>,
    cancel_flag: Arc<Mutex<bool>>,
    build_error: Arc<Mutex<Option<String>>>,
    log_tx: Option<mpsc::Sender<build::LogLine>>,

    // System state
    docker_status: String,
    docker_running: bool,
    docker_checked: bool,
    registry_status: String,
    current_mirror: String,

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
        Self {
            method: "spack".into(),
            version: "2025.2".into(),
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
            log_tx: None,
            docker_status: "Not checked".into(),
            docker_running: false,
            docker_checked: false,
            registry_status: "Not checked".into(),
            current_mirror,
            mirror_input: String::new(),
            settings_message: String::new(),
            active_tab: Tab::Build,
            log_auto_scroll: true,
        }
    }
}

impl Forge2kApp {
    fn start_build(&mut self) {
        if *self.is_building.lock().unwrap() {
            return;
        }

        // Clear previous logs
        self.build_logs.lock().unwrap().clear();
        *self.build_error.lock().unwrap() = None;
        *self.cancel_flag.lock().unwrap() = false;

        let num_jobs: u32 = self.jobs.parse().unwrap_or(0);
        let num_jobs = if num_jobs == 0 {
            std::thread::available_parallelism()
                .map(|n| n.get() as u32)
                .unwrap_or(4)
        } else {
            num_jobs
        };

        let config = build::BuildConfig {
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
            no_cache: false,
            dockerfile: None,
            shm_size: self.shm_size.clone(),
            _output: "docker".into(),
        };

        let (tx, rx) = mpsc::channel::<build::LogLine>();
        self.log_tx = Some(tx.clone());

        let logs = self.build_logs.clone();
        let is_building = self.is_building.clone();
        let cancel = self.cancel_flag.clone();
        let error = self.build_error.clone();

        *is_building.lock().unwrap() = true;

        // Spawn log collector thread
        std::thread::spawn(move || {
            for line in rx {
                logs.lock().unwrap().push(line);
            }
        });

        // Spawn build thread
        std::thread::spawn(move || {
            let result = build::execute_build(&config, tx, cancel);
            match result {
                Ok(()) => {}
                Err(e) => {
                    *error.lock().unwrap() = Some(e.to_string());
                }
            }
            *is_building.lock().unwrap() = false;
        });
    }

    fn cancel_build(&mut self) {
        *self.cancel_flag.lock().unwrap() = true;
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
                    self.registry_status.push_str("\n   Try: Settings → Auto-detect Mirror");
                }
            }
            build::NetworkStatus::Unknown(reason) => {
                self.registry_status = format!("❓ {}", reason);
            }
        }
    }

    #[allow(dead_code)]
    fn config_summary(&self) -> Vec<(String, String)> {
        vec![
            ("Method".into(), format!("{} / CP2K {}", self.method, self.version)),
            ("MPI".into(), self.mpi.clone()),
            ("CPU Target".into(), self.cpu.clone()),
            ("CUDA".into(), self.cuda.clone()),
            ("Variant".into(), self.variant.clone()),
            ("Jobs".into(), if self.jobs == "0" { "Auto".into() } else { self.jobs.clone() }),
            ("SHM Size".into(), self.shm_size.clone()),
        ]
    }
}

impl eframe::App for Forge2kApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Keep repainting during build
        if *self.is_building.lock().unwrap() {
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
                    RichText::new("v1.0.0")
                        .size(12.0)
                        .color(Color32::from_rgb(120, 120, 140)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(12.0);
                    if *self.is_building.lock().unwrap() {
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
                        RichText::new(if self.docker_running { "● Docker OK" } else { "○ Docker" })
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
                let tabs = [("🛠 Build", Tab::Build), ("📊 System", Tab::System), ("⚙ Settings", Tab::Settings), ("ℹ About", Tab::About)];
                for (label, tab) in &tabs {
                    let is_active = self.active_tab == *tab;
                    let btn = egui::Button::new(
                        RichText::new(*label)
                            .size(14.0)
                            .color(if is_active {
                                Color32::from_rgb(255, 200, 50)
                            } else {
                                Color32::from_rgb(160, 160, 180)
                            }),
                    )
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
    fn render_build_tab(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        let is_building_val = *self.is_building.lock().unwrap();

        ui.horizontal(|ui| {
            // ─── LEFT: Configuration Panel ───
            egui::SidePanel::left("config_panel")
                .resizable(false)
                .default_width(280.0)
                .min_width(240.0)
                .show_inside(ui, |ui| {
                    ScrollArea::vertical().show(ui, |ui| {
                        ui.add_space(4.0);
                        ui.heading(RichText::new("Build Configuration").size(15.0).color(Color32::from_rgb(255, 200, 50)));
                        ui.add_space(8.0);

                        // Method
                        ui.label(RichText::new("Build Method").size(12.0).color(Color32::from_rgb(150, 150, 170)));
                        egui::ComboBox::from_id_salt("method")
                            .selected_text(&self.method)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.method, "spack".into(), "Spack (Docker, Ubuntu 24.04)");
                                ui.selectable_value(&mut self.method, "toolchain".into(), "Toolchain (Docker, Ubuntu 22.04)");
                                ui.selectable_value(&mut self.method, "native".into(), "Native (direct on host)");
                            });
                        ui.add_space(6.0);

                        // Version
                        ui.label(RichText::new("CP2K Version").size(12.0).color(Color32::from_rgb(150, 150, 170)));
                        let versions = match self.method.as_str() {
                            "toolchain" => vec!["master", "2026.1", "2025.2", "2024.3", "2024.2", "2024.1", "2023.2"],
                            "native" => vec!["master", "2026.1", "2025.2", "2024.3"],
                            _ => vec!["2026.1", "2025.2", "2024.3", "2024.2"],
                        };
                        egui::ComboBox::from_id_salt("version")
                            .selected_text(&self.version)
                            .show_ui(ui, |ui| {
                                for v in &versions {
                                    ui.selectable_value(&mut self.version, v.to_string(), *v);
                                }
                            });
                        ui.add_space(6.0);

                        // MPI
                        ui.label(RichText::new("MPI Implementation").size(12.0).color(Color32::from_rgb(150, 150, 170)));
                        let mpis = match self.method.as_str() {
                            "toolchain" => vec!["mpich"],
                            _ => vec!["mpich", "openmpi"],
                        };
                        egui::ComboBox::from_id_salt("mpi")
                            .selected_text(&self.mpi)
                            .show_ui(ui, |ui| {
                                for m in &mpis {
                                    ui.selectable_value(&mut self.mpi, m.to_string(), *m);
                                }
                            });
                        ui.add_space(6.0);

                        // CPU Target
                        ui.label(RichText::new("CPU Target").size(12.0).color(Color32::from_rgb(150, 150, 170)));
                        let cpus: Vec<&str> = match self.method.as_str() {
                            "toolchain" => vec!["generic"],
                            _ => vec!["x86_64", "cascadelake", "haswell", "skylake-avx512", "generic"],
                        };
                        egui::ComboBox::from_id_salt("cpu")
                            .selected_text(&self.cpu)
                            .show_ui(ui, |ui| {
                                for c in &cpus {
                                    ui.selectable_value(&mut self.cpu, c.to_string(), *c);
                                }
                            });
                        ui.add_space(6.0);

                        // CUDA
                        ui.label(RichText::new("CUDA Support").size(12.0).color(Color32::from_rgb(150, 150, 170)));
                        let cudas: Vec<&str> = match self.method.as_str() {
                            "spack" => vec!["none"],
                            _ => vec!["none", "P100", "V100"],
                        };
                        egui::ComboBox::from_id_salt("cuda")
                            .selected_text(&self.cuda)
                            .show_ui(ui, |ui| {
                                for c in &cudas {
                                    ui.selectable_value(&mut self.cuda, c.to_string(), *c);
                                }
                            });
                        ui.add_space(6.0);

                        // Variant
                        ui.label(RichText::new("Binary Variant").size(12.0).color(Color32::from_rgb(150, 150, 170)));
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
                        ui.label(RichText::new("Parallel Jobs (0 = auto)").size(12.0).color(Color32::from_rgb(150, 150, 170)));
                        ui.add(egui::TextEdit::singleline(&mut self.jobs).desired_width(80.0));
                        ui.add_space(6.0);

                        // SHM Size
                        ui.label(RichText::new("Shared Memory Size").size(12.0).color(Color32::from_rgb(150, 150, 170)));
                        ui.add(egui::TextEdit::singleline(&mut self.shm_size).desired_width(80.0));
                        ui.add_space(6.0);

                        // Custom tag
                        ui.label(RichText::new("Custom Image Tag (optional)").size(12.0).color(Color32::from_rgb(150, 150, 170)));
                        ui.add(egui::TextEdit::singleline(&mut self.tag).desired_width(200.0).hint_text("auto-generate"));
                        ui.add_space(12.0);

                        // Build button
                        ui.add_space(4.0);
                        if is_building_val {
                            let cancel_btn = egui::Button::new(
                                RichText::new("⛔ CANCEL BUILD").size(14.0).color(Color32::from_rgb(255, 100, 100)),
                            )
                            .fill(Color32::from_rgb(60, 20, 20))
                            .min_size(Vec2::new(240.0, 36.0));
                            if ui.add(cancel_btn).clicked() {
                                self.cancel_build();
                            }
                        } else {
                            let build_btn = egui::Button::new(
                                RichText::new("🔥 START BUILD").size(14.0).color(Color32::from_rgb(20, 20, 30)),
                            )
                            .fill(Color32::from_rgb(255, 200, 50))
                            .min_size(Vec2::new(240.0, 36.0));
                            if ui.add(build_btn).clicked() {
                                self.check_docker();
                                if self.docker_running {
                                    self.start_build();
                                } else {
                                    // Will show error in log
                                    let logs = self.build_logs.clone();
                                    std::thread::spawn(move || {
                                        let mut l = logs.lock().unwrap();
                                        l.push(build::LogLine {
                                            timestamp: "ERROR".into(),
                                            text: "Docker is not running. Please start Docker first.".into(),
                                            is_error: true,
                                        });
                                    });
                                }
                            }
                        }

                        // Build error display
                        if let Some(err) = &*self.build_error.lock().unwrap() {
                            ui.add_space(8.0);
                            ui.label(RichText::new(format!("❌ {}", err)).color(Color32::from_rgb(255, 100, 100)).size(11.0));
                        }
                    });
                });

            // ─── RIGHT: Build Log ───
            egui::CentralPanel::default().show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(RichText::new("Build Output").size(15.0).color(Color32::from_rgb(255, 200, 50)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.checkbox(&mut self.log_auto_scroll, "Auto-scroll");
                        if ui.button("Clear").clicked() {
                            self.build_logs.lock().unwrap().clear();
                        }
                    });
                });
                ui.add_space(4.0);

                let logs = self.build_logs.lock().unwrap().clone();
                let frame = egui::Frame::dark_canvas(ui.style()).fill(Color32::from_rgb(10, 10, 16));
                frame.show(ui, |ui| {
                    ScrollArea::vertical()
                        .stick_to_bottom(self.log_auto_scroll && is_building_val)
                        .show(ui, |ui| {
                            ui.add_space(4.0);
                            if logs.is_empty() {
                                ui.label(RichText::new("  ⚡ Configure your build and click START BUILD")
                                    .size(13.0).color(Color32::from_rgb(80, 80, 100)));
                                ui.label(RichText::new("  Build logs will appear here in real-time.")
                                    .size(12.0).color(Color32::from_rgb(60, 60, 80)));
                            } else {
                                for line in &logs {
                                    let color = if line.is_error {
                                        Color32::from_rgb(255, 100, 100)
                                    } else if line.text.starts_with("✅") || line.text.starts_with("🚀") || line.text.starts_with("📋") {
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
            });
        });
    }

    fn render_system_tab(&mut self, ui: &mut egui::Ui) {
        ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(8.0);

            // Docker Status Card
            ui.heading(RichText::new("🐳 Docker Engine").size(16.0).color(Color32::from_rgb(255, 200, 50)));
            ui.add_space(6.0);

            let docker_card = egui::Frame::group(ui.style())
                .fill(Color32::from_rgb(28, 28, 38))
                .corner_radius(8.0);
            docker_card.show(ui, |ui| {
                ui.horizontal(|ui| {
                    if self.docker_checked {
                        if self.docker_running {
                            ui.label(RichText::new("●").color(Color32::from_rgb(80, 220, 100)).size(20.0));
                            ui.label(RichText::new("Running").color(Color32::from_rgb(80, 220, 100)).strong());
                        } else {
                            ui.label(RichText::new("●").color(Color32::from_rgb(220, 80, 80)).size(20.0));
                            ui.label(RichText::new("Not Running").color(Color32::from_rgb(220, 80, 80)).strong());
                        }
                    } else {
                        ui.label(RichText::new("○").color(Color32::from_rgb(120, 120, 140)).size(20.0));
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

            // Registry Status Card
            ui.heading(RichText::new("🌐 Docker Registry").size(16.0).color(Color32::from_rgb(255, 200, 50)));
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
                    ui.label(RichText::new(format!("Mirror: {}", self.current_mirror))
                        .color(Color32::from_rgb(100, 180, 255)));
                }
            });

            ui.add_space(16.0);

            // Installation guide
            ui.heading(RichText::new("📖 Docker Installation Guide").size(16.0).color(Color32::from_rgb(255, 200, 50)));
            ui.add_space(6.0);

            let guide_frame = egui::Frame::group(ui.style())
                .fill(Color32::from_rgb(28, 28, 38))
                .corner_radius(8.0);
            guide_frame.show(ui, |ui| {
                ui.label(RichText::new("Docker is required to build CP2K images.").size(12.0).color(Color32::from_rgb(150, 150, 170)));
                ui.add_space(4.0);
                if ui.button("📋 Show Installation Guide").clicked() {
                    build::print_docker_install_guide();
                }
            });

            ui.add_space(16.0);

            // Quick Help
            ui.heading(RichText::new("💡 Quick Tips").size(16.0).color(Color32::from_rgb(255, 200, 50)));
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

            ui.heading(RichText::new("⚙ Docker Registry Mirror").size(16.0).color(Color32::from_rgb(255, 200, 50)));
            ui.add_space(4.0);
            ui.label("Configure a registry mirror to speed up Docker pulls in restricted networks.");
            ui.add_space(8.0);

            let settings_card = egui::Frame::group(ui.style())
                .fill(Color32::from_rgb(28, 28, 38))
                .corner_radius(8.0);
            settings_card.show(ui, |ui| {
                ui.label("Mirror URL:");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.mirror_input)
                            .desired_width(400.0)
                            .hint_text("https://docker.mirrors.ustc.edu.cn"),
                    );
                    if ui.button("Apply").clicked() {
                        if self.mirror_input.starts_with("http://") || self.mirror_input.starts_with("https://") {
                            match build::set_registry_mirror(&self.mirror_input) {
                                Ok(()) => {
                                    self.current_mirror = self.mirror_input.clone();
                                    self.settings_message = "✅ Mirror configured! Restart Docker to apply.".into();
                                }
                                Err(e) => {
                                    self.settings_message = format!("❌ Failed: {}", e);
                                }
                            }
                        } else {
                            self.settings_message = "❌ Invalid URL. Must start with http:// or https://".into();
                        }
                    }
                });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("🔍 Auto-detect Best Mirror").clicked() {
                        if let Some(url) = build::detect_best_mirror() {
                            self.mirror_input = url.clone();
                            self.settings_message = format!("✅ Found working mirror: {}", url);
                        } else {
                            self.settings_message = "❌ No working mirror found. Try a manual URL.".into();
                        }
                    }
                    ui.add_space(8.0);
                    if ui.button("🗑 Remove Mirror").clicked() {
                        match build::remove_registry_mirror() {
                            Ok(()) => {
                                self.current_mirror.clear();
                                self.settings_message = "✅ Mirror removed. Restart Docker to apply.".into();
                            }
                            Err(e) => {
                                self.settings_message = format!("❌ Failed: {}", e);
                            }
                        }
                    }
                });

                if !self.settings_message.is_empty() {
                    ui.add_space(8.0);
                    ui.label(RichText::new(&self.settings_message).size(12.0));
                }
            });

            ui.add_space(8.0);
            if !self.current_mirror.is_empty() {
                ui.label(RichText::new(format!("Current mirror: {}", self.current_mirror))
                    .color(Color32::from_rgb(100, 180, 255)));
            }

            ui.add_space(24.0);

            // Known mirrors
            ui.heading(RichText::new("Known Public Mirrors").size(14.0).color(Color32::from_rgb(255, 200, 50)));
            ui.add_space(4.0);
            let known_mirrors = [
                ("USTC", "https://docker.mirrors.ustc.edu.cn", "China - University of Science and Technology"),
                ("Tencent Cloud", "https://mirror.ccs.tencentyun.com", "China - Tencent Cloud"),
                ("DaoCloud", "https://2a59f68c.m.daocloud.io", "China - DaoCloud"),
                ("Docker CN", "https://registry.docker-cn.com", "China - Docker Official CN Mirror"),
                ("DockerHub Proxy", "https://dockerhub.timeweb.cloud", "Russia - Timeweb Cloud"),
            ];
            let mirror_frame = egui::Frame::group(ui.style())
                .fill(Color32::from_rgb(28, 28, 38))
                .corner_radius(8.0);
            mirror_frame.show(ui, |ui| {
                for (name, url, desc) in &known_mirrors {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(*name).strong().size(12.0));
                        ui.label(RichText::new(*url).color(Color32::from_rgb(100, 180, 255)).size(12.0));
                        ui.label(RichText::new(*desc).color(Color32::from_rgb(120, 120, 140)).size(11.0));
                        if ui.button("Use").clicked() {
                            self.mirror_input = url.to_string();
                        }
                    });
                }
            });
        });
    }

    fn render_about_tab(&mut self, ui: &mut egui::Ui) {
        ui.add_space(16.0);

        ui.vertical_centered(|ui| {
            ui.label(RichText::new("🔥 Forge2K").size(32.0).color(Color32::from_rgb(255, 200, 50)).strong());
            ui.label(RichText::new("Version 1.0.0").size(14.0).color(Color32::from_rgb(120, 120, 140)));
            ui.add_space(8.0);
            ui.label(RichText::new("One-click CP2K Docker Image Builder").size(16.0));
            ui.add_space(16.0);

            ui.label("A beautiful cross-platform GUI + CLI tool for building CP2K Docker images");
            ui.label("with support for both Spack-based and Toolchain-based builds.");
            ui.add_space(16.0);

            let features = [
                "🔥 One-click build with full configuration options",
                "📦 Multiple build methods: Spack (2025.2) & Toolchain (2023.2)",
                "🖥 Beautiful GUI & powerful CLI interfaces",
                "🌐 Auto-detects and configures Docker registry mirrors",
                "🐳 Docker Engine detection with installation guide",
                "🎯 CPU target optimization (x86_64, Cascadelake, etc.)",
                "🎮 CUDA GPU support (P100, V100)",
                "🔄 Real-time build log streaming",
                "🪟 Cross-platform: Windows, Linux, WSL2",
            ];
            for f in &features {
                ui.label(RichText::new(*f).size(13.0));
            }

            ui.add_space(16.0);
            ui.label(RichText::new("Powered by Rust  •  egui  •  Docker").color(Color32::from_rgb(100, 100, 120)));
            ui.label(RichText::new("Inspired by github.com/cp2k/cp2k-containers").color(Color32::from_rgb(80, 80, 100)));
            ui.add_space(8.0);
            ui.label(RichText::new("Made with ❤️ for the computational chemistry community").color(Color32::from_rgb(120, 120, 140)));
        });
    }
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
