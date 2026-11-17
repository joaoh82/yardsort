//! A stand-in for the program that serves the dashboard: a host that is whatever a test, or the
//! example that shows the page, says it is.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use host_dashboard::{About, Project, RemoteLink, Source};
use yardsort_core::env::{EnvSource, ShellEnv};
use yardsort_core::settings::SettingsFile;

pub struct StandIn {
    pub settings: SettingsFile,
    pub env: Arc<ShellEnv>,
    pub projects: Mutex<Vec<Project>>,
    pub about: About,
    /// Where a clone "goes".
    pub projects_dir: PathBuf,
}

impl StandIn {
    /// A host whose home is `home`, whose programs are `bin`'s and the system's, and whose
    /// settings are `home`'s `settings.toml`.
    pub fn new(home: &Path, bin: &Path) -> Self {
        Self {
            settings: SettingsFile::load(home.join("settings.toml")),
            env: Arc::new(ShellEnv {
                vars: BTreeMap::from([
                    ("HOME".to_owned(), home.display().to_string()),
                    ("USER".to_owned(), "yardsort".to_owned()),
                    (
                        "PATH".to_owned(),
                        format!("{}:/usr/bin:/bin", bin.display()),
                    ),
                    // Nothing of the machine's own git setup reaches a test, nor the other way.
                    ("GIT_CONFIG_NOSYSTEM".to_owned(), "1".to_owned()),
                    (
                        "XDG_CONFIG_HOME".to_owned(),
                        home.join(".config").display().to_string(),
                    ),
                ]),
                source: EnvSource::Process,
                warning: None,
            }),
            projects: Mutex::new(Vec::new()),
            about: About {
                program: "yardsort-host".into(),
                version: "0.0.0".into(),
                pid: std::process::id(),
                remote: Some(RemoteLink {
                    server_url: "https://remote.example.test".into(),
                    name: "Build box".into(),
                    link: "connected, 1 device(s) on it now".into(),
                    devices: 2,
                }),
                terminals: 3,
                notices: Vec::new(),
                remote_off_hint: "Sign in with `yardsort-host sign-in <server>`.".into(),
            },
            projects_dir: home.join("Projects"),
        }
    }
}

impl Source for StandIn {
    fn about(&self) -> About {
        self.about.clone()
    }

    fn env(&self) -> Arc<ShellEnv> {
        self.env.clone()
    }

    fn settings(&self) -> &SettingsFile {
        &self.settings
    }

    fn projects(&self) -> Vec<Project> {
        self.projects.lock().unwrap().clone()
    }

    /// "Clones" by name alone: the last part of the address. An address with `missing` in it is
    /// one the forge does not have.
    fn clone_project(&self, remote: &str) -> Result<Project, String> {
        if remote.contains("missing") {
            return Err("git could not clone it: repository not found.".into());
        }
        let name = remote
            .trim_end_matches('/')
            .trim_end_matches(".git")
            .rsplit(['/', ':'])
            .next()
            .unwrap_or("repository")
            .to_owned();
        let project = Project {
            id: format!("project-{name}"),
            path: self.projects_dir.join(&name).display().to_string(),
            name,
        };
        self.projects.lock().unwrap().push(project.clone());
        Ok(project)
    }
}
