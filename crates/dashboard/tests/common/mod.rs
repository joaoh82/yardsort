//! A stand-in for the program that serves the dashboard: a host that is whatever a test, or the
//! example that shows the page, says it is.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use host_dashboard::remote::{self, SignInState, SigningIn};
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
    /// Remote as the page manages it, for a stand-in that lets it: `about`'s link is then this
    /// one's.
    pub remote: Option<StandInRemote>,
}

/// A server and its devices, made up: signing in takes the address `…/signed-in?code=good`, and
/// a pairing code is asked after at once by a phone that wants everything.
#[derive(Default)]
pub struct StandInRemote(pub Mutex<Managed>);

#[derive(Default)]
pub struct Managed {
    pub signed_in: Option<RemoteLink>,
    pub signing: SignInState,
    pub name: String,
    pub devices: Vec<remote::Device>,
    pub asking: Vec<remote::Asking>,
    pub offers: u32,
}

/// The port the made-up sign-in sends a browser back to.
pub const BACK_PORT: u16 = 49152;

impl remote::Remote for StandInRemote {
    fn devices(&self) -> Result<remote::Devices, String> {
        let managed = self.0.lock().unwrap();
        Ok(remote::Devices {
            devices: managed.devices.clone(),
            asking: managed.asking.clone(),
            projects: ["my-service", "website"]
                .into_iter()
                .map(|name| remote::Choice {
                    id: format!("project-{name}"),
                    name: name.to_owned(),
                })
                .collect(),
        })
    }

    fn offer(&self) -> Result<remote::Offer, String> {
        let mut managed = self.0.lock().unwrap();
        if managed.signed_in.is_none() {
            return Err("Remote is off: sign in to a server first.".into());
        }
        managed.offers += 1;
        let n = managed.offers;
        managed.asking.push(remote::Asking {
            grant: format!("grant-{n}"),
            name: "Pixel 9".into(),
            wants_control: true,
            wants_workflows: true,
        });
        Ok(remote::Offer {
            code: "K7QF-2M9X".into(),
            link: format!(
                "yardsort://pair?server=https%3A%2F%2Fremote.example.test&secret=offer-secret-{n}"
            ),
            expires_in_ms: 300_000,
        })
    }

    fn answer(&self, grant: &str, answer: remote::Answer) -> Result<(), String> {
        let mut managed = self.0.lock().unwrap();
        let at = managed
            .asking
            .iter()
            .position(|asking| asking.grant == grant)
            .ok_or("That device is no longer waiting. Show a new code to pair it.")?;
        let asked = managed.asking.remove(at);
        if answer.allow {
            managed.devices.push(remote::Device {
                grant: asked.grant,
                name: asked.name,
                control: answer.control,
                projects: answer.projects.map(|ids| {
                    ids.iter()
                        .map(|id| id.trim_start_matches("project-").to_owned())
                        .collect()
                }),
            });
        }
        Ok(())
    }

    fn revoke(&self, grant: &str) -> Result<String, String> {
        let mut managed = self.0.lock().unwrap();
        let at = managed
            .devices
            .iter()
            .position(|device| device.grant == grant)
            .ok_or("No such device is let in.")?;
        Ok(managed.devices.remove(at).name)
    }

    fn begin_sign_in(&self, server: &str, name: &str) -> Result<SigningIn, String> {
        let mut managed = self.0.lock().unwrap();
        if managed.signed_in.is_some() {
            return Err("This computer is signed in already.".into());
        }
        if !server.starts_with("https://") {
            return Err("That is not a server's address.".into());
        }
        managed.name = name.to_owned();
        let told = SigningIn {
            url: format!("{server}/authorize?state=made-up"),
            port: BACK_PORT,
            server: server.to_owned(),
        };
        managed.signing = SignInState::Waiting(told.clone());
        Ok(told)
    }

    fn signing_in(&self) -> SignInState {
        self.0.lock().unwrap().signing.clone()
    }

    fn finish_sign_in(&self, address: &str) -> Result<(), String> {
        let mut managed = self.0.lock().unwrap();
        let SignInState::Waiting(waiting) = managed.signing.clone() else {
            return Err("No sign-in is waiting. Start one first.".into());
        };
        if address != format!("http://127.0.0.1:{BACK_PORT}/signed-in?code=good") {
            return Err("That is not the address this sign-in waits for.".into());
        }
        managed.signed_in = Some(RemoteLink {
            server_url: waiting.server,
            name: managed.name.clone(),
            link: "connecting".into(),
            devices: 0,
        });
        managed.signing = SignInState::None;
        Ok(())
    }

    fn cancel_sign_in(&self) {
        self.0.lock().unwrap().signing = SignInState::None;
    }
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
                    devices: 1,
                }),
                terminals: 3,
                notices: Vec::new(),
                remote_off_hint: "Sign in with `yardsort-host sign-in <server>`.".into(),
            },
            projects_dir: home.join("Projects"),
            remote: None,
        }
    }

    /// The same, with Remote managed from the page: signed out, or signed in as `about` says
    /// with one phone let in.
    pub fn managing(mut self, signed_in: bool) -> Self {
        let link = self.about.remote.take();
        self.remote = Some(StandInRemote(Mutex::new(Managed {
            devices: if signed_in {
                vec![remote::Device {
                    grant: "grant-0".into(),
                    name: "iPhone".into(),
                    control: true,
                    projects: None,
                }]
            } else {
                Vec::new()
            },
            signed_in: link.filter(|_| signed_in),
            ..Managed::default()
        })));
        self
    }
}

impl Source for StandIn {
    fn about(&self) -> About {
        let mut about = self.about.clone();
        if let Some(managing) = &self.remote {
            about.remote = managing.0.lock().unwrap().signed_in.clone();
        }
        about
    }

    fn remote(&self) -> Option<&dyn remote::Remote> {
        self.remote
            .as_ref()
            .map(|managing| managing as &dyn remote::Remote)
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
