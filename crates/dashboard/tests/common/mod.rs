//! A stand-in for the program that serves the dashboard: a host that is whatever a test, or the
//! example that shows the page, says it is.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use host_dashboard::{
    About, PairAnswer, PairProject, Pairing, Project, Remote, RemoteLink, SigningIn, Source,
};
use yardsort_core::env::{EnvSource, ShellEnv};
use yardsort_core::settings::SettingsFile;

pub struct StandIn {
    pub settings: SettingsFile,
    pub env: Arc<ShellEnv>,
    pub projects: Mutex<Vec<Project>>,
    pub about: About,
    /// Where a clone "goes".
    pub projects_dir: PathBuf,
    /// Signing in and pairing from the page, as a host with no screen lets them be done. None,
    /// as the app gives.
    pub remote: Option<StandInRemote>,
}

/// A host's Remote, which a test drives: a device asks when the test says, and every answer is
/// kept to be looked at. An address with `code=good` in it is the sign-in's.
#[derive(Default)]
pub struct StandInRemote {
    pub pairing: Mutex<Option<Pairing>>,
    pub signing_in: Mutex<Option<SigningIn>>,
    pub answers: Mutex<Vec<PairAnswer>>,
    pub signed_out: Mutex<bool>,
}

impl StandInRemote {
    /// A device asks, on an offer that is out.
    pub fn asks(&self, device: &str, wants_control: bool) {
        *self.pairing.lock().unwrap() = Some(Pairing::Asked {
            grant: "grant-1".into(),
            device: device.into(),
            wants_control,
            wants_workflows: false,
            projects: vec![
                PairProject {
                    id: "p1".into(),
                    name: "widgets".into(),
                },
                PairProject {
                    id: "p2".into(),
                    name: "gadgets".into(),
                },
            ],
        });
    }
}

impl Remote for StandInRemote {
    fn pairing(&self) -> Pairing {
        self.pairing
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(Pairing::Idle)
    }

    fn pair(&self) -> Result<Pairing, String> {
        let offered = Pairing::Offered {
            code: "ABCD-1234".into(),
            qr: vec!["101".into(), "010".into(), "101".into()],
            lapses_in_ms: 300_000,
        };
        *self.pairing.lock().unwrap() = Some(offered.clone());
        Ok(offered)
    }

    fn answer(&self, answer: PairAnswer) -> Result<Pairing, String> {
        let device = match &*self.pairing.lock().unwrap() {
            Some(Pairing::Asked { grant, device, .. }) if *grant == answer.grant => device.clone(),
            _ => return Err("No device is asking now. Show a code again.".into()),
        };
        let allowed = answer.allow;
        self.answers.lock().unwrap().push(answer);
        let answered = Pairing::Answered { device, allowed };
        *self.pairing.lock().unwrap() = Some(answered.clone());
        Ok(answered)
    }

    fn stop_pairing(&self) {
        *self.pairing.lock().unwrap() = None;
    }

    fn signing_in(&self) -> SigningIn {
        self.signing_in
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(SigningIn::Idle)
    }

    fn sign_in(&self, server: &str, name: &str) -> Result<SigningIn, String> {
        if server.is_empty() {
            return Err("Name the server.".into());
        }
        let waiting = SigningIn::Waiting {
            url: format!("{server}/v1/login?name={name}"),
            port: 45999,
            problem: None,
        };
        *self.signing_in.lock().unwrap() = Some(waiting.clone());
        Ok(waiting)
    }

    fn pasted(&self, address: &str) -> Result<SigningIn, String> {
        if !address.contains("code=good") {
            return Err("That is not the address this sign-in waits for.".into());
        }
        let done = SigningIn::SignedIn {
            name: "Build box".into(),
            server_url: "https://remote.example.test".into(),
        };
        *self.signing_in.lock().unwrap() = Some(done.clone());
        Ok(done)
    }

    fn stop_signing_in(&self) {
        *self.signing_in.lock().unwrap() = None;
    }

    fn sign_out(&self) -> Result<(), String> {
        *self.signed_out.lock().unwrap() = true;
        Ok(())
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
                    devices: 2,
                }),
                terminals: 3,
                notices: Vec::new(),
                remote_off_hint: "Sign in with `yardsort-host sign-in <server>`.".into(),
            },
            projects_dir: home.join("Projects"),
            remote: None,
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

    fn remote(&self) -> Option<&dyn Remote> {
        self.remote.as_ref().map(|remote| remote as &dyn Remote)
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
