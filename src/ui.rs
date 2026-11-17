use crate::{
    builds,
    config::{self, Invitation, Settings},
    game, service, updates,
};
use eframe::egui;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

enum Event {
    Status(u64, Result<service::Status, String>),
    Launched(Result<game::Running, String>),
    Support(u64, Result<(), String>),
    Checked(Result<Option<updates::Update>, String>),
    Updated(Result<(), String>),
}
pub struct App {
    root: PathBuf,
    settings: Settings,
    invite: Option<Invitation>,
    installation: Option<builds::Installation>,
    rt: Arc<tokio::runtime::Runtime>,
    tx: mpsc::Sender<Event>,
    rx: mpsc::Receiver<Event>,
    status: Option<service::Status>,
    status_text: String,
    notice: String,
    error: String,
    support_text: String,
    status_inflight: bool,
    support_inflight: bool,
    last_status: Instant,
    last_support: Instant,
    game: Option<game::Running>,
    launching: bool,
    generation: u64,
    session: String,
    update: Option<updates::Update>,
    update_busy: bool,
    update_text: String,
    close_question: bool,
    allow_close: bool,
    support_policy: Arc<AtomicU64>,
    last_update_check: Instant,
}
impl App {
    pub fn new(root: PathBuf, rt: Arc<tokio::runtime::Runtime>) -> Self {
        let (tx, rx) = mpsc::channel();
        let mut settings = config::load_settings(&root);
        let mut invite = config::load_invite(&root).ok();
        // Drop the new executable beside the old launcher: invitation/settings import themselves.
        if invite.is_none()
            && let Some(adjacent) = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.join("invite.json")))
                .filter(|p| p.is_file())
            && let Ok((i, legacy)) = config::import_invite(&adjacent)
            && config::store_invite(&root, &i).is_ok()
        {
            if let Some(s) = legacy {
                settings = s;
            }
            invite = Some(i);
            let _ = config::save_settings(&root, &settings);
        }
        if settings.game.is_none() {
            settings.game = builds::discover().into_iter().next();
        }
        let installation = settings.game.as_ref().and_then(|p| builds::inspect(p).ok());
        let mut app = Self {
            root,
            settings,
            invite,
            installation,
            rt,
            tx,
            rx,
            status: None,
            status_text: "Import your private invitation to connect.".into(),
            notice: String::new(),
            error: String::new(),
            support_text: "Support sharing is off.".into(),
            status_inflight: false,
            support_inflight: false,
            last_status: Instant::now() - Duration::from_secs(20),
            last_support: Instant::now() - Duration::from_secs(31),
            game: None,
            launching: false,
            generation: 0,
            session: format!(
                "rust-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            ),
            update: None,
            update_busy: false,
            update_text: "Launcher updates enabled.".into(),
            close_question: false,
            allow_close: false,
            support_policy: Arc::new(AtomicU64::new(0)),
            last_update_check: Instant::now(),
        };
        if app.settings.auto_update {
            app.check_updates();
        }
        app
    }
    fn save(&mut self) {
        if let Err(e) = config::save_settings(&self.root, &self.settings) {
            self.error = e.to_string();
        }
    }
    fn accepted(&self) -> Vec<String> {
        self.status
            .as_ref()
            .map(|s| &s.accepted_builds)
            .filter(|s| !s.is_empty())
            .cloned()
            .or_else(|| self.invite.as_ref().map(|i| i.expected_builds()))
            .unwrap_or_default()
    }
    fn preflight(&self) -> String {
        match &self.installation {
            Some(i) => match builds::compatibility(i, &self.accepted()) {
                Ok(v) => format!("Installed: {v}. Compatible with this server."),
                Err(e) => e.to_string(),
            },
            None => "No valid WoW Forever installation selected.".into(),
        }
    }
    fn check_updates(&mut self) {
        if self.update_busy {
            return;
        }
        self.last_update_check = Instant::now();
        self.update_busy = true;
        self.update_text = "Checking launcher updates...".into();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let answer = async { updates::check(&service::client()?).await }
                .await
                .map_err(|e: anyhow::Error| e.to_string());
            let _ = tx.send(Event::Checked(answer));
        });
    }
    fn import(&mut self, path: PathBuf) {
        match config::import_invite(&path).and_then(|(i, s)| {
            config::store_invite(&self.root, &i)?;
            Ok((i, s))
        }) {
            Ok((i, old)) => {
                if let Some(s) = old {
                    self.settings = s;
                }
                self.invite = Some(i);
                self.generation += 1;
                self.support_policy.fetch_add(1, Ordering::SeqCst);
                self.status_inflight = false;
                self.support_inflight = false;
                self.status = None;
                self.error.clear();
                self.notice = "Invitation imported privately for your Windows user.".into();
                self.last_status = Instant::now() - Duration::from_secs(20);
                self.last_support = Instant::now() - Duration::from_secs(31);
                self.installation = self
                    .settings
                    .game
                    .as_ref()
                    .and_then(|p| builds::inspect(p).ok());
                self.save();
            }
            Err(e) => self.error = e.to_string(),
        }
    }
    fn play(&mut self) {
        if self.launching || self.game.is_some() {
            return;
        }
        let Some(i) = self.invite.clone() else {
            return;
        };
        if let Err(e) = self
            .installation
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Choose WoW Forever first"))
            .and_then(|i| builds::compatibility(i, &self.accepted()))
        {
            self.error = e.to_string();
            return;
        }
        self.launching = true;
        self.error.clear();
        let tx = self.tx.clone();
        let root = self.root.clone();
        let settings = self.settings.clone();
        let accepted = self.accepted();
        self.rt.spawn(async move {
            let result = game::launch(root, settings, i, accepted)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(Event::Launched(result));
        });
    }
    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Status(g, r) if g == self.generation => {
                    self.status_inflight = false;
                    match r {
                        Ok(s) => {
                            self.status_text = if s.ready {
                                "Server ready.".into()
                            } else {
                                "Server is offline or still starting. Your files are safe.".into()
                            };
                            self.status = Some(s);
                        }
                        Err(e) => {
                            self.status_text = e;
                            self.status = None;
                        }
                    }
                }
                Event::Support(g, r) if g == self.generation => {
                    self.support_inflight = false;
                    self.support_text = match r {
                        Ok(()) => "Support report delivered. Next update in 30 seconds.".into(),
                        Err(e) => format!("{e}; retrying automatically."),
                    };
                }
                Event::Launched(r) => {
                    self.launching = false;
                    match r {
                        Ok(game) => {
                            self.game = Some(game);
                            self.notice =
                                "Game started. Minimize this launcher while playing.".into();
                        }
                        Err(e) => self.error = e,
                    }
                }
                Event::Checked(r) => {
                    self.update_busy = false;
                    match r {
                        Ok(Some(update)) => {
                            self.update_text = format!("Launcher {} is available.", update.version);
                            self.update = Some(update);
                        }
                        Ok(None) => {
                            self.update_text = "No newer published launcher release.".into()
                        }
                        Err(e) => self.update_text = format!("Update check deferred: {e}"),
                    }
                }
                Event::Updated(r) => {
                    self.update_busy = false;
                    match r {
                        Ok(()) => {
                            self.allow_close = true;
                            let _ = std::process::Command::new(std::env::current_exe().unwrap())
                                .arg("--updated")
                                .spawn();
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        Err(e) => self.update_text = format!("Update was not installed: {e}"),
                    }
                }
                _ => {}
            }
        }
        if let Some(running) = &mut self.game
            && let Ok(Some(exit)) = running.child.try_wait()
        {
            self.notice = format!("Game connector exited: {exit}. Support logs remain available.");
            self.game = None;
            self.last_support = Instant::now() - Duration::from_secs(31);
        }
        if let Some(invite) = self.invite.clone() {
            if !self.status_inflight && self.last_status.elapsed() > Duration::from_secs(15) {
                self.status_inflight = true;
                self.last_status = Instant::now();
                let tx = self.tx.clone();
                let g = self.generation;
                let i = invite.clone();
                self.rt.spawn(async move {
                    let r = async { service::status(&service::client()?, &i).await }
                        .await
                        .map_err(|e: anyhow::Error| e.to_string());
                    let _ = tx.send(Event::Status(g, r));
                });
            }
            // Starts before Play/version validation, so compatibility failures are diagnosable.
            if self.settings.support_diagnostics
                && self.settings.support_consent_version == 1
                && !self.support_inflight
                && self.last_support.elapsed() > Duration::from_secs(30)
            {
                self.support_inflight = true;
                self.last_support = Instant::now();
                let tx = self.tx.clone();
                let g = self.generation;
                let settings = self.settings.clone();
                let root = self.root.clone();
                let session = self.session.clone();
                let helper = self.game.as_ref().map(|r| r.child.id());
                let preflight = self.preflight();
                let policy = self.support_policy.clone();
                let epoch = policy.load(Ordering::SeqCst);
                self.rt.spawn(async move {
                    let report = tokio::task::spawn_blocking(move || {
                        crate::diagnostics::collect(&settings, &root, &session, helper, &preflight)
                    })
                    .await;
                    let result = async {
                        let report = report?;
                        if policy.load(Ordering::SeqCst) != epoch {
                            return Ok(());
                        }
                        crate::diagnostics::upload(&service::client()?, &invite, &report).await
                    }
                    .await
                    .map_err(|e: anyhow::Error| e.to_string());
                    let _ = tx.send(Event::Support(g, result));
                });
            }
        }
        if self.settings.auto_update
            && self.update.is_some()
            && !self.update_busy
            && self.game.is_none()
            && !self.launching
        {
            self.install_update();
        }
        if self.settings.auto_update
            && !self.update_busy
            && self.last_update_check.elapsed() > Duration::from_secs(6 * 3600)
        {
            self.check_updates();
        }
        ctx.request_repaint_after(Duration::from_millis(250));
    }
    fn install_update(&mut self) {
        if self.game.is_some() || self.launching || self.update_busy {
            return;
        }
        if let Some(update) = self.update.take() {
            self.update_busy = true;
            self.update_text = "Downloading and verifying launcher update...".into();
            let tx = self.tx.clone();
            self.rt.spawn(async move {
                let r = updates::install(&update).await.map_err(|e| e.to_string());
                let _ = tx.send(Event::Updated(r));
            });
        }
    }
}
impl eframe::App for App {
    fn ui(&mut self, root_ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = root_ui.ctx().clone();
        self.poll(&ctx);
        if ctx.input(|i| i.viewport().close_requested())
            && (self.game.is_some() || self.launching)
            && !self.allow_close
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_question = true;
        }
        if self.close_question {
            egui::Window::new("Keep your game connected")
                .collapsible(false)
                .resizable(false)
                .show(&ctx, |ui| {
                    ui.label("This launcher carries the connection to your private server.");
                    if ui.button("Keep playing — minimize launcher").clicked() {
                        self.close_question = false;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    }
                    if ui.button("Exit launcher and disconnect game").clicked() {
                        self.allow_close = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
        }
        egui::Panel::top("header").show(root_ui, |ui| {
            ui.add_space(14.);
            ui.horizontal(|ui| {
                ui.heading("WoW Friends");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.weak(format!("Launcher {}", env!("CARGO_PKG_VERSION")));
                });
            });
            ui.label("Your private realm, one launcher.");
            ui.add_space(12.);
        });
        egui::CentralPanel::default().show(root_ui,|ui|{
        ui.add_space(12.);
        egui::Frame::group(ui.style()).inner_margin(16.).show(ui,|ui|{
            if let Some(i)=&self.invite{ui.heading(format!("Welcome, {}",i.name));ui.label(format!("Server: {}",i.origin()));ui.horizontal(|ui|{
                if ui.button("Copy game account").clicked(){ctx.copy_text(i.account.clone());}
                if let Some(password)=&i.password{if ui.button("Copy game password").clicked(){ctx.copy_text(password.clone());}}else{ui.weak("Use your existing private-server password.");}
            });}else{ui.heading("Connect with your private invitation");ui.label("Use the invitation or folder from your old Friends launcher. Nothing is uploaded to GitHub.");}
            if self.game.is_none()&&!self.launching{ui.horizontal(|ui|{
                if ui.button("Import invitation").clicked()&& let Some(path)=rfd::FileDialog::new().set_title("Choose your PRIVATE invite.json").add_filter("Private invitation",&["json"]).pick_file(){self.import(path);}
                if ui.button("Use existing launcher folder").clicked()&& let Some(folder)=rfd::FileDialog::new().set_title("Choose the folder containing Start WoW.exe and invite.json").pick_folder(){self.import(folder.join("invite.json"));}
            });}
        });
        ui.add_space(12.);
        egui::Frame::group(ui.style()).inner_margin(16.).show(ui,|ui|{
            ui.heading("World of Warcraft: Forever");
            if let Some(path)=&self.settings.game{ui.label(path.display().to_string());}else{ui.weak("No game selected.");}
            ui.label(self.preflight());ui.label(&self.status_text);
            if let Some(game)=&self.game{ui.weak(format!("Active game transport connections: {}",game._relay.connections.load(Ordering::SeqCst)));}
            ui.add_enabled_ui(self.game.is_none()&&!self.launching,|ui|{if ui.button("Choose game — _forever_ / Wow.exe").clicked()&& let Some(path)=rfd::FileDialog::new().set_title("Choose the GAME's Wow.exe inside _forever_").add_filter("World of Warcraft",&["exe"]).pick_file(){self.settings.game=Some(path.clone());match builds::inspect(&path){Ok(i)=>{self.installation=Some(i);self.error.clear();},Err(e)=>{self.installation=None;self.error=e.to_string();}}self.last_support=Instant::now()-Duration::from_secs(31);self.save();}});
            ui.add_space(10.);
            let ready=self.status.as_ref().is_some_and(|s|s.ready)&&self.invite.is_some()&&self.installation.as_ref().is_some_and(|i|builds::compatibility(i,&self.accepted()).is_ok());
            ui.horizontal(|ui|{
                let label=if self.game.is_some(){"GAME RUNNING"}else if self.launching{"Starting..."}else{"PLAY"};
                if ui.add_enabled(ready&&self.game.is_none()&&!self.launching&&!self.update_busy,egui::Button::new(label).fill(egui::Color32::from_rgb(34,128,87)).min_size(egui::vec2(180.,42.))).clicked(){self.play();}
                if self.game.is_none()&& ui.button("Windows setup").on_hover_text("One-time administrator approval for the private game's public CA and localhost hostname. Game itself runs normally.").clicked()&& let Some(invite)=&self.invite{match crate::platform::request_setup(&invite.ca_pem,&self.root){Ok(())=>self.notice="Approve Windows setup, then click Play.".into(),Err(e)=>self.error=e.to_string()}}
            });
            ui.weak("Game updates and launcher updates are separate. Your game files are never silently downgraded.");
        });
        ui.add_space(12.);
        egui::Frame::group(ui.style()).inner_margin(16.).show(ui,|ui|{
            ui.heading("Updates & support");
            if ui.checkbox(&mut self.settings.auto_update,"Automatically update this launcher when idle").changed(){self.save();}
            ui.horizontal(|ui| {
                ui.label(&self.update_text);
                if ui.add_enabled(!self.update_busy, egui::Button::new("Check updates")).clicked() { self.check_updates(); }
                if self.update.is_some() && ui.button("Install update").clicked() { self.install_update(); }
            });
            if ui.checkbox(&mut self.settings.support_diagnostics,"Share private WoW support reports with my server owner").changed(){self.support_policy.fetch_add(1,Ordering::SeqCst);self.settings.support_consent_version=if self.settings.support_diagnostics{1}else{0};self.last_support=Instant::now()-Duration::from_secs(31);self.save();}
            ui.weak("Only bounded game errors, version, connection/control state and your launched game's responsiveness. No screenshots, passwords or remote control.");
            if self.settings.support_diagnostics{ui.label(&self.support_text);}else{ui.weak("Uploads off. Live in-game reporting stops when you exit WoW.");}
        });
        if !self.notice.is_empty(){ui.add_space(8.);ui.label(&self.notice);}
        if !self.error.is_empty(){ui.add_space(8.);ui.colored_label(egui::Color32::from_rgb(245,145,110),&self.error);}
    });
    }
}
pub fn run(root: PathBuf, rt: Arc<tokio::runtime::Runtime>) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([840., 760.])
            .with_min_inner_size([700., 620.]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "WoW Friends Launcher",
        options,
        Box::new(move |cc| {
            let mut visuals = egui::Visuals::dark();
            visuals.panel_fill = egui::Color32::from_rgb(25, 29, 28);
            visuals.window_fill = egui::Color32::from_rgb(30, 35, 33);
            visuals.selection.bg_fill = egui::Color32::from_rgb(35, 115, 78);
            cc.egui_ctx.set_visuals(visuals);
            Ok(Box::new(App::new(root, rt)))
        }),
    )
}
