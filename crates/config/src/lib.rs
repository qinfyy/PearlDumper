use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

const DEFAULT_CONFIG: &str = include_str!("../Config.toml");
const CONFIG_ENV: &str = "CASTORICE_CONFIG";
const CONFIG_FILE_NAME: &str = "Config.toml";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputKind {
    Proto,
    Schema,
    Csharp,
}

impl OutputKind {
    pub const ALL: [Self; 3] = [Self::Proto, Self::Schema, Self::Csharp];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Proto => "proto",
            Self::Schema => "schema",
            Self::Csharp => "csharp",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub output: OutputConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            output: OutputConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OutputConfig {
    #[serde(default = "default_stages")]
    pub stages: Vec<OutputKind>,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            stages: default_stages(),
        }
    }
}

fn default_stages() -> Vec<OutputKind> {
    OutputKind::ALL.to_vec()
}

impl Config {
    pub fn load() -> Result<Self> {
        if let Some(path) = env::var_os(CONFIG_ENV) {
            let path = PathBuf::from(path);
            let contents =
                fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            return Self::parse(&contents, Some(&path));
        }

        let path = Path::new(CONFIG_FILE_NAME);
        if path.is_file() {
            let contents =
                fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
            return Self::parse(&contents, Some(path));
        }

        Self::parse(DEFAULT_CONFIG, None)
    }

    pub fn parse(contents: &str, path: Option<&Path>) -> Result<Self> {
        let config = toml::from_str::<Self>(contents).with_context(|| {
            path.map(|path| format!("parsing {}", path.display()))
                .unwrap_or_else(|| "parsing embedded Config.toml".to_string())
        })?;
        config.validate(path)?;
        Ok(config)
    }

    fn validate(&self, path: Option<&Path>) -> Result<()> {
        if self.output.stages.is_empty() {
            let location = path
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "Config.toml".to_string());
            bail!("{location} must select at least one output stage");
        }

        for (index, stage) in self.output.stages.iter().enumerate() {
            if self.output.stages[..index].contains(stage) {
                bail!(
                    "{} contains duplicate output stage '{}', remove the duplicate",
                    path.map(|path| path.display().to_string())
                        .unwrap_or_else(|| "Config.toml".to_string()),
                    stage.name()
                );
            }
        }

        Ok(())
    }
}
