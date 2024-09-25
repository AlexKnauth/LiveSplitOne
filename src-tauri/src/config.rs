use anyhow::{Context, Result};
use directories::ProjectDirs;
#[cfg(feature = "auto-splitting")]
use livesplit_auto_splitting::AutoSplitter;
use livesplit_core::{
    comparison,
    event::{self, CommandSink, TimerQuery},
    run::{
        parser::{composite, TimerKind},
        saver::livesplit::save_timer,
    },
    HotkeyConfig, HotkeySystem, Run, Segment, TimingMethod,
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

    // Just directly construct the HotkeySystem from the config.
    pub fn configure_hotkeys<E: event::CommandSink + Clone + Send + 'static>(
        &self,
        command_sink: E,
    ) -> Option<HotkeySystem<E>> {
        HotkeySystem::with_config(command_sink, self.hotkeys).ok()
    }

    pub fn set_hotkeys(&mut self, hotkeys: HotkeyConfig) {
        self.hotkeys = hotkeys;
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
