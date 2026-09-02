use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

/// How an adapter should recognize event types. `MarkerTrait` looks for `impl <trait>
/// for X`; `NamingConvention` matches a type's bare name against `event_naming_suffix`
/// (e.g. `PaymentCreatedEvent` with suffix `"Event"`) — deliberately a plain suffix
/// check, not a regex engine, to avoid pulling in a dependency for something this simple.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventStrategy {
    #[default]
    MarkerTrait,
    NamingConvention,
}

/// Detector configuration, loaded from `impact.toml` at a project's root. Every field has
/// a sensible default, so a project with no config file still gets useful API/EVENTS/
/// DATABASE detection out of the box.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct DetectorConfig {
    pub api_frameworks: Vec<String>,
    pub event_strategy: EventStrategy,
    pub event_marker_trait: String,
    pub event_naming_suffix: String,
    pub database_macros: Vec<String>,
}

impl Default for DetectorConfig {
    fn default() -> Self {
        Self {
            api_frameworks: vec![
                "axum".to_string(),
                "net/http".to_string(),
                "fastapi".to_string(),
                "flask".to_string(),
                "express".to_string(),
                "fastify".to_string(),
            ],
            event_strategy: EventStrategy::default(),
            event_marker_trait: "Event".to_string(),
            event_naming_suffix: "Event".to_string(),
            database_macros: vec![
                "query".to_string(),
                "query_as".to_string(),
                "query_scalar".to_string(),
            ],
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ImpactToml {
    detectors: DetectorConfigToml,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct DetectorConfigToml {
    api: ApiToml,
    events: EventsToml,
    database: DatabaseToml,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ApiToml {
    frameworks: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct EventsToml {
    strategy: Option<EventStrategy>,
    marker_trait: Option<String>,
    naming_suffix: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct DatabaseToml {
    macros: Option<Vec<String>>,
}

impl DetectorConfig {
    /// Loads `<project_root>/impact.toml` if present, overlaying only the fields it sets
    /// onto the defaults. A missing file is not an error — it just means defaults.
    pub fn load(project_root: &Path) -> Result<Self> {
        let path = project_root.join("impact.toml");
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let parsed: ImpactToml =
            toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;

        let mut config = Self::default();
        if let Some(frameworks) = parsed.detectors.api.frameworks {
            config.api_frameworks = frameworks;
        }
        if let Some(strategy) = parsed.detectors.events.strategy {
            config.event_strategy = strategy;
        }
        if let Some(marker_trait) = parsed.detectors.events.marker_trait {
            config.event_marker_trait = marker_trait;
        }
        if let Some(naming_suffix) = parsed.detectors.events.naming_suffix {
            config.event_naming_suffix = naming_suffix;
        }
        if let Some(macros) = parsed.detectors.database.macros {
            config.database_macros = macros;
        }
        Ok(config)
    }
}
