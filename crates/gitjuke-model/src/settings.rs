use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub id: u64,
    pub login: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Theme {
    pub mode: String,
    pub canvas: String,
    pub surface: String,
    pub accent: String,
    pub ink: String,
}
impl Default for Theme {
    fn default() -> Self {
        Self {
            mode: "dark".into(),
            canvas: "#121512".into(),
            surface: "#1c211b".into(),
            accent: "#b8d68a".into(),
            ink: "#e7ecdf".into(),
        }
    }
}
impl Theme {
    pub fn valid(&self) -> bool {
        ["light", "dark"].contains(&self.mode.as_str())
            && [&self.canvas, &self.surface, &self.accent, &self.ink]
                .iter()
                .all(|v| {
                    v.len() == 7
                        && v.starts_with('#')
                        && v[1..].bytes().all(|b| b.is_ascii_hexdigit())
                })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default)]
    pub alerts: AlertSettings,
    #[serde(default)]
    pub paged_changes: bool,
    pub paused: bool,
    pub theme: Theme,
    pub gh_path: Option<String>,
    pub config_dir: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum Permission {
    #[default]
    Unknown,
    NotDetermined,
    Denied,
    Granted,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AlertSettings {
    pub onboarded: bool,
    pub enabled: bool,
    pub previews: bool,
    pub categories: [bool; 5],
    pub since: [u64; 5],
    pub permission: Permission,
    pub floor: i64,
}
impl Default for AlertSettings {
    fn default() -> Self {
        Self {
            onboarded: false,
            enabled: false,
            previews: false,
            categories: [true; 5],
            since: [0; 5],
            permission: Permission::Unknown,
            floor: 0,
        }
    }
}
