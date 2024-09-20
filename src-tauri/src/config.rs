use anyhow::{Context, Result};
use directories::ProjectDirs;
use livesplit_core::{
    comparison, event,
    run::{
        parser::{composite, TimerKind},
        saver::livesplit::save_timer,
    },
    HotkeyConfig, HotkeySystem, Run, Segment, SharedTimer, Timer, TimingMethod,
};
use log::error;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, create_dir_all},
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct Config {
    #[serde(default)]
    splits: Splits,
    #[serde(default)]
    general: General,
    #[serde(default)]
    log: Log,
    #[serde(default)]
    hotkeys: HotkeyConfig,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct Splits {
    current: Option<PathBuf>,
    #[serde(skip)]
    can_save: bool,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct General {
    timing_method: Option<TimingMethod>,
    comparison: Option<String>,
    auto_splitter: Option<PathBuf>,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct Log {
    #[serde(default)]
    enable: bool,
    level: Option<log::LevelFilter>,
    #[serde(default)]
    clear: bool,
}

static CONFIG_PATH: Lazy<PathBuf> = Lazy::new(|| {
    ProjectDirs::from("org", "LiveSplit", "LiveSplit One Tauri")
        .map(|dirs| dirs.data_local_dir().join("config.yml"))
        .unwrap_or_default()
});

impl Config {
    pub fn load() -> Self {
        Self::parse().unwrap_or_default()
    }

    fn save_config(&self) -> Option<()> {
        create_dir_all(CONFIG_PATH.parent()?).ok()?;
        self.serialize()
    }

    fn parse() -> Option<Self> {
        let buf = fs::read(CONFIG_PATH.as_path()).ok()?;
        serde_yaml::from_slice(&buf).ok()
    }

    fn serialize(&self) -> Option<()> {
        let buf = serde_yaml::to_string(self).ok()?;
        fs::write(CONFIG_PATH.as_path(), buf).ok()
    }

    fn parse_run(&self) -> Option<(Run, bool)> {
        let path = self.splits.current.clone()?;
        let file = fs::read(&path).ok()?;
        let parsed_run = composite::parse(&file, Some(&path)).ok()?;
        let run = parsed_run.run;
        let can_save = parsed_run.kind == TimerKind::LiveSplit;
        Some((run, can_save))
    }

    pub fn parse_run_or_default(&mut self) -> Run {
        match self.parse_run() {
            Some((run, can_save)) => {
                self.splits.can_save = can_save;
                run
            }
            None => {
                self.splits.can_save = false;
                default_run()
            }
        }
    }

    pub fn is_game_time(&self) -> bool {
        self.general.timing_method == Some(TimingMethod::GameTime)
    }

    // Just directly construct the HotkeySystem from the config.
    pub fn configure_hotkeys<E: event::CommandSink + Clone + Send + 'static>(
        &self,
        command_sink: E,
    ) -> Option<HotkeySystem<E>> {
        HotkeySystem::with_config(command_sink, self.hotkeys).ok()
    }

    pub fn configure_timer(&self, timer: &mut Timer) {
        if self.is_game_time() {
            timer.set_current_timing_method(TimingMethod::GameTime);
        }
        if let Some(comparison) = &self.general.comparison {
            timer.set_current_comparison(comparison.as_str()).ok();
        }
    }

    pub fn set_hotkeys(&mut self, hotkeys: HotkeyConfig) {
        self.hotkeys = hotkeys;
        self.save_config();
    }

    pub fn new_splits(&mut self, timer: &mut Timer) {
        timer.set_run(default_run()).map_err(drop).unwrap();
        self.splits.can_save = false;
        self.splits.current = None;
        self.save_config();
    }

    pub fn open_splits(
        &mut self,
        shared_timer: &SharedTimer,
        #[cfg(feature = "auto-splitting")] runtime: &livesplit_core::auto_splitting::Runtime<
            SharedTimer,
        >,
        path: PathBuf,
    ) -> Result<()> {
        {
            let timer = &mut shared_timer.write().unwrap();
            let file = fs::read(&path).context("Failed reading the file.")?;
            let run = composite::parse(&file, Some(&path)).context("Failed parsing the file.")?;
            timer.set_run(run.run).ok().context(
                "The splits can't be used with the timer because they don't contain a single segment.",
            )?;

            self.splits.can_save = run.kind == TimerKind::LiveSplit;
            self.splits.current = Some(path);

            self.save_config();
        }

        #[cfg(feature = "auto-splitting")]
        // TODO: runtime.reload
        if let Some(auto_splitter) = &self.general.auto_splitter {
            runtime.unload()?;
            runtime.load(auto_splitter.clone(), shared_timer.clone())?;
        }

        Ok(())
    }

    pub fn can_directly_save_splits(&self) -> bool {
        self.splits.current.is_some() && self.splits.can_save
    }

    pub fn save_splits(
        &mut self,
        timer: &mut Timer,
        #[cfg(feature = "auto-splitting")] runtime: &livesplit_core::auto_splitting::Runtime<
            SharedTimer,
        >,
    ) -> Result<()> {
        if let Some(path) = &self.splits.current {
            // TODO: run auto splitter settings map store
            let mut buf = String::new();
            save_timer(timer, &mut buf).context("Failed saving the splits.")?;
            fs::write(path, &buf).context("Failed writing the file.")?;
            timer.mark_as_unmodified();

            self.save_config();
        }
        Ok(())
    }

    pub fn save_splits_as(
        &mut self,
        timer: &mut Timer,
        #[cfg(feature = "auto-splitting")] runtime: &livesplit_core::auto_splitting::Runtime<
            SharedTimer,
        >,
        path: PathBuf,
    ) -> Result<()> {
        // TODO: run auto splitter settings map store
        let mut buf = String::new();
        save_timer(timer, &mut buf).context("Failed saving the splits.")?;
        fs::write(&path, &buf).context("Failed writing the file.")?;
        timer.mark_as_unmodified();

        self.splits.current = Some(path);
        self.splits.can_save = true;

        self.save_config();
        Ok(())
    }

    pub fn open_auto_splitter(
        &mut self,
        #[cfg(feature = "auto-splitting")] shared_timer: &SharedTimer,
        #[cfg(feature = "auto-splitting")] runtime: &livesplit_core::auto_splitting::Runtime<
            SharedTimer,
        >,
        path: &Path,
    ) -> Result<()> {
        self.general.auto_splitter = Some(path.into());
        self.save_config();
        #[cfg(feature = "auto-splitting")]
        runtime.unload()?;
        #[cfg(feature = "auto-splitting")]
        runtime.load(path.into(), shared_timer.clone())?;
        Ok(())
    }

    pub fn maybe_load_auto_splitter(
        &self,
        #[cfg(feature = "auto-splitting")] shared_timer: &SharedTimer,
        #[cfg(feature = "auto-splitting")] runtime: &livesplit_core::auto_splitting::Runtime<
            SharedTimer,
        >,
    ) {
        #[cfg(feature = "auto-splitting")]
        if let Some(auto_splitter) = &self.general.auto_splitter {
            if let Err(e) = runtime.load(auto_splitter.clone(), shared_timer.clone()) {
                // TODO: Error chain
                log::error!("Auto Splitter failed to load: {}", e);
            }
        }
    }

    pub fn set_comparison(&mut self, comparison: String) {
        self.general.comparison = Some(comparison);
        self.save_config();
    }

    pub fn set_timing_method(&mut self, timing_method: TimingMethod) {
        self.general.timing_method = Some(timing_method);
        self.save_config();
    }

    pub fn setup_logging(&self) -> Option<()> {
        if self.log.enable {
            let config_folder = CONFIG_PATH.parent()?;
            create_dir_all(config_folder).ok()?;

            let log_file = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .append(!self.log.clear)
                .truncate(self.log.clear)
                .open(config_folder.join("log.txt"))
                .ok()?;

            fern::Dispatch::new()
                .format(|out, message, record| {
                    out.finish(format_args!(
                        "{}[{}][{}] {}",
                        chrono::Local::now().format("[%Y-%m-%d %H:%M:%S]"),
                        record.target(),
                        record.level(),
                        message
                    ))
                })
                .level(self.log.level.unwrap_or(log::LevelFilter::Warn))
                .chain(log_file)
                .apply()
                .ok()?;
        }
        Some(())
    }
}

fn default_run() -> Run {
    let mut run = Run::new();
    run.push_segment(Segment::new("Time"));
    run
}
