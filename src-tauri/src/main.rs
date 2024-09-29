#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;

use std::{
    borrow::{BorrowMut, Cow},
    fmt, fs,
    future::Future,
    str::FromStr,
    sync::{Arc, OnceLock, RwLock},
};

use anyhow::{Context, Result as AnyhowResult};
use config::Config;
#[cfg(feature = "auto-splitting")]
use livesplit_auto_splitting::{
    settings, time, AutoSplitter, Config as AutoSplitConfig, LogLevel, Runtime,
    Timer as AutoSplitTimer, TimerState,
};
use livesplit_core::{
    event::{self, CommandSink, Event, Result},
    hotkey::KeyCode,
    networking::server_protocol::{self, Command, CommandResult, Response},
    HotkeyConfig, HotkeySystem, TimeSpan, TimingMethod,
};
use tauri::{async_runtime::Receiver, Manager, Window};
use tokio::sync::mpsc;

struct State {
    #[cfg(feature = "auto-splitting")]
    timer: RwLock<Option<TauriTimer>>,
    #[cfg(feature = "auto-splitting")]
    runtime: RwLock<Option<AutoSplitter<TauriTimer>>>,
    config: Arc<RwLock<Config>>,
    hotkey_system: RwLock<Option<HotkeySystem<TauriCommandSink>>>,
    window: RwLock<Option<Window>>,
}

impl State {
    fn new(config: Config, hotkey_system: RwLock<Option<HotkeySystem<TauriCommandSink>>>) -> Self {
        config.setup_logging();

        Self {
            #[cfg(feature = "auto-splitting")]
            timer: RwLock::new(None),
            #[cfg(feature = "auto-splitting")]
            runtime: RwLock::new(None),
            config: Arc::new(RwLock::new(config)),
            hotkey_system,
            window: RwLock::new(None),
        }
    }
}

#[tauri::command]
fn set_hotkey_config(state: tauri::State<'_, State>, config: HotkeyConfig) -> bool {
    let b = if let Some(hotkey_system) = &mut *state.hotkey_system.write().unwrap() {
        hotkey_system.set_config(config).is_ok()
    } else {
        false
    };
    state.config.write().unwrap().set_hotkeys(config);
    b
}

#[tauri::command]
fn set_hotkey_activation(state: tauri::State<'_, State>, active: bool) -> bool {
    if let Some(hotkey_system) = &mut *state.hotkey_system.write().unwrap() {
        if active {
            hotkey_system.activate()
        } else {
            hotkey_system.deactivate()
        }
        .is_ok()
    } else {
        false
    }
}

#[tauri::command]
fn get_hotkey_config(state: tauri::State<'_, State>) -> HotkeyConfig {
    if let Some(hotkey_system) = &*state.hotkey_system.read().unwrap() {
        hotkey_system.config()
    } else {
        HotkeyConfig::default()
    }
}

#[tauri::command]
fn resolve_hotkey(state: tauri::State<'_, State>, key_code: String) -> Cow<'static, str> {
    if let Some(hotkey_system) = &*state.hotkey_system.read().unwrap() {
        if let Ok(key_code) = KeyCode::from_str(&key_code) {
            return hotkey_system.resolve(key_code);
        }
    }
    key_code.into()
}

#[tauri::command]
fn settings_changed(state: tauri::State<'_, State>, always_on_top: bool) {
    if let Some(window) = &*state.window.read().unwrap() {
        window.set_always_on_top(always_on_top).unwrap();
    }
}

#[derive(Clone)]
struct TauriCommandSink(Arc<RwLock<Option<Window>>>, Arc<RwLock<Receiver<String>>>);

impl TauriCommandSink {
    fn send(&self, command: Command) {
        self.0
            .read()
            .unwrap()
            .as_ref()
            .unwrap()
            .emit("command", command)
            .unwrap();
        let r = self.1.write().unwrap().try_recv();
        log::info!("TauriCommandSink send: r = {:?}", r);
    }
}

impl CommandSink for TauriCommandSink {
    fn start(&self) -> impl Future<Output = Result> + 'static {
        self.send(Command::Start);
        async { Ok(Event::Unknown) }
    }

    fn split(&self) -> impl Future<Output = Result> + 'static {
        self.send(Command::Split);
        async { Ok(Event::Unknown) }
    }

    fn split_or_start(&self) -> impl Future<Output = Result> + 'static {
        self.send(Command::SplitOrStart);
        async { Ok(Event::Unknown) }
    }

    fn reset(&self, save_attempt: Option<bool>) -> impl Future<Output = Result> + 'static {
        self.send(Command::Reset { save_attempt });
        async { Ok(Event::Unknown) }
    }

    fn undo_split(&self) -> impl Future<Output = Result> + 'static {
        self.send(Command::UndoSplit);
        async { Ok(Event::Unknown) }
    }

    fn skip_split(&self) -> impl Future<Output = Result> + 'static {
        self.send(Command::SkipSplit);
        async { Ok(Event::Unknown) }
    }

    fn toggle_pause_or_start(&self) -> impl Future<Output = Result> + 'static {
        self.send(Command::TogglePauseOrStart);
        async { Ok(Event::Unknown) }
    }

    fn pause(&self) -> impl Future<Output = Result> + 'static {
        self.send(Command::Pause);
        async { Ok(Event::Unknown) }
    }

    fn resume(&self) -> impl Future<Output = Result> + 'static {
        self.send(Command::Resume);
        async { Ok(Event::Unknown) }
    }

    fn undo_all_pauses(&self) -> impl Future<Output = Result> + 'static {
        self.send(Command::UndoAllPauses);
        async { Ok(Event::Unknown) }
    }

    fn switch_to_previous_comparison(&self) -> impl Future<Output = Result> + 'static {
        self.send(Command::SwitchToPreviousComparison);
        async { Ok(Event::Unknown) }
    }

    fn switch_to_next_comparison(&self) -> impl Future<Output = Result> + 'static {
        self.send(Command::SwitchToNextComparison);
        async { Ok(Event::Unknown) }
    }

    fn set_current_comparison(
        &self,
        comparison: Cow<'_, str>,
    ) -> impl Future<Output = Result> + 'static {
        self.send(Command::SetCurrentComparison { comparison });
        async { Ok(Event::Unknown) }
    }

    fn toggle_timing_method(&self) -> impl Future<Output = Result> + 'static {
        self.send(Command::ToggleTimingMethod);
        async { Ok(Event::Unknown) }
    }

    fn set_current_timing_method(
        &self,
        method: TimingMethod,
    ) -> impl Future<Output = Result> + 'static {
        self.send(Command::SetCurrentTimingMethod {
            timing_method: method,
        });
        async { Ok(Event::Unknown) }
    }

    fn initialize_game_time(&self) -> impl Future<Output = Result> + 'static {
        self.send(Command::InitializeGameTime);
        async { Ok(Event::Unknown) }
    }

    fn set_game_time(&self, time: TimeSpan) -> impl Future<Output = Result> + 'static {
        self.send(Command::SetGameTime { time });
        async { Ok(Event::Unknown) }
    }

    fn pause_game_time(&self) -> impl Future<Output = Result> + 'static {
        self.send(Command::PauseGameTime);
        async { Ok(Event::Unknown) }
    }

    fn resume_game_time(&self) -> impl Future<Output = Result> + 'static {
        self.send(Command::ResumeGameTime);
        async { Ok(Event::Unknown) }
    }

    fn set_loading_times(&self, time: TimeSpan) -> impl Future<Output = Result> + 'static {
        self.send(Command::SetLoadingTimes { time });
        async { Ok(Event::Unknown) }
    }

    fn set_custom_variable(
        &self,
        key: Cow<'_, str>,
        value: Cow<'_, str>,
    ) -> impl Future<Output = Result> + 'static {
        self.send(Command::SetCustomVariable { key, value });
        async { Ok(Event::Unknown) }
    }
}

#[cfg(feature = "auto-splitting")]
#[derive(Clone)]
struct TauriTimer(Arc<RwLock<Option<Window>>>, Arc<RwLock<Receiver<String>>>);

#[cfg(feature = "auto-splitting")]
impl TauriTimer {
    fn send(&self, command: Command) {
        self.0
            .read()
            .unwrap()
            .as_ref()
            .unwrap()
            .emit("command", command)
            .unwrap();
        let r = self.1.write().unwrap().try_recv();
        log::info!("TauriTimer send: r = {:?}", r);
    }
    fn send_receive(&self, command: Command) -> CommandResult<Response, server_protocol::Error> {
        self.0
            .read()
            .unwrap()
            .as_ref()
            .unwrap()
            .emit("command", command)
            .unwrap();
        for _ in 0..10000 {
            if let Ok(r) = self.1.write().unwrap().try_recv() {
                log::info!("TauriTimer send_receive: r = {:?}", r);
                serde_json::from_str(&r).unwrap()
            }
        }
        log::error!("TauriTimer send_receive: 10,000 failures");
        CommandResult::Error(server_protocol::Error::Timer { code: event::Error::Unknown })
    }
}

#[cfg(feature = "auto-splitting")]
impl AutoSplitTimer for TauriTimer {
    fn state(&self) -> TimerState {
        match self.send_receive(Command::GetCurrentState) {
            CommandResult::Success(Response::State(s)) => {
                match s {
                    server_protocol::State::NotRunning => TimerState::NotRunning,
                    server_protocol::State::Running(_) => TimerState::Running,
                    server_protocol::State::Paused(_) => TimerState::Paused,
                    server_protocol::State::Ended => TimerState::Ended,
                }
            }
            r => {
                log::error!("expected success state, given {:?}", serde_json::to_string(&r).unwrap());
                TimerState::NotRunning
            }
        }
    }

    fn start(&mut self) {
        self.send(Command::Start)
    }

    fn split(&mut self) {
        self.send(Command::Split)
    }

    fn skip_split(&mut self) {
        self.send(Command::SkipSplit)
    }

    fn undo_split(&mut self) {
        self.send(Command::UndoSplit)
    }

    fn reset(&mut self) {
        // TODO: configure save_attempt
        self.send(Command::Reset { save_attempt: None })
    }

    fn set_game_time(&mut self, time: time::Duration) {
        self.send(Command::SetGameTime { time: time.into() })
    }

    fn pause_game_time(&mut self) {
        self.send(Command::PauseGameTime)
    }

    fn resume_game_time(&mut self) {
        self.send(Command::ResumeGameTime)
    }

    fn set_variable(&mut self, key: &str, value: &str) {
        self.send(Command::SetCustomVariable {
            key: key.into(),
            value: value.into(),
        });
    }

    fn log_auto_splitter(&mut self, message: fmt::Arguments<'_>) {
        log::info!("{}", message);
    }

    fn log_runtime(&mut self, message: fmt::Arguments<'_>, log_level: LogLevel) {
        log::info!("{}", message);
    }
}

fn main() {
    let (response_sender, response_receiver) = mpsc::channel::<String>(16);
    let response_receiver_box = Arc::new(RwLock::new(response_receiver));
    let sink = TauriCommandSink(Arc::new(RwLock::new(None)), response_receiver_box.clone());
    #[cfg(feature = "auto-splitting")]
    let timer = TauriTimer(Arc::new(RwLock::new(None)), response_receiver_box);
    let config = Config::load();
    let hotkey_system = RwLock::new(config.configure_hotkeys(sink.clone()));
    tauri::Builder::default()
        .manage(State::new(config, hotkey_system))
        .setup(move |app| {
            let main_window = app.windows().values().next().unwrap().clone();
            main_window.listen("response", move |e| {
                log::info!("listen response callback: {:?}", e.payload());
                response_sender.try_send(e.payload().unwrap().to_string()).ok();
            });
            app.state::<State>()
                .window
                .write()
                .unwrap()
                .replace(main_window.clone());
            #[cfg(feature = "auto-splitting")]
            let _ = *timer.0.write().unwrap() = Some(main_window.clone());
            *sink.0.write().unwrap() = Some(main_window);
            #[cfg(feature = "auto-splitting")]
            let runtime = app
                .state::<State>()
                .config
                .read()
                .unwrap()
                .runtime_new(None, timer)
                .unwrap();
            #[cfg(feature = "auto-splitting")]
            if let Some(r) = runtime {
                app.state::<State>().runtime.write().unwrap().replace(r);
                log::info!("before spawn");
                let app_handle = app.handle();
                tauri::async_runtime::spawn(async move {
                    let mut i = 0;
                    let mut c = 0;
                    let mut d = 1;
                    loop {
                        if c == 0 {
                            log::info!("before update: {}", i);
                        }
                        app_handle.state::<State>().runtime.read().unwrap().as_ref().unwrap().lock().update().unwrap();
                        if c == 0 {
                            log::info!("after update: {}", i);
                            d *= 10;
                            c = d;
                        }
                        i += 1;
                        c -= 1;
                    }
                });
                log::info!("after spawn");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            set_hotkey_config,
            set_hotkey_activation,
            get_hotkey_config,
            resolve_hotkey,
            settings_changed,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
