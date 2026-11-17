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
    Picked(Picker, Option<PathBuf>, Duration),
    Status(u64, Result<service::Status, String>),
    Launched(Result<game::Running, String>),
    Support(u64, Result<(), String>),
    Checked(Result<Option<updates::Update>, String>),
    Updated(Result<(), String>),
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Picker {
    Invitation,
    Folder,
    Game,
}
/// Runs a native dialog. Replaceable so UI tests never open a real Windows dialog.
pub(crate) type DialogRunner =
    Arc<dyn Fn(Picker, rfd::FileDialog) -> Option<PathBuf> + Send + Sync>;
fn native_dialog(picker: Picker, dialog: rfd::FileDialog) -> Option<PathBuf> {
    match picker {
        Picker::Folder => dialog.pick_folder(),
        Picker::Invitation | Picker::Game => dialog.pick_file(),
    }
}
/// rfd reports "cancelled" and "could not open" identically; a dialog a person
/// dismissed was visible for a moment, a failed one returns almost instantly.
const DIALOG_FAILED_WITHIN: Duration = Duration::from_millis(400);
/// Accepts what people paste from Explorer: quotes, spaces, Unicode, the old
/// launcher folder, invite.json itself, or any file inside the old folder.
pub(crate) fn resolve_invitation(input: &str) -> Result<PathBuf, String> {
    let input = input.trim().trim_matches(['"', '\'']).trim();
    if input.is_empty() {
        return Err("Paste your old launcher folder or invite.json path first.".into());
    }
    let path = PathBuf::from(input);
    let shown = path.display().to_string();
    let candidate = if path.is_dir() {
        path.join("invite.json")
    } else if path.is_file()
        && !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("json"))
    {
        path.parent().map(|p| p.join("invite.json")).unwrap_or(path)
    } else {
        path
    };
    if candidate.is_file() {
        return Ok(candidate);
    }
    let folder = candidate.parent().unwrap_or(&candidate);
    Err(if folder.is_dir() {
        format!(
            "No invite.json was found in {}. Choose the folder of your OLD Friends launcher (it contains invite.json).",
            folder.display()
        )
    } else {
        format!("This path does not exist on this PC: {shown}")
    })
}
/// What the support section says. Must never claim "off" when the box is
/// ticked, nor "sending" when nothing can be sent.
pub(crate) fn support_summary(
    enabled: bool,
    has_invite: bool,
    last: Option<&Result<(), String>>,
) -> String {
    match (enabled, has_invite, last) {
        (false, _, _) => "Uploads off. Live in-game reporting stops when you exit WoW.".into(),
        (true, false, _) => "Reports are switched on, but nothing can be sent yet: import your private invitation first (it contains the server address and your private access key).".into(),
        (true, true, None) => "Reports are on. Sending the first report...".into(),
        (true, true, Some(Ok(()))) => "Support report delivered. Next update in 30 seconds.".into(),
        (true, true, Some(Err(e))) => format!("Report NOT delivered: {e} Retrying automatically."),
    }
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
    invitation_input: String,
    picker_busy: bool,
    dialog: DialogRunner,
    support_last: Option<Result<(), String>>,
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
            update_text: String::new(),
            close_question: false,
            allow_close: false,
            support_policy: Arc::new(AtomicU64::new(0)),
            last_update_check: Instant::now(),
            invitation_input: String::new(),
            picker_busy: false,
            dialog: Arc::new(native_dialog),
            support_last: None,
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
            Some(_) if self.accepted().is_empty() => {
                "Game found. Compatibility is checked after your private invitation is imported."
                    .into()
            }
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
    fn busy_with_game(&self) -> bool {
        self.game.is_some() || self.launching
    }
    fn import(&mut self, path: PathBuf) {
        if self.busy_with_game() {
            self.error = "Close the game before changing the invitation.".into();
            return;
        }
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
                self.support_last = None;
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
    fn pick(&mut self, picker: Picker, frame: &eframe::Frame) {
        if self.picker_busy || self.busy_with_game() {
            return;
        }
        let dialog = match picker {
            Picker::Invitation => rfd::FileDialog::new()
                .set_title("Choose your PRIVATE invite.json")
                .add_filter("Private invitation", &["json"]),
            Picker::Folder => rfd::FileDialog::new()
                .set_title("Choose the folder of your old Friends launcher (contains invite.json)"),
            Picker::Game => rfd::FileDialog::new()
                .set_title("Choose the GAME's Wow.exe inside _forever_")
                .add_filter("World of Warcraft", &["exe"]),
        }
        // Owned by the launcher window: the dialog cannot open hidden behind it.
        .set_parent(frame);
        self.picker_busy = true;
        self.error.clear();
        self.notice =
            "Windows file picker opened. If you cannot see it, paste the path below instead."
                .into();
        let tx = self.tx.clone();
        let run = self.dialog.clone();
        // A fresh thread always gets its own STA COM apartment, so the render
        // thread's COM state cannot make the dialog fail silently.
        let spawned = std::thread::Builder::new()
            .name("file-picker".into())
            .spawn(move || {
                let started = Instant::now();
                let path =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(picker, dialog)))
                        .ok()
                        .flatten();
                let _ = tx.send(Event::Picked(picker, path, started.elapsed()));
            });
        if spawned.is_err() {
            self.picker_busy = false;
            self.notice =
                "The Windows file picker could not start. Paste the path below instead.".into();
        }
    }
    fn picked(&mut self, picker: Picker, path: Option<PathBuf>, open_for: Duration) {
        self.picker_busy = false;
        let Some(path) = path else {
            self.notice = if open_for < DIALOG_FAILED_WITHIN {
                "The Windows file picker could not open on this PC. Paste your old launcher folder or invite.json path below, then click Import path."
            } else {
                "Nothing was selected. You can also paste your old launcher folder or invite.json path below, then click Import path."
            }
            .into();
            return;
        };
        self.notice.clear();
        match picker {
            Picker::Game => self.select_game(path),
            Picker::Invitation | Picker::Folder => {
                match resolve_invitation(&path.to_string_lossy()) {
                    Ok(p) => self.import(p),
                    Err(e) => self.error = e,
                }
            }
        }
    }
    fn import_typed(&mut self) {
        match resolve_invitation(&self.invitation_input) {
            Ok(path) => self.import(path),
            Err(e) => self.error = e,
        }
    }
    fn select_game(&mut self, path: PathBuf) {
        if self.busy_with_game() {
            self.error = "Close the game before choosing a different installation.".into();
            return;
        }
        match builds::inspect(&path) {
            Ok(i) => {
                self.settings.game = Some(path);
                self.installation = Some(i);
                self.error.clear();
                self.save();
            }
            Err(e) => self.error = e.to_string(),
        }
        self.last_support = Instant::now() - Duration::from_secs(31);
    }
    fn play(&mut self) {
        if self.launching || self.game.is_some() || self.picker_busy {
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
                Event::Picked(picker, path, open_for) => self.picked(picker, path, open_for),
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
                    self.support_last = Some(r);
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
    fn ui(&mut self, root_ui: &mut egui::Ui, frame: &mut eframe::Frame) {
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
        // Messages first and the rest scrollable: at the default window size the
        // old bottom-of-window message was drawn off-screen ("nothing happened").
        if !self.error.is_empty(){ui.add_space(6.);ui.colored_label(egui::Color32::from_rgb(245,145,110),&self.error);}
        if !self.notice.is_empty(){ui.add_space(6.);ui.label(&self.notice);}
        egui::ScrollArea::vertical().auto_shrink([false,false]).show(ui,|ui|{
        ui.add_space(12.);
        egui::Frame::group(ui.style()).inner_margin(16.).show(ui,|ui|{
            if let Some(i)=&self.invite{ui.heading(format!("Welcome, {}",i.name));ui.label(format!("Server: {}",i.origin()));ui.horizontal(|ui|{
                if ui.button("Copy game account").clicked(){ctx.copy_text(i.account.clone());}
                if let Some(password)=&i.password{if ui.button("Copy game password").clicked(){ctx.copy_text(password.clone());}}else{ui.weak("Use your existing private-server password.");}
            });}else{ui.heading("Connect with your private invitation");ui.label("Use the invitation or folder from your old Friends launcher. Nothing is uploaded to GitHub.");}
            if self.game.is_none()&&!self.launching{
                ui.horizontal(|ui|{
                    if ui.add_enabled(!self.picker_busy, egui::Button::new("Import invitation")).clicked(){self.pick(Picker::Invitation, frame);}
                    if ui.add_enabled(!self.picker_busy, egui::Button::new("Use existing launcher folder")).clicked(){self.pick(Picker::Folder, frame);}
                });
                // Works without any native dialog; stays usable while a dialog is pending.
                ui.label("Or paste your old launcher folder or invite.json path (no file picker needed):");
                ui.horizontal(|ui| {
                    let field = ui.add(egui::TextEdit::singleline(&mut self.invitation_input).desired_width(470.).hint_text("e.g. D:\\WoW Friends\\launcher"));
                    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if ui.button("Import path").clicked() || enter { self.import_typed(); }
                });
            }
        });
        ui.add_space(12.);
        egui::Frame::group(ui.style()).inner_margin(16.).show(ui,|ui|{
            ui.heading("World of Warcraft: Forever");
            if let Some(path)=&self.settings.game{ui.label(path.display().to_string());}else{ui.weak("No game selected.");}
            ui.label(self.preflight());ui.label(&self.status_text);
            if let Some(game)=&self.game{ui.weak(format!("Active game transport connections: {}",game._relay.connections.load(Ordering::SeqCst)));}
            ui.add_enabled_ui(self.game.is_none()&&!self.launching&&!self.picker_busy,|ui|{if ui.button("Choose game — _forever_ / Wow.exe").clicked(){self.pick(Picker::Game, frame);}});
            ui.add_space(10.);
            let ready=self.status.as_ref().is_some_and(|s|s.ready)&&self.invite.is_some()&&self.installation.as_ref().is_some_and(|i|builds::compatibility(i,&self.accepted()).is_ok());
            ui.horizontal(|ui|{
                let label=if self.game.is_some(){"GAME RUNNING"}else if self.launching{"Starting..."}else{"PLAY"};
                if ui.add_enabled(ready&&self.game.is_none()&&!self.launching&&!self.update_busy&&!self.picker_busy,egui::Button::new(label).fill(egui::Color32::from_rgb(34,128,87)).min_size(egui::vec2(180.,42.))).clicked(){self.play();}
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
            // A legacy setting without the current consent version is NOT sending; show it unticked.
            let mut sharing = self.settings.support_diagnostics && self.settings.support_consent_version == 1;
            if ui.checkbox(&mut sharing,"Share private WoW support reports with my server owner").changed(){self.settings.support_diagnostics=sharing;self.support_policy.fetch_add(1,Ordering::SeqCst);self.settings.support_consent_version=if sharing{1}else{0};self.support_last=None;self.last_support=Instant::now()-Duration::from_secs(31);self.save();}
            ui.weak("Only bounded game errors, version, connection/control state and your launched game's responsiveness. No screenshots, passwords or remote control.");
            let summary = support_summary(sharing, self.invite.is_some(), self.support_last.as_ref());
            if sharing && (self.invite.is_none() || self.support_last.as_ref().is_some_and(|r| r.is_err())) { ui.colored_label(egui::Color32::from_rgb(245,190,110), summary); } else { ui.label(summary); }
        });
        });
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
            // Theme first: set_visuals() only styles the *current* theme, which
            // follows Windows (light mode would otherwise discard these colours).
            cc.egui_ctx.set_theme(egui::ThemePreference::Dark);
            cc.egui_ctx.set_visuals_of(egui::Theme::Dark, visuals);
            Ok(Box::new(App::new(root, rt)))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    use std::fs;

    const INVITE: &str = r#"{"name":"Fixture","account":"fixture@local.invalid","token":"fixture-only","relay":"https://example.invalid","allowedBuilds":[]}"#;

    /// An old-launcher folder whose name has spaces and non-ASCII characters.
    fn old_launcher(base: &std::path::Path) -> PathBuf {
        let folder = base.join("Stari launcher čćžšđ 世界 ✓");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("invite.json"), INVITE).unwrap();
        fs::write(folder.join("Start WoW.exe"), b"not a real exe").unwrap();
        folder
    }
    fn app_root(base: &std::path::Path) -> PathBuf {
        let root = base.join("data");
        fs::create_dir_all(&root).unwrap();
        // No network update checks from tests.
        fs::write(root.join("settings.json"), r#"{"auto_update":false}"#).unwrap();
        root
    }
    fn harness(root: PathBuf, dialog: Option<DialogRunner>) -> Harness<'static, App> {
        harness_sized(root, dialog, egui::vec2(840., 900.))
    }
    fn harness_sized(
        root: PathBuf,
        dialog: Option<DialogRunner>,
        size: egui::Vec2,
    ) -> Harness<'static, App> {
        let rt = Arc::new(tokio::runtime::Runtime::new().unwrap());
        let mut h = Harness::builder()
            .with_size(size)
            // Same as the release app: unknown glyphs (e.g. CJK) draw as boxes.
            .allow_missing_glyphs()
            .build_eframe(move |_| {
                let mut app = App::new(root, rt);
                if let Some(d) = dialog {
                    app.dialog = d;
                }
                app
            });
        h.step();
        h
    }
    fn wait_until(h: &mut Harness<'static, App>, done: impl Fn(&App) -> bool) {
        for _ in 0..300 {
            h.step();
            if done(h.state()) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!(
            "UI never reached the expected state; notice={:?} error={:?}",
            h.state().notice,
            h.state().error
        );
    }

    #[test]
    fn resolves_quoted_spaced_unicode_folders_and_files() {
        let d = tempfile::tempdir().unwrap();
        let folder = old_launcher(d.path());
        let expected = folder.join("invite.json");
        let f = folder.display().to_string();
        for input in [
            f.clone(),
            format!("\"{f}\""),
            format!("  '{f}'  "),
            format!("\"{}\"", expected.display()),
            folder.join("Start WoW.exe").display().to_string(),
        ] {
            assert_eq!(
                resolve_invitation(&input).unwrap(),
                expected,
                "input {input}"
            );
        }
    }
    #[test]
    fn explains_missing_invitation_and_folder() {
        let d = tempfile::tempdir().unwrap();
        let empty = d.path().join("prazna mapa ž");
        fs::create_dir_all(&empty).unwrap();
        assert!(
            resolve_invitation(&empty.display().to_string())
                .unwrap_err()
                .contains("No invite.json")
        );
        assert!(
            resolve_invitation(&d.path().join("nema").join("x").display().to_string())
                .unwrap_err()
                .contains("does not exist")
        );
        assert!(
            resolve_invitation("  \"\"  ")
                .unwrap_err()
                .contains("Paste")
        );
    }
    #[test]
    fn support_text_never_says_off_while_ticked() {
        assert!(support_summary(true, false, None).contains("nothing can be sent yet"));
        assert!(!support_summary(true, false, None).contains("off"));
        assert!(support_summary(false, true, None).starts_with("Uploads off"));
        assert!(
            support_summary(
                true,
                true,
                Some(&Err("the server answered HTTP 500.".into()))
            )
            .contains("NOT delivered")
        );
        assert!(support_summary(true, true, Some(&Ok(()))).contains("delivered"));
    }

    #[test]
    fn ui_imports_typed_path_without_any_picker() {
        let d = tempfile::tempdir().unwrap();
        let folder = old_launcher(d.path());
        let never: DialogRunner = Arc::new(|_, _| panic!("native picker must not be used"));
        let mut h = harness(app_root(d.path()), Some(never));
        assert!(h.state().invite.is_none());
        let typed = format!("\"{}\"", folder.display());
        h.get_by_role(egui::accesskit::Role::TextInput).focus();
        h.step();
        h.get_by_role(egui::accesskit::Role::TextInput)
            .type_text(&typed);
        h.step();
        assert_eq!(h.state().invitation_input, typed);
        h.get_by_label("Import path").click();
        wait_until(&mut h, |a| a.invite.is_some());
        assert!(
            h.state().notice.contains("Invitation imported"),
            "{}",
            h.state().notice
        );
        assert!(h.state().root.join("invitation.dpapi").is_file());
    }
    #[test]
    fn ui_folder_button_imports_through_worker_thread() {
        let d = tempfile::tempdir().unwrap();
        let folder = old_launcher(d.path());
        let chosen = folder.clone();
        let fake: DialogRunner = Arc::new(move |p, _| {
            assert_eq!(p, Picker::Folder);
            std::thread::sleep(Duration::from_millis(500));
            Some(chosen.clone())
        });
        let mut h = harness(app_root(d.path()), Some(fake));
        h.get_by_label("Use existing launcher folder").click();
        h.step();
        h.step();
        // Pending dialog: render thread keeps running, paste field stays usable.
        assert!(h.state().picker_busy);
        assert!(h.query_by_label("Import path").is_some());
        wait_until(&mut h, |a| a.invite.is_some() && !a.picker_busy);
        assert!(h.state().error.is_empty(), "{}", h.state().error);
    }
    #[test]
    fn ui_reports_picker_that_cannot_open() {
        let d = tempfile::tempdir().unwrap();
        let fails: DialogRunner = Arc::new(|_, _| None);
        let mut h = harness(app_root(d.path()), Some(fails));
        h.get_by_label("Use existing launcher folder").click();
        wait_until(&mut h, |a| !a.picker_busy && !a.notice.contains("opened"));
        assert!(
            h.state().notice.contains("could not open"),
            "{}",
            h.state().notice
        );
        assert!(h.state().invite.is_none());
    }
    #[test]
    fn ui_reports_folder_without_invitation_and_invalid_invitation() {
        let d = tempfile::tempdir().unwrap();
        let empty = d.path().join("Bez pozivnice ✗");
        fs::create_dir_all(&empty).unwrap();
        let bad = d.path().join("Pokvarena");
        fs::create_dir_all(&bad).unwrap();
        fs::write(bad.join("invite.json"), r#"{"name":"x"}"#).unwrap();
        let mut h = harness(app_root(d.path()), None);
        h.state_mut().invitation_input = empty.display().to_string();
        h.get_by_label("Import path").click();
        h.step();
        assert!(
            h.state().error.contains("No invite.json"),
            "{}",
            h.state().error
        );
        h.step(); // let the layout settle after the message appears at the top
        h.state_mut().invitation_input = bad.display().to_string();
        h.get_by_label("Import path").click();
        h.step();
        assert!(
            h.state().error.contains("not a valid private invitation"),
            "{}",
            h.state().error
        );
        assert!(h.state().invite.is_none());
        assert!(!h.state().root.join("invitation.dpapi").exists());
    }
    #[test]
    fn invitation_cannot_switch_while_game_is_starting() {
        let d = tempfile::tempdir().unwrap();
        let folder = old_launcher(d.path());
        let mut h = harness(app_root(d.path()), None);
        h.state_mut().launching = true;
        h.step();
        assert!(h.query_by_label("Import path").is_none());
        assert!(h.query_by_label("Use existing launcher folder").is_none());
        // A late picker result must also be refused.
        h.state_mut()
            .picked(Picker::Folder, Some(folder), Duration::from_secs(3));
        assert!(h.state().invite.is_none());
        assert!(h.state().error.contains("Close the game"));
    }
    #[test]
    fn ui_shows_why_ticked_reports_are_not_sent_without_invitation() {
        let d = tempfile::tempdir().unwrap();
        let root = app_root(d.path());
        fs::write(
            root.join("settings.json"),
            r#"{"auto_update":false,"support_diagnostics":true,"support_consent_version":1}"#,
        )
        .unwrap();
        let mut h = harness(root, None);
        h.step();
        assert!(
            h.query_by_label_contains("nothing can be sent yet")
                .is_some()
        );
    }
    #[test]
    fn picker_messages_are_visible_in_default_window() {
        let d = tempfile::tempdir().unwrap();
        let fails: DialogRunner = Arc::new(|_, _| None);
        // The real default window size (see run()).
        let mut h = harness_sized(app_root(d.path()), Some(fails), egui::vec2(840., 760.));
        h.get_by_label("Use existing launcher folder").click();
        wait_until(&mut h, |a| a.notice.contains("could not open"));
        h.step();
        let notice = h.get_by_label_contains("could not open");
        assert!(
            notice.rect().max.y <= 760.,
            "notice drawn off-screen at {:?}",
            notice.rect()
        );
        h.state_mut().invitation_input = "Z:\nowhere at all".into();
        h.get_by_label("Import path").click();
        h.step();
        h.step();
        let error = h.get_by_label_contains("does not exist");
        assert!(
            error.rect().max.y <= 760.,
            "error drawn off-screen at {:?}",
            error.rect()
        );
    }
}
